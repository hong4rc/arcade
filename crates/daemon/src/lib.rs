//! The Arcade daemon: sessions, panes, their processes and terminals, and the socket clients
//! attach through.

mod paths;
mod pty;
mod render;
mod server;

pub use paths::{default_socket_path, default_socket_path_from, resolve_socket_path};
pub use server::{DaemonConfig, run};
