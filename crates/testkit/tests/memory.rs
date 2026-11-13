//! Memory of a pane with a full scrollback. This test is alone in its file so that it runs in a
//! process of its own: other tests running beside it would blur the measurement. It measures the
//! physical footprint, which only macOS reports.

#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

use arcade_testkit::{AttachOptions, TestDaemon, footprint_kib};
use arcade_workload::scrollback_fill;

/// A pane that has filled its scrollback and gone quiet gets the old lines compressed, which
/// brings its memory under the 30 MB the performance spec asks for.
#[test]
fn a_quiet_pane_gives_back_memory() {
    let daemon = TestDaemon::start();
    let dir = daemon.dir().to_path_buf();
    std::fs::write(dir.join("fill.bin"), scrollback_fill(50_000, 120, 1)).unwrap();
    let script = format!(
        "cd '{d}'; cat fill.bin; touch filled; sleep 120",
        d = dir.display()
    );
    let mut options = AttachOptions::new().size(120, 40);
    options.command = vec!["/bin/sh".into(), "-c".into(), script];
    let _client = daemon.attach(options).unwrap();
    let start = Instant::now();
    while !dir.join("filled").exists() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the pane never filled"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid = std::process::id();
    let before_all = footprint_kib(pid);
    // The pane is quiet from now on; compression starts a moment later.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut now = footprint_kib(pid);
    while Instant::now() < deadline && now + 15 * 1024 > before_all {
        std::thread::sleep(Duration::from_millis(250));
        now = footprint_kib(pid);
    }
    eprintln!("footprint right after the fill: {before_all} KiB, after going quiet: {now} KiB");
    assert!(
        now + 15 * 1024 <= before_all,
        "memory went from {before_all} KiB to {now} KiB; it should drop by at least 15 MB"
    );
}
