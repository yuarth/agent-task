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

/// Open (creating if necessary) the SQLite database at `path`, apply the
/// concurrency pragmas, and ensure the schema exists.
pub fn open_db(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            ensure_db_dir(parent)?;
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
/// Only `dir` itself gets the restrictive mode, not any missing ancestor
/// directories above it — `AGENT_TASK_DB` could point deep into an
/// unrelated shared path, and locking down directories this tool doesn't
/// own would be a surprising side effect.
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

/// Refuse to trust `dir` as the DB storage location if it is a symlink (it
/// could silently redirect storage to a location outside our control, e.g.
/// one an attacker fully owns) or, on Unix, if it is owned by a different
/// user than the one running this process (it could be a directory another
/// local user planted ahead of time at a predictable path). Task data can
/// carry sensitive text, so a directory this tool did not itself create and
/// cannot vouch for the origin of must not be used silently.
fn check_dir_is_trustworthy(dir: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(dir)
        .with_context(|| format!("ディレクトリの確認に失敗しました: {}", dir.display()))?;

    if meta.file_type().is_symlink() {
        bail!(
            "DB ディレクトリ '{}' がシンボリックリンクです。信頼できない場所を指している可能性があるため使用を拒否します。シンボリックリンクを削除するか、AGENT_TASK_DB に実ディレクトリを指定してください。",
            dir.display()
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let owner_uid = meta.uid();
        let running_uid = current_uid();
        if owner_uid != running_uid {
            bail!(
                "DB ディレクトリ '{}' の所有者 (uid={owner_uid}) が実行ユーザー (uid={running_uid}) と一致しません。信頼できない可能性があるため使用を拒否します。",
                dir.display()
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
        // Escape LIKE wildcards (% and _) in user input so a tag like "50%"
        // or "a_b" is matched literally instead of as a pattern.
        sql.push_str(" AND (',' || REPLACE(tags, ' ', '') || ',') LIKE ? ESCAPE '\\'");
        params.push(Box::new(format!("%,{},%", escape_like(tag.trim()))));
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

    /// Regression test for Issue #10: a symlink pre-placed at the would-be DB
    /// directory path must be rejected outright, not silently followed --
    /// otherwise an attacker who can plant a symlink at a predictable path
    /// (e.g. under a shared /tmp) before the victim's first run could
    /// redirect task storage into a directory they fully control.
    #[cfg(unix)]
    #[test]
    fn ensure_db_dir_rejects_symlink() {
        let tmp = tempfile::TempDir::new().unwrap();
        let attacker_dir = tmp.path().join("attacker_controlled");
        std::fs::create_dir(&attacker_dir).unwrap();

        let link = tmp.path().join("victim_db_dir");
        std::os::unix::fs::symlink(&attacker_dir, &link).unwrap();

        let err = ensure_db_dir(&link).unwrap_err().to_string();
        assert!(
            err.contains("シンボリックリンク"),
            "expected a symlink-rejection error, got: {err}"
        );
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
}
