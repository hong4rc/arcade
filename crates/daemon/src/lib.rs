//! The Arcade daemon: sessions, panes, their processes and terminals, and the socket clients
//! attach through.

/// Take a stage-timing mark when the feature is on; nothing at all when it is off. The macros
/// are always defined, so the marks can stay in the code.
#[macro_export]
macro_rules! stage_mark {
    ($point:ident) => {
        #[cfg(feature = "stage-timing")]
        {
            $crate::stage::mark($crate::stage::Point::$point);
        }
    };
}

/// Count a system call when the feature is on; nothing at all when it is off.
#[macro_export]
macro_rules! stage_count {
    ($call:ident) => {
        #[cfg(feature = "stage-timing")]
        {
            $crate::stage::count($crate::stage::Call::$call);
        }
    };
}

mod capture;
mod clipboard;
mod commands;
mod compose;
pub mod config;
mod copy;
pub mod format;
mod keymap;
mod keys;
pub mod keytools;
mod layout;
mod links;
mod log;
mod notify;
mod paths;
mod picker;
mod power;
pub mod pty;
mod render;
mod server;
mod snapshot;
#[cfg(feature = "stage-timing")]
pub mod stage;
mod status;
mod theme;
mod tree;

pub use keymap::{Binding, Keymap, KeysConfig, Table, normalize, parse_key, typed_name};
pub use keys::{Input, KeyReader};
pub use paths::{
    check_socket_path_length, default_socket_path, default_socket_path_from, prepare_socket_dir,
    resolve_socket_path,
};
pub use server::{DaemonConfig, Resume, rehearse, resume, run};

/// The version of this program. Debug builds let a test say another one with the environment
/// variable `ARCADE_FAKE_VERSION`, to stand for a newer or older release.
pub fn program_version() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("ARCADE_FAKE_VERSION") {
        return v;
    }
    env!("CARGO_PKG_VERSION").to_string()
}
