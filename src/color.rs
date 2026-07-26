//! Minimal ANSI color escape helpers used for terminal table/detail output.
//! No external crate dependency — plain escape codes, disabled automatically
//! when stdout is not a TTY (see [`enabled`]).

use std::io::IsTerminal;

pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
pub const CYAN: &str = "\x1b[36m";
pub const YELLOW: &str = "\x1b[33m";
pub const RED: &str = "\x1b[31m";
pub const BOLD_RED: &str = "\x1b[1;31m";
pub const GRAY: &str = "\x1b[90m";

/// Whether color output should be emitted: only when stdout is a real
/// terminal and NO_COLOR is not set, so piping into `jq`/files/tests stays clean.
pub fn enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

/// Wrap `text` in `code` .. RESET, but only if color output is enabled.
pub fn paint(code: &str, text: &str) -> String {
    if enabled() {
        format!("{code}{text}{RESET}")
    } else {
        text.to_string()
    }
}
