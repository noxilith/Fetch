//! `~/.config/fetch/config` parsing and the settings it controls.

use crate::util::{parse_integer, parse_number, strip_inline_hint, truncate_bytes};
use std::fs;

pub const MAX_HEIGHT: i64 = 200;
pub const MAX_CUSTOM: usize = 16;
pub const MAX_EXTRA_DISKS: usize = 8;
const MAX_FIELDS: usize = Field::COUNT + MAX_CUSTOM + 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Os,
    Host,
    Kernel,
    Uptime,
    Packages,
    Shell,
    Display,
    Wm,
    DisplayManager,
    Theme,
    Icons,
    Font,
    Cursor,
    Terminal,
    Cpu,
    Gpu,
    Memory,
    Swap,
    Disk,
    Ip,
    Battery,
    PowerProfile,
    Locale,
    Colors,
}

impl Field {
    pub const COUNT: usize = 24;

    pub const ALL: [Field; Field::COUNT] = [
        Field::Os,
        Field::Host,
        Field::Kernel,
        Field::Uptime,
        Field::Packages,
        Field::Shell,
        Field::Display,
        Field::Wm,
        Field::DisplayManager,
        Field::Theme,
        Field::Icons,
        Field::Font,
        Field::Cursor,
        Field::Terminal,
        Field::Cpu,
        Field::Gpu,
        Field::Memory,
        Field::Swap,
        Field::Disk,
        Field::Ip,
        Field::Battery,
        Field::PowerProfile,
        Field::Locale,
        Field::Colors,
    ];

    const NAMES: [&'static str; Field::COUNT] = [
        "os",
        "host",
        "kernel",
        "uptime",
        "packages",
        "shell",
        "display",
        "wm",
        "displaymanager",
        "theme",
        "icons",
        "font",
        "cursor",
        "terminal",
        "cpu",
        "gpu",
        "memory",
        "swap",
        "disk",
        "ip",
        "battery",
        "powerprofile",
        "locale",
        "colors",
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_name(name: &str) -> Option<Field> {
        Field::NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| Field::ALL[i])
    }
}

/// One entry in the displayed field list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Field(Field),
    Separator,
    Custom(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

pub struct Config {
    pub enabled: [bool; Field::COUNT],
    pub order: Vec<Item>,
    pub custom: Vec<(String, String)>,
    /// SGR code used for labels, e.g. "35".
    pub label_color: String,
    /// 0 = auto
    pub height: i64,
    pub size: f32,
    pub speed: Option<f32>,
    pub spin: Option<(bool, bool)>,
    pub boxed: bool,
    pub minimal: bool,
    pub show_title: bool,
    pub shading: String,
    pub shading_mode: String,
    pub separator: String,
    pub depth: f32,
    pub depth_user_set: bool,
    /// Full SGR escape (e.g. "\x1b[1;35m"), empty if unset.
    pub logo_outer: String,
    pub logo_inner: String,
    pub extra_disks: Vec<String>,
    pub v_align: VAlign,
    pub h_align: HAlign,
    pub light: [f32; 3],
}

impl Default for Config {
    fn default() -> Self {
        use Field::*;
        let defaults = [
            Os, Host, Kernel, Uptime, Packages, Shell, Display, Wm, Theme, Icons, Font, Cursor,
            Terminal, Cpu, Gpu, Memory, Swap, Disk, Ip, Battery, Locale, Colors,
        ];
        let mut enabled = [false; Field::COUNT];
        for f in defaults {
            enabled[f.index()] = true;
        }
        Config {
            enabled,
            order: defaults.iter().map(|&f| Item::Field(f)).collect(),
            custom: Vec::new(),
            label_color: "35".into(),
            height: 0,
            size: 1.0,
            speed: None,
            spin: None,
            boxed: false,
            minimal: false,
            show_title: true,
            shading: String::new(),
            shading_mode: String::new(),
            separator: "-".into(),
            depth: 1.0,
            depth_user_set: false,
            logo_outer: String::new(),
            logo_inner: String::new(),
            extra_disks: Vec::new(),
            v_align: VAlign::Top,
            h_align: HAlign::Left,
            light: [0.4082, 0.8165, -0.4082],
        }
    }
}

fn color_code(name: &str) -> Option<&'static str> {
    Some(match name {
        "red" => "31",
        "green" => "32",
        "yellow" => "33",
        "blue" => "34",
        "magenta" => "35",
        "cyan" => "36",
        "white" => "37",
        _ => return None,
    })
}

/// "magenta" -> "\x1b[1;35m"; anything else is used as a raw SGR parameter.
fn logo_color_escape(val: &str) -> String {
    let code = color_code(val).unwrap_or_else(|| truncate_bytes(val, 31));
    format!("\x1b[1;{code}m")
}

fn truthy(val: &str) -> bool {
    val == "1"
        || val.eq_ignore_ascii_case("y")
        || val.eq_ignore_ascii_case("yes")
        || val.eq_ignore_ascii_case("true")
}

impl Config {
    /// Load `~/.config/fetch/config` if it exists.
    pub fn load(&mut self) {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = std::path::Path::new(&home).join(".config/fetch/config");
        if let Ok(data) = fs::read(path) {
            self.parse(&String::from_utf8_lossy(&data));
        }
    }

    /// Apply config-file text. Once a config exists only the fields it lists are shown.
    pub fn parse(&mut self, text: &str) {
        self.enabled = [false; Field::COUNT];
        self.order.clear();

        for raw in text.lines() {
            let line = raw.trim_end_matches([' ', '\r', '\n']);
            let line = line.trim_start_matches([' ', '\t']);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            self.parse_line(line);
        }
    }

    fn push_order(&mut self, item: Item) {
        if self.order.len() < MAX_FIELDS {
            self.order.push(item);
        }
    }

    fn parse_line(&mut self, line: &str) {
        if let Some(v) = line.strip_prefix("label_color=") {
            let v = strip_inline_hint(v);
            self.label_color = color_code(v).unwrap_or_else(|| truncate_bytes(v, 15)).into();
        } else if let Some(v) = line.strip_prefix("height=") {
            if let Some(n) = parse_integer("height", strip_inline_hint(v), 1, MAX_HEIGHT) {
                self.height = n;
            }
        } else if let Some(v) = line.strip_prefix("size=") {
            if let Some(n) = parse_number("size", strip_inline_hint(v), 0.5, 5.0) {
                self.size = n;
            }
        } else if let Some(v) = line.strip_prefix("speed=") {
            if let Some(n) = parse_number("speed", strip_inline_hint(v), -1000.0, 1000.0) {
                self.speed = Some(n);
            }
        } else if let Some(v) = line.strip_prefix("spin=") {
            let v = strip_inline_hint(v);
            let has = |c: char| v.contains(c) || v.contains(c.to_ascii_uppercase());
            self.spin = Some((has('x'), has('y')));
        } else if let Some(v) = line.strip_prefix("show_title=") {
            self.show_title = truthy(strip_inline_hint(v));
        } else if let Some(v) = line.strip_prefix("box=") {
            self.boxed = truthy(strip_inline_hint(v));
        } else if let Some(v) = line.strip_prefix("minimal=") {
            self.minimal = truthy(strip_inline_hint(v));
        } else if let Some(v) = line.strip_prefix("shading=") {
            // freeform: no hint stripping
            self.shading = truncate_bytes(v, 127).into();
        } else if let Some(v) = line.strip_prefix("shading_mode=") {
            self.shading_mode = truncate_bytes(strip_inline_hint(v), 15).into();
        } else if let Some(v) = line.strip_prefix("separator=") {
            // freeform: no hint stripping
            self.separator = truncate_bytes(v, 7).into();
        } else if let Some(v) = line.strip_prefix("depth=") {
            if let Some(n) = parse_number("depth", strip_inline_hint(v), 0.1, 10.0) {
                self.depth = n;
                self.depth_user_set = true;
            }
        } else if let Some(v) = line.strip_prefix("logo_outer=") {
            self.logo_outer = logo_color_escape(strip_inline_hint(v));
        } else if let Some(v) = line.strip_prefix("logo_inner=") {
            self.logo_inner = logo_color_escape(strip_inline_hint(v));
        } else if let Some(v) = line.strip_prefix("light=") {
            let dir = match strip_inline_hint(v) {
                "top-left" => Some([0.41, 0.82, -0.41]),
                "top-right" => Some([-0.41, 0.82, -0.41]),
                "top" => Some([0.0, 0.89, -0.45]),
                "left" => Some([0.82, 0.41, -0.41]),
                "right" => Some([-0.82, 0.41, -0.41]),
                "front" => Some([0.0, 0.0, -1.0]),
                "bottom-left" => Some([0.41, -0.82, -0.41]),
                "bottom-right" => Some([-0.41, -0.82, -0.41]),
                _ => None,
            };
            if let Some(d) = dir {
                self.light = d;
            }
        } else if line.len() >= 5 && line.as_bytes()[..5].eq_ignore_ascii_case(b"disk=")
        {
            // freeform: paths may contain spaces
            let path = &line[5..];
            if !path.is_empty() && self.extra_disks.len() < MAX_EXTRA_DISKS {
                self.extra_disks.push(truncate_bytes(path, 127).into());
            }
            if !self.enabled[Field::Disk.index()] && self.order.len() < MAX_FIELDS {
                self.enabled[Field::Disk.index()] = true;
                self.order.push(Item::Field(Field::Disk));
            }
        } else if let Some(v) = line.strip_prefix("v_alignment=") {
            match strip_inline_hint(v) {
                "top" => self.v_align = VAlign::Top,
                "center" => self.v_align = VAlign::Center,
                "bottom" => self.v_align = VAlign::Bottom,
                _ => {}
            }
        } else if let Some(v) = line.strip_prefix("h_alignment=") {
            match strip_inline_hint(v) {
                "left" => self.h_align = HAlign::Left,
                "center" => self.h_align = HAlign::Center,
                "right" => self.h_align = HAlign::Right,
                _ => {}
            }
        } else if line == "---" {
            self.push_order(Item::Separator);
        } else if let Some(rest) = line.strip_prefix("custom_") {
            if let Some(eq) = rest.find('=') {
                if eq > 0 && self.custom.len() < MAX_CUSTOM && self.order.len() < MAX_FIELDS {
                    let label = truncate_bytes(&rest[..eq], 63).to_string();
                    let value = truncate_bytes(&rest[eq + 1..], 255).to_string();
                    self.custom.push((label, value));
                    let idx = self.custom.len() - 1;
                    self.order.push(Item::Custom(idx));
                }
            }
        } else if let Some(f) = Field::from_name(line) {
            if !self.enabled[f.index()] && self.order.len() < MAX_FIELDS {
                self.enabled[f.index()] = true;
                self.order.push(Item::Field(f));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_replaces_default_fields() {
        let mut c = Config::default();
        c.parse("cpu\n---\nCPU\nmemory\ncustom_Test=visible\n");
        assert_eq!(
            c.order,
            vec![
                Item::Field(Field::Cpu),
                Item::Separator,
                Item::Field(Field::Memory),
                Item::Custom(0)
            ]
        );
        assert_eq!(c.custom[0], ("Test".into(), "visible".into()));
    }

    #[test]
    fn settings_and_hints() {
        let mut c = Config::default();
        c.parse(
            "label_color=cyan (red, green)\nshow_title=0 (hide title)\nbox=yes\n\
             separator=~\nspeed=junk\nspeed=2\nlogo_outer=red\ndisk=/home dir\n",
        );
        assert_eq!(c.label_color, "36");
        assert!(!c.show_title && c.boxed);
        assert_eq!(c.separator, "~");
        assert_eq!(c.speed, Some(2.0));
        assert_eq!(c.logo_outer, "\x1b[1;31m");
        assert_eq!(c.extra_disks, vec!["/home dir".to_string()]);
        assert_eq!(c.order, vec![Item::Field(Field::Disk)]);
    }
}
