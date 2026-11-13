//! The Arcade daemon: sessions, panes, their processes and terminals, and the socket clients
//! attach through.

mod clipboard;
mod commands;
mod compose;
pub mod config;
mod copy;
mod keymap;
mod keys;
mod layout;
mod log;
mod paths;
mod picker;
pub mod pty;
mod render;
mod server;
mod status;
mod theme;

pub use keymap::{Keymap, KeysConfig, parse_key};
pub use paths::{
    default_socket_path, default_socket_path_from, prepare_socket_dir, resolve_socket_path,
};
pub use server::{DaemonConfig, run};
