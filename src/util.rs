//! Small shared helpers: number parsing, UTF-8/ANSI-aware string handling,
//! and non-shell command execution.

use std::fs;
use std::process::{Command, Stdio};

pub const ESC: u8 = 0x1b;

/// Parse a finite float in `min..=max`, printing the standard error message on failure.
pub fn parse_number(option: &str, text: &str, min: f32, max: f32) -> Option<f32> {
    let parsed = text
        .trim_start()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && *v >= min && *v <= max);
    if parsed.is_none() {
        eprintln!(
            "fetch: {option} requires a finite number from {min} to {max} (got '{text}')"
        );
    }
    parsed
}

/// Parse an integer in `min..=max`, printing the standard error message on failure.
pub fn parse_integer(option: &str, text: &str, min: i64, max: i64) -> Option<i64> {
    let parsed = text
        .trim_start()
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= min && *v <= max);
    if parsed.is_none() {
        eprintln!(
            "fetch: {option} requires an integer from {min} to {max} (got '{text}')"
        );
    }
    parsed
}

/// Byte length of a UTF-8 sequence from its leading byte (invalid = 1).
pub fn utf8_char_len(c: u8) -> usize {
    if c < 0x80 {
        1
    } else if c & 0xE0 == 0xC0 {
        2
    } else if c & 0xF0 == 0xE0 {
        3
    } else if c & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

/// Number of bytes in the ANSI escape (`ESC [ ... final`) at the start of `s`, or 0.
pub fn skip_ansi(s: &[u8]) -> usize {
    if s.len() < 2 || s[0] != ESC || s[1] != b'[' {
        return 0;
    }
    let mut i = 2;
    while i < s.len() && (s[i].is_ascii_digit() || s[i] == b';') {
        i += 1;
    }
    if i < s.len() {
        i += 1; // the final byte
    }
    i
}

/// Visible columns of a string, ignoring ANSI escapes (one codepoint = one column).
pub fn visible_width(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    let mut w = 0;
    while i < b.len() {
        let a = skip_ansi(&b[i..]);
        if a > 0 {
            i += a;
            continue;
        }
        i += utf8_char_len(b[i]);
        w += 1;
    }
    w
}

/// Append `s` to `out`, clipped to `max_cols` visible columns (ANSI passes
/// through; `max_cols < 0` means no limit). Adds "..." when clipping a wide
/// enough field, and a reset if the clip may have cut a color short.
pub fn emit_clipped(out: &mut Vec<u8>, s: &[u8], max_cols: i32) {
    let mut total_w = 0i32;
    let mut i = 0;
    while i < s.len() {
        let a = skip_ansi(&s[i..]);
        if a > 0 {
            i += a;
            continue;
        }
        i += utf8_char_len(s[i]);
        total_w += 1;
    }
    let need_clip = max_cols >= 0 && total_w > max_cols;
    let mut limit = max_cols;
    if need_clip && max_cols >= 6 {
        limit = max_cols - 3; // leave room for "..."
    }

    let mut i = 0;
    let mut w = 0i32;
    let mut had_ansi = false;
    while i < s.len() {
        let a = skip_ansi(&s[i..]);
        if a > 0 {
            out.extend_from_slice(&s[i..i + a]);
            i += a;
            had_ansi = true;
            continue;
        }
        if limit >= 0 && w >= limit {
            break;
        }
        let len = utf8_char_len(s[i]).min(s.len() - i);
        out.extend_from_slice(&s[i..i + len]);
        i += len;
        w += 1;
    }
    if need_clip && max_cols >= 6 {
        out.extend_from_slice(b"...");
    }
    if had_ansi && need_clip {
        out.extend_from_slice(b"\x1b[0m");
    }
}

/// Strip a trailing " (...)" documentation hint from a config value, plus any
/// trailing blanks: "white (red, green, ...)" -> "white".
pub fn strip_inline_hint(val: &str) -> &str {
    let val = match val.find(" (") {
        Some(i) => &val[..i],
        None => val,
    };
    val.trim_end_matches([' ', '\t'])
}

/// Cut `s` to at most `max` bytes without splitting a character.
pub fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// First line of a small pseudo-file (under /proc or /sys), cut at the first
/// NUL (device-tree strings are NUL-terminated) and stripped of line endings.
/// `None` if the file is unreadable or the line is empty.
pub fn read_first_line(path: &str) -> Option<String> {
    let data = fs::read(path).ok()?;
    let line = data.split(|&b| b == b'\n').next().unwrap_or(&[]);
    let line = line.split(|&b| b == 0).next().unwrap_or(&[]);
    let s = String::from_utf8_lossy(line);
    let s = s.trim_end_matches(['\r', '\n']);
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Run a program (no shell) with stderr discarded; stdout as lossy text.
pub fn run_output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Run `<bin> --version` and pull out the first version-looking token
/// (starting at the first digit). Returns an empty string on any failure.
/// `bin` must be made of path/identifier characters only.
pub fn cmd_version(bin: &str) -> String {
    if bin.is_empty()
        || !bin
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'/' | b'_' | b'-'))
    {
        return String::new();
    }
    let Some(out) = run_output(bin, &["--version"]) else {
        return String::new();
    };
    let first = out.lines().next().unwrap_or("");
    let Some(start) = first.find(|c: char| c.is_ascii_digit()) else {
        return String::new();
    };
    first[start..]
        .chars()
        .take_while(|&c| c != ' ' && c != '(')
        .take(30)
        .collect()
}

/// Basename of a path-like string.
pub fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// Count of lines in a command's output (0 if it can't run).
pub fn count_output_lines(program: &str, args: &[&str]) -> usize {
    run_output(program, args).map_or(0, |o| o.lines().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number("x", "1.5", 0.5, 5.0), Some(1.5));
        for bad in ["nan", "inf", "1junk", "1e9999", "0.4", "5.1", ""] {
            assert_eq!(parse_number("x", bad, 0.5, 5.0), None, "{bad}");
        }
        assert_eq!(parse_integer("x", "20", 1, 200), Some(20));
        for bad in ["0", "201", "1.5", "junk", "2147483648"] {
            assert_eq!(parse_integer("x", bad, 1, 200), None, "{bad}");
        }
    }

    #[test]
    fn width_ignores_ansi() {
        assert_eq!(visible_width("\x1b[1;35mab\x1b[0m: ─"), 5);
    }

    #[test]
    fn clipping() {
        let mut out = Vec::new();
        emit_clipped(&mut out, "\x1b[1mabcdefghij\x1b[0m".as_bytes(), 8);
        let s = String::from_utf8(out).unwrap();
        assert_eq!(visible_width(&s), 8);
        assert!(s.contains("abcde...") && s.ends_with("\x1b[0m"));
    }

    #[test]
    fn hints() {
        assert_eq!(strip_inline_hint("white (red, green)"), "white");
        assert_eq!(strip_inline_hint("cyan \t"), "cyan");
    }
}
