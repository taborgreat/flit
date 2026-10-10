//! flit-ipc: the plugin is a thin client; the agent does the work. This is
//! the wire between them (Unix socket / Windows named pipe), the message
//! types, a blocking client with reconnect for the plugin's worker thread,
//! and a tokio server scaffold for the agent.
//!
//! Rule: the plugin's audio callback never touches this crate. Only a
//! separate plugin thread does.

pub mod client;
pub mod framing;
pub mod proto;
pub mod server;

pub use client::{Client, ClientOptions};
pub use proto::*;
pub use server::{serve, ConnId, Handler, ServerHandle};

/// Per-user socket name. Namespaced where the OS supports it (Windows named
/// pipes, Linux abstract sockets), a file in the runtime dir otherwise.
pub fn socket_name() -> SocketName {
    if let Ok(n) = std::env::var("FLIT_AGENT_SOCKET") {
        return SocketName(n);
    }
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "user".into());
    SocketName(format!("flit-agent-{user}.sock"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketName(pub String);

impl SocketName {
    pub fn is_path(&self) -> bool {
        self.0.contains('/') || self.0.contains('\\')
    }
}
