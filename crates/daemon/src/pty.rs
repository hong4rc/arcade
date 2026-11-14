//! Pseudo terminals and the processes that run in them.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub struct SpawnSpec {
    /// Program and arguments. Empty means the user's shell.
    pub argv: Vec<String>,
    pub cwd: String,
    /// The whole environment of the new process, as `NAME=value`.
    pub env: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

pub struct Spawned {
    /// Our end of the pseudo terminal, non-blocking.
    pub master: File,
    pub pid: i32,
    pub child: std::process::Child,
}

fn env_value<'a>(env: &'a [String], name: &str) -> Option<&'a str> {
    env.iter().rev().find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn set_cloexec(fd: i32) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor we own, with plain integer arguments.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn set_nonblocking(fd: i32) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor we own, with plain integer arguments.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn spawn(spec: &SpawnSpec) -> io::Result<Spawned> {
    let mut winsize = libc::winsize {
        ws_row: spec.rows,
        ws_col: spec.cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (mut master_fd, mut slave_fd) = (0, 0);
    // SAFETY: both out pointers are valid; name and termios are optional and passed as null;
    // winsize points to a live struct.
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty returned two fresh descriptors that nothing else owns.
    let (master, slave) = unsafe {
        (
            OwnedFd::from_raw_fd(master_fd),
            OwnedFd::from_raw_fd(slave_fd),
        )
    };
    set_cloexec(master.as_raw_fd())?;
    set_cloexec(slave.as_raw_fd())?;
    set_nonblocking(master.as_raw_fd())?;

    let (program, args, login) = if spec.argv.is_empty() {
        let shell = env_value(&spec.env, "SHELL")
            .filter(|s| !s.is_empty())
            .unwrap_or("/bin/sh")
            .to_string();
        (shell, Vec::new(), true)
    } else {
        (spec.argv[0].clone(), spec.argv[1..].to_vec(), false)
    };

    let mut command = Command::new(&program);
    command.args(&args);
    if login {
        // A leading dash in argv[0] makes the shell a login shell, as in a new terminal window.
        let base = program.rsplit('/').next().unwrap_or(&program);
        command.arg0(format!("-{base}"));
    }
    command.env_clear();
    for entry in &spec.env {
        if let Some((key, value)) = entry.split_once('=') {
            command.env(key, value);
        }
    }
    if !spec.cwd.is_empty() {
        command.current_dir(&spec.cwd);
    }
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // SAFETY: the closure runs in the child between fork and exec and only calls setsid and
    // ioctl, which are async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            // Make the terminal on standard input this session's controlling terminal, so the
            // shell has job control and ^C reaches the foreground job.
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("cannot run {program}: {e}")))?;
    let pid = child.id() as i32;
    Ok(Spawned {
        master: File::from(master),
        pid,
        child,
    })
}

pub fn set_window_size(master: &File, cols: u16, rows: u16) -> io::Result<()> {
    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ reads a winsize struct, which points to a live local.
    let rc = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &winsize) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Send a signal to the whole process group of a pane's first process.
pub fn signal_group(pid: i32, signal: i32) {
    if pid > 0 {
        // SAFETY: kill takes plain integers; a negative pid addresses the process group, which
        // setsid made equal to the child's pid.
        unsafe {
            libc::kill(-pid, signal);
        }
    }
}

/// The working directory of a process, when the system tells us.
#[cfg(target_os = "macos")]
pub fn process_cwd(pid: i32) -> Option<String> {
    // SAFETY: proc_pidinfo fills the struct we give it, of the size we state.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    // SAFETY: as above; `info` is a live local of that size.
    let got = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if got != size {
        return None;
    }
    // SAFETY: the path array is one block of bytes that the system ended with a zero byte.
    let path = unsafe { std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    let path = path.to_str().ok()?;
    (!path.is_empty()).then(|| path.to_string())
}

/// The working directory of a process, when the system tells us.
#[cfg(not(target_os = "macos"))]
pub fn process_cwd(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.display().to_string())
}

/// Wait up to `millis` for `fd` to have something to read or to be hung up. Used for a pane
/// whose process has ended: its last output may reach the master a moment after the exit.
pub fn wait_readable(fd: std::os::fd::RawFd, millis: i32) {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll on one live pollfd.
    unsafe {
        libc::poll(&mut pfd, 1, millis);
    }
}
