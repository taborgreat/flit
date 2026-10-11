//! The bridges this agent can run. The agent is the composition root: it
//! may know every bridge; the core crates never do.

use flit_bridge_fake::FakeHost;
use flit_core::{DeviceSlot, HostTag, MixerChange, ParamId, PluginIdentity, PresencePayload, SongPos, TrackId, TransportChange};
use flit_host::{ApplyOutcome, ApplyReport, Capabilities, Diff, HostBridge, HostEvent, HostModel, ProjectSnapshot, RenderedAudio, Shown};
use std::path::Path;

/// Level 0: no bridge at all. The plugin alone does the work.
#[derive(Debug, Default)]
pub struct NoBridge {
    pub tracks: Vec<TrackId>,
}

impl HostBridge for NoBridge {
    fn tag(&self) -> HostTag {
        HostTag::plugin_only()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::plugin_only()
    }
    fn poll(&mut self) -> Vec<HostEvent> {
        Vec::new()
    }
    fn tracks(&self) -> Vec<TrackId> {
        self.tracks.clone()
    }
}

pub enum AnyBridge {
    None(NoBridge),
    Fake(FakeHost),
    Fl(flit_bridge_fl::FlBridge),
}

impl AnyBridge {
    pub fn fake(&mut self) -> Option<&mut FakeHost> {
        match self {
            AnyBridge::Fake(f) => Some(f),
            _ => None,
        }
    }
}

macro_rules! delegate {
    ($self:ident, $b:ident => $e:expr) => {
        match $self {
            AnyBridge::None($b) => $e,
            AnyBridge::Fake($b) => $e,
            AnyBridge::Fl($b) => $e,
        }
    };
}

impl HostBridge for AnyBridge {
    fn tag(&self) -> HostTag {
        delegate!(self, b => b.tag())
    }
    fn capabilities(&self) -> Capabilities {
        delegate!(self, b => b.capabilities())
    }
    fn poll(&mut self) -> Vec<HostEvent> {
        delegate!(self, b => b.poll())
    }
    fn has_plugin(&self, plugin: &PluginIdentity) -> bool {
        delegate!(self, b => b.has_plugin(plugin))
    }
    fn set_param(&mut self, track: &TrackId, slot: DeviceSlot, plugin: &PluginIdentity, param: &ParamId, value: f32) -> ApplyOutcome {
        delegate!(self, b => b.set_param(track, slot, plugin, param, value))
    }
    fn set_mixer(&mut self, track: &TrackId, change: &MixerChange) -> ApplyOutcome {
        delegate!(self, b => b.set_mixer(track, change))
    }
    fn set_device(&mut self, track: &TrackId, slot: DeviceSlot, plugin: Option<&PluginIdentity>) -> ApplyOutcome {
        delegate!(self, b => b.set_device(track, slot, plugin))
    }
    fn set_transport(&mut self, change: &TransportChange) -> ApplyOutcome {
        delegate!(self, b => b.set_transport(change))
    }
    fn read_project(&mut self, path: &Path) -> Option<ProjectSnapshot> {
        delegate!(self, b => b.read_project(path))
    }
    fn render_track(&mut self, track: &TrackId, from: SongPos, to: SongPos) -> Option<RenderedAudio> {
        delegate!(self, b => b.render_track(track, from, to))
    }
    fn capture_host_state(&mut self, track: &TrackId) -> Option<Vec<u8>> {
        delegate!(self, b => b.capture_host_state(track))
    }
    fn apply_host_state(&mut self, track: &TrackId, blob: &[u8]) -> ApplyOutcome {
        delegate!(self, b => b.apply_host_state(track, blob))
    }
    fn tracks(&self) -> Vec<TrackId> {
        delegate!(self, b => b.tracks())
    }
    fn master_has_processing(&self) -> Option<bool> {
        delegate!(self, b => b.master_has_processing())
    }
    fn current_project(&self) -> Option<std::path::PathBuf> {
        delegate!(self, b => b.current_project())
    }
    fn describe(&self) -> String {
        delegate!(self, b => b.describe())
    }
    fn request_save(&mut self) -> ApplyOutcome {
        delegate!(self, b => b.request_save())
    }
    fn link_up(&self) -> Option<bool> {
        delegate!(self, b => b.link_up())
    }
    fn show_presence(&mut self, shown: &[Shown]) -> ApplyOutcome {
        delegate!(self, b => b.show_presence(shown))
    }
    fn clear_presence(&mut self) {
        delegate!(self, b => b.clear_presence())
    }
    fn follow(&mut self, presence: &PresencePayload) -> ApplyOutcome {
        delegate!(self, b => b.follow(presence))
    }
    fn read_state(&mut self) -> Option<HostModel> {
        delegate!(self, b => b.read_state())
    }
    fn load_snapshot(&mut self, project: &Path) -> ApplyOutcome {
        delegate!(self, b => b.load_snapshot(project))
    }
    fn estimate_cost(&self, diff: &Diff) -> u32 {
        delegate!(self, b => b.estimate_cost(diff))
    }
    fn apply_diff(&mut self, diff: &Diff) -> ApplyReport {
        delegate!(self, b => b.apply_diff(diff))
    }
}
