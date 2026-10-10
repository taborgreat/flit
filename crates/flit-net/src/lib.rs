//! flit-net: IDs, song codes, crew joining, live sync of the current branch,
//! blob transfer and live audio frames, all over iroh (QUIC, hole punching,
//! relay fallback). No server, no account, no port forwarding.
//!
//! Identity and address are the same thing: an ID is an ed25519 public key
//! and also the iroh endpoint id.

pub mod blobs;
pub mod code;
pub mod identity;
pub mod node;
pub mod proto;
pub mod quota;
pub mod sync;

pub use code::SongCode;
pub use identity::IdentityStore;
pub use node::{NetEvent, Node, NodeConfig, PeerStatus};
pub use proto::{AddrInfo, StreamFrame};
pub use quota::{Quota, QuotaState};

pub const ALPN_JOIN: &[u8] = b"flit/join/1";
pub const ALPN_SYNC: &[u8] = b"flit/sync/1";
pub const ALPN_BLOB: &[u8] = b"flit/blob/1";
pub const ALPN_STREAM: &[u8] = b"flit/stream/1";

/// Max size of one framed protocol message.
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("core: {0}")]
    Core(#[from] flit_core::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("refused: {0}")]
    Refused(String),
    #[error("bad song code")]
    BadCode,
    #[error("no song open")]
    NoSong,
    #[error("net: {0}")]
    Net(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Convert an ID to an iroh endpoint id. Same 32 bytes.
pub fn endpoint_id(id: &flit_core::Id) -> Option<iroh::EndpointId> {
    iroh::EndpointId::from_bytes(id.as_bytes()).ok()
}

pub fn id_of(ep: &iroh::EndpointId) -> flit_core::Id {
    flit_core::Id(*ep.as_bytes())
}
