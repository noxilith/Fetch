//! macOS-only helpers: sysctl, process lookups, mount info, and battery via IOKit.
//!
//! Note: this module has not been compiled or run on macOS yet.

use crate::util::{basename, run_output};
use std::ffi::{c_char, c_void, CStr, CString};

pub fn sysctl_str(name: &str) -> Option<String> {
    let c = CString::new(name).ok()?;
    let mut len: usize = 0;
    let mut buf;
    // SAFETY: the standard two-call sysctlbyname pattern: ask for the size,
    // then read into a buffer of that size.
    unsafe {
        if libc::sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf = vec![0u8; len];
        if libc::sysctlbyname(c.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
    }
    buf.truncate(len);
    while buf.last() == Some(&0) {
        buf.pop();
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn sysctl_num<T: Default + Copy>(name: &str) -> T {
    let Ok(c) = CString::new(name) else {
        return T::default();
    };
    let mut val = T::default();
    let mut len = std::mem::size_of::<T>();
    // SAFETY: `val` is valid for `len` bytes.
    let rc = unsafe {
        libc::sysctlbyname(c.as_ptr(), (&mut val as *mut T).cast(), &mut len, std::ptr::null_mut(), 0)
    };
    if rc == 0 {
        val
    } else {
        T::default()
    }
}

pub fn sysctl_long(name: &str) -> i64 {
    sysctl_num::<i64>(name)
}

pub fn sysctl_int(name: &str) -> i32 {
    sysctl_num::<i32>(name)
}

pub fn uptime_secs() -> Option<f64> {
    let mut bt: libc::timeval = libc::timeval { tv_sec: 0, tv_usec: 0 };
    let mut len = std::mem::size_of::<libc::timeval>();
    let name = CString::new("kern.boottime").ok()?;
    // SAFETY: `bt` is valid for `len` bytes.
    let rc = unsafe {
        libc::sysctlbyname(name.as_ptr(), (&mut bt as *mut libc::timeval).cast(), &mut len, std::ptr::null_mut(), 0)
    };
    if rc != 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some((now - bt.tv_sec as f64).floor())
}

/// Command name of a process (basename), via `ps`.
pub fn process_comm(pid: u32) -> Option<String> {
    let out = run_output("ps", &["-p", &pid.to_string(), "-o", "comm="])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| basename(line).to_string())
}

pub fn parent_of(pid: u32) -> Option<u32> {
    let out = run_output("ps", &["-p", &pid.to_string(), "-o", "ppid="])?;
    out.trim().parse().ok().filter(|&p| p > 0)
}

pub fn fstype_of(path: &str) -> Option<String> {
    let mut mnts: *mut libc::statfs = std::ptr::null_mut();
    // SAFETY: getmntinfo returns a pointer to an internal array of `n` entries.
    unsafe {
        let n = libc::getmntinfo(&mut mnts, libc::MNT_NOWAIT);
        for i in 0..n.max(0) as usize {
            let m = &*mnts.add(i);
            if CStr::from_ptr(m.f_mntonname.as_ptr()).to_bytes() == path.as_bytes() {
                return Some(CStr::from_ptr(m.f_fstypename.as_ptr()).to_string_lossy().into_owned());
            }
        }
    }
    None
}

// --- battery (IOKit power sources) ---

type CFTypeRef = *const c_void;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFTypeRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, ps: CFTypeRef) -> CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, idx: isize) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFStringCreateWithCString(alloc: CFTypeRef, c: *const c_char, encoding: u32) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, out: *mut c_void) -> u8;
    fn CFBooleanGetValue(b: CFTypeRef) -> u8;
    fn CFRelease(cf: CFTypeRef);
}

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NUMBER_INT_TYPE: isize = 9;

pub struct Battery {
    pub capacity: i32,
    pub charging: bool,
    /// Minutes, or -1 if unknown.
    pub time_to_empty: i32,
}

/// # Safety
/// `dict` must be a valid CFDictionaryRef.
unsafe fn dict_int(dict: CFTypeRef, key: &str) -> Option<i32> {
    let k = CString::new(key).ok()?;
    let cfkey = CFStringCreateWithCString(std::ptr::null(), k.as_ptr(), CF_STRING_ENCODING_UTF8);
    if cfkey.is_null() {
        return None;
    }
    let v = CFDictionaryGetValue(dict, cfkey);
    CFRelease(cfkey);
    if v.is_null() {
        return None;
    }
    let mut out: i32 = 0;
    (CFNumberGetValue(v, CF_NUMBER_INT_TYPE, (&mut out as *mut i32).cast()) != 0).then_some(out)
}

pub fn battery() -> Option<Battery> {
    // SAFETY: standard IOPowerSources usage; every Copy* result is released.
    unsafe {
        let info = IOPSCopyPowerSourcesInfo();
        if info.is_null() {
            return None;
        }
        let sources = IOPSCopyPowerSourcesList(info);
        if sources.is_null() {
            CFRelease(info);
            return None;
        }
        let mut result = None;
        if CFArrayGetCount(sources) > 0 {
            let src = IOPSGetPowerSourceDescription(info, CFArrayGetValueAtIndex(sources, 0));
            if !src.is_null() {
                if let Some(capacity) = dict_int(src, "Current Capacity").filter(|&c| c >= 0) {
                    let charging = {
                        let k = CString::new("Is Charging").unwrap();
                        let cfkey = CFStringCreateWithCString(std::ptr::null(), k.as_ptr(), CF_STRING_ENCODING_UTF8);
                        let v = if cfkey.is_null() { std::ptr::null() } else { CFDictionaryGetValue(src, cfkey) };
                        if !cfkey.is_null() {
                            CFRelease(cfkey);
                        }
                        !v.is_null() && CFBooleanGetValue(v) != 0
                    };
                    result = Some(Battery {
                        capacity,
                        charging,
                        time_to_empty: dict_int(src, "Time to Empty").unwrap_or(-1),
                    });
                }
            }
        }
        CFRelease(sources);
        CFRelease(info);
        result
    }
}
