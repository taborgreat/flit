//! `FlBridge`: the `HostBridge` for FL Studio, speaking the SysEx protocol
//! to `device_FLit.py` through a `MidiPort`. Level 1 (live control) is
//! verified; level 2 (project read) goes through the PyFLP helper when the
//! project path is known. Presence: the script reports the focused window
//! / selected track and channel; the bridge tints tracks in visitors'
//! colors (restored afterwards) and can select tracks / open windows to
//! follow someone.

use crate::port::MidiPort;
use crate::sysex::*;
use flit_core::{DeviceSlot, HostTag, MixerChange, ParamId, PluginFormat, PluginIdentity, PresencePayload, SongPos, TrackId, TransportChange, View, TICKS_PER_BEAT};
use flit_host::{ApplyOutcome, Baseline, Capabilities, HostBridge, HostDevice, HostEvent, HostModel, HostTrack, Level, ProjectSnapshot, RenderedAudio, Shown, TrackKind};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Mixer-track unity gain in FL's normalized scale. VERIFY ON WINDOWS
/// (flstudio-mcp: 0.8 = 0 dB on FL 25.2.5).
pub const FL_UNITY_VOLUME: f32 = 0.8;
/// Channel (channel rack) default volume = 1000/1280 (API docs).
pub const FL_CHANNEL_UNITY: f32 = 0.78125;
/// How many params to ask for per page (keep each reply under one chunk
/// burst; the script stalls FL if a single tick does too much work).
pub const PARAM_PAGE: u32 = 48;
/// Highest param index worth scanning on a VST wrapper (4096 real + 128 CC
/// + 16 aftertouch are reported; real ones sit at low indices).
pub const MAX_SCAN_PARAM: u32 = 1024; // VERIFY ON WINDOWS: named params beyond 1024 on big synths (Serum 2 had 541)

#[derive(Clone, Debug)]
pub struct FlConfig {
    /// Base name of the virtual MIDI port FL talks over (`port.rs`).
    pub port: String,
    /// Where the .flp files live, to find the project file from the window
    /// title (FL's API does not expose the path). VERIFY ON WINDOWS.
    pub projects_dir: Option<PathBuf>,
    /// `python` executable with pyflp for level-2 reads (see flp.rs).
    pub python: Option<PathBuf>,
    pub request_timeout: Duration,
    /// Notes and clips can only be read from the project file (FL's API has
    /// no note read). With this on, the bridge has FL save quietly once the
    /// user pauses after piano-roll / playlist edits or a recording pass, so
    /// the notes reach the song while people work. Off = notes land on
    /// Save version and the user's own saves only.
    pub quiet_save: bool,
    /// How long notes edits must rest before the quiet save.
    pub notes_settle: Duration,
}

impl Default for FlConfig {
    fn default() -> Self {
        Self {
            port: crate::port::DEFAULT_PORT.into(),
            projects_dir: dirs_projects(),
            python: None,
            request_timeout: Duration::from_millis(1500),
            quiet_save: true,
            notes_settle: Duration::from_secs(3),
        }
    }
}

fn dirs_projects() -> Option<PathBuf> {
    // Default FL projects folder on Windows. VERIFY ON WINDOWS.
    std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Documents").join("Image-Line").join("FL Studio").join("Projects"))
}

/// What we last saw of one plugin slot.
#[derive(Clone, Debug, Default)]
struct SlotState {
    plugin: String,
    params: BTreeMap<u32, (String, f32)>,
    /// Pagination cursor while a scan is in progress.
    scanning_from: Option<u32>,
    scanned_once: bool,
    /// Params whose change we have reported at least once (their first
    /// change is preceded by a Baseline with the value before it).
    reported: BTreeSet<u32>,
    /// We bypassed this slot to stand in for "plugin removed" (FL cannot
    /// delete plugins from outside).
    bypassed: bool,
}

#[derive(Clone, Debug, Default)]
struct TrackState {
    name: String,
    volume: f32,
    pan: f32,
    mute: bool,
    color: u32,
    slots: BTreeMap<i32, SlotState>,
    /// Reported to the core as a track (only tracks in use are; an unused
    /// insert is announced the moment something happens on it).
    announced: bool,
    /// Which mixer controls have had a change reported (see `reported`).
    vol_reported: bool,
    pan_reported: bool,
    mute_reported: bool,
}

/// A channel-rack channel as a track: its generator is device slot 0, its
/// own volume/pan/mute are the track's mixer values.
#[derive(Clone, Debug, Default)]
struct ChanState {
    name: String,
    plugin: Option<String>,
    insert: i32,
    volume: f32,
    pan: f32,
    mute: bool,
    color: u32,
    slot: SlotState,
    announced: bool,
    vol_reported: bool,
    pan_reported: bool,
    mute_reported: bool,
}

#[derive(Debug)]
struct Pending {
    req: FlRequest,
    sent: Instant,
}

pub struct FlBridge {
    port: Box<dyn MidiPort>,
    cfg: FlConfig,
    chunker: Chunker,
    next_id: u16,
    pending: HashMap<u16, Pending>,
    queue: VecDeque<FlRequest>,
    out: Vec<HostEvent>,
    tracks: BTreeMap<i32, TrackState>,
    channels: BTreeMap<i32, ChanState>,
    tempo: f64,
    playing: bool,
    pos_ticks: i64,
    ppq: i32,
    /// Script said hello (and its versions).
    pub script: Option<(i32, String, String)>,
    last_heartbeat: Option<Instant>,
    dirty_tracks: Vec<i32>,
    dirty_channels: Vec<i32>,
    dirty_all: bool,
    title: String,
    /// FL's "changed since save" flag from the last heartbeat (0 = saved).
    last_changed: Option<i32>,
    /// FL's undo history (count, position) from the last heartbeat.
    last_undo: Option<(i32, i32)>,
    /// `project_path()` cache: (when, title it was computed for, path).
    project_cache: Option<(Instant, String, Option<PathBuf>)>,
    /// When we last said hello without an answer (retried every 2 s until
    /// the script is up, so a late FL setup still connects).
    last_hello: Option<Instant>,
    /// Filled by applying peer settings so our own re-read does not echo them.
    suppress: HashMap<(i32, i32, u32), f32>,
    /// Presence tints we put on tracks / channels: (original color).
    tinted_tracks: HashMap<i32, u32>,
    tinted_channels: HashMap<i32, u32>,
    /// The last presence reported, to report only changes.
    last_presence: Option<PresencePayload>,
    /// The focused FL window from the last Focus event ("piano_roll", ...).
    focus: String,
    /// FL's record button, from the heartbeat.
    recording: bool,
    /// Notes or clips were edited (undo step with the piano roll / playlist
    /// focused, or a recording pass ended) and not yet saved: when.
    notes_dirty: Option<Instant>,
    /// A quiet save is in flight: the next "changed flag back to 0" is
    /// NotesChanged, not a Version.
    quiet_save_sent: bool,
    quiet_save_noticed: bool,
    /// Save version asked FL to save (when): that save is a Version even if
    /// notes edits arrive meanwhile. Forgotten after a minute (Save As
    /// cancelled, untitled project).
    version_save_at: Option<Instant>,
}

impl FlBridge {
    pub fn new(port: Box<dyn MidiPort>) -> Self {
        Self::with_config(port, FlConfig::default())
    }

    pub fn with_config(port: Box<dyn MidiPort>, cfg: FlConfig) -> Self {
        let mut b = Self {
            port,
            cfg,
            chunker: Chunker::default(),
            next_id: 1,
            pending: HashMap::new(),
            queue: VecDeque::new(),
            out: Vec::new(),
            tracks: BTreeMap::new(),
            channels: BTreeMap::new(),
            tempo: 120.0,
            playing: false,
            pos_ticks: 0,
            ppq: 96,
            script: None,
            last_heartbeat: None,
            dirty_tracks: Vec::new(),
            dirty_channels: Vec::new(),
            dirty_all: false,
            title: String::new(),
            last_changed: None,
            last_undo: None,
            project_cache: None,
            last_hello: None,
            suppress: HashMap::new(),
            tinted_tracks: HashMap::new(),
            tinted_channels: HashMap::new(),
            last_presence: None,
            focus: String::new(),
            recording: false,
            notes_dirty: None,
            quiet_save_sent: false,
            quiet_save_noticed: false,
            version_save_at: None,
        };
        b.queue.push_back(FlRequest::Hello { protocol: PROTOCOL_VERSION });
        b.queue.push_back(FlRequest::Snapshot);
        b
    }

    /// Open the virtual MIDI port (`port.rs`) and the bridge on it. Fails
    /// when the port is not there, and off Windows, so the app can fall
    /// back to plugin-only mode.
    pub fn open_default() -> std::io::Result<Self> {
        Self::open_with_config(FlConfig::default())
    }

    pub fn open_with_config(cfg: FlConfig) -> std::io::Result<Self> {
        let wire = crate::port::open_platform(&cfg.port).map_err(std::io::Error::other)?;
        Ok(Self::with_config(Box::new(wire), cfg))
    }

    pub fn is_connected(&self) -> bool {
        self.script.is_some() && self.last_heartbeat.map(|t| t.elapsed() < Duration::from_secs(3)).unwrap_or(false)
    }

    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    /// Track id for an FL mixer track (0 = Master). Stable across sessions
    /// as long as the track index is (FL has no track UUIDs). VERIFY ON
    /// WINDOWS: FL moves tracks? Then names should drive the mapping instead.
    pub fn track_for_index(track: i32) -> TrackId {
        TrackId::new(&format!("fl-track-{track}"))
    }

    pub fn index_for_track(track: &TrackId) -> Option<i32> {
        track.0.strip_prefix("fl-track-").and_then(|n| n.parse().ok())
    }

    /// Track id for a channel-rack channel (global index). Same stability
    /// caveat as mixer tracks: FL has no channel ids.
    pub fn track_for_channel(chan: i32) -> TrackId {
        TrackId::new(&format!("fl-channel-{chan}"))
    }

    pub fn channel_for_track(track: &TrackId) -> Option<i32> {
        track.0.strip_prefix("fl-channel-").and_then(|n| n.parse().ok())
    }

    fn plugin_identity(name: &str) -> PluginIdentity {
        // FL's API gives only the plugin's name, not vendor/version/format.
        // Settings therefore share only between FL users until VERIFY ON
        // WINDOWS: whether FL exposes the real VST3/CLAP param ids and the
        // plugin's vendor + version (Verify list item 3).
        PluginIdentity::new(PluginFormat::Other("fl-wrapper".into()), "", name, "fl")
    }

    fn pos(&self) -> SongPos {
        // FL ticks are PPQ-based (default 96 per quarter); ours are 960.
        SongPos::from_ticks(self.pos_ticks * TICKS_PER_BEAT / self.ppq.max(1) as i64)
    }

    fn send_request(&mut self, req: FlRequest) {
        let id = self.next_id;
        self.next_id = if self.next_id >= 0x3FFF { 1 } else { self.next_id + 1 };
        let frame = request_frame(id, &req);
        for m in encode(&frame) {
            if !self.port.send(&m) {
                return;
            }
        }
        self.pending.insert(id, Pending { req, sent: Instant::now() });
    }

    /// Only a few requests in flight at once: the script runs everything on
    /// FL's UI thread.
    fn pump_queue(&mut self) {
        // Not connected and nothing waiting: knock again every 2 s. The
        // script may be assigned in FL long after the agent started.
        if !self.is_connected() && self.queue.is_empty() && self.pending.is_empty() && self.last_hello.map(|t| t.elapsed() > Duration::from_secs(2)).unwrap_or(true) {
            self.last_hello = Some(Instant::now());
            self.queue.push_back(FlRequest::Hello { protocol: PROTOCOL_VERSION });
        }
        while self.pending.len() < 2 {
            let Some(req) = self.queue.pop_front() else { break };
            self.send_request(req);
        }
        let timeout = self.cfg.request_timeout;
        let dead: Vec<u16> = self.pending.iter().filter(|(_, p)| p.sent.elapsed() > timeout).map(|(id, _)| *id).collect();
        for id in dead {
            self.pending.remove(&id);
        }
    }

    /// Notes edits rested long enough and the transport is stopped: have FL
    /// save, so the file (the only place FL shows its notes) is current.
    /// The heartbeat's changed flag then turns into NotesChanged.
    fn maybe_quiet_save(&mut self) {
        let Some(since) = self.notes_dirty else { return };
        if !self.cfg.quiet_save || self.version_save_pending() {
            // off, or a Save version is on its way and covers the notes
            self.notes_dirty = None;
            return;
        }
        if since.elapsed() < self.cfg.notes_settle || self.playing || !self.is_connected() || self.quiet_save_sent {
            return;
        }
        if self.queue.iter().any(|r| matches!(r, FlRequest::Save)) || self.pending.values().any(|p| matches!(p.req, FlRequest::Save)) {
            return;
        }
        if self.project_path().is_none() {
            // untitled project: FL would open Save As; wait for the user
            return;
        }
        self.notes_dirty = None;
        self.restore_colors();
        self.queue.push_back(FlRequest::Save);
        self.quiet_save_sent = true;
        if !self.quiet_save_noticed {
            self.quiet_save_noticed = true;
            self.out.push(HostEvent::Notice("FLit saves your project a few seconds after you edit notes, so they reach the song. Save version still makes a version.".into()));
        }
    }

    fn version_save_pending(&self) -> bool {
        self.version_save_at.map(|t| t.elapsed() < Duration::from_secs(60)).unwrap_or(false)
    }

    fn presence_from(&self, focus: &str, track: i32, slot: i32, channel: Option<i32>) -> PresencePayload {
        let mixer_track = if track >= 0 { Some(Self::track_for_index(track)) } else { None };
        let chan_track = channel.filter(|c| *c >= 0).map(Self::track_for_channel);
        match focus {
            "plugin" => PresencePayload { view: View::Device, track: mixer_track.or(chan_track), device: if slot >= 0 { Some(DeviceSlot(slot as u32)) } else { None }, region: None },
            "mixer" => PresencePayload { view: View::Mixer, track: mixer_track, device: None, region: None },
            "piano_roll" => PresencePayload { view: View::Notes, track: chan_track, device: None, region: None },
            "playlist" => PresencePayload { view: View::Arrange, track: None, device: None, region: None },
            "channel_rack" => PresencePayload { view: View::Other, track: chan_track, device: None, region: Some("channel rack".into()) },
            "browser" => PresencePayload { view: View::Browser, track: None, device: None, region: None },
            _ => PresencePayload { view: View::Other, track: mixer_track.or(chan_track), device: None, region: None },
        }
    }

    fn handle_focus(&mut self, focus: String, track: i32, slot: i32, channel: Option<i32>) {
        let p = self.presence_from(&focus, track, slot, channel);
        self.focus = focus;
        if self.last_presence.as_ref() != Some(&p) {
            self.last_presence = Some(p.clone());
            self.out.push(HostEvent::Presence { presence: p, pos: Some(self.pos()) });
        }
    }

    fn handle_event(&mut self, ev: FlEvent) {
        match ev {
            FlEvent::Hello { api, fl, python } => {
                self.script = Some((api, fl, python));
                self.last_heartbeat = Some(Instant::now());
                self.queue.push_back(FlRequest::Snapshot);
            }
            FlEvent::Heartbeat { playing, tempo, pos_ticks, changed, title, undo_count, undo_pos, recording } => {
                // A new undo step in FL = an edit happened somewhere. Treat it
                // as a dirty hint so a re-read catches edits the flags missed.
                // With the piano roll or playlist focused it is a notes / clip
                // edit, which only the project file can tell us about.
                if let (Some(c), Some(p)) = (undo_count, undo_pos) {
                    if c >= 0 {
                        let now = (c, p);
                        if let Some(prev) = self.last_undo {
                            if prev != now {
                                tracing::debug!("FL undo history {prev:?} -> {now:?} (focus {})", self.focus);
                                self.dirty_all = true;
                                self.queue.push_back(FlRequest::Snapshot);
                                if matches!(self.focus.as_str(), "piano_roll" | "playlist") {
                                    self.notes_dirty = Some(Instant::now());
                                }
                            }
                        }
                        self.last_undo = Some(now);
                    }
                }
                self.last_heartbeat = Some(Instant::now());
                self.pos_ticks = pos_ticks;
                if (tempo - self.tempo).abs() > 1e-6 && tempo > 0.0 {
                    self.tempo = tempo;
                    self.out.push(HostEvent::Transport { change: TransportChange::Tempo(tempo), pos: self.pos() });
                }
                if playing != self.playing {
                    self.playing = playing;
                    self.out.push(HostEvent::Transport { change: if playing { TransportChange::Play } else { TransportChange::Stop }, pos: self.pos() });
                    // Stop with the record button on = a take just ended; its
                    // notes are in the project, not in the API.
                    if !playing && self.recording {
                        self.notes_dirty = Some(Instant::now());
                    }
                }
                if let Some(r) = recording {
                    self.recording = r;
                }
                // A new title = another project (open / new / save as).
                if let Some(t) = title {
                    if t != self.title {
                        // Only the file name counts: FL adds "*" while unsaved.
                        let same_file = match (title_to_filename(&t), title_to_filename(&self.title)) {
                            (Some(a), Some(b)) => a.eq_ignore_ascii_case(&b),
                            (None, None) => true,
                            _ => false,
                        };
                        let first = self.title.is_empty();
                        self.title = t;
                        if !first && !same_file {
                            self.project_switched();
                        }
                    }
                }
                // FL's changed flag dropping to 0 = the user saved. The
                // scripting API has no save callback, so this (plus a fresh
                // mtime on the file) is how saves become Versions.
                if let Some(c) = changed {
                    if matches!(self.last_changed, Some(prev) if prev != 0) && c == 0 {
                        // an asked-for save is a Version; only a save we started
                        // for the notes alone is NotesChanged
                        let quiet = std::mem::take(&mut self.quiet_save_sent) && !self.version_save_pending();
                        self.version_save_at = None;
                        if let Some(path) = self.project_path() {
                            if recently_modified(&path) {
                                let pos = self.pos();
                                self.out.push(if quiet { HostEvent::NotesChanged { path, pos } } else { HostEvent::ProjectSaved { path, pos } });
                            }
                        }
                    }
                    self.last_changed = Some(c);
                }
            }
            FlEvent::Dirty { flags: _, tracks, channels } => {
                // VERIFY ON WINDOWS: which HW_Dirty_* flags fire for plugin
                // knob moves (ControlValues 4096?) vs mixer faders (4).
                if (tracks.is_empty() && channels.is_empty()) || tracks.contains(&-1) {
                    self.dirty_all = true;
                } else {
                    for t in tracks {
                        if !self.dirty_tracks.contains(&t) {
                            self.dirty_tracks.push(t);
                        }
                    }
                    for c in channels {
                        if !self.dirty_channels.contains(&c) {
                            self.dirty_channels.push(c);
                        }
                    }
                }
                self.queue.push_back(FlRequest::Snapshot);
            }
            FlEvent::ProjectLoaded { title } => {
                if !title.is_empty() {
                    self.title = title;
                }
                self.project_switched();
            }
            FlEvent::Focus { focus, track, slot, channel } => self.handle_focus(focus, track, slot, channel),
            FlEvent::Log { text } => tracing::info!("FL script: {text}"),
        }
    }

    fn handle_reply(&mut self, req: FlRequest, reply: FlReply) {
        match (req, reply) {
            (FlRequest::Params { track, slot, .. }, FlReply::Error { error, .. }) => {
                // Verified on FL 24.1.1: "RuntimeError: Plugin not valid" when the
                // plugin left the slot mid-scan. The next snapshot reports the slot
                // change; this scan is over.
                tracing::debug!("FL params scan on track {track} slot {slot} ended: {error}");
                if let Some(ss) = self.tracks.get_mut(&track).and_then(|t| t.slots.get_mut(&slot)) {
                    ss.scanning_from = None;
                    ss.scanned_once = true;
                }
            }
            (_, FlReply::Error { error, .. }) => tracing::warn!("FL script error: {error}"),
            (FlRequest::Hello { .. }, FlReply::Hello { api, fl, python, ppq, .. }) => {
                self.script = Some((api, fl, python));
                self.ppq = ppq.max(1);
                self.last_heartbeat = Some(Instant::now());
            }
            (FlRequest::Snapshot, FlReply::Snapshot { tempo, playing, pos_ticks, ppq, tracks, channels, .. }) => {
                self.ppq = ppq.max(1);
                self.pos_ticks = pos_ticks;
                if (tempo - self.tempo).abs() > 1e-6 && tempo > 0.0 {
                    self.tempo = tempo;
                    self.out.push(HostEvent::Transport { change: TransportChange::Tempo(tempo), pos: self.pos() });
                }
                if playing != self.playing {
                    self.playing = playing;
                    self.out.push(HostEvent::Transport { change: if playing { TransportChange::Play } else { TransportChange::Stop }, pos: self.pos() });
                }
                let dirty_all = std::mem::take(&mut self.dirty_all);
                let dirty = std::mem::take(&mut self.dirty_tracks);
                let dirty_chans = std::mem::take(&mut self.dirty_channels);
                let pos = self.pos();
                self.fold_channels(channels, dirty_all, &dirty_chans, pos);
                for t in tracks {
                    let track = Self::track_for_index(t.index);
                    let kind = if t.index == 0 { TrackKind::Master } else { TrackKind::Part };
                    let is_new = !self.tracks.contains_key(&t.index);
                    let tinted = self.tinted_tracks.contains_key(&t.index);
                    let st = self.tracks.entry(t.index).or_default();
                    if is_new {
                        // Verified on FL 24.1.1: trackCount() reports 127 (Master,
                        // 125 inserts, one utility track); only tracks in use are
                        // announced here; the rest when something first happens.
                        st.volume = t.volume;
                        st.pan = t.pan;
                        st.mute = t.mute;
                        st.name = t.name.clone();
                        st.color = t.color;
                        if track_in_use(&t) {
                            st.announced = true;
                            let name = if t.name.is_empty() { format!("Insert {}", t.index) } else { t.name.clone() };
                            self.out.push(HostEvent::TrackSeen { track: track.clone(), name, kind });
                            // same for the plugins already in its slots
                            for (s, plugin) in &t.slots {
                                self.out.push(HostEvent::DeviceChanged { track: track.clone(), slot: DeviceSlot((*s).max(0) as u32), plugin: Some(Self::plugin_identity(plugin)), pos });
                            }
                        }
                    } else {
                        let changed = (t.volume - st.volume).abs() > 1e-4 || (t.pan - st.pan).abs() > 1e-4 || t.mute != st.mute || t.slots.iter().any(|(s, p)| st.slots.get(s).map(|x| x.plugin != *p).unwrap_or(true)) || st.slots.keys().any(|s| !t.slots.iter().any(|(x, _)| x == s));
                        if changed && !st.announced {
                            st.announced = true;
                            let name = if t.name.is_empty() { format!("Insert {}", t.index) } else { t.name.clone() };
                            self.out.push(HostEvent::TrackSeen { track: track.clone(), name, kind });
                        }
                        // plugins added, replaced or removed since the last snapshot
                        let gone: Vec<i32> = st.slots.keys().filter(|s| !t.slots.iter().any(|(x, _)| x == *s)).copied().collect();
                        for s in gone {
                            st.slots.remove(&s);
                            self.out.push(HostEvent::DeviceChanged { track: track.clone(), slot: DeviceSlot(s.max(0) as u32), plugin: None, pos });
                        }
                        for (s, plugin) in &t.slots {
                            let was = st.slots.get(s).map(|x| x.plugin.clone());
                            if was.as_deref() != Some(plugin.as_str()) {
                                self.out.push(HostEvent::DeviceChanged { track: track.clone(), slot: DeviceSlot((*s).max(0) as u32), plugin: Some(Self::plugin_identity(plugin)), pos });
                            }
                        }
                        if (t.volume - st.volume).abs() > 1e-4 {
                            if !st.vol_reported {
                                st.vol_reported = true;
                                self.out.push(HostEvent::Baseline { baseline: Baseline::Mixer { track: track.clone(), change: MixerChange::Volume(fl_to_gain(st.volume)) }, pos });
                            }
                            st.volume = t.volume;
                            self.out.push(HostEvent::MixerChanged { track: track.clone(), change: MixerChange::Volume(fl_to_gain(t.volume)), pos });
                        }
                        if (t.pan - st.pan).abs() > 1e-4 {
                            if !st.pan_reported {
                                st.pan_reported = true;
                                self.out.push(HostEvent::Baseline { baseline: Baseline::Mixer { track: track.clone(), change: MixerChange::Pan(st.pan) }, pos });
                            }
                            st.pan = t.pan;
                            self.out.push(HostEvent::MixerChanged { track: track.clone(), change: MixerChange::Pan(t.pan), pos });
                        }
                        if t.mute != st.mute {
                            if !st.mute_reported {
                                st.mute_reported = true;
                                self.out.push(HostEvent::Baseline { baseline: Baseline::Mixer { track: track.clone(), change: MixerChange::Mute(st.mute) }, pos });
                            }
                            st.mute = t.mute;
                            self.out.push(HostEvent::MixerChanged { track: track.clone(), change: MixerChange::Mute(t.mute), pos });
                        }
                        // a tinted track keeps its original color on record
                        if !tinted {
                            st.color = t.color;
                        }
                    }
                    st.name = t.name.clone();
                    let want_scan = dirty_all || dirty.contains(&t.index) || is_new;
                    for (slot, plugin) in &t.slots {
                        let ss = st.slots.entry(*slot).or_default();
                        if ss.plugin != *plugin {
                            ss.plugin = plugin.clone();
                            ss.params.clear();
                            ss.scanned_once = false;
                        }
                        if (want_scan || !ss.scanned_once) && ss.scanning_from.is_none() {
                            ss.scanning_from = Some(0);
                            self.queue.push_back(FlRequest::Params { track: t.index, slot: *slot, from: 0, count: PARAM_PAGE });
                        }
                    }
                    st.slots.retain(|s, _| t.slots.iter().any(|(x, _)| x == s));
                }
            }
            (FlRequest::Params { .. }, FlReply::Params { track, slot, total, next, params, .. }) => {
                let pos = self.pos();
                // slot -1 = the generator of channel `track`
                let (ss, plugin, tid) = if slot == -1 {
                    let Some(cs) = self.channels.get_mut(&track) else { return };
                    let name = cs.plugin.clone().unwrap_or_default();
                    (&mut cs.slot, Self::plugin_identity(&name), Self::track_for_channel(track))
                } else {
                    let Some(ts) = self.tracks.get_mut(&track) else { return };
                    let Some(ss) = ts.slots.get_mut(&slot) else { return };
                    let plugin = Self::plugin_identity(&ss.plugin);
                    (ss, plugin, Self::track_for_index(track))
                };
                for p in params {
                    let previous = ss.params.get(&p.index).map(|(_, old)| *old);
                    let changed = match previous {
                        Some(old) => (old - p.value).abs() > 1e-5,
                        None => ss.scanned_once, // first scan = baseline, not a change
                    };
                    let echo = self.suppress.remove(&(track, slot, p.index)).map(|v| (v - p.value).abs() < 1e-4).unwrap_or(false);
                    ss.params.insert(p.index, (p.name.clone(), p.value));
                    if changed && !echo {
                        let dslot = DeviceSlot(slot.max(0) as u32);
                        let param = ParamId::from_u32(p.index);
                        if let Some(old) = previous {
                            if ss.reported.insert(p.index) {
                                self.out.push(HostEvent::Baseline { baseline: Baseline::Param { track: tid.clone(), slot: dslot, plugin: plugin.clone(), param: param.clone(), value: old }, pos });
                            }
                        }
                        self.out.push(HostEvent::ParamChanged { track: tid.clone(), slot: dslot, plugin: plugin.clone(), param, value: p.value, pos });
                    }
                }
                let limit = total.min(MAX_SCAN_PARAM);
                if next < limit && next > 0 {
                    ss.scanning_from = Some(next);
                    self.queue.push_back(FlRequest::Params { track, slot, from: next, count: PARAM_PAGE });
                } else {
                    ss.scanning_from = None;
                    ss.scanned_once = true;
                }
            }
            (FlRequest::Title, FlReply::Title { title, .. }) => self.title = title,
            (FlRequest::Focus, FlReply::Focus { focus, track, slot, channel, .. }) => self.handle_focus(focus, track, slot, channel),
            _ => {}
        }
    }

    /// Fold a snapshot's channel list: new channels are tracks, generator
    /// changes are Device acts, channel volume/pan/mute are mixer values.
    fn fold_channels(&mut self, channels: Vec<FlChannel>, dirty_all: bool, dirty: &[i32], pos: SongPos) {
        let seen: Vec<i32> = channels.iter().map(|c| c.index).collect();
        for c in channels {
            let track = Self::track_for_channel(c.index);
            let is_new = !self.channels.contains_key(&c.index);
            let tinted = self.tinted_channels.contains_key(&c.index);
            let cs = self.channels.entry(c.index).or_default();
            if is_new {
                cs.name = c.name.clone();
                cs.plugin = c.plugin.clone();
                cs.insert = c.insert;
                cs.volume = c.volume;
                cs.pan = c.pan;
                cs.mute = c.mute;
                cs.color = c.color;
                cs.announced = true;
                let name = if c.name.is_empty() { format!("Channel {}", c.index + 1) } else { c.name.clone() };
                self.out.push(HostEvent::TrackSeen { track: track.clone(), name, kind: TrackKind::Part });
                // A channel that arrives with its instrument in it (the user
                // added FL Keys; the first snapshot): the song learns the
                // device too. The live loop skips what the song already has.
                if let Some(p) = &c.plugin {
                    self.out.push(HostEvent::DeviceChanged { track: track.clone(), slot: DeviceSlot(0), plugin: Some(Self::plugin_identity(p)), pos });
                }
            } else {
                if cs.plugin != c.plugin {
                    cs.plugin = c.plugin.clone();
                    cs.slot = SlotState::default();
                    self.out.push(HostEvent::DeviceChanged { track: track.clone(), slot: DeviceSlot(0), plugin: c.plugin.as_ref().map(|p| Self::plugin_identity(p)), pos });
                }
                if (c.volume - cs.volume).abs() > 1e-4 {
                    if !cs.vol_reported {
                        cs.vol_reported = true;
                        self.out.push(HostEvent::Baseline { baseline: Baseline::Mixer { track: track.clone(), change: MixerChange::Volume(chan_to_gain(cs.volume)) }, pos });
                    }
                    cs.volume = c.volume;
                    self.out.push(HostEvent::MixerChanged { track: track.clone(), change: MixerChange::Volume(chan_to_gain(c.volume)), pos });
                }
                if (c.pan - cs.pan).abs() > 1e-4 {
                    if !cs.pan_reported {
                        cs.pan_reported = true;
                        self.out.push(HostEvent::Baseline { baseline: Baseline::Mixer { track: track.clone(), change: MixerChange::Pan(cs.pan) }, pos });
                    }
                    cs.pan = c.pan;
                    self.out.push(HostEvent::MixerChanged { track: track.clone(), change: MixerChange::Pan(c.pan), pos });
                }
                if c.mute != cs.mute {
                    if !cs.mute_reported {
                        cs.mute_reported = true;
                        self.out.push(HostEvent::Baseline { baseline: Baseline::Mixer { track: track.clone(), change: MixerChange::Mute(cs.mute) }, pos });
                    }
                    cs.mute = c.mute;
                    self.out.push(HostEvent::MixerChanged { track: track.clone(), change: MixerChange::Mute(c.mute), pos });
                }
                cs.name = c.name.clone();
                cs.insert = c.insert;
                if !tinted {
                    cs.color = c.color;
                }
            }
            let want_scan = cs.plugin.is_some() && (dirty_all || dirty.contains(&c.index) || !cs.slot.scanned_once);
            if want_scan && cs.slot.scanning_from.is_none() {
                cs.slot.scanning_from = Some(0);
                self.queue.push_back(FlRequest::Params { track: c.index, slot: -1, from: 0, count: PARAM_PAGE });
            }
        }
        // channels deleted in FL: their generator slot is gone
        let gone: Vec<i32> = self.channels.keys().filter(|i| !seen.contains(i)).copied().collect();
        for i in gone {
            if let Some(cs) = self.channels.remove(&i) {
                if cs.plugin.is_some() {
                    self.out.push(HostEvent::DeviceChanged { track: Self::track_for_channel(i), slot: DeviceSlot(0), plugin: None, pos });
                }
            }
        }
    }

    /// Another project is open now: forget the old tracks, re-snapshot, and
    /// tell the core (a first Version lands if this host has none yet).
    fn project_switched(&mut self) {
        self.tracks.clear();
        self.channels.clear();
        self.tinted_tracks.clear();
        self.tinted_channels.clear();
        self.dirty_all = true;
        self.last_changed = None;
        self.queue.push_back(FlRequest::Snapshot);
        if let Some(path) = self.project_path() {
            self.out.push(HostEvent::ProjectOpened { path, pos: self.pos() });
        }
    }

    /// The window title (`ui.getProgTitle()`) is the only hint FL's API
    /// gives about the open file; FL keeps the full paths of recent
    /// projects in the registry. Verified on FL 24.1.1 (Windows): the title
    /// holds "name.flp", MRU lives under HKCU\Software\Image-Line\FL Studio 24.
    pub fn project_path(&self) -> Option<PathBuf> {
        let Some(name) = title_to_filename(&self.title) else {
            // Verified on FL 24.1.1: `ui.getProgTitle()` is just "FL Studio 2024",
            // no file name. Best available guess: FL's most recent file.
            return recent_projects().into_iter().find(|p| p.is_file());
        };
        if let Some(dir) = &self.cfg.projects_dir {
            let p = dir.join(&name);
            if p.is_file() {
                return Some(p);
            }
        }
        if let Some(p) = recent_projects().into_iter().find(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(&name)).unwrap_or(false) && p.is_file()) {
            return Some(p);
        }
        let p = dirs_projects()?.join(&name);
        if p.is_file() { Some(p) } else { None }
    }

    /// Put every tinted track / channel back to its own color.
    fn restore_colors(&mut self) {
        for (t, color) in std::mem::take(&mut self.tinted_tracks) {
            self.queue.push_back(FlRequest::SetTrackColor { track: t, color });
        }
        for (c, color) in std::mem::take(&mut self.tinted_channels) {
            self.queue.push_back(FlRequest::SetChannelColor { chan: c, color });
        }
    }
}

/// An insert nobody touched: default name, no plugins, unity fader, centre,
/// unmuted. Master always counts as in use.
fn track_in_use(t: &FlTrack) -> bool {
    if t.index == 0 || !t.slots.is_empty() || t.mute {
        return true;
    }
    let default_name = t.name.is_empty() || t.name == format!("Insert {}", t.index);
    !default_name || (t.volume - FL_UNITY_VOLUME).abs() > 1e-3 || t.pan.abs() > 1e-3
}

/// "song.flp - FL Studio 2024" / "FL Studio 2024 - song.flp *" -> "song.flp".
pub fn title_to_filename(title: &str) -> Option<String> {
    title
        .split(" - ")
        .map(|s| s.trim().trim_end_matches('*').trim())
        .find(|s| s.to_ascii_lowercase().ends_with(".flp"))
        .map(|s| s.to_string())
}

/// Was the file written in the last couple of minutes? Guards the "changed
/// flag went back to 0" save detection.
fn recently_modified(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|age| age < Duration::from_secs(120))
        .unwrap_or(false)
}

/// FL's recent projects, newest first, from every installed FL version
/// (newest version first). Empty off Windows.
pub fn recent_projects() -> Vec<PathBuf> {
    if let Ok(list) = std::env::var("FLIT_FL_RECENT") {
        return list.split(';').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    }
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let Ok(il) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Image-Line") else { return vec![] };
        let mut versions: Vec<(u32, String)> = il
            .enum_keys()
            .filter_map(|k| k.ok())
            .filter_map(|k| k.strip_prefix("FL Studio ").and_then(|n| n.trim().parse::<u32>().ok()).map(|n| (n, k.clone())))
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0));
        let mut out = Vec::new();
        for (_, key) in versions {
            if let Ok(mru) = il.open_subkey(format!("{key}\\MRU")) {
                let mut entries: Vec<(u32, String)> = mru.enum_values().filter_map(|v| v.ok()).filter_map(|(name, _)| name.parse::<u32>().ok().map(|i| (i, name))).collect();
                entries.sort();
                for (_, name) in entries {
                    if let Ok(p) = mru.get_value::<String, _>(&name) {
                        out.push(PathBuf::from(p));
                    }
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    vec![]
}

/// FL mixer volume (0..1, 0.8 = unity) -> linear gain. VERIFY ON WINDOWS:
/// FL's fader law above unity (we assume linear in the normalized scale).
pub fn fl_to_gain(v: f32) -> f32 {
    (v / FL_UNITY_VOLUME).max(0.0)
}

pub fn gain_to_fl(g: f32) -> f32 {
    (g * FL_UNITY_VOLUME).clamp(0.0, 1.0)
}

/// Channel volume (0..1, 0.78125 = default) -> linear gain, and back.
pub fn chan_to_gain(v: f32) -> f32 {
    (v / FL_CHANNEL_UNITY).max(0.0)
}

pub fn gain_to_chan(g: f32) -> f32 {
    (g * FL_CHANNEL_UNITY).clamp(0.0, 1.0)
}

impl HostBridge for FlBridge {
    fn tag(&self) -> HostTag {
        HostTag::new(crate::FL_HOST_TAG)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            level: if self.cfg.python.is_some() { Level::Project } else { Level::LiveControl },
            set_params: true,
            // FL's scripting API cannot load or remove plugins (RESEARCH.md §6).
            set_devices: false,
            set_mixer: true,
            set_transport: true,
            read_project: self.cfg.python.is_some(),
            // No quiet single-track render through the API: the FLit plugin
            // on the track captures its own output instead. VERIFY ON WINDOWS.
            render_track: false,
            // FL applies fader/pan after the insert chain, so a plugin on the
            // track hears pre-fader audio: we cannot honor post-fader without
            // the user rerouting (or the bridge scaling by the fader). VERIFY ON WINDOWS.
            honors_print_point: false,
            host_state: false,
            report_presence: true,
            // Track / channel colors as tints, restored afterwards. VERIFY ON
            // WINDOWS: a tint must never end up in a saved project (see
            // request_save; a save started inside FL cannot be intercepted).
            show_presence: true,
            follow: true,
            read_state: true,
            // FL cannot be told to load a project file from a script.
            load_snapshot: false,
        }
    }

    fn poll(&mut self) -> Vec<HostEvent> {
        let stale = self.project_cache.as_ref().map(|(w, t, _)| *t != self.title || w.elapsed() >= Duration::from_secs(3)).unwrap_or(true);
        if stale {
            let p = self.project_path();
            self.project_cache = Some((Instant::now(), self.title.clone(), p));
        }
        for msg in self.port.recv() {
            if let Some(frame) = self.chunker.push(&msg) {
                match frame.dir {
                    Dir::Event => match serde_json::from_str::<FlEvent>(&frame.json) {
                        Ok(ev) => self.handle_event(ev),
                        Err(e) => tracing::debug!("bad FL event: {e}: {}", frame.json),
                    },
                    Dir::Reply => {
                        if let Some(p) = self.pending.remove(&frame.id) {
                            match serde_json::from_str::<FlReply>(&frame.json) {
                                Ok(r) => self.handle_reply(p.req, r),
                                Err(e) => tracing::debug!("bad FL reply: {e}: {}", frame.json),
                            }
                        }
                    }
                    Dir::Request => {}
                }
            }
        }
        self.maybe_quiet_save();
        self.pump_queue();
        std::mem::take(&mut self.out)
    }


    fn has_plugin(&self, plugin: &PluginIdentity) -> bool {
        // Same-name plugin somewhere in this project. Real identity matching
        // needs vendor/version from FL: VERIFY ON WINDOWS.
        matches!(plugin.format, PluginFormat::Other(ref f) if f == "fl-wrapper")
            && (self.tracks.values().any(|t| t.slots.values().any(|s| s.plugin == plugin.name)) || self.channels.values().any(|c| c.plugin.as_deref() == Some(plugin.name.as_str())))
    }

    fn set_param(&mut self, track: &TrackId, slot: DeviceSlot, plugin: &PluginIdentity, param: &ParamId, value: f32) -> ApplyOutcome {
        let Ok(p) = param.0.parse::<u32>() else { return ApplyOutcome::NotAvailable };
        if !self.has_plugin(plugin) {
            return ApplyOutcome::NotAvailable;
        }
        if let Some(chan) = Self::channel_for_track(track) {
            // the generator: slot 0 of the channel track = FL slot -1
            if slot.0 != 0 {
                return ApplyOutcome::NotAvailable;
            }
            self.suppress.insert((chan, -1, p), value);
            if let Some(e) = self.channels.get_mut(&chan).and_then(|c| c.slot.params.get_mut(&p)) {
                e.1 = value;
            }
            self.queue.push_back(FlRequest::SetParam { track: chan, slot: -1, param: p, value: value.clamp(0.0, 1.0) });
            return ApplyOutcome::Applied;
        }
        let Some(index) = Self::index_for_track(track) else { return ApplyOutcome::NotAvailable };
        self.suppress.insert((index, slot.0 as i32, p), value);
        if let Some(ss) = self.tracks.get_mut(&index).and_then(|t| t.slots.get_mut(&(slot.0 as i32))) {
            if let Some(e) = ss.params.get_mut(&p) {
                e.1 = value;
            }
        }
        self.queue.push_back(FlRequest::SetParam { track: index, slot: slot.0 as i32, param: p, value: value.clamp(0.0, 1.0) });
        ApplyOutcome::Applied
    }

    fn set_device(&mut self, track: &TrackId, slot: DeviceSlot, plugin: Option<&PluginIdentity>) -> ApplyOutcome {
        // FL's API cannot load or delete plugins. What it can do is switch a
        // mixer effect slot off: "removed" becomes bypassed (with a notice),
        // "back again" re-enables it. Anything else is a step for the user.
        if let Some(chan) = Self::channel_for_track(track) {
            if slot.0 != 0 {
                return ApplyOutcome::NotAvailable;
            }
            let have = self.channels.get(&chan).and_then(|c| c.plugin.clone());
            return match (have, plugin) {
                (None, None) => ApplyOutcome::Applied,
                (Some(h), Some(p)) if h == p.name => ApplyOutcome::Applied,
                _ => ApplyOutcome::Unsupported,
            };
        }
        let Some(index) = Self::index_for_track(track) else { return ApplyOutcome::NotAvailable };
        let s = slot.0 as i32;
        let (have, bypassed) = self.tracks.get(&index).and_then(|t| t.slots.get(&s)).map(|x| (Some(x.plugin.clone()), x.bypassed)).unwrap_or((None, false));
        let slot_no = slot.0 + 1;
        match (have, plugin) {
            (None, None) => ApplyOutcome::Applied,
            (Some(h), Some(p)) if h == p.name => {
                if bypassed {
                    self.queue.push_back(FlRequest::SetSlotEnabled { track: index, slot: s, value: true });
                    if let Some(x) = self.tracks.get_mut(&index).and_then(|t| t.slots.get_mut(&s)) {
                        x.bypassed = false;
                    }
                    self.out.push(HostEvent::Notice(format!("{track}: {h} in slot {slot_no} is on again")));
                }
                ApplyOutcome::Applied
            }
            (Some(h), None) => {
                if !bypassed {
                    self.queue.push_back(FlRequest::SetSlotEnabled { track: index, slot: s, value: false });
                    if let Some(x) = self.tracks.get_mut(&index).and_then(|t| t.slots.get_mut(&s)) {
                        x.bypassed = true;
                    }
                    self.out.push(HostEvent::Notice(format!("{track}: {h} in slot {slot_no} switched off (FL can't delete plugins from outside; remove it yourself if you want it gone)")));
                }
                ApplyOutcome::Applied
            }
            (Some(h), Some(p)) => {
                if !bypassed {
                    self.queue.push_back(FlRequest::SetSlotEnabled { track: index, slot: s, value: false });
                    if let Some(x) = self.tracks.get_mut(&index).and_then(|t| t.slots.get_mut(&s)) {
                        x.bypassed = true;
                    }
                    self.out.push(HostEvent::Notice(format!("{track}: {h} in slot {slot_no} switched off; this moment had {} there", p.name)));
                }
                ApplyOutcome::Unsupported
            }
            (None, Some(_)) => ApplyOutcome::Unsupported,
        }
    }

    fn set_mixer(&mut self, track: &TrackId, change: &MixerChange) -> ApplyOutcome {
        if let Some(chan) = Self::channel_for_track(track) {
            let cs = self.channels.entry(chan).or_default();
            match change {
                MixerChange::Volume(g) => {
                    cs.volume = gain_to_chan(*g);
                    self.queue.push_back(FlRequest::SetChanVolume { chan, value: cs.volume });
                }
                MixerChange::Pan(p) => {
                    cs.pan = *p;
                    self.queue.push_back(FlRequest::SetChanPan { chan, value: *p });
                }
                MixerChange::Mute(m) => {
                    cs.mute = *m;
                    self.queue.push_back(FlRequest::SetChanMute { chan, value: *m });
                }
            }
            return ApplyOutcome::Applied;
        }
        let Some(index) = Self::index_for_track(track) else { return ApplyOutcome::NotAvailable };
        let st = self.tracks.entry(index).or_default();
        match change {
            MixerChange::Volume(g) => {
                st.volume = gain_to_fl(*g);
                self.queue.push_back(FlRequest::SetVolume { track: index, value: st.volume });
            }
            MixerChange::Pan(p) => {
                st.pan = *p;
                self.queue.push_back(FlRequest::SetPan { track: index, value: *p });
            }
            MixerChange::Mute(m) => {
                st.mute = *m;
                self.queue.push_back(FlRequest::SetMute { track: index, value: *m });
            }
        }
        ApplyOutcome::Applied
    }

    fn set_transport(&mut self, change: &TransportChange) -> ApplyOutcome {
        match change {
            TransportChange::Play => {
                self.playing = true;
                self.queue.push_back(FlRequest::Play);
            }
            TransportChange::Stop => {
                self.playing = false;
                self.queue.push_back(FlRequest::Stop);
            }
            TransportChange::Tempo(b) => {
                self.tempo = *b;
                self.queue.push_back(FlRequest::SetTempo { bpm: *b });
            }
            TransportChange::Position(p) => {
                self.queue.push_back(FlRequest::SetPos { ticks: p.ticks * self.ppq as i64 / TICKS_PER_BEAT });
            }
            TransportChange::Loop { .. } | TransportChange::LoopOff => return ApplyOutcome::Unsupported,
        }
        ApplyOutcome::Applied
    }

    fn read_project(&mut self, path: &Path) -> Option<ProjectSnapshot> {
        let python = self.cfg.python.clone()?;
        crate::flp::read_project(&python, path).ok()
    }

    fn render_track(&mut self, _track: &TrackId, _from: SongPos, _to: SongPos) -> Option<RenderedAudio> {
        None
    }

    fn tracks(&self) -> Vec<TrackId> {
        self.tracks.keys().map(|t| Self::track_for_index(*t)).chain(self.channels.keys().map(|c| Self::track_for_channel(*c))).collect()
    }

    fn request_save(&mut self) -> ApplyOutcome {
        // Presence tints must never end up in the file: restore first. FL
        // saves itself; the heartbeat's changed flag then reports the
        // written file (Save As dialog when the project is untitled).
        self.restore_colors();
        // An asked-for save covers pending notes edits and makes a Version.
        self.notes_dirty = None;
        self.quiet_save_sent = false;
        self.version_save_at = Some(Instant::now());
        self.queue.push_back(FlRequest::Save);
        ApplyOutcome::Applied
    }

    fn master_has_processing(&self) -> Option<bool> {
        self.tracks.get(&0).map(|t| !t.slots.is_empty())
    }

    fn current_project(&self) -> Option<PathBuf> {
        // asked every tick by the agent: registry + file checks only every 3 s
        if let Some((when, title, path)) = &self.project_cache {
            if *title == self.title && when.elapsed() < Duration::from_secs(3) {
                return path.clone();
            }
        }
        self.project_path()
    }

    fn describe(&self) -> String {
        let ver = self.script.as_ref().map(|(api, fl, py)| format!("FL {fl}, scripting API v{api}, Python {py}; ")).unwrap_or_default();
        let link = if self.is_connected() {
            "script connected".to_string()
        } else if self.script.is_some() {
            "script went quiet (no heartbeat for 3 s)".to_string()
        } else {
            format!("waiting for the FL script: Options > MIDI Settings, Input list: '{p} In' enabled, Controller type 'FLit (user)', any port number; Output list: '{p} Out' enabled with the SAME port number; then View > Script output > FLit should say 'init ok'", p = self.cfg.port)
        };
        let hb = self.last_heartbeat.map(|t| format!("{:.1}s ago", t.elapsed().as_secs_f32())).unwrap_or_else(|| "never".into());
        format!("{ver}{link}; {} tracks, {} channels seen; title \"{}\"; last heartbeat {hb}", self.tracks.len(), self.channels.len(), self.title)
    }

    fn link_up(&self) -> Option<bool> {
        Some(self.is_connected())
    }

    fn show_presence(&mut self, shown: &[Shown]) -> ApplyOutcome {
        // one tint per track: the first visitor on it
        let mut want_tracks: BTreeMap<i32, u32> = BTreeMap::new();
        let mut want_chans: BTreeMap<i32, u32> = BTreeMap::new();
        for s in shown {
            let Some(t) = &s.presence.track else { continue };
            if let Some(i) = Self::index_for_track(t) {
                want_tracks.entry(i).or_insert(s.color);
            } else if let Some(c) = Self::channel_for_track(t) {
                want_chans.entry(c).or_insert(s.color);
            }
        }
        // restore what is no longer wanted
        let gone: Vec<i32> = self.tinted_tracks.keys().filter(|t| !want_tracks.contains_key(t)).copied().collect();
        for t in gone {
            if let Some(c) = self.tinted_tracks.remove(&t) {
                self.queue.push_back(FlRequest::SetTrackColor { track: t, color: c });
            }
        }
        let gone: Vec<i32> = self.tinted_channels.keys().filter(|c| !want_chans.contains_key(c)).copied().collect();
        for c in gone {
            if let Some(col) = self.tinted_channels.remove(&c) {
                self.queue.push_back(FlRequest::SetChannelColor { chan: c, color: col });
            }
        }
        for (t, color) in want_tracks {
            let original = self.tracks.get(&t).map(|x| x.color).unwrap_or(0);
            self.tinted_tracks.entry(t).or_insert(original);
            self.queue.push_back(FlRequest::SetTrackColor { track: t, color });
        }
        for (c, color) in want_chans {
            let original = self.channels.get(&c).map(|x| x.color).unwrap_or(0);
            self.tinted_channels.entry(c).or_insert(original);
            self.queue.push_back(FlRequest::SetChannelColor { chan: c, color });
        }
        ApplyOutcome::Applied
    }

    fn clear_presence(&mut self) {
        self.restore_colors();
    }

    fn follow(&mut self, presence: &PresencePayload) -> ApplyOutcome {
        if let Some(t) = &presence.track {
            if let Some(i) = Self::index_for_track(t) {
                self.queue.push_back(FlRequest::SelectTrack { track: i });
            } else if let Some(c) = Self::channel_for_track(t) {
                self.queue.push_back(FlRequest::SelectChannel { chan: c });
            }
        }
        let window = match presence.view {
            View::Mixer => Some("mixer"),
            View::Notes => Some("piano_roll"),
            View::Arrange => Some("playlist"),
            View::Browser => Some("browser"),
            View::Device => None,
            View::Other => presence.region.as_deref().filter(|r| *r == "channel rack").map(|_| "channel_rack"),
        };
        if let Some(w) = window {
            self.queue.push_back(FlRequest::ShowWindow { window: w.into() });
        }
        ApplyOutcome::Applied
    }

    fn read_state(&mut self) -> Option<HostModel> {
        let mut m = HostModel { tempo: Some(self.tempo), tracks: BTreeMap::new() };
        for (i, t) in &self.tracks {
            let devices = t
                .slots
                .iter()
                .filter(|(s, _)| **s >= 0)
                .map(|(s, ss)| (DeviceSlot(*s as u32), HostDevice { plugin: Self::plugin_identity(&ss.plugin), params: ss.params.iter().map(|(p, (_, v))| (ParamId::from_u32(*p), *v)).collect() }))
                .collect();
            m.tracks.insert(Self::track_for_index(*i), HostTrack { devices, volume: Some(fl_to_gain(t.volume)), pan: Some(t.pan), mute: Some(t.mute), host_state: None });
        }
        for (c, cs) in &self.channels {
            let mut devices = BTreeMap::new();
            if let Some(p) = &cs.plugin {
                devices.insert(DeviceSlot(0), HostDevice { plugin: Self::plugin_identity(p), params: cs.slot.params.iter().map(|(p, (_, v))| (ParamId::from_u32(*p), *v)).collect() });
            }
            m.tracks.insert(Self::track_for_channel(*c), HostTrack { devices, volume: Some(chan_to_gain(cs.volume)), pan: Some(cs.pan), mute: Some(cs.mute), host_state: None });
        }
        Some(m)
    }
}
