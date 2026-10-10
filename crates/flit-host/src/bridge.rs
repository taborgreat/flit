use flit_core::{
    DeviceSlot, HostTag, LaneId, LaneKind, MixerChange, Note, ParamId, PluginIdentity, PrintPoint, SongPos,
    TransportChange,
};
use std::path::{Path, PathBuf};

/// How deep a bridge reaches into its host.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum Level {
    /// No bridge. The plugin alone: streams its lane, plays others, reads
    /// tempo/position from the plugin host, saves capture rendered audio.
    PluginOnly = 0,
    /// Reports and applies parameter, mixer, transport and tempo changes.
    LiveControl = 1,
    /// Reads the host's project file: versions, notes, clips, arrangement.
    Project = 2,
    /// Reports focus and exposes host-specific tools to AI.
    Ai = 3,
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
    /// Can render one lane quietly (else the plugin captures its own lane).
    pub render_lane: bool,
    /// Can render a lane at the standard print point (post inserts, fader
    /// and pan; pre sends and master) without the user rerouting.
    pub honors_print_point: bool,
    /// Can capture/apply its opaque HostState blobs.
    pub host_state: bool,
    pub reports_focus: bool,
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
            render_lane: false,
            honors_print_point: true,
            host_state: false,
            reports_focus: false,
        }
    }

    pub fn level(&self) -> Level {
        self.level
    }
}

/// Which part of the host the user is looking at (level 3).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Focus {
    Mixer,
    PianoRoll,
    Arrangement,
    PluginEditor { lane: LaneId, slot: DeviceSlot },
    Other(String),
}

/// The value a control had before its first change in this session.
/// A bridge sends it once per control, right before that control's first
/// `ParamChanged` / `MixerChanged`, so History can go back to before the
/// change. Optional: a bridge that cannot read values beforehand sends none.
#[derive(Clone, Debug, PartialEq)]
pub enum Baseline {
    Param { lane: LaneId, slot: DeviceSlot, plugin: PluginIdentity, param: ParamId, value: f32 },
    Mixer { lane: LaneId, change: MixerChange },
}

/// Something the host did. Host -> core.
#[derive(Clone, Debug, PartialEq)]
pub enum HostEvent {
    /// What a control was before the user first moved it (see `Baseline`).
    Baseline { baseline: Baseline, pos: SongPos },
    ParamChanged {
        lane: LaneId,
        slot: DeviceSlot,
        plugin: PluginIdentity,
        param: ParamId,
        /// Normalized 0..1.
        value: f32,
        pos: SongPos,
    },
    MixerChanged {
        lane: LaneId,
        change: MixerChange,
        pos: SongPos,
    },
    /// A plugin was placed in (`Some`) or removed from (`None`) a slot.
    DeviceChanged {
        lane: LaneId,
        slot: DeviceSlot,
        plugin: Option<PluginIdentity>,
        pos: SongPos,
    },
    Transport {
        change: TransportChange,
        pos: SongPos,
    },
    /// The host wrote its project file (level 2).
    ProjectSaved {
        path: PathBuf,
        pos: SongPos,
    },
    /// The host opened a project file (level 2). Becomes a Version only
    /// when this host has none on the branch yet, so the song knows the
    /// starting point.
    ProjectOpened {
        path: PathBuf,
        pos: SongPos,
    },
    /// Something the user should read (the bridge did the nearest thing it
    /// could, or could not do something). Shown in the panel.
    Notice(String),
    /// The user added or renamed a track the bridge maps to a lane.
    LaneSeen {
        lane: LaneId,
        name: String,
        kind: LaneKind,
    },
    FocusChanged(Focus),
}

/// What the host's project holds, host-neutral (level 2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectSnapshot {
    pub tempo: Option<f64>,
    /// (lane, clip id, notes)
    pub notes: Vec<(LaneId, String, Vec<Note>)>,
    /// Arrangement: (lane, clip id, position) placements.
    pub arrangement: Vec<(LaneId, String, SongPos)>,
    /// Files the project uses (samples, presets).
    pub files: Vec<PathBuf>,
}

/// One lane rendered to interleaved f32 PCM.
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
    /// The host has no such plugin / lane; the lane will be heard as audio.
    NotAvailable,
    /// This bridge's level does not include that operation.
    Unsupported,
}

/// The one trait every bridge implements. Everything is optional beyond
/// `tag`, `capabilities` and `poll`; the defaults say "unsupported" and the
/// core falls back (usually to audio).
pub trait HostBridge: Send {
    /// Short tag written into events ("plugin" when there is no bridge).
    fn tag(&self) -> HostTag;
    fn capabilities(&self) -> Capabilities;

    /// Drain what the host did since the last call. Never blocks.
    fn poll(&mut self) -> Vec<HostEvent>;

    /// Does the host have this exact plugin (same format, vendor, name,
    /// version) available so a setting can be applied rather than heard?
    fn has_plugin(&self, _plugin: &PluginIdentity) -> bool {
        false
    }

    fn set_param(&mut self, _lane: &LaneId, _slot: DeviceSlot, _plugin: &PluginIdentity, _param: &ParamId, _value: f32) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }
    fn set_mixer(&mut self, _lane: &LaneId, _change: &MixerChange) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }
    /// Put `plugin` into the slot (or empty it with `None`). A host that
    /// cannot load plugins from outside returns `Unsupported` (the user is
    /// told what to do by hand) or `Applied` when the slot already matches.
    fn set_device(&mut self, _lane: &LaneId, _slot: DeviceSlot, _plugin: Option<&PluginIdentity>) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }
    fn set_transport(&mut self, _change: &TransportChange) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    /// Level 2: read a project file into host-neutral form.
    fn read_project(&mut self, _path: &Path) -> Option<ProjectSnapshot> {
        None
    }

    /// Render one lane at the standard print point. `None` = the plugin on
    /// that lane captures its own output instead.
    fn render_lane(&mut self, _lane: &LaneId, _from: SongPos, _to: SongPos) -> Option<RenderedAudio> {
        None
    }

    fn capture_host_state(&mut self, _lane: &LaneId) -> Option<Vec<u8>> {
        None
    }
    fn apply_host_state(&mut self, _lane: &LaneId, _blob: &[u8]) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    /// Lanes this host currently has (so saves know what changed).
    fn lanes(&self) -> Vec<LaneId> {
        Vec::new()
    }

    /// Does this host have processing on its master? `None` = can't tell.
    /// Used for the warning "Only you hear your master chain".
    fn master_has_processing(&self) -> Option<bool> {
        None
    }

    /// The project file the host has open right now, if the bridge knows.
    /// The panel offers it for "Share this project".
    fn current_project(&self) -> Option<PathBuf> {
        None
    }

    /// Ask the host to save its project now (so the Version carries the
    /// real file and, at level 2, its notes). `Applied` = the host will
    /// report `ProjectSaved` when it has written the file.
    fn request_save(&mut self) -> ApplyOutcome {
        ApplyOutcome::Unsupported
    }

    /// __KEEPLINE__ and the CLI: what the bridge sees (host
    /// version, link state, what to do if it is down).
    fn describe(&self) -> String {
        String::new()
    }

    /// Is the link into the host up? `None` = not applicable (no bridge).
    fn link_up(&self) -> Option<bool> {
        None
    }
}
