use unicode_width::UnicodeWidthStr;

use crate::color;
use crate::models::{Priority, Status, Task};

fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Pad `text` (optionally wrapped in an ANSI color) to `width` visible columns.
fn cell(text: &str, width: usize, color_code: Option<&str>) -> String {
    let visible = display_width(text);
    let pad = " ".repeat(width.saturating_sub(visible));
    match color_code {
        Some(c) if color::enabled() => format!("{c}{text}{}{pad}", color::RESET),
        _ => format!("{text}{pad}"),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if display_width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in text.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

fn status_status_str(status: &str) -> (String, &'static str) {
    match Status::parse(status) {
        Ok(s) => (format!("{} {}", s.emoji(), s.as_str()), color::RESET),
        Err(_) => (status.to_string(), color::RESET),
    }
}

fn priority_color(priority: &str) -> &'static str {
    Priority::parse(priority)
        .map(|p| p.ansi_color())
        .unwrap_or(color::RESET)
}

/// Render a colored, column-aligned table of `tasks` to stdout.
pub fn print_table(tasks: &[Task]) {
    if tasks.is_empty() {
        println!(
            "{}",
            color::paint(color::GRAY, "タスクが見つかりませんでした。")
        );
        return;
    }

    const TITLE_MAX: usize = 40;

    let id_w = tasks
        .iter()
        .map(|t| t.id.to_string().len())
        .max()
        .unwrap_or(2)
        .max(2);
    let status_w = tasks
        .iter()
        .map(|t| display_width(&status_status_str(&t.status).0))
        .max()
        .unwrap_or(6)
        .max(6);
    let priority_w = tasks
        .iter()
        .map(|t| display_width(&t.priority))
        .max()
        .unwrap_or(8)
        .max(8);
    let title_w = tasks
        .iter()
        .map(|t| display_width(&truncate(&t.title, TITLE_MAX)))
        .max()
        .unwrap_or(5)
        .max(5);
    let assigned_w = tasks
        .iter()
        .map(|t| display_width(t.assigned_agent.as_deref().unwrap_or("-")))
        .max()
        .unwrap_or(8)
        .max(8);
    let tags_w = tasks
        .iter()
        .map(|t| display_width(t.tags.as_deref().unwrap_or("-")))
        .max()
        .unwrap_or(4)
        .max(4);
    let due_w = tasks
        .iter()
        .map(|t| display_width(t.due_date.as_deref().unwrap_or("-")))
        .max()
        .unwrap_or(10)
        .max(10);

    let header = format!(
        "{}  {}  {}  {}  {}  {}  {}",
        cell("ID", id_w, Some(color::BOLD)),
        cell("STATUS", status_w, Some(color::BOLD)),
        cell("PRIORITY", priority_w, Some(color::BOLD)),
        cell("TITLE", title_w, Some(color::BOLD)),
        cell("ASSIGNED", assigned_w, Some(color::BOLD)),
        cell("TAGS", tags_w, Some(color::BOLD)),
        cell("DUE", due_w, Some(color::BOLD)),
    );
    println!("{header}");

    for t in tasks {
        let (status_disp, _) = status_status_str(&t.status);
        let title_disp = truncate(&t.title, TITLE_MAX);
        let assigned = t.assigned_agent.clone().unwrap_or_else(|| "-".to_string());
        let tags = t.tags.clone().unwrap_or_else(|| "-".to_string());
        let due = t.due_date.clone().unwrap_or_else(|| "-".to_string());

        let line = format!(
            "{}  {}  {}  {}  {}  {}  {}",
            cell(&t.id.to_string(), id_w, None),
            cell(&status_disp, status_w, None),
            cell(&t.priority, priority_w, Some(priority_color(&t.priority))),
            cell(&title_disp, title_w, None),
            cell(&assigned, assigned_w, Some(color::CYAN)),
            cell(&tags, tags_w, Some(color::GRAY)),
            cell(&due, due_w, None),
        );
        println!("{line}");
    }
}

/// Render a single task's full detail (multi-line, colored) to stdout.
pub fn print_detail(task: &Task) {
    let (status_disp, _) = status_status_str(&task.status);
    println!("{}: {}", color::paint(color::BOLD, "ID"), task.id);
    println!("{}: {}", color::paint(color::BOLD, "Title"), task.title);
    println!("{}: {}", color::paint(color::BOLD, "Status"), status_disp);
    println!(
        "{}: {}",
        color::paint(color::BOLD, "Priority"),
        color::paint(priority_color(&task.priority), &task.priority)
    );
    println!(
        "{}: {}",
        color::paint(color::BOLD, "Assigned"),
        task.assigned_agent.as_deref().unwrap_or("-")
    );
    println!(
        "{}: {}",
        color::paint(color::BOLD, "Tags"),
        task.tags.as_deref().unwrap_or("-")
    );
    println!(
        "{}: {}",
        color::paint(color::BOLD, "Due"),
        task.due_date.as_deref().unwrap_or("-")
    );
    println!(
        "{}: {}",
        color::paint(color::BOLD, "Created"),
        task.created_at
    );
    println!(
        "{}: {}",
        color::paint(color::BOLD, "Updated"),
        task.updated_at
    );
    println!(
        "{}: {}",
        color::paint(color::BOLD, "Description"),
        task.description.as_deref().unwrap_or("-")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_text_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_text_adds_ellipsis() {
        let t = truncate("this is a very long title indeed", 10);
        assert!(display_width(&t) <= 10);
        assert!(t.ends_with('…'));
    }
}
