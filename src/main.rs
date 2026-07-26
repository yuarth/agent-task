mod cli;
mod color;
mod db;
mod models;
mod output;
mod sanitize;
mod validate;

use anyhow::{anyhow, Result};
use clap::Parser;
use rusqlite::Connection;

use cli::{AddArgs, Cli, Commands, CompleteArgs, DeleteArgs, ListArgs, ShowArgs, UpdateArgs};
use db::ListFilter;
use models::{Priority, Status, Task};

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn open_default_connection() -> Result<Connection> {
    let path = db::resolve_db_path()?;
    db::open_db(&path)
}

fn print_task_json(task: &Task) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(task)?);
    Ok(())
}

fn run_add(conn: &Connection, args: AddArgs) -> Result<()> {
    let status = Status::parse(&args.status)?;
    let priority = Priority::parse(&args.priority)?;
    let ts = now();

    let title = sanitize::clean_line(&args.title);
    validate::require_non_empty_title(&title)?;
    validate::check_max_len(&title, "title", validate::MAX_TITLE_CHARS)?;

    let description = args.description.as_deref().map(sanitize::clean_multiline);
    if let Some(d) = &description {
        validate::check_max_len(d, "description", validate::MAX_DESCRIPTION_CHARS)?;
    }
    let assigned = args.assigned.as_deref().map(sanitize::clean_line);
    if let Some(a) = &assigned {
        validate::check_max_len(a, "assigned", validate::MAX_SHORT_FIELD_CHARS)?;
    }
    let tags = args.tags.as_deref().map(sanitize::clean_line);
    if let Some(t) = &tags {
        validate::check_max_len(t, "tags", validate::MAX_SHORT_FIELD_CHARS)?;
    }
    let due = args.due.as_deref().map(sanitize::clean_line);
    if let Some(d) = &due {
        validate::check_max_len(d, "due", validate::MAX_SHORT_FIELD_CHARS)?;
        validate::validate_due_date(d)?;
    }

    let task = db::add_task(
        conn,
        &title,
        description.as_deref(),
        status.as_str(),
        priority.as_str(),
        assigned.as_deref(),
        tags.as_deref(),
        due.as_deref(),
        &ts,
    )?;

    println!(
        "{} タスクを追加しました (ID: {})",
        color::paint(color::CYAN, "✔"),
        task.id
    );
    output::print_detail(&task);
    Ok(())
}

fn run_list(conn: &Connection, args: ListArgs) -> Result<()> {
    if let Some(s) = &args.status {
        Status::parse(s)?;
    }
    if let Some(p) = &args.priority {
        Priority::parse(p)?;
    }

    let filter = ListFilter {
        status: args.status,
        assigned: args.assigned,
        priority: args.priority,
        tag: args.tag,
        all: args.all,
    };
    let tasks = db::list_tasks(conn, &filter)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&tasks)?);
    } else {
        output::print_table(&tasks);
    }
    Ok(())
}

fn run_show(conn: &Connection, args: ShowArgs) -> Result<()> {
    let task = db::get_task(conn, args.id)?
        .ok_or_else(|| anyhow!("タスクが見つかりません (ID: {})", args.id))?;

    if args.json {
        print_task_json(&task)?;
    } else {
        output::print_detail(&task);
    }
    Ok(())
}

fn run_update(conn: &Connection, args: UpdateArgs) -> Result<()> {
    if let Some(s) = &args.status {
        Status::parse(s)?;
    }
    if let Some(p) = &args.priority {
        Priority::parse(p)?;
    }

    let ts = now();

    let title = args.title.as_deref().map(sanitize::clean_line);
    if let Some(t) = &title {
        validate::require_non_empty_title(t)?;
        validate::check_max_len(t, "title", validate::MAX_TITLE_CHARS)?;
    }
    let description = args.description.as_deref().map(sanitize::clean_multiline);
    if let Some(d) = &description {
        validate::check_max_len(d, "description", validate::MAX_DESCRIPTION_CHARS)?;
    }
    let assigned = args.assigned.as_deref().map(sanitize::clean_line);
    if let Some(a) = &assigned {
        validate::check_max_len(a, "assigned", validate::MAX_SHORT_FIELD_CHARS)?;
    }
    let tags = args.tags.as_deref().map(sanitize::clean_line);
    if let Some(t) = &tags {
        validate::check_max_len(t, "tags", validate::MAX_SHORT_FIELD_CHARS)?;
    }
    let due = args.due.as_deref().map(sanitize::clean_line);
    if let Some(d) = &due {
        validate::check_max_len(d, "due", validate::MAX_SHORT_FIELD_CHARS)?;
        validate::validate_due_date(d)?;
    }

    let updated = db::update_task(
        conn,
        args.id,
        title.as_deref(),
        description.as_deref(),
        args.status.as_deref(),
        args.priority.as_deref(),
        assigned.as_deref(),
        tags.as_deref(),
        due.as_deref(),
        &ts,
    )?
    .ok_or_else(|| anyhow!("タスクが見つかりません (ID: {})", args.id))?;

    println!(
        "{} タスクを更新しました (ID: {})",
        color::paint(color::CYAN, "✔"),
        updated.id
    );
    output::print_detail(&updated);
    Ok(())
}

fn run_complete(conn: &Connection, args: CompleteArgs) -> Result<()> {
    let ts = now();
    let task = db::complete_task(conn, args.id, &ts)?
        .ok_or_else(|| anyhow!("タスクが見つかりません (ID: {})", args.id))?;

    println!(
        "{} タスクを完了しました (ID: {})",
        color::paint(color::CYAN, "✔"),
        task.id
    );
    output::print_detail(&task);
    Ok(())
}

fn run_delete(conn: &Connection, args: DeleteArgs) -> Result<()> {
    let deleted = db::delete_task(conn, args.id)?;
    if !deleted {
        return Err(anyhow!("タスクが見つかりません (ID: {})", args.id));
    }
    println!(
        "{} タスクを削除しました (ID: {})",
        color::paint(color::CYAN, "✔"),
        args.id
    );
    Ok(())
}

fn main() {
    let cli = Cli::parse();

    let result = (|| -> Result<()> {
        let conn = open_default_connection()?;
        match cli.command {
            Commands::Add(args) => run_add(&conn, args),
            Commands::List(args) => run_list(&conn, args),
            Commands::Show(args) => run_show(&conn, args),
            Commands::Update(args) => run_update(&conn, args),
            Commands::Complete(args) => run_complete(&conn, args),
            Commands::Delete(args) => run_delete(&conn, args),
        }
    })();

    if let Err(err) = result {
        eprintln!("{} {err}", color::paint(color::RED, "エラー:"));
        std::process::exit(1);
    }
}
