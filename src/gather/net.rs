//! Local IPs per interface, with a best-effort human label.

use crate::info::Info;
use std::ffi::CStr;
use std::fs;
use std::net::Ipv4Addr;

#[cfg(not(target_os = "macos"))]
fn iface_is_wireless(name: &str) -> bool {
    fs::metadata(format!("/sys/class/net/{name}/wireless")).is_ok_and(|m| m.is_dir())
}

/// Best-effort label for an interface; anything unrecognized becomes "Local IP (name)".
fn iface_label(name: &str, _is_default: bool) -> String {
    let kind = if name.starts_with("tailscale") {
        Some("Tailscale")
    } else if name.starts_with("zt") {
        Some("ZeroTier")
    } else if name.starts_with("wg") {
        Some("WireGuard")
    } else if name.starts_with("ppp") {
        Some("PPP")
    } else if ["docker", "br-", "veth", "virbr"].iter().any(|p| name.starts_with(p)) {
        Some("Virtual")
    } else if ["utun", "tun", "tap"].iter().any(|p| name.starts_with(p)) {
        Some("VPN")
    } else {
        platform_kind(name, _is_default)
    };
    match kind {
        Some(k) => format!("{k} ({name})"),
        None => format!("Local IP ({name})"),
    }
}

#[cfg(target_os = "macos")]
fn platform_kind(name: &str, is_default: bool) -> Option<&'static str> {
    (name.starts_with("en") && is_default).then_some("Wi-Fi/Ethernet")
}

#[cfg(not(target_os = "macos"))]
fn platform_kind(name: &str, _is_default: bool) -> Option<&'static str> {
    if iface_is_wireless(name) {
        Some("Wi-Fi")
    } else if name.starts_with("eth") || name.starts_with("en") {
        Some("Ethernet")
    } else {
        None
    }
}

/// Interface of the lowest-metric default route.
fn default_route_iface() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = crate::util::run_output("route", &["-n", "get", "default"])?;
        out.lines().find_map(|l| {
            let v = l.split_once("interface:")?.1.trim();
            (!v.is_empty() && v.len() < 64).then(|| v.to_string())
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let text = fs::read_to_string("/proc/net/route").ok()?;
        let mut best: Option<(u64, String)> = None;
        for line in text.lines().skip(1) {
            // Iface Destination Gateway Flags RefCnt Use Metric ...
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 7 {
                continue;
            }
            let dest = u64::from_str_radix(f[1], 16).ok();
            let metric = f[6].parse::<u64>().ok();
            if let (Some(0), Some(metric)) = (dest, metric) {
                if best.as_ref().is_none_or(|(m, _)| metric < *m) {
                    best = Some((metric, f[0].to_string()));
                }
            }
        }
        best.map(|(_, i)| i)
    }
}

struct Entry {
    name: String,
    addr: String,
    is_default: bool,
}

fn ipv4_interfaces(default_iface: Option<&str>) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a linked list that we walk read-only and free below.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return entries;
        }
        let mut cur = list;
        while !cur.is_null() && entries.len() < 16 {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                continue;
            }
            let name = CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned();
            let loopback = if cfg!(target_os = "macos") { "lo0" } else { "lo" };
            if name == loopback {
                continue;
            }
            let sa = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let ip = Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
            let bits = if ifa.ifa_netmask.is_null() {
                0
            } else {
                let m = &*(ifa.ifa_netmask as *const libc::sockaddr_in);
                u32::from_be(m.sin_addr.s_addr).leading_ones()
            };
            let addr = if bits > 0 {
                format!("{ip}/{bits}")
            } else {
                ip.to_string()
            };
            let is_default = default_iface.is_some_and(|d| d == name);
            entries.push(Entry {
                name,
                addr,
                is_default,
            });
        }
        libc::freeifaddrs(list);
    }
    entries
}

pub fn gather(info: &mut Info) {
    let default_iface = default_route_iface();
    let entries = ipv4_interfaces(default_iface.as_deref());
    // The default-route interface first
    for want_default in [true, false] {
        for e in entries.iter().filter(|e| e.is_default == want_default) {
            info.add_info(&iface_label(&e.name, e.is_default), &e.addr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(iface_label("tailscale0", false), "Tailscale (tailscale0)");
        assert_eq!(iface_label("docker0", false), "Virtual (docker0)");
        assert_eq!(iface_label("wg0", true), "WireGuard (wg0)");
        assert_eq!(iface_label("tun1", false), "VPN (tun1)");
        assert_eq!(iface_label("weirdnet0", false), "Local IP (weirdnet0)");
    }
}
