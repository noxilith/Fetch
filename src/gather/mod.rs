//! System info gatherers. Everything is read natively from /proc, /sys and
//! libc where possible; the few external tools used are run without a shell.

mod display;
mod gpu;
mod hw;
mod net;
mod packages;
mod theme;

use crate::config::{Config, Field};
use crate::info::Info;
use crate::util::{basename, cmd_version, read_first_line};
use std::ffi::CStr;
use std::fs;

#[cfg(target_os = "macos")]
pub(crate) mod mac;

/// Run the gatherer for `field`. `Colors` and separators are handled by the caller.
pub fn gather(field: Field, info: &mut Info, cfg: &Config) {
    match field {
        Field::Os => os(info),
        Field::Host => host(info),
        Field::Kernel => kernel(info),
        Field::Uptime => uptime(info),
        Field::Packages => packages::gather(info),
        Field::Shell => shell(info),
        Field::Display => display::gather(info),
        Field::Wm => wm(info),
        Field::DisplayManager => display_manager(info),
        Field::Theme => theme::gather_theme(info),
        Field::Icons => theme::gather_icons(info),
        Field::Font => theme::gather_font(info),
        Field::Cursor => theme::gather_cursor(info),
        Field::Terminal => terminal(info),
        Field::Cpu => hw::cpu(info),
        Field::Gpu => gpu::gather(info),
        Field::Memory => hw::memory(info),
        Field::Swap => hw::swap(info),
        Field::Disk => hw::disk(info, &cfg.extra_disks),
        Field::Ip => net::gather(info),
        Field::Battery => hw::battery(info),
        Field::PowerProfile => power_profile(info),
        Field::Locale => locale(info),
        Field::Colors => {}
    }
}

/// Re-run the cheap, file-only gatherers for the live refresh tick.
pub fn refresh(field: Field, info: &mut Info) {
    match field {
        Field::Uptime => uptime(info),
        Field::Memory => hw::memory(info),
        Field::Swap => hw::swap(info),
        Field::Battery => hw::battery(info),
        _ => {}
    }
}

/// 31 (red) at 80%+, yellow at 50%+, otherwise green.
pub(crate) fn usage_color(pct: i32) -> &'static str {
    if pct >= 80 {
        "31"
    } else if pct >= 50 {
        "93"
    } else {
        "32"
    }
}

pub(crate) struct Uname {
    pub sysname: String,
    pub release: String,
    pub machine: String,
}

fn field_str(f: &[libc::c_char]) -> String {
    // SAFETY: utsname fields are NUL-terminated.
    unsafe { CStr::from_ptr(f.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn uname() -> Uname {
    // SAFETY: utsname is plain data; uname fills it.
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        libc::uname(&mut u);
        Uname {
            sysname: field_str(&u.sysname),
            release: field_str(&u.release),
            machine: field_str(&u.machine),
        }
    }
}

pub(crate) fn home() -> Option<String> {
    std::env::var("HOME").ok()
}

fn os(info: &mut Info) {
    let u = uname();
    #[cfg(target_os = "macos")]
    {
        if let Some(v) = mac::sysctl_str("kern.osproductversion") {
            info.add_info("OS", &format!("macOS {v} {}", u.machine));
        } else {
            info.add_info("OS", &format!("{} {}", u.sysname, u.machine));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut pretty = String::new();
        if let Ok(data) = fs::read("/etc/os-release") {
            let text = String::from_utf8_lossy(&data);
            if let Some(val) = text.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")) {
                let val = val.strip_prefix(['\'', '"']).unwrap_or(val);
                let val = val.strip_suffix(['\'', '"']).unwrap_or(val);
                pretty = val.to_string();
            }
        }
        if pretty.is_empty() {
            pretty = "Linux".into();
        }
        info.add_info("OS", &format!("{pretty} {}", u.machine));
    }
}

fn host(info: &mut Info) {
    #[cfg(target_os = "macos")]
    {
        if let Some(model) = mac::sysctl_str("hw.model") {
            info.add_info("Host", &model);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut model = String::new();
        let mut vendor = String::new();
        // Device-tree first (ARM/Apple Silicon), then DMI (x86)
        if let Some(m) = read_first_line("/proc/device-tree/model") {
            model = m;
        } else if let Some(m) = read_first_line("/sys/class/dmi/id/product_name") {
            model = m;
            vendor = read_first_line("/sys/class/dmi/id/sys_vendor").unwrap_or_default();
        }
        // Some boards already embed the vendor name in product_name
        if !vendor.is_empty()
            && model.len() >= vendor.len()
            && model.as_bytes()[..vendor.len()].eq_ignore_ascii_case(vendor.as_bytes())
        {
            vendor.clear();
        }
        if !model.is_empty() {
            if vendor.is_empty() {
                info.add_info("Host", &model);
            } else {
                info.add_info("Host", &format!("{vendor} {model}"));
            }
        }
    }
}

fn kernel(info: &mut Info) {
    let u = uname();
    info.add_info("Kernel", &format!("{} {}", u.sysname, u.release));
}

fn plural(n: i64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn uptime(info: &mut Info) {
    #[cfg(target_os = "macos")]
    let secs = match mac::uptime_secs() {
        Some(s) => s,
        None => return,
    };
    #[cfg(not(target_os = "macos"))]
    let secs: f64 = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok()))
        .unwrap_or(0.0);
    let Some(val) = format_uptime(secs) else {
        return;
    };
    info.add_info("Uptime", &val);
}

fn format_uptime(secs: f64) -> Option<String> {
    let total = secs as i64;
    let days = total / 86400;
    let hours = (total % 86400) / 3600;
    let mins = (total % 3600) / 60;
    Some(if days > 0 {
        format!(
            "{days} day{}, {hours} hour{}, {mins} min{}",
            plural(days),
            plural(hours),
            plural(mins)
        )
    } else if hours > 0 {
        format!("{hours} hour{}, {mins} min{}", plural(hours), plural(mins))
    } else {
        format!("{mins} min{}", plural(mins))
    })
}

const SHELLS: [&str; 12] = [
    "bash", "zsh", "fish", "dash", "sh", "nu", "elvish", "tcsh", "csh", "xonsh", "ksh", "ion",
];

fn shell(info: &mut Info) {
    // Detect the actual running shell from the parent process: $SHELL is the
    // login shell, which may differ (e.g. fish launched from bash).
    let ppid = std::os::unix::process::parent_id();
    #[cfg(target_os = "macos")]
    let parent = mac::process_comm(ppid);
    #[cfg(not(target_os = "macos"))]
    let parent = read_first_line(&format!("/proc/{ppid}/comm"));

    let shell = parent
        .filter(|c| SHELLS.contains(&c.as_str()))
        // Fall back to $SHELL (login shell)
        .or_else(|| std::env::var("SHELL").ok());
    let Some(shell) = shell else {
        return;
    };
    let name = basename(&shell);
    let version = cmd_version(&shell);
    if version.is_empty() {
        info.add_info("Shell", name);
    } else {
        info.add_info("Shell", &format!("{name} {version}"));
    }
}

/// Process command names under /proc in ascending pid order.
#[cfg(not(target_os = "macos"))]
fn proc_comms() -> impl Iterator<Item = String> {
    let mut pids: Vec<u32> = fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse().ok()))
        .filter(|&p| p > 0)
        .collect();
    pids.sort_unstable();
    pids.into_iter()
        .filter_map(|pid| read_first_line(&format!("/proc/{pid}/comm")))
}

fn wm(info: &mut Info) {
    #[cfg(target_os = "macos")]
    {
        info.add_info("WM", "Aqua");
    }
    #[cfg(not(target_os = "macos"))]
    {
        const KNOWN_WMS: [&str; 10] = [
            "dwl", "sway", "river", "labwc", "weston", "i3", "bspwm", "openbox", "awesome", "dwm",
        ];
        let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let is_wayland = nonempty("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland");
        let desktop = nonempty("XDG_CURRENT_DESKTOP");

        // Process detection is most accurate (e.g. dwl sets
        // XDG_CURRENT_DESKTOP=sway for compat), so it goes before the DE name.
        let mut wm = String::new();
        // true only if `wm` holds an actual invocable binary name
        let mut wm_is_binary = false;

        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            wm = "Hyprland".into();
            wm_is_binary = true;
        }
        if wm.is_empty() {
            if let Some(c) = proc_comms().find(|c| KNOWN_WMS.contains(&c.as_str())) {
                wm = c;
                wm_is_binary = true;
            }
        }
        // Fall back to the DE-to-WM mapping from XDG_CURRENT_DESKTOP
        if wm.is_empty() {
            if let Some(desktop) = &desktop {
                let first = desktop.split(':').next().unwrap_or("");
                let mapped = match first.to_ascii_lowercase().as_str() {
                    "kde" | "deepin" => Some("KWin"),
                    "gnome" | "budgie" => Some("Mutter"),
                    "xfce" => Some("xfwm4"),
                    "cinnamon" => Some("Muffin"),
                    "mate" => Some("Marco"),
                    "lxqt" => Some("Openbox"),
                    _ => None,
                };
                match mapped {
                    Some(m) => wm = m.into(),
                    None => {
                        // Not a known DE name: likely the WM's own name (e.g.
                        // niri), which is also a real invocable binary.
                        wm = crate::util::truncate_bytes(desktop, 63).into();
                        wm_is_binary = true;
                    }
                }
            }
        }

        if !wm.is_empty() {
            let version = if wm_is_binary {
                cmd_version(&wm)
            } else {
                String::new()
            };
            let wl = if is_wayland { " (Wayland)" } else { "" };
            if version.is_empty() {
                info.add_info("WM", &format!("{wm}{wl}"));
            } else {
                info.add_info("WM", &format!("{wm} {version}{wl}"));
            }
        }
    }
}

fn display_manager(info: &mut Info) {
    #[cfg(not(target_os = "macos"))]
    {
        const LABELS: [(&str, &str); 10] = [
            ("sddm", "SDDM"),
            ("gdm", "GDM"),
            ("gdm3", "GDM"),
            ("lightdm", "LightDM"),
            ("lxdm", "LXDM"),
            ("greetd", "greetd"),
            ("xdm", "XDM"),
            ("slim", "SLiM"),
            ("lemurs", "lemurs"),
            ("ly", "ly"),
        ];
        // Scanning running processes is portable across init systems and
        // reflects what's running right now rather than what's just enabled.
        let label = proc_comms().find_map(|c| {
            LABELS
                .iter()
                .find(|(id, _)| *id == c.as_str())
                .map(|(_, l)| *l)
        });
        if let Some(l) = label {
            info.add_info("Display Manager", l);
        }
    }
    #[cfg(target_os = "macos")]
    let _ = info;
}

const NON_TERMINALS: [&str; 10] = [
    "bash", "zsh", "sh", "dash", "fish", "nu", "elvish", "xonsh", "tcsh", "csh",
];

fn terminal(info: &mut Info) {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let mut term = String::new();
    if let Some(tp) = env("TERM_PROGRAM") {
        term = tp;
    } else if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        term = "kitty".into();
    } else if std::env::var_os("ALACRITTY_LOG").is_some() {
        term = "alacritty".into();
    } else if std::env::var_os("WEZTERM_PANE").is_some() {
        term = "wezterm".into();
    } else if std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some() {
        term = "ghostty".into();
    } else if let Ok(t) = std::env::var("TERMINAL_EMULATOR") {
        term = t;
    } else {
        // Walk up the process tree looking for the first non-shell
        let mut pid = std::process::id();
        for _ in 0..4 {
            #[cfg(target_os = "macos")]
            let (ppid, comm) = match mac::parent_of(pid) {
                Some(ppid) => (ppid, mac::process_comm(ppid)),
                None => break,
            };
            #[cfg(not(target_os = "macos"))]
            let (ppid, comm) = {
                let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
                    break;
                };
                let ppid = status
                    .lines()
                    .find_map(|l| l.strip_prefix("PPid:"))
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .unwrap_or(0);
                if ppid == 0 {
                    break;
                }
                (ppid, read_first_line(&format!("/proc/{ppid}/comm")))
            };
            pid = ppid;
            let Some(comm) = comm else {
                break;
            };
            if !NON_TERMINALS.contains(&comm.as_str()) {
                term = comm;
                break;
            }
        }
    }
    if term.is_empty() {
        return;
    }
    let term = crate::util::truncate_bytes(&term, 63).to_string();
    let version = cmd_version(&term);
    if version.is_empty() {
        info.add_info("Terminal", &term);
    } else {
        info.add_info("Terminal", &format!("{term} {version}"));
    }
}

fn power_profile(info: &mut Info) {
    #[cfg(not(target_os = "macos"))]
    {
        // Kernel ACPI platform profile is the simplest source; typical values
        // are "power saver", "balanced", "performance".
        let profile = read_first_line("/sys/firmware/acpi/platform_profile").or_else(|| {
            // Fall back to power-profiles-daemon, common across desktop distros.
            crate::util::run_output("powerprofilesctl", &["get"])
                .and_then(|o| o.lines().next().map(|l| l.trim_end_matches('\r').to_string()))
                .filter(|l| !l.is_empty())
        });
        let Some(profile) = profile else {
            return;
        };
        // Title-case the first letter and swap hyphens for spaces
        let mut chars = profile.chars();
        let first = chars.next().unwrap();
        let mut out = String::new();
        out.push(first.to_ascii_uppercase());
        out.extend(chars);
        info.add_info("Power Profile", &out.replace('-', " "));
    }
    #[cfg(target_os = "macos")]
    let _ = info;
}

fn locale(info: &mut Info) {
    if let Some(lang) = std::env::var("LANG").ok().filter(|l| !l.is_empty()) {
        info.add_info("Locale", &lang);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_formats() {
        assert_eq!(format_uptime(59.0).unwrap(), "0 mins");
        assert_eq!(format_uptime(3660.0).unwrap(), "1 hour, 1 min");
        assert_eq!(
            format_uptime(90061.0).unwrap(),
            "1 day, 1 hour, 1 min"
        );
        assert_eq!(
            format_uptime(2.0 * 86400.0 + 7200.0 + 120.0).unwrap(),
            "2 days, 2 hours, 2 mins"
        );
    }

    #[test]
    fn usage_colors() {
        assert_eq!(usage_color(10), "32");
        assert_eq!(usage_color(50), "93");
        assert_eq!(usage_color(80), "31");
    }
}
