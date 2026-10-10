//! The live flow, the same for every host:
//!   bridge sees a change -> settle -> one event -> peers.
//!   peer event -> decide (apply setting / hear audio) -> bridge.
//!   save -> Version + render changed lanes -> chunk, hash, Audio event.

use crate::bridge::{ApplyOutcome, Baseline, HostBridge, HostEvent, Level, RenderedAudio};
use flit_core::state::BranchState;
use crate::settle::{ControlKey, Settled, Settler};
use flit_core::chunk::ChunkPlan;
use flit_core::{
    AudioPayload, BlobStore, Stamp, StampId, Hash, HostTag, Keypair, LaneId, LaneKind, BranchId, ManifestEntry,
    MixerChange, Payload, PluginIdentity, PrintPoint, Song, SongPos, Time, TransportChange,
};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Turns rendered PCM into the two chunk sets of an Audio event. The real
/// one (Opus + FLAC) lives in flit-audio; tests use `PcmChunkEncoder`.
pub trait ChunkEncoder: Send {
    /// Returns (listening chunks, archive chunks), one of each per bar.
    fn encode(&mut self, store: &BlobStore, audio: &RenderedAudio, plan: &ChunkPlan) -> flit_core::Result<(Vec<Hash>, Vec<Hash>)>;
    fn bit_depth(&self) -> u8 {
        24
    }
}

/// Raw f32 chunks for both sets. Tests only.
#[derive(Default)]
pub struct PcmChunkEncoder;

impl ChunkEncoder for PcmChunkEncoder {
    fn encode(&mut self, store: &BlobStore, audio: &RenderedAudio, plan: &ChunkPlan) -> flit_core::Result<(Vec<Hash>, Vec<Hash>)> {
        let hashes = flit_core::chunk::store_audio(store, &audio.samples, plan)?;
        Ok((hashes.clone(), hashes))
    }
    fn bit_depth(&self) -> u8 {
        32
    }
}

/// One member's live session: their bridge, their key, their branch.
pub struct LiveLoop<B: HostBridge, E: ChunkEncoder> {
    pub bridge: B,
    pub encoder: E,
    settler: Settler,
    key: Keypair,
    branch: BranchId,
    /// Lanes with changes since this host's last Version on this branch.
    dirty: BTreeSet<LaneId>,
    /// Audio the plugin captured itself (level 0, or when the bridge can't render).
    captured: HashMap<LaneId, RenderedAudio>,
    /// Host events for lanes someone else owns (dropped, counted).
    pub ignored_foreign: usize,
    pub tempo_hint: f64,
    claimed: Vec<Stamp>,
    /// Name for the next Version the host reports (the user pressed Save
    /// version and the host was asked to save).
    pub next_version_name: Option<String>,
    /// Notices from the bridge, for the panel (drained by the agent).
    pub notices: Vec<String>,
}

impl<B: HostBridge, E: ChunkEncoder> LiveLoop<B, E> {
    pub fn new(bridge: B, encoder: E, key: Keypair, branch: BranchId) -> Self {
        Self {
            bridge,
            encoder,
            settler: Settler::default(),
            key,
            branch,
            dirty: BTreeSet::new(),
            captured: HashMap::new(),
            ignored_foreign: 0,
            tempo_hint: 120.0,
            claimed: Vec::new(),
            next_version_name: None,
            notices: Vec::new(),
        }
    }

    pub fn with_settle_ms(mut self, ms: u64) -> Self {
        self.settler = Settler::new(ms);
        self
    }

    pub fn branch(&self) -> BranchId {
        self.branch
    }

    /// Switch branch (you are on exactly one branch at a time).
    pub fn set_branch(&mut self, branch: BranchId) {
        self.branch = branch;
        self.settler = Settler::default();
        self.dirty.clear();
    }

    pub fn id(&self) -> flit_core::Id {
        self.key.id()
    }

    pub fn host(&self) -> HostTag {
        self.bridge.tag()
    }

    /// Prime the settler from the song so untouched knobs stay silent.
    pub fn prime_from(&mut self, song: &Song) {
        if let Some(st) = song.state(&self.branch) {
            for (lane, ls) in &st.lanes {
                for (k, v) in &ls.settings {
                    self.settler.prime(ControlKey::Param { lane: lane.clone(), slot: k.slot, param: k.param.clone() }, v.value as f64);
                }
                self.settler.prime(ControlKey::Volume(lane.clone()), ls.mixer.volume as f64);
                self.settler.prime(ControlKey::Pan(lane.clone()), ls.mixer.pan as f64);
                self.settler.prime(ControlKey::Mute(lane.clone()), if ls.mixer.mute { 1.0 } else { 0.0 });
            }
            self.settler.prime(ControlKey::Tempo, st.transport.tempo);
            self.tempo_hint = st.transport.tempo;
        }
    }

    /// Level 0 path: the plugin read tempo/transport from its host's
    /// transport info. Treated exactly like a bridge report (tempo settles,
    /// play/stop are instant).
    pub fn report_transport(&mut self, song: &mut Song, change: TransportChange, pos: SongPos, now_ms: u64) -> flit_core::Result<Vec<Stamp>> {
        match change {
            TransportChange::Tempo(bpm) => {
                self.settler.input(ControlKey::Tempo, bpm, None, pos, now_ms);
                Ok(vec![])
            }
            other => {
                let ev = song.transport(&self.key, &self.branch, other, &self.host(), Time::at(now_ms, pos))?;
                Ok(vec![ev])
            }
        }
    }

    /// Mark a lane as changed since the last version (level 0: the plugin
    /// on it is sending audio, so a save must render it).
    pub fn push_dirty(&mut self, lane: LaneId) {
        self.dirty.insert(lane);
    }

    /// The plugin on a lane captured that lane's output (level 0 path).
    pub fn push_captured(&mut self, lane: LaneId, audio: RenderedAudio) {
        self.dirty.insert(lane.clone());
        self.captured.insert(lane, audio);
    }

    /// Poll the bridge, settle, append events. Returns what was appended.
    pub fn tick(&mut self, song: &mut Song, now_ms: u64) -> flit_core::Result<Vec<Stamp>> {
        let mut out = Vec::new();
        for hev in self.bridge.poll() {
            match hev {
                HostEvent::ParamChanged { lane, slot, plugin, param, value, pos } => {
                    self.settler.input(ControlKey::Param { lane, slot, param }, value as f64, Some(plugin), pos, now_ms);
                }
                HostEvent::DeviceChanged { lane, slot, plugin, pos } => {
                    // Not settled: a plugin appears or goes at once.
                    if self.ensure_owned(song, &lane, now_ms, pos)? {
                        self.dirty.insert(lane.clone());
                        let host = self.host();
                        out.push(song.device(&self.key, &self.branch, &lane, slot, plugin, &host, Time::at(now_ms, pos))?);
                    }
                }
                HostEvent::MixerChanged { lane, change, pos } => {
                    let (key, v) = match change {
                        flit_core::MixerChange::Volume(v) => (ControlKey::Volume(lane), v as f64),
                        flit_core::MixerChange::Pan(p) => (ControlKey::Pan(lane), p as f64),
                        flit_core::MixerChange::Mute(m) => (ControlKey::Mute(lane), if m { 1.0 } else { 0.0 }),
                    };
                    self.settler.input(key, v, None, pos, now_ms);
                }
                HostEvent::Transport { change, pos } => match change {
                    TransportChange::Tempo(bpm) => self.settler.input(ControlKey::Tempo, bpm, None, pos, now_ms),
                    other => {
                        let ev = song.transport(&self.key, &self.branch, other, &self.host(), Time::at(now_ms, pos))?;
                        out.push(ev);
                    }
                },
                HostEvent::Baseline { baseline, pos } => {
                    out.extend(self.record_baseline(song, baseline, now_ms, pos)?);
                }
                HostEvent::ProjectSaved { path, pos } => {
                    let name = self.next_version_name.take().unwrap_or_else(|| "Save".into());
                    out.extend(self.save_version(song, &name, &path, now_ms, pos)?);
                }
                HostEvent::ProjectOpened { path, pos } => {
                    if !self.has_version(song) {
                        let name = path.file_name().map(|n| format!("Opened {}", n.to_string_lossy())).unwrap_or_else(|| "Opened".into());
                        out.extend(self.save_version(song, &name, &path, now_ms, pos)?);
                    }
                }
                HostEvent::LaneSeen { lane, name, kind } => {
                    if song.state(&self.branch).and_then(|s| s.owner(&lane)).is_none() {
                        let ev = song.claim_lane(&self.key, &self.branch, &lane, kind, Some(&name), &self.host(), Time::at(now_ms, SongPos::START))?;
                        out.push(ev);
                    }
                }
                HostEvent::FocusChanged(_) => {}
                HostEvent::Notice(text) => self.notices.push(text),
            }
        }
        for s in self.settler.flush(now_ms) {
            if let Some(ev) = self.emit_settled(song, s, now_ms)? {
                out.push(ev);
            }
        }
        Ok(self.with_claims(out))
    }

    /// Lane claims made along the way go first, in causal order.
    fn with_claims(&mut self, mut out: Vec<Stamp>) -> Vec<Stamp> {
        if self.claimed.is_empty() {
            return out;
        }
        // The batch in the order it was stamped: the act chain links every
        // stamp of this Name, so the fold's merge order recovers it exactly.
        let mut all = std::mem::take(&mut self.claimed);
        all.append(&mut out);
        let map: HashMap<flit_core::StampId, Stamp> = all.into_iter().map(|s| (s.id, s)).collect();
        let members: std::collections::HashSet<flit_core::StampId> = map.keys().copied().collect();
        flit_core::seal::fold::merge_order(|id| map.get(id), &members).into_iter().filter_map(|id| map.get(&id).cloned()).collect()
    }

    /// Does this host already have a Version on the current branch?
    pub fn has_version(&self, song: &Song) -> bool {
        song.state(&self.branch).map(|s| s.latest_version.contains_key(&self.host())).unwrap_or(false)
    }

    /// A control's value from before its first move. If the song has no
    /// value for it yet, it lands as a normal Setting/Mixer event so a later
    /// Go back can put the control where it was; either way the settler now
    /// knows the start value, so a knob returned there makes no event.
    fn record_baseline(&mut self, song: &mut Song, b: Baseline, now_ms: u64, pos: SongPos) -> flit_core::Result<Vec<Stamp>> {
        let host = self.host();
        let mut out = Vec::new();
        match b {
            Baseline::Param { lane, slot, plugin, param, value } => {
                let known = song.state(&self.branch).and_then(|s| s.lane(&lane)).and_then(|l| l.setting(slot, &param));
                let key = ControlKey::Param { lane: lane.clone(), slot, param: param.clone() };
                match known {
                    Some(v) => self.settler.prime(key, v as f64),
                    None => {
                        if self.ensure_owned(song, &lane, now_ms, pos)? {
                            out.push(song.setting(&self.key, &self.branch, &lane, slot, &plugin, &param, value.clamp(0.0, 1.0), &host, Time::at(now_ms, pos))?);
                            self.settler.prime(key, value as f64);
                        }
                    }
                }
            }
            Baseline::Mixer { lane, change } => {
                let ls = song.state(&self.branch).and_then(|s| s.lane(&lane).cloned());
                let (key, value, known) = match change {
                    MixerChange::Volume(v) => (ControlKey::Volume(lane.clone()), v as f64, ls.as_ref().filter(|l| l.mixer_set.volume).map(|l| l.mixer.volume as f64)),
                    MixerChange::Pan(p) => (ControlKey::Pan(lane.clone()), p as f64, ls.as_ref().filter(|l| l.mixer_set.pan).map(|l| l.mixer.pan as f64)),
                    MixerChange::Mute(m) => (ControlKey::Mute(lane.clone()), if m { 1.0 } else { 0.0 }, ls.as_ref().filter(|l| l.mixer_set.mute).map(|l| if l.mixer.mute { 1.0 } else { 0.0 })),
                };
                match known {
                    Some(v) => self.settler.prime(key, v),
                    None => {
                        if self.ensure_owned(song, &lane, now_ms, pos)? {
                            out.push(song.mixer(&self.key, &self.branch, &lane, change, &host, Time::at(now_ms, pos))?);
                            self.settler.prime(key, value);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Lanes claimed on the way (reported with the next batch of events).
    fn ensure_owned(&mut self, song: &mut Song, lane: &LaneId, now_ms: u64, pos: SongPos) -> flit_core::Result<bool> {
        match song.state(&self.branch).and_then(|s| s.owner(lane)) {
            Some(o) if o == self.key.id() => Ok(true),
            Some(_) => {
                self.ignored_foreign += 1;
                Ok(false)
            }
            None => {
                let ev = song.claim_lane(&self.key, &self.branch, lane, LaneKind::Part, None, &self.host(), Time::at(now_ms, pos))?;
                self.claimed.push(ev);
                Ok(true)
            }
        }
    }

    fn emit_settled(&mut self, song: &mut Song, s: Settled, now_ms: u64) -> flit_core::Result<Option<Stamp>> {
        let host = self.host();
        Ok(match s {
            Settled::Param { lane, slot, plugin, param, value, pos } => {
                if !self.ensure_owned(song, &lane, now_ms, pos)? {
                    return Ok(None);
                }
                self.dirty.insert(lane.clone());
                Some(song.setting(&self.key, &self.branch, &lane, slot, &plugin, &param, value.clamp(0.0, 1.0), &host, Time::at(now_ms, pos))?)
            }
            Settled::Mixer { lane, change, pos } => {
                if !self.ensure_owned(song, &lane, now_ms, pos)? {
                    return Ok(None);
                }
                self.dirty.insert(lane.clone());
                Some(song.mixer(&self.key, &self.branch, &lane, change, &host, Time::at(now_ms, pos))?)
            }
            Settled::Tempo { bpm, pos } => {
                self.tempo_hint = bpm;
                Some(song.tempo(&self.key, &self.branch, bpm, &host, Time::at(now_ms, pos))?)
            }
        })
    }

    /// Save version: project file -> blob + Version event, then render only
    /// the lanes changed since this host's last version, chunk, hash, Audio.
    /// At level 0 there is no project file: pass the plugin's own state file
    /// (or any path) and the captured lane audio via `push_captured`.
    pub fn save_version(&mut self, song: &mut Song, name: &str, project_path: &Path, now_ms: u64, pos: SongPos) -> flit_core::Result<Vec<Stamp>> {
        let host = self.host();
        let time = Time::at(now_ms, pos);
        let mut out = Vec::new();

        // Level 2: notes/clips from the project file.
        let mut used_files: Vec<PathBuf> = Vec::new();
        if self.bridge.capabilities().read_project {
            if let Some(snap) = self.bridge.read_project(project_path) {
                used_files = snap.files.clone();
                for (lane, clip, notes) in snap.notes {
                    if !self.ensure_owned(song, &lane, now_ms, pos)? {
                        continue;
                    }
                    let current = song.state(&self.branch).and_then(|s| s.lane(&lane)).and_then(|l| l.clips.get(&clip).cloned());
                    if current.as_ref() != Some(&notes) {
                        out.push(song.notes(&self.key, &self.branch, &lane, &clip, notes, &host, time)?);
                        self.dirty.insert(lane.clone());
                    }
                }
            }
        }

        // Version
        let project = if project_path.is_file() { song.blobs().put_file(project_path)? } else { song.blobs().put(b"")? };
        let mut manifest = Vec::new();
        for f in &used_files {
            if f.is_file() {
                let hash = song.blobs().put_file(f)?;
                manifest.push(ManifestEntry { path: f.to_string_lossy().into_owned(), hash, size: std::fs::metadata(f)?.len() });
            }
        }
        out.push(song.version(&self.key, &self.branch, name, project, manifest, &host, time)?);

        // Render changed lanes I own.
        let mine: Vec<LaneId> = self.dirty.iter().cloned().collect();
        let tempo = song.state(&self.branch).map(|s| s.transport.tempo).unwrap_or(self.tempo_hint);
        for lane in mine {
            if !self.ensure_owned(song, &lane, now_ms, pos)? {
                continue;
            }
            let rendered = self.captured.remove(&lane).or_else(|| {
                if self.bridge.capabilities().render_lane {
                    self.bridge.render_lane(&lane, SongPos::START, pos)
                } else {
                    None
                }
            });
            let Some(audio) = rendered else { continue };
            if audio.samples.is_empty() {
                continue;
            }
            let plan = ChunkPlan { sample_rate: audio.sample_rate, channels: audio.channels, bpm: tempo, time_sig: Default::default() };
            let (opus, flac) = self.encoder.encode(song.blobs(), &audio, &plan)?;
            let payload = AudioPayload {
                start: audio.start,
                sample_rate: audio.sample_rate,
                bit_depth: self.encoder.bit_depth(),
                channels: audio.channels,
                print_point: audio.print_point,
                opus_chunks: opus,
                flac_chunks: flac,
                credit: None,
            };
            out.push(song.audio(&self.key, &self.branch, &lane, payload, &host, time)?);
        }
        self.dirty.clear();
        Ok(self.with_claims(out))
    }

    pub fn dirty_lanes(&self) -> Vec<LaneId> {
        self.dirty.iter().cloned().collect()
    }
}

/// What a receiver does with an event from a peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiverDecision {
    /// Same plugin, same version, bridge can set params: apply it.
    ApplySetting,
    ApplyMixer,
    /// The bridge can place/remove plugins: do it.
    ApplyDevice,
    ApplyTransport,
    /// Same host: apply its opaque state.
    ApplyHostState,
    /// Can't reproduce it here: play the owner's live stream for that lane.
    HearAsAudio(LaneId),
    /// Nothing for the host to do (chat, crew, versions...).
    Nothing,
}

/// The receiver side of the live flow.
pub struct Receiver;

impl Receiver {
    /// Decide, in order: (a) have the plugin and a bridge that sets params
    /// -> apply; (b) recipe in the song -> build (later milestone); (c)
    /// else hear the owner's audio. HostState applies only on the same host.
    pub fn decide<B: HostBridge + ?Sized>(bridge: &B, ev: &Stamp) -> ReceiverDecision {
        let caps = bridge.capabilities();
        let lane = ev.lane().cloned();
        match ev.payload() {
            Payload::Setting(s) => {
                if caps.level >= Level::LiveControl && caps.set_params && bridge.has_plugin(&s.plugin) {
                    ReceiverDecision::ApplySetting
                } else {
                    lane.map(ReceiverDecision::HearAsAudio).unwrap_or(ReceiverDecision::Nothing)
                }
            }
            Payload::Mixer(_) => {
                if caps.level >= Level::LiveControl && caps.set_mixer {
                    ReceiverDecision::ApplyMixer
                } else {
                    lane.map(ReceiverDecision::HearAsAudio).unwrap_or(ReceiverDecision::Nothing)
                }
            }
            Payload::Device(_) => {
                if caps.level >= Level::LiveControl && caps.set_devices {
                    ReceiverDecision::ApplyDevice
                } else {
                    lane.map(ReceiverDecision::HearAsAudio).unwrap_or(ReceiverDecision::Nothing)
                }
            }
            Payload::Transport(_) => {
                if caps.level >= Level::LiveControl && caps.set_transport {
                    ReceiverDecision::ApplyTransport
                } else {
                    ReceiverDecision::Nothing
                }
            }
            Payload::HostState(h) => {
                if h.host == bridge.tag() && caps.host_state {
                    ReceiverDecision::ApplyHostState
                } else {
                    lane.map(ReceiverDecision::HearAsAudio).unwrap_or(ReceiverDecision::Nothing)
                }
            }
            Payload::Notes(_) | Payload::Audio(_) => lane.map(ReceiverDecision::HearAsAudio).unwrap_or(ReceiverDecision::Nothing),
            Payload::Bring(b) => {
                // A group: apply what can be applied, hear the rest.
                let any_hear = b.facts.iter().any(|e| matches!(Self::decide(bridge, e), ReceiverDecision::HearAsAudio(_)));
                if any_hear {
                    lane.map(ReceiverDecision::HearAsAudio).unwrap_or(ReceiverDecision::Nothing)
                } else {
                    ReceiverDecision::ApplySetting
                }
            }
            _ => ReceiverDecision::Nothing,
        }
    }

    /// Apply an event that is IN THE LOG (so it passed validation) to the
    /// host. Returns the decision taken. Never call with an event that was
    /// not accepted by `Song::import`.
    pub fn apply<B: HostBridge + ?Sized>(bridge: &mut B, song: &Song, id: &StampId) -> Option<ReceiverDecision> {
        let ev = song.chains().get(id)?.clone();
        let decision = Self::decide(bridge, &ev);
        let lane = ev.lane().cloned();
        let outcome = match (&decision, ev.payload()) {
            (ReceiverDecision::ApplySetting, Payload::Setting(s)) => {
                bridge.set_param(lane.as_ref()?, s.slot, &s.plugin, &s.param, s.value)
            }
            (ReceiverDecision::ApplySetting, Payload::Bring(b)) => {
                let mut res = ApplyOutcome::Applied;
                for inner in &b.facts {
                    let r = match inner.payload() {
                        Payload::Setting(s) => bridge.set_param(lane.as_ref()?, s.slot, &s.plugin, &s.param, s.value),
                        Payload::Mixer(m) => bridge.set_mixer(lane.as_ref()?, m),
                        Payload::Device(d) => bridge.set_device(lane.as_ref()?, d.slot, d.plugin.as_ref()),
                        Payload::HostState(h) => bridge.apply_host_state(lane.as_ref()?, &h.blob),
                        _ => ApplyOutcome::Applied,
                    };
                    if r != ApplyOutcome::Applied {
                        res = r;
                    }
                }
                res
            }
            (ReceiverDecision::ApplyMixer, Payload::Mixer(m)) => bridge.set_mixer(lane.as_ref()?, m),
            (ReceiverDecision::ApplyDevice, Payload::Device(d)) => bridge.set_device(lane.as_ref()?, d.slot, d.plugin.as_ref()),
            (ReceiverDecision::ApplyTransport, Payload::Transport(t)) => bridge.set_transport(t),
            (ReceiverDecision::ApplyHostState, Payload::HostState(h)) => bridge.apply_host_state(lane.as_ref()?, &h.blob),
            _ => ApplyOutcome::Applied,
        };
        // The host turned out not to have it after all: fall back to audio.
        if outcome == ApplyOutcome::NotAvailable {
            return lane.map(ReceiverDecision::HearAsAudio);
        }
        Some(decision)
    }
}

/// What `Receiver::apply_state` managed to push into the host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    pub settings: usize,
    pub mixer: usize,
    pub devices: usize,
    pub tempo: bool,
    /// Lanes with settings this host cannot reproduce (hear them as audio).
    pub heard: Vec<LaneId>,
    /// Plugin placements the host cannot do from outside: what the user
    /// must do by hand to match the moment ("Drums: put X in slot 2").
    pub manual: Vec<String>,
}

impl Receiver {
    /// Push the whole sound of a branch into the host: after Go back, Undo,
    /// Bring in or switching branches the DAW must follow History. Every
    /// setting the host can set is set; faders/pans/mutes the song has
    /// recorded are set; the tempo is set. Untouched controls stay as they
    /// are in the host. Lanes the host cannot reproduce come back in
    /// `heard` so their owner's audio can be played instead.
    pub fn apply_state<B: HostBridge + ?Sized>(bridge: &mut B, state: &BranchState) -> Applied {
        let caps = bridge.capabilities();
        let mut r = Applied::default();
        if caps.level < Level::LiveControl {
            r.heard = state.lanes.iter().filter(|(_, l)| !l.settings.is_empty() || !l.audio.is_empty()).map(|(l, _)| l.clone()).collect();
            return r;
        }
        for (lane, ls) in &state.lanes {
            let mut heard = false;
            // devices first: a setting lands on the plugin that is there
            for (slot, plugin) in &ls.devices {
                match bridge.set_device(lane, *slot, Some(plugin)) {
                    ApplyOutcome::Applied => r.devices += 1,
                    _ => r.manual.push(format!("{lane}: put {} in slot {}", plugin.name, slot.0 + 1)),
                }
            }
            for (k, v) in &ls.settings {
                let outcome = if caps.set_params && bridge.has_plugin(&v.plugin) {
                    bridge.set_param(lane, k.slot, &v.plugin, &k.param, v.value)
                } else {
                    ApplyOutcome::NotAvailable
                };
                match outcome {
                    ApplyOutcome::Applied => r.settings += 1,
                    _ => heard = true,
                }
            }
            if caps.set_mixer {
                let changes = [
                    ls.mixer_set.volume.then_some(MixerChange::Volume(ls.mixer.volume)),
                    ls.mixer_set.pan.then_some(MixerChange::Pan(ls.mixer.pan)),
                    ls.mixer_set.mute.then_some(MixerChange::Mute(ls.mixer.mute)),
                ];
                for c in changes.into_iter().flatten() {
                    if bridge.set_mixer(lane, &c) == ApplyOutcome::Applied {
                        r.mixer += 1;
                    }
                }
            }
            if caps.host_state {
                if let Some(blob) = ls.host_state.get(&bridge.tag()) {
                    let _ = bridge.apply_host_state(lane, blob);
                }
            }
            if heard {
                r.heard.push(lane.clone());
            }
        }
        if caps.set_transport {
            r.tempo = bridge.set_transport(&TransportChange::Tempo(state.transport.tempo)) == ApplyOutcome::Applied;
        }
        r
    }
}

/// Where a lane's audio should be captured for a plugin-only (level 0)
/// host: at the print point, by the plugin on that lane.
pub fn plugin_only_print_point() -> PrintPoint {
    PrintPoint::PostFaderPreSend
}

/// The host tag for a plugin-only session.
pub fn plugin_only_tag() -> HostTag {
    HostTag::plugin_only()
}

/// Which plugins can share settings: same identity on both sides.
pub fn settings_shareable(a: &PluginIdentity, b: &PluginIdentity) -> bool {
    a.compatible_with(b)
}
