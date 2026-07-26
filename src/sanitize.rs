//! Strips terminal control characters and ANSI/VT100 escape sequences from
//! user-supplied text *before it is stored*, so no value read back out of
//! the database can later manipulate the terminal when rendered by
//! `list`/`show` (cursor movement, screen clears, hidden-text tricks, etc.).
//!
//! A naive filter that only drops the ESC (0x1B) byte would leave the rest
//! of a CSI/OSC sequence (e.g. the `[31m` in `ESC[31m`) behind as visible
//! junk, so this recognizes and drops whole sequences:
//! - CSI: `ESC '[' params... final-byte` (final byte in `0x40..=0x7E`)
//! - OSC: `ESC ']' ... (BEL | ESC '\\')`
//! - other two-byte escapes: `ESC <any char>`

/// Max characters scanned after `ESC '['` while looking for a CSI final byte
/// (0x40..=0x7E). Real CSI sequences are always short (a handful of
/// parameter/intermediate bytes); without a cap, a malformed/malicious
/// `ESC '[' <many non-final bytes>` payload with no final byte would let the
/// scan run to the end of the input, silently swallowing everything after
/// it. If no final byte turns up within the cap, the sequence is not
/// treated as CSI: only the ESC itself is dropped, and scanning resumes
/// from the very next character (the `[` and whatever follows it are
/// treated as ordinary text, same as any other input).
const CSI_PARAM_LIMIT: usize = 16;

fn strip_ansi<F: Fn(char) -> bool>(input: &str, keep_control: F) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < n {
        let c = chars[i];

        if c == '\u{1b}' {
            if chars.get(i + 1) == Some(&'[') {
                let params_start = i + 2;
                let scan_end = (params_start + CSI_PARAM_LIMIT).min(n);
                let final_byte =
                    (params_start..scan_end).find(|&j| ('\x40'..='\x7e').contains(&chars[j]));

                i = match final_byte {
                    // Whole ESC '[' params... final-byte sequence: drop it.
                    Some(j) => j + 1,
                    // No final byte within the cap: drop only the ESC.
                    None => i + 1,
                };
                continue;
            }

            if chars.get(i + 1) == Some(&']') {
                // OSC: ESC ']' ... (BEL | ESC '\\')
                let mut j = i + 2;
                let mut end = None;
                while j < n {
                    if chars[j] == '\u{7}' {
                        end = Some(j);
                        break;
                    }
                    if chars[j] == '\u{1b}' && chars.get(j + 1) == Some(&'\\') {
                        end = Some(j + 1);
                        break;
                    }
                    j += 1;
                }
                i = match end {
                    Some(e) => e + 1,
                    None => i + 1,
                };
                continue;
            }

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
/// ANSI escape sequences plus every remaining Unicode control character
/// (including `\n`/`\r`/`\t` and stray C1 bytes 0x80-0x9F).
pub fn clean_line(input: &str) -> String {
    strip_ansi(input, |_| false)
}

/// For multi-line fields (description): same as [`clean_line`] but keeps
/// `\n` and `\t` so legitimate multi-line notes survive.
pub fn clean_multiline(input: &str) -> String {
    strip_ansi(input, |c| c == '\n' || c == '\t')
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
    fn clean_line_strips_c1_and_newlines() {
        let input = "a\u{9b}b\nc\rd\te";
        assert_eq!(clean_line(input), "abcde");
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
}
