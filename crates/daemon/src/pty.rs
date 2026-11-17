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
    /// A copy of the program's end, kept open so that the output the program wrote just before
    /// it ended is still there to read: macOS throws away what the master has not read yet when
    /// the last descriptor of the program's end closes, Linux keeps it. Closing this copy is what
    /// lets the master report the end of output. (On macOS the exit of the program also waits
    /// until the master has read what it wrote, which the daemon's loop does at once.)
    pub slave: OwnedFd,
    pub pid: i32,
    pub child: std::process::Child,
}

fn env_value<'a>(env: &'a [String], name: &str) -> Option<&'a str> {
    env.iter().rev().find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        (key == name).then_some(value)
    })
}

/// Whether a program started by exec keeps this descriptor open.
pub fn set_inheritable(fd: i32, inheritable: bool) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor with plain integer arguments.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let wanted = if inheritable {
            flags & !libc::FD_CLOEXEC
        } else {
            flags | libc::FD_CLOEXEC
        };
        if libc::fcntl(fd, libc::F_SETFD, wanted) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
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
            std::ptr::addr_of_mut!(winsize),
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
        .stderr(Stdio::from(slave.try_clone()?));
    // SAFETY: the closure runs in the child between fork and exec and only calls setsid, signal,
    // sigprocmask and ioctl, which are async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            // A signal that was ignored where the daemon was started (a `&` job of a script,
            // `nohup`) stays ignored in what it starts, and then ^C would not stop anything in
            // the pane. Give the shell the default of each and an empty mask.
            for signal in [
                libc::SIGINT,
                libc::SIGQUIT,
                libc::SIGTERM,
                libc::SIGHUP,
                libc::SIGTSTP,
                libc::SIGTTIN,
                libc::SIGTTOU,
                libc::SIGPIPE,
            ] {
                libc::signal(signal, libc::SIG_DFL);
            }
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut mask);
            libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
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
        slave,
        pid,
        child,
    })
}

/// Wait until the process has ended, without taking its exit status: the status stays for
/// [`reap`]. The wait does not depend on who started the process, so a program that replaced the
/// daemon in place can wait for the children it was given.
pub fn wait_ended(pid: i32) -> io::Result<()> {
    loop {
        // SAFETY: waitid fills the siginfo struct we pass, which is a live zeroed local; with
        // WNOWAIT it leaves the process waitable.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Take the exit status of a process that has ended; `None` while it still runs or when it is
/// not ours to wait for.
pub fn reap(pid: i32) -> Option<std::process::ExitStatus> {
    use std::os::unix::process::ExitStatusExt;
    let mut status = 0;
    // SAFETY: waitpid writes one int into the local we pass.
    let rc = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    (rc == pid).then(|| std::process::ExitStatus::from_raw(status))
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

/// The name of the program a process runs, when the system tells us.
#[cfg(target_os = "macos")]
pub fn process_name(pid: i32) -> Option<String> {
    let mut name = [0u8; 256];
    // SAFETY: proc_name writes at most the number of bytes given into the buffer.
    let got = unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
    let len = usize::try_from(got).ok().filter(|n| *n > 0)?;
    Some(String::from_utf8_lossy(&name[..len.min(name.len())]).into_owned())
}

/// The name of the program a process runs, when the system tells us.
#[cfg(not(target_os = "macos"))]
pub fn process_name(pid: i32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The name of the program in the foreground of the terminal whose master side is `master`,
/// when it is not the first process (the shell) itself.
pub fn foreground_name(master: std::os::fd::RawFd, first_pid: i32) -> Option<String> {
    // SAFETY: tcgetpgrp reads the foreground group of the terminal behind a descriptor.
    let group = unsafe { libc::tcgetpgrp(master) };
    if group <= 0 || group == first_pid {
        return None;
    }
    process_name(group)
}

/// The foreground process group of the terminal whose master side is `master`.
pub fn foreground_group(master: std::os::fd::RawFd) -> Option<i32> {
    // SAFETY: tcgetpgrp reads the foreground group of the terminal behind a descriptor.
    let group = unsafe { libc::tcgetpgrp(master) };
    (group > 0).then_some(group)
}

/// Some process of the group still exists (a zombie that nobody has waited for counts).
pub fn group_alive(group: i32) -> bool {
    // SAFETY: signal 0 only asks whether the group can be signalled.
    let r = unsafe { libc::kill(-group, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The name of the program in the foreground of the terminal, the shell included: what a window
/// is called while nobody named it.
pub fn foreground_program(master: std::os::fd::RawFd, first_pid: i32) -> Option<String> {
    let pid = foreground_group(master).unwrap_or(first_pid);
    let name = process_name(pid).or_else(|| process_name(first_pid))?;
    // A login shell may show with the dash it was started with.
    let name = name.trim_start_matches('-');
    (!name.is_empty()).then(|| name.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn spec(argv: &[&str]) -> SpawnSpec {
        SpawnSpec {
            argv: argv.iter().map(|a| a.to_string()).collect(),
            cwd: String::new(),
            env: vec!["PATH=/usr/bin:/bin".to_string()],
            cols: 80,
            rows: 24,
        }
    }

    /// Everything that can be read from the master right now.
    fn read_now(master: &mut File) -> String {
        let mut text = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            match master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => text.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&text).to_string()
    }

    /// The program writes and ends at once, and nothing reads the master for a while (longer than
    /// macOS waits for it before it throws the output away: about 0.6 s): the output is still
    /// there afterwards. Without the program's end of the terminal kept open in the daemon, a
    /// command that printed and ended at once could show nothing when the daemon was slow to
    /// read. (With the end kept open macOS holds the program's exit until the master is read, so
    /// the program is waited for only after the read.)
    #[test]
    fn what_a_program_wrote_just_before_it_ended_can_still_be_read() {
        let mut spawned = spawn(&spec(&["/bin/echo", "last-words"])).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let text = read_now(&mut spawned.master);
        assert!(text.contains("last-words"), "{text:?}");
        spawned.child.wait().unwrap();
    }

    /// Letting go of the kept end is what lets the master report the end of output.
    #[test]
    fn the_master_reports_the_end_once_the_kept_end_is_closed() {
        let mut spawned = spawn(&spec(&["/bin/echo", "x"])).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(read_now(&mut spawned.master).contains('x'));
        spawned.child.wait().unwrap();
        let Spawned {
            mut master, slave, ..
        } = spawned;
        drop(slave);
        let mut buf = [0u8; 16];
        // End of output: zero bytes, or an error (Linux reports EIO); never "would block".
        match master.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => panic!("{n} more bytes"),
            Err(e) => assert_ne!(e.kind(), io::ErrorKind::WouldBlock, "{e}"),
        }
    }
}
