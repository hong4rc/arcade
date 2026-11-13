//! Where the socket lives.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// `$XDG_RUNTIME_DIR/arcade/default.sock`, else `$TMPDIR/arcade-<uid>/default.sock`.
pub fn default_socket_path() -> PathBuf {
    default_socket_path_from(
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("TMPDIR"),
        // SAFETY: getuid has no preconditions and cannot fail.
        unsafe { libc::getuid() },
    )
}

pub fn default_socket_path_from(
    xdg_runtime_dir: Option<OsString>,
    tmpdir: Option<OsString>,
    uid: u32,
) -> PathBuf {
    match xdg_runtime_dir.filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("arcade").join("default.sock"),
        None => {
            let base = tmpdir
                .filter(|v| !v.is_empty())
                .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
            base.join(format!("arcade-{uid}")).join("default.sock")
        }
    }
}

/// Socket to use: the explicit choice, else `ARCADE_SOCKET`, else `ARCADE` (which the daemon
/// sets in every pane, so commands run from a pane find their own daemon), else the default.
pub fn resolve_socket_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    for name in ["ARCADE_SOCKET", "ARCADE"] {
        if let Some(path) = std::env::var_os(name).filter(|v| !v.is_empty()) {
            return PathBuf::from(path);
        }
    }
    default_socket_path()
}

/// Create the socket's directory (mode 0700) when it is missing, and refuse one that other
/// users could enter.
pub fn prepare_socket_dir(socket: &Path) -> io::Result<()> {
    let Some(dir) = socket.parent() else {
        return Ok(());
    };
    match fs::metadata(dir) {
        Ok(meta) => {
            // SAFETY: getuid has no preconditions and cannot fail.
            let uid = unsafe { libc::getuid() };
            if meta.uid() != uid {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is owned by another user", dir.display()),
                ));
            }
            if meta.permissions().mode() & 0o077 != 0 {
                // Only the directory made for the socket may be tightened; never touch a
                // shared one such as /tmp.
                let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name == "arcade" || name.starts_with("arcade-") || name.starts_with("arc-") {
                    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "{} is accessible to other users; use a private directory",
                            dir.display()
                        ),
                    ));
                }
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir),
        Err(e) => Err(e),
    }
}
