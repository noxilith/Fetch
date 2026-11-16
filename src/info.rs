//! The system-info block shown beside the logo: line storage, labeled-line
//! formatting, live refresh in place, and the optional box.

use crate::config::Field;
use crate::util::{emit_clipped, truncate_bytes, visible_width};

pub const MAX_FETCH_LINES: usize = 32;
pub const MAX_LINE_LEN: usize = 512;

pub struct Info {
    pub lines: Vec<String>,
    /// Line index of each field, `None` if not shown.
    pub field_line: [Option<usize>; Field::COUNT],
    /// Which field is currently being gathered.
    pub current: Option<Field>,
    /// True during the animation's live-refresh tick.
    pub refresh: bool,
    /// > 0 once `box_wrap_lines` has run.
    pub box_width: usize,
    label_color: String,
    show_title: bool,
}

impl Info {
    pub fn new(label_color: &str, show_title: bool) -> Self {
        Info {
            lines: Vec::new(),
            field_line: [None; Field::COUNT],
            current: None,
            refresh: false,
            box_width: 0,
            label_color: label_color.to_string(),
            show_title,
        }
    }

    pub fn add_line(&mut self, line: &str) {
        if self.lines.len() >= MAX_FETCH_LINES {
            return;
        }
        self.lines
            .push(truncate_bytes(line, MAX_LINE_LEN - 1).to_string());
    }

    /// Add a `Label: value` line using the configured label color. On a
    /// refresh tick the current field's existing line is replaced in place;
    /// on the initial pass a new line is always appended (so gatherers that
    /// emit several rows, like multi-GPU, don't overwrite themselves).
    pub fn add_info(&mut self, label: &str, value: &str) {
        let line = format!("\x1b[1;{}m{}\x1b[0m: {}", self.label_color, label, value);

        if self.refresh {
            if let Some(idx) = self.current.and_then(|f| self.field_line[f.index()]) {
                let text = if self.box_width > 0 {
                    let pad = self.box_width.saturating_sub(visible_width(&line));
                    format!("│ {line}{} │", " ".repeat(pad))
                } else {
                    line
                };
                self.lines[idx] = truncate_bytes(&text, MAX_LINE_LEN - 1).to_string();
                return;
            }
        }
        if self.lines.len() >= MAX_FETCH_LINES {
            return;
        }
        if let Some(f) = self.current {
            self.field_line[f.index()] = Some(self.lines.len());
        }
        self.add_line(&line);
    }

    /// The `user@host` title and its underline.
    pub fn add_title(&mut self, separator: &str) {
        // SAFETY: getlogin returns null or a pointer to a NUL-terminated string.
        let login = unsafe {
            let p = libc::getlogin();
            if p.is_null() {
                None
            } else {
                Some(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
            }
        };
        let user = login
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_default();
        let user = truncate_bytes(&user, 63);

        let mut buf = [0u8; 64];
        // SAFETY: buf is valid for 64 bytes.
        unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let host = String::from_utf8_lossy(&buf[..end]).into_owned();

        let lc = self.label_color.clone();
        self.add_line(&format!(
            "\x1b[1;{lc}m{user}\x1b[0m@\x1b[1;{lc}m{host}\x1b[0m"
        ));

        let title_len = user.len() + 1 + host.len();
        let mut sep = String::new();
        if !separator.is_empty() {
            for _ in 0..title_len {
                if sep.len() + separator.len() >= MAX_LINE_LEN {
                    break;
                }
                sep.push_str(separator);
            }
        }
        self.add_line(&sep);
    }

    /// Wrap the info lines (skipping the title + underline) in a box. Must run
    /// once, after all initial gathering; it shifts `field_line` so later
    /// live-refresh writes land on the right row.
    pub fn box_wrap_lines(&mut self) {
        let start = if self.show_title { 2 } else { 0 };
        if start >= self.lines.len() {
            return;
        }

        let max_w = self.lines[start..]
            .iter()
            .map(|l| visible_width(l))
            .max()
            .unwrap_or(0)
            .min((MAX_LINE_LEN - 16) / 3);
        self.box_width = max_w;

        let content_count = (self.lines.len() - start).min(MAX_FETCH_LINES - start - 2);
        let content: Vec<String> = self.lines[start..start + content_count]
            .iter()
            .map(|l| {
                let mut v = Vec::new();
                emit_clipped(&mut v, l.as_bytes(), max_w as i32);
                String::from_utf8_lossy(&v).into_owned()
            })
            .collect();

        let border = |l: &str, r: &str| format!("{l}{}{r}", "─".repeat(max_w + 2));

        self.lines.truncate(start);
        self.lines.push(border("╭", "╮"));
        for c in &content {
            let pad = max_w.saturating_sub(visible_width(c));
            let row = format!("│ {c}{} │", " ".repeat(pad));
            self.lines
                .push(truncate_bytes(&row, MAX_LINE_LEN - 1).to_string());
        }
        if self.lines.len() < MAX_FETCH_LINES {
            self.lines.push(border("╰", "╯"));
        }

        for fl in self.field_line.iter_mut() {
            if let Some(i) = *fl {
                if i >= start {
                    *fl = if i < start + content_count {
                        Some(i + 1)
                    } else {
                        None
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(show_title: bool) -> Info {
        Info::new("35", show_title)
    }

    #[test]
    fn info_bounds() {
        let mut i = info(true);
        i.add_line("title");
        i.add_line("-----");

        let long_value = "x".repeat(MAX_LINE_LEN - 1);
        for n in 0..MAX_FETCH_LINES - 2 {
            i.current = Some(if n == 0 { Field::Uptime } else { Field::Memory });
            i.add_info("Long field", &long_value);
        }
        i.current = Some(Field::Gpu);
        i.add_info("Dropped", "value");
        assert_eq!(i.lines.len(), MAX_FETCH_LINES);
        assert_eq!(i.field_line[Field::Gpu.index()], None);

        i.box_wrap_lines();
        assert_eq!(i.lines.len(), MAX_FETCH_LINES);
        assert_eq!(i.field_line[Field::Uptime.index()], Some(3));
        assert_eq!(i.field_line[Field::Memory.index()], None);
        assert!(i.box_width > 0 && i.box_width <= (MAX_LINE_LEN - 16) / 3);
        assert!(i.lines[2].starts_with('╭'));
        assert!(i.lines[MAX_FETCH_LINES - 1].starts_with('╰'));
        for l in &i.lines[2..] {
            assert_eq!(visible_width(l), i.box_width + 4);
        }

        i.refresh = true;
        i.current = Some(Field::Uptime);
        i.add_info("Uptime", "refreshed");
        assert_eq!(i.lines.len(), MAX_FETCH_LINES);
        let idx = i.field_line[Field::Uptime.index()].unwrap();
        assert!(i.lines[idx].contains("refreshed"));
        assert_eq!(visible_width(&i.lines[idx]), i.box_width + 4);
    }

    #[test]
    fn box_without_title() {
        let mut i = info(false);
        i.current = Some(Field::Uptime);
        i.add_info("Uptime", "initial");
        i.current = Some(Field::Memory);
        i.add_info("Memory", "initial");
        i.current = None;

        i.box_wrap_lines();
        assert_eq!(i.lines.len(), 4);
        assert!(i.lines[0].starts_with('╭'));
        assert!(i.lines[3].starts_with('╰'));
        assert_eq!(i.field_line[Field::Uptime.index()], Some(1));
        assert_eq!(i.field_line[Field::Memory.index()], Some(2));
        assert!(i.lines[1].contains("Uptime"));
        assert!(i.lines[2].contains("Memory"));

        i.refresh = true;
        i.current = Some(Field::Uptime);
        i.add_info("Uptime", "updated");
        assert!(i.lines[1].contains("updated"));
        assert_eq!(visible_width(&i.lines[1]), i.box_width + 4);
    }
}
