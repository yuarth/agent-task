//! End-to-end tests that exercise the compiled `agent-task` binary against a
//! throwaway SQLite database (via `AGENT_TASK_DB`), covering the CRUD
//! command surface, JSON output, and concurrent multi-process writes.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn cmd(db_path: &PathBuf) -> Command {
    let mut c = Command::cargo_bin("agent-task").unwrap();
    c.env("AGENT_TASK_DB", db_path);
    c.env("NO_COLOR", "1");
    c
}

fn new_db() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tasks.db");
    (dir, path)
}

#[test]
fn add_then_list_shows_task() {
    let (_dir, db) = new_db();

    cmd(&db)
        .args([
            "add",
            "最初のタスク",
            "--priority",
            "high",
            "--tags",
            "backend,bugfix",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("タスクを追加しました"));

    cmd(&db)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("最初のタスク"))
        .stdout(predicate::str::contains("high"));
}

#[test]
fn list_default_hides_done_all_flag_shows_it() {
    let (_dir, db) = new_db();

    cmd(&db).args(["add", "完了予定タスク"]).assert().success();
    cmd(&db).args(["complete", "1"]).assert().success();

    // Default view excludes done tasks.
    cmd(&db)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("完了予定タスク").not());

    // --all includes them.
    cmd(&db)
        .args(["list", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("完了予定タスク"));

    // Explicit --status done also includes it.
    cmd(&db)
        .args(["list", "--status", "done"])
        .assert()
        .success()
        .stdout(predicate::str::contains("完了予定タスク"));
}

#[test]
fn show_returns_valid_json_with_expected_fields() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args([
            "add",
            "JSON確認タスク",
            "--assigned",
            "claude",
            "--due",
            "2026-08-01",
        ])
        .assert()
        .success();

    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("show --json は有効な JSON であること");
    assert_eq!(value["id"], 1);
    assert_eq!(value["title"], "JSON確認タスク");
    assert_eq!(value["assigned_agent"], "claude");
    assert_eq!(value["due_date"], "2026-08-01");
    assert_eq!(value["status"], "todo");
}

#[test]
fn list_json_returns_valid_array() {
    let (_dir, db) = new_db();
    cmd(&db).args(["add", "A"]).assert().success();
    cmd(&db).args(["add", "B"]).assert().success();

    let output = cmd(&db).args(["list", "--all", "--json"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("list --json は有効な JSON であること");
    let arr = value.as_array().expect("JSON 配列であること");
    assert_eq!(arr.len(), 2);
}

#[test]
fn update_changes_only_specified_fields() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args([
            "add",
            "更新前",
            "--description",
            "元の説明",
            "--priority",
            "low",
        ])
        .assert()
        .success();

    cmd(&db)
        .args([
            "update",
            "1",
            "--status",
            "in_progress",
            "--assigned",
            "agy",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("タスクを更新しました"));

    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "in_progress");
    assert_eq!(value["assigned_agent"], "agy");
    assert_eq!(value["title"], "更新前");
    assert_eq!(value["priority"], "low");
    assert_eq!(value["description"], "元の説明");
}

#[test]
fn complete_sets_status_done() {
    let (_dir, db) = new_db();
    cmd(&db).args(["add", "完了させる"]).assert().success();
    cmd(&db)
        .args(["complete", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("タスクを完了しました"));

    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["status"], "done");
}

#[test]
fn delete_removes_task() {
    let (_dir, db) = new_db();
    cmd(&db).args(["add", "削除対象"]).assert().success();
    cmd(&db)
        .args(["delete", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("タスクを削除しました"));

    cmd(&db)
        .args(["show", "1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("見つかりません"));
}

#[test]
fn show_missing_task_fails_with_message() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["show", "999"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("見つかりません"));
}

#[test]
fn invalid_status_is_rejected() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "不正ステータス", "--status", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("無効な status"));
}

#[test]
fn invalid_priority_is_rejected() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "不正優先度", "--priority", "urgentish"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("無効な priority"));
}

#[test]
fn invalid_due_date_is_rejected() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "不正期限", "--due", "らくだ"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("無効な due"));
}

#[test]
fn valid_due_date_formats_are_accepted() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "A", "--due", "2026-08-01"])
        .assert()
        .success();
    cmd(&db)
        .args(["add", "B", "--due", "2026-08-01T12:00:00+09:00"])
        .assert()
        .success();
}

#[test]
fn update_rejects_invalid_due_date() {
    let (_dir, db) = new_db();
    cmd(&db).args(["add", "T"]).assert().success();
    cmd(&db)
        .args(["update", "1", "--due", "2026-13-99"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("無効な due"));
}

#[test]
fn tag_filter_matches_exact_tag_only() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "A", "--tags", "backend,bugfix"])
        .assert()
        .success();
    cmd(&db)
        .args(["add", "B", "--tags", "frontend"])
        .assert()
        .success();

    cmd(&db)
        .args(["list", "--tag", "backend"])
        .assert()
        .success()
        .stdout(predicate::str::contains("A"))
        .stdout(predicate::str::contains("B").not());
}

#[test]
fn ansi_escape_sequences_are_sanitized_on_add() {
    let (_dir, db) = new_db();
    let malicious_title = "\x1b[31mHacked\x1b[0m\x07Title";
    cmd(&db).args(["add", malicious_title]).assert().success();

    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let stored_title = value["title"].as_str().unwrap();
    assert!(!stored_title.contains('\u{1b}'));
    assert!(!stored_title.contains('\u{7}'));
    assert_eq!(stored_title, "HackedTitle");
}

#[test]
fn ansi_escape_sequences_are_sanitized_on_update() {
    let (_dir, db) = new_db();
    cmd(&db).args(["add", "普通のタイトル"]).assert().success();

    let malicious_desc = "line1\x1b[2J\x1b[Hline2";
    cmd(&db)
        .args(["update", "1", "--description", malicious_desc])
        .assert()
        .success();

    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let stored_desc = value["description"].as_str().unwrap();
    assert!(!stored_desc.contains('\u{1b}'));
    assert_eq!(stored_desc, "line1line2");
}

#[test]
fn ansi_escape_in_assigned_and_tags_is_sanitized() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args([
            "add",
            "T",
            "--assigned",
            "claude\x1b[31m",
            "--tags",
            "back\x1b[3mend,bug\x07fix",
        ])
        .assert()
        .success();

    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["assigned_agent"], "claude");
    assert_eq!(value["tags"], "backend,bugfix");
}

#[test]
fn overlong_unterminated_csi_payload_does_not_swallow_trailing_title_text() {
    let (_dir, db) = new_db();
    let junk_params: String = "9".repeat(50);
    let malicious_title = format!("\x1b[{junk_params}TAIL");
    cmd(&db).args(["add", &malicious_title]).assert().success();

    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let stored = value["title"].as_str().unwrap();
    assert!(
        stored.ends_with("TAIL"),
        "後続文字列が欠落してはならない: {stored}"
    );
    assert!(!stored.contains('\u{1b}'));
}

#[test]
fn tag_filter_percent_wildcard_is_escaped_in_cli() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "A", "--tags", "100%"])
        .assert()
        .success();
    cmd(&db)
        .args(["add", "B", "--tags", "100X"])
        .assert()
        .success();

    cmd(&db)
        .args(["list", "--tag", "100%"])
        .assert()
        .success()
        .stdout(predicate::str::contains("A"))
        .stdout(predicate::str::contains("B").not());
}

#[test]
fn add_rejects_empty_title() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", ""])
        .assert()
        .failure()
        .stderr(predicate::str::contains("空にすることはできません"));
}

#[test]
fn add_rejects_whitespace_only_title() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "   "])
        .assert()
        .failure()
        .stderr(predicate::str::contains("空にすることはできません"));
}

#[test]
fn add_rejects_overlong_title() {
    let (_dir, db) = new_db();
    let long_title = "a".repeat(501);
    cmd(&db)
        .args(["add", &long_title])
        .assert()
        .failure()
        .stderr(predicate::str::contains("長すぎます"));
}

#[test]
fn add_rejects_overlong_assigned_field() {
    let (_dir, db) = new_db();
    let long_value = "a".repeat(301);
    cmd(&db)
        .args(["add", "T", "--assigned", &long_value])
        .assert()
        .failure()
        .stderr(predicate::str::contains("長すぎます"));
}

/// Regression test found in adversarial review: length validation must
/// reject an oversized value based on its *raw* length before the
/// (potentially expensive, for adversarial content) sanitize pass ever
/// runs on it — otherwise the length limit added to guard against exactly
/// this kind of input doesn't actually prevent it from being processed.
/// This payload is built from the same pattern that caused an O(n^2) hang
/// in sanitize::skip_osc pre-fix; if validation ran after sanitizing, this
/// would take many seconds (or, at larger sizes, much longer). It must
/// instead be rejected immediately for its length.
#[test]
fn overlong_field_with_adversarial_content_is_rejected_quickly() {
    let (_dir, db) = new_db();
    let adversarial_payload = "\x1b]".repeat(50_000); // 100,000 chars, well over the 300-char cap
    let start = std::time::Instant::now();
    cmd(&db)
        .args(["add", "T", "--assigned", &adversarial_payload])
        .assert()
        .failure()
        .stderr(predicate::str::contains("長すぎます"));
    assert!(
        start.elapsed().as_secs() < 3,
        "rejection took {:?}; length validation should short-circuit before sanitizing",
        start.elapsed()
    );
}

#[test]
fn add_rejects_zero_width_space_only_title() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "\u{200b}\u{200b}\u{200b}"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("空にすることはできません"));
}

/// Regression test found in a second round of adversarial review: a title
/// made of zero-width characters interleaved with plain spaces (e.g.
/// "  \u{200b}  \u{200b}  ") used to bypass the zero-width-title check,
/// because `.trim()` only strips a leading/trailing *run* of whitespace and
/// stops at the first non-whitespace (zero-width) character from each edge
/// -- leaving interior plain spaces, which have nonzero width on their own,
/// untrimmed. The title still rendered as an entirely blank cell in `list`.
#[test]
fn add_rejects_title_with_zero_width_chars_interleaved_with_spaces() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "  \u{200b}  \u{200b}  "])
        .assert()
        .failure()
        .stderr(predicate::str::contains("空にすることはできません"));
}

#[test]
fn update_rejects_setting_title_to_empty() {
    let (_dir, db) = new_db();
    cmd(&db).args(["add", "元のタイトル"]).assert().success();
    cmd(&db)
        .args(["update", "1", "--title", ""])
        .assert()
        .failure()
        .stderr(predicate::str::contains("空にすることはできません"));

    // The original title must be untouched after the rejected update.
    let output = cmd(&db).args(["show", "1", "--json"]).output().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["title"], "元のタイトル");
}

/// A malicious/garbled `--status` value that fails validation must not leak
/// raw ANSI/OSC escape bytes into the error text printed to stderr. Unlike
/// the sanitization applied to *stored* fields (title/description/etc.),
/// this exercises the separate "reject up-front, but still echo the value
/// back in the error message" code path in `Status::parse`.
#[test]
fn invalid_status_error_message_strips_ansi_escape_bytes() {
    let (_dir, db) = new_db();
    let malicious_status = "\x1b]8;;http://evil.example\x07click\x1b]8;;\x07me";
    let output = cmd(&db)
        .args(["add", "T", "--status", malicious_status])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains('\u{1b}'),
        "stderr must not contain a raw ESC byte: {stderr:?}"
    );
    assert!(stderr.contains("clickme"));
}

/// Same class of bug, via `--priority` on `update`.
#[test]
fn invalid_priority_error_message_strips_ansi_escape_bytes() {
    let (_dir, db) = new_db();
    cmd(&db).args(["add", "T"]).assert().success();
    let malicious_priority = "\x1b[31mnope\x1b[0m";
    let output = cmd(&db)
        .args(["update", "1", "--priority", malicious_priority])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains('\u{1b}'));
    assert!(stderr.contains("nope"));
}

#[test]
fn tag_filter_underscore_wildcard_is_escaped_in_cli() {
    let (_dir, db) = new_db();
    cmd(&db)
        .args(["add", "A", "--tags", "a_b"])
        .assert()
        .success();
    cmd(&db)
        .args(["add", "B", "--tags", "axb"])
        .assert()
        .success();

    cmd(&db)
        .args(["list", "--tag", "a_b"])
        .assert()
        .success()
        .stdout(predicate::str::contains("A"))
        .stdout(predicate::str::contains("B").not());
}

/// Multiple OS processes writing to the same SQLite DB concurrently must not
/// crash or deadlock: WAL mode + busy_timeout should serialize writers.
#[test]
fn concurrent_multi_process_writes_do_not_crash_or_lose_data() {
    let (_dir, db) = new_db();
    let bin = assert_cmd::cargo::cargo_bin("agent-task");

    const WRITERS: usize = 12;
    let mut children = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let child = StdCommand::new(&bin)
            .env("AGENT_TASK_DB", &db)
            .env("NO_COLOR", "1")
            .args(["add", &format!("並行タスク{i}"), "--assigned", "agent"])
            .spawn()
            .expect("プロセス起動に失敗");
        children.push(child);
    }

    let mut failures = 0;
    for mut child in children {
        let status = child.wait().expect("プロセス待機に失敗");
        if !status.success() {
            failures += 1;
        }
    }
    assert_eq!(
        failures, 0,
        "並行書き込みで失敗したプロセスがあってはならない"
    );

    let output = StdCommand::new(&bin)
        .env("AGENT_TASK_DB", &db)
        .env("NO_COLOR", "1")
        .args(["list", "--all", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let arr = value.as_array().unwrap();
    assert_eq!(
        arr.len(),
        WRITERS,
        "全プロセスの書き込みが失われずに反映されていること"
    );
}

/// A freshly-created DB directory must be owner-only (0700): tasks can carry
/// sensitive text, and on a shared machine the default umask-derived mode
/// would let other local users traverse into the directory and read the
/// db/wal/shm files inside.
#[cfg(unix)]
#[test]
fn new_db_directory_is_created_with_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    // A nested, not-yet-existing path so `open_db` creates the parent itself
    // (as opposed to `new_db()`, whose TempDir already exists).
    let db = dir.path().join("nested").join("tasks.db");

    cmd(&db).args(["add", "T"]).assert().success();

    let parent = db.parent().unwrap();
    let mode = std::fs::metadata(parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o700,
        "expected owner-only dir permissions, got {mode:o}"
    );
}

/// An already-existing DB directory keeps whatever permissions it had —
/// hardening only applies to directories this tool creates itself.
#[cfg(unix)]
#[test]
fn pre_existing_db_directory_permissions_are_left_alone() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, db) = new_db();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

    cmd(&db).args(["add", "T"]).assert().success();

    let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "pre-existing directory permissions must be untouched"
    );
}

/// Regression test found in adversarial review: the directory-hardening fix
/// creates the directory with its restrictive mode set atomically (via
/// `DirBuilder::mode`) instead of create-then-chmod, to close a TOCTOU
/// window. That atomic primitive errors with `AlreadyExists` if the
/// directory already exists — which several concurrent processes racing to
/// create the *same brand-new* directory will legitimately trigger for all
/// but one of them. Since this tool is explicitly designed for multiple
/// concurrent agent processes sharing one DB, that race must not surface as
/// a failure to any of them, and the directory must still end up 0700.
#[cfg(unix)]
#[test]
fn concurrent_fresh_directory_creation_does_not_fail_any_process() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    // Not-yet-existing, so every process races to create it.
    let db = dir.path().join("brand_new_nested_dir").join("tasks.db");
    let bin = assert_cmd::cargo::cargo_bin("agent-task");

    const WRITERS: usize = 12;
    let mut children = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let child = StdCommand::new(&bin)
            .env("AGENT_TASK_DB", &db)
            .env("NO_COLOR", "1")
            .args(["add", &format!("同時作成タスク{i}")])
            .spawn()
            .expect("プロセス起動に失敗");
        children.push(child);
    }

    let mut failures = 0;
    for mut child in children {
        let status = child.wait().expect("プロセス待機に失敗");
        if !status.success() {
            failures += 1;
        }
    }
    assert_eq!(
        failures, 0,
        "concurrent first-time directory creation must not fail any process"
    );

    let parent = db.parent().unwrap();
    let mode = std::fs::metadata(parent).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o700,
        "directory must still end up owner-only regardless of which process created it"
    );
}
