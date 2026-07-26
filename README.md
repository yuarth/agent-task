# agent-task

複数プロセスの AI エージェント（Claude Code, Antigravity, カスタムスクリプト等）
および人間が、タスクの登録・状態更新・割り当て・進捗参照を高速かつ堅牢に行う
ための Rust 製ネイティブ CLI ツールです。

バックエンド DB には SQLite を採用し、WAL モードおよびビジータイムアウトを
設定することで、複数プロセス/エージェントからの同時読み書きに対するロック
衝突を防止します。

## インストール / ビルド

```bash
# cargo
cargo build --release
./target/release/agent-task --help

# Nix (チャンネル版)
nix-build default.nix
./result/bin/agent-task --help

# Nix (flake 版)
nix build .#default
./result/bin/agent-task --help
```

## データベース

デフォルトの保存先: `~/.local/share/agent-task/tasks.db`

環境変数 `AGENT_TASK_DB` でパスを上書きできます（テストや複数環境の分離用）。

```bash
export AGENT_TASK_DB=/tmp/my-tasks.db
```

## コマンド

```bash
# 追加
agent-task add "タイトル" \
  --description "詳細説明" \
  --status todo \
  --priority high \
  --assigned claude \
  --tags "backend,bugfix" \
  --due 2026-08-01

# 一覧 (デフォルトは done を除く未完了タスクのみ)
agent-task list
agent-task list --all
agent-task list --status in_progress --assigned claude
agent-task list --tag backend --json

# 詳細表示
agent-task show 1
agent-task show 1 --json

# 更新 (指定したフィールドのみ変更)
agent-task update 1 --status in_progress --priority urgent

# 完了ショートカット
agent-task complete 1

# 削除
agent-task delete 1
```

## 開発

```bash
cargo test              # 単体 + 結合テスト
cargo clippy --all-targets -- -D warnings
cargo fmt
```

検証結果の詳細は [`qa_matrix.md`](./qa_matrix.md) を参照してください。
