//! Input-size and non-emptiness validation applied to user-supplied task
//! fields before they reach the database.
//!
//! `sanitize` strips dangerous/invisible bytes (ANSI escapes, control
//! characters) but deliberately does not judge length or emptiness — that is
//! a distinct concern, handled here, so each module stays independently
//! testable. Without this check, an unbounded `--assigned`/`--tags`/`--due`
//! value blows up `list`'s table column widths (every row pads to the
//! longest value), and an unbounded `title`/`description` lets a single task
//! grow the SQLite file without limit. An empty/whitespace-only title is
//! also rejected: it produces a task nothing can usefully identify in
//! `list`'s table view.

use anyhow::{bail, Result};

pub const MAX_TITLE_CHARS: usize = 500;
pub const MAX_DESCRIPTION_CHARS: usize = 20_000;
/// Applies to assigned_agent / tags / due_date — short, single-line fields.
pub const MAX_SHORT_FIELD_CHARS: usize = 300;

pub fn require_non_empty_title(title: &str) -> Result<()> {
    if title.trim().is_empty() {
        bail!("title を空にすることはできません");
    }
    Ok(())
}

pub fn check_max_len(value: &str, field_name: &str, max_chars: usize) -> Result<()> {
    let len = value.chars().count();
    if len > max_chars {
        bail!("{field_name} が長すぎます ({len} 文字, 上限 {max_chars} 文字)");
    }
    Ok(())
}

/// Validate that `value` is a `YYYY-MM-DD` date or an RFC3339 timestamp, as
/// documented for `--due` in the CLI help/README. Without this, any string
/// (e.g. a typo, or free-text unrelated to a date) is silently accepted and
/// stored, only to fail or be silently ignored by future date-aware features
/// (sorting, overdue detection, ...) that read it back.
pub fn validate_due_date(value: &str) -> Result<()> {
    let is_plain_date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok();
    let is_rfc3339 = chrono::DateTime::parse_from_rfc3339(value).is_ok();
    if is_plain_date || is_rfc3339 {
        return Ok(());
    }
    let safe = crate::sanitize::sanitize_for_message(value);
    bail!("無効な due です: '{safe}' (有効形式: YYYY-MM-DD または RFC3339)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_title_is_rejected() {
        assert!(require_non_empty_title("").is_err());
    }

    #[test]
    fn whitespace_only_title_is_rejected() {
        assert!(require_non_empty_title("   \t  ").is_err());
    }

    #[test]
    fn non_empty_title_is_accepted() {
        assert!(require_non_empty_title("普通のタイトル").is_ok());
    }

    #[test]
    fn value_within_limit_is_accepted() {
        let s = "a".repeat(MAX_SHORT_FIELD_CHARS);
        assert!(check_max_len(&s, "tags", MAX_SHORT_FIELD_CHARS).is_ok());
    }

    #[test]
    fn value_over_limit_is_rejected() {
        let s = "a".repeat(MAX_SHORT_FIELD_CHARS + 1);
        let err = check_max_len(&s, "tags", MAX_SHORT_FIELD_CHARS)
            .unwrap_err()
            .to_string();
        assert!(err.contains("tags"));
    }

    #[test]
    fn check_max_len_counts_chars_not_bytes() {
        // Multi-byte (Japanese) characters must be counted as one character
        // each, not by UTF-8 byte length, so a limit isn't unfairly stricter
        // for non-ASCII text.
        let s = "あ".repeat(MAX_SHORT_FIELD_CHARS);
        assert!(check_max_len(&s, "tags", MAX_SHORT_FIELD_CHARS).is_ok());
    }

    #[test]
    fn due_date_accepts_plain_date() {
        assert!(validate_due_date("2026-08-01").is_ok());
    }

    #[test]
    fn due_date_accepts_rfc3339() {
        assert!(validate_due_date("2026-08-01T12:00:00+09:00").is_ok());
    }

    #[test]
    fn due_date_rejects_garbage() {
        let err = validate_due_date("らくだ").unwrap_err().to_string();
        assert!(err.contains("無効な due"));
    }

    #[test]
    fn due_date_rejects_invalid_calendar_date() {
        assert!(validate_due_date("2026-13-99").is_err());
    }

    #[test]
    fn due_date_error_message_strips_ansi_from_echoed_value() {
        let malicious = "\x1b[31mnope\x1b[0m";
        let err = validate_due_date(malicious).unwrap_err().to_string();
        assert!(!err.contains('\u{1b}'));
    }
}
