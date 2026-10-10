//! The node: one iroh endpoint per user, one open song, connections to the
//! crew. Everything the agent needs from the network is here.

use crate::blobs::BlobHandler;
use crate::proto::*;
use crate::quota::{Quota, QuotaState};
use crate::sync::{SyncHandler, SyncOutbox};
use crate::{Error, Result, ALPN_BLOB, ALPN_JOIN, ALPN_STREAM, ALPN_SYNC};
use flit_core::{BlockList, Stamp, StampId, Hash, Id, Keypair, Payload, Song, SongId, UserSettings};
use flit_audio::StreamConfig;
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
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct NodeConfig {
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
    /// Events from a peer were validated and applied.
    Imported { from: Id, ids: Vec<StampId> },
    /// An event from a peer was rejected (never applied). Logged, shown.
    Rejected { from: Id, reason: String },
    /// Door 1: someone asked to join with our song code.
    JoinRequest { from: Id, display_name: String },
    /// Door 2: an owner added our ID to their song.
    Invite { from: Id, song: SongId, name: String },
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
    pub display_name: String,
    pub ep: Endpoint,
    pub lookup: MemoryLookup,
    pub local_only: bool,
    pub song: Mutex<Option<Song>>,
    pub blocklist: Mutex<BlockList>,
    pub quota: Mutex<QuotaState>,
    pub settings: Mutex<UserSettings>,
    pub unmetered: bool,
    pub peers: Mutex<HashMap<Id, Vec<SyncOutbox>>>,
    pub known: Mutex<HashMap<Id, AddrInfo>>,
    pub pending_joins: Mutex<Vec<PendingJoin>>,
    pub invites: Mutex<Vec<(JoinInvite, AddrInfo)>>,
    pub origin: Mutex<HashMap<StampId, Id>>,
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
    pub genesis: Stamp,
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
            display_name: cfg.display_name.clone(),
            ep: ep.clone(),
            lookup,
            local_only: cfg.local_only,
            song: Mutex::new(None),
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
    }

    pub fn close_song(&self) -> Option<Song> {
        self.inner.song.lock().unwrap().take()
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

    /// Events I appended locally: push to everyone connected.
    pub fn publish(&self, events: &[Stamp]) {
        if events.is_empty() {
            return;
        }
        self.inner.broadcast_events(events, None);
    }

    // ----- crew -----

    /// Door 1: ask to join with a song code. Waits for the owner (up to
    /// `wait`). On Welcome the song is created in `dir` and opened.
    pub async fn request_join(&self, code: &crate::SongCode, dir: &std::path::Path, wait: Duration) -> Result<JoinReply> {
        let addr = code.owner_addr.to_endpoint_addr().ok_or(Error::BadCode)?;
        self.inner.learn(&code.owner_addr);
        let conn = self.inner.ep.connect(addr, ALPN_JOIN).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let from = self.id();
        let sig = self.inner.key.sign(&JoinRequest::signing_bytes(&code.song, &from, &self.inner.display_name));
        let req = JoinRequest { song: code.song, from, display_name: self.inner.display_name.clone(), addr: self.addr_info(), sig };
        write_msg(&mut send, &JoinMsg::Request(req)).await?;
        send.finish().ok();
        let reply: Option<JoinReply> = tokio::time::timeout(wait, read_msg(&mut recv)).await.map_err(|_| Error::Protocol("owner did not answer in time".into()))??;
        let reply = reply.ok_or_else(|| Error::Protocol("connection closed".into()))?;
        if let JoinReply::Welcome { name, genesis, peers } = &reply {
            if genesis.id != code.song {
                return Err(Error::Protocol("owner sent a different song".into()));
            }
            let song = Song::join(dir, genesis.clone(), name)?;
            self.open_song(song);
            for p in peers {
                self.inner.learn(p);
            }
            let _ = self.inner.events_tx.send(NetEvent::Joined { song: code.song });
            self.connect_peer(&code.owner_addr).await.ok();
        }
        Ok(reply)
    }

    /// Door 1, owner side: approve a pending request (appends the Crew
    /// event, answers the asker).
    pub fn approve(&self, id: Id, owner: bool) -> Result<Option<Stamp>> {
        let host = flit_core::HostTag::plugin_only();
        let mut appended = None;
        let (genesis, name, already) = self.with_song(|s| {
            let already = s.crew().is_member(&id);
            (s.genesis().clone(), s.name().to_string(), already)
        })?;
        if !already {
            let ev = self.with_song(|s| s.approve(&self.inner.key, id, owner, &host, flit_core::Time::now(flit_core::SongPos::START)))??;
            self.publish(std::slice::from_ref(&ev));
            appended = Some(ev);
        }
        let peers = self.inner.known_peers();
        let mut pend = self.inner.pending_joins.lock().unwrap();
        let mut i = 0;
        while i < pend.len() {
            if pend[i].request.from == id {
                let p = pend.remove(i);
                let _ = p.reply.send(JoinReply::Welcome { name: name.clone(), genesis: genesis.clone(), peers: peers.clone() });
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

    /// Door 2, owner side: add a friend's ID. Appends the Crew event and
    /// sends them an invite (they tap Accept on their side).
    pub async fn invite(&self, friend: &AddrInfo, owner: bool) -> Result<Stamp> {
        let host = flit_core::HostTag::plugin_only();
        let (ev, genesis, name) = self.with_song(|s| -> Result<_> {
            let ev = if s.crew().is_member(&friend.id) {
                s.chains().branch_stamps(&flit_core::stamp::main_branch()).into_iter().find(|e| matches!(e.payload(), Payload::Crew(flit_core::CrewAction::Approve { id, .. }) if *id == friend.id)).cloned().ok_or(Error::NoSong)?
            } else {
                s.approve(&self.inner.key, friend.id, owner, &host, flit_core::Time::now(flit_core::SongPos::START))?
            };
            Ok((ev, s.genesis().clone(), s.name().to_string()))
        })??;
        self.publish(std::slice::from_ref(&ev));
        self.inner.learn(friend);
        let addr = friend.to_endpoint_addr().ok_or(Error::BadCode)?;
        let conn = self.inner.ep.connect(addr, ALPN_JOIN).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let inv = JoinInvite { song: genesis.id, name, genesis, from: self.id(), peers: self.inner.known_peers() };
        write_msg(&mut send, &JoinMsg::Invite(inv)).await?;
        send.finish().ok();
        let _: Option<JoinReply> = read_msg(&mut recv).await.unwrap_or(None);
        Ok(ev)
    }

    pub fn pending_invites(&self) -> Vec<JoinInvite> {
        self.inner.invites.lock().unwrap().iter().map(|(i, _)| i.clone()).collect()
    }

    /// Door 2, friend side: accept an invite, create the song locally and
    /// start syncing with the owner.
    pub async fn accept_invite(&self, song: SongId, dir: &std::path::Path) -> Result<()> {
        let found = {
            let mut inv = self.inner.invites.lock().unwrap();
            let pos = inv.iter().position(|(i, _)| i.song == song);
            pos.map(|p| inv.remove(p))
        };
        let (inv, owner_addr) = found.ok_or_else(|| Error::Protocol("no such invite".into()))?;
        let s = Song::join(dir, inv.genesis.clone(), &inv.name)?;
        self.open_song(s);
        for p in &inv.peers {
            self.inner.learn(p);
        }
        let _ = self.inner.events_tx.send(NetEvent::Joined { song });
        self.connect_peer(&owner_addr).await
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

    /// Open a sync session with a peer (idempotent-ish: a second session to
    /// the same peer is harmless).
    pub async fn connect_peer(&self, addr: &AddrInfo) -> Result<()> {
        self.inner.learn(addr);
        let ep_addr = addr.to_endpoint_addr().ok_or(Error::BadCode)?;
        let conn = self.inner.ep.connect(ep_addr, ALPN_SYNC).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
        let inner = self.inner.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::sync::run_session(inner, conn, true).await {
                tracing::debug!("sync session ended: {e}");
            }
        });
        Ok(())
    }

    /// Dial every crew member we know an address for and are not connected to.
    pub async fn connect_crew(&self) -> usize {
        let members: Vec<Id> = self.with_song(|s| s.crew().members.iter().copied().collect()).unwrap_or_default();
        let known = self.inner.known.lock().unwrap().clone();
        let connected: Vec<Id> = self.inner.peers.lock().unwrap().keys().copied().collect();
        let mut n = 0;
        for m in members {
            if m == self.id() || connected.contains(&m) {
                continue;
            }
            if let Some(a) = known.get(&m) {
                if self.connect_peer(a).await.is_ok() {
                    n += 1;
                }
            }
        }
        n
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

    // ----- blobs -----

    /// Pull blobs from a peer, verified. Returns the ones that arrived.
    pub async fn fetch_blobs(&self, from: Id, hashes: Vec<Hash>) -> Result<Vec<Hash>> {
        crate::blobs::fetch(self.inner.clone(), from, hashes).await
    }

    /// Pull everything a branch is missing: Opus first, then FLAC if the
    /// download policy says so (or `force_flac`).
    pub async fn fetch_missing(&self, branch: &flit_core::BranchId, force_flac: bool) -> Result<(usize, usize)> {
        let (opus, flac) = self.with_song(|s| (s.missing_opus(branch), s.missing_flac(branch)))?;
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

    /// Start sending a live stream (lane, submix, reference mix or Talk)
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

    /// Send events to every connected peer except `except`.
    pub fn broadcast_events(&self, events: &[Stamp], except: Option<&Id>) {
        let peers = self.peers.lock().unwrap();
        for (id, outboxes) in peers.iter() {
            if Some(id) == except {
                continue;
            }
            for ob in outboxes {
                ob.send(SyncMsg::Stamps(events.to_vec()));
            }
        }
    }

    /// Import events that came from `from`. Validated by the log; quota
    /// checked; rejected ones are reported, never applied.
    pub fn import_from(self: &Arc<Self>, from: Id, events: Vec<Stamp>) -> (Vec<StampId>, Vec<StampId>) {
        let mut imported = Vec::new();
        let mut want = Vec::new();
        let mut forward = Vec::new();
        let now = now_ms();
        {
            let mut g = self.song.lock().unwrap();
            let Some(song) = g.as_mut() else { return (imported, want) };
            for ev in events {
                if !self.quota.lock().unwrap().allow_event(&from, now) {
                    self.emit(NetEvent::QuotaExceeded(from));
                    break;
                }
                if self.is_blocked(&ev.name()) {
                    self.emit(NetEvent::Rejected { from, reason: format!("author {:?} is blocked", ev.name()) });
                    continue;
                }
                let id = ev.id;
                match song.import(ev.clone()) {
                    Ok(flit_core::AppendOutcome::Appended) => {
                        imported.push(id);
                        self.origin.lock().unwrap().insert(id, from);
                        forward.push(ev);
                    }
                    Ok(flit_core::AppendOutcome::Pending) => {}
                    Ok(flit_core::AppendOutcome::Duplicate) => {}
                    Err(e) => {
                        tracing::warn!("rejected event {id:?} from {from:?}: {e}");
                        self.emit(NetEvent::Rejected { from, reason: e.to_string() });
                    }
                }
            }
            want = song.chains().wanted();
        }
        if !forward.is_empty() {
            self.broadcast_events(&forward, Some(&from));
            self.emit(NetEvent::Imported { from, ids: imported.clone() });
            self.spawn_blob_fetch(from, &forward);
        }
        (imported, want)
    }

    /// Opus copies first; FLAC in the background per the download policy.
    fn spawn_blob_fetch(self: &Arc<Self>, from: Id, events: &[Stamp]) {
        let mut opus = Vec::new();
        let mut flac = Vec::new();
        let mut version = Vec::new();
        for ev in events {
            match ev.payload() {
                Payload::Audio(a) => {
                    opus.extend(a.opus_chunks.iter().copied());
                    flac.extend(a.flac_chunks.iter().copied());
                }
                Payload::Version(v) => {
                    version.push(v.project);
                    version.extend(v.manifest.iter().map(|m| m.hash));
                }
                Payload::Bring(b) => {
                    for inner in &b.facts {
                        if let Payload::Audio(a) = inner.payload() {
                            opus.extend(a.opus_chunks.iter().copied());
                            flac.extend(a.flac_chunks.iter().copied());
                        }
                    }
                }
                _ => {}
            }
        }
        if opus.is_empty() && flac.is_empty() && version.is_empty() {
            return;
        }
        let background_flac = self.settings.lock().unwrap().fetch_flac_in_background(self.unmetered);
        let me = self.clone();
        tokio::spawn(async move {
            let _ = crate::blobs::fetch(me.clone(), from, opus).await;
            let _ = crate::blobs::fetch(me.clone(), from, version).await;
            if background_flac {
                let _ = crate::blobs::fetch(me, from, flac).await;
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
                let state = {
                    let g = inner.song.lock().unwrap();
                    match g.as_ref() {
                        None => Err("no song open"),
                        Some(s) if s.id() != req.song => Err("not this song"),
                        Some(s) if !s.crew().is_owner(&inner.key.id()) => Err("ask an owner"),
                        Some(s) if s.crew().is_member(&req.from) => Ok(Some((s.name().to_string(), s.genesis().clone()))),
                        Some(_) => Ok(None),
                    }
                };
                match state {
                    Err(reason) => {
                        write_msg(&mut send, &JoinReply::Refused(reason.into())).await.ok();
                    }
                    Ok(Some((name, genesis))) => {
                        let peers = inner.known_peers();
                        write_msg(&mut send, &JoinReply::Welcome { name, genesis, peers }).await.ok();
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
                if inv.from != from || inv.genesis.id != inv.song || inv.genesis.name() != from {
                    write_msg(&mut send, &JoinReply::Refused("bad invite".into())).await.ok();
                    return Ok(());
                }
                if inv.genesis.verify().is_err() {
                    write_msg(&mut send, &JoinReply::Refused("bad genesis".into())).await.ok();
                    return Ok(());
                }
                let owner_addr = inv.peers.iter().find(|p| p.id == from).cloned().unwrap_or(AddrInfo { id: from, ips: vec![], relay: None });
                inner.learn(&owner_addr);
                inner.invites.lock().unwrap().push((inv.clone(), owner_addr));
                inner.emit(NetEvent::Invite { from, song: inv.song, name: inv.name.clone() });
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
        let member = inner.song.lock().unwrap().as_ref().map(|s| s.crew().is_member(&from)).unwrap_or(false);
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
