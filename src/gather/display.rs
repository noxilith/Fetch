//! Display info: per-connector DRM enumeration on Linux (active mode from the
//! CRTC, monitor name and size from EDID), `system_profiler` on macOS.

use crate::info::Info;

#[cfg(target_os = "macos")]
pub fn gather(info: &mut Info) {
    let Some(out) = crate::util::run_output("system_profiler", &["SPDisplaysDataType"]) else {
        return;
    };
    let field = |line: &str, keys: &[&str]| -> Option<String> {
        let k = keys.iter().find_map(|k| line.find(k))?;
        let v = line[k..].split_once(':')?.1.trim();
        (!v.is_empty() && v.len() < 64).then(|| v.to_string())
    };
    let mut name = String::new();
    let mut res = String::new();
    for line in out.lines() {
        if name.is_empty() {
            if let Some(v) = field(line, &["Chipset Model:", "Chip:"]) {
                name = v;
            }
        }
        if res.is_empty() {
            if let Some(v) = field(line, &["Resolution:"]) {
                res = v;
            }
        }
    }
    if !name.is_empty() && !res.is_empty() {
        info.add_info("Display", &format!("{name} {res}"));
    } else if !res.is_empty() {
        info.add_info("Display", &res);
    }
}

#[cfg(not(target_os = "macos"))]
pub use linux::gather;

#[cfg(not(target_os = "macos"))]
mod linux {
    use super::Info;
    use crate::util::read_first_line;
    use std::fs;
    use std::os::fd::AsRawFd;

    // Minimal copies of the kernel's DRM uAPI structs (drm_mode.h), just
    // enough to ask which mode a connector is actually driving.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ModeInfo {
        clock: u32,
        hdisplay: u16,
        hsync_start: u16,
        hsync_end: u16,
        htotal: u16,
        hskew: u16,
        vdisplay: u16,
        vsync_start: u16,
        vsync_end: u16,
        vtotal: u16,
        vscan: u16,
        vrefresh: u32,
        flags: u32,
        kind: u32,
        name: [u8; 32],
    }

    #[repr(C)]
    struct Crtc {
        set_connectors_ptr: u64,
        count_connectors: u32,
        crtc_id: u32,
        fb_id: u32,
        x: u32,
        y: u32,
        gamma_size: u32,
        mode_valid: u32,
        mode: ModeInfo,
    }

    #[repr(C)]
    struct Encoder {
        encoder_id: u32,
        encoder_type: u32,
        crtc_id: u32,
        possible_crtcs: u32,
        possible_clones: u32,
    }

    #[repr(C)]
    struct Connector {
        encoders_ptr: u64,
        modes_ptr: u64,
        props_ptr: u64,
        prop_values_ptr: u64,
        count_modes: u32,
        count_props: u32,
        count_encoders: u32,
        encoder_id: u32,
        connector_id: u32,
        connector_type: u32,
        connector_type_id: u32,
        connection: u32,
        mm_width: u32,
        mm_height: u32,
        subpixel: u32,
        pad: u32,
    }

    /// `_IOWR(ty, nr, T)` from <linux/ioctl.h> (the generic layout).
    const fn iowr<T>(ty: u8, nr: u8) -> libc::c_ulong {
        (3 << 30) | ((std::mem::size_of::<T>() as libc::c_ulong) << 16) | ((ty as libc::c_ulong) << 8) | nr as libc::c_ulong
    }

    const DRM_GETCRTC: libc::c_ulong = iowr::<Crtc>(b'd', 0xA1);
    const DRM_GETENCODER: libc::c_ulong = iowr::<Encoder>(b'd', 0xA6);
    const DRM_GETCONNECTOR: libc::c_ulong = iowr::<Connector>(b'd', 0xA7);

    const FLAG_INTERLACE: u32 = 0x10;
    const FLAG_DBLSCAN: u32 = 0x20;

    #[derive(Default)]
    struct ActiveMode {
        w: i32,
        h: i32,
        hz: f32,
        mm_w: i32,
        mm_h: i32,
    }

    /// Ask the kernel for the connector's physical size and, if it has an
    /// active CRTC, the resolution and refresh rate it is driving.
    fn drm_active_mode(card: &str, conn_id: u32, out: &mut ActiveMode) -> bool {
        let Ok(file) = fs::File::open(format!("/dev/dri/{card}")) else {
            return false;
        };
        let fd = file.as_raw_fd();

        // SAFETY: all-zero is a valid value for these plain C structs, and
        // each ioctl only reads/writes the struct we pass (modes_ptr points at
        // `scratch`, which is valid for the one mode we ask for).
        unsafe {
            let mut scratch: ModeInfo = std::mem::zeroed();
            let mut conn: Connector = std::mem::zeroed();
            conn.connector_id = conn_id;
            conn.modes_ptr = &mut scratch as *mut ModeInfo as u64;
            conn.count_modes = 1;
            if libc::ioctl(fd, DRM_GETCONNECTOR as _, &mut conn) != 0 {
                return false;
            }
            if conn.mm_width != 0 && conn.mm_height != 0 {
                out.mm_w = conn.mm_width as i32;
                out.mm_h = conn.mm_height as i32;
            }
            let mut enc: Encoder = std::mem::zeroed();
            enc.encoder_id = conn.encoder_id;
            if conn.encoder_id == 0
                || libc::ioctl(fd, DRM_GETENCODER as _, &mut enc) != 0
                || enc.crtc_id == 0
            {
                return false;
            }
            let mut crtc: Crtc = std::mem::zeroed();
            crtc.crtc_id = enc.crtc_id;
            if libc::ioctl(fd, DRM_GETCRTC as _, &mut crtc) != 0
                || crtc.mode_valid == 0
                || crtc.mode.htotal == 0
                || crtc.mode.vtotal == 0
            {
                return false;
            }
            let m = &crtc.mode;
            out.w = i32::from(m.hdisplay);
            out.h = i32::from(m.vdisplay);
            let mut r = m.clock as f32 * 1000.0 / (f32::from(m.htotal) * f32::from(m.vtotal));
            if m.flags & FLAG_INTERLACE != 0 {
                r *= 2.0;
            }
            if m.flags & FLAG_DBLSCAN != 0 {
                r /= 2.0;
            }
            if m.vscan > 1 {
                r /= f32::from(m.vscan);
            }
            out.hz = r;
            true
        }
    }

    fn read_edid(conn_dir: &str) -> Option<Vec<u8>> {
        let mut data = fs::read(format!("/sys/class/drm/{conn_dir}/edid")).ok()?;
        data.truncate(256);
        const HDR: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
        (data.len() >= 128 && data[..8] == HDR).then_some(data)
    }

    /// Monitor name from the EDID descriptor blocks, else the PNP id + product code.
    fn edid_name(e: &[u8]) -> String {
        for i in (54..=108).step_by(18) {
            let d = &e[i..i + 18];
            if d[0] != 0 || d[1] != 0 || d[2] != 0 || d[3] != 0xFC {
                continue;
            }
            let bytes: Vec<u8> = d[5..18].iter().copied().take_while(|&b| b != 0x0A).collect();
            let name = String::from_utf8_lossy(&bytes).trim_end_matches(' ').to_string();
            if !name.is_empty() {
                return name;
            }
        }
        let v = (u32::from(e[8]) << 8) | u32::from(e[9]);
        let letter = |shift: u32| -> Option<char> {
            let c = b'A' as i32 + ((v >> shift) & 0x1F) as i32 - 1;
            (b'A' as i32..=b'Z' as i32).contains(&c).then_some(c as u8 as char)
        };
        match (letter(10), letter(5), letter(0)) {
            (Some(a), Some(b), Some(c)) => format!("{a}{b}{c}{:02X}{:02X}", e[11], e[10]),
            _ => String::new(),
        }
    }

    /// Physical size in mm: detailed timing block first, else the basic (cm) size.
    fn edid_size_mm(e: &[u8]) -> Option<(i32, i32)> {
        let d = &e[54..72];
        if d[0] != 0 || d[1] != 0 {
            let mw = i32::from(d[12]) | ((i32::from(d[14]) & 0xF0) << 4);
            let mh = i32::from(d[13]) | ((i32::from(d[14]) & 0x0F) << 8);
            if mw > 0 && mh > 0 {
                return Some((mw, mh));
            }
        }
        (e[21] != 0 && e[22] != 0).then(|| (i32::from(e[21]) * 10, i32::from(e[22]) * 10))
    }

    /// Refresh rate from the first detailed timing block (the preferred mode).
    fn edid_refresh(e: &[u8]) -> f32 {
        let d = &e[54..72];
        let clock = u32::from(d[0]) | (u32::from(d[1]) << 8);
        if clock == 0 {
            return 0.0;
        }
        let htotal = (u32::from(d[2]) | ((u32::from(d[4]) & 0xF0) << 4))
            + (u32::from(d[3]) | ((u32::from(d[4]) & 0x0F) << 8));
        let vtotal = (u32::from(d[5]) | ((u32::from(d[7]) & 0xF0) << 4))
            + (u32::from(d[6]) | ((u32::from(d[7]) & 0x0F) << 8));
        if htotal == 0 || vtotal == 0 {
            return 0.0;
        }
        clock as f32 * 10000.0 / (htotal * vtotal) as f32
    }

    fn format_hz(hz: f32) -> String {
        if hz <= 0.0 {
            String::new()
        } else if (hz - hz.round()).abs() < 0.05 {
            format!("{hz:.0} Hz")
        } else {
            format!("{hz:.2} Hz")
        }
    }

    fn connector_is_internal(conn: &str) -> bool {
        conn.starts_with("eDP") || conn.starts_with("LVDS") || conn.starts_with("DSI")
    }

    fn drm_entries() -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        v.sort();
        v
    }

    pub fn gather(info: &mut Info) {
        let entries = drm_entries();
        let mut emitted = 0;

        for name in &entries {
            // Pattern: cardN-CONNECTOR (skip bare cardN and renderD*)
            let Some(rest) = name.strip_prefix("card") else {
                continue;
            };
            let Some(dash) = rest.find('-') else {
                continue;
            };
            let conn = &rest[dash + 1..];
            let card = &name[..4 + dash];

            let dir = format!("/sys/class/drm/{name}");
            if read_first_line(&format!("{dir}/status")).as_deref() != Some("connected") {
                continue;
            }
            let Some(mode) = read_first_line(&format!("{dir}/modes")) else {
                continue;
            };
            if card.len() >= 32 {
                continue;
            }

            let edid = read_edid(name);

            let mut active = ActiveMode::default();
            let conn_id: u32 = read_first_line(&format!("{dir}/connector_id"))
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            if conn_id != 0 {
                drm_active_mode(card, conn_id, &mut active);
            }

            if active.hz <= 0.0 {
                if let Some(e) = &edid {
                    active.hz = edid_refresh(e);
                }
            }
            if active.mm_w == 0 || active.mm_h == 0 {
                if let Some((w, h)) = edid.as_deref().and_then(edid_size_mm) {
                    active.mm_w = w;
                    active.mm_h = h;
                }
            }

            let res = if active.w > 0 && active.h > 0 {
                format!("{}x{}", active.w, active.h)
            } else {
                mode
            };
            let monitor = edid.as_deref().map(edid_name).unwrap_or_default();
            let label = format!(
                "Display ({})",
                if monitor.is_empty() { conn } else { &monitor }
            );

            let inches = if active.mm_w > 0 && active.mm_h > 0 {
                let diag = (((active.mm_w * active.mm_w + active.mm_h * active.mm_h) as f32)
                    .sqrt()
                    / 25.4)
                    .round() as i32;
                if diag > 0 {
                    format!(" in {diag}\"")
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let rate = format_hz(active.hz);
            let sep = if rate.is_empty() {
                ""
            } else if inches.is_empty() {
                " @ "
            } else {
                ", "
            };
            let kind = if connector_is_internal(conn) {
                "Built-in"
            } else {
                "External"
            };
            info.add_info(&label, &format!("{res}{inches}{sep}{rate} [{kind}]"));
            emitted += 1;
        }

        // Fallback: the first mode of any card
        if emitted == 0 {
            for name in &entries {
                if !name.starts_with("card") {
                    continue;
                }
                if let Some(mode) = read_first_line(&format!("/sys/class/drm/{name}/modes")) {
                    info.add_info("Display", &mode);
                    break;
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn edid_with_name(name: &str) -> Vec<u8> {
            let mut e = vec![0u8; 128];
            e[..8].copy_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]);
            // 'ABC' PNP id, product code 0x1234 (little endian at 10..12)
            let v: u16 = (1 << 10) | (2 << 5) | 3;
            e[8] = (v >> 8) as u8;
            e[9] = v as u8;
            e[10] = 0x34;
            e[11] = 0x12;
            if !name.is_empty() {
                let d = &mut e[72..90];
                d[3] = 0xFC;
                for (i, b) in name.bytes().enumerate() {
                    d[5 + i] = b;
                }
                d[5 + name.len()] = 0x0A;
            }
            e
        }

        #[test]
        fn edid_names() {
            assert_eq!(edid_name(&edid_with_name("DELL U2720Q")), "DELL U2720Q");
            assert_eq!(edid_name(&edid_with_name("")), "ABC1234");
        }

        #[test]
        fn edid_geometry() {
            let mut e = edid_with_name("");
            // 1920x1080@60-style timing: 148.5 MHz, htotal 2200, vtotal 1125
            let d = &mut e[54..72];
            let clock = 14850u16;
            d[0] = clock as u8;
            d[1] = (clock >> 8) as u8;
            let (hb, vb) = (280u32, 45u32); // blanking
            let (ha, va) = (1920u32, 1080u32);
            d[2] = ha as u8;
            d[3] = hb as u8;
            d[4] = (((ha >> 8) << 4) | (hb >> 8)) as u8;
            d[5] = va as u8;
            d[6] = vb as u8;
            d[7] = (((va >> 8) << 4) | (vb >> 8)) as u8;
            d[12] = 0x20; // 0x120 = 288 mm
            d[13] = 0xA2; // 0x1A2 = 418? low byte only
            d[14] = 0x11;
            assert!((edid_refresh(&e) - 60.0).abs() < 0.01);
            assert_eq!(edid_size_mm(&e), Some((0x120, 0x1A2)));
        }

        #[test]
        fn hz_format() {
            assert_eq!(format_hz(60.0), "60 Hz");
            assert_eq!(format_hz(59.94), "59.94 Hz");
            assert_eq!(format_hz(0.0), "");
        }
    }
}
