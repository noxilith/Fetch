//! GPU detection: DRM devices + `lspci` for full names (Linux), `system_profiler` (macOS).

use crate::info::Info;

#[cfg(target_os = "macos")]
pub fn gather(info: &mut Info) {
    let Some(out) = crate::util::run_output("system_profiler", &["SPDisplaysDataType"]) else {
        return;
    };
    for line in out.lines() {
        let Some(k) = line.find("Chipset Model:").or_else(|| line.find("Chip:")) else {
            continue;
        };
        if let Some((_, v)) = line[k..].split_once(':') {
            let v = v.trim();
            if !v.is_empty() && v.len() < 128 {
                info.add_info("GPU", v);
                return;
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub use linux::gather;

#[cfg(not(target_os = "macos"))]
mod linux {
    use super::Info;
    use crate::util::{read_first_line, run_output, truncate_bytes};
    use std::fs;

    /// Extract the human product name from `lspci -d VVVV:DDDD` output.
    /// Example: "01:00.0 VGA compatible controller: NVIDIA Corporation AD106M
    /// [GeForce RTX 4070 Max-Q / Mobile] (rev a1)". Prefers the bracket
    /// content; otherwise the chunk after "Corporation ".
    fn parse_lspci(line: &str) -> Option<String> {
        let mut line = line.trim_end_matches(['\n', '\r']).to_string();
        if let Some(rev) = line.find(" (rev ") {
            line.truncate(rev);
        }

        let lb = line.rfind('[');
        let rb = line.rfind(']');
        let mut name: Option<String> = None;

        // AMD's lspci strings always wrap the vendor tag in brackets
        // (e.g. "[AMD/ATI]"); discrete cards additionally wrap the product
        // name in a second pair ("[Radeon RX 9070 XT]"), but some iGPUs only
        // have the single vendor bracket. Blindly taking the last bracket
        // pair would pick up "AMD/ATI" there, so count brackets first.
        let bracket_count = line.matches('[').count();
        if let (Some(lb), Some(rb)) = (lb, rb) {
            if rb > lb {
                if bracket_count >= 2 {
                    name = Some(line[lb + 1..rb].to_string());
                } else {
                    let after = line[rb + 1..].trim_start_matches(' ');
                    if !after.is_empty() {
                        name = Some(after.to_string());
                    }
                }
            }
        }

        let name = name.unwrap_or_else(|| {
            let mut start = None;
            if let Some(p) = line.find(" Corporation ") {
                start = Some(p + 13);
            } else if let Some(p) = line.find(" Corp ") {
                start = Some(p + 6);
            } else if let Some(c1) = line.find(':') {
                // Skip past the "bus:dev.fn class:" prefix even without Corporation.
                if let Some(c2) = line[c1 + 1..].find(':') {
                    start = Some(c1 + 1 + c2 + 1);
                }
            }
            line[start.unwrap_or(0)..].trim_start_matches(' ').to_string()
        });
        (!name.is_empty()).then_some(name)
    }

    fn lspci_name(pci_id: &str) -> Option<String> {
        if pci_id.is_empty() {
            return None;
        }
        let out = run_output("lspci", &["-d", pci_id])?;
        parse_lspci(out.lines().next()?)
    }

    fn amd_name_is_igpu(name: &str) -> bool {
        const CODENAMES: [&str; 16] = [
            "Raphael",
            "Phoenix",
            "Phoenix2",
            "Hawk Point",
            "Renoir",
            "Cezanne",
            "Rembrandt",
            "Picasso",
            "Raven",
            "Raven2",
            "Van Gogh",
            "Mendocino",
            "Barcelo",
            "Strix Point",
            "Strix Halo",
            "Krackan Point",
        ];
        let lower = name.to_ascii_lowercase();
        CODENAMES
            .iter()
            .any(|c| lower.contains(&c.to_ascii_lowercase()))
    }

    /// "Apple M1" from a device-tree model like "Apple MacBook Air (M1, 2020)".
    fn apple_chip(model: &str) -> Option<&str> {
        let rest = &model[model.find('(')? + 1..];
        let end = rest.find(',').or_else(|| rest.find(')'))?;
        Some(&rest[..end])
    }

    pub fn gather(info: &mut Info) {
        let mut cards: Vec<String> = fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            // Only cardN (not cardN-CONNECTOR or renderD*)
            .filter(|n| {
                n.strip_prefix("card")
                    .is_some_and(|r| r.bytes().all(|c| c.is_ascii_digit()))
            })
            .collect();
        cards.sort();

        for card in cards {
            let Ok(uevent) = fs::read(format!("/sys/class/drm/{card}/device/uevent")) else {
                continue;
            };
            let uevent = String::from_utf8_lossy(&uevent);
            let (mut driver, mut pci_id, mut compat) = (String::new(), String::new(), String::new());
            for line in uevent.lines() {
                if let Some(v) = line.strip_prefix("DRIVER=") {
                    driver = truncate_bytes(v, 31).into();
                } else if let Some(v) = line.strip_prefix("PCI_ID=") {
                    pci_id = truncate_bytes(v, 15).into();
                } else if let Some(v) = line.strip_prefix("OF_COMPATIBLE_0=") {
                    compat = truncate_bytes(v, 63).into();
                }
            }

            // Skip display subsystems: they're not GPUs
            if compat.contains("display-subsystem") {
                continue;
            }

            let mut name = String::new();
            let mut kind = "";
            if compat.starts_with("apple,agx") {
                let chip = read_first_line("/proc/device-tree/model")
                    .and_then(|m| apple_chip(&m).map(str::to_string));
                name = match chip {
                    Some(c) if !c.is_empty() => format!("Apple {c}"),
                    _ => "Apple GPU".into(),
                };
                kind = "Integrated";
            } else if let Some(n) = lspci_name(&pci_id) {
                name = n;
            } else if driver == "i915" || driver == "xe" {
                name = "Intel Graphics".into();
            } else if driver == "amdgpu" || driver == "radeon" {
                name = "AMD Graphics".into();
            } else if driver == "nvidia" || driver == "nouveau" {
                name = "NVIDIA GPU".into();
            } else if !driver.is_empty() {
                name = driver.clone();
            }
            let name = truncate_bytes(&name, 159).to_string();

            if kind.is_empty() {
                kind = match driver.as_str() {
                    "i915" | "xe" => "Integrated",
                    "nvidia" | "nouveau" => "Discrete",
                    "amdgpu" => {
                        if amd_name_is_igpu(&name) {
                            "Integrated"
                        } else {
                            "Discrete"
                        }
                    }
                    _ => "",
                };
            }

            if name.is_empty() {
                continue;
            }
            if kind.is_empty() {
                info.add_info("GPU", &name);
            } else {
                info.add_info("GPU", &format!("{name} [{kind}]"));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn lspci_names() {
            assert_eq!(
                parse_lspci("01:00.0 VGA compatible controller: NVIDIA Corporation AD106M [GeForce RTX 4070 Max-Q / Mobile] (rev a1)\n").as_deref(),
                Some("AD106M [GeForce RTX 4070 Max-Q / Mobile]")
            );
            // discrete AMD: the second bracket pair is the product name
            assert_eq!(
                parse_lspci("03:00.0 VGA compatible controller: Advanced Micro Devices, Inc. [AMD/ATI] Navi 48 [Radeon RX 9070/9070 XT] (rev c0)").as_deref(),
                Some("Radeon RX 9070/9070 XT")
            );
            // iGPU with only the vendor bracket: use what follows it
            assert_eq!(
                parse_lspci("c5:00.0 VGA compatible controller: Advanced Micro Devices, Inc. [AMD/ATI] Phoenix1 (rev c1)").as_deref(),
                Some("Phoenix1")
            );
            assert_eq!(
                parse_lspci("00:02.0 VGA compatible controller: Intel Corporation UHD Graphics 620 (rev 07)").as_deref(),
                Some("UHD Graphics 620")
            );
        }

        #[test]
        fn igpu_detection() {
            assert!(amd_name_is_igpu("Phoenix1"));
            assert!(amd_name_is_igpu("raphael"));
            assert!(!amd_name_is_igpu("Radeon RX 9070 XT"));
        }

        #[test]
        fn apple_chip_names() {
            assert_eq!(apple_chip("Apple MacBook Air (M1, 2020)"), Some("M1"));
            assert_eq!(apple_chip("Apple Mac (M2)"), Some("M2"));
            assert_eq!(apple_chip("no parens"), None);
        }
    }
}
