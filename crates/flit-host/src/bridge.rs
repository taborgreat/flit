//! The one trait every DAW bridge implements, and the host-neutral picture
//! of a DAW the core applies moments through (FLIT_SPEC §9 presence, §20
//! fold vs the DAW).

use flit_core::{BeingRef, DeviceSlot, HostTag, MixerChange, Note, ParamId, PluginIdentity, PresencePayload, PrintPoint, SongPos, SongState, TrackId, TransportChange};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How deep a bridge reaches into its host.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum Level {
    /// No bridge. The plugin alone: streams its track, plays others, reads
    /// tempo/position from the plugin host, saves capture rendered audio.
    PluginOnly = 0,
    /// Reports and applies parameter, mixer, transport and tempo changes.
    LiveControl = 1,
    /// Reads the host's project file: versions, notes, clips, arrangement.
    Project = 2,
    /// Reports presence in detail and exposes host-specific tools to ai beings.
    Ai = 3,
}

/// What a track is for. Descriptive only: no owners.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, serde::Serialize, serde::Deserialize, Default)]
pub enum TrackKind {
    #[default]
    Part,
    Bus,
    Master,
}

/// What a bridge can do. A bridge declares these once; the core consults
/// them and handles everything the bridge cannot.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Capabilities {
    pub level: Level,
    /// Can set parameters on third-party plugins (not only its own).
    pub set_params: bool,
    /// Can put a plugin into a device slot / take it out (FL cannot).
    #[serde(default)]
    pub set_devices: bool,
    pub set_mixer: bool,
    pub set_transport: bool,
    /// Can read the host project file in host-neutral form.
    pub read_project: bool,
    /// Can render one track quietly (else the plugin captures its own track).
    pub render_track: bool,
    /// Can render a track at the standard print point (post inserts, fader
    /// and pan; pre sends and master) without the user rerouting.
    pub honors_print_point: bool,
    /// Can capture/apply its opaque HostState blobs.
    pub host_state: bool,
    /// Translates the DAW's focus into presence reports.
    #[serde(default)]
    pub report_presence: bool,
    /// Can show other beings inside the DAW (tints), restored afterwards.
    #[serde(default)]
    pub show_presence: bool,
    /// Can navigate the DAW to where another being is.
    #[serde(default)]
    pub follow: bool,
    /// Can report its current state as a `HostModel` (so applies are diffs).
    #[serde(default)]
    pub read_state: bool,
    /// Can load a project file in one go (the snapshot path for big jumps).
    #[serde(default)]
    pub load_snapshot: bool,
}

impl Capabilities {
    pub const fn plugin_only() -> Self {
        Self {
            level: Level::PluginOnly,
            set_params: false,
            set_devices: false,
            set_mixer: false,
            set_transport: false,
            read_project: false,
            render_track: false,
            honors_print_point: true,
            host_state: false,
            report_presence: false,
            show_presence: false,
            follow: false,
            read_state: false,
            load_snapshot: false,
        }
    }

    pub fn level(&self) -> Level {
        self.level
    }
}

/// The value a control had before its first change in this session.
/// A bridge sends it once per control, right before that control's first
/// `ParamChanged` / `MixerChanged`, so Go back can reach before the change.
#[derive(Clone, Debug, PartialEq)]
pub enum Baseline {
    Param { track: TrackId, slot: DeviceSlot, plugin: PluginIdentity, param: ParamId, value: f32 },
    Mixer { track: TrackId, change: MixerChange },
}

/// Something the host did. Host -> core.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    /// What a control was before the user first moved it (see `Baseline`).
    Baseline { baseline: Baseline, pos: SongPos },
    ParamChanged {
        track: TrackId,
        slot: DeviceSlot,
        plugin: PluginIdentity,
        param: ParamId,
        /// Normalized 0..1.
        value: f32,
        pos: SongPos,
    },
    MixerChanged {
        track: TrackId,
        change: MixerChange,
        pos: SongPos,
    },
    /// A plugin was placed in (`Some`) or removed from (`None`) a slot.
    DeviceChanged {
        track: TrackId,
        slot: DeviceSlot,
        plugin: Option<PluginIdentity>,
        pos: SongPos,
    },
    Transport {
        change: TransportChange,
        pos: SongPos,
    },
    /// The host wrote its project file (level 2): a Version, plus the
    /// notes and audio that changed.
    ProjectSaved {
        path: PathBuf,
        pos: SongPos,
    },
    /// The project file at `path` holds the current notes (a quiet save
    /// the bridge asked for, or the host's own autosave): sync them, no
    /// Version. This is how notes reach the song while people work.
    NotesChanged {
        path: PathBuf,
        pos: SongPos,
    },
    /// The host opened a project file (level 2). Becomes a Version only
    /// when this host has none in the history yet, so the song knows the
    /// starting point.
    ProjectOpened {
        path: PathBuf,
        pos: SongPos,
    },
    /// Something the user should read (the bridge did the nearest thing it
    /// could, or could not do something). Shown in the panel.
    Notice(String),
    /// The user added or renamed a track the bridge maps to a track id.
    TrackSeen {
        track: TrackId,
        name: String,
        kind: TrackKind,
    },
    /// Where the user is in the DAW (unsettled; the live loop settles it).
    Presence {
        presence: PresencePayload,
        pos: Option<SongPos>,
    },
}

/// What the host's project holds, host-neutral (level 2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectSnapshot {
    pub tempo: Option<f64>,
    /// (track, clip id, notes)
    pub notes: Vec<(TrackId, String, Vec<Note>)>,
    /// Arrangement: (track, clip id, position) placements.
    pub arrangement: Vec<(TrackId, String, SongPos)>,
    /// Files the project uses (samples, presets).
    pub files: Vec<PathBuf>,
}

/// One track rendered to interleaved f32 PCM.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedAudio {
    pub sample_rate: u32,
    pub channels: u8,
    pub start: SongPos,
    pub samples: Vec<f32>,
    pub print_point: PrintPoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    /// The host has no such plugin / track; the track will be heard as audio.
    NotAvailable,
    /// This bridge's level does not include that operation.
    Unsupported,
}

// ----- the host model and diffs (§20) -----

/// What the DAW has right now, in the song model's terms. Cheap data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostModel {
    pub tempo: Option<f64>,
    pub tracks: BTreeMap<TrackId, HostTrack>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostTrack {
    pub devices: BTreeMap<DeviceSlot, HostDevice>,
    pub volume: Option<f32>,
    pub pan: Option<f32>,
    pub mute: Option<bool>,
    /// The bridge's opaque state for the track, if it reports it.
    pub host_state: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostDevice {
    pub plugin: PluginIdentity,
    pub params: BTreeMap<ParamId, f32>,
}

/// What has to change in the DAW to show a moment: only the differences.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    pub tempo: Option<f64>,
    pub tracks: Vec<TrackDiff>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackDiff {
    pub track: TrackId,
    /// Devices to place (slot, plugin): the slot is empty or holds another plugin.
    pub add_devices: Vec<(DeviceSlot, PluginIdentity)>,
    /// Slots to empty.
    pub remove_devices: Vec<DeviceSlot>,
    /// Parameters that differ: (slot, plugin, param, value).
    pub params: Vec<(DeviceSlot, PluginIdentity, ParamId, f32)>,
    pub mixer: Vec<MixerChange>,
    /// Full device state to restore in one call (this host's blob).
    pub host_state: Option<Vec<u8>>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.tempo.is_none() && self.tracks.iter().all(|t| t.add_devices.is_empty() && t.remove_devices.is_empty() && t.params.is_empty() && t.mixer.is_empty() && t.host_state.is_none())
    }
    pub fn device_changes(&self) -> usize {
        self.tracks.iter().map(|t| t.add_devices.len() + t.remove_devices.len()).sum()
    }
    pub fn param_changes(&self) -> usize {
        self.tracks.iter().map(|t| t.params.len()).sum()
    }
    pub fn mixer_changes(&self) -> usize {
        self.tracks.iter().map(|t| t.mixer.len()).sum()
    }
}

/// Cost of one device add/remove, in diff cost units (a param set is 1).
pub const DEVICE_COST: u32 = 20;
/// Above this cost the snapshot path (load the nearest Version's project
/// file, then the small remaining diff) is cheaper.
pub const SNAPSHOT_THRESHOLD: u32 = 60;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

/// Compare the folded model at a moment with the DAW's current state:
/// params that differ, devices that differ, mixer values the song has
/// recorded that differ, tempo. Devices present in both (same plugin) are
/// never reloaded.
pub fn diff(target: &SongState, current: &HostModel, host: &HostTag) -> Diff {
    let mut out = Diff::default();
    if current.tempo.map(|t| (t - target.settings.tempo).abs() > 1e-6).unwrap_or(true) {
        out.tempo = Some(target.settings.tempo);
    }
    for (track, ts) in &target.tracks {
        let cur = current.tracks.get(track);
        let mut td = TrackDiff { track: track.clone(), add_devices: vec![], remove_devices: vec![], params: vec![], mixer: vec![], host_state: None };
        for (slot, dev) in &ts.devices {
            let Some(plugin) = &dev.plugin else { continue };
            let cur_dev = cur.and_then(|c| c.devices.get(slot));
            let same_plugin = cur_dev.map(|d| d.plugin == *plugin).unwrap_or(false);
            if !same_plugin {
                td.add_devices.push((*slot, plugin.clone()));
            }
            for (param, v) in &dev.settings {
                let have = if same_plugin { cur_dev.and_then(|d| d.params.get(param).copied()) } else { None };
                if have.map(|h| !close(h, v.value)).unwrap_or(true) {
                    td.params.push((*slot, v.plugin.clone(), param.clone(), v.value));
                }
            }
        }
        // Slots the DAW fills that the moment does not: emptied only when
        // the song tracks devices on this track at all (a device FLit never
        // saw is left alone).
        if let Some(c) = cur {
            let tracked = ts.devices.values().any(|d| d.plugin.is_some());
            for slot in c.devices.keys() {
                let empty_in_target = ts.devices.get(slot).map(|d| d.plugin.is_none()).unwrap_or(true);
                if tracked && empty_in_target {
                    td.remove_devices.push(*slot);
                }
            }
        }
        if ts.mixer_set.volume && cur.and_then(|c| c.volume).map(|v| !close(v, ts.mixer.volume)).unwrap_or(true) {
            td.mixer.push(MixerChange::Volume(ts.mixer.volume));
        }
        if ts.mixer_set.pan && cur.and_then(|c| c.pan).map(|v| !close(v, ts.mixer.pan)).unwrap_or(true) {
            td.mixer.push(MixerChange::Pan(ts.mixer.pan));
        }
        if ts.mixer_set.mute && cur.and_then(|c| c.mute).map(|v| v != ts.mixer.mute).unwrap_or(true) {
            td.mixer.push(MixerChange::Mute(ts.mixer.mute));
        }
        if let Some(blob) = ts.host_state.get(host) {
            if cur.and_then(|c| c.host_state.as_ref()).map(|b| b != blob).unwrap_or(true) {
                td.host_state = Some(blob.clone());
            }
        }
        if !(td.add_devices.is_empty() && td.remove_devices.is_empty() && td.params.is_empty() && td.mixer.is_empty() && td.host_state.is_none()) {
            out.tracks.push(td);
        }
    }
    out
}

/// What `apply_diff` managed to push into the host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyReport {
    pub settings: usize,
    pub mixer: usize,
    pub devices: usize,
    pub tempo: bool,
    /// Tracks with settings this host cannot reproduce (hear them as audio).
    pub heard: Vec<TrackId>,
    /// Plugin placements the host cannot do from outside: what the user
    /// must do by hand to match the moment ("Drums: put X in slot 2").
    pub manual: Vec<String>,
    /// The snapshot path was taken (a project file was loaded first).
    pub snapshot_loaded: bool,
}

/// Another being to show inside the DAW.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub being: BeingRef,
    pub label: String,
    /// 0xRRGGBB.
    pub color: u32,
    pub presence: PresencePayload,
}

/// The one trait every bridge implements. Everything is optional beyond
/// `tag`, `capabilities` and `poll`; the defaults say "unsupported" and the
/// core falls back (usually to audio).
pub trait HostBridge: Send {
    /// Short tag written into acts ("plugin" when there is no bridge).
    fn tag(&self) -> HostTag;
    fn capabilities(&self) -> Capabilities;

    /// Drain what the host did since the last call. Never blocks.
    fn poll(&mut self) -> Vec<HostEvent>;

    /// Does the host have this exact plugin (same format, vendor, name,
    /// version) available so a setting can be applied rather than heard?
    fn has_plugin(&self, _plugin: &PluginIdentity) -> bool {
        false
    }

    fn set_param(&mut self, _track: &TrackId, _slot: DeviceSlot, _plugin: &PluginIdentity, _param: &ParamId, _value: f32) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }
    fn set_mixer(&mut self, _track: &TrackId, _change: &MixerChange) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }
    /// Put `plugin` into the slot (or empty it with `None`). A host that
    /// cannot load plugins from outside returns `Unsupported` (the user is
    /// told what to do by hand) or `Applied` when the slot already matches.
    fn set_device(&mut self, _track: &TrackId, _slot: DeviceSlot, _plugin: Option<&PluginIdentity>) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }
    fn set_transport(&mut self, _change: &TransportChange) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    /// Level 2: read a project file into host-neutral form.
    fn read_project(&mut self, _path: &Path) -> Option<ProjectSnapshot> {
        None
    }

    /// Render one track at the standard print point. `None` = the plugin on
    /// that track captures its own output instead.
    fn render_track(&mut self, _track: &TrackId, _from: SongPos, _to: SongPos) -> Option<RenderedAudio> {
        None
    }

    fn capture_host_state(&mut self, _track: &TrackId) -> Option<Vec<u8>> {
        None
    }
    fn apply_host_state(&mut self, _track: &TrackId, _blob: &[u8]) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    /// Tracks this host currently has (so saves know what changed).
    fn tracks(&self) -> Vec<TrackId> {
        Vec::new()
    }

    /// Does this host have processing on its master? `None` = can't tell.
    /// Used for the warning "Only you hear your master chain".
    fn master_has_processing(&self) -> Option<bool> {
        None
    }

    /// The project file the host has open right now, if the bridge knows.
    fn current_project(&self) -> Option<PathBuf> {
        None
    }

    /// Ask the host to save its project now (so the Version carries the
    /// real file and, at level 2, its notes). `Applied` = the host will
    /// report `ProjectSaved` when it has written the file.
    fn request_save(&mut self) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    /// For the panel and the CLI: what the bridge sees (host version, link
    /// state, what to do if it is down).
    fn describe(&self) -> String {
        String::new()
    }

    /// Is the link into the host up? `None` = not applicable (no bridge).
    fn link_up(&self) -> Option<bool> {
        None
    }

    // ----- presence (§9) -----

    /// Show where other beings are inside the DAW (tints in their colors).
    /// The tint is never an edit: originals restored by `clear_presence`,
    /// stripped before any save.
    fn show_presence(&mut self, _shown: &[Shown]) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }
    fn clear_presence(&mut self) {}
    /// Navigate the DAW to where a being is.
    fn follow(&mut self, _presence: &PresencePayload) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    // ----- fold vs the DAW (§20) -----

    /// The DAW's current state in model form, so an apply can be a diff.
    fn read_state(&mut self) -> Option<HostModel> {
        None
    }

    /// Load a project file in one go (the snapshot path).
    fn load_snapshot(&mut self, _project: &Path) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    /// How expensive a diff is to apply here (param = 1, device = 20).
    fn estimate_cost(&self, diff: &Diff) -> u32 {
        diff.device_changes() as u32 * DEVICE_COST + diff.param_changes() as u32 + diff.mixer_changes() as u32 + diff.tempo.is_some() as u32
    }

    /// Apply a diff: devices that differ first (a setting lands on the
    /// plugin that is there), then params, mixer, host state, tempo. The
    /// default uses the single setters; a bridge may batch.
    fn apply_diff(&mut self, diff: &Diff) -> ApplyReport {
        let caps = self.capabilities();
        let mut r = ApplyReport::default();
        for td in &diff.tracks {
            let mut heard = false;
            for slot in &td.remove_devices {
                let _ = self.set_device(&td.track, *slot, None);
            }
            for (slot, plugin) in &td.add_devices {
                match self.set_device(&td.track, *slot, Some(plugin)) {
                    ApplyOutcome::Applied => r.devices += 1,
                    _ => r.manual.push(format!("{}: put {} in slot {}", td.track, plugin.name, slot.0 + 1)),
                }
            }
            if let Some(blob) = &td.host_state {
                if caps.host_state && self.apply_host_state(&td.track, blob) == ApplyOutcome::Applied {
                    // the blob carries the settings: params below only fix leftovers
                }
            }
            for (slot, plugin, param, value) in &td.params {
                let outcome = if caps.set_params && self.has_plugin(plugin) { self.set_param(&td.track, *slot, plugin, param, *value) } else { ApplyOutcome::NotAvailable };
                match outcome {
                    ApplyOutcome::Applied => r.settings += 1,
                    _ => heard = true,
                }
            }
            if caps.set_mixer {
                for c in &td.mixer {
                    if self.set_mixer(&td.track, c) == ApplyOutcome::Applied {
                        r.mixer += 1;
                    }
                }
            }
            if heard {
                r.heard.push(td.track.clone());
            }
        }
        if let Some(t) = diff.tempo {
            if caps.set_transport {
                r.tempo = self.set_transport(&TransportChange::Tempo(t)) == ApplyOutcome::Applied;
            }
        }
        r
    }
}
