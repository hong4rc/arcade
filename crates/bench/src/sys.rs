//! Operating system pieces the benchmark needs: terminals, clocks, and per-process counters.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

/// Monotonic nanoseconds. The same clock in every process on the machine, so a time stamped by
/// the program in a pane can be compared with one taken here.
pub fn now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime fills the struct we hand it.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// A program running in a terminal that this process owns: it plays the host terminal.
pub struct Pty {
    pub master: File,
    pub pid: i32,
    child: Option<Child>,
}

pub fn spawn_pty(argv: &[String], cols: u16, rows: u16) -> io::Result<Pty> {
    let mut ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (mut master, mut slave) = (0, 0);
    // SAFETY: out pointers are valid; name and termios are optional and null; ws is a live local.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!(ws),
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: two fresh descriptors that nothing else owns.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    // SAFETY: fcntl on descriptors we own.
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(slave.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    command
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("ARCADE")
        .env_remove("ARCADE_PANE");
    command.env("TERM", "xterm-256color");
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // SAFETY: only setsid and ioctl run between fork and exec; both are async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let child = command
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("cannot run {}: {e}", argv[0])))?;
    let pid = child.id() as i32;
    Ok(Pty {
        master: File::from(master),
        pid,
        child: Some(child),
    })
}

impl Pty {
    /// Wait up to `timeout_ms` for output and read what is there. `Ok(0)` means nothing came
    /// in time; an error means the terminal is closed.
    pub fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> io::Result<usize> {
        let mut pfd = libc::pollfd {
            fd: self.master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll on one live pollfd.
        let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if rc <= 0 {
            return Ok(0);
        }
        match self.master.read(buf) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "terminal closed",
            )),
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.master.write_all(bytes)
    }

    /// Stop the program and reap it.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.kill();
    }
}

/// CPU seconds a process has used, as `ps` reports them (hundredths of a second).
pub fn cpu_seconds(pid: i32) -> f64 {
    let out = Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let (rest, seconds) = text.rsplit_once(':').unwrap_or(("0", "0"));
    let mut total: f64 = seconds.parse().unwrap_or(0.0);
    let mut parts = rest.rsplit(':');
    total += parts
        .next()
        .and_then(|m| m.parse::<f64>().ok())
        .unwrap_or(0.0)
        * 60.0;
    total += parts
        .next()
        .and_then(|h| h.parse::<f64>().ok())
        .unwrap_or(0.0)
        * 3600.0;
    total
}

/// Memory a process really holds, in megabytes: on macOS the system's "physical footprint", which
/// follows memory that was compressed or given back; elsewhere the resident size.
pub fn footprint_mb(pid: i32) -> f64 {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: proc_pid_rusage fills the struct we pass for the flavor we ask for.
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        // SAFETY: as above; info is a live local of the size the flavor expects.
        let rc = unsafe {
            libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V2,
                (&raw mut info).cast::<libc::rusage_info_t>(),
            )
        };
        if rc == 0 {
            return info.ri_phys_footprint as f64 / 1_048_576.0;
        }
    }
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    let kib: f64 = out
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(0.0)
        })
        .unwrap_or(0.0);
    kib / 1024.0
}

/// How many times the system switched this process in or out so far: each is a wake-up or a
/// preemption. Zero change over an idle minute means the process did not wake at all.
pub fn context_switches(pid: i32) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: proc_pidinfo fills a proc_taskinfo of the size we give.
        let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
        // SAFETY: as above; info is a live local.
        let rc = unsafe {
            libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&raw mut info).cast(), size)
        };
        if rc == size {
            return Some(info.pti_csw as u64);
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        let text = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let field = |name: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(name))
                .and_then(|v| v.trim().parse::<u64>().ok())
        };
        Some(field("voluntary_ctxt_switches:")? + field("nonvoluntary_ctxt_switches:")?)
    }
}

/// The process id of a running program whose command line contains all of `needles`.
pub fn find_pid(needles: &[&str]) -> Option<i32> {
    let out = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .ok()?;
    let me = std::process::id() as i32;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            let line = line.trim_start();
            let (pid, command) = line.split_once(' ')?;
            let pid: i32 = pid.parse().ok()?;
            (pid != me && needles.iter().all(|n| command.contains(n))).then_some(pid)
        })
}

/// Resident size in megabytes, as `ps` reports it. tmux's baseline was taken this way.
pub fn rss_mb(pid: i32) -> f64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    let kib: f64 = out
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(0.0)
        })
        .unwrap_or(0.0);
    kib / 1024.0
}
