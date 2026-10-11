//! FLit's shared UI (PACK 2 §8.1). Everything the FLit app window and the
//! plugin panel both draw lives here, so the two can never drift apart:
//! - `theme`: design tokens, the Plex fonts, the widgets (FLIT_SPEC §12)
//! - `client`: what the UI reads from the helper and the queue it writes
//!   (`Shared`), plus the pump that moves pushes and actions over IPC
//! - `host`: what a surface (plugin or app) must provide to the screens
//! - `panel`: the screens, the Timeline bar, the Merge dialog, first run
//!
//! The crate never touches a DAW or a spine: it only talks to the helper
//! over `flit-ipc`, and only the app writes truth.

pub use egui;

pub mod client;
pub mod compact;
pub mod host;
pub mod panel;
pub mod theme;

pub use client::Shared;
pub use host::{Host, Mode, Surface};
pub use panel::PanelState;
