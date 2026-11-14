//! Waiting for the keyboard, the socket and the signal pipe at once.
//!
//! `poll` is slow on macOS, where it is built on top of the event queue and sets one up on every
//! call; waiting on a queue made once is quicker, and so is the wakeup. This uses `kqueue` on
//! macOS and `epoll` on Linux, level-triggered like `poll`, so a read that leaves bytes behind
//! is reported again. When the queue cannot take one of the descriptors (some terminal devices
//! refuse), it answers `None` and the caller keeps using `poll`.

use std::io;
use std::os::fd::RawFd;

/// Which of the three descriptors can be read (or have hung up or failed).
pub type Ready = [bool; 3];

pub struct Waiter {
    queue: RawFd,
}

#[cfg(target_os = "macos")]
impl Waiter {
    pub fn new(fds: [RawFd; 3]) -> Option<Waiter> {
        // SAFETY: kqueue takes no arguments.
        let queue = unsafe { libc::kqueue() };
        if queue < 0 {
            return None;
        }
        let waiter = Waiter { queue };
        for (index, fd) in fds.into_iter().enumerate() {
            let change = libc::kevent {
                ident: fd as usize,
                filter: libc::EVFILT_READ,
                flags: libc::EV_ADD | libc::EV_ENABLE,
                fflags: 0,
                data: 0,
                udata: index as *mut libc::c_void,
            };
            // SAFETY: one change from a live struct, no event list.
            let rc = unsafe {
                libc::kevent(queue, &change, 1, std::ptr::null_mut(), 0, std::ptr::null())
            };
            if rc < 0 {
                return None;
            }
        }
        Some(waiter)
    }

    /// Wait until something can be read. `Ok(None)` when a signal interrupted the wait.
    pub fn wait(&self) -> io::Result<Option<Ready>> {
        // SAFETY: an all-zero kevent is a valid value.
        let mut events: [libc::kevent; 3] = unsafe { std::mem::zeroed() };
        // SAFETY: the event list is three live structs; no timeout means wait.
        let n = unsafe {
            libc::kevent(
                self.queue,
                std::ptr::null(),
                0,
                events.as_mut_ptr(),
                3,
                std::ptr::null(),
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(e)
            };
        }
        let mut ready = [false; 3];
        for event in &events[..n as usize] {
            ready[event.udata as usize] = true;
        }
        Ok(Some(ready))
    }
}

#[cfg(target_os = "linux")]
impl Waiter {
    pub fn new(fds: [RawFd; 3]) -> Option<Waiter> {
        // SAFETY: epoll_create1 takes only a flag.
        let queue = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if queue < 0 {
            return None;
        }
        let waiter = Waiter { queue };
        for (index, fd) in fds.into_iter().enumerate() {
            let mut event = libc::epoll_event {
                events: libc::EPOLLIN as u32,
                u64: index as u64,
            };
            // SAFETY: one live event struct.
            if unsafe { libc::epoll_ctl(queue, libc::EPOLL_CTL_ADD, fd, &mut event) } < 0 {
                return None;
            }
        }
        Some(waiter)
    }

    /// Wait until something can be read. `Ok(None)` when a signal interrupted the wait.
    pub fn wait(&self) -> io::Result<Option<Ready>> {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 3];
        // SAFETY: the event list is three live structs; -1 waits without a limit.
        let n = unsafe { libc::epoll_wait(self.queue, events.as_mut_ptr(), 3, -1) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted {
                Ok(None)
            } else {
                Err(e)
            };
        }
        let mut ready = [false; 3];
        for event in &events[..n as usize] {
            ready[event.u64 as usize] = true;
        }
        Ok(Some(ready))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl Waiter {
    pub fn new(_fds: [RawFd; 3]) -> Option<Waiter> {
        None
    }

    pub fn wait(&self) -> io::Result<Option<Ready>> {
        Ok(None)
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        // SAFETY: closing the queue we made.
        unsafe {
            libc::close(self.queue);
        }
    }
}
