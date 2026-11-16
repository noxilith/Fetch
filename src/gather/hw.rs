//! Hardware stats: CPU, memory, swap, disks, battery.

use super::usage_color;
use crate::info::Info;

#[cfg(not(target_os = "macos"))]
use crate::util::read_first_line;
use std::fs;

// --- CPU ---

#[cfg(target_os = "macos")]
pub fn cpu(info: &mut Info) {
    use super::mac;
    let Some(name) = mac::sysctl_str("machdep.cpu.brand_string") else {
        return;
    };
    let cores = mac::sysctl_int("hw.ncpu");
    let freq_hz = mac::sysctl_long("hw.cpufrequency_max");
    if freq_hz > 0 {
        let ghz = freq_hz as f32 / 1e9;
        info.add_info("CPU", &format!("{name} ({cores}) @ {ghz:.2} GHz"));
    } else if cores > 0 {
        info.add_info("CPU", &format!("{name} ({cores})"));
    } else {
        info.add_info("CPU", &name);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn cpu(info: &mut Info) {
    let mut name = String::new();
    let mut cores = 0;

    // x86 model name first
    if let Ok(data) = fs::read("/proc/cpuinfo") {
        let text = String::from_utf8_lossy(&data);
        for line in text.lines() {
            if name.is_empty() && line.starts_with("model name") {
                if let Some((_, val)) = line.split_once(':') {
                    let val = val.trim_start_matches(' ');
                    if !val.is_empty() && val.len() < 128 {
                        name = val.split(" @ ").next().unwrap_or(val).to_string();
                    }
                }
            }
            if line.starts_with("processor") {
                cores += 1;
            }
        }
    }

    // ARM/Apple Silicon: extract the chip from the device-tree model, e.g.
    // "M1" from "Apple MacBook Air (M1, 2020)"
    if name.is_empty() {
        if let Some(model) = read_first_line("/proc/device-tree/model") {
            if let Some(p) = model.find('(') {
                let rest = &model[p + 1..];
                if let Some(end) = rest.find(',').or_else(|| rest.find(')')) {
                    name = format!("Apple {}", &rest[..end]);
                }
            }
        }
    }

    // Max frequency from cpufreq
    let mut max_khz = 0i64;
    if let Ok(dir) = fs::read_dir("/sys/devices/system/cpu/cpufreq") {
        for e in dir.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if !n.starts_with("policy") {
                continue;
            }
            let path = format!("/sys/devices/system/cpu/cpufreq/{n}/cpuinfo_max_freq");
            if let Some(khz) = fs::read_to_string(path)
                .ok()
                .and_then(|s| s.trim().parse::<i64>().ok())
            {
                max_khz = max_khz.max(khz);
            }
        }
    }
    let max_ghz = if max_khz > 0 {
        max_khz as f32 / 1_000_000.0
    } else {
        0.0
    };

    if !name.is_empty() {
        if cores > 0 && max_ghz > 0.0 {
            info.add_info("CPU", &format!("{name} ({cores}) @ {max_ghz:.2} GHz"));
        } else if cores > 0 {
            info.add_info("CPU", &format!("{name} ({cores})"));
        } else {
            info.add_info("CPU", &name);
        }
    }
}

// --- memory + swap ---

/// "MemTotal:   123 kB" style value (in kB) for `key`.
#[cfg(not(target_os = "macos"))]
fn meminfo(text: &str, key: &str) -> i64 {
    text.lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn usage_line(used: String, total: String, pct: i32) -> String {
    format!(
        "{used} / {total} (\x1b[{}m{pct}%\x1b[0m)",
        usage_color(pct)
    )
}

#[cfg(target_os = "macos")]
pub fn memory(info: &mut Info) {
    use super::mac;
    let total = mac::sysctl_long("hw.memsize");
    if total <= 0 {
        return;
    }
    let (mut page_size, mut free, mut inactive, mut speculative) = (4096i64, 0i64, 0i64, 0i64);
    if let Some(out) = crate::util::run_output("vm_stat", &[]) {
        let num = |line: &str, key: &str| -> Option<i64> {
            line.strip_prefix(key)?
                .trim()
                .trim_end_matches('.')
                .parse()
                .ok()
        };
        for line in out.lines() {
            if let Some(v) = num(line, "Pages free:") {
                free = v;
            } else if let Some(v) = num(line, "Pages inactive:") {
                inactive = v;
            } else if let Some(v) = num(line, "Pages speculative:") {
                speculative = v;
            } else if let Some(rest) = line.split_once("page size of ") {
                if let Some(v) = rest.1.split(|c: char| !c.is_ascii_digit()).next() {
                    page_size = v.parse().unwrap_or(page_size);
                }
            }
        }
    }
    let avail = (free + inactive + speculative) * page_size;
    let gib = 1073741824.0f32;
    let pct = ((total - avail) * 100 / total) as i32;
    info.add_info(
        "Memory",
        &usage_line(
            format!("{:.2} GiB", (total - avail) as f32 / gib),
            format!("{:.2} GiB", total as f32 / gib),
            pct,
        ),
    );
}

#[cfg(not(target_os = "macos"))]
pub fn memory(info: &mut Info) {
    let Ok(text) = fs::read_to_string("/proc/meminfo") else {
        return;
    };
    let total = meminfo(&text, "MemTotal:");
    let avail = meminfo(&text, "MemAvailable:");
    if total <= 0 {
        return;
    }
    let used_gib = (total - avail) as f32 / 1048576.0;
    let total_gib = total as f32 / 1048576.0;
    let pct = ((total - avail) * 100 / total) as i32;
    info.add_info(
        "Memory",
        &usage_line(
            format!("{used_gib:.2} GiB"),
            format!("{total_gib:.2} GiB"),
            pct,
        ),
    );
}

#[cfg(target_os = "macos")]
pub fn swap(info: &mut Info) {
    let Some(s) = super::mac::sysctl_str("vm.swapusage") else {
        return;
    };
    // "total = 2048.00M  used = 1024.00M  free = 1024.00M  (encrypted)"
    let num = |key: &str| -> f32 {
        s.split_once(key)
            .and_then(|(_, r)| {
                r.trim_start_matches([' ', '='])
                    .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                    .next()
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(0.0)
    };
    let (total_mb, used_mb) = (num("total"), num("used"));
    if total_mb <= 0.0 {
        return;
    }
    let pct = (used_mb * 100.0 / total_mb) as i32;
    info.add_info(
        "Swap",
        &usage_line(
            format!("{used_mb:.2} MiB"),
            format!("{total_mb:.2} MiB"),
            pct,
        ),
    );
}

#[cfg(not(target_os = "macos"))]
pub fn swap(info: &mut Info) {
    let Ok(text) = fs::read_to_string("/proc/meminfo") else {
        return;
    };
    let total = meminfo(&text, "SwapTotal:");
    let free = meminfo(&text, "SwapFree:");
    if total <= 0 {
        return;
    }
    let used = total - free;
    let pct = (used * 100 / total) as i32;
    let (u, t) = if total >= 1048576 {
        (
            format!("{:.2} GiB", used as f32 / 1048576.0),
            format!("{:.2} GiB", total as f32 / 1048576.0),
        )
    } else {
        (
            format!("{:.2} MiB", used as f32 / 1024.0),
            format!("{:.2} MiB", total as f32 / 1024.0),
        )
    };
    info.add_info("Swap", &usage_line(u, t, pct));
}

// --- disks ---

fn filesystem_type(path: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        super::mac::fstype_of(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let data = fs::read("/proc/mounts").ok()?;
        String::from_utf8_lossy(&data).lines().find_map(|l| {
            let mut f = l.split_whitespace();
            let (_dev, mnt, fs) = (f.next()?, f.next()?, f.next()?);
            (mnt == path).then(|| fs.to_string())
        })
    }
}

fn disk_one(info: &mut Info, path: &str) {
    let Ok(cpath) = std::ffi::CString::new(path) else {
        return;
    };
    // SAFETY: statvfs is plain data; the call fills it for a valid C path.
    let st = unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(cpath.as_ptr(), &mut st) != 0 {
            return;
        }
        st
    };
    if st.f_blocks == 0 {
        return;
    }

    let gib = 1073741824.0f32;
    let total_gib = st.f_blocks as f32 * st.f_frsize as f32 / gib;
    let free_gib = st.f_bfree as f32 * st.f_frsize as f32 / gib;
    let used_gib = total_gib - free_gib;
    let pct = (used_gib * 100.0 / total_gib) as i32;

    let label = format!("Disk ({path})");
    let label = crate::util::truncate_bytes(&label, 63);
    let mut line = usage_line(
        format!("{used_gib:.2} GiB"),
        format!("{total_gib:.2} GiB"),
        pct,
    );
    if let Some(fstype) = filesystem_type(path) {
        line.push_str(&format!(" - {fstype}"));
    }
    info.add_info(label, &line);
}

pub fn disk(info: &mut Info, extra: &[String]) {
    disk_one(info, "/");
    for p in extra {
        disk_one(info, p);
    }
}

// --- battery ---

fn battery_line(capacity: i32, time: &str, status: &str) -> String {
    let color = if capacity >= 50 {
        "32"
    } else if capacity >= 20 {
        "93"
    } else {
        "31"
    };
    let head = format!("\x1b[{color}m{capacity}%\x1b[0m");
    match (time.is_empty(), status.is_empty()) {
        (false, false) => format!("{head} ({time}) [{status}]"),
        (_, false) => format!("{head} [{status}]"),
        _ => head,
    }
}

fn remaining(mins_left: i32) -> String {
    let (hours, mins) = (mins_left / 60, mins_left % 60);
    let s = |n: i32| if n == 1 { "" } else { "s" };
    if hours > 0 {
        format!("{hours} hour{}, {mins} min{} remaining", s(hours), s(mins))
    } else {
        format!("{mins} min{} remaining", s(mins))
    }
}

#[cfg(target_os = "macos")]
pub fn battery(info: &mut Info) {
    if let Some(b) = super::mac::battery() {
        let (time, status) = match (b.charging, b.time_to_empty) {
            (true, _) => (String::new(), "Charging"),
            (false, t) if t > 0 => (remaining(t), "Discharging"),
            (false, t) if t >= 0 => (String::new(), "Discharging"),
            _ => (String::new(), ""),
        };
        info.add_info("Battery", &battery_line(b.capacity, &time, status));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn battery(info: &mut Info) {
    let base = "/sys/class/power_supply";
    // First power supply that has a capacity file
    let mut names: Vec<String> = fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    let Some(bat) = names
        .into_iter()
        .find(|n| fs::metadata(format!("{base}/{n}/capacity")).is_ok())
    else {
        return;
    };
    let num = |file: &str| -> i64 {
        fs::read_to_string(format!("{base}/{bat}/{file}"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    };

    // Prefer energy_now/energy_full for an accurate percentage
    let energy_now = num("energy_now");
    let energy_full = num("energy_full");
    let mut capacity: i32 = -1;
    if energy_full > 0 {
        capacity = (energy_now * 100 / energy_full) as i32;
    }
    // Fall back to charge_now/charge_full
    if capacity < 0 {
        let charge_full = num("charge_full");
        if charge_full > 0 {
            capacity = (num("charge_now") * 100 / charge_full) as i32;
        }
    }
    // Last resort: the capacity file
    if capacity < 0 {
        capacity = fs::read_to_string(format!("{base}/{bat}/capacity"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(-1);
    }

    let status = read_first_line(&format!("{base}/{bat}/status")).unwrap_or_default();

    // Time remaining estimate from power_now (negative while discharging on some systems)
    let mut time = String::new();
    if energy_now > 0 {
        let power_now = num("power_now").abs();
        if power_now > 0 {
            time = remaining((energy_now as f32 / power_now as f32 * 60.0) as i32);
        }
    }

    let model = read_first_line(&format!("{base}/{bat}/model_name"))
        .or_else(|| read_first_line(&format!("{base}/{bat}/manufacturer")))
        .map(|m| crate::util::truncate_bytes(m.trim_end(), 63).to_string())
        .unwrap_or_default();
    let label = if model.is_empty() {
        "Battery".to_string()
    } else {
        format!("Battery ({model})")
    };

    if capacity >= 0 {
        info.add_info(&label, &battery_line(capacity, &time, &status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_text() {
        assert_eq!(
            battery_line(75, "2 hours, 5 mins remaining", "Discharging"),
            "\x1b[32m75%\x1b[0m (2 hours, 5 mins remaining) [Discharging]"
        );
        assert_eq!(battery_line(10, "", "Charging"), "\x1b[31m10%\x1b[0m [Charging]");
        assert_eq!(battery_line(30, "", ""), "\x1b[93m30%\x1b[0m");
        assert_eq!(remaining(61), "1 hour, 1 min remaining");
        assert_eq!(remaining(5), "5 mins remaining");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn meminfo_values() {
        let t = "MemTotal:       16000000 kB\nMemAvailable:    8000000 kB\nSwapTotal: 0 kB\n";
        assert_eq!(meminfo(t, "MemTotal:"), 16000000);
        assert_eq!(meminfo(t, "MemAvailable:"), 8000000);
        assert_eq!(meminfo(t, "SwapFree:"), 0);
    }

    #[test]
    fn usage_line_colors() {
        assert_eq!(
            usage_line("1.00 GiB".into(), "2.00 GiB".into(), 50),
            "1.00 GiB / 2.00 GiB (\x1b[93m50%\x1b[0m)"
        );
    }
}
