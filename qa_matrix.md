# QA Matrix — agent-task CLI

`spec_agent_task_cli.md` §5「検証マトリクス」に基づく検証結果。すべて Rust
`edition 2021` / `cargo test` および Nix サンドボックスビルド上で確認済み。

| テスト分類 | テスト項目 | 判定基準 | 結果 | 根拠 |
| :--- | :--- | :--- | :--- | :--- |
| 単体テスト | DB スキーマ作成・テーブル初期化 | 自動作成およびインデックスが正常生成されること | ✅ PASS | `src/db.rs::tests::schema_creates_table_and_indexes`, `schema_init_is_idempotent` — `sqlite_master` に `tasks` テーブルおよび `idx_tasks_status` / `idx_tasks_assigned` インデックスが存在することを検証 |
| CRUD 操作 | `add` | 挿入した値が正しく取得できること | ✅ PASS | `src/db.rs::tests::add_and_get_task_roundtrip`, `tests/cli.rs::add_then_list_shows_task` |
| CRUD 操作 | `list` (フィルタ: status/assigned/priority/tag, `--all`) | 各フィルタ条件で期待どおりの結果集合になること | ✅ PASS | `src/db.rs::tests::list_filters_by_status_assigned_priority_tag`, `list_default_excludes_done_unless_all`, `tests/cli.rs::list_default_hides_done_all_flag_shows_it`, `tag_filter_matches_exact_tag_only` |
| CRUD 操作 | `show` | 指定 ID のタスク詳細を取得できること／存在しない ID はエラーになること | ✅ PASS | `src/db.rs::tests::get_task_missing_returns_none`, `tests/cli.rs::show_returns_valid_json_with_expected_fields`, `show_missing_task_fails_with_message` |
| CRUD 操作 | `update` | 指定フィールドのみが更新され、他は保持されること | ✅ PASS | `src/db.rs::tests::update_task_changes_only_given_fields`, `update_missing_task_returns_none`, `tests/cli.rs::update_changes_only_specified_fields` |
| CRUD 操作 | `complete` | ステータスが `done` に遷移するショートカットとして機能すること | ✅ PASS | `src/db.rs::tests::complete_task_sets_status_done`, `tests/cli.rs::complete_sets_status_done` |
| CRUD 操作 | `delete` | 行が削除され、以後 `show` で取得不可になること | ✅ PASS | `src/db.rs::tests::delete_task_removes_row`, `tests/cli.rs::delete_removes_task` |
| 入力検証 | 不正な `status` / `priority` | わかりやすいエラーメッセージで拒否されること | ✅ PASS | `tests/cli.rs::invalid_status_is_rejected`, `invalid_priority_is_rejected` |
| 並行性・ロック | 複数プロセス同時書き込み | WAL モード & busy_timeout によりデッドロックやクラッシュを起こさないこと | ✅ PASS | `tests/cli.rs::concurrent_multi_process_writes_do_not_crash_or_lose_data` — 12 個の独立 OS プロセスが同一 DB ファイルへ同時に `add` を実行し、全プロセスが正常終了 (exit 0) かつ全 12 件が欠損なく反映されることを確認 |
| 表示・カラー | ANSI カラーテーブル表示 (`status` 絵文字 / `priority` 色分け) | 🟢in_progress / 🔵done / 🟡todo / 🔴blocked と priority が視認性良く出力されること | ✅ PASS | `src/models.rs::Status::emoji` が仕様どおりの絵文字を返すことを型で保証。`src/output.rs` の `print_table` / `print_detail` で ANSI 装飾を付与（`NO_COLOR` 環境変数または非 TTY 出力時は自動的に無効化し、テスト・パイプ出力を汚さないことを確認: `tests/cli.rs` は全件 `NO_COLOR=1` 下で実行し安定してパースできている） |
| データフォーマット | `--json` 出力 | 正当な JSON 配列/オブジェクト構造が出力されパース可能であること | ✅ PASS | `tests/cli.rs::show_returns_valid_json_with_expected_fields`, `list_json_returns_valid_array` — `serde_json::from_str` でパース可能なことおよび主要フィールドの値を検証 |
| セキュリティ (レビュー指摘 #1) | ANSI エスケープ/制御文字のサニタイズ | `title`/`description`/`assigned_agent`/`tags`/`due_date` に含まれる ESC (0x1B) 起点の CSI/OSC シーケンスおよびその他の制御文字が、格納時に無害化されること | ✅ PASS | `src/sanitize.rs::tests` (CSI カラー/カーソル移動/画面クリア/OSC ハイパーリンク偽装/C1 制御文字を網羅)、`tests/cli.rs::ansi_escape_sequences_are_sanitized_on_add`, `ansi_escape_sequences_are_sanitized_on_update`, `ansi_escape_in_assigned_and_tags_is_sanitized` — `add`/`update` 経由で悪意あるエスケープシーケンスを注入し、`show --json` で取得した値にシーケンスが残存しないことを確認 |
| セキュリティ (レビュー指摘 #2) | `tag` フィルタの LIKE ワイルドカードエスケープ | タグ値に含まれる `%` / `_` が LIKE パターンのワイルドカードとして解釈されず、リテラル一致すること | ✅ PASS | `src/db.rs::tests::tag_filter_escapes_percent_wildcard`, `tag_filter_escapes_underscore_wildcard`、`tests/cli.rs::tag_filter_percent_wildcard_is_escaped_in_cli`, `tag_filter_underscore_wildcard_is_escaped_in_cli` — エスケープなしなら誤ヒットする組（`100%`/`100X`, `a_b`/`axb`）で意図した1件のみに絞り込まれることを確認。`assigned` フィルタは元々 `=` による厳密一致（バインドパラメータ経由）で `LIKE` を使用していないため、ワイルドカード解釈のリスクはなく変更不要 |
| Nix ビルド | `nix-build default.nix` | サンドボックス内でコンパイル・全テストが通過し単一バイナリが生成されること | ✅ PASS | `nix-build default.nix --no-out-link` 成功。ビルド中に `cargo test` (単体20件 + 結合17件 = 37件) が Nix サンドボックス内で実行され全件 PASS。生成バイナリ (`bin/agent-task`) の `--version` / `add` / `list --json` 実行を確認 |
| Nix ビルド | `nix build .#default` | サンドボックス内でコンパイル・全テストが通過し単一バイナリが生成されること | ✅ PASS | `nix build .#default --no-link` 成功 (flake, `nixpkgs/nixos-unstable` 入力)。生成バイナリの `--help` 実行を確認 |

## 実行コマンド一覧（再現用）

```bash
# ユニット + 結合テスト (37件)
cargo test

# Lint / フォーマット
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# Nix ビルド (チャンネル版 / flake 版 双方)
nix-build default.nix
nix build .#default
```

## 既知の設計判断

- `agent-task list`（フィルタなし）はデフォルトで `status = done` のタスクを
  非表示にする（エージェントに未完了の作業を優先的に見せるため）。全件表示
  したい場合は `--all`、または `--status done` を明示する。
- カラー出力は標準出力が TTY のときのみ有効。`NO_COLOR` 環境変数、または
  パイプ/リダイレクト時は自動的にプレーンテキストへフォールバックする。
- DB パスは `AGENT_TASK_DB` 環境変数で上書き可能（テスト・複数エージェント
  環境での分離に使用）。未設定時は `~/.local/share/agent-task/tasks.db`。
- ANSI エスケープ/制御文字のサニタイズは `src/sanitize.rs` にて格納時
  (`add`/`update`) に実施する。ESC 単体の除去だけでは CSI シーケンス本体
  (例: `[31m`) が可視ゴミとして残るため、CSI (`ESC '[' ... final-byte`) /
  OSC (`ESC ']' ... (BEL|ST)`) / その他の2バイトエスケープをシーケンス単位
  で検出・除去したうえで、残る制御文字（`description` のみ `\n`/`\t` を許容）
  も除去する。
- `tag` フィルタの LIKE パターンは `ESCAPE '\'` 句を用い、ユーザー入力中の
  `\`/`%`/`_` をエスケープしてからバインドする。`assigned`/`status`/`priority`
  フィルタはもともと `=` による厳密一致（パラメータバインド）のため対象外。
