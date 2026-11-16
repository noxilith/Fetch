//! Logo loading (custom file, fastfetch, built-in), parsing into cells, and
//! per-character relief weights.

use crate::util::{truncate_bytes, ESC};
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

pub const MAX_LOGO_ROWS: usize = 64;
pub const MAX_LOGO_COLS: usize = 128;

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    /// ANSI foreground code from the source art (0 = none).
    pub color: i32,
}

#[derive(Default)]
pub struct Logo {
    /// Raw rows, possibly with ANSI color escapes.
    pub raw: Vec<String>,
    /// Parsed cells per row (filled by `process`).
    pub cells: Vec<Vec<Cell>>,
    pub cols: usize,
    pub has_ansi: bool,
}

fn is_fg_color(n: i32) -> bool {
    (30..=37).contains(&n) || n == 39 || (90..=97).contains(&n)
}

/// Split a row into codepoints, extracting foreground colors from SGR escapes.
fn process_row(raw: &str, has_ansi: &mut bool) -> Vec<Cell> {
    let b = raw.as_bytes();
    let mut cells = Vec::new();
    let mut p = 0;
    let mut cur_color = 0;
    while p < b.len() && cells.len() < MAX_LOGO_COLS {
        if b[p] == ESC && b.get(p + 1) == Some(&b'[') {
            let mut i = 2;
            let mut num = 0i32;
            let mut has_num = false;
            while let Some(&c) = b.get(p + i) {
                if c.is_ascii_digit() {
                    num = num.saturating_mul(10).saturating_add(i32::from(c - b'0'));
                    has_num = true;
                } else if c == b';' {
                    if has_num && is_fg_color(num) {
                        cur_color = num;
                    }
                    if has_num && (num == 0 || num == 22) {
                        cur_color = 0;
                    }
                    num = 0;
                    has_num = false;
                } else {
                    break;
                }
                i += 1;
            }
            if has_num && is_fg_color(num) {
                cur_color = num;
            }
            if has_num && num == 0 {
                cur_color = 0;
            }
            if p + i < b.len() {
                i += 1; // final byte
                while p + i < b.len() && !raw.is_char_boundary(p + i) {
                    i += 1;
                }
            }
            if cur_color > 0 {
                *has_ansi = true;
            }
            p += i;
            continue;
        }
        if !raw.is_char_boundary(p) {
            p += 1;
            continue;
        }
        let ch = raw[p..].chars().next().unwrap();
        cells.push(Cell {
            ch,
            color: cur_color,
        });
        p += ch.len_utf8();
    }
    cells
}

fn trim_row_end(buf: &mut Vec<u8>) {
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
}

/// Is this an ANSI escape that isn't a color/SGR one (cursor movement etc.)?
fn is_cursor_escape(p: &[u8]) -> bool {
    if p.len() < 2 || p[0] != ESC || p[1] != b'[' {
        return false;
    }
    let mut i = 2;
    while i < p.len() && (p[i].is_ascii_digit() || p[i] == b';') {
        i += 1;
    }
    i < p.len() && p[i] != b'm'
}

fn spawn_fastfetch(args: &[&str]) -> Option<std::process::Child> {
    Command::new("fastfetch")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

impl Logo {
    pub fn rows(&self) -> usize {
        self.raw.len()
    }

    fn trim_trailing_blank(&mut self) {
        while self.raw.last().is_some_and(|r| r.is_empty()) {
            self.raw.pop();
        }
    }

    /// Parse all raw rows into cells.
    pub fn process(&mut self) {
        self.has_ansi = false;
        let mut has_ansi = false;
        self.cells = self
            .raw
            .iter()
            .map(|r| process_row(r, &mut has_ansi))
            .collect();
        self.has_ansi = has_ansi;
        self.cols = self.cells.iter().map(Vec::len).max().unwrap_or(0);
    }

    /// Load `~/.config/fetch/logo.txt`. Returns whether a logo was loaded and
    /// the `# distro:` color-scheme name from its first line, if any.
    pub fn load_file(&mut self) -> (bool, String) {
        let mut distro = String::new();
        let Some(home) = std::env::var_os("HOME") else {
            return (false, distro);
        };
        let path = std::path::Path::new(&home).join(".config/fetch/logo.txt");
        let Ok(data) = fs::read(path) else {
            return (false, distro);
        };
        let text = String::from_utf8_lossy(&data);
        for line in text.split('\n') {
            if self.raw.len() >= MAX_LOGO_ROWS {
                break;
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if self.raw.is_empty() {
                if let Some(rest) = line.strip_prefix("# distro:") {
                    distro = truncate_bytes(rest.trim_start_matches(' '), 63).into();
                    continue;
                }
                if line.is_empty() {
                    continue;
                }
            }
            self.raw.push(line.to_string());
        }
        // A trailing newline leaves one empty piece; trim_trailing_blank drops it.
        self.trim_trailing_blank();
        (!self.raw.is_empty(), distro)
    }

    /// Load from fastfetch's colored output, falling back to `--print-logos`.
    pub fn load_fastfetch(&mut self, name: &str) -> bool {
        self.load_ff_colored(name) || self.load_ff_plain(name)
    }

    fn load_ff_colored(&mut self, name: &str) -> bool {
        let Some(mut child) = spawn_fastfetch(&[
            "-c", "none", "-l", name, "-s", "break", "--pipe", "false",
        ]) else {
            return false;
        };
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut buf = Vec::new();
        while self.raw.len() < MAX_LOGO_ROWS {
            buf.clear();
            if reader.read_until(b'\n', &mut buf).unwrap_or(0) == 0 {
                break;
            }
            trim_row_end(&mut buf);

            // Cut at the first cursor-movement escape (end of logo content),
            // keeping the last complete color sequence before it.
            let mut truncated = false;
            let mut last_sgr_end: Option<usize> = None;
            let mut i = 0;
            while i + 2 < buf.len() {
                if is_cursor_escape(&buf[i..]) {
                    buf.truncate(last_sgr_end.unwrap_or(i));
                    truncated = true;
                    break;
                }
                if buf[i] == ESC && buf[i + 1] == b'[' {
                    let mut j = i + 2;
                    while j < buf.len() && (buf[j].is_ascii_digit() || buf[j] == b';') {
                        j += 1;
                    }
                    if buf.get(j) == Some(&b'm') {
                        last_sgr_end = Some(j + 1);
                        i = j;
                    }
                }
                i += 1;
            }

            if buf.is_empty() && self.raw.is_empty() {
                continue;
            }
            if buf.is_empty() && truncated {
                break;
            }
            self.raw.push(String::from_utf8_lossy(&buf).into_owned());
        }
        drop(reader);
        let _ = child.kill();
        let _ = child.wait();
        self.trim_trailing_blank();
        !self.raw.is_empty()
    }

    /// Fallback: `--print-logos` (no colors, but works on older fastfetch).
    fn load_ff_plain(&mut self, name: &str) -> bool {
        let Some(mut child) = spawn_fastfetch(&["-c", "none", "--print-logos"]) else {
            return false;
        };
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut found = false;
        let mut buf = Vec::new();
        loop {
            buf.clear();
            if reader.read_until(b'\n', &mut buf).unwrap_or(0) == 0 {
                break;
            }
            trim_row_end(&mut buf);
            let line = String::from_utf8_lossy(&buf).into_owned();
            let len = line.len();

            if !found {
                if len > 0
                    && len <= name.len() + 1
                    && line.ends_with(':')
                    && line[..len - 1].eq_ignore_ascii_case(name)
                {
                    found = true;
                }
                continue;
            }

            // Next logo's header ends this one.
            if len > 1
                && len < 40
                && line.ends_with(':')
                && !self.raw.is_empty()
                && line.as_bytes()[0].is_ascii_alphabetic()
                && !line.as_bytes().contains(&ESC)
            {
                break;
            }
            if self.raw.len() >= MAX_LOGO_ROWS {
                break;
            }
            self.raw.push(line);
        }
        drop(reader);
        let _ = child.kill();
        let _ = child.wait();
        self.trim_trailing_blank();
        !self.raw.is_empty()
    }

    /// Load the logo built into the binary for this distro, if there is one.
    pub fn load_builtin(&mut self, distro: &str) -> bool {
        let Some(art) = crate::logos::builtin(distro) else {
            return false;
        };
        self.raw = art.iter().map(|s| s.to_string()).collect();
        true
    }

    /// The built-in Gentoo logo.
    pub fn load_default(&mut self) {
        const GENTOO: [&str; 18] = [
            "         -/oyddmdhs+:.            ",
            "     -odNMMMMMMMMNNmhy+-`         ",
            "   -yNMMMMMMMMMMMNNNmmdhy+-       ",
            " `omMMMMMMMMMMMMNmdmmmmddhhy/`    ",
            " omMMMMMMMMMMMNhhyyyohmdddhhhdo`  ",
            ".ydMMMMMMMMMMdhs++so/smdddhhhhdm+`",
            " oyhdmNMMMMMMMNdyooydMddddhhhhyhNd.",
            "  :oyhhdNNMMMMMMMNNMMMdddhhhhhyymMh",
            "    .:+sydNMMMMMNNMMMMdddhhhhhhmMmy",
            "       /mMMMMMMNNNMMMdddhhhhhmMNhs:",
            "    `oNMMMMMMMNNNMMMddddhhdmMNhs+` ",
            "  `sNMMMMMMMMNNNMMMdddddmNMmhs/.   ",
            " /NMMMMMMMMNNNNMMMdddmNMNdso:`     ",
            "+MMMMMMMNNNNNMMMMdMNMNdso/-        ",
            "yMMNNNNNNNMMMMMNNMmhs+/-`          ",
            "/hMMNNNNNNNNMNdhs++/-`             ",
            "`/ohdmmddhys+++/:.`                ",
            "  `-//////:--.                     ",
        ];
        self.raw = GENTOO.iter().map(|s| s.to_string()).collect();
    }
}

// --- distro detection ---

#[derive(Default)]
pub struct Distro {
    pub id: String,
    pub id_like: String,
}

/// Value from a `KEY="value"` os-release line: quotes stripped, 1..max-1 bytes.
fn os_release_val(v: &str, max: usize) -> Option<String> {
    if v.len() >= 256 {
        return None;
    }
    let v = v.trim_end_matches(['\n', '\r']);
    let v = v.strip_prefix('"').unwrap_or(v);
    let v = v.strip_suffix('"').unwrap_or(v);
    if !v.is_empty() && v.len() < max {
        Some(v.to_string())
    } else {
        None
    }
}

/// The first string value after the colon in a JSON-ish `"key": "value"` line.
fn json_string_value(line: &str, key_pos: usize) -> Option<&str> {
    let after = &line[key_pos..];
    let colon = after.find(':')?;
    let rest = &after[colon..];
    let q1 = rest.find('"')? + 1;
    let q2 = rest[q1..].find('"')?;
    Some(&rest[q1..q1 + q2])
}

/// `fastfetch --json` knows more than os-release (it spots Proxmox even
/// though ID=debian).
fn detect_fastfetch(d: &mut Distro) -> bool {
    let Some(out) = crate::util::run_output("fastfetch", &["-c", "none", "--json"]) else {
        return false;
    };
    let mut found_os = false;
    for line in out.lines() {
        if line.contains("\"OS\"") {
            found_os = true;
        }
        if !found_os {
            continue;
        }
        if let Some(pos) = line.find("\"id\"") {
            if let Some(v) = json_string_value(line, pos) {
                if !v.is_empty() && v.len() < 64 {
                    d.id = v.to_string();
                    return true;
                }
            }
        }
        if let Some(pos) = line.find("\"idLike\"") {
            if let Some(v) = json_string_value(line, pos) {
                if !v.is_empty() && v.len() < 64 {
                    d.id_like = v.to_string();
                }
            }
        }
    }
    false
}

fn detect_os_release(d: &mut Distro) -> bool {
    let Ok(data) = fs::read("/etc/os-release") else {
        return false;
    };
    let text = String::from_utf8_lossy(&data);
    let mut found_id = false;
    for line in text.lines() {
        if !found_id && line.starts_with("ID=") {
            if let Some(v) = os_release_val(&line[3..], 64) {
                d.id = v;
                found_id = true;
            }
        } else if let Some(rest) = line.strip_prefix("ID_LIKE=") {
            if let Some(v) = os_release_val(rest, 64) {
                d.id_like = v;
            }
        }
    }
    found_id
}

/// Detect the running distro (empty `id` if unknown).
pub fn detect_distro() -> Distro {
    let mut d = Distro::default();
    if detect_fastfetch(&mut d) {
        return d;
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(name) = crate::util::run_output("sw_vers", &["-productName"]) {
            let name = name.lines().next().unwrap_or("").trim_end();
            if !name.is_empty() && name.len() < 64 {
                d.id = name.to_string();
                return d;
            }
        }
        d.id = "macos".into();
        d
    }
    #[cfg(not(target_os = "macos"))]
    {
        detect_os_release(&mut d);
        d
    }
}

// --- relief weights and colors ---

/// How tall a character stands in the 3D relief: dense glyphs are tall.
pub fn char_weight(c: char) -> f32 {
    let cp = c as u32;
    if cp < 0x80 {
        return match c {
            'M' => 1.00,
            'N' => 0.88,
            'm' => 0.76,
            'd' => 0.66,
            'h' | 'b' => 0.56,
            'y' => 0.46,
            'o' | 'n' => 0.38,
            's' => 0.30,
            '+' | '=' => 0.22,
            ':' => 0.18,
            '-' => 0.14,
            '`' => 0.08,
            '.' => 0.10,
            '/' => 0.12,
            '\'' => 0.06,
            ' ' => 0.0,
            // Generic: uppercase heavy, lowercase medium, punctuation light
            'A'..='Z' => 0.80,
            'a'..='z' => 0.50,
            '0'..='9' => 0.40,
            _ => 0.15,
        };
    }
    match cp {
        0x2588 => 1.00,                                  // █
        0x2593 => 0.75,                                  // ▓
        0x2592 => 0.50,                                  // ▒
        0x2591 => 0.25,                                  // ░
        0x2580 | 0x2584 | 0x258C | 0x2590 => 0.50,       // half blocks
        0x259B | 0x259C | 0x2599 | 0x259F => 0.75,       // three-quarter blocks
        0x2596 | 0x2597 | 0x2598 | 0x259D => 0.25,       // quarter blocks
        0x2500..=0x257F => 0.20,                         // box drawing
        0x2800..=0x28FF => {
            // Braille. The original weighted by popcount of the last UTF-8
            // byte, which carries a constant 0x80 bit: keep that scale.
            let b = 0x80u32 | (cp & 0x3F);
            b.count_ones() as f32 / 8.0
        }
        _ => 0.30,
    }
}

/// Default logo colors (outer, inner) for a distro name.
pub fn distro_colors(distro: &str) -> Option<(&'static str, &'static str)> {
    let d = distro.to_ascii_lowercase();
    Some(match d.as_str() {
        "gentoo" => ("\x1b[1;35m", "\x1b[1;37m"),
        "arch" => ("\x1b[1;36m", "\x1b[1;36m"),
        "ubuntu" | "debian" => ("\x1b[1;31m", "\x1b[1;37m"),
        "asahi" | "asahi2" | "fedora-asahi-remix" => ("\x1b[1;31m", "\x1b[1;37m"),
        "fedora" => ("\x1b[1;34m", "\x1b[1;37m"),
        s if s.starts_with("fedora-") => ("\x1b[1;34m", "\x1b[1;37m"),
        "nixos" => ("\x1b[1;34m", "\x1b[1;36m"),
        "void" => ("\x1b[1;32m", "\x1b[1;32m"),
        "alpine" => ("\x1b[1;34m", "\x1b[1;37m"),
        "opensuse-tumbleweed" | "opensuse-leap" | "opensuse" => ("\x1b[1;32m", "\x1b[1;37m"),
        "macos" => ("\x1b[1;36m", "\x1b[1;37m"),
        // violet #7c6cff outside, lavender #c9c1ff on the faces
        "singularity" | "singularitysys" => ("\x1b[1;38;5;99m", "\x1b[1;38;5;189m"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_parse_colors() {
        let mut l = Logo {
            raw: vec!["\x1b[1;31m@@\x1b[0m.".into(), "ab".into()],
            ..Default::default()
        };
        l.process();
        assert!(l.has_ansi);
        assert_eq!(l.cols, 3);
        assert_eq!(l.cells[0][0].color, 31);
        assert_eq!(l.cells[0][2].color, 0);
    }

    #[test]
    fn weights() {
        assert_eq!(char_weight('M'), 1.0);
        assert_eq!(char_weight('█'), 1.0);
        assert_eq!(char_weight('é'), 0.30);
        assert_eq!(char_weight('─'), 0.20);
        assert_eq!(char_weight('@'), 0.15);
    }

    #[test]
    fn os_release_values() {
        assert_eq!(os_release_val("\"arch\"\n", 64).as_deref(), Some("arch"));
        assert_eq!(os_release_val("\n", 64), None);
    }
}
