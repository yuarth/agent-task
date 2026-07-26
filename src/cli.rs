use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "agent-task",
    version,
    about = "AI エージェント向け SQLite タスク管理 CLI",
    long_about = "複数プロセスの AI エージェントおよび人間が、タスクの登録・状態更新・割り当て・進捗参照を\n高速かつ堅牢に行うための CLI ツールです。"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// タスクを新規追加する
    Add(AddArgs),
    /// タスクを一覧・検索する
    List(ListArgs),
    /// タスクの詳細を表示する
    Show(ShowArgs),
    /// タスクを更新する
    Update(UpdateArgs),
    /// タスクを完了状態にする (status=done のショートカット)
    Complete(CompleteArgs),
    /// タスクを削除する
    Delete(DeleteArgs),
}

#[derive(Parser, Debug)]
pub struct AddArgs {
    /// タスクのタイトル
    pub title: String,

    /// 詳細説明
    #[arg(long)]
    pub description: Option<String>,

    /// ステータス (todo|in_progress|done|blocked)
    #[arg(long, default_value = "todo")]
    pub status: String,

    /// 優先度 (low|medium|high|urgent)
    #[arg(long, default_value = "medium")]
    pub priority: String,

    /// 割り当てエージェント名
    #[arg(long)]
    pub assigned: Option<String>,

    /// カンマ区切りタグ (例: "backend,bugfix")
    #[arg(long)]
    pub tags: Option<String>,

    /// 期限 (YYYY-MM-DD または RFC3339)
    #[arg(long)]
    pub due: Option<String>,
}

#[derive(Parser, Debug)]
pub struct ListArgs {
    /// ステータスで絞り込み
    #[arg(long)]
    pub status: Option<String>,

    /// 割り当てエージェントで絞り込み
    #[arg(long)]
    pub assigned: Option<String>,

    /// 優先度で絞り込み
    #[arg(long)]
    pub priority: Option<String>,

    /// タグで絞り込み
    #[arg(long)]
    pub tag: Option<String>,

    /// 完了 (done) タスクも含めて全件表示する
    #[arg(long)]
    pub all: bool,

    /// JSON 形式で出力する
    #[arg(long)]
    pub json: bool,
}

#[derive(Parser, Debug)]
pub struct ShowArgs {
    /// タスク ID
    pub id: i64,

    /// JSON 形式で出力する
    #[arg(long)]
    pub json: bool,
}

#[derive(Parser, Debug)]
pub struct UpdateArgs {
    /// タスク ID
    pub id: i64,

    /// 新タイトル
    #[arg(long)]
    pub title: Option<String>,

    /// 新説明
    #[arg(long)]
    pub description: Option<String>,

    /// 新ステータス (todo|in_progress|done|blocked)
    #[arg(long)]
    pub status: Option<String>,

    /// 新優先度 (low|medium|high|urgent)
    #[arg(long)]
    pub priority: Option<String>,

    /// 新しい割り当てエージェント名
    #[arg(long)]
    pub assigned: Option<String>,

    /// 新タグ (カンマ区切り)
    #[arg(long)]
    pub tags: Option<String>,

    /// 新期限
    #[arg(long)]
    pub due: Option<String>,
}

#[derive(Parser, Debug)]
pub struct CompleteArgs {
    /// タスク ID
    pub id: i64,
}

#[derive(Parser, Debug)]
pub struct DeleteArgs {
    /// タスク ID
    pub id: i64,
}
