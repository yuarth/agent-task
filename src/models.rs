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
                bail!("無効な status です: '{other}' (有効値: todo, in_progress, done, blocked)")
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
            other => bail!("無効な priority です: '{other}' (有効値: low, medium, high, urgent)"),
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
