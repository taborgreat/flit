//! flit-host: the one trait every DAW bridge implements, plus the pieces of
//! the live flow that are the same for every host (settle rule, receiver
//! decision, the loop that turns host changes into events).
//!
//! The core never sees a DAW. A bridge translates one DAW into `HostEvent`s
//! and back; this crate says what a bridge must provide at each level.

pub mod bridge;
pub mod live;
pub mod settle;

pub use bridge::{
    ApplyOutcome, Baseline, Capabilities, Focus, HostBridge, HostEvent, Level, ProjectSnapshot, RenderedAudio,
};
pub use live::{Applied, ChunkEncoder, LiveLoop, PcmChunkEncoder, Receiver, ReceiverDecision};
pub use settle::{ControlKey, Settled, Settler, DEFAULT_SETTLE_MS};
