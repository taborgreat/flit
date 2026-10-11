//! The live flow, the same for every host:
//!   bridge sees a change -> settle -> one act -> peers.
//!   peer act -> decide (apply setting / hear audio) -> bridge.
//!   save -> Version + render changed tracks -> chunk, hash, Audio act.
//!   focus rests ~500 ms -> one Presence act.

use crate::bridge::{diff, ApplyOutcome, ApplyReport, Baseline, HostBridge, HostEvent, HostModel, Level, RenderedAudio, TrackKind, SNAPSHOT_THRESHOLD};
use crate::settle::{ControlKey, Settled, Settler};
use flit_core::chunk::ChunkPlan;
use flit_core::{Act, ActId, AudioPayload, BlobStore, Hash, HistoryId, HostTag, ManifestEntry, MixerChange, Note, Payload, PluginIdentity, PresencePayload, PrintPoint, Song, SongPos, SongState, Stamper, TrackId, TransportChange};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Presence is stamped only after focus rested this long (local timer).
pub const PRESENCE_SETTLE_MS: u64 = 500;

/// Turns rendered PCM into the two chunk sets of an Audio act. The real
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

/// One member's live session: their bridge, their being, their history.
pub struct LiveLoop<B: HostBridge, E: ChunkEncoder> {
    pub bridge: B,
    pub encoder: E,
    settler: Settler,
    who: Stamper,
    history: HistoryId,
    /// Tracks with changes since this host's last Version in this history.
    dirty: BTreeSet<TrackId>,
    /// Audio the plugin captured itself (level 0, or when the bridge can't render).
    captured: HashMap<TrackId, RenderedAudio>,
    pub tempo_hint: f64,
    /// Name for the next Version the host reports (the user pressed Save
    /// version and the host was asked to save).
    pub next_version_name: Option<String>,
    /// Notices from the bridge, for the panel (drained by the agent).
    pub notices: Vec<String>,
    /// Track names and kinds the host reported (display only).
    track_names: BTreeMap<TrackId, (String, TrackKind)>,
    /// The unsettled presence signal: (presence, pos, since).
    presence_pending: Option<(PresencePayload, Option<SongPos>, u64)>,
    /// The last presence stamped (so resting on the same spot stamps nothing).
    last_presence: Option<PresencePayload>,
    presence_settle_ms: u64,
}

impl<B: HostBridge, E: ChunkEncoder> LiveLoop<B, E> {
    pub fn new(bridge: B, encoder: E, who: Stamper, history: HistoryId) -> Self {
        Self {
            bridge,
            encoder,
            settler: Settler::default(),
            who,
            history,
            dirty: BTreeSet::new(),
            captured: HashMap::new(),
            tempo_hint: 120.0,
            next_version_name: None,
            notices: Vec::new(),
            track_names: BTreeMap::new(),
            presence_pending: None,
            last_presence: None,
            presence_settle_ms: PRESENCE_SETTLE_MS,
        }
    }

    pub fn with_settle_ms(mut self, ms: u64) -> Self {
        self.settler = Settler::new(ms);
        self
    }

    pub fn with_presence_settle_ms(mut self, ms: u64) -> Self {
        self.presence_settle_ms = ms;
        self
    }

    pub fn history(&self) -> HistoryId {
        self.history
    }

    /// Switch history (a being is in exactly one history at a time).
    pub fn set_history(&mut self, history: HistoryId) {
        self.history = history;
        self.settler = Settler::default();
        self.dirty.clear();
        self.last_presence = None;
    }

    pub fn stamper(&self) -> &Stamper {
        &self.who
    }

    /// Act as another being of the same Name (or another Name).
    pub fn set_stamper(&mut self, who: Stamper) {
        self.who = who;
        self.last_presence = None;
    }

    pub fn id(&self) -> flit_core::Id {
        self.who.name
    }

    pub fn host(&self) -> HostTag {
        self.bridge.tag()
    }

    pub fn track_names(&self) -> &BTreeMap<TrackId, (String, TrackKind)> {
        &self.track_names
    }

    pub fn track_name(&self, track: &TrackId) -> Option<&str> {
        self.track_names.get(track).map(|(n, _)| n.as_str())
    }

    /// Prime the settler from the song so untouched knobs stay silent.
    pub fn prime_from(&mut self, song: &Song) {
        if let Some(st) = song.state(&self.history) {
            for (track, ts) in &st.tracks {
                for (slot, dev) in &ts.devices {
                    for (param, v) in &dev.settings {
                        self.settler.prime(ControlKey::Param { track: track.clone(), slot: *slot, param: param.clone() }, v.value as f64);
                    }
                }
                self.settler.prime(ControlKey::Volume(track.clone()), ts.mixer.volume as f64);
                self.settler.prime(ControlKey::Pan(track.clone()), ts.mixer.pan as f64);
                self.settler.prime(ControlKey::Mute(track.clone()), if ts.mixer.mute { 1.0 } else { 0.0 });
            }
            self.settler.prime(ControlKey::Tempo, st.settings.tempo);
            self.tempo_hint = st.settings.tempo;
        }
    }

    /// Level 0 path: the plugin read tempo/transport from its host's
    /// transport info. Treated exactly like a bridge report (tempo settles,
    /// play/stop are instant).
    pub fn report_transport(&mut self, song: &mut Song, change: TransportChange, pos: SongPos, now_ms: u64) -> flit_core::Result<Vec<Act>> {
        match change {
            TransportChange::Tempo(bpm) => {
                self.settler.input(ControlKey::Tempo, bpm, None, pos, now_ms);
                Ok(vec![])
            }
            other => Ok(vec![song.transport(&self.who, self.history, other, now_ms)?]),
        }
    }

    /// Where the user is (from the bridge, or the plugin itself at level
    /// 0). Settles: stamped once focus has rested.
    pub fn report_presence(&mut self, presence: PresencePayload, pos: Option<SongPos>, now_ms: u64) {
        match &mut self.presence_pending {
            Some((p, pp, since)) if *p == presence => {
                *pp = pos;
                let _ = since;
            }
            _ => self.presence_pending = Some((presence, pos, now_ms)),
        }
    }

    /// The unsettled presence (for live avatar movement, never recorded).
    pub fn presence_now(&self) -> Option<&PresencePayload> {
        self.presence_pending.as_ref().map(|(p, _, _)| p).or(self.last_presence.as_ref())
    }

    /// Mark a track as changed since the last version (level 0: the plugin
    /// on it is sending audio, so a save must render it).
    pub fn push_dirty(&mut self, track: TrackId) {
        self.dirty.insert(track);
    }

    /// The plugin on a track captured that track's output (level 0 path).
    pub fn push_captured(&mut self, track: TrackId, audio: RenderedAudio) {
        self.dirty.insert(track.clone());
        self.captured.insert(track, audio);
    }

    /// Poll the bridge, settle, stamp. Returns what was stamped.
    pub fn tick(&mut self, song: &mut Song, now_ms: u64) -> flit_core::Result<Vec<Act>> {
        let mut out = Vec::new();
        for hev in self.bridge.poll() {
            match hev {
                HostEvent::ParamChanged { track, slot, plugin, param, value, pos } => {
                    self.settler.input(ControlKey::Param { track, slot, param }, value as f64, Some(plugin), pos, now_ms);
                }
                HostEvent::DeviceChanged { track, slot, plugin, pos } => {
                    // Not settled: a plugin appears or goes at once. Bridges
                    // may announce every device they see (a new track with a
                    // plugin already in it, the first snapshot); only a real
                    // change against the song is a moment.
                    let _ = pos;
                    let current = song.state(&self.history).and_then(|s| s.track(&track)).and_then(|t| t.plugin(slot).cloned());
                    if current != plugin {
                        self.dirty.insert(track.clone());
                        out.push(song.device(&self.who, self.history, &track, slot, plugin, now_ms)?);
                    }
                }
                HostEvent::MixerChanged { track, change, pos } => {
                    let (key, v) = match change {
                        MixerChange::Volume(v) => (ControlKey::Volume(track), v as f64),
                        MixerChange::Pan(p) => (ControlKey::Pan(track), p as f64),
                        MixerChange::Mute(m) => (ControlKey::Mute(track), if m { 1.0 } else { 0.0 }),
                    };
                    self.settler.input(key, v, None, pos, now_ms);
                }
                HostEvent::Transport { change, pos } => match change {
                    TransportChange::Tempo(bpm) => self.settler.input(ControlKey::Tempo, bpm, None, pos, now_ms),
                    other => out.push(song.transport(&self.who, self.history, other, now_ms)?),
                },
                HostEvent::Baseline { baseline, pos } => {
                    out.extend(self.record_baseline(song, baseline, now_ms, pos)?);
                }
                HostEvent::ProjectSaved { path, pos } => {
                    let name = self.next_version_name.take().unwrap_or_else(|| "Save".into());
                    out.extend(self.save_version(song, &name, &path, now_ms, pos)?);
                }
                HostEvent::NotesChanged { path, pos: _ } => {
                    out.extend(self.sync_notes(song, &path, now_ms)?);
                }
                HostEvent::ProjectOpened { path, pos } => {
                    if !self.has_version(song) {
                        let name = path.file_name().map(|n| format!("Opened {}", n.to_string_lossy())).unwrap_or_else(|| "Opened".into());
                        out.extend(self.save_version(song, &name, &path, now_ms, pos)?);
                    }
                }
                HostEvent::TrackSeen { track, name, kind } => {
                    self.track_names.insert(track, (name, kind));
                }
                HostEvent::Presence { presence, pos } => self.report_presence(presence, pos, now_ms),
                HostEvent::Notice(text) => self.notices.push(text),
            }
        }
        for s in self.settler.flush(now_ms) {
            out.push(self.emit_settled(song, s, now_ms)?);
        }
        if let Some((p, pos, since)) = &self.presence_pending {
            if now_ms.saturating_sub(*since) >= self.presence_settle_ms {
                let (p, pos) = (p.clone(), *pos);
                self.presence_pending = None;
                if self.last_presence.as_ref() != Some(&p) {
                    out.push(song.presence(&self.who, self.history, p.clone(), pos, now_ms)?);
                    self.last_presence = Some(p);
                }
            }
        }
        Ok(out)
    }

    /// Does this host already have a Version in the current history?
    pub fn has_version(&self, song: &Song) -> bool {
        song.state(&self.history).map(|s| s.latest_version.contains_key(&self.host())).unwrap_or(false)
    }

    /// A control's value from before its first move. If the song has no
    /// value for it yet, it lands as a normal Setting/Mixer act so a later
    /// Go back can put the control where it was; either way the settler now
    /// knows the start value, so a knob returned there makes no act.
    fn record_baseline(&mut self, song: &mut Song, b: Baseline, now_ms: u64, pos: SongPos) -> flit_core::Result<Vec<Act>> {
        let mut out = Vec::new();
        match b {
            Baseline::Param { track, slot, plugin, param, value } => {
                let known = song.state(&self.history).and_then(|s| s.track(&track)).and_then(|l| l.setting(slot, &param));
                let key = ControlKey::Param { track: track.clone(), slot, param: param.clone() };
                match known {
                    Some(v) => self.settler.prime(key, v as f64),
                    None => {
                        out.push(song.setting(&self.who, self.history, &track, slot, &plugin, &param, value.clamp(0.0, 1.0), Some(pos), now_ms)?);
                        self.settler.prime(key, value as f64);
                    }
                }
            }
            Baseline::Mixer { track, change } => {
                let ts = song.state(&self.history).and_then(|s| s.track(&track).cloned());
                let (key, value, known) = match change {
                    MixerChange::Volume(v) => (ControlKey::Volume(track.clone()), v as f64, ts.as_ref().filter(|l| l.mixer_set.volume).map(|l| l.mixer.volume as f64)),
                    MixerChange::Pan(p) => (ControlKey::Pan(track.clone()), p as f64, ts.as_ref().filter(|l| l.mixer_set.pan).map(|l| l.mixer.pan as f64)),
                    MixerChange::Mute(m) => (ControlKey::Mute(track.clone()), if m { 1.0 } else { 0.0 }, ts.as_ref().filter(|l| l.mixer_set.mute).map(|l| if l.mixer.mute { 1.0 } else { 0.0 })),
                };
                match known {
                    Some(v) => self.settler.prime(key, v),
                    None => {
                        out.push(song.mixer(&self.who, self.history, &track, change, now_ms)?);
                        self.settler.prime(key, value);
                    }
                }
            }
        }
        Ok(out)
    }

    fn emit_settled(&mut self, song: &mut Song, s: Settled, now_ms: u64) -> flit_core::Result<Act> {
        Ok(match s {
            Settled::Param { track, slot, plugin, param, value, pos } => {
                self.dirty.insert(track.clone());
                song.setting(&self.who, self.history, &track, slot, &plugin, &param, value.clamp(0.0, 1.0), Some(pos), now_ms)?
            }
            Settled::Mixer { track, change, pos: _ } => {
                self.dirty.insert(track.clone());
                song.mixer(&self.who, self.history, &track, change, now_ms)?
            }
            Settled::Tempo { bpm, pos: _ } => {
                self.tempo_hint = bpm;
                song.tempo(&self.who, self.history, bpm, now_ms)?
            }
        })
    }

    /// Level 2: read the project file and stamp what its notes changed,
    /// one Notes act per clip (added and removed notes). The same path
    /// serves a quiet save while people edit and the save behind a Version.
    /// Returns the acts and the files the project uses.
    fn notes_from_project(&mut self, song: &mut Song, project_path: &Path, now_ms: u64) -> flit_core::Result<(Vec<Act>, Vec<PathBuf>)> {
        let mut out = Vec::new();
        let mut used_files: Vec<PathBuf> = Vec::new();
        if self.bridge.capabilities().read_project {
            if let Some(snap) = self.bridge.read_project(project_path) {
                used_files = snap.files.clone();
                for (track, clip, notes) in snap.notes {
                    let current = song.state(&self.history).and_then(|s| s.track(&track)).and_then(|l| l.clips.get(&clip).cloned()).unwrap_or_default();
                    let (add, remove) = notes_delta(&current, &notes);
                    if !add.is_empty() || !remove.is_empty() {
                        out.push(song.notes(&self.who, self.history, &track, &clip, add, remove, now_ms)?);
                        self.dirty.insert(track.clone());
                    }
                }
            }
        }
        Ok((out, used_files))
    }

    /// Notes only, no Version: the project file was written so the song can
    /// read the notes (a quiet save or an autosave).
    pub fn sync_notes(&mut self, song: &mut Song, project_path: &Path, now_ms: u64) -> flit_core::Result<Vec<Act>> {
        Ok(self.notes_from_project(song, project_path, now_ms)?.0)
    }

    /// Save version: project file -> blob + Version act, then render only
    /// the tracks changed since this host's last version, chunk, hash,
    /// Audio. At level 2 the project's notes are diffed against the song
    /// first (`notes_from_project`).
    pub fn save_version(&mut self, song: &mut Song, name: &str, project_path: &Path, now_ms: u64, pos: SongPos) -> flit_core::Result<Vec<Act>> {
        let (mut out, used_files) = self.notes_from_project(song, project_path, now_ms)?;

        // Version
        let project = if project_path.is_file() { song.blobs().put_file(project_path)? } else { song.blobs().put(b"")? };
        let mut manifest = Vec::new();
        for f in &used_files {
            if f.is_file() {
                let hash = song.blobs().put_file(f)?;
                manifest.push(ManifestEntry { path: f.to_string_lossy().into_owned(), hash, size: std::fs::metadata(f)?.len() });
            }
        }
        out.push(song.version(&self.who, self.history, name, project, manifest, now_ms)?);

        // Render changed tracks.
        let changed: Vec<TrackId> = self.dirty.iter().cloned().collect();
        let tempo = song.state(&self.history).map(|s| s.settings.tempo).unwrap_or(self.tempo_hint);
        for track in changed {
            let rendered = self.captured.remove(&track).or_else(|| {
                if self.bridge.capabilities().render_track {
                    self.bridge.render_track(&track, SongPos::START, pos)
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
            out.push(song.audio(&self.who, self.history, &track, payload, now_ms)?);
        }
        self.dirty.clear();
        Ok(out)
    }

    pub fn dirty_tracks(&self) -> Vec<TrackId> {
        self.dirty.iter().cloned().collect()
    }
}

/// Notes to add and to remove to turn `current` into `wanted`. A note is
/// the same note by (start, key, channel); a changed velocity or length is
/// an add (it replaces).
pub fn notes_delta(current: &[Note], wanted: &[Note]) -> (Vec<Note>, Vec<Note>) {
    let same = |a: &Note, b: &Note| a.start == b.start && a.key == b.key && a.channel == b.channel;
    let add: Vec<Note> = wanted.iter().filter(|w| !current.iter().any(|c| c == *w)).cloned().collect();
    let remove: Vec<Note> = current.iter().filter(|c| !wanted.iter().any(|w| same(c, w))).cloned().collect();
    (add, remove)
}

/// What a receiver does with an act from a peer.
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
    /// Can't reproduce it here: play that track's audio instead.
    HearAsAudio(TrackId),
    /// Nothing for the host to do (chat, crew, versions...).
    Nothing,
}

/// The receiver side of the live flow.
pub struct Receiver;

impl Receiver {
    /// Decide, in order: (a) have the plugin and a bridge that sets params
    /// -> apply; (b) else hear the audio. HostState applies only on the same host.
    pub fn decide<B: HostBridge + ?Sized>(bridge: &B, act: &Act) -> ReceiverDecision {
        let caps = bridge.capabilities();
        let track = act.track().cloned();
        let hear = |t: Option<TrackId>| t.map(ReceiverDecision::HearAsAudio).unwrap_or(ReceiverDecision::Nothing);
        match act.payload() {
            Payload::Setting(s) => {
                if caps.level >= Level::LiveControl && caps.set_params && bridge.has_plugin(&s.plugin) {
                    ReceiverDecision::ApplySetting
                } else {
                    hear(track)
                }
            }
            Payload::Mixer(_) => {
                if caps.level >= Level::LiveControl && caps.set_mixer {
                    ReceiverDecision::ApplyMixer
                } else {
                    hear(track)
                }
            }
            Payload::Device(_) => {
                if caps.level >= Level::LiveControl && caps.set_devices {
                    ReceiverDecision::ApplyDevice
                } else {
                    hear(track)
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
                    hear(track)
                }
            }
            Payload::Notes(_) | Payload::Audio(_) => hear(track),
            Payload::Merge(m) => {
                // A group: apply what can be applied, hear the rest.
                let any_hear = m.acts.iter().any(|e| matches!(Self::decide(bridge, e), ReceiverDecision::HearAsAudio(_)));
                if any_hear {
                    hear(track.or_else(|| m.acts.iter().find_map(|a| a.track().cloned())))
                } else {
                    ReceiverDecision::ApplySetting
                }
            }
            _ => ReceiverDecision::Nothing,
        }
    }

    /// Apply an act that is IN THE CHAINS (so it passed validation) to the
    /// host. Returns the decision taken.
    pub fn apply<B: HostBridge + ?Sized>(bridge: &mut B, song: &Song, id: &ActId) -> Option<ReceiverDecision> {
        let act = song.chains().get(id)?.clone();
        let decision = Self::decide(bridge, &act);
        let outcome = Self::apply_act(bridge, &act, &decision);
        if outcome == ApplyOutcome::NotAvailable {
            return act.track().cloned().map(ReceiverDecision::HearAsAudio);
        }
        Some(decision)
    }

    fn apply_act<B: HostBridge + ?Sized>(bridge: &mut B, act: &Act, decision: &ReceiverDecision) -> ApplyOutcome {
        let track = act.track().cloned();
        match (decision, act.payload()) {
            (ReceiverDecision::ApplySetting, Payload::Setting(s)) => match (track, act.thing().device_slot()) {
                (Some(t), Some(slot)) => bridge.set_param(&t, slot, &s.plugin, &s.param, s.value),
                _ => ApplyOutcome::NotAvailable,
            },
            (ReceiverDecision::ApplySetting, Payload::Merge(m)) => {
                let mut res = ApplyOutcome::Applied;
                for inner in &m.acts {
                    let d = Self::decide(bridge, inner);
                    let r = Self::apply_act(bridge, inner, &d);
                    if r != ApplyOutcome::Applied {
                        res = r;
                    }
                }
                res
            }
            (ReceiverDecision::ApplyMixer, Payload::Mixer(m)) => track.map(|t| bridge.set_mixer(&t, m)).unwrap_or(ApplyOutcome::NotAvailable),
            (ReceiverDecision::ApplyDevice, Payload::Device(d)) => track.map(|t| bridge.set_device(&t, d.slot, d.plugin.as_ref())).unwrap_or(ApplyOutcome::NotAvailable),
            (ReceiverDecision::ApplyTransport, Payload::Transport(t)) => bridge.set_transport(t),
            (ReceiverDecision::ApplyHostState, Payload::HostState(h)) => track.map(|t| bridge.apply_host_state(&t, &h.blob)).unwrap_or(ApplyOutcome::NotAvailable),
            _ => ApplyOutcome::Applied,
        }
    }

    /// Push the whole sound of a moment into the host (Show in DAW, Go
    /// back, New history, switching history, Merge confirm): a DIFF against
    /// what the DAW has now, never a rebuild. Devices present in both are
    /// never reloaded. A big jump takes the snapshot path when the bridge
    /// can load `snapshot` (the nearest Version's project file) and the
    /// diff is costlier than loading it. Level 0 hosts: nothing touched;
    /// tracks come back in `heard`.
    pub fn apply_state<B: HostBridge + ?Sized>(bridge: &mut B, state: &SongState, snapshot: Option<&Path>) -> ApplyReport {
        let caps = bridge.capabilities();
        if caps.level < Level::LiveControl {
            let mut r = ApplyReport::default();
            r.heard = state.tracks.iter().filter(|(_, l)| l.devices.values().any(|d| !d.settings.is_empty()) || !l.audio.is_empty()).map(|(l, _)| l.clone()).collect();
            return r;
        }
        let host = bridge.tag();
        let model = bridge.read_state().unwrap_or_default();
        let mut d = diff(state, &model, &host);
        let mut report = ApplyReport::default();
        if let Some(path) = snapshot {
            if caps.load_snapshot && bridge.estimate_cost(&d) > SNAPSHOT_THRESHOLD && bridge.load_snapshot(path) == ApplyOutcome::Applied {
                report.snapshot_loaded = true;
                let model: HostModel = bridge.read_state().unwrap_or_default();
                d = diff(state, &model, &host);
            }
        }
        let r = bridge.apply_diff(&d);
        report.settings = r.settings;
        report.mixer = r.mixer;
        report.devices = r.devices;
        report.tempo = r.tempo;
        report.heard = r.heard;
        report.manual = r.manual;
        report
    }
}

/// Where a track's audio should be captured for a plugin-only (level 0)
/// host: at the print point, by the plugin on that track.
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
