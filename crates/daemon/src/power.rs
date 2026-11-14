//! Keeping the machine from going to sleep because it sat idle, for as long as the daemon runs.
//!
//! The daemon starts one small process of the operating system that holds the machine awake and
//! leaves when the daemon does (it is told to wait for the daemon's process). Nothing in the
//! daemon itself runs on a timer or calls into the system libraries for this.
//!
//! * macOS: `/usr/bin/caffeinate -i -w <daemon>`.
//! * Linux with `systemd-inhibit`: `systemd-inhibit` holding an idle and sleep inhibitor around
//!   `tail --pid=<daemon> -f /dev/null`.
//! * Anywhere else: nothing is started, and the caller is told why.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The process that holds the machine awake.
pub struct SleepGuard {
    pid: u32,
}

impl SleepGuard {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Ask the process to end; whoever waits for it learns that it did.
    pub fn stop(&self) {
        if let Ok(pid) = i32::try_from(self.pid) {
            // SAFETY: kill only sends a signal to a process this daemon started.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

/// The program and arguments that hold the machine awake while `daemon_pid` runs, or why there
/// are none on this system.
fn plan(daemon_pid: u32) -> Result<(PathBuf, Vec<String>), String> {
    if cfg!(target_os = "macos") {
        return Ok((
            PathBuf::from("/usr/bin/caffeinate"),
            vec!["-i".into(), "-w".into(), daemon_pid.to_string()],
        ));
    }
    if cfg!(target_os = "linux") {
        let program = find_in_path("systemd-inhibit")
            .ok_or_else(|| "systemd-inhibit is not installed".to_string())?;
        return Ok((
            program,
            vec![
                "--what=idle:sleep".into(),
                "--who=arcade".into(),
                "--why=arcade sessions are running".into(),
                "tail".into(),
                format!("--pid={daemon_pid}"),
                "-f".into(),
                "/dev/null".into(),
            ],
        ));
    }
    Err("this system has no way to hold it awake that arcade knows".to_string())
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Start the process. `on_exit` is called, from a thread that does nothing else, with the
/// process number and how it ended, once it has ended (also after `stop`).
pub fn start(on_exit: impl FnOnce(u32, String) + Send + 'static) -> Result<SleepGuard, String> {
    let (program, args) = plan(std::process::id())?;
    let mut child = Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{} did not start: {e}", program.display()))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let how = match child.wait() {
            Ok(status) => status.to_string(),
            Err(e) => format!("lost: {e}"),
        };
        on_exit(pid, how);
    });
    Ok(SleepGuard { pid })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_waits_for_the_daemon_and_does_not_use_a_shell() {
        match plan(4242) {
            Ok((program, args)) => {
                assert!(args.iter().any(|a| a.contains("4242")), "{args:?}");
                let name = program.file_name().unwrap().to_string_lossy().into_owned();
                assert!(name == "caffeinate" || name == "systemd-inhibit", "{name}");
                assert!(!args.iter().any(|a| a == "-c"), "{args:?}");
            }
            // A system without the tool says so in words.
            Err(why) => assert!(!why.is_empty()),
        }
    }
}
