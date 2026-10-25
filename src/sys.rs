use fltk::{draw, enums::Font};

/// glibc-only tuning + fontconfig warm-up. Both are Linux-isms.
///
/// Only enabled on GNU/Linux with the `glibc` feature (the default). musl, BSD,
/// macOS and Windows all fall back to the portable no-op versions below, which
/// keeps the binary linkable everywhere.
#[cfg(all(target_os = "linux", feature = "glibc"))]
mod imp {
    unsafe extern "C" {
        fn mallopt(param: i32, value: i32) -> i32;
        fn malloc_trim(pad: usize) -> i32;
        fn FcInit() -> i32;
    }

    const M_ARENA_MAX: i32 = -8;
    const M_TRIM_THRESHOLD: i32 = -1;
    const M_MMAP_THRESHOLD: i32 = -3;

    pub fn tune_allocator() {
        unsafe {
            mallopt(M_ARENA_MAX, 1);
            mallopt(M_TRIM_THRESHOLD, 128 * 1024);
            mallopt(M_MMAP_THRESHOLD, 128 * 1024);
        }
    }

    pub fn trim_allocator() {
        unsafe {
            malloc_trim(0);
        }
    }

    pub fn init_fonts() {
        unsafe {
            FcInit();
        }
    }
}

#[cfg(not(all(target_os = "linux", feature = "glibc")))]
mod imp {
    pub fn tune_allocator() {}

    pub fn trim_allocator() {}

    pub fn init_fonts() {}
}

pub use imp::{init_fonts, trim_allocator, tune_allocator};

pub fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut index = 0;
    while value >= 1024.0 && index + 1 < units.len() {
        value /= 1024.0;
        index += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", units[index])
    } else if value >= 10.0 {
        format!("{value:.1} {}", units[index])
    } else {
        format!("{value:.2} {}", units[index])
    }
}

pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn format_duration(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("{} ms", (seconds * 1000.0).round() as u64)
    } else if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else {
        let minutes = (seconds / 60.0).floor() as u64;
        let rest = seconds - minutes as f64 * 60.0;
        format!("{minutes}m {rest:.0}s")
    }
}

pub fn read_rss_kb() -> u64 {
    rss::read_rss_kb()
}

#[cfg(target_os = "linux")]
mod rss {
    pub fn read_rss_kb() -> u64 {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                return rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
            }
        }
        0
    }
}

#[cfg(target_os = "macos")]
mod rss {
    const PROC_PIDTASKINFO: i32 = 4;

    #[repr(C)]
    #[derive(Default)]
    struct ProcTaskInfo {
        virtual_size: u64,
        resident_size: u64,
        total_user: u64,
        total_system: u64,
        threaded: u64,
        policy: i32,
        faults: i32,
        pageins: i32,
        cow_faults: i32,
        messages_sent: i32,
        messages_received: i32,
        syscalls_mach: i32,
        syscalls_unix: i32,
        csw: i32,
        threadnum: i32,
        numrunning: i32,
        priority: i32,
    }

    unsafe extern "C" {
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut u8, size: i32) -> i32;
    }

    pub fn read_rss_kb() -> u64 {
        let mut info = ProcTaskInfo::default();
        let size = std::mem::size_of::<ProcTaskInfo>() as i32;
        let written = unsafe {
            proc_pidinfo(
                std::process::id() as i32,
                PROC_PIDTASKINFO,
                0,
                (&mut info as *mut ProcTaskInfo).cast(),
                size,
            )
        };
        if written == size {
            info.resident_size / 1024
        } else {
            0
        }
    }
}

#[cfg(windows)]
mod rss {
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
        fn GetCurrentProcess() -> isize;
    }

    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    pub fn read_rss_kb() -> u64 {
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            ..Default::default()
        };
        let ok =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        if ok != 0 {
            (counters.working_set_size / 1024) as u64
        } else {
            0
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod rss {
    pub fn read_rss_kb() -> u64 {
        0
    }
}

pub fn sanitize_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_control() || ch == '\t' || ch == '\n' || ch == '\r' {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn truncate_text(text: &str, max_width: i32, font: Font, size: i32) -> String {
    if max_width <= 0 {
        return String::new();
    }
    draw::set_font(font, size);
    if draw::measure(text, false).0 <= max_width {
        return text.to_string();
    }

    let ellipsis = "…";
    let ellipsis_width = draw::measure(ellipsis, false).0;
    let available = max_width - ellipsis_width;
    if available <= 0 {
        return ellipsis.to_string();
    }

    let mut out = String::new();
    let mut width = 0;
    for ch in text.chars() {
        let mut buffer = [0u8; 4];
        let text = ch.encode_utf8(&mut buffer);
        let char_width = draw::measure(text, false).0;
        if width + char_width > available {
            break;
        }
        out.push(ch);
        width += char_width;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_human_readable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(12 * 1024 * 1024), "12.0 MiB");
        assert_eq!(format_bytes(150 * 1024 * 1024 * 1024), "150 GiB");
    }

    #[test]
    fn counts_use_separators() {
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1,000");
        assert_eq!(format_count(1234567), "1,234,567");
    }
}
