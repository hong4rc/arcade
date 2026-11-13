//! Memory use of the current process, as the operating system accounts it.

/// Physical memory footprint in bytes (what Activity Monitor shows as "Memory" on macOS;
/// resident set size on Linux).
#[cfg(target_os = "macos")]
pub fn memory_bytes() -> u64 {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: the buffer is a properly sized and aligned rusage_info_v2, matching the flavor.
    let rc = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast(),
        )
    };
    assert_eq!(rc, 0, "proc_pid_rusage failed");
    // SAFETY: the call succeeded, so the kernel filled the structure.
    unsafe { info.assume_init() }.ri_phys_footprint
}

#[cfg(target_os = "linux")]
pub fn memory_bytes() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    parse_vm_rss_kb(&status).expect("VmRSS line") * 1024
}

/// Extract the resident set size in kilobytes from the text of `/proc/<pid>/status`.
pub fn parse_vm_rss_kb(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_status_text() {
        let text = "Name:\tcat\nVmPeak:\t  5000 kB\nVmRSS:\t    1234 kB\nThreads:\t1\n";
        assert_eq!(parse_vm_rss_kb(text), Some(1234));
        assert_eq!(parse_vm_rss_kb("Name:\tcat\n"), None);
    }

    #[test]
    fn memory_grows_when_memory_is_used() {
        let before = memory_bytes();
        let block = vec![1u8; 64 * 1024 * 1024];
        let after = memory_bytes();
        assert!(
            after > before + 32 * 1024 * 1024,
            "before {before}, after {after}"
        );
        drop(block);
    }
}
