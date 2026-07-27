use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::models::Task;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'todo',
    priority TEXT NOT NULL DEFAULT 'medium',
    assigned_agent TEXT,
    tags TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    due_date TEXT
);

CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_assigned ON tasks(assigned_agent);
"#;

/// Resolve the database file path: `$AGENT_TASK_DB` overrides the default
/// `~/.local/share/agent-task/tasks.db`.
pub fn resolve_db_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("AGENT_TASK_DB") {
        if !p.trim().is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = dirs::home_dir().context("ホームディレクトリを解決できませんでした")?;
    Ok(home
        .join(".local")
        .join("share")
        .join("agent-task")
        .join("tasks.db"))
}

/// Number of attempts [`open_db`] makes before giving up on a transient
/// `SQLITE_BUSY`/`SQLITE_LOCKED` error while opening/initializing the
/// database. See [`open_db`]'s doc comment for why this exists.
const OPEN_DB_MAX_ATTEMPTS: u32 = 5;

/// Open (creating if necessary) the SQLite database at `path`, apply the
/// concurrency pragmas, and ensure the schema exists.
///
/// Retries a bounded number of times on a transient `SQLITE_BUSY`/
/// `SQLITE_LOCKED` error: this tool is explicitly designed for many
/// concurrent agent processes to race to create the *same brand-new*
/// database file, and `conn.busy_timeout()` below is only set (and thus
/// only able to retry internally) *after* `Connection::open` returns --
/// the handful of milliseconds needed to open the file, apply pragmas, and
/// run `init_schema` before that point is a real, if narrow, window where
/// many simultaneous first-time WAL-mode initializations can still surface
/// `SQLITE_BUSY` to the caller. Confirmed empirically: this can occur (rarely)
/// even without any of this module's own logic in the way, and stress-testing
/// showed the exact frequency is sensitive to how much work happens before
/// the first `Connection::open` call (e.g. this module's own directory
/// trust checks), so a bounded retry here is more robust than trying to
/// eliminate the window by shaving individual syscalls off that path.
pub fn open_db(path: &Path) -> Result<Connection> {
    let mut last_err = None;
    for attempt in 0..OPEN_DB_MAX_ATTEMPTS {
        match try_open_db(path) {
            Ok(conn) => return Ok(conn),
            Err(e) if attempt + 1 < OPEN_DB_MAX_ATTEMPTS && is_transient_busy(&e) => {
                std::thread::sleep(Duration::from_millis(50 * u64::from(attempt + 1)));
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.expect("loop always sets this before exiting via retry exhaustion"))
}

/// `true` if `err`'s cause chain contains a `rusqlite::Error` carrying
/// `SQLITE_BUSY` or `SQLITE_LOCKED` -- the specific, transient conditions
/// [`open_db`]'s retry loop exists to ride out. Any other error (a genuine
/// I/O failure, a permissions problem, a malformed database, ...) is
/// propagated immediately without retrying.
fn is_transient_busy(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<rusqlite::Error>()
            .and_then(rusqlite::Error::sqlite_error_code)
            .is_some_and(|code| {
                matches!(
                    code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
            })
    })
}

fn try_open_db(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            ensure_db_dir(parent)?;
        }
    }

    // Refuse to follow a symlink planted at the exact DB file path. This
    // matters even inside a directory `ensure_db_dir` otherwise trusts: a
    // shared, sticky-bit directory like `/tmp` still lets *any* local user
    // create a brand-new entry with a name they can predict (the sticky bit
    // only stops non-owners from deleting/replacing an entry someone else
    // already created) -- e.g. pre-planting `/tmp/my-tasks.db` as a symlink
    // to an arbitrary file before the victim's first run. `symlink_metadata`
    // reports symlink-ness without following the link, so this also catches
    // a dangling symlink (whose target doesn't exist, but which `exists()`
    // below wouldn't flag as needing this check on its own).
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            bail!(
                "DB ファイル '{}' がシンボリックリンクです。信頼できない場所を指している可能性があるため使用を拒否します。",
                path.display()
            );
        }
    }

    // Checked before `Connection::open` creates the file, so we only harden
    // permissions on a file *we* just created, not one that already existed
    // (which may have deliberately different permissions).
    let db_is_new = !path.exists();

    let conn = Connection::open(path)
        .with_context(|| format!("データベースを開けませんでした: {}", path.display()))?;

    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;

    init_schema(&conn)?;

    if db_is_new {
        harden_new_db_file_permissions(path)?;
    }

    Ok(conn)
}

/// Restrict a freshly-created `tasks.db` to owner-only read/write (0600).
/// The containing directory (see [`ensure_db_dir`]) is the primary access
/// control (its 0700 mode already blocks traversal by other local users),
/// but the file's own default, umask-derived mode is typically world- or
/// group-readable, so this is defense in depth in case the directory's
/// protection is ever bypassed or the directory is reused across a mode
/// change. No-op on non-Unix targets, which have no equivalent primitive
/// here.
#[cfg(unix)]
fn harden_new_db_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "DB ファイルのパーミッション設定に失敗しました: {}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_new_db_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

/// Ensure `dir` (the directory that will hold `tasks.db`/`-wal`/`-shm`)
/// exists, creating it with owner-only permissions if we're the one
/// creating it.
///
/// Task titles/descriptions can carry sensitive info, and on a shared
/// multi-user machine the default directory mode (subject to umask,
/// typically 0755) would let any local user read the db/wal/shm files
/// inside by traversing into it. So: leave an already-existing `dir`
/// untouched (its owner may have deliberately shared it) *as long as it is
/// actually trustworthy* -- see [`check_dir_is_trustworthy`] -- but if we
/// create it fresh, restrict it to owner-only (0700).
///
/// The immediate ancestor of `dir` is checked the same way if it already
/// exists (closing the gap where a symlink planted one level *above* `dir`,
/// rather than at `dir` itself, would otherwise still redirect storage), but
/// no further up than that -- `AGENT_TASK_DB` could point deep into an
/// unrelated shared path, and locking down directories this tool doesn't own
/// would be a surprising side effect.
fn ensure_db_dir(dir: &Path) -> Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(_) => return check_dir_is_trustworthy(dir),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(e)
                .with_context(|| format!("ディレクトリの確認に失敗しました: {}", dir.display()))
        }
    }

    if let Some(ancestor) = dir.parent() {
        if !ancestor.as_os_str().is_empty() {
            if std::fs::symlink_metadata(ancestor).is_ok() {
                check_dir_is_trustworthy(ancestor)?;
            }
            std::fs::create_dir_all(ancestor).with_context(|| {
                format!("ディレクトリの作成に失敗しました: {}", ancestor.display())
            })?;
        }
    }

    create_dir_owner_only(dir)
        .with_context(|| format!("ディレクトリの作成に失敗しました: {}", dir.display()))?;

    // `create_dir_owner_only` treats `AlreadyExists` as success, since
    // several of our *own* processes may legitimately race to create the
    // same brand-new directory. But `mkdir()` also returns `AlreadyExists`
    // if literally anything (including a symlink an attacker planted in the
    // window between the check above and this call) now occupies the path.
    // Re-validate before trusting the result either way.
    check_dir_is_trustworthy(dir)
}

/// Refuse to trust `dir` as a DB storage location unless it resolves (after
/// following any symlinks) to a real directory that is either owned by the
/// user running this process, or -- on Unix -- carries the sticky bit
/// (mode `01000`). The sticky bit is the standard Unix convention for a
/// directory that is safe to share with untrusted local users (`/tmp`,
/// `/var/tmp`, ...): it stops anyone but an entry's own owner (or root) from
/// deleting or renaming it, which is exactly the property that makes a
/// shared directory usable as a parent for files this tool itself creates
/// and owns.
///
/// `dir` itself may be a symlink -- e.g. macOS's `/tmp` is a symlink to
/// `/private/tmp` -- what matters is whether the *resolved* target is
/// trustworthy, not whether reaching it involved a symlink. An earlier
/// version of this check rejected any symlink and any non-owned directory
/// outright, which broke this project's own documented
/// `AGENT_TASK_DB=/tmp/my-tasks.db` example: `/tmp` is a symlink on macOS
/// and root-owned (not the invoking user) on typical Linux systems, so that
/// exact command failed on both (found in outer adversarial review of PR
/// #14). Task data can still carry sensitive text, so this only relaxes the
/// check to the standard "sticky shared directory" case -- an attacker's own
/// unprivileged directory (not owned by the running user, sticky bit not
/// set, as an unprivileged user cannot chown to another uid or usefully fake
/// this combination) is still rejected. A predictable *file* name inside a
/// trusted shared directory is separately guarded in [`open_db`], since the
/// sticky bit alone doesn't stop another local user from creating a
/// brand-new entry there before this tool's first run.
fn check_dir_is_trustworthy(dir: &Path) -> Result<()> {
    let real = std::fs::canonicalize(dir)
        .with_context(|| format!("ディレクトリの確認に失敗しました: {}", dir.display()))?;
    let meta = std::fs::metadata(&real)
        .with_context(|| format!("ディレクトリの確認に失敗しました: {}", real.display()))?;

    if !meta.is_dir() {
        bail!(
            "DB ディレクトリ '{}' (実体: '{}') はディレクトリではありません",
            dir.display(),
            real.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let owner_uid = meta.uid();
        let running_uid = current_uid();
        let has_sticky_bit = meta.mode() & 0o1000 != 0;
        if owner_uid != running_uid && !has_sticky_bit {
            bail!(
                "DB ディレクトリ '{}' (実体: '{}') の所有者 (uid={owner_uid}) が実行ユーザー \
                 (uid={running_uid}) と一致せず、共有ディレクトリを示すスティッキービットも \
                 設定されていません。信頼できない可能性があるため使用を拒否します。",
                dir.display(),
                real.display()
            );
        }
    }

    Ok(())
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: `geteuid()` takes no arguments, performs no pointer
    // dereferences, and cannot fail.
    unsafe { libc::geteuid() }
}

/// Create `dir` with owner-only (0700) permissions set atomically at
/// creation time, rather than via a separate `chmod` afterward — a
/// create-then-chmod sequence leaves a window where the directory briefly
/// has the default, looser mode, which a co-resident local user could race
/// to exploit. `AlreadyExists` is treated as success: this tool is designed
/// for multiple concurrent agent processes sharing one DB, so another
/// process may have won the race to create this same directory — and since
/// it would have gone through this same function, it already has the
/// right mode. No-op-equivalent fallback on non-Unix targets, which have no
/// mode-on-create primitive here.
#[cfg(unix)]
fn create_dir_owner_only(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(not(unix))]
fn create_dir_owner_only(dir: &Path) -> std::io::Result<()> {
    match std::fs::create_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

/// Filters accepted by `list`.
#[derive(Debug, Default)]
pub struct ListFilter {
    pub status: Option<String>,
    pub assigned: Option<String>,
    pub priority: Option<String>,
    pub tag: Option<String>,
    pub all: bool,
}

/// Escape `\`, `%`, and `_` so a value can be interpolated into a SQL
/// `LIKE ... ESCAPE '\'` pattern and matched literally.
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

const TASK_COLUMNS: &str =
    "id, title, description, status, priority, assigned_agent, tags, created_at, updated_at, due_date";

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        status: row.get(3)?,
        priority: row.get(4)?,
        assigned_agent: row.get(5)?,
        tags: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        due_date: row.get(9)?,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn add_task(
    conn: &Connection,
    title: &str,
    description: Option<&str>,
    status: &str,
    priority: &str,
    assigned: Option<&str>,
    tags: Option<&str>,
    due: Option<&str>,
    now: &str,
) -> Result<Task> {
    conn.execute(
        "INSERT INTO tasks (title, description, status, priority, assigned_agent, tags, created_at, updated_at, due_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
        rusqlite::params![title, description, status, priority, assigned, tags, now, due],
    )?;
    let id = conn.last_insert_rowid();
    get_task(conn, id)?.context("挿入直後のタスク取得に失敗しました")
}

pub fn get_task(conn: &Connection, id: i64) -> Result<Option<Task>> {
    let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(rusqlite::params![id], row_to_task)?;
    match rows.next() {
        Some(t) => Ok(Some(t?)),
        None => Ok(None),
    }
}

pub fn list_tasks(conn: &Connection, filter: &ListFilter) -> Result<Vec<Task>> {
    let mut sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE 1=1");
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(status) = &filter.status {
        sql.push_str(" AND status = ?");
        params.push(Box::new(status.clone()));
    } else if !filter.all {
        // Default view hides completed tasks so agents see actionable work first.
        sql.push_str(" AND status != 'done'");
    }

    if let Some(assigned) = &filter.assigned {
        sql.push_str(" AND assigned_agent = ?");
        params.push(Box::new(assigned.clone()));
    }

    if let Some(priority) = &filter.priority {
        sql.push_str(" AND priority = ?");
        params.push(Box::new(priority.clone()));
    }

    if let Some(tag) = &filter.tag {
        // The stored side is compared with spaces stripped (`REPLACE(tags, '
        // ', '')`), so the search term must be normalized the same way --
        // otherwise "backend" and "back end" match inconsistently depending
        // on which side of the comparison happens to have the space (a task
        // tagged "back end" would wrongly match a search for "backend", while
        // searching for "back end" itself would wrongly find nothing).
        // Escape LIKE wildcards (% and _) in user input so a tag like "50%"
        // or "a_b" is matched literally instead of as a pattern.
        sql.push_str(" AND (',' || REPLACE(tags, ' ', '') || ',') LIKE ? ESCAPE '\\'");
        let normalized_tag = tag.trim().replace(' ', "");
        params.push(Box::new(format!("%,{},%", escape_like(&normalized_tag))));
    }

    sql.push_str(" ORDER BY \
        CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END, \
        id ASC");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), row_to_task)?;
    let mut tasks = Vec::new();
    for row in rows {
        tasks.push(row?);
    }
    Ok(tasks)
}

#[allow(clippy::too_many_arguments)]
pub fn update_task(
    conn: &Connection,
    id: i64,
    title: Option<&str>,
    description: Option<&str>,
    status: Option<&str>,
    priority: Option<&str>,
    assigned: Option<&str>,
    tags: Option<&str>,
    due: Option<&str>,
    now: &str,
) -> Result<Option<Task>> {
    // No separate existence pre-check: UPDATE against a non-existent id
    // safely affects zero rows (no error), and the final `get_task` below
    // already reports the correct Some/None either way — a pre-check here
    // would just be a second, redundant query.
    let mut sets: Vec<String> = vec!["updated_at = ?".to_string()];
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now.to_string())];

    macro_rules! set_field {
        ($col:literal, $val:expr) => {
            if let Some(v) = $val {
                sets.push(format!("{} = ?", $col));
                params.push(Box::new(v.to_string()));
            }
        };
    }
    set_field!("title", title);
    set_field!("description", description);
    set_field!("status", status);
    set_field!("priority", priority);
    set_field!("assigned_agent", assigned);
    set_field!("tags", tags);
    set_field!("due_date", due);

    let sql = format!("UPDATE tasks SET {} WHERE id = ?", sets.join(", "));
    params.push(Box::new(id));

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, param_refs.as_slice())?;

    get_task(conn, id)
}

pub fn complete_task(conn: &Connection, id: i64, now: &str) -> Result<Option<Task>> {
    update_task(
        conn,
        id,
        None,
        None,
        Some("done"),
        None,
        None,
        None,
        None,
        now,
    )
}

pub fn delete_task(conn: &Connection, id: i64) -> Result<bool> {
    let affected = conn.execute("DELETE FROM tasks WHERE id = ?1", rusqlite::params![id])?;
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn schema_creates_table_and_indexes() {
        let conn = mem_conn();
        let table_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'tasks'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);

        let index_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name IN ('idx_tasks_status', 'idx_tasks_assigned')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 2);
    }

    #[test]
    fn schema_init_is_idempotent() {
        let conn = mem_conn();
        // Calling init again must not error (IF NOT EXISTS guards).
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
    }

    #[test]
    fn add_and_get_task_roundtrip() {
        let conn = mem_conn();
        let t = add_task(
            &conn,
            "テストタスク",
            Some("説明文"),
            "todo",
            "medium",
            Some("claude"),
            Some("backend,bugfix"),
            Some("2026-08-01"),
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        assert_eq!(t.title, "テストタスク");
        assert_eq!(t.status, "todo");
        assert_eq!(t.tag_list(), vec!["backend", "bugfix"]);

        let fetched = get_task(&conn, t.id).unwrap().unwrap();
        assert_eq!(fetched.id, t.id);
        assert_eq!(fetched.assigned_agent.as_deref(), Some("claude"));
    }

    #[test]
    fn get_task_missing_returns_none() {
        let conn = mem_conn();
        assert!(get_task(&conn, 999).unwrap().is_none());
    }

    #[test]
    fn list_default_excludes_done_unless_all() {
        let conn = mem_conn();
        add_task(
            &conn,
            "A",
            None,
            "todo",
            "medium",
            None,
            None,
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        add_task(
            &conn,
            "B",
            None,
            "done",
            "medium",
            None,
            None,
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();

        let default_list = list_tasks(&conn, &ListFilter::default()).unwrap();
        assert_eq!(default_list.len(), 1);
        assert_eq!(default_list[0].title, "A");

        let all_list = list_tasks(
            &conn,
            &ListFilter {
                all: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all_list.len(), 2);
    }

    #[test]
    fn list_filters_by_status_assigned_priority_tag() {
        let conn = mem_conn();
        add_task(
            &conn,
            "A",
            None,
            "todo",
            "high",
            Some("claude"),
            Some("backend"),
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        add_task(
            &conn,
            "B",
            None,
            "in_progress",
            "low",
            Some("agy"),
            Some("frontend,ui"),
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();

        let by_status = list_tasks(
            &conn,
            &ListFilter {
                status: Some("in_progress".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_status.len(), 1);
        assert_eq!(by_status[0].title, "B");

        let by_assigned = list_tasks(
            &conn,
            &ListFilter {
                assigned: Some("claude".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_assigned.len(), 1);
        assert_eq!(by_assigned[0].title, "A");

        let by_priority = list_tasks(
            &conn,
            &ListFilter {
                priority: Some("low".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_priority.len(), 1);
        assert_eq!(by_priority[0].title, "B");

        let by_tag = list_tasks(
            &conn,
            &ListFilter {
                tag: Some("ui".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].title, "B");
    }

    #[test]
    fn update_task_changes_only_given_fields() {
        let conn = mem_conn();
        let t = add_task(
            &conn,
            "Orig",
            Some("desc"),
            "todo",
            "medium",
            None,
            None,
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();

        let updated = update_task(
            &conn,
            t.id,
            None,
            None,
            Some("in_progress"),
            None,
            None,
            None,
            None,
            "2026-07-26T01:00:00Z",
        )
        .unwrap()
        .unwrap();
        assert_eq!(updated.status, "in_progress");
        assert_eq!(updated.title, "Orig");
        assert_eq!(updated.description.as_deref(), Some("desc"));
        assert_eq!(updated.updated_at, "2026-07-26T01:00:00Z");
    }

    #[test]
    fn update_missing_task_returns_none() {
        let conn = mem_conn();
        let res = update_task(
            &conn,
            42,
            Some("x"),
            None,
            None,
            None,
            None,
            None,
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn complete_task_sets_status_done() {
        let conn = mem_conn();
        let t = add_task(
            &conn,
            "T",
            None,
            "in_progress",
            "medium",
            None,
            None,
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        let done = complete_task(&conn, t.id, "2026-07-26T02:00:00Z")
            .unwrap()
            .unwrap();
        assert_eq!(done.status, "done");
    }

    #[test]
    fn tag_filter_normalizes_internal_spaces_like_stored_column() {
        // Regression test for Issue #11: the stored side is compared with
        // spaces stripped (`REPLACE(tags, ' ', '')`), so a filter of "backend"
        // must also match a task whose tag is literally "back end", and a
        // filter of "back end" must find it too -- both sides need the same
        // normalization, not just the stored one.
        let conn = mem_conn();
        add_task(
            &conn,
            "A",
            None,
            "todo",
            "medium",
            None,
            Some("back end,other"),
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();

        for query in ["backend", "back end"] {
            let matches = list_tasks(
                &conn,
                &ListFilter {
                    tag: Some(query.into()),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                matches.len(),
                1,
                "query {query:?} should match the \"back end\" tag"
            );
        }
    }

    #[test]
    fn tag_filter_escapes_percent_wildcard() {
        let conn = mem_conn();
        add_task(
            &conn,
            "A",
            None,
            "todo",
            "medium",
            None,
            Some("100%"),
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        add_task(
            &conn,
            "B",
            None,
            "todo",
            "medium",
            None,
            Some("100X"),
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();

        // Without escaping, "100%" would match "100X" too via the LIKE wildcard.
        let matches = list_tasks(
            &conn,
            &ListFilter {
                tag: Some("100%".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].title, "A");
    }

    #[test]
    fn tag_filter_escapes_underscore_wildcard() {
        let conn = mem_conn();
        add_task(
            &conn,
            "A",
            None,
            "todo",
            "medium",
            None,
            Some("a_b"),
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        add_task(
            &conn,
            "B",
            None,
            "todo",
            "medium",
            None,
            Some("axb"),
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();

        // Without escaping, "a_b" would match "axb" too since "_" matches any char.
        let matches = list_tasks(
            &conn,
            &ListFilter {
                tag: Some("a_b".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].title, "A");
    }

    #[test]
    fn delete_task_removes_row() {
        let conn = mem_conn();
        let t = add_task(
            &conn,
            "T",
            None,
            "todo",
            "medium",
            None,
            None,
            None,
            "2026-07-26T00:00:00Z",
        )
        .unwrap();
        assert!(delete_task(&conn, t.id).unwrap());
        assert!(get_task(&conn, t.id).unwrap().is_none());
        assert!(!delete_task(&conn, t.id).unwrap());
    }

    /// Regression test for outer adversarial review of PR #14: an earlier
    /// version of `check_dir_is_trustworthy` rejected *any* symlink
    /// unconditionally, which broke this project's own documented
    /// `AGENT_TASK_DB=/tmp/my-tasks.db` example (`/tmp` is a symlink to
    /// `/private/tmp` on macOS). A symlink to a directory the running user
    /// already owns must be accepted -- what matters is the trustworthiness
    /// of the resolved target, not whether a symlink was involved.
    #[cfg(unix)]
    #[test]
    fn ensure_db_dir_accepts_symlink_to_owned_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let real_dir = tmp.path().join("relocated_storage");
        std::fs::create_dir(&real_dir).unwrap();

        let link = tmp.path().join("db_dir_symlink");
        std::os::unix::fs::symlink(&real_dir, &link).unwrap();

        ensure_db_dir(&link).unwrap();
    }

    /// Direct regression test for the actual reported bug: a sticky-bit
    /// shared directory (the standard Unix convention for a directory safe
    /// to share with untrusted local users, e.g. `/tmp` itself, mode 01777)
    /// must be usable as the DB directory even though it isn't owned by the
    /// running user. Without this, `AGENT_TASK_DB=/tmp/my-tasks.db` fails on
    /// any system where `/tmp` isn't owned by the invoking user (i.e. nearly
    /// everywhere -- `/tmp` is normally root-owned).
    #[cfg(unix)]
    #[test]
    fn ensure_db_dir_accepts_sticky_shared_directory() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let shared = tmp.path().join("shared_tmp_like");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();

        ensure_db_dir(&shared).unwrap();
    }

    /// A dangling symlink (target doesn't exist) must be rejected with a
    /// clear error, not panic.
    #[cfg(unix)]
    #[test]
    fn ensure_db_dir_rejects_dangling_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let link = tmp.path().join("dangling_link");
        std::os::unix::fs::symlink(tmp.path().join("does_not_exist"), &link).unwrap();

        assert!(ensure_db_dir(&link).is_err());
    }

    /// A symlink pointing at a regular *file* (not a directory) is an
    /// invalid configuration and must be rejected with a clear error rather
    /// than silently misbehaving later when SQLite tries to treat it as a
    /// directory.
    #[cfg(unix)]
    #[test]
    fn ensure_db_dir_rejects_symlink_to_non_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let regular_file = tmp.path().join("just_a_file");
        std::fs::write(&regular_file, b"not a directory").unwrap();

        let link = tmp.path().join("link_to_file");
        std::os::unix::fs::symlink(&regular_file, &link).unwrap();

        assert!(ensure_db_dir(&link).is_err());
    }

    /// A symlink placed one level *above* the DB-holding directory (rather
    /// than at the DB directory itself) must be subject to the same
    /// trust check -- otherwise it silently redirects storage exactly like a
    /// symlink at the DB directory itself would (found in outer adversarial
    /// review of PR #14: only `dir` itself was checked, never
    /// `dir.parent()`). Here the resolved ancestor is owned by the running
    /// user (the only case a single-user test can construct without root),
    /// so it must be accepted -- the important behavioral change is that the
    /// ancestor is actually *inspected* at all now, matching `dir`'s
    /// existing protection instead of being skipped entirely.
    #[cfg(unix)]
    #[test]
    fn ensure_db_dir_checks_immediate_ancestor_symlink_too() {
        let tmp = tempfile::TempDir::new().unwrap();
        let real_ancestor = tmp.path().join("real_ancestor");
        std::fs::create_dir(&real_ancestor).unwrap();

        let ancestor_link = tmp.path().join("ancestor_link");
        std::os::unix::fs::symlink(&real_ancestor, &ancestor_link).unwrap();

        // `dir` itself (the DB-holding directory) doesn't exist yet -- it
        // sits one level below the symlinked ancestor.
        let dir = ancestor_link.join("db_subdir");
        ensure_db_dir(&dir).unwrap();
        assert!(real_ancestor.join("db_subdir").is_dir());
    }

    /// A brand-new directory (no pre-existing path component at all) must
    /// still be created normally -- the symlink/ownership check in
    /// `check_dir_is_trustworthy` must not reject the common case.
    #[test]
    fn ensure_db_dir_creates_fresh_directory_normally() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("fresh");
        ensure_db_dir(&dir).unwrap();
        assert!(dir.is_dir());
    }

    /// Regression test for outer adversarial review of PR #14: stress-testing
    /// `concurrent_fresh_directory_creation_does_not_fail_any_process`
    /// (12 racing OS processes) revealed that this project's directory-trust
    /// checks -- by adding a little more work before the first
    /// `Connection::open` call -- made an already-latent, rare `SQLITE_BUSY`
    /// race (present even before this PR, at roughly 1-in-200 runs, since
    /// `conn.busy_timeout()` can only start retrying *after* `Connection::open`
    /// returns) noticeably more frequent (roughly 1-in-30 in local testing).
    /// `open_db`'s bounded retry loop must recognize a real SQLITE_BUSY/
    /// SQLITE_LOCKED error so it knows to retry rather than propagate it
    /// immediately.
    #[test]
    fn is_transient_busy_detects_sqlite_busy_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("locktest.db");

        let conn1 = Connection::open(&path).unwrap();
        conn1.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let conn2 = Connection::open(&path).unwrap();
        // No retrying on conn2's side -- we want the raw SQLITE_BUSY here,
        // not for rusqlite to have already waited it out internally.
        conn2.busy_timeout(Duration::from_millis(0)).unwrap();
        let err: anyhow::Error = conn2
            .execute("CREATE TABLE t (id INTEGER)", [])
            .unwrap_err()
            .into();

        assert!(
            is_transient_busy(&err),
            "expected a transient-busy error, got: {err}"
        );
    }

    /// An unrelated error (not a database lock contention) must not be
    /// retried -- only SQLITE_BUSY/SQLITE_LOCKED should trigger `open_db`'s
    /// retry loop, so a genuine failure (bad permissions, corrupt file, ...)
    /// still surfaces immediately instead of being masked by pointless
    /// retries.
    #[test]
    fn is_transient_busy_is_false_for_unrelated_errors() {
        let err = anyhow::anyhow!("some unrelated error");
        assert!(!is_transient_busy(&err));
    }
}
