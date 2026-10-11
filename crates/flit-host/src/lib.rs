//! flit-host: the one trait every DAW bridge implements, plus the pieces of
//! the live flow that are the same for every host (settle rule, receiver
//! decision, the loop that turns host changes into acts, the diff that
//! shows a moment in the DAW without wrecking it).
//!
//! The core never sees a DAW. A bridge translates one DAW into `HostEvent`s
//! and back; this crate says what a bridge must provide at each level.

pub mod bridge;
pub mod live;
pub mod settle;

pub use bridge::{diff, ApplyOutcome, ApplyReport, Baseline, Capabilities, Diff, HostBridge, HostDevice, HostEvent, HostModel, HostTrack, Level, ProjectSnapshot, RenderedAudio, Shown, TrackDiff, TrackKind, DEVICE_COST, SNAPSHOT_THRESHOLD};
pub use live::{notes_delta, ChunkEncoder, LiveLoop, PcmChunkEncoder, Receiver, ReceiverDecision, PRESENCE_SETTLE_MS};
pub use settle::{ControlKey, Settled, Settler, DEFAULT_SETTLE_MS};
