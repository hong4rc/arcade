//! The process that keeps the machine awake while the daemon runs.
//!
//! The daemon in these tests runs inside the test process, so the children of the test process
//! are its children. The tests of this file take turns (they look at the children of one
//! process), and none of them touches a `caffeinate` that is not a child of the daemon under
//! test: they find the process through the number the daemon wrote to its log, and look for
//! children only by parent.

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
use std::time::{Duration, Instant};

#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
use arcade_testkit::{AttachOptions, Screened, TestDaemon, WAIT};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn log_of(daemon: &TestDaemon) -> String {
    std::fs::read_to_string(daemon.socket().with_extension("log")).unwrap_or_default()
}

fn with_log(on: bool) -> impl FnOnce(&mut arcade_daemon::DaemonConfig) {
    move |c| {
        c.log_path = Some(c.socket_path.with_extension("log"));
        c.config.prevent_sleep = on;
    }
}

/// The number the daemon logged for the process that keeps the machine awake, once it has.
fn guard_pid(daemon: &TestDaemon) -> Option<u32> {
    log_of(daemon)
        .lines()
        .filter_map(|l| l.split("keeping the machine awake, process ").nth(1))
        .next_back()
        .and_then(|n| n.trim().parse().ok())
}

#[cfg(target_os = "macos")]
fn wait_for_guard(daemon: &TestDaemon) -> u32 {
    let start = Instant::now();
    loop {
        if let Some(pid) = guard_pid(daemon) {
            return pid;
        }
        assert!(
            start.elapsed() < WAIT,
            "no process was started; the log says:\n{}",
            log_of(daemon)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(target_os = "macos")]
fn wait_gone(pid: u32, within: Duration) -> bool {
    let start = Instant::now();
    while alive(pid) {
        if start.elapsed() > within {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// How many children of this process have the given program name.
fn children_named(name: &str) -> usize {
    let out = Command::new("ps")
        .args(["-A", "-o", "ppid=,comm="])
        .output()
        .expect("ps");
    let me = std::process::id().to_string();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            let mut parts = l.split_whitespace();
            parts.next() == Some(me.as_str())
                && parts
                    .next()
                    .is_some_and(|c| Path::new(c).file_name().is_some_and(|f| f == name))
        })
        .count()
}

#[cfg(target_os = "macos")]
#[test]
fn turned_on_the_daemon_has_one_child_that_holds_the_machine_awake() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let daemon = TestDaemon::start_with(with_log(true));
    let pid = wait_for_guard(&daemon);
    let out = Command::new("ps")
        .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        line.starts_with(&std::process::id().to_string()) && line.ends_with("caffeinate"),
        "{line:?}"
    );
    assert_eq!(children_named("caffeinate"), 1);
    // The system lists a hold against idle sleep that belongs to that process.
    let start = Instant::now();
    loop {
        let out = Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        if text.lines().any(|l| {
            l.contains(&format!("pid {pid}(caffeinate)"))
                && l.contains("PreventUserIdleSystemSleep")
        }) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "no hold against idle sleep for {pid}:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn turned_off_the_daemon_starts_nothing() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let daemon = TestDaemon::start_with(with_log(false));
    let mut client = daemon.attach(AttachOptions::new()).unwrap();
    client.wait_for_prompt().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(guard_pid(&daemon).is_none(), "{}", log_of(&daemon));
    assert_eq!(children_named("caffeinate"), 0);
    assert_eq!(children_named("systemd-inhibit"), 0);
}

#[cfg(target_os = "macos")]
#[test]
fn loading_the_configuration_again_changes_it_both_ways_and_never_makes_two() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let file_dir = std::env::temp_dir().join(format!("arc-power-{}", std::process::id()));
    std::fs::create_dir_all(&file_dir).unwrap();
    let file = file_dir.join("config.toml");
    std::fs::write(&file, "[power]\nprevent-sleep = true\n").unwrap();
    let path = file.clone();
    let daemon = TestDaemon::start_with(move |c| {
        with_log(true)(c);
        c.config_path = Some(path);
    });
    let mut client = daemon.attach(AttachOptions::new()).unwrap();
    client.wait_for_prompt().unwrap();
    let first = wait_for_guard(&daemon);
    // Turned off: the process goes.
    std::fs::write(&file, "[power]\nprevent-sleep = false\n").unwrap();
    let (ok, why) = daemon.command("arcade.reload-config", &[]).unwrap();
    assert!(ok, "{why}");
    assert!(
        wait_gone(first, Duration::from_secs(3)),
        "{}",
        log_of(&daemon)
    );
    assert_eq!(children_named("caffeinate"), 0);
    // Turned on again: exactly one, and it is a new one.
    std::fs::write(&file, "[power]\nprevent-sleep = true\n").unwrap();
    let (ok, why) = daemon.command("arcade.reload-config", &[]).unwrap();
    assert!(ok, "{why}");
    let start = Instant::now();
    let second = loop {
        match guard_pid(&daemon) {
            Some(p) if p != first => break p,
            _ => assert!(start.elapsed() < WAIT, "{}", log_of(&daemon)),
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(alive(second));
    assert_eq!(children_named("caffeinate"), 1);
    // Loading it again with the same setting leaves the same process.
    let (ok, _) = daemon.command("arcade.reload-config", &[]).unwrap();
    assert!(ok);
    assert_eq!(guard_pid(&daemon), Some(second));
    assert_eq!(children_named("caffeinate"), 1);
    let _ = std::fs::remove_dir_all(&file_dir);
}

#[cfg(target_os = "macos")]
#[test]
fn stopping_the_daemon_normally_takes_the_process_with_it() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut daemon = TestDaemon::start_with(with_log(true));
    let pid = wait_for_guard(&daemon);
    let (ok, _) = daemon.command("server.kill", &[]).unwrap();
    assert!(ok);
    daemon.wait_stopped(WAIT).unwrap();
    assert!(
        wait_gone(pid, Duration::from_secs(3)),
        "{pid} is still there"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_process_ends_by_itself_when_the_process_it_waits_for_is_killed_hard() {
    // The same arguments the daemon uses, with a stand-in for the daemon: a process of our own
    // that is killed with signal 9 and so cannot say goodbye.
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut stand_in = Command::new("sleep").arg("120").spawn().unwrap();
    let mut guard = Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &stand_in.id().to_string()])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(guard.try_wait().unwrap().is_none(), "it ended at once");
    stand_in.kill().unwrap();
    let _ = stand_in.wait();
    let start = Instant::now();
    loop {
        if guard.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = guard.kill();
            panic!("it did not end within 5 seconds");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_process_dying_on_its_own_changes_nothing_and_is_not_started_again() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let daemon = TestDaemon::start_with(with_log(true));
    let mut client = daemon.attach(AttachOptions::new()).unwrap();
    client.wait_for_prompt().unwrap();
    let pid = wait_for_guard(&daemon);
    // Only the process the daemon made, by its number, never one found by name.
    let status = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(wait_gone(pid, Duration::from_secs(3)));
    let start = Instant::now();
    while !log_of(&daemon).contains("ended") {
        assert!(start.elapsed() < WAIT, "{}", log_of(&daemon));
        std::thread::sleep(Duration::from_millis(10));
    }
    // The daemon and the session carry on.
    let (ok, out) = daemon.command("server.ping", &[]).unwrap();
    assert!(ok && out == "pong");
    client.type_line("echo still-here");
    client.wait_for_row("still-here").unwrap();
    // Nothing starts again by itself.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(children_named("caffeinate"), 0);
    assert_eq!(
        log_of(&daemon).matches("keeping the machine awake").count(),
        1
    );
}

#[cfg(target_os = "linux")]
#[test]
fn without_the_tool_the_daemon_runs_and_says_so_once() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let tool_there = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("systemd-inhibit").is_file()));
    if tool_there {
        // The check below is about a system without it.
        return;
    }
    let daemon = TestDaemon::start_with(with_log(true));
    let mut client = daemon.attach(AttachOptions::new()).unwrap();
    client.wait_for_prompt().unwrap();
    client.type_line("echo works");
    client.wait_for_row("works").unwrap();
    let log = log_of(&daemon);
    assert_eq!(
        log.matches("the machine is not kept awake").count(),
        1,
        "{log}"
    );
    assert!(log.contains("systemd-inhibit is not installed"), "{log}");
}
