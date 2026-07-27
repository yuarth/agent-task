use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Task lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    Done,
    Blocked,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::Done => "done",
            Status::Blocked => "blocked",
        }
    }

    /// Emoji marker used in table/detail output, per spec:
    /// 🟢in_progress / 🔵done / 🟡todo / 🔴blocked
    pub fn emoji(&self) -> &'static str {
        match self {
            Status::Todo => "🟡",
            Status::InProgress => "🟢",
            Status::Done => "🔵",
            Status::Blocked => "🔴",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "todo" => Ok(Status::Todo),
            "in_progress" => Ok(Status::InProgress),
            "done" => Ok(Status::Done),
            "blocked" => Ok(Status::Blocked),
            other => {
                let safe = crate::sanitize::sanitize_for_message(other);
                bail!("無効な status です: '{safe}' (有効値: todo, in_progress, done, blocked)")
            }
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Task priority level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "low" => Ok(Priority::Low),
            "medium" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            "urgent" => Ok(Priority::Urgent),
            other => {
                let safe = crate::sanitize::sanitize_for_message(other);
                bail!("無効な priority です: '{safe}' (有効値: low, medium, high, urgent)")
            }
        }
    }

    /// ANSI color code applied to the priority label for readability.
    pub fn ansi_color(&self) -> &'static str {
        match self {
            Priority::Low => crate::color::DIM,
            Priority::Medium => crate::color::CYAN,
            Priority::High => crate::color::YELLOW,
            Priority::Urgent => crate::color::BOLD_RED,
        }
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A single task row as stored in / read from SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub priority: String,
    pub assigned_agent: Option<String>,
    pub tags: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub due_date: Option<String>,
}

impl Task {
    pub fn tag_list(&self) -> Vec<String> {
        match &self.tags {
            Some(t) if !t.trim().is_empty() => t
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tag_list().iter().any(|t| t.eq_ignore_ascii_case(tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parse_error_strips_ansi_from_echoed_value() {
        // A malicious/garbled --status value must not leak raw ANSI/OSC
        // escape sequences into the error text printed to the terminal.
        let malicious = "\x1b]8;;http://evil.example\x07click\x1b]8;;\x07me";
        let err = Status::parse(malicious).unwrap_err().to_string();
        assert!(
            !err.contains('\u{1b}'),
            "error message must not contain ESC: {err}"
        );
        assert!(err.contains("clickme"));
    }

    #[test]
    fn priority_parse_error_strips_ansi_from_echoed_value() {
        let malicious = "\x1b[31mnope\x1b[0m";
        let err = Priority::parse(malicious).unwrap_err().to_string();
        assert!(
            !err.contains('\u{1b}'),
            "error message must not contain ESC: {err}"
        );
        assert!(err.contains("nope"));
    }

    #[test]
    fn status_parse_error_truncates_overlong_value() {
        let long = "x".repeat(500);
        let err = Status::parse(&long).unwrap_err().to_string();
        // Bounded regardless of how long the invalid input was.
        assert!(
            err.len() < 300,
            "error message should be bounded: {} bytes",
            err.len()
        );
        assert!(err.contains('…'));
    }

    #[test]
    fn status_parse_valid_values_roundtrip() {
        for s in ["todo", "in_progress", "done", "blocked"] {
            assert_eq!(Status::parse(s).unwrap().as_str(), s);
        }
    }

    #[test]
    fn priority_parse_valid_values_roundtrip() {
        for p in ["low", "medium", "high", "urgent"] {
            assert_eq!(Priority::parse(p).unwrap().as_str(), p);
        }
    }

    #[test]
    fn task_tag_list_splits_trims_and_drops_empties() {
        let mut t = sample_task();
        t.tags = Some(" backend , , bugfix ,".to_string());
        assert_eq!(t.tag_list(), vec!["backend", "bugfix"]);
    }

    #[test]
    fn task_tag_list_none_is_empty() {
        let mut t = sample_task();
        t.tags = None;
        assert!(t.tag_list().is_empty());
    }

    #[test]
    fn task_has_tag_is_case_insensitive() {
        let mut t = sample_task();
        t.tags = Some("Backend,BugFix".to_string());
        assert!(t.has_tag("backend"));
        assert!(t.has_tag("BUGFIX"));
        assert!(!t.has_tag("frontend"));
    }

    fn sample_task() -> Task {
        Task {
            id: 1,
            title: "T".to_string(),
            description: None,
            status: "todo".to_string(),
            priority: "medium".to_string(),
            assigned_agent: None,
            tags: None,
            created_at: "2026-07-26T00:00:00Z".to_string(),
            updated_at: "2026-07-26T00:00:00Z".to_string(),
            due_date: None,
        }
    }
}
