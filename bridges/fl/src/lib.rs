//! FL Studio bridge. Everything FL lives here; the core never sees it.
//!
//! How it works (see RESEARCH.md for sources):
//! - FL only lets third-party code in through a MIDI controller script
//!   (`fl-script/device_FLit.py`). The script has no sockets, threads or
//!   file writes, so the agent talks to it with SysEx over a loopback MIDI
//!   port (loopMIDI on Windows).
//! - Wire format: `F0 7D 'F' 'L' 'i' 't' dir id_hi id_lo cont base64(json) F7`,
//!   chunked at 600 bytes of payload (Windows drops SysEx over ~1 KB).
//! - FL reports no per-parameter change events, only "dirty" flags. The
//!   script sends a `dirty` event; the bridge then re-reads the dirty
//!   tracks' plugin parameters (paginated) and emits what changed. The
//!   core's settle rule turns that into one event per gesture as usual.
//!
//! Every assumption about FL that could not be checked on Linux is marked
//! `// VERIFY ON WINDOWS`. HANDOFF.md lists them all with file:line.

pub mod bridge;
pub mod flp;
pub mod midi;
pub mod sysex;

pub use bridge::{FlBridge, FlConfig};
pub use midi::{LoopbackPort, MidiPort};
pub use sysex::{Chunker, FlEvent, FlReply, FlRequest, Frame, Dir};

/// Host tag written into events produced through this bridge.
pub const FL_HOST_TAG: &str = "fl";
