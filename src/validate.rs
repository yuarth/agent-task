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
use unicode_width::UnicodeWidthStr;

pub const MAX_TITLE_CHARS: usize = 500;
pub const MAX_DESCRIPTION_CHARS: usize = 20_000;
/// Applies to assigned_agent / tags / due_date — short, single-line fields.
pub const MAX_SHORT_FIELD_CHARS: usize = 300;

/// A title is rejected if it renders with zero visible terminal columns.
///
/// `str::trim()` alone isn't enough: it only strips *leading/trailing*
/// `White_Space`-property characters, so a title made entirely of e.g.
/// U+200B ZERO WIDTH SPACE survives `.trim()` non-empty while still
/// rendering as a blank cell in `list`'s table — exactly the
/// "indistinguishable task" problem this check exists to prevent, just via
/// a different kind of invisible character.
///
/// Filtering *every* whitespace character (not just a leading/trailing
/// run) before measuring width closes a further bypass of the naive
/// `width(title.trim()) == 0` version of this check: a title like
/// `"  \u{200b}  \u{200b}  "` interleaves zero-width characters between
/// plain spaces so `.trim()` stops at the first non-whitespace (zero-width)
/// character from each edge, leaving interior plain spaces un-trimmed —
/// those have nonzero display width on their own and would make the old
/// check accept a title that still renders as an entirely blank cell.
/// Stripping whitespace throughout (not just at the edges) before the width
/// check removes that loophole while still correctly accepting ordinary
/// titles with interior spaces ("hello world" has nonzero width from its
/// letters once whitespace is stripped, same as before).
pub fn require_non_empty_title(title: &str) -> Result<()> {
    let visible: String = title.chars().filter(|c| !c.is_whitespace()).collect();
    if UnicodeWidthStr::width(visible.as_str()) == 0 {
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
    fn zero_width_space_only_title_is_rejected() {
        // Regression test found in adversarial review: a title made only of
        // U+200B survives `.trim()` (it has no Unicode White_Space
        // property) but is fully invisible, defeating the point of this
        // check.
        assert!(require_non_empty_title("\u{200b}\u{200b}\u{200b}").is_err());
    }

    #[test]
    fn zero_width_space_surrounded_by_visible_text_is_accepted() {
        // Zero-width characters mixed into an otherwise visible title
        // shouldn't be rejected outright — only titles with zero total
        // display width are.
        assert!(require_non_empty_title("a\u{200b}b").is_ok());
    }

    #[test]
    fn zero_width_spaces_interleaved_with_plain_spaces_are_rejected() {
        // Regression test found in a second round of adversarial review: an
        // earlier version of this check computed width(title.trim()), which
        // only strips a *leading/trailing run* of whitespace. Interleaving
        // zero-width characters between plain spaces (e.g.
        // "  \u{200b}  \u{200b}  ") plants a non-whitespace character right
        // at each edge, so `.trim()` stops immediately and the interior
        // plain spaces survive untrimmed -- those have nonzero width on
        // their own, so the old check wrongly accepted a title that still
        // renders as an entirely blank cell in `list`. Stripping whitespace
        // throughout (not just at the edges) before measuring width closes
        // this.
        assert!(require_non_empty_title("  \u{200b}  \u{200b}  ").is_err());
        assert!(require_non_empty_title("\u{200b} \u{200b}").is_err());
        assert!(require_non_empty_title(" \u{200b} \u{200b} ").is_err());
    }

    #[test]
    fn interior_spaces_around_visible_text_are_still_accepted() {
        // The interleaved-whitespace fix must not start rejecting ordinary
        // titles with legitimate interior spaces.
        assert!(require_non_empty_title("hello world").is_ok());
        assert!(require_non_empty_title("  a  b  ").is_ok());
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
