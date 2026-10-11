//! The node: one iroh endpoint per Name, one open song plus the containers
//! it holds alongside (orgs, the personal shelf; PACK 2), connections to
//! their crews. Everything the agent needs from the network is here.

use crate::blobs::BlobHandler;
use crate::proto::*;
use crate::quota::{Quota, QuotaState};
use crate::sync::{SyncHandler, SyncOutbox};
use crate::{Error, Result, ALPN_BLOB, ALPN_JOIN, ALPN_STREAM, ALPN_SYNC};
use flit_audio::StreamConfig;
use flit_core::{Access, Act, ActId, AppendOutcome, BlockList, ContainerKind, Fact, Hash, Id, Keypair, Outbound, Payload, Song, SongId, Stamper, UserSettings};
use std::collections::BTreeSet;
use iroh::address_lookup::MemoryLookup;
use iroh::endpoint::presets;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, RelayMode, Watcher};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};

pub fn now_ms() -> u64 {
    flit_core::now_ms()
}

#[derive(Clone, Debug)]
pub struct NodeConfig {
    /// The Name key: identity and network address.
    pub key: Keypair,
    pub display_name: String,
    /// Tests and LAN-only use: no relay, no DNS discovery, loopback bind.
    pub local_only: bool,
    /// Privacy: never expose my IP to the crew; everything goes via relay.
    pub relay_only: bool,
    pub quota: Quota,
    pub settings: UserSettings,
    /// Treat this connection as unmetered (for the Wi-Fi-only policy).
    pub unmetered: bool,
}

impl NodeConfig {
    pub fn new(key: Keypair, display_name: &str) -> Self {
        Self { key, display_name: display_name.to_string(), local_only: false, relay_only: false, quota: Quota::default(), settings: UserSettings::default(), unmetered: true }
    }
    pub fn local(mut self) -> Self {
        self.local_only = true;
        self
    }
}

/// What the network tells the agent / panel.
#[derive(Clone, Debug)]
pub enum NetEvent {
    PeerUp(Id),
    PeerDown(Id),
    /// Acts from a peer were validated and applied (optimistically or sealed).
    Imported { from: Id, ids: Vec<ActId> },
    /// Facts arrived for acts that were already here.
    Sealed { from: Id, ids: Vec<ActId> },
    /// An act from a peer was rejected (never applied). Logged, shown.
    Rejected { from: Id, reason: String },
    /// Door 1: someone asked to join with our song code.
    JoinRequest { from: Id, display_name: String },
    /// Door 2: an owner added our ID to their song, or invited us to an org.
    Invite { from: Id, song: SongId, name: String, kind: ContainerKind },
    /// We are in: a song was joined and opened.
    Joined { song: SongId },
    BlobFetched { hash: Hash, from: Id },
    BlobFailed { hash: Hash, from: Id },
    QuotaExceeded(Id),
    StreamOpened { from: Id, stream: u16, config: StreamConfig },
    StreamFrame { from: Id, frame: StreamFrame },
    StreamClosed { from: Id, stream: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerStatus {
    pub id: Id,
    pub connected: bool,
    pub over_quota: bool,
}

pub(crate) struct PendingJoin {
    pub request: JoinRequest,
    pub reply: oneshot::Sender<JoinReply>,
}

pub(crate) struct Inner {
    pub key: Keypair,
    pub display_name: Mutex<String>,
    pub ep: Endpoint,
    pub lookup: MemoryLookup,
    pub local_only: bool,
    pub song: Mutex<Option<Song>>,
    /// Orgs and the personal shelf: synced alongside the open song.
    pub containers: Mutex<HashMap<SongId, Song>>,
    /// Who stamps crew acts on this machine (set by the agent).
    pub stamper: Mutex<Option<Stamper>>,
    pub blocklist: Mutex<BlockList>,
    pub quota: Mutex<QuotaState>,
    pub settings: Mutex<UserSettings>,
    pub unmetered: bool,
    pub peers: Mutex<HashMap<Id, Vec<SyncOutbox>>>,
    pub known: Mutex<HashMap<Id, AddrInfo>>,
    pub pending_joins: Mutex<Vec<PendingJoin>>,
    pub invites: Mutex<Vec<(JoinInvite, AddrInfo)>>,
    pub origin: Mutex<HashMap<ActId, Id>>,
    pub events_tx: broadcast::Sender<NetEvent>,
    pub streams_out: Mutex<HashMap<Id, iroh::endpoint::Connection>>,
    pub next_stream: Mutex<u16>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Inner({})", self.key.id())
    }
}

/// Door 2 message: an owner invites an ID.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct JoinInvite {
    pub song: SongId,
    pub name: String,
    pub genesis: Act,
    pub genesis_fact: Option<Fact>,
    pub from: Id,
    pub peers: Vec<AddrInfo>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum JoinMsg {
    Request(JoinRequest),
    Invite(JoinInvite),
}

#[derive(Clone)]
pub struct Node {
    pub(crate) inner: Arc<Inner>,
    router: Router,
}

impl Node {
    pub async fn bind(cfg: NodeConfig) -> Result<Node> {
        let secret = iroh::SecretKey::from_bytes(&cfg.key.secret_bytes());
        let lookup = MemoryLookup::new();
        let mut b = if cfg.local_only {
            Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
        } else {
            Endpoint::builder(presets::N0)
        };
        b = b.secret_key(secret).address_lookup(lookup.clone());
        if cfg.local_only {
            b = b.clear_ip_transports().bind_addr("127.0.0.1:0").map_err(|e| Error::Protocol(e.to_string()))?;
        }
        if cfg.relay_only && !cfg.local_only {
            b = b.clear_ip_transports();
        }
        let ep = b.bind().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let (events_tx, _) = broadcast::channel(4096);
        let inner = Arc::new(Inner {
            key: cfg.key.clone(),
            display_name: Mutex::new(cfg.display_name.clone()),
            ep: ep.clone(),
            lookup,
            local_only: cfg.local_only,
            song: Mutex::new(None),
            containers: Mutex::new(HashMap::new()),
            stamper: Mutex::new(None),
            blocklist: Mutex::new(BlockList::in_memory()),
            quota: Mutex::new(QuotaState::new(cfg.quota)),
            settings: Mutex::new(cfg.settings.clone()),
            unmetered: cfg.unmetered,
            peers: Mutex::new(HashMap::new()),
            known: Mutex::new(HashMap::new()),
            pending_joins: Mutex::new(Vec::new()),
            invites: Mutex::new(Vec::new()),
            origin: Mutex::new(HashMap::new()),
            events_tx,
            streams_out: Mutex::new(HashMap::new()),
            next_stream: Mutex::new(1),
        });
        let router = Router::builder(ep)
            .accept(ALPN_JOIN, JoinHandler(inner.clone()))
            .accept(ALPN_SYNC, SyncHandler(inner.clone()))
            .accept(ALPN_BLOB, BlobHandler(inner.clone()))
            .accept(ALPN_STREAM, StreamHandler(inner.clone()))
            .spawn();
        Ok(Node { inner, router })
    }

    pub fn id(&self) -> Id {
        self.inner.key.id()
    }

    /// A cheap handle for async work outside a lock. Same node.
    pub fn handle(&self) -> Node {
        self.clone()
    }

    /// Remember how to reach an ID.
    pub fn learn(&self, addr: &AddrInfo) {
        self.inner.learn(addr);
    }

    pub fn key(&self) -> &Keypair {
        &self.inner.key
    }

    /// What join requests call me.
    pub fn set_display_name(&self, name: &str) {
        *self.inner.display_name.lock().unwrap() = name.to_string();
    }

    /// Who stamps crew acts (approve, invite) on this machine.
    pub fn set_stamper(&self, who: Stamper) {
        *self.inner.stamper.lock().unwrap() = Some(who);
    }

    pub fn stamper(&self) -> Option<Stamper> {
        self.inner.stamper.lock().unwrap().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NetEvent> {
        self.inner.events_tx.subscribe()
    }

    /// How to reach me right now.
    pub fn addr_info(&self) -> AddrInfo {
        self.inner.addr_info()
    }

    /// Wait until the endpoint knows an address (relay or direct).
    pub async fn wait_ready(&self, timeout: Duration) -> bool {
        if self.inner.local_only {
            return true;
        }
        let mut w = self.inner.ep.watch_addr();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let a = w.get();
            if a.ip_addrs().next().is_some() || a.relay_urls().next().is_some() {
                return true;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return false,
                r = w.updated() => { if r.is_err() { return false; } }
            }
        }
    }

    // ----- song -----

    pub fn open_song(&self, song: Song) {
        *self.inner.song.lock().unwrap() = Some(song);
        self.inner.refresh_org_links();
    }

    pub fn close_song(&self) -> Option<Song> {
        self.inner.song.lock().unwrap().take()
    }

    // ----- containers (orgs, personal shelf) -----

    /// Hold (and sync) a container alongside the open song.
    pub fn add_container(&self, song: Song) {
        self.inner.containers.lock().unwrap().insert(song.id(), song);
        self.inner.refresh_org_links();
    }

    pub fn remove_container(&self, id: &SongId) -> Option<Song> {
        let s = self.inner.containers.lock().unwrap().remove(id);
        self.inner.refresh_org_links();
        s
    }

    /// Run `f` on a container (the open song or a held one).
    pub fn with_container<R>(&self, id: &SongId, f: impl FnOnce(&mut Song) -> R) -> Result<R> {
        self.inner.with_container(id, f).ok_or(Error::NoSong)
    }

    /// Every container here: the open song first, then the held ones.
    pub fn container_ids(&self) -> Vec<SongId> {
        self.inner.container_ids()
    }

    /// Ids of the held containers of one kind (orgs, or the shelf).
    pub fn containers_of(&self, kind: ContainerKind) -> Vec<SongId> {
        self.inner.containers.lock().unwrap().iter().filter(|(_, s)| s.kind() == kind).map(|(id, _)| *id).collect()
    }

    /// Songs with access = org learn their org's members from the org
    /// container; call after anything that changes a crew.
    pub fn refresh_org_links(&self) {
        self.inner.refresh_org_links();
    }

    /// Run `f` with the open song. Short, synchronous work only.
    pub fn with_song<R>(&self, f: impl FnOnce(&mut Song) -> R) -> Result<R> {
        let mut g = self.inner.song.lock().unwrap();
        let s = g.as_mut().ok_or(Error::NoSong)?;
        Ok(f(s))
    }

    pub fn song_id(&self) -> Option<SongId> {
        self.inner.song.lock().unwrap().as_ref().map(|s| s.id())
    }

    /// Share this song: the code to give a friend.
    pub fn song_code(&self) -> Result<crate::SongCode> {
        let song = self.song_id().ok_or(Error::NoSong)?;
        Ok(crate::SongCode { song, owner: self.id(), owner_addr: self.addr_info() })
    }

    /// Send everything the song and the held containers have queued (my
    /// acts as ACT, the keeper's seals as MOMENT) to the sessions of each.
    /// Call after any stamping.
    pub fn publish(&self) -> usize {
        let mut n = 0;
        for id in self.inner.container_ids() {
            let items: Vec<Outbound> = self.inner.with_container(&id, |s| s.take_outbox()).unwrap_or_default();
            n += items.len();
            self.inner.broadcast(&id, items, None);
        }
        if n > 0 {
            self.inner.refresh_org_links();
        }
        n
    }

    // ----- crew -----

    fn stamper_or_err(&self) -> Result<Stamper> {
        self.stamper().ok_or_else(|| Error::Protocol("no being set on this node".into()))
    }

    /// Door 1: ask to join with a song code. Waits for the owner (up to
    /// `wait`). On Welcome the song is created in `dir` and opened.
    pub async fn request_join(&self, code: &crate::SongCode, dir: &std::path::Path, wait: Duration) -> Result<JoinReply> {
        self.join_with(code, dir, wait, false).await
    }

    /// The same for an org or shelf: on Welcome the container is created in
    /// `dir` and held alongside the open song (not opened as the song).
    pub async fn request_join_container(&self, code: &crate::SongCode, dir: &std::path::Path, wait: Duration) -> Result<JoinReply> {
        self.join_with(code, dir, wait, true).await
    }

    async fn join_with(&self, code: &crate::SongCode, dir: &std::path::Path, wait: Duration, as_container: bool) -> Result<JoinReply> {
        let addr = code.owner_addr.to_endpoint_addr().ok_or(Error::BadCode)?;
        self.inner.learn(&code.owner_addr);
        let conn = self.inner.ep.connect(addr, ALPN_JOIN).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let from = self.id();
        let display_name = self.inner.display_name.lock().unwrap().clone();
        let sig = self.inner.key.sign(&JoinRequest::signing_bytes(&code.song, &from, &display_name));
        let req = JoinRequest { song: code.song, from, display_name, addr: self.addr_info(), sig };
        write_msg(&mut send, &JoinMsg::Request(req)).await?;
        send.finish().ok();
        let reply: Option<JoinReply> = tokio::time::timeout(wait, read_msg(&mut recv)).await.map_err(|_| Error::Protocol("owner did not answer in time".into()))??;
        let reply = reply.ok_or_else(|| Error::Protocol("connection closed".into()))?;
        if let JoinReply::Welcome { name, genesis, genesis_fact, peers } = &reply {
            if genesis.id != code.song {
                return Err(Error::Protocol("owner sent a different song".into()));
            }
            let song = Song::join(dir, genesis.clone(), genesis_fact.clone(), name, now_ms())?;
            if as_container {
                self.add_container(song);
            } else {
                self.open_song(song);
            }
            for p in peers {
                self.inner.learn(p);
            }
            let _ = self.inner.events_tx.send(NetEvent::Joined { song: code.song });
            self.connect_peer_for(&code.owner_addr, code.song).await.ok();
        }
        Ok(reply)
    }

    /// Door 1, owner side: approve a pending request (stamps the Crew act,
    /// answers the asker).
    pub fn approve(&self, id: Id, owner: bool) -> Result<Option<Act>> {
        let who = self.stamper_or_err()?;
        let mut appended = None;
        let (genesis, genesis_fact, name, already) = self.with_song(|s| {
            let already = s.crew().is_member(&id);
            (s.genesis().clone(), s.chains().fact_for(&s.genesis().id).cloned(), s.name().to_string(), already)
        })?;
        if !already {
            let act = self.with_song(|s| s.approve(&who, id, owner, now_ms()))??;
            self.publish();
            appended = Some(act);
        }
        let peers = self.inner.known_peers();
        let mut pend = self.inner.pending_joins.lock().unwrap();
        let mut i = 0;
        while i < pend.len() {
            if pend[i].request.from == id {
                let p = pend.remove(i);
                let _ = p.reply.send(JoinReply::Welcome { name: name.clone(), genesis: genesis.clone(), genesis_fact: genesis_fact.clone(), peers: peers.clone() });
            } else {
                i += 1;
            }
        }
        Ok(appended)
    }

    pub fn deny(&self, id: Id) {
        let mut pend = self.inner.pending_joins.lock().unwrap();
        pend.retain_mut(|p| p.request.from != id);
    }

    pub fn pending_join_requests(&self) -> Vec<(Id, String)> {
        self.inner.pending_joins.lock().unwrap().iter().map(|p| (p.request.from, p.request.display_name.clone())).collect()
    }

    /// Door 2, owner side: add a friend's ID to the open song. Stamps the
    /// Crew act and sends them an invite (they tap Accept on their side).
    pub async fn invite(&self, friend: &AddrInfo, owner: bool) -> Result<Act> {
        let song = self.song_id().ok_or(Error::NoSong)?;
        self.invite_to(&song, friend, owner).await
    }

    /// The same for any container. An org stamps `Invite` (membership
    /// needs the friend's `Accept`); a song approves at once.
    pub async fn invite_to(&self, container: &SongId, friend: &AddrInfo, owner: bool) -> Result<Act> {
        let who = self.stamper_or_err()?;
        let (act, genesis, genesis_fact, name) = self.with_container(container, |s| -> Result<_> {
            let main = flit_core::main_history();
            let act = if s.kind() == ContainerKind::Org {
                if s.crew().is_member(&friend.id) || s.crew().is_invited(&friend.id) {
                    s.chains().sequence(&main).into_iter().rev().find(|e| matches!(e.payload(), Payload::Crew(flit_core::CrewAction::Invite { id } | flit_core::CrewAction::Approve { id, .. }) if *id == friend.id)).cloned().ok_or(Error::NoSong)?
                } else {
                    s.invite(&who, friend.id, now_ms())?
                }
            } else if s.crew().is_member(&friend.id) {
                s.chains().sequence(&main).into_iter().find(|e| matches!(e.payload(), Payload::Crew(flit_core::CrewAction::Approve { id, .. }) if *id == friend.id)).cloned().ok_or(Error::NoSong)?
            } else {
                s.approve(&who, friend.id, owner, now_ms())?
            };
            Ok((act, s.genesis().clone(), s.chains().fact_for(&s.genesis().id).cloned(), s.name().to_string()))
        })??;
        self.publish();
        self.inner.learn(friend);
        let addr = friend.to_endpoint_addr().ok_or(Error::BadCode)?;
        let conn = self.inner.ep.connect(addr, ALPN_JOIN).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let inv = JoinInvite { song: genesis.id, name, genesis, genesis_fact, from: self.id(), peers: self.inner.known_peers() };
        write_msg(&mut send, &JoinMsg::Invite(inv)).await?;
        send.finish().ok();
        let _: Option<JoinReply> = read_msg(&mut recv).await.unwrap_or(None);
        Ok(act)
    }

    pub fn pending_invites(&self) -> Vec<JoinInvite> {
        self.inner.invites.lock().unwrap().iter().map(|(i, _)| i.clone()).collect()
    }

    /// Door 2, friend side: accept an invite, create the song locally and
    /// start syncing with the owner.
    pub async fn accept_invite(&self, song: SongId, dir: &std::path::Path) -> Result<()> {
        self.accept_invite_with(song, dir, false).await
    }

    /// Accept an org (or shelf) invite: the container is created in `dir`
    /// and held; the caller then stamps `Accept` in it.
    pub async fn accept_invite_container(&self, song: SongId, dir: &std::path::Path) -> Result<()> {
        self.accept_invite_with(song, dir, true).await
    }

    async fn accept_invite_with(&self, song: SongId, dir: &std::path::Path, as_container: bool) -> Result<()> {
        let found = {
            let mut inv = self.inner.invites.lock().unwrap();
            let pos = inv.iter().position(|(i, _)| i.song == song);
            pos.map(|p| inv.remove(p))
        };
        let (inv, owner_addr) = found.ok_or_else(|| Error::Protocol("no such invite".into()))?;
        let s = Song::join(dir, inv.genesis.clone(), inv.genesis_fact.clone(), &inv.name, now_ms())?;
        if as_container {
            self.add_container(s);
        } else {
            self.open_song(s);
        }
        for p in &inv.peers {
            self.inner.learn(p);
        }
        let _ = self.inner.events_tx.send(NetEvent::Joined { song });
        self.connect_peer_for(&owner_addr, song).await
    }

    /// Forget an invite without answering (the inbox's Decline for a song;
    /// an org Decline is an act and needs the container, so it declines by
    /// forgetting too).
    pub fn forget_invite(&self, song: SongId) {
        self.inner.invites.lock().unwrap().retain(|(i, _)| i.song != song);
    }

    pub fn block(&self, id: Id) {
        let _ = self.inner.blocklist.lock().unwrap().block(id);
        self.inner.drop_peer(&id);
    }

    pub fn set_blocklist(&self, list: BlockList) {
        *self.inner.blocklist.lock().unwrap() = list;
    }

    pub fn is_blocked(&self, id: &Id) -> bool {
        self.inner.blocklist.lock().unwrap().is_blocked(id)
    }

    // ----- sync -----

    /// Open a sync session for the open song with a peer (idempotent-ish:
    /// a second session to the same peer is harmless).
    pub async fn connect_peer(&self, addr: &AddrInfo) -> Result<()> {
        let song = self.song_id().ok_or(Error::NoSong)?;
        self.connect_peer_for(addr, song).await
    }

    /// Open a sync session for one container with a peer.
    pub async fn connect_peer_for(&self, addr: &AddrInfo, container: SongId) -> Result<()> {
        self.inner.learn(addr);
        let ep_addr = addr.to_endpoint_addr().ok_or(Error::BadCode)?;
        let conn = self.inner.ep.connect(ep_addr, ALPN_SYNC).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let inner = self.inner.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::sync::run_session(inner, conn, Some(container)).await {
                tracing::debug!("sync session ended: {e}");
            }
        });
        Ok(())
    }

    /// Dial, for every container, every member we know an address for and
    /// have no session with for that container.
    pub async fn connect_crew(&self) -> usize {
        let known = self.inner.known.lock().unwrap().clone();
        let mut n = 0;
        for container in self.inner.container_ids() {
            let members = self.inner.members_of(&container);
            let connected: Vec<Id> = {
                let peers = self.inner.peers.lock().unwrap();
                peers.iter().filter(|(_, v)| v.iter().any(|o| o.container == container)).map(|(k, _)| *k).collect()
            };
            for m in members {
                if m == self.id() || connected.contains(&m) {
                    continue;
                }
                if let Some(a) = known.get(&m) {
                    if self.connect_peer_for(a, container).await.is_ok() {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// Ask every connected peer for whatever came after my heads (after a
    /// reconnect, or when the chains report missing prevs).
    pub fn catch_up(&self) {
        let peers = self.inner.peers.lock().unwrap();
        for outboxes in peers.values() {
            for ob in outboxes {
                if let Ok(heads) = crate::sync::my_heads(&self.inner, &ob.container) {
                    ob.send(SyncMsg::After(heads));
                }
            }
        }
    }

    pub fn peers(&self) -> Vec<PeerStatus> {
        let connected = self.inner.peers.lock().unwrap();
        let quota = self.inner.quota.lock().unwrap();
        let members: Vec<Id> = self.with_song(|s| s.crew().members.iter().copied().collect()).unwrap_or_default();
        members
            .into_iter()
            .filter(|m| *m != self.id())
            .map(|id| PeerStatus { id, connected: connected.get(&id).map(|v| !v.is_empty()).unwrap_or(false), over_quota: quota.is_exceeded(&id) })
            .collect()
    }

    pub fn connected_peers(&self) -> Vec<Id> {
        self.inner.peers.lock().unwrap().iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| *k).collect()
    }

    /// Does any current connection go through a relay? None = not connected.
    pub fn relayed(&self) -> Option<bool> {
        if self.connected_peers().is_empty() {
            return None;
        }
        Some(self.inner.ep.addr().ip_addrs().next().is_none())
    }

    // ----- blobs -----

    /// Pull blobs from a peer, verified. Returns the ones that arrived.
    pub async fn fetch_blobs(&self, from: Id, hashes: Vec<Hash>) -> Result<Vec<Hash>> {
        crate::blobs::fetch(self.inner.clone(), from, hashes).await
    }

    /// Pull everything a history is missing: Opus first, then FLAC if the
    /// download policy says so (or `force_flac`).
    pub async fn fetch_missing(&self, history: &flit_core::HistoryId, force_flac: bool) -> Result<(usize, usize)> {
        let (opus, flac) = self.with_song(|s| (s.missing_opus(history), s.missing_flac(history)))?;
        let peers = self.connected_peers();
        let mut got_opus = 0;
        let mut got_flac = 0;
        for p in &peers {
            if !opus.is_empty() {
                got_opus += self.fetch_blobs(*p, opus.clone()).await.map(|v| v.len()).unwrap_or(0);
            }
        }
        let want_flac = force_flac || self.inner.settings.lock().unwrap().fetch_flac_in_background(self.inner.unmetered);
        if want_flac {
            for p in &peers {
                if !flac.is_empty() {
                    got_flac += self.fetch_blobs(*p, flac.clone()).await.map(|v| v.len()).unwrap_or(0);
                }
            }
        }
        Ok((got_opus, got_flac))
    }

    // ----- live streams -----

    /// Start sending a live stream (track, submix, reference mix or Talk)
    /// to one peer. Returns the stream number to use with `send_frame`.
    pub async fn open_stream(&self, to: Id, config: StreamConfig) -> Result<u16> {
        let conn = self.inner.stream_conn(&to).await?;
        let stream = {
            let mut n = self.inner.next_stream.lock().unwrap();
            let s = *n;
            *n = n.wrapping_add(1).max(1);
            s
        };
        let (mut send, _recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        write_msg(&mut send, &StreamCtl::Open { stream, config }).await?;
        send.finish().ok();
        Ok(stream)
    }

    pub fn send_frame(&self, to: &Id, frame: &StreamFrame) -> Result<()> {
        let conn = self.inner.streams_out.lock().unwrap().get(to).cloned().ok_or_else(|| Error::Protocol("no stream connection".into()))?;
        let bytes = postcard::to_allocvec(frame).expect("frame encodes");
        conn.send_datagram(bytes::Bytes::from(bytes)).map_err(|e| Error::Protocol(e.to_string()))
    }

    pub async fn close_stream(&self, to: Id, stream: u16) -> Result<()> {
        let conn = self.inner.stream_conn(&to).await?;
        let (mut send, _recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        write_msg(&mut send, &StreamCtl::Close { stream }).await?;
        send.finish().ok();
        Ok(())
    }

    pub async fn shutdown(self) {
        let _ = self.router.shutdown().await;
    }
}

impl Inner {
    pub fn addr_info(&self) -> AddrInfo {
        let a = self.ep.addr();
        let mut info = AddrInfo::from_endpoint_addr(&a);
        if self.local_only || info.ips.is_empty() {
            for s in self.ep.bound_sockets() {
                let ip: SocketAddr = if s.ip().is_unspecified() { SocketAddr::new([127, 0, 0, 1].into(), s.port()) } else { s };
                if !info.ips.contains(&ip) {
                    info.ips.push(ip);
                }
            }
        }
        info
    }

    /// Remember how to reach an ID (and tell iroh).
    pub fn learn(&self, addr: &AddrInfo) {
        if addr.id == self.key.id() {
            return;
        }
        if let Some(ea) = addr.to_endpoint_addr() {
            if !ea.is_empty() {
                self.lookup.add_endpoint_info(ea);
            }
        }
        self.known.lock().unwrap().insert(addr.id, addr.clone());
    }

    pub fn known_peers(&self) -> Vec<AddrInfo> {
        let mut v: Vec<AddrInfo> = self.known.lock().unwrap().values().cloned().collect();
        v.push(self.addr_info());
        v
    }

    pub fn emit(&self, ev: NetEvent) {
        let _ = self.events_tx.send(ev);
    }

    pub fn is_blocked(&self, id: &Id) -> bool {
        self.blocklist.lock().unwrap().is_blocked(id)
    }

    pub fn drop_peer(&self, id: &Id) {
        self.peers.lock().unwrap().remove(id);
        self.streams_out.lock().unwrap().remove(id);
    }

    /// Run `f` on a container: the open song, or one held alongside.
    /// (Locks `song` and `containers` one after the other, never nested.)
    pub fn with_container<R>(&self, id: &SongId, f: impl FnOnce(&mut Song) -> R) -> Option<R> {
        {
            let mut g = self.song.lock().unwrap();
            if let Some(s) = g.as_mut() {
                if s.id() == *id {
                    return Some(f(s));
                }
            }
        }
        let mut c = self.containers.lock().unwrap();
        c.get_mut(id).map(f)
    }

    pub fn container_ids(&self) -> Vec<SongId> {
        let mut ids: Vec<SongId> = self.song.lock().unwrap().as_ref().map(|s| s.id()).into_iter().collect();
        ids.extend(self.containers.lock().unwrap().keys().copied());
        ids
    }

    /// Who belongs to a container: its crew; in an org also the Names an
    /// owner invited (they must see the chain to accept); in a song with
    /// access = org also the org's members, when the org is held here.
    pub fn members_of(&self, id: &SongId) -> Vec<Id> {
        let Some((mut members, org, access, is_song)) = self.with_container(id, |s| {
            let mut m: Vec<Id> = s.crew().members.iter().copied().collect();
            if s.kind() == ContainerKind::Org {
                m.extend(s.crew().invited.iter().copied());
            }
            (m, s.org(), s.access(), s.kind().is_song())
        }) else {
            return vec![];
        };
        if is_song && access == Access::Org {
            if let Some(o) = org {
                if let Some(extra) = self.with_container(&o, |s| s.crew().members.iter().copied().collect::<Vec<Id>>()) {
                    for e in extra {
                        if !members.contains(&e) {
                            members.push(e);
                        }
                    }
                }
            }
        }
        members
    }

    pub fn member_of(&self, id: &SongId, peer: &Id) -> bool {
        self.members_of(id).contains(peer)
    }

    /// Songs with access = org read their org's members from the org
    /// container (for the MAY check); None when the org is not here.
    pub fn refresh_org_links(&self) {
        let orgs: HashMap<SongId, BTreeSet<Id>> = {
            let c = self.containers.lock().unwrap();
            c.iter().filter(|(_, s)| s.kind() == ContainerKind::Org).map(|(id, s)| (*id, s.crew().members.clone())).collect()
        };
        let mut g = self.song.lock().unwrap();
        if let Some(s) = g.as_mut() {
            if s.kind().is_song() {
                let members = s.org().and_then(|o| orgs.get(&o).cloned());
                s.set_org_members(members);
            }
        }
    }

    /// Send acts / moments of `container` to its sessions, except `except`.
    pub fn broadcast(&self, container: &SongId, items: Vec<Outbound>, except: Option<&Id>) {
        if items.is_empty() {
            return;
        }
        let msgs: Vec<SyncMsg> = items
            .into_iter()
            .map(|o| match o {
                Outbound::Act(a) => SyncMsg::Act(a),
                Outbound::Moment(a, f) => SyncMsg::Moment(a, f),
            })
            .collect();
        let peers = self.peers.lock().unwrap();
        for (id, outboxes) in peers.iter() {
            if Some(id) == except {
                continue;
            }
            for ob in outboxes.iter().filter(|o| o.container == *container) {
                for m in &msgs {
                    ob.send(m.clone());
                }
            }
        }
    }

    /// Import acts / moments that came from `from`. Validated by the
    /// chains; quota checked; rejected ones are reported, never applied.
    /// Returns the acts we are now waiting for (to ask the peer).
    pub fn import_from(self: &Arc<Self>, from: Id, container: &SongId, items: Vec<SyncMsg>) -> Vec<ActId> {
        let mut imported = Vec::new();
        let mut sealed = Vec::new();
        let mut forward: Vec<Outbound> = Vec::new();
        let mut fetch_acts: Vec<Act> = Vec::new();
        let mut want = Vec::new();
        let now = now_ms();
        let mut crew_changed = false;
        let Some(outbox) = self.with_container(container, |song| {
            for item in items {
                if !self.quota.lock().unwrap().allow_event(&from, now) {
                    self.emit(NetEvent::QuotaExceeded(from));
                    break;
                }
                let (act, fact) = match item {
                    SyncMsg::Act(a) => (a, None),
                    SyncMsg::Moment(a, f) => (a, Some(f)),
                    _ => continue,
                };
                if self.is_blocked(&act.name()) {
                    self.emit(NetEvent::Rejected { from, reason: format!("author {:?} is blocked", act.name()) });
                    continue;
                }
                let id = act.id;
                let known_before = song.chains().contains(&id);
                crew_changed |= matches!(act.payload(), Payload::Crew(_) | Payload::Settings(flit_core::SettingsChange::Org { .. }));
                let res = match fact.clone() {
                    Some(f) => song.import_moment(act.clone(), f, now),
                    None => song.import_act(act.clone(), now),
                };
                match res {
                    Ok(AppendOutcome::Appended) => {
                        if known_before {
                            sealed.push(id);
                        } else {
                            imported.push(id);
                            self.origin.lock().unwrap().insert(id, from);
                            fetch_acts.push(act.clone());
                        }
                        forward.push(match fact {
                            Some(f) => Outbound::Moment(act, f),
                            None => Outbound::Act(act),
                        });
                    }
                    Ok(AppendOutcome::Pending) => {}
                    Ok(AppendOutcome::Duplicate) => {
                        // the act was here; the fact may be new
                        if fact.is_some() && song.chains().is_sealed(&id) && !known_before {
                            sealed.push(id);
                        }
                    }
                    Ok(AppendOutcome::Equivocation) => {
                        self.emit(NetEvent::Rejected { from, reason: format!("{:?} stamped two moments with one number", act.being()) });
                    }
                    Err(e) => {
                        tracing::warn!("rejected act {id:?} from {from:?}: {e}");
                        self.emit(NetEvent::Rejected { from, reason: e.to_string() });
                    }
                }
            }
            want = song.chains().wanted();
            song.take_outbox()
        }) else {
            return want;
        };
        if crew_changed {
            self.refresh_org_links();
        }
        if !forward.is_empty() {
            self.broadcast(container, forward, Some(&from));
        }
        if !outbox.is_empty() {
            // the keeper's seals (and any History acts it stamped): everyone
            self.broadcast(container, outbox, None);
        }
        if !imported.is_empty() {
            self.emit(NetEvent::Imported { from, ids: imported });
        }
        if !sealed.is_empty() {
            self.emit(NetEvent::Sealed { from, ids: sealed });
        }
        if !fetch_acts.is_empty() {
            self.spawn_blob_fetch(from, *container, &fetch_acts);
        }
        want
    }

    /// Opus copies first; FLAC in the background per the download policy.
    fn spawn_blob_fetch(self: &Arc<Self>, from: Id, container: SongId, acts: &[Act]) {
        let mut opus = Vec::new();
        let mut flac = Vec::new();
        let mut version = Vec::new();
        let mut visit = |p: &Payload| match p {
            Payload::Audio(a) => {
                opus.extend(a.opus_chunks.iter().copied());
                flac.extend(a.flac_chunks.iter().copied());
            }
            Payload::Version(v) => {
                version.push(v.project);
                version.extend(v.manifest.iter().map(|m| m.hash));
            }
            // album covers travel like project files: small, wanted at once
            Payload::Album(flit_core::AlbumAction::SetCover(Some(h))) => version.push(*h),
            _ => {}
        };
        for a in acts {
            match a.payload() {
                Payload::Merge(m) => {
                    for inner in &m.acts {
                        visit(inner.payload());
                    }
                }
                p => visit(p),
            }
        }
        if opus.is_empty() && flac.is_empty() && version.is_empty() {
            return;
        }
        let background_flac = self.settings.lock().unwrap().fetch_flac_in_background(self.unmetered);
        let me = self.clone();
        tokio::spawn(async move {
            let _ = crate::blobs::fetch_into(me.clone(), from, container, opus).await;
            let _ = crate::blobs::fetch_into(me.clone(), from, container, version).await;
            if background_flac {
                let _ = crate::blobs::fetch_into(me, from, container, flac).await;
            }
        });
    }

    pub async fn stream_conn(&self, to: &Id) -> Result<iroh::endpoint::Connection> {
        if let Some(c) = self.streams_out.lock().unwrap().get(to).cloned() {
            if c.close_reason().is_none() {
                return Ok(c);
            }
        }
        let ep_id = crate::endpoint_id(to).ok_or(Error::BadCode)?;
        let addr = self.known.lock().unwrap().get(to).and_then(|a| a.to_endpoint_addr()).unwrap_or_else(|| EndpointAddr::new(ep_id));
        let conn = self.ep.connect(addr, ALPN_STREAM).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        self.streams_out.lock().unwrap().insert(*to, conn.clone());
        Ok(conn)
    }
}

/// Feed an act into a node as if a peer had sent it (tests).
pub fn inject_for_test(node: &Node, from: Id, act: Act) -> Vec<ActId> {
    let Some(song) = node.song_id() else { return vec![] };
    node.inner.import_from(from, &song, vec![SyncMsg::Act(act)])
}

// ----- protocol handlers -----

#[derive(Clone)]
pub(crate) struct JoinHandler(pub Arc<Inner>);

impl std::fmt::Debug for JoinHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JoinHandler")
    }
}

impl ProtocolHandler for JoinHandler {
    async fn accept(&self, conn: iroh::endpoint::Connection) -> std::result::Result<(), AcceptError> {
        let inner = self.0.clone();
        let from = crate::id_of(&conn.remote_id());
        if inner.is_blocked(&from) {
            conn.close(1u32.into(), b"blocked");
            return Ok(());
        }
        let (mut send, mut recv) = conn.accept_bi().await?;
        let msg: Option<JoinMsg> = read_msg(&mut recv).await.map_err(AcceptError::from_err)?;
        match msg {
            Some(JoinMsg::Request(req)) => {
                if req.from != from || !req.verify() {
                    write_msg(&mut send, &JoinReply::Refused("bad signature".into())).await.ok();
                    return Ok(());
                }
                inner.learn(&req.addr);
                let me = inner.key.id();
                let looked = inner.with_container(&req.song, |s| {
                    let welcome = (s.name().to_string(), s.genesis().clone(), s.chains().fact_for(&s.genesis().id).cloned());
                    (s.kind(), s.crew().is_member(&req.from), s.crew().is_invited(&req.from), s.crew().is_owner(&me), welcome)
                });
                let state = match looked {
                    None => Err("not this song"),
                    Some((kind, member, invited, i_own, welcome)) => {
                        if member || (kind == ContainerKind::Org && invited) || (kind.is_song() && inner.member_of(&req.song, &req.from)) {
                            Ok(Some(welcome))
                        } else if kind.is_song() && i_own {
                            Ok(None)
                        } else if kind.is_song() {
                            Err("ask an owner")
                        } else {
                            Err("not invited")
                        }
                    }
                };
                match state {
                    Err(reason) => {
                        write_msg(&mut send, &JoinReply::Refused(reason.into())).await.ok();
                    }
                    Ok(Some((name, genesis, genesis_fact))) => {
                        let peers = inner.known_peers();
                        write_msg(&mut send, &JoinReply::Welcome { name, genesis, genesis_fact, peers }).await.ok();
                    }
                    Ok(None) => {
                        let (tx, rx) = oneshot::channel();
                        inner.pending_joins.lock().unwrap().push(PendingJoin { request: req.clone(), reply: tx });
                        inner.emit(NetEvent::JoinRequest { from: req.from, display_name: req.display_name.clone() });
                        // Wait for the owner to tap Approve (or the asker to give up).
                        let reply = match tokio::time::timeout(Duration::from_secs(600), rx).await {
                            Ok(Ok(r)) => r,
                            _ => JoinReply::Pending,
                        };
                        inner.pending_joins.lock().unwrap().retain(|p| p.request.from != req.from);
                        write_msg(&mut send, &reply).await.ok();
                    }
                }
            }
            Some(JoinMsg::Invite(inv)) => {
                // The genesis must be its own author's; the inviter need not
                // be the author (any org owner invites). A bogus invite
                // fails later: the Accept needs an owner's Invite act.
                let author = inv.genesis.name();
                if inv.from != from || inv.genesis.id != inv.song || inv.genesis.check_id().is_err() || !inv.genesis.signed_by(&author) {
                    write_msg(&mut send, &JoinReply::Refused("bad invite".into())).await.ok();
                    return Ok(());
                }
                let kind = ContainerKind::from_genesis_property(inv.genesis.body.property.as_deref()).unwrap_or_default();
                let owner_addr = inv.peers.iter().find(|p| p.id == from).cloned().unwrap_or(AddrInfo { id: from, ips: vec![], relay: None });
                inner.learn(&owner_addr);
                {
                    let mut invites = inner.invites.lock().unwrap();
                    invites.retain(|(i, _)| i.song != inv.song);
                    invites.push((inv.clone(), owner_addr));
                }
                inner.emit(NetEvent::Invite { from, song: inv.song, name: inv.name.clone(), kind });
                write_msg(&mut send, &JoinReply::Pending).await.ok();
            }
            None => {}
        }
        send.finish().ok();
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct StreamHandler(pub Arc<Inner>);

impl std::fmt::Debug for StreamHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StreamHandler")
    }
}

impl ProtocolHandler for StreamHandler {
    async fn accept(&self, conn: iroh::endpoint::Connection) -> std::result::Result<(), AcceptError> {
        let inner = self.0.clone();
        let from = crate::id_of(&conn.remote_id());
        let song = inner.song.lock().unwrap().as_ref().map(|s| s.id());
        let member = song.map(|id| inner.member_of(&id, &from)).unwrap_or(false);
        if inner.is_blocked(&from) || !member {
            conn.close(1u32.into(), b"not in crew");
            return Ok(());
        }
        let c2 = conn.clone();
        let i2 = inner.clone();
        // control streams
        let ctl = tokio::spawn(async move {
            while let Ok((_send, mut recv)) = c2.accept_bi().await {
                if let Ok(Some(msg)) = read_msg::<StreamCtl>(&mut recv).await {
                    match msg {
                        StreamCtl::Open { stream, config } => i2.emit(NetEvent::StreamOpened { from, stream, config }),
                        StreamCtl::Close { stream } => i2.emit(NetEvent::StreamClosed { from, stream }),
                    }
                }
            }
        });
        // frames
        while let Ok(d) = conn.read_datagram().await {
            if let Ok(frame) = postcard::from_bytes::<StreamFrame>(&d) {
                if frame.packet.opus.len() <= 4000 {
                    inner.emit(NetEvent::StreamFrame { from, frame });
                }
            }
        }
        ctl.abort();
        Ok(())
    }
}
