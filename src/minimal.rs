//! `--minimal`: print the logo and info once, statically (like fastfetch), then
//! exit. No animation, point cloud, render buffers, raw mode or signal handlers.

use crate::util::{emit_clipped, skip_ansi, visible_width};

const GAP: usize = 3;
const RESET: &str = "\x1b[0m";

pub struct Opts<'a> {
    pub use_color: bool,
    /// SGR escape for logos that carry no colors of their own.
    pub color: &'a str,
    /// Terminal width, 0 if unknown.
    pub term_cols: i32,
}

pub fn strip_ansi(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let a = skip_ansi(&b[i..]);
        if a > 0 {
            i += a;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Cut a row to `cols` visible columns, keeping its color escapes (no ellipsis:
/// this is art).
fn clip_row(row: &str, cols: usize) -> String {
    let b = row.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let (mut i, mut w) = (0, 0);
    while i < b.len() {
        let a = skip_ansi(&b[i..]);
        if a > 0 {
            out.extend_from_slice(&b[i..i + a]);
            i += a;
            continue;
        }
        if w >= cols {
            break;
        }
        let len = crate::util::utf8_char_len(b[i]).min(b.len() - i);
        out.extend_from_slice(&b[i..i + len]);
        i += len;
        w += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lay out the logo rows on the left and the info lines on the right, top
/// aligned. On a terminal too narrow for both, the info goes below the logo.
pub fn render(rows: &[String], info: &[String], logo_has_ansi: bool, o: &Opts) -> Vec<u8> {
    let cols = o.term_cols;
    // A logo wider than the terminal would wrap into a mess: cut it to fit.
    let clipped: Vec<String>;
    let rows = if cols > 0 && rows.iter().any(|r| visible_width(r) > cols as usize) {
        clipped = rows.iter().map(|r| clip_row(r, cols as usize)).collect();
        &clipped[..]
    } else {
        rows
    };
    let width = rows.iter().map(|r| visible_width(r)).max().unwrap_or(0);
    let stacked = cols > 0 && !rows.is_empty() && !info.is_empty() && (cols as usize) < width + GAP + 10;

    let paint = |row: &str| -> String {
        if !o.use_color {
            strip_ansi(row)
        } else if logo_has_ansi {
            format!("{row}{RESET}")
        } else {
            format!("{}{row}{RESET}", o.color)
        }
    };
    let pad = |row: &str| " ".repeat(width.saturating_sub(visible_width(row)));

    let mut out: Vec<u8> = Vec::new();
    if stacked {
        for r in rows {
            out.extend_from_slice(paint(r).as_bytes());
            out.push(b'\n');
        }
        for l in info {
            emit_clipped(&mut out, l.as_bytes(), cols);
            out.push(b'\n');
        }
        return out;
    }

    let avail = if cols > 0 { (cols as usize).saturating_sub(width + GAP) as i32 } else { -1 };
    for i in 0..rows.len().max(info.len()) {
        match rows.get(i) {
            Some(r) => {
                out.extend_from_slice(paint(r).as_bytes());
                out.extend_from_slice(pad(r).as_bytes());
            }
            None => out.extend(std::iter::repeat_n(b' ', width)),
        }
        if let Some(l) = info.get(i) {
            out.extend(std::iter::repeat_n(b' ', GAP));
            emit_clipped(&mut out, l.as_bytes(), avail);
        } else {
            // nothing to the right: drop the padding we just wrote
            while out.last() == Some(&b' ') {
                out.pop();
            }
        }
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::visible_width;

    fn opts(cols: i32) -> Opts<'static> {
        Opts { use_color: false, color: "\x1b[1;35m", term_cols: cols }
    }

    fn lines(out: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(out).lines().map(str::to_string).collect()
    }

    #[test]
    fn aligns_info_beside_logo_top() {
        let rows = vec!["@@@".to_string(), "@".to_string()];
        let info = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let l = lines(&render(&rows, &info, false, &opts(0)));
        assert_eq!(l, vec!["@@@   a", "@     b", "      c"]);
    }

    #[test]
    fn colors_plain_logos_and_keeps_fastfetch_colors() {
        let rows = vec!["ab".to_string()];
        let plain = Opts { use_color: true, color: "\x1b[1;35m", term_cols: 0 };
        assert_eq!(lines(&render(&rows, &[], false, &plain))[0], "\x1b[1;35mab\x1b[0m");
        let colored = vec!["\x1b[31mab".to_string()];
        assert_eq!(lines(&render(&colored, &[], true, &plain))[0], "\x1b[31mab\x1b[0m");
        assert_eq!(lines(&render(&colored, &[], true, &opts(0)))[0], "ab");
    }

    #[test]
    fn clips_info_to_the_terminal() {
        let rows = vec!["@@@@".to_string()];
        let info = vec!["x".repeat(50)];
        let l = lines(&render(&rows, &info, false, &opts(30)));
        assert_eq!(l.len(), 1);
        assert!(visible_width(&l[0]) <= 30);
    }

    #[test]
    fn clips_logos_wider_than_the_terminal() {
        let rows = vec!["@".repeat(40), "\x1b[31m".to_string() + &"#".repeat(40)];
        let out = render(&rows, &[], true, &Opts { use_color: true, color: "", term_cols: 20 });
        for l in lines(&out) {
            assert!(visible_width(&l) <= 20, "{l:?}");
        }
        assert!(String::from_utf8_lossy(&out).contains("\x1b[31m"));
    }

    #[test]
    fn stacks_on_narrow_terminals() {
        let rows = vec!["@".repeat(40)];
        let info = vec!["hello".to_string()];
        let l = lines(&render(&rows, &info, false, &opts(30)));
        assert_eq!(l.len(), 2);
        assert_eq!(l[1], "hello");
    }
}
