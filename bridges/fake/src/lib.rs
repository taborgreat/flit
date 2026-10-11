//! FakeHost: a simulated DAW that every test runs against. It has tracks
//! with plugins and parameters, a mixer, a transport, a project file it can
//! write and load, tracks it can render (test tones), a focus it reports as
//! presence, and it counts what the core asks of it (so tests can prove
//! that applies are diffs). It can run at any level: at level 0 it is
//! silent (the plugin alone does the work).

use flit_core::{DeviceSlot, HostTag, MixerChange, Note, ParamId, PluginIdentity, PresencePayload, PrintPoint, SongPos, TrackId, TransportChange, View};
use flit_host::{ApplyOutcome, Capabilities, HostBridge, HostDevice, HostEvent, HostModel, HostTrack, Level, ProjectSnapshot, RenderedAudio, Shown, TrackKind};
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
    pub kind: TrackKind,
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
    /// Presence tint (0xRRGGBB) and the color it had before, if tinted.
    #[serde(default)]
    pub color: u32,
}

impl Default for FakeTrack {
    fn default() -> Self {
        Self { name: String::new(), kind: TrackKind::Part, devices: BTreeMap::new(), volume: 1.0, pan: 0.0, mute: false, clips: BTreeMap::new(), tone_hz: 440.0, routing: vec![], color: 0x808080 }
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
    pub tracks: BTreeMap<TrackId, FakeTrack>,
    /// Plugins "installed" in this fake DAW.
    pub installed: BTreeSet<PluginIdentity>,
    pub files: Vec<PathBuf>,
    queue: Vec<HostEvent>,
    /// Params the core set on us (for assertions).
    pub applied: Vec<(TrackId, DeviceSlot, ParamId, f32)>,
    pub applied_mixer: Vec<(TrackId, MixerChange)>,
    pub applied_transport: Vec<TransportChange>,
    pub applied_host_state: Vec<(TrackId, Vec<u8>)>,
    pub renders: usize,
    /// Plugins actually loaded into a slot by `set_device` (a slot that
    /// already held the plugin counts nothing).
    pub device_loads: usize,
    pub device_removals: usize,
    /// Project files loaded through `load_snapshot`.
    pub snapshot_loads: usize,
    /// Other beings shown in the DAW right now.
    pub shown: Vec<Shown>,
    /// Where `follow` took us.
    pub followed: Vec<PresencePayload>,
    /// Original colors of tinted tracks, to restore.
    tinted: BTreeMap<TrackId, u32>,
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
            device_loads: 0,
            device_removals: 0,
            snapshot_loads: 0,
            shown: vec![],
            followed: vec![],
            tinted: BTreeMap::new(),
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

    /// Add a track with a plugin in slot 0. Reported to the core as a track.
    pub fn add_track(&mut self, track: &str, name: &str, kind: TrackKind, plugin: Option<PluginIdentity>, tone_hz: f32) {
        let mut t = FakeTrack { name: name.to_string(), kind, tone_hz, ..Default::default() };
        if let Some(p) = plugin {
            self.installed.insert(p.clone());
            t.devices.insert(0, FakeDevice { plugin: p, params: BTreeMap::new() });
        }
        self.tracks.insert(TrackId::new(track), t);
        if self.live() {
            self.queue.push(HostEvent::TrackSeen { track: TrackId::new(track), name: name.to_string(), kind });
        }
    }

    /// The user drops a plugin into a slot (or replaces the one there).
    pub fn add_device(&mut self, track: &str, slot: DeviceSlot, plugin: PluginIdentity) {
        let track = TrackId::new(track);
        let Some(t) = self.tracks.get_mut(&track) else { return };
        self.installed.insert(plugin.clone());
        t.devices.insert(slot.0, FakeDevice { plugin: plugin.clone(), params: BTreeMap::new() });
        if self.live() {
            self.queue.push(HostEvent::DeviceChanged { track, slot, plugin: Some(plugin), pos: self.position });
        }
    }

    /// The user removes the plugin from a slot.
    pub fn remove_device(&mut self, track: &str, slot: DeviceSlot) {
        let track = TrackId::new(track);
        let Some(t) = self.tracks.get_mut(&track) else { return };
        if t.devices.remove(&slot.0).is_some() && self.live() {
            self.queue.push(HostEvent::DeviceChanged { track, slot, plugin: None, pos: self.position });
        }
    }

    pub fn device(&self, track: &str, slot: DeviceSlot) -> Option<&PluginIdentity> {
        self.tracks.get(&TrackId::new(track))?.devices.get(&slot.0).map(|d| &d.plugin)
    }

    /// The user turns a knob. Reported live at level >= 1, silent at 0.
    pub fn twist(&mut self, track: &str, slot: DeviceSlot, param: &str, value: f32) {
        let track = TrackId::new(track);
        let Some(t) = self.tracks.get_mut(&track) else { return };
        let Some(d) = t.devices.get_mut(&slot.0) else { return };
        d.params.insert(ParamId::new(param), value);
        let plugin = d.plugin.clone();
        if self.live() {
            self.queue.push(HostEvent::ParamChanged { track, slot, plugin, param: ParamId::new(param), value, pos: self.position });
        }
    }

    /// Many small moves of one knob, like a real twist.
    pub fn sweep(&mut self, track: &str, slot: DeviceSlot, param: &str, from: f32, to: f32, steps: usize) {
        for i in 0..=steps {
            let v = from + (to - from) * i as f32 / steps as f32;
            self.twist(track, slot, param, v);
        }
    }

    pub fn set_volume(&mut self, track: &str, v: f32) {
        let track = TrackId::new(track);
        if let Some(t) = self.tracks.get_mut(&track) {
            t.volume = v;
            if self.live() {
                self.queue.push(HostEvent::MixerChanged { track, change: MixerChange::Volume(v), pos: self.position });
            }
        }
    }

    pub fn set_pan(&mut self, track: &str, p: f32) {
        let track = TrackId::new(track);
        if let Some(t) = self.tracks.get_mut(&track) {
            t.pan = p;
            if self.live() {
                self.queue.push(HostEvent::MixerChanged { track, change: MixerChange::Pan(p), pos: self.position });
            }
        }
    }

    pub fn set_mute(&mut self, track: &str, m: bool) {
        let track = TrackId::new(track);
        if let Some(t) = self.tracks.get_mut(&track) {
            t.mute = m;
            if self.live() {
                self.queue.push(HostEvent::MixerChanged { track, change: MixerChange::Mute(m), pos: self.position });
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

    pub fn write_notes(&mut self, track: &str, clip: &str, notes: Vec<Note>) {
        if let Some(t) = self.tracks.get_mut(&TrackId::new(track)) {
            t.clips.insert(clip.to_string(), notes);
        }
    }

    pub fn set_routing(&mut self, track: &str, routing: Vec<u8>) {
        if let Some(t) = self.tracks.get_mut(&TrackId::new(track)) {
            t.routing = routing;
        }
    }

    /// The user looks at something: reported as (unsettled) presence.
    pub fn focus(&mut self, view: View, track: Option<&str>, slot: Option<DeviceSlot>) {
        if self.live() {
            self.queue.push(HostEvent::Presence { presence: PresencePayload { view, track: track.map(TrackId::new), device: slot, region: None }, pos: Some(self.position) });
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

    pub fn param(&self, track: &str, slot: DeviceSlot, param: &str) -> Option<f32> {
        self.tracks.get(&TrackId::new(track))?.devices.get(&slot.0)?.params.get(&ParamId::new(param)).copied()
    }

    pub fn color(&self, track: &str) -> Option<u32> {
        self.tracks.get(&TrackId::new(track)).map(|t| t.color)
    }

    /// What the plugin sitting on a track would capture at the print point
    /// (post inserts, fader, pan; pre sends). Works at every level: this is
    /// the plugin's own capture, not a bridge render.
    pub fn capture(&self, track: &str, from: SongPos, to: SongPos) -> Option<RenderedAudio> {
        let t = self.tracks.get(&TrackId::new(track))?;
        Some(tone(t, from, to, self.tempo))
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    fn track_mut(&mut self, track: &TrackId) -> &mut FakeTrack {
        self.tracks.entry(track.clone()).or_insert_with(|| FakeTrack { name: track.0.clone(), ..Default::default() })
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
                render_track: true,
                honors_print_point: true,
                host_state: true,
                report_presence: true,
                show_presence: true,
                follow: true,
                read_state: true,
                load_snapshot: true,
            },
        }
    }

    fn poll(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut self.queue)
    }

    fn has_plugin(&self, plugin: &PluginIdentity) -> bool {
        self.live() && self.installed.contains(plugin)
    }

    fn set_param(&mut self, track: &TrackId, slot: DeviceSlot, plugin: &PluginIdentity, param: &ParamId, value: f32) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        if !self.installed.contains(plugin) {
            return ApplyOutcome::NotAvailable;
        }
        let t = self.track_mut(track);
        let d = t.devices.entry(slot.0).or_insert_with(|| FakeDevice { plugin: plugin.clone(), params: BTreeMap::new() });
        if d.plugin != *plugin {
            d.plugin = plugin.clone();
            d.params.clear();
        }
        d.params.insert(param.clone(), value);
        self.applied.push((track.clone(), slot, param.clone(), value));
        ApplyOutcome::Applied
    }

    fn set_device(&mut self, track: &TrackId, slot: DeviceSlot, plugin: Option<&PluginIdentity>) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        match plugin {
            Some(p) => {
                if !self.installed.contains(p) {
                    return ApplyOutcome::NotAvailable;
                }
                let t = self.track_mut(track);
                match t.devices.get_mut(&slot.0) {
                    Some(d) if d.plugin == *p => {}
                    Some(d) => {
                        d.plugin = p.clone();
                        d.params.clear();
                        self.device_loads += 1;
                    }
                    None => {
                        t.devices.insert(slot.0, FakeDevice { plugin: p.clone(), params: BTreeMap::new() });
                        self.device_loads += 1;
                    }
                }
            }
            None => {
                let t = self.track_mut(track);
                if t.devices.remove(&slot.0).is_some() {
                    self.device_removals += 1;
                }
            }
        }
        ApplyOutcome::Applied
    }

    fn set_mixer(&mut self, track: &TrackId, change: &MixerChange) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        let t = self.track_mut(track);
        match change {
            MixerChange::Volume(v) => t.volume = *v,
            MixerChange::Pan(p) => t.pan = *p,
            MixerChange::Mute(m) => t.mute = *m,
        }
        self.applied_mixer.push((track.clone(), change.clone()));
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
        for (track, t) in &p.tracks {
            for (clip, notes) in &t.clips {
                snap.notes.push((TrackId::new(track), clip.clone(), notes.clone()));
                snap.arrangement.push((TrackId::new(track), clip.clone(), SongPos::START));
            }
        }
        Some(snap)
    }

    fn render_track(&mut self, track: &TrackId, from: SongPos, to: SongPos) -> Option<RenderedAudio> {
        if !self.live() {
            return None;
        }
        self.renders += 1;
        let t = self.tracks.get(track)?;
        Some(tone(t, from, to, self.tempo))
    }

    fn capture_host_state(&mut self, track: &TrackId) -> Option<Vec<u8>> {
        if !self.live() {
            return None;
        }
        let t = self.tracks.get(track)?;
        if t.routing.is_empty() { None } else { Some(t.routing.clone()) }
    }

    fn apply_host_state(&mut self, track: &TrackId, blob: &[u8]) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        let t = self.track_mut(track);
        t.routing = blob.to_vec();
        self.applied_host_state.push((track.clone(), blob.to_vec()));
        ApplyOutcome::Applied
    }

    fn tracks(&self) -> Vec<TrackId> {
        self.tracks.keys().cloned().collect()
    }

    fn master_has_processing(&self) -> Option<bool> {
        Some(self.tracks.values().find(|t| t.kind == TrackKind::Master).map(|t| !t.devices.is_empty()).unwrap_or(false))
    }

    fn show_presence(&mut self, shown: &[Shown]) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        self.clear_presence();
        for s in shown {
            if let Some(track) = &s.presence.track {
                if let Some(t) = self.tracks.get_mut(track) {
                    self.tinted.entry(track.clone()).or_insert(t.color);
                    t.color = s.color;
                }
            }
        }
        self.shown = shown.to_vec();
        ApplyOutcome::Applied
    }

    fn clear_presence(&mut self) {
        for (track, color) in std::mem::take(&mut self.tinted) {
            if let Some(t) = self.tracks.get_mut(&track) {
                t.color = color;
            }
        }
        self.shown.clear();
    }

    fn follow(&mut self, presence: &PresencePayload) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        self.followed.push(presence.clone());
        ApplyOutcome::Applied
    }

    fn read_state(&mut self) -> Option<HostModel> {
        if !self.live() {
            return None;
        }
        let mut m = HostModel { tempo: Some(self.tempo), tracks: BTreeMap::new() };
        for (id, t) in &self.tracks {
            let devices = t.devices.iter().map(|(s, d)| (DeviceSlot(*s), HostDevice { plugin: d.plugin.clone(), params: d.params.clone() })).collect();
            m.tracks.insert(id.clone(), HostTrack { devices, volume: Some(t.volume), pan: Some(t.pan), mute: Some(t.mute), host_state: if t.routing.is_empty() { None } else { Some(t.routing.clone()) } });
        }
        Some(m)
    }

    fn load_snapshot(&mut self, project: &Path) -> ApplyOutcome {
        if !self.live() {
            return ApplyOutcome::Unsupported;
        }
        let Ok(p) = Self::load_project(project) else { return ApplyOutcome::NotAvailable };
        self.tempo = p.tempo;
        self.tracks = p.tracks.into_iter().map(|(k, v)| (TrackId::new(&k), v)).collect();
        for t in self.tracks.values() {
            for d in t.devices.values() {
                self.installed.insert(d.plugin.clone());
            }
        }
        self.files = p.files;
        self.snapshot_loads += 1;
        ApplyOutcome::Applied
    }
}
