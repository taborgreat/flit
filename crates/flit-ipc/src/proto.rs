//! Messages between plugin (client) and agent (server).

use flit_core::{ActId, BeingId, BeingKind, BeingRef, HistoryId, Id, Lens, PluginIdentity, PresencePayload, Row, SongId, SongPos, Thing, TrackId, TransportChange, View};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 4: PACK 2 (orgs, albums, invites, Library) and the app window.
pub const PROTOCOL_VERSION: u32 = 4;

/// What kind of plugin instance (or tool) is talking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientKind {
    /// The FLit plugin on the master track: panel UI.
    Panel,
    /// The plugin on a track: sends that track's audio out.
    Track { track: TrackId },
    /// The plugin playing a friend's track in.
    Receiver { track: TrackId },
    /// Talk mode on a mixer track (sender), outside the song.
    TalkSend { name: String, to: Vec<Id> },
    /// Talk receiver on a mixer track.
    TalkReceive { from: Option<Id> },
    /// A command-line tool.
    Cli,
    /// The FLit app's window (PACK 2 §7): the full UI, no audio.
    Window,
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
    /// Send my tracks as one submix instead of separately.
    pub submix: Option<bool>,
    /// Hear the reference mix (the Reference DAW's master) instead of my local mix.
    pub hear_reference: Option<bool>,
    pub relay_only: Option<bool>,
    /// Tint tracks in the DAW with the colors of beings looking at them.
    pub show_presence_in_daw: Option<bool>,
    /// Where the release list lives (GitHub Releases manifest).
    pub update_manifest_url: Option<String>,
    /// "stable" | "beta"
    pub update_channel: Option<String>,
}

/// Driving the simulated DAW when the agent runs FakeHost (tests, demos).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FakeCmd {
    AddTrack { track: String, name: String, kind: String, plugin: Option<PluginIdentity>, tone_hz: f32 },
    AddDevice { track: String, slot: u32, plugin: PluginIdentity },
    RemoveDevice { track: String, slot: u32 },
    Twist { track: String, slot: u32, param: String, value: f32 },
    Sweep { track: String, slot: u32, param: String, from: f32, to: f32, steps: usize },
    Volume { track: String, value: f32 },
    Tempo { bpm: f64 },
    Play,
    Stop,
    SaveProject { path: PathBuf },
    /// Read back a parameter the core may have applied.
    Param { track: String, slot: u32, param: String },
    /// The user looks at something.
    Focus { view: View, track: Option<String>, slot: Option<u32> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Hello { kind: ClientKind, sample_rate: u32, channels: u8, protocol: u32 },
    /// Bring the FLit window forward (on `song` when given). With `spawn`,
    /// start the app when no window is connected (a plugin's [Open FLit]).
    OpenWindow { song: Option<PathBuf>, spawn: bool },
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
    BlockBeing { being: BeingRef },
    UnblockBeing { being: BeingRef },
    /// An owner settles an equivocation: the keeper seals the being again.
    Pardon { being: BeingRef },
    // beings of my Name
    Beings,
    ActAs { being: BeingId },
    NewBeing { id: BeingId, kind: BeingKind, display_name: String, color: Option<u32> },
    UpdateBeing { id: BeingId, display_name: String, color: u32 },
    RevokeBeing { id: BeingId },
    /// Let (or stop letting) one of my ai / scripted beings act on tracks directly.
    Grant { being: BeingId, tracks: Vec<TrackId>, grant: bool },
    /// "I think this being is not what it claims to be."
    Flag { being: BeingRef, note: String },
    // Names on this machine
    Names,
    SwitchName { id: Id },
    /// Name the current Name (first run).
    SetNameLabel { label: String },
    NewName { label: String, being: BeingId, display_name: String, kind: BeingKind },
    Logout,
    RemoveName { id: Id },
    /// Write an encrypted backup file of the current Name.
    BackupName { passphrase: String, path: PathBuf },
    ImportName { path: PathBuf, passphrase: String },
    /// The current Name's recovery code (shown once).
    RecoveryCode,
    ImportRecovery { code: String, label: String, being: BeingId },
    // keeper and song settings (owners)
    MoveKeeper { name: Id, machine: String },
    SetRequiredVersion { version: String },
    SetReferenceDaw { being: BeingRef },
    // the live flow
    /// Level 0: tempo/transport read from the plugin host's transport info.
    Transport { change: TransportChange, pos: SongPos },
    /// A track plugin's captured block (interleaved f32 at the Hello rate),
    /// with the host position of its first sample.
    Audio { track: TrackId, pos: SongPos, pcm: Vec<f32> },
    /// A Talk sender's block.
    TalkAudio { pcm: Vec<f32> },
    /// Record the Talk stream I am receiving into a track.
    RecordTalk { track: TrackId, start: bool },
    SaveVersion { name: String },
    /// Move the host's transport to a song position (chat "jump to moment").
    Jump { pos: SongPos },
    /// Level 0: where this plugin's user is (the agent settles it).
    Presence { presence: PresencePayload, pos: Option<SongPos> },
    /// Navigate my DAW to where a being is; `keep` = keep following.
    Follow { being: Option<BeingRef>, keep: bool },
    /// Show in DAW: fold the current history up to `at` and show it in the
    /// host as a read-only look. `None` = back to now. Live work is
    /// untouched until Go back / New history from here.
    ShowMoment { at: Option<ActId> },
    // histories and the timeline
    Histories,
    Timeline { history: HistoryId, lens: Lens, show_presence: bool },
    GoBack { to: ActId },
    NewHistory { name: String, from: ActId },
    SwitchHistory { history: HistoryId },
    Undo { target: ActId },
    Tracks,
    MergePreview { from_history: HistoryId, tracks: Vec<TrackId> },
    /// Confirm a Merge: FROM `from_history` TO the current history, with
    /// the conflict choices (thing, property, keep FROM).
    Merge { from_history: HistoryId, tracks: Vec<TrackId>, choices: Vec<(Thing, String, bool)> },
    // chat
    Chat { text: String, pinned: Option<ActId> },
    ChatList,
    // settings and updates
    Settings(SettingsPatch),
    CheckUpdates,

    // ----- PACK 2: the Library (orgs, albums, invites) -----
    /// Songs, orgs, pending invites and the open song's org in one view.
    Library,
    NewOrg { name: String },
    OrgMembers { org: SongId },
    /// An owner invites a Name: a friend code / Studio ID text, or a bare id.
    OrgInvite { org: SongId, friend: String },
    /// Join the org's chain and stamp Accept.
    AcceptOrgInvite { org: SongId },
    /// Forget an invite (song or org) without joining.
    DeclineInvite { container: SongId },
    OrgLeave { org: SongId },
    OrgRemove { org: SongId, id: Id },
    OrgPromote { org: SongId, id: Id },
    OrgDemote { org: SongId, id: Id },
    OrgSettings { org: SongId, label: Option<String>, color: Option<u32>, defaults: Option<flit_core::OrgDefaults>, archived: Option<bool> },
    OrgMoveKeeper { org: SongId, name: Id, machine: String },
    /// The open song: its org and who may act in it.
    SetSongOrg { org: Option<SongId>, access: flit_core::Access },
    SetAiPolicy { policy: flit_core::AiPolicy },
    /// New song under Me or an org (access from the org's defaults when None).
    NewSongIn { name: String, dir: PathBuf, org: Option<SongId>, access: Option<flit_core::Access> },
    Albums,
    NewAlbum { scope: AlbumScope, name: String },
    AlbumAct { scope: AlbumScope, album: flit_core::AlbumId, action: flit_core::AlbumAction },
    /// The album's own tape (answered with Timeline).
    AlbumTimeline { scope: AlbumScope, album: flit_core::AlbumId },
    // fake host (tests/demos only)
    Fake(FakeCmd),
    Shutdown,
    // (new requests go at the end: an installed plugin keeps its wire indices)
    /// Set an album's cover from a PNG file on this machine (stored as a
    /// blob of the org / shelf, sent to members like a project file).
    AlbumCover { scope: AlbumScope, album: flit_core::AlbumId, path: PathBuf },
    /// A blob of an org / shelf (an album cover), answered with Bytes.
    ContainerBlob { scope: AlbumScope, hash: flit_core::Hash },
    /// Open a song here and in its DAW: its project file with the OS
    /// default app (FL for .flp), or the DAW itself when the song has no
    /// project yet (save into the song's folder). Text says what happened.
    OpenInDaw { dir: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: Id,
    pub name: Option<String>,
    pub connected: bool,
    pub over_quota: bool,
    pub owner: bool,
}

/// A track as the panel lists it (names come from the host).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub track: TrackId,
    pub name: String,
    /// "part" | "bus" | "master"
    pub kind: String,
    /// Moments on the track (and its devices) in the current history.
    pub moments: usize,
    pub devices: usize,
}

/// A being as the panel shows it, with where it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeingRow {
    pub being: BeingRef,
    pub kind: BeingKind,
    pub display_name: String,
    /// 0xRRGGBB.
    pub color: u32,
    pub revoked: bool,
    pub delegated: bool,
    pub granted: Vec<TrackId>,
    pub flags: usize,
    /// Its Name's machine is connected right now.
    pub online: bool,
    /// The history it is in (from its last presence), if known.
    pub history: Option<HistoryId>,
    pub presence: Option<PresencePayload>,
    /// One of my Name's beings.
    pub mine: bool,
    /// The being I am acting as.
    pub acting: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NameRow {
    pub id: Id,
    pub label: String,
    pub current: bool,
    pub beings: Vec<(BeingId, String)>,
    pub backed_up: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryRow {
    pub history: HistoryId,
    pub name: String,
    pub current: bool,
    /// The history this one was started from (None for main).
    pub parent: Option<HistoryId>,
    /// The moment it started from, and that moment's song position.
    pub from: Option<ActId>,
    pub from_pos: Option<SongPos>,
    pub created_by: Option<BeingRef>,
    /// Beings in it right now.
    pub beings: Vec<BeingRef>,
    pub moments: usize,
    pub unsealed: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConflictRow {
    pub thing: Thing,
    pub property: String,
    /// For people: "Drums / Comp 7".
    pub label: String,
    pub from_summary: String,
    pub to_summary: String,
    pub from_being: BeingRef,
    pub to_being: BeingRef,
    pub keep_from: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MergeInfo {
    pub from_history: HistoryId,
    pub to_history: HistoryId,
    pub acts: usize,
    /// Moments per track since the histories parted.
    pub counts: Vec<(TrackId, usize)>,
    pub conflicts: Vec<ConflictRow>,
    pub flac_needed: usize,
}

/// How this machine reaches the crew right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Connection {
    #[default]
    Offline,
    Direct,
    Relayed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SongStatus {
    pub id: SongId,
    pub name: String,
    pub dir: PathBuf,
    pub code: String,
    pub owners: Vec<Id>,
    pub members: Vec<Id>,
    pub history: HistoryId,
    pub history_name: String,
    pub tracks: Vec<TrackInfo>,
    pub reference_daw: Option<BeingRef>,
    pub moments: usize,
    /// Moments waiting to be sealed (song-wide).
    pub unsealed: usize,
    /// My acts not yet handed to any peer.
    pub unsent: usize,
    pub tempo: f64,
    /// Tracks being heard as audio because this host can't reproduce them.
    pub heard_as_audio: Vec<TrackId>,
    /// Listening / archive chunks still to download for this history.
    pub missing_opus: usize,
    pub missing_flac: usize,
    pub keeper: Option<(Id, String)>,
    pub i_am_keeper: bool,
    pub keeper_online: bool,
    pub required_version: Option<String>,
    /// Set when this FLit is not the version the song requires.
    pub version_mismatch: Option<String>,
    /// Every being in the song, with where it is.
    pub beings: Vec<BeingRow>,
    pub histories: Vec<HistoryRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsView {
    pub download_full_quality: flit_core::DownloadPolicy,
    pub monitoring_bitrate: u32,
    pub submix: bool,
    pub hear_reference: bool,
    pub relay_only: bool,
    pub show_presence_in_daw: bool,
    pub update_manifest_url: String,
    pub update_channel: String,
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
    pub format_version: u32,
    /// The current Name.
    pub name: Id,
    pub short_code: String,
    pub id_text: String,
    pub name_label: String,
    /// The being I am acting as.
    pub being: BeingId,
    pub being_label: String,
    pub being_kind: BeingKind,
    /// The current Name's beings on this machine: (id, display name, kind).
    pub my_beings: Vec<(BeingId, String, BeingKind)>,
    pub names_on_machine: usize,
    pub host: String,
    pub level: u8,
    /// The project file the host has open, when the bridge knows it.
    pub project: Option<PathBuf>,
    /// The moment the host is showing instead of now, if any.
    pub previewing: Option<ActId>,
    /// What the bridge sees, in one line (empty without a bridge).
    pub bridge: String,
    /// Is the bridge's link into the host up? None = no bridge.
    pub bridge_up: Option<bool>,
    /// The DAW project vs. the open song.
    pub link: ProjectLink,
    /// Why live edits are not being stamped right now, if they are not.
    pub paused: Option<String>,
    pub connection: Connection,
    pub song: Option<SongStatus>,
    pub peers: Vec<PeerInfo>,
    pub pending_joins: Vec<(Id, String)>,
    pub invites: Vec<(Id, SongId, String)>,
    pub settings: SettingsView,
    pub warnings: Vec<String>,
    pub connected_clients: usize,
    /// The being I keep following, if any.
    pub following: Option<BeingRef>,
    /// A newer release than this one, if the last check found one.
    pub update_available: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatRow {
    pub id: ActId,
    pub from: BeingRef,
    pub from_label: String,
    pub color: u32,
    pub text: String,
    pub history: HistoryId,
    pub history_name: String,
    pub pos: Option<SongPos>,
    /// Witness time (the sender's clock), display only.
    pub stamped_at: u64,
    pub pinned: Option<ActId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Friend {
    pub id: Id,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReleaseRow {
    pub version: String,
    pub channel: String,
    pub notes: String,
    pub installed: bool,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current: String,
    pub newest: Option<String>,
    pub channel: String,
    pub releases: Vec<ReleaseRow>,
    /// What happened ("checked", "no manifest configured", an error).
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Response {
    Ok,
    Error(String),
    Pong,
    Status(Status),
    Songs(Vec<(String, PathBuf)>),
    Code(String),
    Histories(Vec<HistoryRow>),
    Timeline(Vec<Row>),
    Tracks(Vec<TrackInfo>),
    Chat(Vec<ChatRow>),
    MergePreview(MergeInfo),
    Friends(Vec<Friend>),
    Beings(Vec<BeingRow>),
    Names(Vec<NameRow>),
    Stamped(ActId),
    History(HistoryId),
    Value(Option<f32>),
    Text(String),
    Updates(UpdateInfo),
    Library(LibraryView),
    OrgMembers(Vec<OrgMemberRow>),
    Albums(Vec<AlbumRow>),
    Album(flit_core::AlbumId),
    Bytes(Vec<u8>),
}

// ----- PACK 2 rows -----

/// A song the Library knows (opened or made here), with what the registry
/// remembers of its org.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibrarySong {
    pub song: Option<SongId>,
    pub name: String,
    pub dir: PathBuf,
    pub org: Option<SongId>,
    pub access: Option<flit_core::Access>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrgRow {
    pub org: SongId,
    pub name: String,
    pub color: Option<u32>,
    pub owner: bool,
    pub member: bool,
    pub members: usize,
    pub archived: bool,
    pub keeper: Option<(Id, String)>,
    pub i_am_keeper: bool,
    pub defaults: flit_core::OrgDefaults,
    pub songs: Vec<LibrarySong>,
    pub albums: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrgMemberRow {
    pub id: Id,
    pub label: String,
    pub owner: bool,
    /// Invited, not yet accepted.
    pub invited: bool,
    /// (being, display name, kind, color) declared in the org.
    pub beings: Vec<(BeingId, String, BeingKind, u32)>,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlbumScope {
    Personal,
    Org(SongId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlbumSongRow {
    pub song: SongId,
    pub name: Option<String>,
    pub dir: Option<PathBuf>,
    pub access: Option<flit_core::Access>,
    /// In the album, but not a song this machine can open.
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlbumRow {
    pub scope: AlbumScope,
    pub scope_name: String,
    pub album: flit_core::AlbumId,
    pub name: String,
    pub description: String,
    pub cover: Option<flit_core::Hash>,
    pub archived: bool,
    pub created_by: Option<BeingRef>,
    /// Rename, describe, cover, archive: owners and the creator.
    pub can_curate: bool,
    pub songs: Vec<AlbumSongRow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InviteRow {
    pub from: Id,
    pub from_label: String,
    pub container: SongId,
    pub name: String,
    pub kind: flit_core::ContainerKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryView {
    pub songs: Vec<LibrarySong>,
    pub orgs: Vec<OrgRow>,
    pub invites: Vec<InviteRow>,
    /// The open song's org (id, name), access and ai policy.
    pub song_org: Option<(SongId, String)>,
    pub access: flit_core::Access,
    pub ai_policy: flit_core::AiPolicy,
    pub personal_albums: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Push {
    /// Something changed; here is the whole status.
    Status(Status),
    /// A new moment on the current history's tape.
    Row(Row),
    Chat(ChatRow),
    JoinRequest { id: Id, name: String },
    /// A song invite (Door 2) or an org invite (PACK 2 §1).
    Invite { from: Id, song: SongId, name: String, kind: flit_core::ContainerKind },
    /// Decoded audio of a friend's track, for the Receiver plugin on it.
    TrackAudio { track: TrackId, pcm: Vec<f32> },
    /// Decoded Talk audio for a TalkReceive plugin.
    TalkAudio { from: Id, name: String, pcm: Vec<f32> },
    Rejected { from: Id, reason: String },
    Warning(String),
    /// One of my moments collided with someone else's and was saved as a history.
    Declined { act: ActId, history: HistoryId, history_name: String, property: String },
    /// Where every being is right now (unsettled signals included).
    Presence(Vec<BeingRow>),
    /// To the app window: show yourself (on `song` when given).
    OpenWindow { song: Option<PathBuf> },
}
