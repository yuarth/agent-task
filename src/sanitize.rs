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

fn strip_ansi<F: Fn(char) -> bool>(input: &str, keep_control: F) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    for nc in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&nc) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(nc) = chars.next() {
                        if nc == '\u{7}' {
                            break;
                        }
                        if nc == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
            continue;
        }

        if c.is_control() && !keep_control(c) {
            continue;
        }
        out.push(c);
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
}
