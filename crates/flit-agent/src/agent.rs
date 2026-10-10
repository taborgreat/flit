//! The agent: one per user per machine. Owns the song, the network, the
//! codecs and the bridge; serves every plugin instance over IPC.

use crate::bridge::{AnyBridge, NoBridge};
use crate::store::Registry;
use flit_audio::{FlitChunkEncoder, StreamConfig, StreamDecoder, StreamEncoder, StreamKind};
use flit_core::song::{BringTarget, MIX_LANE};
use flit_core::*;
use flit_host::{ApplyOutcome, ChunkEncoder, HostBridge, LiveLoop, Receiver, ReceiverDecision, RenderedAudio};
use flit_ipc::*;
use flit_net::{AddrInfo, NetEvent, Node, NodeConfig, StreamFrame};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Keep at most this much captured audio per lane between saves (10 min stereo at 48k).
const MAX_CAPTURE_SAMPLES: usize = 48_000 * 2 * 600;

pub struct AgentConfig {
    pub key: Keypair,
    pub display_name: String,
    pub data_dir: PathBuf,
    pub bridge: AnyBridge,
    pub local_only: bool,
    pub relay_only: bool,
    pub open_song: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct ClientInfo {
    kind: ClientKind,
    sample_rate: u32,
    channels: u8,
}

/// Audio captured from one lane plugin since the last save, by segment.
#[derive(Default)]
struct Capture {
    segments: Vec<(SongPos, Vec<f32>)>,
    last_pos: Option<SongPos>,
    sample_rate: u32,
    channels: u8,
}

struct Outgoing {
    enc: StreamEncoder,
    /// (peer, stream number) this lane is open on.
    streams: HashMap<Id, u16>,
}

pub struct State {
    /// Last bridge link state pushed to the panel (so a flip pushes a Status).
    last_link: Option<bool>,
    pub node: Node,
    pub live: LiveLoop<AnyBridge, FlitChunkEncoder>,
    registry: Registry,
    registry_path: PathBuf,
    settings: UserSettings,
    clients: HashMap<ConnId, ClientInfo>,
    outgoing: HashMap<LaneId, Outgoing>,
    incoming: HashMap<(Id, u16), (StreamKind, StreamDecoder)>,
    capture: HashMap<LaneId, Capture>,
    heard_as_audio: HashSet<LaneId>,
    talk_record: Option<(LaneId, Id, Vec<f32>, SongPos, u32, u8)>,
    warnings: Vec<String>,
    master_warned: bool,
    status_dirty: bool,
    song_dir: Option<PathBuf>,
    last_pos: SongPos,
    /// Timeline Exact view: the row the host is showing instead of now.
    preview: Option<StampId>,
    /// The DAW project the agent last linked (or tried to).
    linked_project: Option<PathBuf>,
    link: ProjectLink,
    /// Live edits are not stamped while this is set (project/song mismatch).
    paused: Option<String>,
    pub shutdown: bool,
}

#[derive(Clone)]
pub struct Agent {
    pub state: Arc<Mutex<State>>,
    pub handle: Arc<Mutex<Option<ServerHandle>>>,
}

fn now_ms() -> u64 {
    flit_net::node::now_ms()
}

fn lane_infos(song: &Song, branch: &BranchId, me: &Id) -> Vec<LaneInfo> {
    let Some(st) = song.state(branch) else { return vec![] };
    let mut lanes: HashSet<LaneId> = st.owners.keys().cloned().collect();
    lanes.extend(st.lanes.keys().cloned());
    let mut v: Vec<LaneInfo> = lanes
        .into_iter()
        .map(|l| LaneInfo { name: st.lane_names.get(&l).cloned(), kind: st.kind(&l), owner: st.owner(&l), mine: st.owner(&l) == Some(*me), lane: l })
        .collect();
    v.sort_by(|a, b| (a.kind != LaneKind::Mix).cmp(&(b.kind != LaneKind::Mix)).then(a.lane.cmp(&b.lane)));
    v
}

impl State {
    /// What to call a Name: my own nickname, else what they set in the song.
    fn name_for(&self, id: &Id) -> Option<String> {
        if let Some(n) = self.registry.friend_name(id) {
            return Some(n);
        }
        self.node.with_song(|s| s.display_name(id).map(str::to_string)).ok().flatten()
    }

    /// My display name goes into the open song as a Profile stamp (once per
    /// change), so the crew folds it from my key.
    fn announce_name(&mut self) {
        let Some(name) = self.settings.display_name.clone().filter(|n| !n.trim().is_empty()) else { return };
        let me = self.node.id();
        let current = self.node.with_song(|s| s.display_name(&me).map(str::to_string)).ok().flatten();
        if current.as_deref() == Some(name.trim()) {
            return;
        }
        let key = self.node.key().clone();
        let host = self.live.host();
        let time = self.time();
        if let Ok(Ok(ev)) = self.node.with_song(|s| s.set_display_name(&key, &name, &host, time)) {
            self.node.publish(std::slice::from_ref(&ev));
            self.status_dirty = true;
        }
    }

    fn song_status(&self) -> Option<SongStatus> {
        let me = self.node.id();
        let branch = self.live.branch();
        let code = self.node.song_code().map(|c| c.encode()).unwrap_or_default();
        self.node
            .with_song(|s| {
                let st = s.state(&branch);
                let mut ids: Vec<Id> = s.crew().members.iter().copied().collect();
                for id in s.chains().names().keys() {
                    if !ids.contains(id) {
                        ids.push(*id);
                    }
                }
                let names: Vec<(Id, String)> = ids
                    .into_iter()
                    .filter_map(|id| self.registry.friend_name(&id).or_else(|| s.display_name(&id).map(str::to_string)).map(|n| (id, n)))
                    .collect();
                SongStatus {
                    id: s.id(),
                    name: s.name().to_string(),
                    dir: self.song_dir.clone().unwrap_or_else(|| s.dir().to_path_buf()),
                    code,
                    owners: s.crew().owners.iter().copied().collect(),
                    members: s.crew().members.iter().copied().collect(),
                    branch,
                    branch_name: s.chains().branch_name(&branch).unwrap_or("?").to_string(),
                    lanes: lane_infos(s, &branch, &me),
                    mix_owner: st.and_then(|st| st.mix_owner()),
                    i_own_mix: st.and_then(|st| st.mix_owner()) == Some(me),
                    events: s.chains().len(),
                    tempo: st.map(|st| st.transport.tempo).unwrap_or(120.0),
                    heard_as_audio: self.heard_as_audio.iter().cloned().collect(),
                    missing_opus: s.missing_opus(&branch).len(),
                    missing_flac: s.missing_flac(&branch).len(),
                    names,
                }
            })
            .ok()
    }

    fn status(&self) -> Status {
        let id = self.node.id();
        let song = self.song_status();
        let peers = self
            .node
            .peers()
            .into_iter()
            .map(|p| PeerInfo { name: self.name_for(&p.id), owner: song.as_ref().map(|s| s.owners.contains(&p.id)).unwrap_or(false), id: p.id, connected: p.connected, over_quota: p.over_quota })
            .collect();
        let invites = self.node.pending_invites().into_iter().map(|i| (i.from, i.song, i.name)).collect();
        Status {
            agent_version: VERSION.to_string(),
            id,
            short_code: id.short_code(),
            id_text: self.node.addr_info().encode(),
            host: self.live.host().0,
            level: self.live.bridge.capabilities().level as u8,
            project: self.live.bridge.current_project(),
            previewing: self.preview,
            bridge: self.live.bridge.describe(),
            bridge_up: self.live.bridge.link_up(),
            link: self.link.clone(),
            paused: self.paused.clone(),
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
                display_name: self.settings.display_name.clone().unwrap_or_default(),
            },
            warnings: self.warnings.clone(),
            connected_clients: self.clients.len(),
        }
    }

    fn chat_rows(&self) -> Vec<ChatRow> {
        self.node
            .with_song(|s| {
                s.all_chat()
                    .into_iter()
                    .filter_map(|(ev, branch_name)| match ev.payload() {
                        Payload::Chat(c) => Some(ChatRow {
                            id: ev.id,
                            from: ev.name(),
                            from_name: self.registry.friend_name(&ev.name()).or_else(|| s.display_name(&ev.name()).map(str::to_string)),
                            text: c.text.clone(),
                            branch: ev.branch(),
                            branch_name,
                            pos: ev.wall().pos,
                            wall_ms: ev.wall().wall_ms,
                            pinned: c.pinned,
                        }),
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
        self.live.set_branch(stamp::main_branch());
        self.node.with_song(|s| self.live.prime_from(s)).ok();
        self.registry.remember_song(&name, project_dir);
        self.registry.save(&self.registry_path);
        self.heard_as_audio.clear();
        self.status_dirty = true;
        self.announce_name();
        Ok(())
    }

    fn time(&self) -> Time {
        Time::at(now_ms(), self.last_pos)
    }

    /// Add my lane's block: capture for the next save, and encode for streams.
    fn lane_block(&mut self, lane: &LaneId, pos: SongPos, pcm: &[f32], sample_rate: u32, channels: u8) -> Vec<(Id, StreamFrame)> {
        let cap = self.capture.entry(lane.clone()).or_default();
        cap.sample_rate = sample_rate;
        cap.channels = channels;
        let frames = pcm.len() / channels.max(1) as usize;
        let tempo = self.node.with_song(|s| s.state(&self.live.branch()).map(|st| st.transport.tempo).unwrap_or(120.0)).unwrap_or(120.0);
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
        self.live.push_dirty(lane.clone());

        // stream out
        let peers = self.node.connected_peers();
        if peers.is_empty() {
            return vec![];
        }
        let bitrate = self.registry.monitoring_bitrate.unwrap_or(128_000);
        let kind = if self.registry.submix { StreamKind::Submix } else { StreamKind::InSong { lane: lane.clone() } };
        let key = if self.registry.submix { LaneId::new("__submix") } else { lane.clone() };
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

    /// Lanes whose streams still need opening on some peer.
    fn streams_to_open(&self) -> Vec<(Id, LaneId, StreamConfig)> {
        let peers = self.node.connected_peers();
        let mut v = Vec::new();
        for (lane, out) in &self.outgoing {
            for p in &peers {
                if !out.streams.contains_key(p) {
                    v.push((*p, lane.clone(), out.enc.config().clone()));
                }
            }
        }
        v
    }

    fn save_version(&mut self, name: &str) -> std::result::Result<Vec<Stamp>, String> {
        let dir = self.song_dir.clone().ok_or("no song")?;
        let tempo = self.node.with_song(|s| s.state(&self.live.branch()).map(|st| st.transport.tempo).unwrap_or(120.0)).map_err(|e| e.to_string())?;
        let _ = tempo;
        // level 0: captured lane audio becomes the render
        let captures: Vec<(LaneId, Capture)> = self.capture.drain().collect();
        for (lane, cap) in captures {
            for (start, samples) in cap.segments {
                if samples.is_empty() {
                    continue;
                }
                self.live.push_captured(lane.clone(), RenderedAudio { sample_rate: cap.sample_rate, channels: cap.channels, start, samples, print_point: PrintPoint::PostFaderPreSend });
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
        let evs = self.node.with_song(|s| self.live.save_version(s, name, &state_path, now, pos)).map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        self.node.publish(&evs);
        self.status_dirty = true;
        Ok(evs)
    }

    fn history_rows(&self, branch: &BranchId) -> Vec<HistoryRow> {
        self.node.with_song(|s| s.history(branch)).unwrap_or_default()
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
            // the project's own song: open it (even over another open song)
            match self.open_song_dir(&dir) {
                Ok(()) => {
                    let name = self.node.with_song(|s| s.name().to_string()).unwrap_or_default();
                    notices.push(format!("Opened the song \"{name}\" for {file}"));
                }
                Err(e) => notices.push(format!("Could not open the song in {}: {e}", dir.display())),
            }
        }
        let same_folder = self.song_dir.as_ref().map(|d| same_dir(d, &dir)).unwrap_or(false);
        let (link, paused) = if same_folder {
            let from = self.node.with_song(|s| s.project_file().map(str::to_string)).ok().flatten();
            match from {
                None => {
                    // first time this song meets its project: remember the file
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
                Some(format!("FL has {file} open, but the open song \"{name}\" belongs to {}. Edits are paused until you make {file} a song or open its folder's song.", song_dir.display())),
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

    /// The DAW follows History: after Go back, Undo, Bring in or a branch
    /// switch, push the branch's sound into the host (settings, faders,
    /// tempo) and re-prime the settle rule. Returns notices for the panel.
    fn follow_history(&mut self) -> Vec<String> {
        self.preview = None;
        let branch = self.live.branch();
        let me = self.node.id();
        let Some(state) = self.node.with_song(|s| s.state(&branch).cloned()).ok().flatten() else { return vec![] };
        let applied = Receiver::apply_state(&mut self.live.bridge, &state);
        self.node.with_song(|s| self.live.prime_from(s)).ok();
        tracing::info!("followed History: {} settings, {} mixer changes, {} devices, tempo {}", applied.settings, applied.mixer, applied.devices, applied.tempo);
        let mut notices = Vec::new();
        if !applied.manual.is_empty() {
            notices.push(format!("{} can't add or remove plugins from outside. To match this moment: {}", self.live.host(), applied.manual.join("; ")));
        }
        for l in applied.heard {
            if state.owner(&l) == Some(me) {
                continue;
            }
            if self.heard_as_audio.insert(l.clone()) {
                notices.push(format!("Hearing {l} as audio (this host can't reproduce it)"));
            }
        }
        self.status_dirty = true;
        notices
    }

    /// The project file this host had at row `at` (its nearest Version at
    /// or before it). No DAW can be made to load a project through a
    /// bridge, so the file is written beside the song for the user to open;
    /// knobs and faders are restored live by `follow_history`.
    fn export_version_at(&self, branch: &BranchId, at: &StampId) -> Option<String> {
        let host = self.live.host();
        let dir = self.song_dir.clone()?;
        let plan = self.node.with_song(|s| s.restore_plan(branch, at, &host)).ok()?.ok()?;
        let v = plan.version?;
        let Payload::Version(vp) = v.payload() else { return None };
        let current = self.node.with_song(|s| s.state(branch).and_then(|st| st.latest_version.get(&host).copied())).ok().flatten();
        if current == Some(v.id) {
            return None; // already on that project file
        }
        let bytes = self.node.with_song(|s| s.blobs().get(&vp.project)).ok()?.ok()?;
        if bytes.is_empty() {
            return None;
        }
        let ext = self.live.bridge.current_project().and_then(|p| p.extension().map(|e| e.to_string_lossy().into_owned())).unwrap_or_else(|| "project".into());
        let safe: String = vp.name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ') { c } else { '_' }).collect();
        let out = dir.join(".flit").join("restore");
        std::fs::create_dir_all(&out).ok()?;
        let path = out.join(format!("{} {}.{}", safe.trim(), v.id.short(), ext));
        std::fs::write(&path, bytes).ok()?;
        Some(format!("Knobs and faders are back. For notes and arrangement, open the project file from \"{}\": {}", vp.name, path.display()))
    }
}

impl Agent {
    pub async fn start(cfg: AgentConfig) -> anyhow::Result<Agent> {
        let registry_path = cfg.data_dir.join("registry.json");
        let registry = Registry::load(&registry_path);
        let settings = UserSettings::load(cfg.data_dir.join("settings.json")).unwrap_or_default();
        let blocklist = BlockList::load(cfg.data_dir.join("blocked.txt")).unwrap_or_default();
        let mut ncfg = NodeConfig::new(cfg.key.clone(), &cfg.display_name);
        ncfg.local_only = cfg.local_only;
        ncfg.relay_only = cfg.relay_only || registry.relay_only;
        ncfg.settings = settings.clone();
        let node = Node::bind(ncfg).await?;
        node.set_blocklist(blocklist);
        for f in &registry.friends {
            if let Some(a) = &f.addr {
                node.learn(a);
            }
        }
        let live = LiveLoop::new(cfg.bridge, FlitChunkEncoder::default(), cfg.key.clone(), stamp::main_branch());
        let mut state = State {
            last_link: None,
            node,
            live,
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
            song_dir: None,
            last_pos: SongPos::START,
            preview: None,
            linked_project: None,
            link: ProjectLink::Unknown,
            paused: None,
            shutdown: false,
        };
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
        // 1. tick: bridge -> settle -> events -> peers; status pushes
        let me = self.clone();
        tokio::spawn(async move {
            let mut last_status = std::time::Instant::now();
            let mut last_connect = std::time::Instant::now();
            let mut last_link_check = std::time::Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let mut pending_notices: Vec<String> = Vec::new();
                let (events, opens, dirty, done) = {
                    let mut st = me.state.lock().unwrap();
                    if st.shutdown {
                        (vec![], vec![], false, true)
                    } else {
                        let now = now_ms();
                        let st = &mut *st;
                        // mismatch safeguard: a paused agent reads the host but stamps nothing
                        let events = if st.paused.is_some() {
                            let _ = st.live.bridge.poll();
                            vec![]
                        } else { match st.node.with_song(|s| st.live.tick(s, now)) {
                            Ok(Ok(evs)) => evs,
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
                        } };
                        for n in std::mem::take(&mut st.live.notices) {
                            st.warnings.push(n.clone());
                            pending_notices.push(n);
                        }
                        if last_link_check.elapsed() > Duration::from_millis(1000) {
                            last_link_check = std::time::Instant::now();
                            pending_notices.extend(st.auto_link());
                        }
                        if !events.is_empty() {
                            st.node.publish(&events);
                            st.status_dirty |= events.iter().any(|e| !matches!(e.kind(), Kind::Setting | Kind::Mixer));
                        }
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
                        (events, opens, dirty, false)
                    }
                };
                if done {
                    break;
                }
                for n in pending_notices.drain(..) {
                    me.push_all(Push::Warning(n));
                }
                for ev in &events {
                    let row = me.state.lock().unwrap().history_rows(&ev.branch()).into_iter().find(|r| r.id == ev.id);
                    if let Some(r) = row {
                        me.push_all(Push::Row(r));
                    }
                }
                for (peer, lane, cfg) in opens {
                    let res = me.open_stream(peer, cfg).await;
                    let mut st = me.state.lock().unwrap();
                    if let Some(out) = st.outgoing.get_mut(&lane) {
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
            self.state.lock().unwrap().status_dirty = true;
        }
    }

    fn on_net_event(&self, ev: NetEvent) {
        match ev {
            NetEvent::Imported { from, ids } => {
                let (rows, heard) = {
                    let mut st = self.state.lock().unwrap();
                    let st = &mut *st;
                    let mut rows = Vec::new();
                    let mut heard = Vec::new();
                    let branch = st.live.branch();
                    let mut follow = false;
                    for id in &ids {
                        let decision = st.node.with_song(|s| Receiver::apply(&mut st.live.bridge, s, id)).ok().flatten();
                        if let Some(ReceiverDecision::HearAsAudio(l)) = decision {
                            if st.heard_as_audio.insert(l.clone()) {
                                heard.push(l);
                            }
                        }
                        if let Some(r) = st.node.with_song(|s| s.history(&branch).into_iter().find(|r| r.id == *id)).ok().flatten() {
                            // A peer's Go back / Undo changes the whole branch's sound:
                            // the per-event receiver cannot apply those.
                            follow |= matches!(r.kind, Kind::GoBack | Kind::Undo);
                            rows.push(r);
                        }
                    }
                    if follow {
                        heard.extend(st.follow_history().into_iter().map(|n| LaneId::new(&n)));
                    }
                    st.status_dirty = true;
                    let _ = from;
                    (rows, heard)
                };
                for r in rows {
                    if r.kind == Kind::Chat {
                        let chat = self.state.lock().unwrap().chat_rows().into_iter().find(|c| c.id == r.id);
                        if let Some(c) = chat {
                            self.push_all(Push::Chat(c));
                        }
                    }
                    self.push_all(Push::Row(r));
                }
                for l in heard {
                    if l.0.starts_with("Hearing ") {
                        self.push_all(Push::Warning(l.0.clone()));
                    } else {
                        self.push_all(Push::Warning(format!("Hearing {l} as audio (this host can't reproduce it)")));
                    }
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
            NetEvent::Invite { from, song, name } => {
                self.state.lock().unwrap().status_dirty = true;
                self.push_all(Push::Invite { from, song, name });
            }
            NetEvent::PeerUp(_) | NetEvent::PeerDown(_) | NetEvent::Joined { .. } | NetEvent::BlobFetched { .. } => {
                self.state.lock().unwrap().status_dirty = true;
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
                    StreamKind::InSong { lane } => {
                        let l2 = lane.clone();
                        self.push_where(Push::LaneAudio { lane, pcm }, move |k| matches!(k, ClientKind::Receiver { lane } if *lane == l2));
                    }
                    StreamKind::Submix => {
                        self.push_where(Push::LaneAudio { lane: LaneId::new("__submix"), pcm }, |k| matches!(k, ClientKind::Receiver { .. }));
                    }
                    StreamKind::ReferenceMix => {
                        let hear = self.state.lock().unwrap().registry.hear_reference;
                        if hear {
                            self.push_where(Push::LaneAudio { lane: LaneId::new(MIX_LANE), pcm }, |k| matches!(k, ClientKind::Panel));
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
                        self.push_where(Push::TalkAudio { from, name: "voice".into(), pcm }, |k| matches!(k, ClientKind::Panel));
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
        if let ClientKind::Lane { lane } = &kind {
            if let AnyBridge::None(nb) = &mut st.live.bridge {
                if !nb.lanes.contains(lane) {
                    nb.lanes.push(lane.clone());
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
                        st.live.set_branch(stamp::main_branch());
                        st.registry.remember_song(&name, dir);
                        st.registry.save(&st.registry_path);
                        st.status_dirty = true;
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
                        st.live.set_branch(stamp::main_branch());
                        st.registry.remember_song(&name, dir);
                        st.registry.save(&st.registry_path);
                        st.status_dirty = true;
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
            Request::BringIn { lane, source_branch, keep_both_as } => {
                let (node, preview, peers) = {
                    let st = self.state.lock().unwrap();
                    let branch = st.live.branch();
                    let p = st.node.with_song(|s| s.bring_preview(&branch, lane, s, source_branch, None));
                    match p {
                        Ok(Ok(p)) => (st.node.handle(), p, st.node.connected_peers()),
                        Ok(Err(e)) => return Self::err(e),
                        Err(e) => return Self::err(e),
                    }
                };
                // A peer MUST pull the FLAC before Bring in.
                if !preview.flac_needed.is_empty() {
                    for p in &peers {
                        let _ = node.fetch_blobs(*p, preview.flac_needed.clone()).await;
                    }
                }
                let mut st = self.state.lock().unwrap();
                let branch = st.live.branch();
                let host = st.live.host();
                let time = st.time();
                let target = keep_both_as.clone().map(BringTarget::NewLane).unwrap_or(BringTarget::SameLane);
                let key = st.node.key().clone();
                return match st.node.with_song(|s| s.bring_in(&key, &branch, lane, &preview, target, &host, time)) {
                    Ok(Ok(ev)) => {
                        st.node.publish(std::slice::from_ref(&ev));
                        for n in st.follow_history() {
                            self.push_all(Push::Warning(n));
                        }
                        Response::Stamped(ev.id)
                    }
                    Ok(Err(e)) => Self::err(e),
                    Err(e) => Self::err(e),
                };
            }
            _ => {}
        }

        let mut st = self.state.lock().unwrap();
        let st = &mut *st;
        let key = st.node.key().clone();
        let host = st.live.host();
        let branch = st.live.branch();
        let time = st.time();
        let me = key.id();

        macro_rules! song {
            ($f:expr) => {
                match st.node.with_song($f) {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return Self::err(e),
                    Err(e) => return Self::err(e),
                }
            };
        }
        macro_rules! publish_event {
            ($f:expr) => {{
                let ev: Stamp = song!($f);
                st.node.publish(std::slice::from_ref(&ev));
                st.status_dirty = true;
                Response::Stamped(ev.id)
            }};
        }

        match req {
            Request::Hello { .. } => Response::Ok,
            Request::Ping => Response::Pong,
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
                match Song::create(&flit, &key, &name, &host, time) {
                    Ok(s) => {
                        st.node.close_song();
                        st.node.open_song(s);
                        st.song_dir = Some(dir.clone());
                        st.live.set_branch(stamp::main_branch());
                        st.registry.remember_song(&name, &dir);
                        st.registry.save(&st.registry_path);
                        st.capture.clear();
                        st.heard_as_audio.clear();
                        st.status_dirty = true;
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
                match Song::create(&flit, &key, &name, &host, time) {
                    Ok(mut s) => {
                        let r = s.save_version_from_file(&key, &stamp::main_branch(), "v1", &project, &[], &host, time);
                        if let Err(e) = r {
                            return Self::err(e);
                        }
                        if let Some(f) = project.file_name() {
                            let _ = s.set_project_file(&f.to_string_lossy());
                        }
                        st.node.close_song();
                        st.node.open_song(s);
                        st.song_dir = Some(dir.clone());
                        st.live.set_branch(stamp::main_branch());
                        st.registry.remember_song(&name, &dir);
                        st.registry.save(&st.registry_path);
                        st.status_dirty = true;
                        st.linked_project = None;
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
            Request::CloseSong => {
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
            Request::Promote { id } => publish_event!(|s| s.promote(&key, id, &host, time)),
            Request::RemoveFromCrew { id } => publish_event!(|s| s.remove(&key, id, &host, time)),
            Request::LeaveSong => {
                st.node.close_song();
                st.song_dir = None;
                st.status_dirty = true;
                Response::Ok
            }
            Request::Transport { change, pos } => {
                st.last_pos = pos;
                let now = now_ms();
                let evs = song!(|s| st.live.report_transport(s, change, pos, now));
                st.node.publish(&evs);
                Response::Ok
            }
            Request::Audio { lane, pos, pcm } => {
                let Some(ci) = st.clients.get(&conn).cloned() else { return Response::Error("say hello first".into()) };
                st.last_pos = pos;
                // Panel on the master: the reference mix when I own the mix lane.
                if lane.0 == MIX_LANE {
                    let i_own = st.node.with_song(|s| s.state(&branch).and_then(|x| x.mix_owner()) == Some(me)).unwrap_or(false);
                    if !i_own {
                        if !st.master_warned && st.live.bridge.master_has_processing() == Some(true) {
                            st.master_warned = true;
                            let owner = st.node.with_song(|s| s.state(&branch).and_then(|x| x.mix_owner())).ok().flatten();
                            let msg = format!("Only you hear your master chain. The song's master belongs to {}.", owner.map(|o| o.short_code()).unwrap_or_else(|| "nobody yet".into()));
                            st.warnings.push(msg.clone());
                            self.push_all(Push::Warning(msg));
                        }
                        return Response::Ok;
                    }
                    let peers = st.node.connected_peers();
                    let out = st.outgoing.entry(LaneId::new("__reference")).or_insert_with(|| {
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
                // A lane I don't own: ignore (the plugin is a receiver there).
                let owner = st.node.with_song(|s| s.state(&branch).and_then(|x| x.owner(&lane))).unwrap_or(None);
                if owner.is_some() && owner != Some(me) {
                    return Response::Ok;
                }
                let frames = st.lane_block(&lane, pos, &pcm, ci.sample_rate, ci.channels);
                for (peer, f) in frames {
                    let _ = st.node.send_frame(&peer, &f);
                }
                Response::Ok
            }
            Request::TalkAudio { pcm } => {
                let Some(ci) = st.clients.get(&conn).cloned() else { return Response::Error("say hello first".into()) };
                let ClientKind::TalkSend { name, to } = ci.kind.clone() else { return Response::Error("not a Talk sender".into()) };
                let keyname = LaneId::new(&format!("__talk:{name}"));
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
            Request::RecordTalk { lane, start } => {
                if start {
                    let Some(ci) = st.clients.get(&conn).cloned() else { return Response::Error("say hello first".into()) };
                    let ClientKind::TalkReceive { from } = ci.kind.clone() else { return Response::Error("not a Talk receiver".into()) };
                    let from = from.or_else(|| st.incoming.keys().map(|(id, _)| *id).next());
                    let Some(from) = from else { return Response::Error("nobody is sending".into()) };
                    st.talk_record = Some((lane, from, Vec::new(), st.last_pos, ci.sample_rate, ci.channels));
                    Response::Ok
                } else {
                    let Some((lane, who, pcm, start_pos, sr, ch)) = st.talk_record.take() else { return Response::Error("not recording".into()) };
                    if pcm.is_empty() {
                        return Response::Error("nothing recorded".into());
                    }
                    let tempo = st.node.with_song(|s| s.state(&branch).map(|x| x.transport.tempo).unwrap_or(120.0)).unwrap_or(120.0);
                    let plan = flit_core::chunk::ChunkPlan { sample_rate: sr, channels: ch, bpm: tempo, time_sig: Default::default() };
                    let audio = RenderedAudio { sample_rate: sr, channels: ch, start: start_pos, samples: pcm, print_point: PrintPoint::Raw };
                    let (opus, flac) = match st.node.with_song(|s| st.live.encoder.encode(s.blobs(), &audio, &plan)) {
                        Ok(Ok(v)) => v,
                        Ok(Err(e)) => return Self::err(e),
                        Err(e) => return Self::err(e),
                    };
                    let payload = AudioPayload { start: start_pos, sample_rate: sr, bit_depth: 24, channels: ch, print_point: PrintPoint::Raw, opus_chunks: opus, flac_chunks: flac, credit: Some(who) };
                    if st.node.with_song(|s| s.state(&branch).and_then(|x| x.owner(&lane))).unwrap_or(None).is_none() {
                        let _ = song!(|s| s.claim_lane(&key, &branch, &lane, LaneKind::Part, None, &host, time));
                    }
                    publish_event!(|s| s.audio(&key, &branch, &lane, payload.clone(), &host, time))
                }
            }
            Request::Preview { at } => {
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
                        let state = match st.node.with_song(|s| s.chains().state_at(&at)) {
                            Ok(Ok(s)) => s,
                            Ok(Err(e)) => return Self::err(e),
                            Err(e) => return Self::err(e),
                        };
                        let applied = Receiver::apply_state(&mut st.live.bridge, &state);
                        tracing::info!("preview {}: {} settings, {} mixer changes, {} devices", at.short(), applied.settings, applied.mixer, applied.devices);
                        if !applied.manual.is_empty() {
                            self.push_all(Push::Warning(format!("{} can't add or remove plugins from outside. At this moment: {}", st.live.host(), applied.manual.join("; "))));
                        }
                        st.preview = Some(at);
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
                Ok(evs) if evs.is_empty() => {
                    self.push_all(Push::Warning(format!("Asked {} to save; \"{}\" lands as soon as the file is written.", st.live.host(), name)));
                    Response::Stamped(Hash::ZERO)
                }
                Ok(evs) => Response::Stamped(evs.iter().find(|e| e.kind() == Kind::Version).map(|e| e.id).unwrap_or(Hash::ZERO)),
                Err(e) => Response::Error(e),
            },
            Request::Branches => {
                let infos = st
                    .node
                    .with_song(|s| {
                        s.chains()
                            .branches()
                            .into_iter()
                            .map(|(b, name)| {
                                let base = s.chains().branch_base(&b);
                                let from_pos = base.and_then(|(_, from)| s.chains().get(&from).map(|x| x.wall().pos));
                                BranchInfo { current: b == branch, branch: b, name, parent: base.map(|(p, _)| p), from: base.map(|(_, f)| f), from_pos }
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                Response::Branches(infos)
            }
            Request::History { branch: l } => Response::History(st.history_rows(&l)),
            Request::GoBack { to } => {
                let export = st.export_version_at(&branch, &to);
                let ev: Stamp = song!(|s| s.go_back(&key, &branch, to, &host, time));
                st.node.publish(std::slice::from_ref(&ev));
                st.status_dirty = true;
                for n in st.follow_history().into_iter().chain(export) {
                    self.push_all(Push::Warning(n));
                }
                Response::Stamped(ev.id)
            }
            Request::NewBranch { name, from } => {
                let new_branch = song!(|s| s.new_branch(&key, &name, from, &host, time));
                let ev = st.node.with_song(|s| s.chains().get(&new_branch).cloned()).ok().flatten();
                if let Some(ev) = ev {
                    st.node.publish(&[ev]);
                }
                st.live.set_branch(new_branch);
                for n in st.follow_history() {
                    self.push_all(Push::Warning(n));
                }
                Response::Branch(new_branch)
            }
            Request::SwitchBranch { branch: l } => {
                if !st.node.with_song(|s| s.chains().has_branch(&l)).unwrap_or(false) {
                    return Response::Error("no such branch".into());
                }
                st.live.set_branch(l);
                for n in st.follow_history() {
                    self.push_all(Push::Warning(n));
                }
                Response::Ok
            }
            Request::Undo { target } => {
                let ev: Stamp = song!(|s| s.undo(&key, &branch, target, &host, time));
                st.node.publish(std::slice::from_ref(&ev));
                st.status_dirty = true;
                for n in st.follow_history() {
                    self.push_all(Push::Warning(n));
                }
                Response::Stamped(ev.id)
            }
            Request::Lanes => Response::Lanes(st.node.with_song(|s| lane_infos(s, &branch, &me)).unwrap_or_default()),
            Request::ClaimLane { lane, kind, name } => publish_event!(|s| s.claim_lane(&key, &branch, &lane, kind, name.as_deref(), &host, time)),
            Request::HandOverLane { lane, to } => {
                let kind = st.node.with_song(|s| s.state(&branch).map(|x| x.kind(&lane)).unwrap_or_default()).unwrap_or_default();
                publish_event!(|s| s.assign_lane(&key, &branch, &lane, to, kind, None, &host, time))
            }
            Request::BringPreview { lane, source_branch } => {
                let p = song!(|s| s.bring_preview(&branch, &lane, s, &source_branch, None));
                let allowed = st.node.with_song(|s| s.can_bring(&me, &branch, &lane)).unwrap_or(false);
                Response::Preview(PreviewInfo { changes: p.facts.len(), conflict: p.conflict, flac_needed: p.flac_needed.len(), allowed })
            }
            Request::BringIn { .. } | Request::JoinWithCode { .. } | Request::AcceptInvite { .. } | Request::Invite { .. } => unreachable!("handled above"),
            Request::OfferLane { lane } => publish_event!(|s| s.offer_lane(&key, &branch, &lane, &host, time)),
            Request::Chat { text, pinned } => {
                let ev: Stamp = song!(|s| s.chat(&key, &branch, &text, pinned, &host, time));
                st.node.publish(std::slice::from_ref(&ev));
                let row = st.chat_rows().into_iter().find(|c| c.id == ev.id);
                if let Some(c) = row {
                    self.push_all(Push::Chat(c));
                }
                Response::Stamped(ev.id)
            }
            Request::ChatList => Response::Chat(st.chat_rows()),
            Request::Settings(p) => {
                if let Some(d) = p.download_full_quality {
                    st.settings.download_full_quality = d;
                }
                if let Some(n) = p.display_name {
                    st.settings.display_name = Some(n);
                    let _ = st.settings.save();
                    st.announce_name();
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
                let _ = st.settings.save();
                st.registry.save(&st.registry_path);
                st.status_dirty = true;
                Response::Ok
            }
            Request::Fake(cmd) => {
                let Some(f) = st.live.bridge.fake() else { return Response::Error("agent is not running FakeHost".into()) };
                match cmd {
                    FakeCmd::AddTrack { lane, name, kind, plugin, tone_hz } => f.add_track(&lane, &name, kind, plugin, tone_hz),
                    FakeCmd::Twist { lane, slot, param, value } => f.twist(&lane, DeviceSlot(slot), &param, value),
                    FakeCmd::Sweep { lane, slot, param, from, to, steps } => f.sweep(&lane, DeviceSlot(slot), &param, from, to, steps),
                    FakeCmd::Volume { lane, value } => f.set_volume(&lane, value),
                    FakeCmd::Tempo { bpm } => f.set_tempo(bpm),
                    FakeCmd::Play => f.play(),
                    FakeCmd::Stop => f.stop(),
                    FakeCmd::SaveProject { path } => {
                        if let Err(e) = f.save_project(&path) {
                            return Self::err(e);
                        }
                    }
                    FakeCmd::Param { lane, slot, param } => return Response::Value(f.param(&lane, DeviceSlot(slot), &param)),
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
