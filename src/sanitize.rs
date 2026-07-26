//! Strips terminal control characters and ANSI/VT100 escape sequences from
//! user-supplied text *before it is stored*, so no value read back out of
//! the database can later manipulate the terminal when rendered by
//! `list`/`show` (cursor movement, screen clears, hidden-text tricks, etc.).
//!
//! A naive filter that only drops the ESC (0x1B) byte would leave the rest
//! of a CSI/OSC sequence (e.g. the `[31m` in `ESC[31m`) behind as visible
//! junk, so this recognizes and drops whole sequences, in both their 7-bit
//! (ESC-prefixed) and 8-bit (single C1 control code) forms:
//! - CSI: `ESC '[' params... final-byte` (final byte in `0x40..=0x7E`), or
//!   the single-byte C1 introducer `0x9B` in place of `ESC '['`
//! - OSC: `ESC ']' ... (BEL | ESC '\\')`, or the single-byte C1 introducer
//!   `0x9D` in place of `ESC ']'`
//! - other two-byte escapes: `ESC <any char>`

/// Max characters scanned after a CSI introducer while looking for a CSI
/// final byte (0x40..=0x7E). Real CSI sequences are always short (a handful
/// of parameter/intermediate bytes); without a cap, a malformed/malicious
/// CSI payload with no final byte would let the scan run to the end of the
/// input, silently swallowing everything after it. If no final byte turns up
/// within the cap, the sequence is not treated as CSI: only the introducer
/// itself is dropped, and scanning resumes from the very next character.
const CSI_PARAM_LIMIT: usize = 16;

/// Scan for a CSI final byte starting at `params_start` (the position right
/// after the introducer, whether that was `ESC '['` or the single-byte `0x9B`).
/// Returns the index just past the whole sequence if a well-formed one is
/// found within [`CSI_PARAM_LIMIT`], or `None` otherwise (caller then drops
/// only the introducer and keeps scanning).
fn skip_csi(chars: &[char], params_start: usize, n: usize) -> Option<usize> {
    let scan_end = (params_start + CSI_PARAM_LIMIT).min(n);
    (params_start..scan_end)
        .find(|&j| ('\x40'..='\x7e').contains(&chars[j]))
        .map(|j| j + 1)
}

/// Scan for an OSC terminator (BEL, or ESC '\\' i.e. ST) starting at
/// `params_start`. Returns the index just past the whole sequence if found,
/// or `None` if unterminated (caller then drops only the introducer).
fn skip_osc(chars: &[char], params_start: usize, n: usize) -> Option<usize> {
    let mut j = params_start;
    while j < n {
        if chars[j] == '\u{7}' {
            return Some(j + 1);
        }
        if chars[j] == '\u{1b}' && chars.get(j + 1) == Some(&'\\') {
            return Some(j + 2);
        }
        j += 1;
    }
    None
}

fn strip_ansi<F: Fn(char) -> bool>(input: &str, keep_control: F) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < n {
        let c = chars[i];

        if c == '\u{1b}' && chars.get(i + 1) == Some(&'[') {
            i = skip_csi(&chars, i + 2, n).unwrap_or(i + 1);
            continue;
        }
        if c == '\u{9b}' {
            // 8-bit C1 CSI introducer: equivalent to `ESC '['`.
            i = skip_csi(&chars, i + 1, n).unwrap_or(i + 1);
            continue;
        }

        if c == '\u{1b}' && chars.get(i + 1) == Some(&']') {
            i = skip_osc(&chars, i + 2, n).unwrap_or(i + 1);
            continue;
        }
        if c == '\u{9d}' {
            // 8-bit C1 OSC introducer: equivalent to `ESC ']'`.
            i = skip_osc(&chars, i + 1, n).unwrap_or(i + 1);
            continue;
        }

        if c == '\u{1b}' {
            // Generic two-byte escape (ESC + one char), or a lone trailing ESC.
            i += if i + 1 < n { 2 } else { 1 };
            continue;
        }

        if c.is_control() && !keep_control(c) {
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }

    out
}

/// For single-line fields (title, assigned_agent, tags, due_date): strip
/// ANSI/C1 escape sequences (both `ESC`-prefixed and single-byte forms)
/// plus every remaining Unicode control character (including `\n`/`\r`/`\t`
/// and any other stray C1 byte in 0x80-0x9F not already consumed as part of
/// a CSI/OSC sequence).
pub fn clean_line(input: &str) -> String {
    strip_ansi(input, |_| false)
}

/// For multi-line fields (description): same as [`clean_line`] but keeps
/// `\n` and `\t` so legitimate multi-line notes survive.
pub fn clean_multiline(input: &str) -> String {
    strip_ansi(input, |c| c == '\n' || c == '\t')
}

/// Max characters kept when echoing untrusted input back into a
/// human-readable message (see [`sanitize_for_message`]).
const MESSAGE_ECHO_LIMIT: usize = 80;

/// Make untrusted input safe to embed in a message printed to the terminal
/// (e.g. a CLI validation error that echoes back the invalid value).
///
/// Unlike [`clean_line`], which sanitizes values *before they are stored* in
/// the database, this covers a different path: input that is rejected
/// up-front (never stored) but still gets echoed into an error string and
/// printed directly to stderr. Without this, an invalid `--status`/`--priority`
/// value containing ANSI/OSC escape sequences would be written to the
/// terminal verbatim, e.g. `agent-task add T --status $'\x1b]8;;http://evil\x07x'`.
/// Also bounds the length so a pathologically long argument can't produce an
/// unbounded error line.
pub fn sanitize_for_message(input: &str) -> String {
    let cleaned = clean_line(input);
    if cleaned.chars().count() <= MESSAGE_ECHO_LIMIT {
        return cleaned;
    }
    let mut out: String = cleaned.chars().take(MESSAGE_ECHO_LIMIT).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_line_strips_csi_color_sequences() {
        let malicious = "\x1b[31mRed\x1b[0m\x07Bell";
        assert_eq!(clean_line(malicious), "RedBell");
    }

    #[test]
    fn clean_line_strips_cursor_and_clear_sequences() {
        // e.g. move cursor, clear screen — common terminal-hijack payloads.
        let malicious = "\x1b[2J\x1b[Hpwned";
        assert_eq!(clean_line(malicious), "pwned");
    }

    #[test]
    fn clean_line_strips_osc_sequence() {
        // OSC 8 hyperlink trick: title text hidden behind a fake link.
        let malicious = "\x1b]8;;http://evil.example\x07click\x1b]8;;\x07me";
        assert_eq!(clean_line(malicious), "clickme");
    }

    #[test]
    fn clean_line_strips_generic_c1_control_byte_and_newlines() {
        // U+0085 (NEL) is a plain C1 control byte with no CSI/OSC meaning;
        // it must simply be dropped like any other control character. (0x9B
        // is deliberately not used here since it's now a CSI introducer —
        // see the dedicated c1_csi/c1_osc tests below.)
        let input = "a\u{85}b\nc\rd\te";
        assert_eq!(clean_line(input), "abcde");
    }

    #[test]
    fn clean_line_strips_c1_csi_sequence() {
        // 0x9B is the 8-bit CSI introducer: equivalent to `ESC '['`. A naive
        // filter that only recognizes the 7-bit `ESC '['` form would leave
        // the parameter/final bytes of this variant behind as visible junk.
        let malicious = "\u{9b}31mRed\u{9b}0mEnd";
        assert_eq!(clean_line(malicious), "RedEnd");
    }

    #[test]
    fn clean_line_strips_c1_osc_sequence() {
        // 0x9D is the 8-bit OSC introducer: equivalent to `ESC ']'`.
        let malicious = "\u{9d}8;;http://evil.example\u{7}click\u{9d}8;;\u{7}me";
        assert_eq!(clean_line(malicious), "clickme");
    }

    #[test]
    fn clean_line_c1_csi_over_param_limit_drops_only_introducer() {
        // Mirrors clean_line_gives_up_on_csi_over_param_limit_and_keeps_scanning
        // but for the 8-bit introducer: unterminated within the cap, so only
        // the introducer itself is dropped and the rest survives literally.
        let params = "1".repeat(16);
        let input = format!("\u{9b}{params}mEND");
        let expected = format!("{params}mEND");
        assert_eq!(clean_line(&input), expected);
    }

    #[test]
    fn clean_line_preserves_plain_text() {
        assert_eq!(clean_line("普通のタイトル 123"), "普通のタイトル 123");
    }

    #[test]
    fn clean_multiline_keeps_newlines_and_tabs_but_strips_escape() {
        let input = "line1\n\x1b[2Jline2\tend";
        assert_eq!(clean_multiline(input), "line1\nline2\tend");
    }

    #[test]
    fn clean_line_recognizes_csi_sequence_right_at_param_limit() {
        // 15 parameter bytes + 1 final byte = 16 bytes scanned: within the cap.
        let params = "1".repeat(15);
        let input = format!("\x1b[{params}mEND");
        assert_eq!(clean_line(&input), "END");
    }

    #[test]
    fn clean_line_gives_up_on_csi_over_param_limit_and_keeps_scanning() {
        // 16 parameter bytes + 1 final byte = 17 bytes scanned: exceeds the
        // cap, so this must NOT be treated as a CSI sequence. Only the ESC
        // is dropped; the '[' and everything after it survive as literal
        // text (including the stray "final" byte, which is no longer
        // interpreted as one since no CSI was recognized).
        let params = "1".repeat(16);
        let input = format!("\x1b[{params}mEND");
        let expected = format!("[{params}mEND");
        assert_eq!(clean_line(&input), expected);
    }

    #[test]
    fn clean_line_unterminated_csi_does_not_swallow_trailing_text() {
        // No final byte anywhere: without a cap this would consume the rest
        // of the string. It must stop at the limit and preserve "TAIL".
        let junk = "9".repeat(50);
        let input = format!("\x1b[{junk}TAIL");
        assert!(
            clean_line(&input).ends_with("TAIL"),
            "trailing text must survive an unterminated CSI payload"
        );
    }

    #[test]
    fn sanitize_for_message_strips_ansi() {
        let malicious = "\x1b[31mnope\x1b[0m";
        assert_eq!(sanitize_for_message(malicious), "nope");
    }

    #[test]
    fn sanitize_for_message_passes_short_plain_text_unchanged() {
        assert_eq!(sanitize_for_message("urgentish"), "urgentish");
    }

    #[test]
    fn sanitize_for_message_truncates_long_input_with_ellipsis() {
        let long = "a".repeat(500);
        let out = sanitize_for_message(&long);
        assert!(out.chars().count() <= MESSAGE_ECHO_LIMIT + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn sanitize_for_message_does_not_panic_on_multibyte_boundary() {
        // Truncation must respect char boundaries even with multi-byte
        // (e.g. Japanese) characters right at the cutoff point.
        let long: String = "あ".repeat(500);
        let out = sanitize_for_message(&long);
        assert!(out.ends_with('…'));
    }
}
