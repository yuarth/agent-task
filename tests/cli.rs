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
