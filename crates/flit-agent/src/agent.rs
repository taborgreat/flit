//! The agent: one per user per machine. Owns the Names, the song, the
//! network, the codecs and the bridge; serves every plugin instance over
//! IPC. It is the keeper of a song when the song says so.

use crate::bridge::{AnyBridge, NoBridge};
use crate::store::{machine_id, Registry, SongNote};
use flit_audio::{FlitChunkEncoder, StreamConfig, StreamDecoder, StreamEncoder, StreamKind};
use flit_core::*;
use flit_host::{ApplyOutcome, ChunkEncoder, HostBridge, LiveLoop, Receiver, ReceiverDecision, RenderedAudio, Shown, TrackKind};
use flit_ipc::*;
use flit_net::{AddrInfo, NetEvent, Node, NodeConfig, StreamFrame};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Keep at most this much captured audio per track between saves (10 min stereo at 48k).
const MAX_CAPTURE_SAMPLES: usize = 48_000 * 2 * 600;
/// The id of the reference-mix stream key.
const REFERENCE_KEY: &str = "__reference";
const SUBMIX_KEY: &str = "__submix";

pub struct AgentConfig {
    /// Fixed test identity instead of the stored Name (tests only).
    pub seed: Option<u64>,
    /// Display name for the first being when a Name is created on first run.
    pub display_name: String,
    pub data_dir: PathBuf,
    pub bridge: AnyBridge,
    pub local_only: bool,
    pub relay_only: bool,
    pub open_song: Option<PathBuf>,
    /// Set by the FLit app: called when someone asks for the window and no
    /// window client is connected (the app then opens one). None = the
    /// helper starts `flit-app` instead.
    pub window_hook: Option<WindowHook>,
}

/// Called with the song folder to show, if any.
pub type WindowHook = Arc<dyn Fn(Option<PathBuf>) + Send + Sync>;

#[derive(Clone, Debug)]
struct ClientInfo {
    kind: ClientKind,
    sample_rate: u32,
    channels: u8,
}

/// Audio captured from one track plugin since the last save, by segment.
#[derive(Default)]
struct Capture {
    segments: Vec<(SongPos, Vec<f32>)>,
    last_pos: Option<SongPos>,
    sample_rate: u32,
    channels: u8,
}

struct Outgoing {
    enc: StreamEncoder,
    /// (peer, stream number) this track is open on.
    streams: HashMap<Id, u16>,
}

pub struct State {
    /// Last bridge link state pushed to the panel (so a flip pushes a Status).
    last_link: Option<bool>,
    /// The last "could not open the song" the link produced; said once.
    last_link_error: Option<String>,
    window_hook: Option<WindowHook>,
    pub node: Node,
    pub live: LiveLoop<AnyBridge, FlitChunkEncoder>,
    names: NameStore,
    profile: NameProfile,
    machine: String,
    data_dir: PathBuf,
    registry: Registry,
    registry_path: PathBuf,
    settings: UserSettings,
    clients: HashMap<ConnId, ClientInfo>,
    outgoing: HashMap<String, Outgoing>,
    incoming: HashMap<(Id, u16), (StreamKind, StreamDecoder)>,
    capture: HashMap<TrackId, Capture>,
    heard_as_audio: HashSet<TrackId>,
    talk_record: Option<(TrackId, Id, Vec<f32>, SongPos, u32, u8)>,
    warnings: Vec<String>,
    master_warned: bool,
    status_dirty: bool,
    presence_dirty: bool,
    song_dir: Option<PathBuf>,
    last_pos: SongPos,
    /// Show in DAW: the moment the host is showing instead of now.
    preview: Option<ActId>,
    /// The DAW project the agent last linked (or tried to).
    linked_project: Option<PathBuf>,
    link: ProjectLink,
    /// Live edits are not stamped while this is set (project/song mismatch).
    paused: Option<String>,
    /// The being my DAW keeps following, if any.
    following: Option<BeingRef>,
    /// Last presence list pushed (to push only on change).
    last_presence: Vec<BeingRow>,
    /// What others' presence looked like when the DAW was last tinted.
    last_shown: Vec<(BeingRef, PresencePayload)>,
    update_available: Option<String>,
    /// Orgs joined whose Invite act has not arrived yet: Accept is retried.
    pending_accepts: HashSet<SongId>,
    pub shutdown: bool,
}

#[derive(Clone)]
pub struct Agent {
    pub state: Arc<Mutex<State>>,
    pub handle: Arc<Mutex<Option<ServerHandle>>>,
}

fn now_ms() -> u64 {
    flit_core::now_ms()
}

/// A being id from a display name ("Tabor Great" -> "tabor-great").
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if (c == ' ' || c == '-' || c == '_') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "me".into() } else { out }
}

/// The DAW to start for a song without a project: FL Studio where it is
/// usually installed (newest version first), else none.
fn daw_exe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("FLIT_DAW") {
        return Some(PathBuf::from(p));
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for base in ["C:\\Program Files\\Image-Line", "C:\\Program Files (x86)\\Image-Line"] {
        if let Ok(rd) = std::fs::read_dir(base) {
            for e in rd.flatten() {
                let exe = e.path().join("FL64.exe");
                if exe.is_file() {
                    found.push(exe);
                }
            }
        }
    }
    found.sort();
    found.pop()
}

fn kind_name(k: TrackKind) -> &'static str {
    match k {
        TrackKind::Part => "part",
        TrackKind::Bus => "bus",
        TrackKind::Master => "master",
    }
}

impl State {
    fn stamper(&self) -> Stamper {
        self.live.stamper().clone()
    }

    fn me(&self) -> Id {
        self.live.stamper().name
    }

    fn being(&self) -> BeingRef {
        self.live.stamper().being_ref()
    }

    fn history(&self) -> HistoryId {
        self.live.history()
    }

    /// What to call a Name: my own nickname, else what its beings are called.
    fn name_for(&self, id: &Id) -> Option<String> {
        if let Some(n) = self.registry.friend_name(id) {
            return Some(n);
        }
        self.node.with_song(|s| s.beings().of_name(id).into_iter().find(|(_, b)| !b.revoked && !b.display_name.is_empty()).map(|(_, b)| b.display_name.clone())).ok().flatten()
    }

    fn being_label(&self, b: &BeingRef) -> String {
        self.node.with_song(|s| s.being_label(b)).unwrap_or_else(|_| b.being.0.clone())
    }

    /// Every being of my Name goes into the open song as Being acts (once
    /// per change), so the crew folds them from my key.
    fn announce_beings(&mut self) {
        let who = self.stamper();
        let profile = self.profile.clone();
        let now = now_ms();
        let changed = self
            .node
            .with_song(|s| {
                let mut changed = false;
                for b in &profile.beings {
                    let r = BeingRef { name: who.name, being: b.id.clone() };
                    match s.beings().get(&r) {
                        None => {
                            if s.declare_being(&who, &b.id.0, b.kind, &b.display_name, b.color, b.delegated_key, vec![], now).is_ok() {
                                changed = true;
                            }
                        }
                        Some(info) if !info.revoked && (info.display_name != b.display_name.trim() || info.color != b.color) => {
                            if s.update_being(&who, &b.id.0, &b.display_name, b.color, now).is_ok() {
                                changed = true;
                            }
                        }
                        _ => {}
                    }
                }
                changed
            })
            .unwrap_or(false);
        if changed {
            self.node.publish();
            self.status_dirty = true;
        }
    }

    /// Take the keeper role when the song names this Name and machine.
    fn claim_keeper_if_mine(&mut self) {
        let me = self.me();
        let machine = self.machine.clone();
        let who = self.stamper();
        let now = now_ms();
        let _ = self.node.with_song(|s| {
            let mine = match s.keeper() {
                Some((name, m)) => name == me && (m.is_empty() || m == machine),
                None => false,
            };
            let had = s.keeper_key().is_some();
            if mine && !had {
                tracing::info!("this machine is the keeper of \"{}\"", s.name());
                let _ = s.set_keeper(Some(who.keeper_key()), now);
            } else if !mine && had {
                tracing::info!("the keeper role left this machine");
                let _ = s.set_keeper(None, now);
            }
        });
        self.node.publish();
    }

    fn drain_notices(&mut self) -> Vec<Push> {
        let notices = self.node.with_song(|s| s.take_notices()).unwrap_or_default();
        let mut out = Vec::new();
        let me = self.me();
        for n in notices {
            match n {
                Notice::Declined { act, being, history, history_name, property } => {
                    if being.name == me {
                        out.push(Push::Declined { act, history, history_name: history_name.clone(), property: property.clone() });
                        self.warnings.push(format!("Your change to {property} collided with someone else's. Saved as the history \"{history_name}\"."));
                    }
                }
                Notice::Held { act, reason } => {
                    if self.node.with_song(|s| s.chains().get(&act).map(|a| a.name() == me)).ok().flatten().unwrap_or(false) {
                        out.push(Push::Warning(format!("A moment of yours is waiting: {reason}")));
                    }
                }
                Notice::Equivocation { being, acts } => {
                    let label = self.being_label(&being);
                    out.push(Push::Warning(format!("{label} stamped two moments with one number. The keeper holds their new moments until an owner decides.")));
                    // an owner flags it with the evidence
                    let who = self.stamper();
                    if self.node.with_song(|s| s.crew().is_owner(&me)).unwrap_or(false) {
                        let _ = self.node.with_song(|s| s.flag(&who, being.clone(), FlagKind::Equivocation { acts }, "", now_ms()));
                        self.node.publish();
                    }
                }
            }
        }
        if !out.is_empty() {
            self.status_dirty = true;
        }
        out
    }

    fn being_rows(&self, song: &Song) -> Vec<BeingRow> {
        let me = self.me();
        let acting = self.being();
        let online: HashSet<Id> = self.node.connected_peers().into_iter().collect();
        let mut rows = Vec::new();
        let unsettled = self.live.presence_now().cloned();
        for (b, info) in song.beings().all() {
            let state = song.state(&self.history());
            let folded = state.and_then(|st| st.presence.get(&b)).map(|p| PresencePayload { view: p.view, track: p.track.clone(), device: p.device, region: p.region.clone() });
            let presence = if b == acting { unsettled.clone().or(folded) } else { folded };
            let history = song.histories().into_iter().find(|h| song.chains().act_chain(&b, &h.id).last().map(|a| a.id) == song.chains().latest(&h.id)).map(|h| h.id);
            rows.push(BeingRow {
                kind: info.kind,
                display_name: info.display_name.clone(),
                color: info.color,
                revoked: info.revoked,
                delegated: info.key.is_some(),
                granted: song.beings().grants_of(&b),
                flags: song.beings().flags_for(&b).len(),
                online: b.name == me || online.contains(&b.name),
                history: history.or(Some(self.history())),
                presence,
                mine: b.name == me,
                acting: b == acting,
                being: b,
            });
        }
        rows
    }

    fn history_rows(&self, song: &Song) -> Vec<HistoryRow> {
        let cur = self.history();
        let beings = self.being_rows(song);
        song.histories()
            .into_iter()
            .map(|h| HistoryRow {
                history: h.id,
                name: h.name.clone(),
                current: h.id == cur,
                parent: h.parent,
                from: h.from,
                from_pos: h.from.and_then(|f| song.chains().get(&f)).and_then(|a| a.pos()),
                created_by: h.created_by.clone(),
                beings: beings.iter().filter(|b| b.history == Some(h.id) && b.online).map(|b| b.being.clone()).collect(),
                moments: song.chains().seal_seq(&h.id) as usize,
                unsealed: song.chains().unsealed_count(&h.id),
            })
            .collect()
    }

    fn track_infos(&self, song: &Song) -> Vec<TrackInfo> {
        let h = self.history();
        let mut counts: BTreeMap<TrackId, usize> = BTreeMap::new();
        for r in song.history(&h) {
            if let Some(t) = r.thing.track() {
                *counts.entry(t.clone()).or_default() += 1;
            }
        }
        let mut ids: HashSet<TrackId> = self.live.track_names().keys().cloned().collect();
        if let Some(st) = song.state(&h) {
            ids.extend(st.tracks.keys().cloned());
        }
        ids.extend(counts.keys().cloned());
        let mut v: Vec<TrackInfo> = ids
            .into_iter()
            .map(|t| {
                let (name, kind) = self.live.track_names().get(&t).cloned().unwrap_or_else(|| (t.0.clone(), TrackKind::Part));
                let devices = song.state(&h).and_then(|s| s.track(&t)).map(|ts| ts.devices.values().filter(|d| d.plugin.is_some()).count()).unwrap_or(0);
                TrackInfo { moments: counts.get(&t).copied().unwrap_or(0), devices, track: t, name, kind: kind_name(kind).into() }
            })
            .collect();
        v.sort_by(|a, b| (a.kind != "master").cmp(&(b.kind != "master")).then(a.name.cmp(&b.name)));
        v
    }

    fn song_status(&self) -> Option<SongStatus> {
        let me = self.me();
        let history = self.history();
        let code = self.node.song_code().map(|c| c.encode()).unwrap_or_default();
        let online: HashSet<Id> = self.node.connected_peers().into_iter().collect();
        self.node
            .with_song(|s| {
                let st = s.state(&history);
                let keeper = s.keeper();
                let required = s.chains().required_version();
                SongStatus {
                    id: s.id(),
                    name: s.name().to_string(),
                    dir: self.song_dir.clone().unwrap_or_else(|| s.dir().to_path_buf()),
                    code,
                    owners: s.crew().owners.iter().copied().collect(),
                    members: s.crew().members.iter().copied().collect(),
                    history,
                    history_name: s.history_name(&history).unwrap_or("?").to_string(),
                    tracks: self.track_infos(s),
                    reference_daw: st.and_then(|st| st.settings.reference_daw.clone()),
                    moments: s.chains().len(),
                    unsealed: s.unsealed_count(),
                    unsent: if online.is_empty() { s.chains().unsealed(&history).iter().filter(|a| a.name() == me).count() } else { 0 },
                    tempo: st.map(|st| st.settings.tempo).unwrap_or(120.0),
                    heard_as_audio: self.heard_as_audio.iter().cloned().collect(),
                    missing_opus: s.missing_opus(&history).len(),
                    missing_flac: s.missing_flac(&history).len(),
                    i_am_keeper: s.is_keeper(),
                    keeper_online: keeper.as_ref().map(|(n, _)| *n == me || online.contains(n)).unwrap_or(false),
                    keeper,
                    version_mismatch: required.as_ref().filter(|r| **r != VERSION).cloned(),
                    required_version: required,
                    beings: self.being_rows(s),
                    histories: self.history_rows(s),
                }
            })
            .ok()
    }

    fn status(&self) -> Status {
        let id = self.me();
        let song = self.song_status();
        let peers = self
            .node
            .peers()
            .into_iter()
            .map(|p| PeerInfo { name: self.name_for(&p.id), owner: song.as_ref().map(|s| s.owners.contains(&p.id)).unwrap_or(false), id: p.id, connected: p.connected, over_quota: p.over_quota })
            .collect();
        let invites = self.node.pending_invites().into_iter().map(|i| (i.from, i.song, i.name)).collect();
        let being = self.being();
        let active = self.profile.being(&being.being);
        Status {
            agent_version: VERSION.to_string(),
            format_version: FORMAT_VERSION,
            name: id,
            short_code: id.short_code(),
            id_text: self.node.addr_info().encode(),
            name_label: self.profile.label.clone(),
            being: being.being.clone(),
            being_label: active.map(|b| b.display_name.clone()).unwrap_or_else(|| being.being.0.clone()),
            being_kind: active.map(|b| b.kind).unwrap_or_default(),
            my_beings: self.profile.beings.iter().map(|b| (b.id.clone(), b.display_name.clone(), b.kind)).collect(),
            names_on_machine: self.names.list().map(|v| v.len()).unwrap_or(1),
            host: self.live.host().0,
            level: self.live.bridge.capabilities().level as u8,
            project: self.live.bridge.current_project(),
            previewing: self.preview,
            bridge: self.live.bridge.describe(),
            bridge_up: self.live.bridge.link_up(),
            link: self.link.clone(),
            paused: self.paused.clone(),
            connection: match self.node.relayed() {
                None => Connection::Offline,
                Some(true) => Connection::Relayed,
                Some(false) => Connection::Direct,
            },
            song,
            peers,
            pending_joins: self.node.pending_join_requests(),
            invites,
            settings: SettingsView {
                download_full_quality: self.settings.download_full_quality,
                monitoring_bitrate: self.registry.monitoring_bitrate.unwrap_or(128_000),
                submix: self.registry.submix,
                hear_reference: self.registry.hear_reference,
                relay_only: self.registry.relay_only,
                show_presence_in_daw: self.registry.show_presence_in_daw,
                update_manifest_url: self.registry.update_manifest_url.clone(),
                update_channel: self.registry.update_channel.clone(),
            },
            warnings: self.warnings.clone(),
            connected_clients: self.clients.len(),
            following: self.following.clone(),
            update_available: self.update_available.clone(),
        }
    }

    fn chat_rows(&self) -> Vec<ChatRow> {
        self.node
            .with_song(|s| {
                s.all_chat()
                    .into_iter()
                    .filter_map(|(a, history_name)| match a.payload() {
                        Payload::Chat(c) => {
                            let from = a.being();
                            Some(ChatRow {
                                id: a.id,
                                from_label: s.being_label(&from),
                                color: s.being_color(&from).unwrap_or(0x9AA5B1),
                                from,
                                text: c.text.clone(),
                                history: s.chains().history_of(&a.id).unwrap_or(a.history()),
                                history_name,
                                pos: a.pos(),
                                stamped_at: a.body.stamped_at,
                                pinned: c.pinned,
                            })
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn open_song_dir(&mut self, project_dir: &Path) -> std::result::Result<(), String> {
        let flit = project_dir.join(".flit");
        if !Song::exists(&flit) {
            return Err(format!("no song at {}", flit.display()));
        }
        let song = Song::open(&flit).map_err(|e| e.to_string())?;
        let name = song.name().to_string();
        self.node.close_song();
        self.node.open_song(song);
        self.song_dir = Some(project_dir.to_path_buf());
        self.live.set_history(main_history());
        self.node.with_song(|s| self.live.prime_from(s)).ok();
        self.registry.remember_song(&name, project_dir);
        self.registry.save(&self.registry_path);
        self.heard_as_audio.clear();
        self.preview = None;
        self.status_dirty = true;
        self.claim_keeper_if_mine();
        self.announce_beings();
        self.note_open_song();
        Ok(())
    }

    // ----- PACK 2: orgs, the personal shelf, albums -----

    fn orgs_dir(&self) -> PathBuf {
        self.data_dir.join("orgs")
    }

    fn org_dir(&self, org: &SongId) -> PathBuf {
        self.orgs_dir().join(org.to_hex())
    }

    fn personal_dir(&self) -> PathBuf {
        self.data_dir.join("names").join(hex::encode(self.me().as_bytes())).join("personal")
    }

    /// Open every org under `orgs/` and the Name's personal shelf (made on
    /// first use), hold them in the node, claim their keeper role if mine.
    fn load_containers(&mut self) {
        if let Ok(rd) = std::fs::read_dir(self.orgs_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_dir() || !Song::exists(&p) {
                    continue;
                }
                match Song::open(&p) {
                    Ok(s) if s.kind() == ContainerKind::Org => {
                        let id = s.id();
                        self.node.add_container(s);
                        self.claim_container_keeper(&id);
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("could not open the org in {}: {e}", p.display()),
                }
            }
        }
        let pdir = self.personal_dir();
        let shelf = if Song::exists(&pdir) { Song::open(&pdir) } else { Song::create_kind(&pdir, &self.stamper(), "", ContainerKind::Personal, now_ms()) };
        match shelf {
            Ok(s) => {
                let id = s.id();
                self.node.add_container(s);
                self.claim_container_keeper(&id);
            }
            Err(e) => tracing::warn!("personal shelf: {e}"),
        }
        self.node.refresh_org_links();
    }

    fn claim_container_keeper(&mut self, id: &SongId) {
        let me = self.me();
        let machine = self.machine.clone();
        let who = self.stamper();
        let now = now_ms();
        let _ = self.node.with_container(id, |s| {
            let mine = match s.keeper() {
                Some((name, m)) => name == me && (m.is_empty() || m == machine),
                None => false,
            };
            let had = s.keeper_key().is_some();
            if mine && !had {
                let _ = s.set_keeper(Some(who.keeper_key()), now);
            } else if !mine && had {
                let _ = s.set_keeper(None, now);
            }
        });
    }

    fn personal_shelf(&self) -> Option<SongId> {
        self.node.containers_of(ContainerKind::Personal).into_iter().next()
    }

    fn scope_id(&self, scope: &AlbumScope) -> Option<SongId> {
        match scope {
            AlbumScope::Personal => self.personal_shelf(),
            AlbumScope::Org(id) => Some(*id),
        }
    }

    /// Orgs joined before their Invite act arrived: stamp Accept once it has.
    fn retry_pending_accepts(&mut self) {
        if self.pending_accepts.is_empty() {
            return;
        }
        let who = self.stamper();
        let now = now_ms();
        let pending: Vec<SongId> = self.pending_accepts.iter().copied().collect();
        for org in pending {
            let done = self.node.with_container(&org, |s| s.crew().is_member(&who.name) || s.accept(&who, now).is_ok()).unwrap_or(true);
            if done {
                self.pending_accepts.remove(&org);
                self.node.publish();
                self.status_dirty = true;
            }
        }
    }

    /// Keep the registry's note of the open song (id, org, access) current.
    fn note_open_song(&mut self) {
        let Some(dir) = self.song_dir.clone() else { return };
        let Some(note) = self.node.with_song(|s| SongNote { id: Some(s.id()), org: s.org(), access: Some(s.access()) }).ok() else { return };
        if self.registry.note_song(&dir, note) {
            self.registry.save(&self.registry_path);
        }
    }

    fn library_songs(&self) -> Vec<LibrarySong> {
        self.registry
            .songs
            .iter()
            .map(|(name, dir)| {
                let note = self.registry.song_note(dir).cloned().unwrap_or_default();
                LibrarySong { song: note.id, name: name.clone(), dir: dir.clone(), org: note.org, access: note.access }
            })
            .collect()
    }

    fn org_rows(&self) -> Vec<OrgRow> {
        let me = self.me();
        let machine = self.machine.clone();
        let songs = self.library_songs();
        let mut rows = Vec::new();
        for id in self.node.containers_of(ContainerKind::Org) {
            let row = self.node.with_container(&id, |s| {
                let st = s.main();
                OrgRow {
                    org: id,
                    name: st.settings.label.clone().unwrap_or_else(|| s.name().to_string()),
                    color: st.settings.color,
                    owner: s.crew().is_owner(&me),
                    member: s.crew().is_member(&me),
                    members: s.crew().members.len(),
                    archived: st.settings.archived,
                    keeper: s.keeper(),
                    i_am_keeper: s.keeper().map(|(n, m)| n == me && (m.is_empty() || m == machine)).unwrap_or(false),
                    defaults: st.settings.defaults.clone(),
                    songs: songs.iter().filter(|x| x.org == Some(id)).cloned().collect(),
                    albums: s.albums().values().filter(|a| !a.archived).count(),
                }
            });
            if let Ok(r) = row {
                rows.push(r);
            }
        }
        rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        rows
    }

    fn invite_rows(&self) -> Vec<InviteRow> {
        self.node
            .pending_invites()
            .into_iter()
            .map(|i| InviteRow { from_label: self.name_for(&i.from).unwrap_or_else(|| i.from.short_code()), from: i.from, container: i.song, name: i.name.clone(), kind: ContainerKind::from_genesis_property(i.genesis.body.property.as_deref()).unwrap_or_default() })
            .collect()
    }

    fn library(&self) -> LibraryView {
        let orgs = self.org_rows();
        let (song_org, access, ai_policy) = self
            .node
            .with_song(|s| {
                let org = s.org().map(|o| (o, orgs.iter().find(|r| r.org == o).map(|r| r.name.clone()).unwrap_or_else(|| o.short())));
                (org, s.access(), s.main().settings.ai_policy)
            })
            .unwrap_or((None, flit_core::Access::Private, flit_core::AiPolicy::Grants));
        let personal_albums = self.personal_shelf().and_then(|p| self.node.with_container(&p, |s| s.albums().values().filter(|a| !a.archived).count()).ok()).unwrap_or(0);
        LibraryView { songs: self.library_songs(), orgs, invites: self.invite_rows(), song_org, access, ai_policy, personal_albums }
    }

    fn org_member_rows(&self, org: &SongId) -> Vec<OrgMemberRow> {
        let connected = self.node.connected_peers();
        self.node
            .with_container(org, |s| {
                let crew = s.crew();
                let mut ids: Vec<(Id, bool)> = crew.members.iter().map(|id| (*id, false)).collect();
                ids.extend(crew.invited.iter().map(|id| (*id, true)));
                ids.into_iter()
                    .map(|(id, invited)| {
                        let beings: Vec<(BeingId, String, BeingKind, u32)> = s.beings().of_name(&id).into_iter().filter(|(_, b)| !b.revoked).map(|(b, info)| (b.clone(), info.display_name.clone(), info.kind, info.color)).collect();
                        let label = self.registry.friend_name(&id).or_else(|| beings.first().map(|b| b.1.clone()).filter(|l| !l.is_empty())).unwrap_or_else(|| id.short_code());
                        OrgMemberRow { id, label, owner: crew.is_owner(&id), invited, beings, connected: connected.contains(&id) }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn album_rows(&self) -> Vec<AlbumRow> {
        let me = self.me();
        let songs = self.library_songs();
        let mut scopes: Vec<(AlbumScope, SongId)> = self.personal_shelf().map(|p| (AlbumScope::Personal, p)).into_iter().collect();
        scopes.extend(self.node.containers_of(ContainerKind::Org).into_iter().map(|o| (AlbumScope::Org(o), o)));
        let mut rows = Vec::new();
        for (scope, id) in scopes {
            let _ = self.node.with_container(&id, |s| {
                let st = s.main();
                let scope_name = match scope {
                    AlbumScope::Personal => "Mine".to_string(),
                    AlbumScope::Org(_) => st.settings.label.clone().unwrap_or_else(|| s.name().to_string()),
                };
                let show_locked = st.settings.defaults.show_locked || scope == AlbumScope::Personal;
                for (album, a) in st.albums.iter() {
                    let can_curate = s.crew().is_owner(&me) || a.created_by.as_ref().map(|b| b.name == me).unwrap_or(false);
                    let song_rows: Vec<AlbumSongRow> = a
                        .songs
                        .iter()
                        .filter_map(|song| {
                            let known = songs.iter().find(|x| x.song == Some(*song));
                            let locked = known.is_none();
                            if locked && !show_locked {
                                return None;
                            }
                            Some(AlbumSongRow { song: *song, name: known.map(|k| k.name.clone()), dir: known.map(|k| k.dir.clone()), access: known.and_then(|k| k.access), locked })
                        })
                        .collect();
                    rows.push(AlbumRow { scope: scope.clone(), scope_name: scope_name.clone(), album: *album, name: a.name.clone(), description: a.description.clone(), cover: a.cover, archived: a.archived, created_by: a.created_by.clone(), can_curate, songs: song_rows });
                }
            });
        }
        rows
    }

    fn switch_name(&mut self, id: &Id) -> std::result::Result<(), String> {
        let key = self.names.key(id).map_err(|e| e.to_string())?;
        let profile = self.names.profile(id).map_err(|e| e.to_string())?;
        self.names.switch(id).map_err(|e| e.to_string())?;
        let being = profile.active().map(|b| b.id.0.clone()).unwrap_or_else(|| "me".into());
        let who = Stamper::new(key, &being, self.live.host(), VERSION, &self.machine);
        self.profile = profile;
        self.node.set_stamper(who.clone());
        self.live.set_stamper(who);
        // the network identity is the Name key: a switch takes effect for
        // songs opened from now on; the node keeps its endpoint
        self.node.close_song();
        self.song_dir = None;
        self.warnings.push("Switched Name. Open a song to continue; the network identity follows on the next agent start.".into());
        self.status_dirty = true;
        Ok(())
    }

    /// Add my track's block: capture for the next save, and encode for streams.
    fn track_block(&mut self, track: &TrackId, pos: SongPos, pcm: &[f32], sample_rate: u32, channels: u8) -> Vec<(Id, StreamFrame)> {
        let cap = self.capture.entry(track.clone()).or_default();
        cap.sample_rate = sample_rate;
        cap.channels = channels;
        let frames = pcm.len() / channels.max(1) as usize;
        let tempo = self.node.with_song(|s| s.state(&self.live.history()).map(|st| st.settings.tempo).unwrap_or(120.0)).unwrap_or(120.0);
        let contiguous = cap.last_pos.map(|lp| (pos.ticks - lp.ticks).abs() <= TICKS_PER_BEAT / 8).unwrap_or(false);
        if !contiguous || cap.segments.is_empty() {
            cap.segments.push((pos, Vec::new()));
        }
        let seg = cap.segments.last_mut().unwrap();
        if seg.1.len() + pcm.len() <= MAX_CAPTURE_SAMPLES {
            seg.1.extend_from_slice(pcm);
        }
        let advance = SongPos::from_beats(frames as f64 / sample_rate as f64 * tempo / 60.0);
        cap.last_pos = Some(SongPos::from_ticks(pos.ticks + advance.ticks));
        self.live.push_dirty(track.clone());

        // stream out
        let peers = self.node.connected_peers();
        if peers.is_empty() {
            return vec![];
        }
        let bitrate = self.registry.monitoring_bitrate.unwrap_or(128_000);
        let kind = if self.registry.submix { StreamKind::Submix } else { StreamKind::InSong { track: track.clone() } };
        let key = if self.registry.submix { SUBMIX_KEY.to_string() } else { track.0.clone() };
        let out = self.outgoing.entry(key).or_insert_with(|| {
            let cfg = StreamConfig { kind, channels, bitrate, low_delay: false, sample_rate };
            Outgoing { enc: StreamEncoder::new(cfg).expect("opus encoder"), streams: HashMap::new() }
        });
        let packets = out.enc.push(pcm).unwrap_or_default();
        let mut frames_out = Vec::new();
        for p in packets {
            for (peer, stream) in &out.streams {
                frames_out.push((*peer, StreamFrame { stream: *stream, packet: p.clone() }));
            }
        }
        frames_out
    }

    /// Streams that still need opening on some peer.
    fn streams_to_open(&self) -> Vec<(Id, String, StreamConfig)> {
        let peers = self.node.connected_peers();
        let mut v = Vec::new();
        for (key, out) in &self.outgoing {
            for p in &peers {
                if !out.streams.contains_key(p) {
                    v.push((*p, key.clone(), out.enc.config().clone()));
                }
            }
        }
        v
    }

    fn save_version(&mut self, name: &str) -> std::result::Result<Vec<Act>, String> {
        let dir = self.song_dir.clone().ok_or("no song")?;
        // level 0: captured track audio becomes the render
        let captures: Vec<(TrackId, Capture)> = self.capture.drain().collect();
        for (track, cap) in captures {
            for (start, samples) in cap.segments {
                if samples.is_empty() {
                    continue;
                }
                self.live.push_captured(track.clone(), RenderedAudio { sample_rate: cap.sample_rate, channels: cap.channels, start, samples, print_point: PrintPoint::PostFaderPreSend });
            }
        }
        // A host with a bridge saves its own project: the Version (and, at
        // level 2, the notes) land when it reports the written file.
        if self.live.bridge.request_save() == ApplyOutcome::Applied {
            self.live.next_version_name = Some(name.to_string());
            self.status_dirty = true;
            return Ok(vec![]);
        }
        let state_path = dir.join(".flit").join("plugin-state.json");
        let _ = std::fs::write(&state_path, serde_json::to_string(&serde_json::json!({ "saved_at": now_ms(), "name": name })).unwrap());
        let now = now_ms();
        let pos = self.last_pos;
        let acts = self.node.with_song(|s| self.live.save_version(s, name, &state_path, now, pos)).map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        self.node.publish();
        self.status_dirty = true;
        Ok(acts)
    }

    fn timeline(&self, history: &HistoryId, lens: &Lens, show_presence: bool) -> Vec<Row> {
        self.node.with_song(|s| s.rows(history, lens, show_presence)).unwrap_or_default()
    }

    /// Keep the open song and the DAW's open project together. The song of
    /// a project lives in the project's folder, so: a project whose folder
    /// holds a song opens that song; a project without one is offered as a
    /// new song; a song from another folder than the open project pauses
    /// live edits until the user picks. Returns notices for the panel.
    fn auto_link(&mut self) -> Vec<String> {
        let project = self.live.bridge.current_project();
        let open_dir = self.song_dir.clone();
        let mut notices = Vec::new();
        let Some(project) = project else {
            if self.live.bridge.link_up() == Some(true) {
                self.link = ProjectLink::Unknown;
            }
            self.paused = None;
            return notices;
        };
        let changed = self.linked_project.as_ref() != Some(&project);
        self.linked_project = Some(project.clone());
        let Some(dir) = project.parent().map(|d| d.to_path_buf()) else { return notices };
        let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let has_song = Song::exists(dir.join(".flit"));
        let same_folder = open_dir.as_ref().map(|d| same_dir(d, &dir)).unwrap_or(false);
        if has_song && !same_folder {
            match self.open_song_dir(&dir) {
                Ok(()) => {
                    self.last_link_error = None;
                    let name = self.node.with_song(|s| s.name().to_string()).unwrap_or_default();
                    notices.push(format!("Opened the song \"{name}\" for {file}"));
                }
                Err(e) => {
                    let msg = format!("Could not open the song in {}: {e}", dir.display());
                    if self.last_link_error.as_ref() != Some(&msg) {
                        self.last_link_error = Some(msg.clone());
                        notices.push(msg);
                    }
                }
            }
        }
        let same_folder = self.song_dir.as_ref().map(|d| same_dir(d, &dir)).unwrap_or(false);
        let (link, paused) = if same_folder {
            let from = self.node.with_song(|s| s.project_file().map(str::to_string)).ok().flatten();
            match from {
                None => {
                    let _ = self.node.with_song(|s| s.set_project_file(&file));
                    (ProjectLink::Linked { project: project.clone() }, None)
                }
                Some(f) if f.eq_ignore_ascii_case(&file) => (ProjectLink::Linked { project: project.clone() }, None),
                Some(f) => (ProjectLink::OtherFile { project: project.clone(), song_from: f.clone() }, Some(format!("The song in this folder was made from {f}, but {file} is open. Save it as {f}, or make {file} its own song in another folder."))),
            }
        } else if let Some(song_dir) = &self.song_dir {
            let name = self.node.with_song(|s| s.name().to_string()).unwrap_or_default();
            (
                ProjectLink::Mismatch { project: project.clone(), song_dir: song_dir.clone(), song_name: name.clone() },
                Some(format!("The DAW has {file} open, but the open song \"{name}\" belongs to {}. Edits are paused until you make {file} a song or open its folder's song.", song_dir.display())),
            )
        } else {
            (ProjectLink::Unlinked { project: project.clone() }, None)
        };
        if changed {
            if let Some(p) = &paused {
                notices.push(p.clone());
            } else if matches!(link, ProjectLink::Unlinked { .. }) {
                notices.push(format!("{file} is not a song yet. Make it one from the Home screen."));
            }
        }
        self.link = link;
        self.paused = paused;
        self.status_dirty = true;
        notices
    }

    /// Show a moment in the DAW: a diff against what the DAW has now (never
    /// a rebuild); a big jump loads the nearest Version's project file
    /// first when the bridge can. Returns notices.
    fn show_state(&mut self, at: Option<ActId>) -> Vec<String> {
        let history = self.history();
        let state = match at {
            Some(a) => self.node.with_song(|s| s.state_at(&a)).ok().and_then(|r| r.ok()),
            None => self.node.with_song(|s| s.state(&history).cloned()).ok().flatten(),
        };
        let Some(state) = state else { return vec![] };
        let (snapshot_note, snapshot) = match at {
            Some(a) => self.export_version_at(&a),
            None => self.node.with_song(|s| s.latest(&history)).ok().flatten().map(|l| self.export_version_at(&l)).unwrap_or((None, None)),
        };
        let r = Receiver::apply_state(&mut self.live.bridge, &state, snapshot.as_deref());
        tracing::info!("showed moment {:?}: {} settings, {} mixer changes, {} devices, tempo {}, snapshot {}", at, r.settings, r.mixer, r.devices, r.tempo, r.snapshot_loaded);
        let mut notices = Vec::new();
        if !r.manual.is_empty() {
            notices.push(format!("{} can't add or remove plugins from outside. To match this moment: {}", self.live.host(), r.manual.join("; ")));
        }
        if !r.snapshot_loaded {
            if let Some(n) = snapshot_note {
                notices.push(n);
            }
        }
        for t in r.heard {
            if self.heard_as_audio.insert(t.clone()) {
                let name = self.live.track_name(&t).map(str::to_string).unwrap_or(t.0.clone());
                notices.push(format!("Hearing {name} as audio (this host can't reproduce it)"));
            }
        }
        self.status_dirty = true;
        notices
    }

    /// The DAW follows History: after Go back, Undo, Merge or a history
    /// switch, push the history's sound into the host and re-prime the
    /// settle rule. Returns notices for the panel.
    fn follow_history(&mut self) -> Vec<String> {
        self.preview = None;
        let notices = self.show_state(None);
        self.node.with_song(|s| self.live.prime_from(s)).ok();
        self.tint_presence(None);
        notices
    }

    /// The project file this host had at moment `at` (its nearest Version
    /// at or before it), written beside the song: the bridge loads it on a
    /// big jump; else the user can open it by hand for notes and
    /// arrangement. Returns (notice, path).
    fn export_version_at(&self, at: &ActId) -> (Option<String>, Option<PathBuf>) {
        let host = self.live.host();
        let Some(dir) = self.song_dir.clone() else { return (None, None) };
        let Some(plan) = self.node.with_song(|s| s.restore_plan(at, &host)).ok().and_then(|r| r.ok()) else { return (None, None) };
        let Some(v) = plan.version else { return (None, None) };
        let Payload::Version(vp) = v.payload() else { return (None, None) };
        let Some(bytes) = self.node.with_song(|s| s.blobs().get(&vp.project)).ok().and_then(|r| r.ok()) else { return (None, None) };
        if bytes.is_empty() {
            return (None, None);
        }
        let ext = self.live.bridge.current_project().and_then(|p| p.extension().map(|e| e.to_string_lossy().into_owned())).unwrap_or_else(|| "project".into());
        let safe: String = vp.name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') { c } else { '_' }).collect();
        let out = dir.join(".flit").join("restore");
        if std::fs::create_dir_all(&out).is_err() {
            return (None, None);
        }
        let path = out.join(format!("{} {}.{}", safe.trim(), v.id.short(), ext));
        if std::fs::write(&path, bytes).is_err() {
            return (None, None);
        }
        let current = self.node.with_song(|s| s.state(&self.history()).and_then(|st| st.latest_version.get(&host).copied())).ok().flatten();
        let note = if current == Some(v.id) { None } else { Some(format!("Knobs and faders are back. For notes and arrangement, open the project file from \"{}\": {}", vp.name, path.display())) };
        (note, Some(path))
    }

    /// Where every other being is, for the panel and (opt-in) the DAW.
    fn presence_rows(&self) -> Vec<BeingRow> {
        self.node.with_song(|s| self.being_rows(s)).unwrap_or_default()
    }

    /// Tint the DAW with other beings' colors where they are (at a moment,
    /// or now). Only when the bridge can and the setting is on.
    fn tint_presence(&mut self, at: Option<ActId>) {
        if !self.registry.show_presence_in_daw || !self.live.bridge.capabilities().show_presence {
            return;
        }
        let me = self.being();
        let history = self.history();
        let shown: Vec<Shown> = self
            .node
            .with_song(|s| {
                let state = match at {
                    Some(a) => s.state_at(&a).ok(),
                    None => s.state(&history).cloned(),
                };
                let Some(st) = state else { return vec![] };
                st.presence
                    .iter()
                    .filter(|(b, _)| **b != me)
                    .map(|(b, p)| Shown { being: b.clone(), label: s.being_label(b), color: s.being_color(b).unwrap_or(0x9AA5B1), presence: PresencePayload { view: p.view, track: p.track.clone(), device: p.device, region: p.region.clone() } })
                    .collect()
            })
            .unwrap_or_default();
        let key: Vec<(BeingRef, PresencePayload)> = shown.iter().map(|s| (s.being.clone(), s.presence.clone())).collect();
        if key == self.last_shown {
            return;
        }
        self.last_shown = key;
        if shown.is_empty() {
            self.live.bridge.clear_presence();
        } else {
            let _ = self.live.bridge.show_presence(&shown);
        }
    }

    /// Navigate my DAW to where a being is.
    fn follow(&mut self, being: &BeingRef) -> Option<String> {
        let history = self.history();
        let p = self.node.with_song(|s| s.state(&history).and_then(|st| st.presence.get(being)).map(|p| PresencePayload { view: p.view, track: p.track.clone(), device: p.device, region: p.region.clone() })).ok().flatten();
        let Some(p) = p else { return Some("No presence known for that being yet".into()) };
        match self.live.bridge.follow(&p) {
            ApplyOutcome::Applied => None,
            _ => Some("This host can't be navigated from here".into()),
        }
    }
}

impl Agent {
    pub async fn start(cfg: AgentConfig) -> anyhow::Result<Agent> {
        let registry_path = cfg.data_dir.join("registry.json");
        let registry = Registry::load(&registry_path);
        let settings = UserSettings::load(cfg.data_dir.join("settings.json")).unwrap_or_default();
        let blocklist = BlockList::load(cfg.data_dir.join("blocked.txt")).unwrap_or_default();
        let machine = machine_id(&cfg.data_dir);
        let names = NameStore::open(&cfg.data_dir)?;

        // The Name: a test seed, the current Name, an old identity file
        // (migrated once), or a fresh Name with one human being.
        let first_being = BeingProfile { id: BeingId::new(&slug(&cfg.display_name)), kind: BeingKind::Human, display_name: cfg.display_name.clone(), color: BEING_COLORS[0], delegated_key: None };
        let (key, profile) = match cfg.seed {
            Some(seed) => {
                let k = Keypair::from_seed(seed);
                let p = names.adopt(&k, &cfg.display_name, vec![first_being.clone()])?;
                (k, p)
            }
            None => match names.current()? {
                Some(p) => (names.key(&p.id)?, p),
                None => {
                    let old_path = cfg.data_dir.join("identity.key");
                    let old = old_path.is_file().then(|| flit_net::IdentityStore::at(old_path).load().ok().flatten()).flatten();
                    match old {
                        Some(k) => {
                            tracing::info!("adopted the existing identity as a Name (the panel's first-run walk names it)");
                            let p = names.adopt(&k, "", vec![])?;
                            (k, p)
                        }
                        None => {
                            tracing::info!("first run: made a key; the panel's first-run walk names it and makes the first being");
                            names.create("", None)?
                        }
                    }
                }
            },
        };
        let being = profile.active().map(|b| b.id.0.clone()).unwrap_or_else(|| "me".into());
        let host = cfg.bridge.tag();
        let who = Stamper::new(key.clone(), &being, host, VERSION, &machine);

        let mut ncfg = NodeConfig::new(key.clone(), if profile.label.trim().is_empty() { &cfg.display_name } else { profile.label.trim() });
        ncfg.local_only = cfg.local_only;
        ncfg.relay_only = cfg.relay_only || registry.relay_only;
        ncfg.settings = settings.clone();
        let node = Node::bind(ncfg).await?;
        node.set_stamper(who.clone());
        node.set_blocklist(blocklist);
        for f in &registry.friends {
            if let Some(a) = &f.addr {
                node.learn(a);
            }
        }
        let live = LiveLoop::new(cfg.bridge, FlitChunkEncoder::default(), who, main_history());
        let mut state = State {
            last_link: None,
            last_link_error: None,
            window_hook: cfg.window_hook.clone(),
            node,
            live,
            names,
            profile,
            machine,
            data_dir: cfg.data_dir.clone(),
            registry,
            registry_path,
            settings,
            clients: HashMap::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            capture: HashMap::new(),
            heard_as_audio: HashSet::new(),
            talk_record: None,
            warnings: Vec::new(),
            master_warned: false,
            status_dirty: true,
            presence_dirty: true,
            song_dir: None,
            last_pos: SongPos::START,
            preview: None,
            linked_project: None,
            link: ProjectLink::Unknown,
            paused: None,
            following: None,
            last_presence: Vec::new(),
            last_shown: Vec::new(),
            update_available: None,
            pending_accepts: HashSet::new(),
            shutdown: false,
        };
        state.load_containers();
        if let Some(dir) = cfg.open_song.or_else(|| state.registry.last_song.clone()) {
            if let Err(e) = state.open_song_dir(&dir) {
                tracing::warn!("could not reopen {}: {e}", dir.display());
            }
        }
        let agent = Agent { state: Arc::new(Mutex::new(state)), handle: Arc::new(Mutex::new(None)) };
        agent.spawn_loops();
        Ok(agent)
    }

    pub fn set_handle(&self, h: ServerHandle) {
        *self.handle.lock().unwrap() = Some(h);
    }

    fn push_all(&self, p: Push) {
        if let Some(h) = self.handle.lock().unwrap().as_ref() {
            h.broadcast(p);
        }
    }

    fn push_where(&self, p: Push, pred: impl FnMut(&ClientKind) -> bool) {
        if let Some(h) = self.handle.lock().unwrap().as_ref() {
            h.push_where(p, pred);
        }
    }

    fn spawn_loops(&self) {
        // 1. tick: bridge -> settle -> acts -> peers; status and presence pushes
        let me = self.clone();
        tokio::spawn(async move {
            let mut last_status = std::time::Instant::now();
            let mut last_connect = std::time::Instant::now();
            let mut last_link_check = std::time::Instant::now();
            let mut last_presence = std::time::Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let mut pending_pushes: Vec<Push> = Vec::new();
                let (acts, opens, dirty, presence, done) = {
                    let mut st = me.state.lock().unwrap();
                    if st.shutdown {
                        (vec![], vec![], false, None, true)
                    } else {
                        let now = now_ms();
                        let st = &mut *st;
                        // mismatch safeguard: a paused agent reads the host but stamps nothing
                        let acts = if st.paused.is_some() {
                            let _ = st.live.bridge.poll();
                            vec![]
                        } else {
                            match st.node.with_song(|s| st.live.tick(s, now)) {
                                Ok(Ok(acts)) => acts,
                                Ok(Err(e)) => {
                                    st.warnings.push(format!("live: {e}"));
                                    vec![]
                                }
                                Err(_) => {
                                    // No song open: still drain the bridge so the link
                                    // comes up and the panel can say so.
                                    let _ = st.live.bridge.poll();
                                    vec![]
                                }
                            }
                        };
                        for n in std::mem::take(&mut st.live.notices) {
                            st.warnings.push(n.clone());
                            pending_pushes.push(Push::Warning(n));
                        }
                        if last_link_check.elapsed() > Duration::from_millis(1000) {
                            last_link_check = std::time::Instant::now();
                            pending_pushes.extend(st.auto_link().into_iter().map(Push::Warning));
                            // my beings land in the song once the crew knows me
                            st.announce_beings();
                            st.retry_pending_accepts();
                            st.note_open_song();
                        }
                        if !acts.is_empty() {
                            st.node.publish();
                            st.status_dirty |= acts.iter().any(|e| !matches!(e.kind(), Kind::Setting | Kind::Mixer | Kind::Presence));
                            st.presence_dirty |= acts.iter().any(|e| e.kind() == Kind::Presence);
                        }
                        pending_pushes.extend(st.drain_notices());
                        let link = st.live.bridge.link_up();
                        if link != st.last_link {
                            st.last_link = link;
                            st.status_dirty = true;
                            tracing::info!("bridge: {}", st.live.bridge.describe());
                        }
                        let opens = st.streams_to_open();
                        let dirty = st.status_dirty && last_status.elapsed() > Duration::from_millis(300);
                        if dirty {
                            st.status_dirty = false;
                        }
                        let presence = if last_presence.elapsed() > Duration::from_millis(500) {
                            last_presence = std::time::Instant::now();
                            let rows = st.presence_rows();
                            if rows != st.last_presence || st.presence_dirty {
                                st.presence_dirty = false;
                                st.last_presence = rows.clone();
                                if st.preview.is_none() {
                                    st.tint_presence(None);
                                }
                                Some(rows)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        (acts, opens, dirty, presence, false)
                    }
                };
                if done {
                    break;
                }
                for p in pending_pushes.drain(..) {
                    me.push_all(p);
                }
                for a in &acts {
                    if a.kind() == Kind::Presence {
                        continue;
                    }
                    let row = {
                        let st = me.state.lock().unwrap();
                        let h = st.history();
                        st.timeline(&h, &Lens::Song, false).into_iter().find(|r| r.act == a.id)
                    };
                    if let Some(r) = row {
                        me.push_all(Push::Row(r));
                    }
                }
                for (peer, key, cfg) in opens {
                    let res = me.open_stream(peer, cfg).await;
                    let mut st = me.state.lock().unwrap();
                    if let Some(out) = st.outgoing.get_mut(&key) {
                        match res {
                            Ok(n) => {
                                out.streams.insert(peer, n);
                            }
                            Err(e) => tracing::debug!("open stream to {peer:?}: {e}"),
                        }
                    }
                }
                if dirty {
                    last_status = std::time::Instant::now();
                    let s = me.state.lock().unwrap().status();
                    me.push_all(Push::Status(s));
                }
                if let Some(rows) = presence {
                    me.push_all(Push::Presence(rows));
                }
                if last_connect.elapsed() > Duration::from_secs(5) {
                    last_connect = std::time::Instant::now();
                    me.reconnect().await;
                }
            }
        });

        // 2. network events
        let me = self.clone();
        tokio::spawn(async move {
            let mut rx = me.state.lock().unwrap().node.subscribe();
            loop {
                let ev = match rx.recv().await {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                me.on_net_event(ev);
            }
        });
    }

    async fn open_stream(&self, peer: Id, cfg: StreamConfig) -> flit_net::Result<u16> {
        let node = self.state.lock().unwrap().node.handle();
        node.open_stream(peer, cfg).await
    }

    async fn reconnect(&self) {
        let node = self.state.lock().unwrap().node.handle();
        let n = node.connect_crew().await;
        if n > 0 {
            let mut st = self.state.lock().unwrap();
            st.status_dirty = true;
            st.node.catch_up();
        }
    }

    fn on_net_event(&self, ev: NetEvent) {
        match ev {
            NetEvent::Imported { from, ids } | NetEvent::Sealed { from, ids } => {
                let (rows, pushes) = {
                    let mut st = self.state.lock().unwrap();
                    let st = &mut *st;
                    let mut rows = Vec::new();
                    let mut pushes: Vec<Push> = Vec::new();
                    let history = st.history();
                    let me = st.being();
                    let mut follow = false;
                    let mut keeper_changed = false;
                    let mut crew_changed = false;
                    let mut follow_being: Option<BeingRef> = None;
                    for id in &ids {
                        let act = st.node.with_song(|s| s.chains().get(id).cloned()).ok().flatten();
                        let Some(act) = act else { continue };
                        if act.being() == me {
                            // my own act sealed: nothing to apply
                            continue;
                        }
                        let in_my_history = st.node.with_song(|s| s.chains().history_of(id)).ok().flatten() == Some(history);
                        if in_my_history {
                            let decision = st.node.with_song(|s| Receiver::apply(&mut st.live.bridge, s, id)).ok().flatten();
                            if let Some(ReceiverDecision::HearAsAudio(t)) = decision {
                                if st.heard_as_audio.insert(t.clone()) {
                                    let name = st.live.track_name(&t).map(str::to_string).unwrap_or(t.0.clone());
                                    pushes.push(Push::Warning(format!("Hearing {name} as audio (this host can't reproduce it)")));
                                }
                            }
                            follow |= matches!(act.kind(), Kind::GoBack | Kind::Undo | Kind::Merge);
                        }
                        match act.kind() {
                            Kind::Keeper => keeper_changed = true,
                            Kind::Crew => crew_changed = true,
                            Kind::Presence => {
                                st.presence_dirty = true;
                                if st.following.as_ref() == Some(&act.being()) {
                                    follow_being = Some(act.being());
                                }
                            }
                            _ => {}
                        }
                        if let Some(r) = st.timeline(&history, &Lens::Song, false).into_iter().find(|r| r.act == *id) {
                            rows.push(r);
                        }
                    }
                    if keeper_changed {
                        st.claim_keeper_if_mine();
                    }
                    if crew_changed {
                        st.announce_beings();
                    }
                    if follow {
                        pushes.extend(st.follow_history().into_iter().map(Push::Warning));
                    }
                    if let Some(b) = follow_being {
                        st.follow(&b);
                    }
                    pushes.extend(st.drain_notices());
                    st.status_dirty = true;
                    let _ = from;
                    (rows, pushes)
                };
                for r in rows {
                    if r.kind == Kind::Chat {
                        let chat = self.state.lock().unwrap().chat_rows().into_iter().find(|c| c.id == r.act);
                        if let Some(c) = chat {
                            self.push_all(Push::Chat(c));
                        }
                    }
                    self.push_all(Push::Row(r));
                }
                for p in pushes {
                    self.push_all(p);
                }
            }
            NetEvent::Rejected { from, reason } => {
                tracing::warn!("rejected from {from:?}: {reason}");
                self.push_all(Push::Rejected { from, reason });
            }
            NetEvent::JoinRequest { from, display_name } => {
                self.state.lock().unwrap().status_dirty = true;
                self.push_all(Push::JoinRequest { id: from, name: display_name });
            }
            NetEvent::Invite { from, song, name, kind } => {
                self.state.lock().unwrap().status_dirty = true;
                self.push_all(Push::Invite { from, song, name, kind });
            }
            NetEvent::PeerUp(_) => {
                let mut st = self.state.lock().unwrap();
                st.status_dirty = true;
                st.presence_dirty = true;
                st.node.catch_up();
            }
            NetEvent::PeerDown(_) | NetEvent::Joined { .. } | NetEvent::BlobFetched { .. } => {
                let mut st = self.state.lock().unwrap();
                st.status_dirty = true;
                st.presence_dirty = true;
            }
            NetEvent::QuotaExceeded(id) => {
                let mut st = self.state.lock().unwrap();
                st.warnings.push(format!("{} is over quota; not accepting more from them", id.short_code()));
                st.status_dirty = true;
            }
            NetEvent::BlobFailed { .. } => {}
            NetEvent::StreamOpened { from, stream, config } => {
                let mut st = self.state.lock().unwrap();
                let rate = st.clients.values().find(|c| matches!(c.kind, ClientKind::Receiver { .. } | ClientKind::Panel)).map(|c| c.sample_rate).unwrap_or(48_000);
                let mut cfg = config.clone();
                cfg.sample_rate = rate;
                if let Ok(dec) = StreamDecoder::new(cfg) {
                    st.incoming.insert((from, stream), (config.kind, dec));
                }
            }
            NetEvent::StreamClosed { from, stream } => {
                self.state.lock().unwrap().incoming.remove(&(from, stream));
            }
            NetEvent::StreamFrame { from, frame } => {
                let decoded = {
                    let mut st = self.state.lock().unwrap();
                    let Some((kind, dec)) = st.incoming.get_mut(&(from, frame.stream)) else { return };
                    let kind = kind.clone();
                    match dec.decode(&frame.packet) {
                        Ok(pcm) => Some((kind, pcm)),
                        Err(_) => None,
                    }
                };
                let Some((kind, pcm)) = decoded else { return };
                match kind {
                    StreamKind::InSong { track } => {
                        let t2 = track.clone();
                        self.push_where(Push::TrackAudio { track, pcm }, move |k| matches!(k, ClientKind::Receiver { track } if *track == t2));
                    }
                    StreamKind::Submix => {
                        self.push_where(Push::TrackAudio { track: TrackId::new(SUBMIX_KEY), pcm }, |k| matches!(k, ClientKind::Receiver { .. }));
                    }
                    StreamKind::ReferenceMix => {
                        let hear = self.state.lock().unwrap().registry.hear_reference;
                        if hear {
                            self.push_where(Push::TrackAudio { track: TrackId::new(REFERENCE_KEY), pcm }, |k| matches!(k, ClientKind::Receiver { track } if track.0.eq_ignore_ascii_case("master")));
                        }
                    }
                    StreamKind::Talk { name } => {
                        {
                            let mut st = self.state.lock().unwrap();
                            if let Some((_, who, buf, _, _, _)) = st.talk_record.as_mut() {
                                if *who == from && buf.len() < MAX_CAPTURE_SAMPLES {
                                    buf.extend_from_slice(&pcm);
                                }
                            }
                        }
                        self.push_where(Push::TalkAudio { from, name, pcm }, move |k| matches!(k, ClientKind::TalkReceive { from: None }) || matches!(k, ClientKind::TalkReceive { from: Some(f) } if *f == from));
                    }
                    StreamKind::Voice => {
                        self.push_where(Push::TalkAudio { from, name: "voice".into(), pcm }, |k| matches!(k, ClientKind::TalkReceive { .. }));
                    }
                }
            }
        }
    }

    fn err<T: ToString>(e: T) -> Response {
        Response::Error(e.to_string())
    }
}

impl Handler for Agent {
    async fn on_hello(&self, conn: ConnId, kind: ClientKind, sample_rate: u32, channels: u8) {
        let mut st = self.state.lock().unwrap();
        if let ClientKind::Track { track } = &kind {
            if let AnyBridge::None(nb) = &mut st.live.bridge {
                if !nb.tracks.contains(track) {
                    nb.tracks.push(track.clone());
                }
            }
        }
        st.clients.insert(conn, ClientInfo { kind, sample_rate, channels });
        st.status_dirty = true;
    }

    async fn on_disconnect(&self, conn: ConnId) {
        let mut st = self.state.lock().unwrap();
        st.clients.remove(&conn);
        st.status_dirty = true;
    }

    async fn on_request(&self, conn: ConnId, req: Request) -> Response {
        // Requests that must await the network are handled first, with the
        // node handle cloned out of the lock.
        match &req {
            Request::JoinWithCode { code, dir } => {
                let Some(code) = flit_net::SongCode::decode(code) else { return Response::Error("that is not a song code".into()) };
                let node = self.state.lock().unwrap().node.handle();
                let flit = dir.join(".flit");
                return match node.request_join(&code, &flit, Duration::from_secs(120)).await {
                    Ok(flit_net::proto::JoinReply::Welcome { name, .. }) => {
                        let mut st = self.state.lock().unwrap();
                        st.song_dir = Some(dir.clone());
                        st.live.set_history(main_history());
                        st.registry.remember_song(&name, dir);
                        st.registry.save(&st.registry_path);
                        st.status_dirty = true;
                        st.announce_beings();
                        Response::Ok
                    }
                    Ok(flit_net::proto::JoinReply::Pending) => Response::Error("the owner has not answered yet; try again later".into()),
                    Ok(flit_net::proto::JoinReply::Refused(r)) => Response::Error(format!("refused: {r}")),
                    Err(e) => Self::err(e),
                };
            }
            Request::AcceptInvite { song, dir } => {
                let node = self.state.lock().unwrap().node.handle();
                let flit = dir.join(".flit");
                return match node.accept_invite(*song, &flit).await {
                    Ok(()) => {
                        let mut st = self.state.lock().unwrap();
                        let name = st.node.with_song(|s| s.name().to_string()).unwrap_or_default();
                        st.song_dir = Some(dir.clone());
                        st.live.set_history(main_history());
                        st.registry.remember_song(&name, dir);
                        st.registry.save(&st.registry_path);
                        st.status_dirty = true;
                        st.announce_beings();
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                };
            }
            Request::Invite { friend } => {
                let (node, addr) = {
                    let st = self.state.lock().unwrap();
                    let addr = st.registry.friends.iter().find(|f| f.id == *friend).and_then(|f| f.addr.clone()).unwrap_or(AddrInfo { id: *friend, ips: vec![], relay: None });
                    (st.node.handle(), addr)
                };
                return match node.invite(&addr, false).await {
                    Ok(_) => {
                        self.state.lock().unwrap().status_dirty = true;
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                };
            }
            Request::OrgInvite { org, friend } => {
                let Some(mut addr) = AddrInfo::decode(friend) else { return Response::Error("that is not a Studio ID".into()) };
                let node = {
                    let mut st = self.state.lock().unwrap();
                    if addr.ips.is_empty() && addr.relay.is_none() {
                        if let Some(known) = st.registry.friends.iter().find(|f| f.id == addr.id).and_then(|f| f.addr.clone()) {
                            addr = known;
                        }
                    } else {
                        st.node.learn(&addr);
                        if st.registry.friend_name(&addr.id).is_none() {
                            st.registry.add_friend(addr.id, "", Some(addr.clone()));
                            st.registry.save(&st.registry_path);
                        }
                    }
                    st.node.handle()
                };
                return match node.invite_to(org, &addr, false).await {
                    Ok(_) => {
                        self.state.lock().unwrap().status_dirty = true;
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                };
            }
            Request::AcceptOrgInvite { org } => {
                let (node, dir) = {
                    let st = self.state.lock().unwrap();
                    (st.node.handle(), st.org_dir(org))
                };
                let already = node.containers_of(ContainerKind::Org).contains(org);
                if !already {
                    if let Err(e) = node.accept_invite_container(*org, &dir).await {
                        return Self::err(e);
                    }
                }
                let mut st = self.state.lock().unwrap();
                let who = st.stamper();
                let accepted = st.node.with_container(org, |s| s.crew().is_member(&who.name) || s.accept(&who, now_ms()).is_ok()).unwrap_or(false);
                if accepted {
                    st.pending_accepts.remove(org);
                } else {
                    // the Invite act is still on its way: accept when it lands
                    st.pending_accepts.insert(*org);
                }
                st.node.publish();
                st.status_dirty = true;
                return Response::Ok;
            }
            Request::Merge { from_history, tracks, choices } => {
                let (node, mut preview, peers) = {
                    let st = self.state.lock().unwrap();
                    let to = st.history();
                    let p = st.node.with_song(|s| s.merge_preview(to, s, *from_history, tracks, None));
                    match p {
                        Ok(Ok(p)) => (st.node.handle(), p, st.node.connected_peers()),
                        Ok(Err(e)) => return Self::err(e),
                        Err(e) => return Self::err(e),
                    }
                };
                for (thing, property, keep_from) in choices {
                    preview.set_choice(thing, property, *keep_from);
                }
                // A peer MUST pull the FLAC before a Merge.
                if !preview.flac_needed.is_empty() {
                    for p in &peers {
                        let _ = node.fetch_blobs(*p, preview.flac_needed.clone()).await;
                    }
                }
                let mut st = self.state.lock().unwrap();
                let who = st.stamper();
                return match st.node.with_song(|s| s.merge(&who, &preview, now_ms())) {
                    Ok(Ok(act)) => {
                        st.node.publish();
                        for n in st.follow_history() {
                            self.push_all(Push::Warning(n));
                        }
                        Response::Stamped(act.id)
                    }
                    Ok(Err(e)) => Self::err(e),
                    Err(e) => Self::err(e),
                };
            }
            Request::CheckUpdates => {
                let (url, channel, data_dir) = {
                    let st = self.state.lock().unwrap();
                    (st.registry.update_manifest_url.clone(), st.registry.update_channel.clone(), st.data_dir.clone())
                };
                let info = tokio::task::spawn_blocking(move || {
                    let installed = crate::updates::installed_versions(&data_dir);
                    crate::updates::check(&url, &channel, VERSION, &installed)
                })
                .await
                .unwrap_or_else(|e| UpdateInfo { current: VERSION.into(), newest: None, channel: String::new(), releases: vec![], message: e.to_string() });
                let mut st = self.state.lock().unwrap();
                st.update_available = info.newest.clone();
                st.status_dirty = true;
                return Response::Updates(info);
            }
            _ => {}
        }

        let mut st = self.state.lock().unwrap();
        let st = &mut *st;
        let who = st.stamper();
        let history = st.history();
        let now = now_ms();
        let me = who.name;

        macro_rules! song {
            ($f:expr) => {
                match st.node.with_song($f) {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return Self::err(e),
                    Err(e) => return Self::err(e),
                }
            };
        }
        macro_rules! cont {
            ($id:expr, $f:expr) => {
                match st.node.with_container(&$id, $f) {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return Self::err(e),
                    Err(e) => return Self::err(e),
                }
            };
        }
        macro_rules! stamp {
            ($f:expr) => {{
                let act: Act = song!($f);
                st.node.publish();
                st.status_dirty = true;
                for p in st.drain_notices() {
                    self.push_all(p);
                }
                Response::Stamped(act.id)
            }};
        }

        match req {
            Request::Hello { .. } => Response::Ok,
            Request::Ping => Response::Pong,
            Request::OpenWindow { song, spawn } => {
                let windows = self.handle.lock().unwrap().as_ref().map(|h| h.clients().iter().filter(|(_, k)| matches!(k, Some(ClientKind::Window))).count()).unwrap_or(0);
                if windows > 0 {
                    self.push_where(Push::OpenWindow { song }, |k| matches!(k, ClientKind::Window));
                    return Response::Ok;
                }
                if let Some(hook) = st.window_hook.clone() {
                    hook(song);
                    return Response::Ok;
                }
                if spawn {
                    return match crate::cli::spawn_app(song) {
                        Ok(()) => Response::Ok,
                        Err(e) => Self::err(e),
                    };
                }
                Response::Error("no FLit window".into())
            }
            Request::Status => Response::Status(st.status()),
            Request::Shutdown => {
                st.shutdown = true;
                Response::Ok
            }
            Request::NewSong { name, dir } => {
                let flit = dir.join(".flit");
                if Song::exists(&flit) {
                    return Response::Error("this folder already holds a song".into());
                }
                match Song::create(&flit, &who, &name, now) {
                    Ok(s) => {
                        st.node.close_song();
                        st.node.open_song(s);
                        st.song_dir = Some(dir.clone());
                        st.live.set_history(main_history());
                        st.registry.remember_song(&name, &dir);
                        st.registry.save(&st.registry_path);
                        st.capture.clear();
                        st.heard_as_audio.clear();
                        st.status_dirty = true;
                        st.claim_keeper_if_mine();
                        st.announce_beings();
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                }
            }
            Request::ShareProject { name, dir, project } => {
                let flit = dir.join(".flit");
                if Song::exists(&flit) {
                    return Response::Error("this project is already a song".into());
                }
                match Song::create(&flit, &who, &name, now) {
                    Ok(mut s) => {
                        if let Err(e) = s.save_version_from_file(&who, main_history(), "v1", &project, &[], now) {
                            return Self::err(e);
                        }
                        if let Some(f) = project.file_name() {
                            let _ = s.set_project_file(&f.to_string_lossy());
                        }
                        st.node.close_song();
                        st.node.open_song(s);
                        st.song_dir = Some(dir.clone());
                        st.live.set_history(main_history());
                        st.registry.remember_song(&name, &dir);
                        st.registry.save(&st.registry_path);
                        st.status_dirty = true;
                        st.linked_project = None;
                        st.claim_keeper_if_mine();
                        st.announce_beings();
                        for n in st.auto_link() {
                            self.push_all(Push::Warning(n));
                        }
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                }
            }
            Request::OpenSong { dir } => match st.open_song_dir(&dir) {
                Ok(()) => {
                    st.linked_project = None;
                    for n in st.auto_link() {
                        self.push_all(Push::Warning(n));
                    }
                    Response::Ok
                }
                Err(e) => Response::Error(e),
            },
            Request::CloseSong | Request::LeaveSong => {
                st.node.close_song();
                st.song_dir = None;
                st.paused = None;
                st.linked_project = None;
                st.status_dirty = true;
                let _ = st.auto_link();
                Response::Ok
            }
            Request::Songs => Response::Songs(st.registry.songs.clone()),
            Request::SongCode => match st.node.song_code() {
                Ok(c) => Response::Code(c.encode()),
                Err(e) => Self::err(e),
            },
            Request::Approve { id, owner } => match st.node.approve(id, owner) {
                Ok(_) => {
                    st.status_dirty = true;
                    Response::Ok
                }
                Err(e) => Self::err(e),
            },
            Request::Deny { id } => {
                st.node.deny(id);
                st.status_dirty = true;
                Response::Ok
            }
            Request::Block { id } => {
                st.node.block(id);
                let path = st.registry_path.with_file_name("blocked.txt");
                if let Ok(mut bl) = BlockList::load(&path) {
                    let _ = bl.block(id);
                }
                st.status_dirty = true;
                Response::Ok
            }
            Request::AddFriend { id, name } => {
                let Some(addr) = AddrInfo::decode(&id) else { return Response::Error("that is not an ID".into()) };
                st.node.learn(&addr);
                st.registry.add_friend(addr.id, &name, if addr.ips.is_empty() && addr.relay.is_none() { None } else { Some(addr.clone()) });
                st.registry.save(&st.registry_path);
                Response::Ok
            }
            Request::Friends => Response::Friends(st.registry.friends.iter().map(|f| Friend { id: f.id, name: f.name.clone() }).collect()),
            Request::Promote { id } => stamp!(|s| s.promote(&who, id, now)),
            Request::RemoveFromCrew { id } => stamp!(|s| s.remove(&who, id, now)),
            Request::BlockBeing { being } => stamp!(|s| s.block_being(&who, being.clone(), now)),
            Request::UnblockBeing { being } => stamp!(|s| s.unblock_being(&who, being.clone(), now)),
            Request::Pardon { being } => stamp!(|s| s.pardon(&who, being.clone(), now)),

            // ----- beings -----
            Request::Beings => Response::Beings(st.node.with_song(|s| st.being_rows(s)).unwrap_or_else(|_| {
                st.profile
                    .beings
                    .iter()
                    .map(|b| BeingRow { being: BeingRef { name: me, being: b.id.clone() }, kind: b.kind, display_name: b.display_name.clone(), color: b.color, revoked: false, delegated: b.delegated_key.is_some(), granted: vec![], flags: 0, online: true, history: None, presence: None, mine: true, acting: b.id == who.being })
                    .collect()
            })),
            Request::ActAs { being } => match st.names.set_active_being(&me, &being) {
                Ok(p) => {
                    st.profile = p;
                    let mut w = who.clone();
                    w.being = being;
                    st.node.set_stamper(w.clone());
                    st.live.set_stamper(w);
                    st.status_dirty = true;
                    st.presence_dirty = true;
                    Response::Ok
                }
                Err(e) => Self::err(e),
            },
            Request::NewBeing { id, kind, display_name, color } => {
                let color = color.unwrap_or_else(|| st.profile.next_color());
                match st.names.add_being(&me, BeingProfile { id, kind, display_name, color, delegated_key: None }) {
                    Ok(p) => {
                        // the first being of a Name is the one it acts as
                        let first = p.beings.len() == 1;
                        let active = p.active().map(|b| b.id.clone());
                        st.profile = p;
                        if first {
                            if let Some(b) = active {
                                let mut w = who.clone();
                                w.being = b;
                                st.node.set_stamper(w.clone());
                                st.live.set_stamper(w);
                            }
                        }
                        st.announce_beings();
                        st.status_dirty = true;
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                }
            }
            Request::UpdateBeing { id, display_name, color } => {
                let mut p = st.profile.clone();
                let Some(b) = p.beings.iter_mut().find(|b| b.id == id) else { return Response::Error("no such being".into()) };
                b.display_name = display_name;
                b.color = color;
                if let Err(e) = st.names.save_profile(&p) {
                    return Self::err(e);
                }
                st.profile = p;
                st.announce_beings();
                st.status_dirty = true;
                Response::Ok
            }
            Request::RevokeBeing { id } => {
                if id == who.being {
                    return Response::Error("switch to another being first".into());
                }
                match st.names.remove_being(&me, &id) {
                    Ok(p) => {
                        st.profile = p;
                        stamp!(|s| s.revoke_being(&who, &id.0, now))
                    }
                    Err(e) => Self::err(e),
                }
            }
            Request::Grant { being, tracks, grant } => stamp!(|s| s.grant(&who, &being.0, tracks.clone(), grant, now)),
            Request::Flag { being, note } => stamp!(|s| s.flag(&who, being.clone(), FlagKind::ClaimedKind, &note, now)),

            // ----- Names on this machine -----
            Request::Names => match st.names.list() {
                Ok(v) => Response::Names(v.into_iter().map(|p| NameRow { current: p.id == me, beings: p.beings.iter().map(|b| (b.id.clone(), b.display_name.clone())).collect(), id: p.id, label: p.label, backed_up: p.backed_up }).collect()),
                Err(e) => Self::err(e),
            },
            Request::SetNameLabel { label } => {
                let label = label.trim().to_string();
                if label.is_empty() || label.chars().count() > 64 {
                    return Response::Error("a Name needs 1 to 64 characters".into());
                }
                match st.names.set_label(&me, &label) {
                    Ok(p) => {
                        st.profile = p;
                        st.node.set_display_name(&label);
                        st.status_dirty = true;
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                }
            }
            Request::SwitchName { id } => match st.switch_name(&id) {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error(e),
            },
            Request::NewName { label, being, display_name, kind } => {
                let color = BEING_COLORS[0];
                match st.names.create(&label, Some(BeingProfile { id: being, kind, display_name, color, delegated_key: None })) {
                    Ok((k, _)) => match st.switch_name(&k.id()) {
                        Ok(()) => Response::Ok,
                        Err(e) => Response::Error(e),
                    },
                    Err(e) => Self::err(e),
                }
            }
            Request::Logout => match st.names.logout() {
                Ok(()) => {
                    st.warnings.push("Logged out. Pick a Name to continue.".into());
                    st.status_dirty = true;
                    Response::Ok
                }
                Err(e) => Self::err(e),
            },
            Request::RemoveName { id } => {
                if id == me {
                    return Response::Error("switch to another Name first".into());
                }
                match st.names.remove(&id) {
                    Ok(()) => Response::Ok,
                    Err(e) => Self::err(e),
                }
            }
            Request::BackupName { passphrase, path } => match st.names.export_backup(&me, &passphrase) {
                Ok(bytes) => match std::fs::write(&path, bytes) {
                    Ok(()) => {
                        let _ = st.names.mark_backed_up(&me);
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                },
                Err(e) => Self::err(e),
            },
            Request::ImportName { path, passphrase } => match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| st.names.import_backup(&b, &passphrase).map_err(|e| e.to_string())) {
                Ok(p) => match st.switch_name(&p.id) {
                    Ok(()) => Response::Ok,
                    Err(e) => Response::Error(e),
                },
                Err(e) => Response::Error(e),
            },
            Request::RecoveryCode => {
                let _ = st.names.mark_backed_up(&me);
                Response::Text(NameStore::recovery_code(&who.key))
            }
            Request::ImportRecovery { code, label, being } => match NameStore::from_recovery_code(&code) {
                Ok(k) => {
                    let display = being.0.clone();
                    match st.names.adopt(&k, &label, vec![BeingProfile { id: being, kind: BeingKind::Human, display_name: display, color: BEING_COLORS[0], delegated_key: None }]) {
                        Ok(_) => match st.switch_name(&k.id()) {
                            Ok(()) => Response::Ok,
                            Err(e) => Response::Error(e),
                        },
                        Err(e) => Self::err(e),
                    }
                }
                Err(e) => Self::err(e),
            },

            // ----- keeper and settings -----
            Request::MoveKeeper { name, machine } => {
                let r = stamp!(|s| s.move_keeper(&who, name, &machine, now));
                st.claim_keeper_if_mine();
                r
            }
            Request::SetRequiredVersion { version } => stamp!(|s| s.set_required_version(&who, &version, now)),
            Request::SetReferenceDaw { being } => stamp!(|s| s.set_reference_daw(&who, history, being.clone(), now)),

            // ----- live flow -----
            Request::Transport { change, pos } => {
                st.last_pos = pos;
                // Every Track instance in a DAW reports the same transport:
                // a change is stamped once, whoever says it first.
                let playing = st.node.with_song(|s| s.state(&history).map(|x| x.transport.playing)).ok().flatten();
                if matches!((&change, playing), (TransportChange::Play, Some(true)) | (TransportChange::Stop, Some(false))) {
                    return Response::Ok;
                }
                let acts = song!(|s| st.live.report_transport(s, change, pos, now));
                if !acts.is_empty() {
                    st.node.publish();
                }
                Response::Ok
            }
            Request::Audio { track, pos, pcm } => {
                let Some(ci) = st.clients.get(&conn).cloned() else { return Response::Error("say hello first".into()) };
                st.last_pos = pos;
                // Panel on the master: the reference mix when my being is the Reference DAW.
                if track.0 == REFERENCE_KEY {
                    let reference = st.node.with_song(|s| s.state(&history).and_then(|x| x.settings.reference_daw.clone())).unwrap_or(None);
                    let i_am = reference.as_ref() == Some(&who.being_ref());
                    if !i_am {
                        if !st.master_warned && st.live.bridge.master_has_processing() == Some(true) {
                            st.master_warned = true;
                            let owner = reference.map(|r| st.being_label(&r)).unwrap_or_else(|| "nobody yet".into());
                            let msg = format!("Only you hear your master chain. The song's reference DAW is {owner}.");
                            st.warnings.push(msg.clone());
                            self.push_all(Push::Warning(msg));
                        }
                        return Response::Ok;
                    }
                    let peers = st.node.connected_peers();
                    let out = st.outgoing.entry(REFERENCE_KEY.to_string()).or_insert_with(|| {
                        let cfg = StreamConfig { kind: StreamKind::ReferenceMix, channels: ci.channels, bitrate: 192_000, low_delay: false, sample_rate: ci.sample_rate };
                        Outgoing { enc: StreamEncoder::new(cfg).expect("opus"), streams: HashMap::new() }
                    });
                    let packets = out.enc.push(&pcm).unwrap_or_default();
                    for p in packets {
                        for peer in &peers {
                            if let Some(n) = out.streams.get(peer) {
                                let _ = st.node.send_frame(peer, &StreamFrame { stream: *n, packet: p.clone() });
                            }
                        }
                    }
                    return Response::Ok;
                }
                let frames = st.track_block(&track, pos, &pcm, ci.sample_rate, ci.channels);
                for (peer, f) in frames {
                    let _ = st.node.send_frame(&peer, &f);
                }
                Response::Ok
            }
            Request::TalkAudio { pcm } => {
                let Some(ci) = st.clients.get(&conn).cloned() else { return Response::Error("say hello first".into()) };
                let ClientKind::TalkSend { name, to } = ci.kind.clone() else { return Response::Error("not a Talk sender".into()) };
                let keyname = format!("__talk:{name}");
                let bitrate = st.registry.monitoring_bitrate.unwrap_or(128_000);
                let out = st.outgoing.entry(keyname).or_insert_with(|| {
                    let cfg = StreamConfig::talk(&name, ci.sample_rate, ci.channels).with_bitrate(bitrate);
                    Outgoing { enc: StreamEncoder::new(cfg).expect("opus"), streams: HashMap::new() }
                });
                let packets = out.enc.push(&pcm).unwrap_or_default();
                for p in packets {
                    for (peer, n) in &out.streams {
                        if to.is_empty() || to.contains(peer) {
                            let _ = st.node.send_frame(peer, &StreamFrame { stream: *n, packet: p.clone() });
                        }
                    }
                }
                Response::Ok
            }
            Request::RecordTalk { track, start } => {
                if start {
                    let Some(ci) = st.clients.get(&conn).cloned() else { return Response::Error("say hello first".into()) };
                    let ClientKind::TalkReceive { from } = ci.kind.clone() else { return Response::Error("not a Talk receiver".into()) };
                    let from = from.or_else(|| st.incoming.keys().map(|(id, _)| *id).next());
                    let Some(from) = from else { return Response::Error("nobody is sending".into()) };
                    st.talk_record = Some((track, from, Vec::new(), st.last_pos, ci.sample_rate, ci.channels));
                    Response::Ok
                } else {
                    let Some((track, from, pcm, start_pos, sr, ch)) = st.talk_record.take() else { return Response::Error("not recording".into()) };
                    if pcm.is_empty() {
                        return Response::Error("nothing recorded".into());
                    }
                    let tempo = st.node.with_song(|s| s.state(&history).map(|x| x.settings.tempo).unwrap_or(120.0)).unwrap_or(120.0);
                    let plan = flit_core::chunk::ChunkPlan { sample_rate: sr, channels: ch, bpm: tempo, time_sig: Default::default() };
                    let audio = RenderedAudio { sample_rate: sr, channels: ch, start: start_pos, samples: pcm, print_point: PrintPoint::Raw };
                    let (opus, flac) = match st.node.with_song(|s| st.live.encoder.encode(s.blobs(), &audio, &plan)) {
                        Ok(Ok(v)) => v,
                        Ok(Err(e)) => return Self::err(e),
                        Err(e) => return Self::err(e),
                    };
                    // credit the player: their first declared being
                    let credit = st.node.with_song(|s| s.beings().of_name(&from).first().map(|(b, _)| BeingRef { name: from, being: b.clone() })).ok().flatten();
                    let payload = AudioPayload { start: start_pos, sample_rate: sr, bit_depth: 24, channels: ch, print_point: PrintPoint::Raw, opus_chunks: opus, flac_chunks: flac, credit };
                    stamp!(|s| s.audio(&who, history, &track, payload.clone(), now))
                }
            }
            Request::Presence { presence, pos } => {
                st.live.report_presence(presence, pos, now);
                st.presence_dirty = true;
                Response::Ok
            }
            Request::Follow { being, keep } => match being {
                None => {
                    st.following = None;
                    st.status_dirty = true;
                    Response::Ok
                }
                Some(b) => {
                    st.following = if keep { Some(b.clone()) } else { None };
                    st.status_dirty = true;
                    match st.follow(&b) {
                        None => Response::Ok,
                        Some(e) => Response::Error(e),
                    }
                }
            },
            Request::ShowMoment { at } => {
                if st.live.bridge.capabilities().level < flit_host::Level::LiveControl {
                    return Response::Error("This host has no bridge: it can't show a moment. Listen scrub is coming.".into());
                }
                match at {
                    None => {
                        if st.preview.is_some() {
                            for n in st.follow_history() {
                                self.push_all(Push::Warning(n));
                            }
                        }
                        Response::Ok
                    }
                    Some(at) => {
                        if !st.node.with_song(|s| s.chains().contains(&at)).unwrap_or(false) {
                            return Response::Error("no such moment".into());
                        }
                        let notices = st.show_state(Some(at));
                        for n in notices {
                            self.push_all(Push::Warning(n));
                        }
                        st.preview = Some(at);
                        st.tint_presence(Some(at));
                        st.status_dirty = true;
                        Response::Ok
                    }
                }
            }
            Request::Jump { pos } => match st.live.bridge.set_transport(&TransportChange::Position(pos)) {
                ApplyOutcome::Applied => Response::Ok,
                _ => Response::Error("This host can't be moved from here; use its own transport.".into()),
            },
            Request::SaveVersion { name } => match st.save_version(&name) {
                Ok(acts) if acts.is_empty() => {
                    self.push_all(Push::Warning(format!("Asked {} to save; \"{}\" lands as soon as the file is written.", st.live.host(), name)));
                    Response::Stamped(Hash::ZERO)
                }
                Ok(acts) => Response::Stamped(acts.iter().find(|e| e.kind() == Kind::Version).map(|e| e.id).unwrap_or(Hash::ZERO)),
                Err(e) => Response::Error(e),
            },

            // ----- histories and the timeline -----
            Request::Histories => Response::Histories(st.node.with_song(|s| st.history_rows(s)).unwrap_or_default()),
            Request::Timeline { history: h, lens, show_presence } => Response::Timeline(st.timeline(&h, &lens, show_presence)),
            Request::GoBack { to } => {
                let r = stamp!(|s| s.go_back(&who, history, to, now));
                for n in st.follow_history() {
                    self.push_all(Push::Warning(n));
                }
                r
            }
            Request::NewHistory { name, from } => {
                let new_history = song!(|s| s.new_history(&who, &name, from, now));
                st.node.publish();
                st.live.set_history(new_history);
                for n in st.follow_history() {
                    self.push_all(Push::Warning(n));
                }
                st.status_dirty = true;
                Response::History(new_history)
            }
            Request::SwitchHistory { history: h } => {
                if !st.node.with_song(|s| s.chains().has_history(&h)).unwrap_or(false) {
                    return Response::Error("no such history".into());
                }
                st.live.set_history(h);
                for n in st.follow_history() {
                    self.push_all(Push::Warning(n));
                }
                st.status_dirty = true;
                st.presence_dirty = true;
                Response::Ok
            }
            Request::Undo { target } => {
                let r = stamp!(|s| s.undo(&who, history, target, now));
                for n in st.follow_history() {
                    self.push_all(Push::Warning(n));
                }
                r
            }
            Request::Tracks => Response::Tracks(st.node.with_song(|s| st.track_infos(s)).unwrap_or_default()),
            Request::MergePreview { from_history, tracks } => {
                let p = song!(|s| s.merge_preview(history, s, from_history, &tracks, None));
                let info = st
                    .node
                    .with_song(|s| {
                        let summary = |id: &ActId| s.chains().get(id).map(|a| (flit_core::seal::lens::summarize(a, s.chains()), a.being())).unwrap_or_else(|| (String::new(), who.being_ref()));
                        MergeInfo {
                            from_history: p.from_history,
                            to_history: p.to_history,
                            acts: p.acts.len(),
                            counts: p.counts.iter().map(|(t, n)| (t.clone(), *n)).collect(),
                            conflicts: p
                                .conflicts
                                .iter()
                                .map(|c| {
                                    let (fs, fb) = summary(&c.from_act);
                                    let (ts, tb) = summary(&c.to_act);
                                    let track = c.thing.track().map(|t| st.live.track_name(t).map(str::to_string).unwrap_or(t.0.clone())).unwrap_or_else(|| "Song".into());
                                    ConflictRow { thing: c.thing.clone(), property: c.property.clone(), label: format!("{track} / {}", c.property), from_summary: fs, to_summary: ts, from_being: fb, to_being: tb, keep_from: c.keep_from }
                                })
                                .collect(),
                            flac_needed: p.flac_needed.len(),
                        }
                    })
                    .unwrap_or_else(|_| MergeInfo { from_history, to_history: history, acts: 0, counts: vec![], conflicts: vec![], flac_needed: 0 });
                Response::MergePreview(info)
            }
            Request::Merge { .. } | Request::JoinWithCode { .. } | Request::AcceptInvite { .. } | Request::Invite { .. } | Request::CheckUpdates | Request::OrgInvite { .. } | Request::AcceptOrgInvite { .. } => unreachable!("handled above"),

            // ----- PACK 2: the Library -----
            Request::Library => Response::Library(st.library()),
            Request::NewOrg { name } => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Response::Error("an org needs a name".into());
                }
                let tmp = st.orgs_dir().join(format!("new-{}", now));
                match Song::create_kind(&tmp, &who, &name, ContainerKind::Org, now) {
                    Ok(s) => {
                        let id = s.id();
                        drop(s);
                        let dir = st.org_dir(&id);
                        if let Err(e) = std::fs::rename(&tmp, &dir) {
                            return Self::err(e);
                        }
                        match Song::open(&dir) {
                            Ok(s) => {
                                st.node.add_container(s);
                                st.claim_container_keeper(&id);
                                st.status_dirty = true;
                                Response::Ok
                            }
                            Err(e) => Self::err(e),
                        }
                    }
                    Err(e) => Self::err(e),
                }
            }
            Request::OrgMembers { org } => Response::OrgMembers(st.org_member_rows(&org)),
            Request::DeclineInvite { container } => {
                st.node.forget_invite(container);
                let _ = st.node.with_container(&container, |s| s.decline(&who, now));
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::OrgLeave { org } => {
                cont!(org, |s| s.leave(&who, now));
                st.node.publish();
                if let Some(s) = st.node.remove_container(&org) {
                    drop(s);
                    let dir = st.org_dir(&org);
                    let _ = std::fs::rename(&dir, dir.with_extension("left"));
                }
                st.status_dirty = true;
                Response::Ok
            }
            Request::OrgRemove { org, id } => {
                cont!(org, |s| s.remove(&who, id, now));
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::OrgPromote { org, id } => {
                cont!(org, |s| s.promote(&who, id, now));
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::OrgDemote { org, id } => {
                cont!(org, |s| s.demote(&who, id, now));
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::OrgSettings { org, label, color, defaults, archived } => {
                if let Some(l) = label {
                    cont!(org, |s| s.set_label(&who, &l, now));
                }
                if let Some(c) = color {
                    cont!(org, |s| s.set_color(&who, c, now));
                }
                if let Some(d) = defaults {
                    cont!(org, |s| s.set_defaults(&who, d, now));
                }
                if let Some(a) = archived {
                    cont!(org, |s| s.set_archived(&who, a, now));
                }
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::OrgMoveKeeper { org, name, machine } => {
                cont!(org, |s| s.move_keeper(&who, name, &machine, now));
                st.node.publish();
                st.claim_container_keeper(&org);
                st.status_dirty = true;
                Response::Ok
            }
            Request::SetSongOrg { org, access } => {
                if let Some(o) = org {
                    if !st.node.containers_of(ContainerKind::Org).contains(&o) {
                        return Response::Error("not a member of that org".into());
                    }
                }
                song!(|s| s.set_org(&who, org, access, now));
                st.node.publish();
                st.node.refresh_org_links();
                st.note_open_song();
                st.status_dirty = true;
                Response::Ok
            }
            Request::SetAiPolicy { policy } => {
                song!(|s| s.set_ai_policy(&who, policy, now));
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::NewSongIn { name, dir, org, access } => {
                let flit = dir.join(".flit");
                if Song::exists(&flit) {
                    return Response::Error("this folder already holds a song".into());
                }
                let defaults = org.and_then(|o| st.node.with_container(&o, |s| s.main().settings.defaults.clone()).ok());
                if org.is_some() && defaults.is_none() {
                    return Response::Error("not a member of that org".into());
                }
                match Song::create(&flit, &who, &name, now) {
                    Ok(mut s) => {
                        if let Some(o) = org {
                            let d = defaults.unwrap_or_default();
                            if let Err(e) = s.set_org(&who, Some(o), access.unwrap_or(d.access), now) {
                                return Self::err(e);
                            }
                            if let Some(v) = &d.required_version {
                                let _ = s.set_required_version(&who, v, now);
                            }
                            if d.ai_policy != flit_core::AiPolicy::default() {
                                let _ = s.set_ai_policy(&who, d.ai_policy, now);
                            }
                            if let Some((kn, km)) = &d.keeper {
                                let _ = s.move_keeper(&who, *kn, km, now);
                            }
                        }
                        st.node.close_song();
                        st.node.open_song(s);
                        st.song_dir = Some(dir.clone());
                        st.live.set_history(main_history());
                        st.registry.remember_song(&name, &dir);
                        st.registry.save(&st.registry_path);
                        st.capture.clear();
                        st.heard_as_audio.clear();
                        st.status_dirty = true;
                        st.claim_keeper_if_mine();
                        st.announce_beings();
                        st.note_open_song();
                        st.node.publish();
                        Response::Ok
                    }
                    Err(e) => Self::err(e),
                }
            }
            Request::Albums => Response::Albums(st.album_rows()),
            Request::NewAlbum { scope, name } => {
                let Some(id) = st.scope_id(&scope) else { return Response::Error("no such shelf".into()) };
                let album = cont!(id, |s| s.create_album(&who, &name, now));
                st.node.publish();
                st.status_dirty = true;
                Response::Album(album)
            }
            Request::AlbumAct { scope, album, action } => {
                let Some(id) = st.scope_id(&scope) else { return Response::Error("no such shelf".into()) };
                cont!(id, |s| s.album(&who, album, action, now));
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::AlbumCover { scope, album, path } => {
                let Some(id) = st.scope_id(&scope) else { return Response::Error("no such shelf".into()) };
                let bytes = match std::fs::read(&path) {
                    Ok(b) => b,
                    Err(e) => return Response::Error(format!("could not read {}: {e}", path.display())),
                };
                if bytes.len() > 4 * 1024 * 1024 {
                    return Response::Error("a cover must be 4 MB or less".into());
                }
                if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
                    return Response::Error("a cover is a PNG file".into());
                }
                let hash = cont!(id, |s| s.blobs().put(&bytes));
                cont!(id, |s| s.album(&who, album, AlbumAction::SetCover(Some(hash)), now));
                st.node.publish();
                st.status_dirty = true;
                Response::Ok
            }
            Request::ContainerBlob { scope, hash } => {
                let Some(id) = st.scope_id(&scope) else { return Response::Error("no such shelf".into()) };
                match st.node.with_container(&id, |s| s.blobs().get(&hash)) {
                    Ok(Ok(b)) => Response::Bytes(b),
                    Ok(Err(e)) => Self::err(e),
                    Err(e) => Self::err(e),
                }
            }
            Request::OpenInDaw { dir } => {
                if st.song_dir.as_deref() != Some(dir.as_path()) {
                    if let Err(e) = st.open_song_dir(&dir) {
                        return Response::Error(e);
                    }
                    st.linked_project = None;
                }
                let project = st.node.with_song(|s| s.project_file().map(|f| dir.join(f))).ok().flatten().filter(|p| p.is_file());
                match project {
                    Some(p) => match open::that_detached(&p) {
                        Ok(()) => Response::Text(format!("Opening {} in the DAW", p.display())),
                        Err(e) => Response::Error(format!("could not open {}: {e}", p.display())),
                    },
                    None => match daw_exe() {
                        Some(exe) => {
                            let mut cmd = std::process::Command::new(&exe);
                            cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                            match cmd.spawn() {
                                Ok(_) => Response::Text(format!("Started {}. Save the project into {} and it becomes this song's.", exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), dir.display())),
                                Err(e) => Self::err(e),
                            }
                        }
                        None => Response::Error(format!("this song has no project file yet; save your DAW project into {}", dir.display())),
                    },
                }
            }
            Request::AlbumTimeline { scope, album } => {
                let Some(id) = st.scope_id(&scope) else { return Response::Error("no such shelf".into()) };
                let rows = st.node.with_container(&id, |s| s.rows(&main_history(), &Lens::Album(album), false)).unwrap_or_default();
                Response::Timeline(rows)
            }

            // ----- chat -----
            Request::Chat { text, pinned } => {
                let act: Act = song!(|s| s.chat(&who, history, &text, pinned, now));
                st.node.publish();
                let row = st.chat_rows().into_iter().find(|c| c.id == act.id);
                if let Some(c) = row {
                    self.push_all(Push::Chat(c));
                }
                Response::Stamped(act.id)
            }
            Request::ChatList => Response::Chat(st.chat_rows()),

            // ----- settings -----
            Request::Settings(p) => {
                if let Some(d) = p.download_full_quality {
                    st.settings.download_full_quality = d;
                }
                if let Some(b) = p.monitoring_bitrate {
                    st.registry.monitoring_bitrate = Some(b.clamp(64_000, 256_000));
                    st.outgoing.clear();
                }
                if let Some(s) = p.submix {
                    st.registry.submix = s;
                    st.outgoing.clear();
                }
                if let Some(h) = p.hear_reference {
                    st.registry.hear_reference = h;
                }
                if let Some(r) = p.relay_only {
                    st.registry.relay_only = r;
                    st.warnings.push("Relay-only mode takes effect when the agent restarts".into());
                }
                if let Some(v) = p.show_presence_in_daw {
                    st.registry.show_presence_in_daw = v;
                    if !v {
                        st.live.bridge.clear_presence();
                        st.last_shown.clear();
                    }
                }
                if let Some(u) = p.update_manifest_url {
                    st.registry.update_manifest_url = u;
                }
                if let Some(c) = p.update_channel {
                    st.registry.update_channel = if c == "beta" { "beta".into() } else { "stable".into() };
                }
                let _ = st.settings.save();
                st.registry.save(&st.registry_path);
                st.status_dirty = true;
                Response::Ok
            }
            Request::Fake(cmd) => {
                let Some(f) = st.live.bridge.fake() else { return Response::Error("agent is not running FakeHost".into()) };
                match cmd {
                    FakeCmd::AddTrack { track, name, kind, plugin, tone_hz } => {
                        let kind = match kind.as_str() {
                            "bus" => TrackKind::Bus,
                            "master" => TrackKind::Master,
                            _ => TrackKind::Part,
                        };
                        f.add_track(&track, &name, kind, plugin, tone_hz)
                    }
                    FakeCmd::AddDevice { track, slot, plugin } => f.add_device(&track, DeviceSlot(slot), plugin),
                    FakeCmd::RemoveDevice { track, slot } => f.remove_device(&track, DeviceSlot(slot)),
                    FakeCmd::Twist { track, slot, param, value } => f.twist(&track, DeviceSlot(slot), &param, value),
                    FakeCmd::Sweep { track, slot, param, from, to, steps } => f.sweep(&track, DeviceSlot(slot), &param, from, to, steps),
                    FakeCmd::Volume { track, value } => f.set_volume(&track, value),
                    FakeCmd::Tempo { bpm } => f.set_tempo(bpm),
                    FakeCmd::Play => f.play(),
                    FakeCmd::Stop => f.stop(),
                    FakeCmd::SaveProject { path } => {
                        if let Err(e) = f.save_project(&path) {
                            return Self::err(e);
                        }
                    }
                    FakeCmd::Param { track, slot, param } => return Response::Value(f.param(&track, DeviceSlot(slot), &param)),
                    FakeCmd::Focus { view, track, slot } => f.focus(view, track.as_deref(), slot.map(DeviceSlot)),
                }
                Response::Ok
            }
        }
    }
}

/// Same folder, whatever the spelling of the path.
fn same_dir(a: &Path, b: &Path) -> bool {
    let ca = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Build a level-0 agent config (no bridge).
pub fn no_bridge() -> AnyBridge {
    AnyBridge::None(NoBridge::default())
}
