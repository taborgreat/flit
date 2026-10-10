//! Messages between plugin (client) and agent (server).

use flit_core::{StampId, HistoryRow, Id, LaneId, LaneKind, BranchId, SongId, SongPos, TransportChange};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PROTOCOL_VERSION: u32 = 1;

/// What kind of plugin instance (or tool) is talking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientKind {
    /// The FLit plugin on the master track: panel UI.
    Panel,
    /// The plugin on a lane: sends that lane's audio out.
    Lane { lane: LaneId },
    /// The plugin playing a friend's lane in.
    Receiver { lane: LaneId },
    /// Talk mode on a mixer track (sender), outside the song.
    TalkSend { name: String, to: Vec<Id> },
    /// Talk receiver on a mixer track.
    TalkReceive { from: Option<Id> },
    /// A command-branch tool.
    Cli,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// Request id; responses echo it; pushes use 0.
    pub id: u32,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Body {
    Request(Request),
    Response(Response),
    Push(Push),
}

/// Changes to per-user settings from the panel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingsPatch {
    pub download_full_quality: Option<flit_core::DownloadPolicy>,
    /// 64 000..=256 000
    pub monitoring_bitrate: Option<u32>,
    /// Send my lanes as one submix instead of separately.
    pub submix: Option<bool>,
    /// Hear the reference mix (mix owner's master) instead of my local mix.
    pub hear_reference: Option<bool>,
    pub relay_only: Option<bool>,
    pub display_name: Option<String>,
}

/// Driving the simulated DAW when the agent runs FakeHost (tests, demos).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FakeCmd {
    AddTrack { lane: String, name: String, kind: LaneKind, plugin: Option<flit_core::PluginIdentity>, tone_hz: f32 },
    Twist { lane: String, slot: u32, param: String, value: f32 },
    Sweep { lane: String, slot: u32, param: String, from: f32, to: f32, steps: usize },
    Volume { lane: String, value: f32 },
    Tempo { bpm: f64 },
    Play,
    Stop,
    SaveProject { path: PathBuf },
    /// Read back a parameter the core may have applied.
    Param { lane: String, slot: u32, param: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Hello { kind: ClientKind, sample_rate: u32, channels: u8, protocol: u32 },
    Ping,
    Status,
    // songs
    NewSong { name: String, dir: PathBuf },
    /// Current project becomes a song, v1.
    ShareProject { name: String, dir: PathBuf, project: PathBuf },
    OpenSong { dir: PathBuf },
    CloseSong,
    Songs,
    // crew
    SongCode,
    JoinWithCode { code: String, dir: PathBuf },
    Approve { id: Id, owner: bool },
    Deny { id: Id },
    AcceptInvite { song: SongId, dir: PathBuf },
    Invite { friend: Id },
    Block { id: Id },
    AddFriend { id: String, name: String },
    Friends,
    Promote { id: Id },
    RemoveFromCrew { id: Id },
    LeaveSong,
    // the live flow
    /// Level 0: tempo/transport read from the plugin host's transport info.
    Transport { change: TransportChange, pos: SongPos },
    /// A lane plugin's captured block (interleaved f32 at the Hello rate),
    /// with the host position of its first sample.
    Audio { lane: LaneId, pos: SongPos, pcm: Vec<f32> },
    /// A Talk sender's block.
    TalkAudio { pcm: Vec<f32> },
    /// Record the Talk stream I am receiving into a lane I own.
    RecordTalk { lane: LaneId, start: bool },
    SaveVersion { name: String },
    /// Move the host's transport to a song position (chat "jump to moment").
    Jump { pos: SongPos },
    /// Timeline Exact view (addendum 4): fold the current branch up to `at`
    /// and show it in the host as a read-only look. `None` = back to now.
    /// Live work is untouched until Go back / New branch.
    Preview { at: Option<StampId> },
    // history
    Branches,
    History { branch: BranchId },
    GoBack { to: StampId },
    NewBranch { name: String, from: StampId },
    SwitchBranch { branch: BranchId },
    Undo { target: StampId },
    Lanes,
    ClaimLane { lane: LaneId, kind: LaneKind, name: Option<String> },
    HandOverLane { lane: LaneId, to: Id },
    BringPreview { lane: LaneId, source_branch: BranchId },
    BringIn { lane: LaneId, source_branch: BranchId, keep_both_as: Option<LaneId> },
    OfferLane { lane: LaneId },
    // chat
    Chat { text: String, pinned: Option<StampId> },
    ChatList,
    // settings
    Settings(SettingsPatch),
    // fake host (tests/demos only)
    Fake(FakeCmd),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: Id,
    pub name: Option<String>,
    pub connected: bool,
    pub over_quota: bool,
    pub owner: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LaneInfo {
    pub lane: LaneId,
    pub name: Option<String>,
    pub kind: LaneKind,
    pub owner: Option<Id>,
    pub mine: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub branch: BranchId,
    pub name: String,
    pub current: bool,
    /// The branch this one forked from (None for Main).
    #[serde(default)]
    pub parent: Option<BranchId>,
    /// The row it forked from, and that row's song position.
    #[serde(default)]
    pub from: Option<StampId>,
    #[serde(default)]
    pub from_pos: Option<SongPos>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SongStatus {
    pub id: SongId,
    pub name: String,
    pub dir: PathBuf,
    pub code: String,
    pub owners: Vec<Id>,
    pub members: Vec<Id>,
    pub branch: BranchId,
    pub branch_name: String,
    pub lanes: Vec<LaneInfo>,
    pub mix_owner: Option<Id>,
    pub i_own_mix: bool,
    pub events: usize,
    pub tempo: f64,
    /// Lanes being heard as audio because this host can't reproduce them.
    pub heard_as_audio: Vec<LaneId>,
    /// Listening / archive chunks still to download for this branch.
    pub missing_opus: usize,
    pub missing_flac: usize,
    /// What to call each Name: your own nickname for them if you set one,
    /// else the name they set for themselves in the song.
    #[serde(default)]
    pub names: Vec<(Id, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsView {
    pub download_full_quality: flit_core::DownloadPolicy,
    pub monitoring_bitrate: u32,
    pub submix: bool,
    pub hear_reference: bool,
    pub relay_only: bool,
    pub display_name: String,
}

/// How the DAW's open project relates to the open song (bridges only).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum ProjectLink {
    /// No bridge, or the bridge does not know the project.
    #[default]
    Unknown,
    /// The project's folder holds the open song.
    Linked { project: PathBuf },
    /// The project's folder holds no song (offer to make one).
    Unlinked { project: PathBuf },
    /// A song is open that belongs to another folder than the project:
    /// edits are paused until this is resolved.
    Mismatch { project: PathBuf, song_dir: PathBuf, song_name: String },
    /// The folder's song was made from another project file.
    OtherFile { project: PathBuf, song_from: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub agent_version: String,
    pub id: Id,
    pub short_code: String,
    pub id_text: String,
    pub host: String,
    pub level: u8,
    /// The project file the host has open, when the bridge knows it.
    #[serde(default)]
    pub project: Option<PathBuf>,
    /// The moment the host is showing as a preview (timeline scrub), if any.
    #[serde(default)]
    pub previewing: Option<StampId>,
    /// What the bridge sees, in __KEEPLINE__ (empty without a bridge).
    #[serde(default)]
    pub bridge: String,
    /// Is the bridge's link into the host up? None = no bridge.
    #[serde(default)]
    pub bridge_up: Option<bool>,
    /// The DAW project vs. the open song.
    #[serde(default)]
    pub link: ProjectLink,
    /// Why live edits are not being stamped right now, if they are not.
    #[serde(default)]
    pub paused: Option<String>,
    pub song: Option<SongStatus>,
    pub peers: Vec<PeerInfo>,
    pub pending_joins: Vec<(Id, String)>,
    pub invites: Vec<(Id, SongId, String)>,
    pub settings: SettingsView,
    pub warnings: Vec<String>,
    pub connected_clients: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatRow {
    pub id: StampId,
    pub from: Id,
    pub from_name: Option<String>,
    pub text: String,
    pub branch: BranchId,
    pub branch_name: String,
    pub pos: SongPos,
    pub wall_ms: u64,
    pub pinned: Option<StampId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewInfo {
    pub changes: usize,
    pub conflict: bool,
    pub flac_needed: usize,
    pub allowed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Friend {
    pub id: Id,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Response {
    Ok,
    Error(String),
    Pong,
    Status(Status),
    Songs(Vec<(String, PathBuf)>),
    Code(String),
    Branches(Vec<BranchInfo>),
    History(Vec<HistoryRow>),
    Lanes(Vec<LaneInfo>),
    Chat(Vec<ChatRow>),
    Preview(PreviewInfo),
    Friends(Vec<Friend>),
    Stamped(StampId),
    Branch(BranchId),
    Value(Option<f32>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Push {
    /// Something changed; ask for Status (or here it is).
    Status(Status),
    Row(HistoryRow),
    Chat(ChatRow),
    JoinRequest { id: Id, name: String },
    Invite { from: Id, song: SongId, name: String },
    /// Decoded audio of a friend's lane, for the Receiver plugin on it.
    LaneAudio { lane: LaneId, pcm: Vec<f32> },
    /// Decoded Talk audio for a TalkReceive plugin.
    TalkAudio { from: Id, name: String, pcm: Vec<f32> },
    Rejected { from: Id, reason: String },
    Warning(String),
}
