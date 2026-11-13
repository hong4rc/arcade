//! The Arcade daemon: sessions, panes, their processes and terminals, and the socket clients
//! attach through.

mod log;
mod paths;
pub mod pty;
mod render;
mod server;

pub use paths::{
    default_socket_path, default_socket_path_from, prepare_socket_dir, resolve_socket_path,
};
pub use server::{DaemonConfig, run};
