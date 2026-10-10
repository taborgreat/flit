//! FakeHost: a simulated DAW that every test runs against. It has tracks
//! with plugins and parameters, a mixer, a transport, a project file it can
//! write, and lanes it can render (test tones). It can run at any level:
//! at level 0 it is silent (the plugin alone does the work).

use flit_core::{
    DeviceSlot, HostTag, LaneId, LaneKind, MixerChange, Note, ParamId, PluginIdentity, PrintPoint, SongPos,
    TransportChange,
};
use flit_host::{ApplyOutcome, Capabilities, Focus, HostBridge, HostEvent, Level, ProjectSnapshot, RenderedAudio};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const FAKE_HOST_TAG: &str = "fake";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FakeDevice {
    pub plugin: PluginIdentity,
    pub params: BTreeMap<ParamId, f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FakeTrack {
    pub name: String,
    pub kind: LaneKind,
    /// Device slots in use (slot -> plugin + params).
    pub devices: BTreeMap<u32, FakeDevice>,
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    /// Clips of notes, keyed by clip id.
    pub clips: BTreeMap<String, Vec<Note>>,
    /// Test tone frequency this track "plays" when rendered.
    pub tone_hz: f32,
    /// Host-only state (routing etc.) as opaque bytes.
    pub routing: Vec<u8>,
}

impl Default for FakeTrack {
    fn default() -> Self {
        Self { name: String::new(), kind: LaneKind::Part, devices: BTreeMap::new(), volume: 1.0, pan: 0.0, mute: false, clips: BTreeMap::new(), tone_hz: 440.0, routing: vec![] }
    }
}

/// The sample project file FakeHost writes on save.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FakeProject {
    pub tempo: f64,
    pub tracks: BTreeMap<String, FakeTrack>,
    pub files: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct FakeHost {
    level: Level,
    tag: HostTag,
    pub tempo: f64,
    pub playing: bool,
    pub position: SongPos,
    pub tracks: BTreeMap<LaneId, FakeTrack>,
    /// Plugins "installed" in this fake DAW.
    pub installed: BTreeSet<PluginIdentity>,
    pub files: Vec<PathBuf>,
    queue: Vec<HostEvent>,
    /// Params the core set on us (for assertions).
    pub applied: Vec<(LaneId, DeviceSlot, ParamId, f32)>,
    pub applied_mixer: Vec<(LaneId, MixerChange)>,
    pub applied_transport: Vec<TransportChange>,
    pub applied_host_state: Vec<(LaneId, Vec<u8>)>,
    pub renders: usize,
}

impl FakeHost {
    pub fn new(level: Level) -> Self {
        Self {
            level,
            tag: HostTag::new(FAKE_HOST_TAG),
            tempo: 120.0,
            playing: false,
            position: SongPos::START,
            tracks: BTreeMap::new(),
            installed: BTreeSet::new(),
            files: vec![],
            queue: vec![],
            applied: vec![],
            applied_mixer: vec![],
            applied_transport: vec![],
            applied_host_state: vec![],
            renders: 0,
        }
    }

    /// A fake host with a different tag, to play "another DAW" in tests.
    pub fn with_tag(mut self, tag: &str) -> Self {
        self.tag = HostTag::new(tag);
        self
    }

    pub fn level(&self) -> Level {
        self.level
    }

    fn live(&self) -> bool {
        self.level >= Level::LiveControl
    }

    pub fn install(&mut self, plugin: PluginIdentity) {
        self.installed.insert(plugin);
    }

    /// Add a track with a plugin in slot 0. Reported to the core as a lane.
    pub fn add_track(&mut self, lane: &str, name: &str, kind: LaneKind, plugin: Option<PluginIdentity>, tone_hz: f32) {
        let mut t = FakeTrack { name: name.to_string(), kind, tone_hz, ..Default::default() };
        if let Some(p) = plugin {
            self.installed.insert(p.clone());
            t.devices.insert(0, FakeDevice { plugin: p, params: BTreeMap::new() });
        }
        self.tracks.insert(LaneId::new(lane), t);
        if self.live() {
            self.queue.push(HostEvent::LaneSeen { lane: LaneId::new(lane), name: name.to_string(), kind });
        }
    }

    /// The user drops a plugin into a slot (or replaces the one there).
    pub fn add_device(&mut self, lane: &str, slot: DeviceSlot, plugin: PluginIdentity) {
        let lane = LaneId::new(lane);
        let Some(t) = self.tracks.get_mut(&lane) else { return };
        self.installed.insert(plugin.clone());
        t.devices.insert(slot.0, FakeDevice { plugin: plugin.clone(), params: BTreeMap::new() });
        if self.live() {
            self.queue.push(HostEvent::DeviceChanged { lane, slot, plugin: Some(plugin), pos: self.position });
        }
    }

    /// The user removes the plugin from a slot.
    pub fn remove_device(&mut self, lane: &str, slot: DeviceSlot) {
        let lane = LaneId::new(lane);
        let Some(t) = self.tracks.get_mut(&lane) else { return };
        if t.devices.remove(&slot.0).is_some() && self.live() {
            self.queue.push(HostEvent::DeviceChanged { lane, slot, plugin: None, pos: self.position });
        }
    }

    pub fn device(&self, lane: &str, slot: DeviceSlot) -> Option<&PluginIdentity> {
        self.tracks.get(&LaneId::new(lane))?.devices.get(&slot.0).map(|d| &d.plugin)
    }

    /// The user turns a knob. Reported live at level >= 1, silent at 0.
    pub fn twist(&mut self, lane: &str, slot: DeviceSlot, param: &str, value: f32) {
        let lane = LaneId::new(lane);
        let Some(t) = self.tracks.get_mut(&lane) else { return };
        let Some(d) = t.devices.get_mut(&slot.0) else { return };
        d.params.insert(ParamId::new(param), value);
        let plugin = d.plugin.clone();
        if self.live() {
            self.queue.push(HostEvent::ParamChanged { lane, slot, plugin, param: ParamId::new(param), value, pos: self.position });
        }
    }

    /// Many small moves of one knob, like a real twist.
    pub fn sweep(&mut self, lane: &str, slot: DeviceSlot, param: &str, from: f32, to: f32, steps: usize) {
        for i in 0..=steps {
            let v = from + (to - from) * i as f32 / steps as f32;
            self.twist(lane, slot, param, v);
        }
    }

    pub fn set_volume(&mut self, lane: &str, v: f32) {
        let lane = LaneId::new(lane);
        if let Some(t) = self.tracks.get_mut(&lane) {
            t.volume = v;
            if self.live() {
                self.queue.push(HostEvent::MixerChanged { lane, change: MixerChange::Volume(v), pos: self.position });
            }
        }
    }

    pub fn set_pan(&mut self, lane: &str, p: f32) {
        let lane = LaneId::new(lane);
        if let Some(t) = self.tracks.get_mut(&lane) {
            t.pan = p;
            if self.live() {
                self.queue.push(HostEvent::MixerChanged { lane, change: MixerChange::Pan(p), pos: self.position });
            }
        }
    }

    pub fn set_mute(&mut self, lane: &str, m: bool) {
        let lane = LaneId::new(lane);
        if let Some(t) = self.tracks.get_mut(&lane) {
            t.mute = m;
            if self.live() {
                self.queue.push(HostEvent::MixerChanged { lane, change: MixerChange::Mute(m), pos: self.position });
            }
        }
    }

    pub fn set_tempo(&mut self, bpm: f64) {
        self.tempo = bpm;
        if self.live() {
            self.queue.push(HostEvent::Transport { change: TransportChange::Tempo(bpm), pos: self.position });
        }
    }

    pub fn play(&mut self) {
        self.playing = true;
        if self.live() {
            self.queue.push(HostEvent::Transport { change: TransportChange::Play, pos: self.position });
        }
    }

    pub fn stop(&mut self) {
        self.playing = false;
        if self.live() {
            self.queue.push(HostEvent::Transport { change: TransportChange::Stop, pos: self.position });
        }
    }

    pub fn seek(&mut self, pos: SongPos) {
        self.position = pos;
    }

    pub fn write_notes(&mut self, lane: &str, clip: &str, notes: Vec<Note>) {
        if let Some(t) = self.tracks.get_mut(&LaneId::new(lane)) {
            t.clips.insert(clip.to_string(), notes);
        }
    }

    pub fn set_routing(&mut self, lane: &str, routing: Vec<u8>) {
        if let Some(t) = self.tracks.get_mut(&LaneId::new(lane)) {
            t.routing = routing;
        }
    }

    pub fn focus(&mut self, f: Focus) {
        if self.level >= Level::Ai {
            self.queue.push(HostEvent::FocusChanged(f));
        }
    }

    pub fn project(&self) -> FakeProject {
        FakeProject {
            tempo: self.tempo,
            tracks: self.tracks.iter().map(|(k, v)| (k.0.clone(), v.clone())).collect(),
            files: self.files.clone(),
        }
    }

    /// The user hits save: write the project file. Reported at level >= 2.
    pub fn save_project(&mut self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.project()).unwrap();
        std::fs::write(path, json)?;
        if self.level >= Level::Project {
            self.queue.push(HostEvent::ProjectSaved { path: path.to_path_buf(), pos: self.position });
        }
        Ok(())
    }

    pub fn load_project(path: &Path) -> std::io::Result<FakeProject> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    pub fn param(&self, lane: &str, slot: DeviceSlot, param: &str) -> Option<f32> {
        self.tracks.get(&LaneId::new(lane))?.devices.get(&slot.0)?.params.get(&ParamId::new(param)).copied()
    }

    /// What the plugin sitting on a track would capture at the print point
    /// (post inserts, fader, pan; pre sends). Works at every level: this is
    /// the plugin's own capture, not a bridge render.
    pub fn capture(&self, lane: &str, from: SongPos, to: SongPos) -> Option<RenderedAudio> {
        let t = self.tracks.get(&LaneId::new(lane))?;
        Some(tone(t, from, to, self.tempo))
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }
}

fn tone(t: &FakeTrack, from: SongPos, to: SongPos, bpm: f64) -> RenderedAudio {
    let sr = 48_000u32;
    let frames = (to.samples(bpm, sr) - from.samples(bpm, sr)).max(0) as usize;
    let gain = if t.mute { 0.0 } else { t.volume * 0.5 };
    let l = gain * (1.0 - t.pan.max(0.0));
    let r = gain * (1.0 + t.pan.min(0.0));
    // The fader and inserts applied once: a device param "x" scales the tone
    // so renders reflect settings.
    let scale = t.devices.values().flat_map(|d| d.params.values()).fold(1.0f32, |a, v| a * (0.5 + 0.5 * v));
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = (i as f32 * t.tone_hz * std::f32::consts::TAU / sr as f32).sin() * scale;
        samples.push(s * l);
        samples.push(s * r);
    }
    RenderedAudio { sample_rate: sr, channels: 2, start: from, samples, print_point: PrintPoint::PostFaderPreSend }
}

impl HostBridge for FakeHost {
    fn tag(&self) -> HostTag {
        self.tag.clone()
    }

    fn capabilities(&self) -> Capabilities {
        match self.level {
            Level::PluginOnly => Capabilities::plugin_only(),
            l => Capabilities {
                level: l,
                set_params: true,
                set_devices: true,
                set_mixer: true,
                set_transport: true,
                read_project: l >= Level::Project,
                render_lane: true,
                honors_print_point: true,
                host_state: true,
                reports_focus: l >= Level::Ai,
            },
        }
    }

    fn poll(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut self.queue)
    }

    fn has_plugin(&self, plugin: &PluginIdentity) -> bool {
        self.live() && self.installed.contains(plugin)
    }

    fn set_param(&mut self, lane: &LaneId, slot: DeviceSlot, plugin: &PluginIdentity, param: &ParamId, value: f32) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        if !self.installed.contains(plugin) {
            return ApplyOutcome::NotAvailable;
        }
        let t = self.tracks.entry(lane.clone()).or_insert_with(|| FakeTrack { name: lane.0.clone(), ..Default::default() });
        let d = t.devices.entry(slot.0).or_insert_with(|| FakeDevice { plugin: plugin.clone(), params: BTreeMap::new() });
        if d.plugin != *plugin {
            d.plugin = plugin.clone();
            d.params.clear();
        }
        d.params.insert(param.clone(), value);
        self.applied.push((lane.clone(), slot, param.clone(), value));
        ApplyOutcome::Applied
    }

    fn set_device(&mut self, lane: &LaneId, slot: DeviceSlot, plugin: Option<&PluginIdentity>) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        let t = self.tracks.entry(lane.clone()).or_insert_with(|| FakeTrack { name: lane.0.clone(), ..Default::default() });
        match plugin {
            Some(p) => {
                if !self.installed.contains(p) {
                    return ApplyOutcome::NotAvailable;
                }
                let d = t.devices.entry(slot.0).or_insert_with(|| FakeDevice { plugin: p.clone(), params: BTreeMap::new() });
                if d.plugin != *p {
                    d.plugin = p.clone();
                    d.params.clear();
                }
            }
            None => {
                t.devices.remove(&slot.0);
            }
        }
        ApplyOutcome::Applied
    }

    fn set_mixer(&mut self, lane: &LaneId, change: &MixerChange) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        let t = self.tracks.entry(lane.clone()).or_insert_with(|| FakeTrack { name: lane.0.clone(), ..Default::default() });
        match change {
            MixerChange::Volume(v) => t.volume = *v,
            MixerChange::Pan(p) => t.pan = *p,
            MixerChange::Mute(m) => t.mute = *m,
        }
        self.applied_mixer.push((lane.clone(), change.clone()));
        ApplyOutcome::Applied
    }

    fn set_transport(&mut self, change: &TransportChange) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        match change {
            TransportChange::Play => self.playing = true,
            TransportChange::Stop => self.playing = false,
            TransportChange::Position(p) => self.position = *p,
            TransportChange::Tempo(b) => self.tempo = *b,
            TransportChange::Loop { .. } | TransportChange::LoopOff => {}
        }
        self.applied_transport.push(change.clone());
        ApplyOutcome::Applied
    }

    fn read_project(&mut self, path: &Path) -> Option<ProjectSnapshot> {
        if self.level < Level::Project {
            return None;
        }
        let p = Self::load_project(path).ok()?;
        let mut snap = ProjectSnapshot { tempo: Some(p.tempo), files: p.files.clone(), ..Default::default() };
        for (lane, t) in &p.tracks {
            for (clip, notes) in &t.clips {
                snap.notes.push((LaneId::new(lane), clip.clone(), notes.clone()));
                snap.arrangement.push((LaneId::new(lane), clip.clone(), SongPos::START));
            }
        }
        Some(snap)
    }

    fn render_lane(&mut self, lane: &LaneId, from: SongPos, to: SongPos) -> Option<RenderedAudio> {
        if !self.live() {
            return None;
        }
        self.renders += 1;
        let t = self.tracks.get(lane)?;
        Some(tone(t, from, to, self.tempo))
    }

    fn capture_host_state(&mut self, lane: &LaneId) -> Option<Vec<u8>> {
        if !self.live() {
            return None;
        }
        let t = self.tracks.get(lane)?;
        if t.routing.is_empty() { None } else { Some(t.routing.clone()) }
    }

    fn apply_host_state(&mut self, lane: &LaneId, blob: &[u8]) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        let t = self.tracks.entry(lane.clone()).or_insert_with(|| FakeTrack { name: lane.0.clone(), ..Default::default() });
        t.routing = blob.to_vec();
        self.applied_host_state.push((lane.clone(), blob.to_vec()));
        ApplyOutcome::Applied
    }

    fn lanes(&self) -> Vec<LaneId> {
        self.tracks.keys().cloned().collect()
    }

    fn master_has_processing(&self) -> Option<bool> {
        Some(self.tracks.get(&LaneId::new("mix")).map(|t| !t.devices.is_empty()).unwrap_or(false))
    }
}
