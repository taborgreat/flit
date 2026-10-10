//! The bridges this agent can run. The agent is the composition root: it
//! may know every bridge; the core crates never do.

use flit_bridge_fake::FakeHost;
use flit_core::{DeviceSlot, HostTag, LaneId, MixerChange, ParamId, PluginIdentity, SongPos, TransportChange};
use flit_host::{ApplyOutcome, Capabilities, HostBridge, HostEvent, ProjectSnapshot, RenderedAudio};
use std::path::Path;

/// Level 0: no bridge at all. The plugin alone does the work.
#[derive(Debug, Default)]
pub struct NoBridge {
    pub lanes: Vec<LaneId>,
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
    fn lanes(&self) -> Vec<LaneId> {
        self.lanes.clone()
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
    fn set_param(&mut self, lane: &LaneId, slot: DeviceSlot, plugin: &PluginIdentity, param: &ParamId, value: f32) -> ApplyOutcome {
        delegate!(self, b => b.set_param(lane, slot, plugin, param, value))
    }
    fn set_mixer(&mut self, lane: &LaneId, change: &MixerChange) -> ApplyOutcome {
        delegate!(self, b => b.set_mixer(lane, change))
    }
    fn set_transport(&mut self, change: &TransportChange) -> ApplyOutcome {
        delegate!(self, b => b.set_transport(change))
    }
    fn read_project(&mut self, path: &Path) -> Option<ProjectSnapshot> {
        delegate!(self, b => b.read_project(path))
    }
    fn render_lane(&mut self, lane: &LaneId, from: SongPos, to: SongPos) -> Option<RenderedAudio> {
        delegate!(self, b => b.render_lane(lane, from, to))
    }
    fn capture_host_state(&mut self, lane: &LaneId) -> Option<Vec<u8>> {
        delegate!(self, b => b.capture_host_state(lane))
    }
    fn apply_host_state(&mut self, lane: &LaneId, blob: &[u8]) -> ApplyOutcome {
        delegate!(self, b => b.apply_host_state(lane, blob))
    }
    fn lanes(&self) -> Vec<LaneId> {
        delegate!(self, b => b.lanes())
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
}
