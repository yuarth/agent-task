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
| セキュリティ (レビュー指摘 #3) | CSI パラメータ長の上限 | `ESC '['` に続く終端バイト無しの長大な入力でも、後続文字列を無限にスキャン/消費しないこと(上限16バイトで打ち切り、超過時は ESC のみ破棄してスキャン継続) | ✅ PASS | `src/sanitize.rs::tests::clean_line_recognizes_csi_sequence_right_at_param_limit`(境界内=16バイトは正規CSIとして除去), `clean_line_gives_up_on_csi_over_param_limit_and_keeps_scanning`(境界超過=17バイトはESCのみ破棄しリテラル保持), `clean_line_unterminated_csi_does_not_swallow_trailing_text`, `tests/cli.rs::overlong_unterminated_csi_payload_does_not_swallow_trailing_title_text` — 終端バイトの無い50バイトのダミーパラメータを付与しても末尾の `TAIL` 文字列が欠落しないことを確認 |
| Nix ビルド | `nix-build default.nix` | サンドボックス内でコンパイル・全テストが通過し単一バイナリが生成されること | ✅ PASS | `nix-build default.nix --no-out-link` 成功。ビルド中に `cargo test` (単体23件 + 結合18件 = 41件) が Nix サンドボックス内で実行され全件 PASS。生成バイナリ (`bin/agent-task`) の `--version` / `add` / `list --json` 実行を確認 |
| Nix ビルド | `nix build .#default` | サンドボックス内でコンパイル・全テストが通過し単一バイナリが生成されること | ✅ PASS | `nix build .#default --no-link` 成功 (flake, `nixpkgs/nixos-unstable` 入力)。生成バイナリの `--help` 実行を確認 |
| セキュリティ (敵対的監査 #1) | `--status`/`--priority` バリデーションエラー時のANSIエスケープ注入 | 不正な `--status`/`--priority` 値にCSI/OSCエスケープシーケンスを含めても、エラーメッセージ (stderr) にESCバイトがそのまま出力されないこと | ✅ FIXED / PASS | 修正前は `Status::parse`/`Priority::parse` の `bail!` が生の入力値をそのままエラー文字列に埋め込んでおり、格納データに対する `sanitize` の保護対象外だった。`src/sanitize.rs::sanitize_for_message`（ANSI除去+80文字での丸め）を追加し `src/models.rs` の両 `parse()` に適用。`src/models.rs::tests::status_parse_error_strips_ansi_from_echoed_value` ほか、`tests/cli.rs::invalid_status_error_message_strips_ansi_escape_bytes`, `invalid_priority_error_message_strips_ansi_escape_bytes` で、stderr に生ESCバイトが含まれないことを確認 |
| 入力検証 (敵対的監査 #2) | `title` の空文字列/空白のみ、および各フィールドの長さ上限 | 空/空白のみの `title` が拒否されること。`title`(500文字)/`description`(20,000文字)/`assigned`・`tags`・`due`(300文字)を超える入力が明確なエラーで拒否されること | ✅ FIXED / PASS | 修正前は空タイトルのタスクが作成でき、`assigned`/`tags`/`due` に長さ制限がなく `list` のテーブル列幅が実測で200文字超の値1件で400文字超に破綻することを確認（対策前）。`src/validate.rs` を新設し `add`/`update` に適用。`src/validate.rs::tests`、`tests/cli.rs::add_rejects_empty_title`, `add_rejects_whitespace_only_title`, `add_rejects_overlong_title`, `add_rejects_overlong_assigned_field`, `update_rejects_setting_title_to_empty` で確認 |
| セキュリティ (敵対的監査 #3) | DB 保存ディレクトリのパーミッション | 本ツールが新規作成する DB ディレクトリが所有者のみアクセス可能 (0700) であること。既存ディレクトリのパーミッションは変更しないこと | ✅ FIXED / PASS | 修正前はディレクトリ作成時の umask 依存（既定 0755 等）で、共有マシン上の他ローカルユーザーが `tasks.db`/WAL/SHM を読める可能性があった。`src/db.rs::harden_dir_permissions` (Unix限定, 新規作成時のみ) を追加。`tests/cli.rs::new_db_directory_is_created_with_owner_only_permissions`, `pre_existing_db_directory_permissions_are_left_alone` で確認 |
| セキュリティ強化 (敵対的監査 #4 / Issue #5) | C1 8bit制御コード (0x9B/0x9D) によるCSI/OSCシーケンス | `ESC`版だけでなく、8bit C1導入符号 (0x9B=CSI, 0x9D=OSC) 単体で始まるシーケンスも、パラメータ〜終端バイト/BEL/STまで含めて丸ごと除去されること | ✅ FIXED / PASS | 修正前は 0x9B/0x9D が「制御文字1文字」として除去されるのみで、後続のパラメータ文字列（例: `31m`）がリテラルテキストとして残存していた（ESC版との非対称）。`src/sanitize.rs` の `strip_ansi` を `skip_csi`/`skip_osc` ヘルパーに整理し、ESC版・C1版の両方から共有。`sanitize::tests::clean_line_strips_c1_csi_sequence`, `clean_line_strips_c1_osc_sequence`, `clean_line_c1_csi_over_param_limit_drops_only_introducer`, `clean_line_strips_generic_c1_control_byte_and_newlines` で確認 |
| 入力検証 (敵対的監査 #5 / Issue #6) | `--due` の日付フォーマット検証 | `YYYY-MM-DD` または RFC3339 以外の値が明確なエラーで拒否されること | ✅ FIXED / PASS | 修正前は任意の文字列（例: `らくだ`, `2026-13-99`）がそのまま `due_date` に保存されていた。`src/validate.rs::validate_due_date`（`chrono::NaiveDate`/`chrono::DateTime::parse_from_rfc3339` で検証）を追加し `add`/`update` に適用。`validate::tests::due_date_*`、`tests/cli.rs::invalid_due_date_is_rejected`, `valid_due_date_formats_are_accepted`, `update_rejects_invalid_due_date` で確認 |
| コード品質 (敵対的監査 #6 / Issue #7) | `update_task` の冗長な事前存在チェック | 事前の `get_task` 呼び出しを削除しても、存在しない ID への `update`/`complete` が引き続き正しく "not found" として扱われること | ✅ FIXED / PASS | `src/db.rs::update_task` 冒頭の冗長な `get_task` 呼び出しを削除（UPDATE自体は0行影響でも安全に完了し、末尾の `get_task` が最終状態を正しく返すため元々不要だった）。既存の `db::tests::update_missing_task_returns_none` が削除後も PASS することを確認（回帰なし） |
| 依存関係 | `cargo audit` | 既知の RustSec 脆弱性がロック済み依存関係に含まれないこと | ✅ PASS | `cargo audit`（RustSec advisory-db 1169件）実行、Cargo.lock 上の116クレートに対し指摘0件 |
| 静的解析 | `cargo clippy --all-targets -- -D warnings` | 警告ゼロで通過すること | ✅ PASS | 敵対的監査で指摘した全6件（Issue #2〜#7）の修正後のコードで再実行し警告0件を確認。`cargo test` は単体49件+結合30件=79件、全PASS |
| セキュリティ (PR #8 外部敵対的レビュー #1) | `skip_osc` の O(n²) DoS | 終端バイト(BEL/ST)を持たない `ESC ']'`/`0x9D` が連続する巨大な入力(320,000〜100万文字)でも、書き込み系(`add`/`update`)・読み取り専用系(`list --status`)のいずれも準線形時間で応答すること | ✅ FIXED / PASS | 修正前は `skip_csi` にのみ存在した上限 (`CSI_PARAM_LIMIT=16`) が新設の `skip_osc` には無く、未終端OSCの反復でO(n²)（実測: 320,000文字で約11.6秒、`list --status` のような読み取り専用コマンドでも同様に発生）。`OSC_PARAM_LIMIT=512` を導入し `skip_osc` を同じ形で打ち切るよう修正。`sanitize::tests::clean_line_osc_over_param_limit_drops_only_introducer`, `clean_line_many_unterminated_osc_introducers_completes_quickly`(20万個の未終端導入符号が5秒未満で完了することをタイミングアサートで保証) で確認。実バイナリでも再検証: 100万文字のESC OSC/CSI、40万文字のC1 OSC(`0x9D`)いずれも1秒未満で応答 |
| セキュリティ (PR #8 外部敵対的レビュー #2) | バリデーション(長さ制限)がサニタイズより先に実行されること | `title`/`description`/`assigned`/`tags`/`due` いずれも、生の引数長が上限を超える場合は `sanitize::clean_line`/`clean_multiline` を一切実行せず即座に拒否されること | ✅ FIXED / PASS | 修正前は `sanitize` が先、長さチェックが後の順序で、上記#1のような「最終的には拒否されるべき」巨大ペイロードでも高コストな処理を先に受けてしまい、長さ制限がDoS対策として機能していなかった。`src/main.rs` の `run_add`/`run_update` で全フィールドについて `validate::check_max_len` を生の引数に対して先に実行する順序へ変更。`tests/cli.rs::overlong_field_with_adversarial_content_is_rejected_quickly` — `"\x1b]".repeat(50_000)`(10万文字)の敵対的ペイロードが3秒未満で長さエラーとして拒否されることをタイミングアサートで確認 |
| セキュリティ (PR #8 外部敵対的レビュー #3) | DB ディレクトリ作成のTOCTOU | 新規DBディレクトリの作成からパーミッション設定(0700)までの間に、緩い権限(umask依存)を持つ瞬間が存在しないこと。12並列プロセスによる同一ディレクトリへの初回同時作成でも、いずれのプロセスも失敗せず最終的に0700で確定すること | ✅ FIXED / PASS | 修正前は `create_dir_all`(既定パーミッションで作成)→事後`chmod`の2段階で、作成からchmodまでの間に緩い権限のウィンドウが存在した(CodeRabbitの自動レビューでも同一箇所を独立に指摘)。`std::os::unix::fs::DirBuilderExt::mode(0o700)` を用いて作成とパーミッション設定をアトミックに行うよう変更。`AlreadyExists` は成功として扱う(複数エージェントプロセスが同じ新規ディレクトリの作成を同時に試みる設計を前提とするため)。`tests/cli.rs::concurrent_fresh_directory_creation_does_not_fail_any_process` — 12並列プロセスでの初回同時作成で全プロセス成功・最終権限0700を確認。実バイナリでも `umask 022` 下での単発作成・12並列作成の双方を再検証し、いずれも0700であることを確認 |
| 入力検証 (PR #8 外部敵対的レビュー #4) | ゼロ幅文字のみの `title` が空タイトル拒否をすり抜けないこと | U+200B等のゼロ幅文字のみ、または通常の空白とゼロ幅文字を交互に配置した `title` が拒否されること。一方で通常の可視文字を含む `title`(内部に空白を含むものを含む)は誤って拒否されないこと | ✅ FIXED / PASS | 1回目の修正(`width(title.trim()) == 0`)は純粋なゼロ幅文字のみのタイトル(例: U+200B×3)は正しく拒否したが、2回目の敵対的レビューで「`"  \u{200b}  \u{200b}  "` のように通常の空白とゼロ幅文字を交互に配置すると `.trim()` が最初の非空白(ゼロ幅)文字で停止し、内部の通常空白(表示幅を持つ)が未トリムのまま残るため、依然として視覚的に空白セルとして `list` に表示されるタスクが作成できてしまう」というバイパスが新たに発見された。`title.chars().filter(|c| !c.is_whitespace())` で空白文字を先頭・末尾に限らず全体から除去してから表示幅を判定するよう修正し、このバイパスを解消。`validate::tests::zero_width_spaces_interleaved_with_plain_spaces_are_rejected`, `interior_spaces_around_visible_text_are_still_accepted`, `tests/cli.rs::add_rejects_title_with_zero_width_chars_interleaved_with_spaces` で確認。実バイナリでもゼロ幅/不可視文字10種の単体テストに加え、乱数生成した60通りの組み合わせがいずれも正しく拒否され、かつ通常タイトル5種が誤って拒否されないことを確認 |

| セキュリティ (第2巡敵対的監査 #1 / Issue #9) | Unicode双方向制御文字(RLO等)によるタイトル/タグ表示のなりすまし | `title`/`description`/`assigned`/`tags`/`due` に含まれる双方向書式文字 (`Cf`: U+202A-U+202E, U+2066-U+2069, U+200E, U+200F) が格納時に無害化されること | ✅ FIXED / PASS | 修正前は `sanitize::strip_ansi` が Unicode 制御文字 (`Cc`) のみを対象としており、`char::is_control()` が `false` を返す `Cf` カテゴリの双方向書式文字（U+202E RIGHT-TO-LEFT OVERRIDE 等）を除去できず、実バイナリで `add "safe‮exe.txt⁩"` を実行すると `show`/`list` の出力でタイトルが視覚的に反転して表示されることを確認した（"Trojan Source" 系のなりすまし手法、参考: CVE-2021-42574 と同クラス）。`src/sanitize.rs::is_bidi_control` を追加し `strip_ansi` の除去対象に組み込んだ（`description` を含む全フィールドで無条件に除去、`\n`/`\t` のような許容例外は設けない）。`sanitize::tests::clean_line_strips_rtl_override`, `clean_line_strips_all_bidi_control_chars`, `clean_multiline_strips_bidi_control_even_though_it_keeps_newlines_and_tabs`、`tests/cli.rs::bidi_override_characters_are_stripped_from_title`, `bidi_override_characters_are_stripped_from_assigned_and_tags` で確認 |
| セキュリティ (第2巡敵対的監査 #2 / Issue #10) | DBディレクトリのシンボリックリンク/所有者不一致の信頼境界 | あらかじめ配置されたシンボリックリンクや、実行ユーザーと所有者が異なる既存ディレクトリを DB ディレクトリとして無条件に信頼しないこと。新規作成される `tasks.db` 自体も所有者のみ読み書き可能 (0600) であること | ✅ FIXED / PASS | 修正前は `ensure_db_dir` が `Path::exists()`（シンボリックリンクを解決してしまう）で存在確認しており、事前に配置されたシンボリックリンクを「既存ディレクトリ」として無条件に信頼し、リンク先（攻撃者が完全に制御するディレクトリ、実測で 0777）にそのまま `tasks.db` を書き込んでしまうことを実バイナリで確認した。`src/db.rs::check_dir_is_trustworthy` を追加し、`symlink_metadata` でシンボリックリンクを検出して拒否、Unix では所有者 uid が実行ユーザーと一致するかも検証するようにした。`create_dir_owner_only` の `AlreadyExists` 許容パス（複数の自プロセスが同一新規ディレクトリ作成を競合するケース向け）についても、作成後に改めて `check_dir_is_trustworthy` を通すことで、その隙間に攻撃者がシンボリックリンクを割り込ませるレースも塞いだ。あわせて `tasks.db` 本体も新規作成時のみ `0600` に制限する `harden_new_db_file_permissions` を追加（多層防御、既存ファイルの権限は変更しない）。`db::tests::ensure_db_dir_rejects_symlink`, `ensure_db_dir_creates_fresh_directory_normally`、`tests/cli.rs::db_directory_that_is_a_symlink_is_rejected`, `new_db_file_has_owner_only_permissions` で確認。所有者不一致のケースは異なる uid の生成に root 権限が必要なためテスト環境では自動テスト化していない |
| 入力検証 (第2巡敵対的監査 #3 / Issue #11) | `tag` フィルタの空白正規化の非対称性 | タグ値内部に空白を含む場合でも、格納側・検索側で一貫した一致結果になること | ✅ FIXED / PASS | 修正前は格納側の比較のみ `REPLACE(tags, ' ', '')` で空白除去済みの値と比較する一方、検索側の `tag` 引数は `.trim()`（先頭/末尾のみ）しか適用しておらず、実バイナリで「`--tags "back end,other"` のタスクが `--tag "backend"` で誤ヒットする（誤マッチ）」「同じタスクが `--tag "back end"` では逆に0件になる（取りこぼし）」の両方を確認した。`src/db.rs::list_tasks` で検索側にも `tag.trim().replace(' ', "")` の同一正規化を適用し、両クエリが一貫して一致するよう修正。`db::tests::tag_filter_normalizes_internal_spaces_like_stored_column`、`tests/cli.rs::tag_filter_normalizes_internal_spaces_consistently` で確認 |
| 堅牢性 (第2巡敵対的監査 #4 / Issue #12) | `update` の無変更呼び出しがサイレントに成功する | 変更対象フィールドを1つも指定しない `update <id>` が明確なエラーで拒否されること | ✅ FIXED / PASS | 修正前は `db::update_task` が常に `updated_at` を SET 句に含めるため、`agent-task update 1`（フィールド指定なし）が無条件に成功し `updated_at` のみが更新されることを実バイナリで確認した。エージェントによるフラグ指定ミス（値の渡し忘れ等）が気付かれずに「成功」してしまう問題があった。`src/main.rs::run_update` で全フィールドが `None` の場合に明確なエラーを返すよう修正。`tests/cli.rs::update_with_no_fields_is_rejected` で確認 |
| 堅牢性 (第2巡敵対的監査 #5 / Issue #13 派生) | `--due` の極端な年(5桁以上)がビルドプロファイル間で挙動不一致 | `--due` の年が4桁でない値(例: `99999-01-01`)が、デバッグ/リリースいずれのビルドでも一貫して拒否されること | ✅ FIXED / PASS | `chrono` の `%Y`/RFC3339 パーサーが年の桁数を厳密に4桁に制限しておらず、内部の日付計算で整数オーバーフローが発生することを発見した。`cargo test`(デバッグビルド、オーバーフローチェック有効)では `--due 99999-01-01` が拒否される一方、`nix-build`/`cargo build --release`(オーバーフローチェック無効)で生成した実バイナリでは同じ値が無条件に受理されてしまうことを実測で確認した（テストスイートが実際の配布物と異なる挙動を検証してしまっていた）。`src/validate.rs::has_four_digit_year` を追加し、年部分が正確に4桁のASCII数字であることを `chrono` に渡す前に検証することで、ビルドプロファイルに依存しない決定論的な挙動にした。`validate::tests::due_date_rejects_year_with_more_than_four_digits`, `due_date_accepts_four_digit_year_boundaries`、`tests/cli.rs::due_date_with_more_than_four_digit_year_is_rejected_deterministically` で確認 |
| テストカバレッジ (第2巡敵対的監査, Issue #13 残項目) | 同一行への並行 update/delete、不正UTF-8引数、`AGENT_TASK_DB` 異常系、結合文字大量連結 | いずれもクラッシュ(パニック/シグナル終了)せず、明確な成功/失敗として処理されること | ✅ PASS | `tests/cli.rs::concurrent_update_and_delete_on_same_row_do_not_crash`(同一行への update/delete 各10プロセスの競合), `invalid_utf8_argument_is_rejected_cleanly_without_panicking`(不正UTF-8バイト列を含む引数がclapレベルで明確に拒否されること), `agent_task_db_with_non_directory_parent_component_fails_cleanly`(`AGENT_TASK_DB` の親パス要素が通常ファイルの場合に明確なエラーになること), `title_heavy_with_combining_marks_does_not_crash_add_or_list`(結合文字400個の Zalgo 風タイトルで `add`/`list` がクラッシュしないこと) をそれぞれ追加し、いずれもクラッシュせず正常終了することを確認 |

## 実行コマンド一覧（再現用）

```bash
# ユニット + 結合テスト (41件)
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
- CSI スキャン (`ESC '[' params... final-byte`) は `ESC '['` の後、最大
  `CSI_PARAM_LIMIT = 16` 文字まで終端バイト (`0x40..=0x7E`) を探索する。
  上限内に見つからない場合はそのシーケンスを CSI とみなさず、ESC のみを
  破棄して次の文字からスキャンを再開する（`[` や後続文字はそのまま通常の
  文字として扱われる）。これにより、終端バイトを持たない不正/悪意ある
  `ESC '['` ペイロードが入力の残り全体を無制限に飲み込んでしまう
  （後続テキストが丸ごと欠落する）事態を防いでいる。
- `title`/`description`/`assigned`/`tags`/`due` は `add`/`update` 時に
  `src/validate.rs` で長さ検証される
  (`title` ≤ 500 文字, `description` ≤ 20,000 文字, その他 ≤ 300 文字)。
  この長さチェックは常に `sanitize::clean_line`/`clean_multiline` より
  *先に*、生の引数に対して行う。順序を逆にすると、最終的には長さで
  拒否されるべき巨大な敵対的ペイロードでもサニタイズ処理そのものを
  先に受けてしまい、長さ上限がDoS対策として機能しなくなるため。
  `title` は空文字列・空白のみも拒否される。文字数はバイト数ではなく
  Unicode スカラ値単位でカウントするため、日本語などマルチバイト文字を
  不当に不利に扱わない。
- `title` の非空判定 (`validate::require_non_empty_title`) は、単純な
  `title.trim().is_empty()` ではなく、文字列中の空白文字（Unicode
  `White_Space` プロパティ）を先頭・末尾に限らず全て除去したうえで、
  残った文字列の表示幅 (`unicode-width`) がゼロかどうかで判定する。
  `.trim()` だけでは「先頭/末尾の空白ラン」しか除去できないため、
  U+200B 等のゼロ幅文字のみのタイトルや、通常の空白とゼロ幅文字を
  交互に配置したタイトル（`.trim()` が最初の非空白文字であるゼロ幅
  文字で止まってしまい、内部の通常空白が未トリムのまま残る）が
  非空と誤判定され、`list` 上で視覚的に区別できないタスクが作成
  できてしまう問題を防いでいる。
- CLI バリデーションエラー（`Status::parse`/`Priority::parse` が拒否した
  不正な `--status`/`--priority` 値）をエラーメッセージに埋め込む際は
  `src/sanitize.rs::sanitize_for_message` を通す。`sanitize::clean_line`
  は「格納される」値の無害化であり、拒否されて保存されない値の
  「エラーメッセージへの埋め込み」は別経路のため、専用の関数で対応する
  （ANSI/制御文字除去 + 80 文字を超える場合は省略記号で丸め）。
- 新規作成する DB ディレクトリ（親ディレクトリが存在しない場合）は Unix
  では `0700` に制限する (`src/db.rs::create_dir_owner_only`)。既存の
  ディレクトリを利用する場合はパーミッションを変更しない。WAL/SHM
  補助ファイル自体の権限までは制御していないため、ディレクトリの
  実行権限（トラバース禁止）がアクセス制御の主な担保となる点に留意。
  作成とパーミッション設定は `DirBuilder::mode(0o700)` によりアトミックに
  行われる（作成後に別途 `chmod` するcreate-then-chmod方式ではない）ため、
  緩い権限が一瞬でも露出するウィンドウは存在しない。複数のエージェント
  プロセスが同一の新規ディレクトリを同時に作成しようとするケースは
  `AlreadyExists` を成功として扱うことで許容している。
- CSI/OSC の検出は `ESC`版（`ESC '['`/`ESC ']'`）だけでなく、同じ意味を持つ
  8bit C1 制御コード単体（`0x9B` = CSI導入符号, `0x9D` = OSC導入符号）にも
  対応する。`strip_ansi` 内部の走査ロジックは `skip_csi`/`skip_osc` として
  共通化し、どちらの導入符号からも同じ規則でシーケンス全体を除去する。
  CSI は `CSI_PARAM_LIMIT = 16` 文字、OSC は `OSC_PARAM_LIMIT = 512` 文字を
  それぞれ上限としてスキャンを打ち切る（OSC 8 ハイパーリンクのURI等、
  正当なペイロードはCSIより長くなりうるため上限を分けている）。終端の
  未終端導入符号が入力中に多数連続していても、各スキャンが上限で
  打ち切られることで全体の計算量は O(n) に保たれる（上限を設ける前は、
  未終端の導入符号が繰り返されるたびに毎回残り長さ全体を走査してしまい
  O(n²) となっていた）。
- `--due` は `src/validate.rs::validate_due_date` により
  `YYYY-MM-DD`（`chrono::NaiveDate`）または RFC3339
  （`chrono::DateTime::parse_from_rfc3339`）のいずれかの形式のみ許容する。
- `db::update_task` は事前の存在チェックを行わない。UPDATE文自体が
  存在しない ID に対して安全に 0 行影響で完了し、関数末尾の `get_task`
  が最終状態を正しく返すため、事前チェックは不要かつ冗長だった。
- `sanitize::strip_ansi` は ANSI/C1 エスケープシーケンスおよび Unicode
  制御文字 (`Cc`) に加え、双方向書式文字 (`Cf` の一部: U+202A-U+202E,
  U+2066-U+2069, U+200E, U+200F) も除去する（`src/sanitize.rs::is_bidi_control`）。
  これらは `char::is_control()` の対象外であるため別途チェックが必要で、
  `description` を含む全フィールドで無条件に除去する（`\n`/`\t` のような
  許容例外は設けない）。
- DB ディレクトリは `Path::exists()`（シンボリックリンクを解決してしまう）
  ではなく `symlink_metadata` で存在確認し、対象がシンボリックリンクで
  あれば無条件に拒否する。既存の実ディレクトリを尊重する場合も、Unix では
  所有者 uid が実行ユーザーと一致するかを確認し、一致しなければ拒否する
  （`src/db.rs::check_dir_is_trustworthy`）。`create_dir_owner_only` の
  `AlreadyExists` 許容（複数の自プロセスによる同一新規ディレクトリ作成の
  競合を許容するため）についても、作成後に改めてこのチェックを通すことで、
  その隙間に第三者がシンボリックリンクを割り込ませるレースを防いでいる。
  新規作成される `tasks.db` 本体も、作成時のみ `0600` に制限する
  （多層防御。既存ファイルの権限は変更しない）。
- `tag` フィルタは検索側の入力にも格納側と同じ空白除去正規化
  (`tag.trim().replace(' ', "")`) を適用してから `escape_like` に渡す。
  格納側は `REPLACE(tags, ' ', '')` で空白を除去した値と比較するため、
  検索側だけ空白を保持したままだと "backend" と "back end" が
  一貫しない結果になっていた。
- `update` は `title`/`description`/`status`/`priority`/`assigned`/`tags`/
  `due` のいずれも指定されていない場合、明確なエラーで拒否する
  （`db::update_task` は常に `updated_at` を更新するため、フィールド指定
  なしの呼び出しでも無条件に「成功」してしまい、呼び出し側のミスが
  気付かれなくなることを防ぐため）。
- `--due` は `validate::has_four_digit_year` により、年部分が正確に4桁の
  ASCII数字であることを `chrono` に渡す前に検証する。`chrono` の
  `%Y`/RFC3339 パーサーは年の桁数を厳密に制限しておらず、5桁以上の年で
  内部の日付計算が整数オーバーフローを起こす。オーバーフローチェックは
  デバッグビルドでのみ有効なため、この事前検証がないと `cargo test`
  (デバッグ) とリリースビルド(配布物)とで `--due` の受理/拒否が食い違う
  ことを実測で確認した。
