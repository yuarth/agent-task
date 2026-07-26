use anyhow::{Context, Result};
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
            std::fs::create_dir_all(parent).with_context(|| {
                format!("ディレクトリの作成に失敗しました: {}", parent.display())
            })?;
        }
    }

    let conn = Connection::open(path)
        .with_context(|| format!("データベースを開けませんでした: {}", path.display()))?;

    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;

    init_schema(&conn)?;
    Ok(conn)
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
    if get_task(conn, id)?.is_none() {
        return Ok(None);
    }

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
}
