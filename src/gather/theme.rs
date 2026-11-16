//! Theme / icons / font / cursor, from GTK 2/3 and Qt (qt5ct/qt6ct, kdeglobals)
//! configs on Linux and `defaults` on macOS.

use crate::info::Info;

#[cfg(target_os = "macos")]
fn defaults(args: &[&str]) -> Option<String> {
    let out = crate::util::run_output("defaults", args)?;
    let l = out.lines().next()?.trim_end();
    (!l.is_empty()).then(|| l.to_string())
}

#[cfg(target_os = "macos")]
pub fn gather_theme(info: &mut Info) {
    let style = defaults(&["read", "-g", "AppleInterfaceStyle"]);
    info.add_info("Theme", style.as_deref().unwrap_or("Light"));
}

#[cfg(target_os = "macos")]
pub fn gather_icons(info: &mut Info) {
    info.add_info("Icons", "System");
}

#[cfg(target_os = "macos")]
pub fn gather_font(info: &mut Info) {
    if let Some(f) = defaults(&["read", "NSGlobalDomain", "AppleFont"]) {
        info.add_info("Font", &f);
    }
}

#[cfg(target_os = "macos")]
pub fn gather_cursor(_info: &mut Info) {}

#[cfg(not(target_os = "macos"))]
pub use linux::{gather_cursor, gather_font, gather_icons, gather_theme};

#[cfg(not(target_os = "macos"))]
mod linux {
    use super::Info;
    use std::fs;

    /// Value of `key` in an INI-ish file, optionally restricted to `[section]`
    /// (`None` searches every section). Surrounding quotes are stripped and
    /// values of `maxlen` bytes or more are rejected, like the fixed buffers
    /// they historically fit in.
    pub(super) fn read_ini_key(
        path: &str,
        section: Option<&str>,
        key: &str,
        maxlen: usize,
    ) -> Option<String> {
        let data = fs::read(path).ok()?;
        let text = String::from_utf8_lossy(&data);
        let mut in_section = section.is_none();
        for raw in text.lines() {
            let line = raw
                .trim_start_matches([' ', '\t'])
                .trim_end_matches([' ', '\t', '\r', '\n']);
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(sec) = section {
                    in_section = rest.starts_with(sec) && rest[sec.len()..].starts_with(']');
                }
                continue;
            }
            if !in_section {
                continue;
            }
            let Some(rest) = line.strip_prefix(key) else {
                continue;
            };
            let rest = rest.trim_start_matches([' ', '\t']);
            let Some(val) = rest.strip_prefix('=') else {
                continue;
            };
            let mut val = val.trim_start_matches([' ', '\t']);
            if val.len() >= 2
                && ((val.starts_with('"') && val.ends_with('"'))
                    || (val.starts_with('\'') && val.ends_with('\'')))
            {
                val = &val[1..val.len() - 1];
            }
            return (!val.is_empty() && val.len() < maxlen).then(|| val.to_string());
        }
        None
    }

    fn gtk3(key: &str, maxlen: usize) -> Option<String> {
        let home = crate::gather::home()?;
        read_ini_key(&format!("{home}/.config/gtk-3.0/settings.ini"), None, key, maxlen)
    }

    fn gtk2(key: &str, maxlen: usize) -> Option<String> {
        let home = crate::gather::home()?;
        read_ini_key(&format!("{home}/.gtkrc-2.0"), None, key, maxlen)
            .or_else(|| read_ini_key(&format!("{home}/.config/gtk-2.0/gtkrc"), None, key, maxlen))
    }

    /// qt5ct/qt6ct config: `$QT_QPA_PLATFORMTHEME` if it names one, else qt6ct, else qt5ct.
    fn qtct_conf() -> Option<String> {
        let home = crate::gather::home()?;
        let readable = |p: &str| fs::File::open(p).is_ok();
        if let Ok(platform) = std::env::var("QT_QPA_PLATFORMTHEME") {
            if platform.starts_with("qt") {
                let p = format!("{home}/.config/{platform}/{platform}.conf");
                if readable(&p) {
                    return Some(p);
                }
            }
        }
        ["qt6ct/qt6ct.conf", "qt5ct/qt5ct.conf"]
            .iter()
            .map(|v| format!("{home}/.config/{v}"))
            .find(|p| readable(p))
    }

    fn kdeglobals(section: &str, key: &str, maxlen: usize) -> Option<String> {
        let home = crate::gather::home()?;
        read_ini_key(&format!("{home}/.config/kdeglobals"), Some(section), key, maxlen)
    }

    fn qt_theme(maxlen: usize) -> Option<String> {
        if let Some(v) = qtct_conf().and_then(|p| read_ini_key(&p, Some("Appearance"), "style", maxlen)) {
            return Some(v);
        }
        // A qtct-managed kdeglobals style isn't the real theme
        kdeglobals("KDE", "widgetStyle", maxlen).filter(|v| !v.contains("ct-style"))
    }

    fn qt_icons(maxlen: usize) -> Option<String> {
        if let Some(v) = qtct_conf().and_then(|p| read_ini_key(&p, Some("Appearance"), "icon_theme", maxlen)) {
            return Some(v);
        }
        kdeglobals("Icons", "Theme", maxlen)
    }

    /// "Noto Sans 11" -> "Noto Sans (11pt)"
    fn font_pt_format(font: &str) -> String {
        if let Some((name, size)) = font.rsplit_once(' ') {
            if !size.is_empty() && size.bytes().all(|c| c.is_ascii_digit()) {
                return format!("{name} ({size}pt)");
            }
        }
        font.to_string()
    }

    /// "Noto Sans,10,-1,5,..." from a qtct config -> "Noto Sans (10pt)"
    fn qt_font() -> Option<String> {
        let conf = qtct_conf()?;
        let raw = read_ini_key(&conf, Some("Fonts"), "general", 192)?;
        let (name, rest) = raw.split_once(',')?;
        let size = rest.split(',').next().unwrap_or("");
        Some(format!("{name} ({size}pt)"))
    }

    fn append_tagged(dst: &mut String, val: &str, tag: &str) {
        if val.is_empty() {
            return;
        }
        if !dst.is_empty() {
            dst.push_str(", ");
        }
        dst.push_str(&format!("{val} [{tag}]"));
    }

    fn append_gtk_pair(dst: &mut String, gtk2: &str, gtk3: &str) {
        if !gtk2.is_empty() && gtk2 == gtk3 {
            append_tagged(dst, gtk3, "GTK2/3");
            return;
        }
        append_tagged(dst, gtk2, "GTK2");
        append_tagged(dst, gtk3, "GTK3");
    }

    fn combined(qt: Option<String>, g2: Option<String>, g3: Option<String>) -> String {
        let mut out = String::new();
        append_tagged(&mut out, &qt.unwrap_or_default(), "Qt");
        append_gtk_pair(&mut out, &g2.unwrap_or_default(), &g3.unwrap_or_default());
        out
    }

    pub fn gather_theme(info: &mut Info) {
        let out = combined(qt_theme(64), gtk2("gtk-theme-name", 64), gtk3("gtk-theme-name", 64));
        if !out.is_empty() {
            info.add_info("Theme", &out);
        }
    }

    pub fn gather_icons(info: &mut Info) {
        let out = combined(
            qt_icons(64),
            gtk2("gtk-icon-theme-name", 64),
            gtk3("gtk-icon-theme-name", 64),
        );
        if !out.is_empty() {
            info.add_info("Icons", &out);
        }
    }

    pub fn gather_font(info: &mut Info) {
        let out = combined(
            qt_font(),
            gtk2("gtk-font-name", 128).map(|f| font_pt_format(&f)),
            gtk3("gtk-font-name", 128).map(|f| font_pt_format(&f)),
        );
        if !out.is_empty() {
            info.add_info("Font", &out);
        }
    }

    pub fn gather_cursor(info: &mut Info) {
        if let Some(cursor) = gtk3("gtk-cursor-theme-name", 64) {
            match gtk3("gtk-cursor-theme-size", 16) {
                Some(size) => info.add_info("Cursor", &format!("{cursor} ({size}px)")),
                None => info.add_info("Cursor", &cursor),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn temp_ini(name: &str, body: &str) -> String {
            let p = std::env::temp_dir().join(format!("fetch-ini-{}-{name}", std::process::id()));
            fs::write(&p, body).unwrap();
            p.to_string_lossy().into_owned()
        }

        #[test]
        fn ini_keys_and_sections() {
            let p = temp_ini(
                "a",
                "[Settings]\ngtk-theme-name = \"Adwaita-dark\"\ngtk-theme-name2=x\n[Other]\nstyle=No\n\
                 [Appearance]\n  style=Fusion  \nicon_theme='breeze'\n",
            );
            assert_eq!(read_ini_key(&p, None, "gtk-theme-name", 64).as_deref(), Some("Adwaita-dark"));
            assert_eq!(read_ini_key(&p, Some("Appearance"), "style", 64).as_deref(), Some("Fusion"));
            assert_eq!(read_ini_key(&p, Some("Appearance"), "icon_theme", 64).as_deref(), Some("breeze"));
            assert_eq!(read_ini_key(&p, Some("Settings"), "style", 64), None);
            assert_eq!(read_ini_key(&p, None, "gtk-theme-name", 5), None);
            fs::remove_file(p).unwrap();
        }

        #[test]
        fn fonts_and_tags() {
            assert_eq!(font_pt_format("Noto Sans 11"), "Noto Sans (11pt)");
            assert_eq!(font_pt_format("Noto Sans"), "Noto Sans");
            assert_eq!(font_pt_format("Sans Bold 12 x"), "Sans Bold 12 x");
            assert_eq!(
                combined(Some("Fusion".into()), Some("Adw".into()), Some("Adw".into())),
                "Fusion [Qt], Adw [GTK2/3]"
            );
            assert_eq!(
                combined(None, Some("A".into()), Some("B".into())),
                "A [GTK2], B [GTK3]"
            );
        }
    }
}
