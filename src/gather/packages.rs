//! Package counts across the common package managers.

use crate::info::Info;
use crate::util::{basename, count_output_lines, run_output};
use std::fs;

/// Non-hidden entries of a directory (non-recursive).
fn count_entries(path: &str) -> usize {
    fs::read_dir(path).map_or(0, |d| {
        d.flatten()
            .filter(|e| !e.file_name().as_encoded_bytes().starts_with(b"."))
            .count()
    })
}

/// Lines in a file that start with `prefix`.
fn count_prefixed_lines(path: &str, prefix: &str) -> usize {
    fs::read(path).map_or(0, |data| {
        String::from_utf8_lossy(&data)
            .lines()
            .filter(|l| l.starts_with(prefix))
            .count()
    })
}

/// `nix-store -q --references <paths>` and how many store paths it lists.
fn nix_references(paths: &[&str]) -> Vec<String> {
    let mut args = vec!["-q", "--references"];
    args.extend_from_slice(paths);
    run_output("nix-store", &args)
        .map(|o| o.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Nix (NixOS, or Nix on other distros): system and per-user package counts.
fn nix_val() -> Option<String> {
    let system = nix_references(&["/run/current-system/sw"]).len();

    // Only a username charset is allowed before it is used in a path.
    let user = crate::gather::home()
        .map(|h| basename(&h).to_string())
        .filter(|b| {
            !b.is_empty()
                && b.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        })
        .and_then(|b| fs::canonicalize(format!("/etc/profiles/per-user/{b}")).ok())
        .map_or(0, |resolved| {
            let resolved = resolved.to_string_lossy().into_owned();
            // The profile references one hop of wrappers; count what those pull in.
            let hop1 = nix_references(&[&resolved]);
            if hop1.is_empty() {
                0
            } else {
                let hop1: Vec<&str> = hop1.iter().flat_map(|l| l.split_whitespace()).collect();
                nix_references(&hop1).len()
            }
        });

    match (system > 0, user > 0) {
        (true, true) => Some(format!("{system} (system), {user} (user)")),
        (true, false) => Some(format!("{system} (system)")),
        (false, true) => Some(format!("{user} (user)")),
        (false, false) => None,
    }
}

pub fn gather(info: &mut Info) {
    let mut parts: Vec<String> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        // Homebrew: check both ARM and Intel Cellar paths
        let total: usize = ["/opt/homebrew/Cellar", "/usr/local/Cellar"]
            .iter()
            .map(|p| count_entries(p))
            .sum();
        if total > 0 {
            parts.push(format!("{total} (brew)"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let native = native_manager();
        if let Some(n) = native {
            parts.push(n);
        }
    }

    if let Some(n) = nix_val() {
        parts.push(n);
    }
    let flatpak = count_output_lines("flatpak", &["list", "--columns=ref"]);
    if flatpak > 0 {
        parts.push(format!("{flatpak} (flatpak)"));
    }

    if !parts.is_empty() {
        info.add_info("Packages", &parts.join(", "));
    }
}

/// The first native package manager that reports packages.
#[cfg(not(target_os = "macos"))]
fn native_manager() -> Option<String> {
    // emerge (Gentoo): category/package two-level directories
    let emerge: usize = fs::read_dir("/var/db/pkg").map_or(0, |d| {
        d.flatten()
            .filter(|c| !c.file_name().as_encoded_bytes().starts_with(b"."))
            .map(|c| count_entries(&c.path().to_string_lossy()))
            .sum()
    });
    if emerge > 0 {
        return Some(format!("{emerge} (emerge)"));
    }
    // pacman (Arch); -1 for ALPM_DB_VERSION
    let pacman = count_entries("/var/lib/pacman/local");
    if pacman > 1 {
        return Some(format!("{} (pacman)", pacman - 1));
    }
    // dpkg (Debian/Ubuntu)
    let dpkg = count_prefixed_lines("/var/lib/dpkg/status", "Package:");
    if dpkg > 0 {
        return Some(format!("{dpkg} (dpkg)"));
    }
    // rpm (Fedora, RHEL, openSUSE, ...)
    let rpm = count_output_lines("rpm", &["-qa"]);
    if rpm > 0 {
        return Some(format!("{rpm} (rpm)"));
    }
    // xbps (Void)
    let xbps = count_output_lines("xbps-query", &["-l"]);
    if xbps > 0 {
        return Some(format!("{xbps} (xbps)"));
    }
    // apk (Alpine)
    let apk = count_prefixed_lines("/lib/apk/db/installed", "P:");
    if apk > 0 {
        return Some(format!("{apk} (apk)"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_files_and_dirs() {
        let dir = std::env::temp_dir().join(format!("fetch-pkg-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join(".hidden")).unwrap();
        fs::write(dir.join("b"), "").unwrap();
        assert_eq!(count_entries(dir.to_str().unwrap()), 2);
        fs::write(dir.join("status"), "Package: x\nVersion: 1\nPackage: y\n").unwrap();
        assert_eq!(
            count_prefixed_lines(dir.join("status").to_str().unwrap(), "Package:"),
            2
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
