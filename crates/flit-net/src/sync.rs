//! Live sync of a container (a song, an org, a personal shelf) between two
//! peers. A session is about one container: the initiator says which in
//! its Hello, the other side answers with its heads for that container
//! (act counts per being per history, seal counts per history), each sends
//! the MOMENTs and ACTs the other lacks, then every new act or moment is
//! pushed as it happens. Out-of-order arrivals are held by the chains and
//! the missing prevs are asked for. One connection per (peer, container).

use crate::node::{now_ms, Inner, NetEvent};
use crate::proto::*;
use crate::{Error, Result};
use flit_core::{ActId, Id, SongId};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Where a session's outgoing messages go.
#[derive(Clone, Debug)]
pub struct SyncOutbox {
    tx: mpsc::UnboundedSender<SyncMsg>,
    /// The container this session carries.
    pub container: SongId,
}

impl SyncOutbox {
    pub fn send(&self, msg: SyncMsg) {
        let _ = self.tx.send(msg);
    }
}

#[derive(Clone)]
pub(crate) struct SyncHandler(pub Arc<Inner>);

impl std::fmt::Debug for SyncHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SyncHandler")
    }
}

impl ProtocolHandler for SyncHandler {
    async fn accept(&self, conn: Connection) -> std::result::Result<(), AcceptError> {
        let inner = self.0.clone();
        if let Err(e) = run_session(inner, conn, None).await {
            tracing::debug!("sync session ended: {e}");
        }
        Ok(())
    }
}

/// Where my copy of `container` stands.
pub(crate) fn my_heads(inner: &Inner, container: &SongId) -> Result<Heads> {
    inner.with_container(container, |s| Heads { acts: s.chains().act_heads(), seals: s.chains().seal_heads() }).ok_or(Error::NoSong)
}

/// Everything a peer with `theirs` lacks: sealed moments first (in the
/// keeper's order, main first), then acts nobody sealed yet.
pub(crate) fn missing_for(inner: &Inner, container: &SongId, theirs: &Heads) -> Vec<SyncMsg> {
    inner
        .with_container(container, |s| {
            let chains = s.chains();
            let mut out = Vec::new();
            let mut histories: Vec<flit_core::HistoryId> = chains.histories().iter().map(|h| h.id).collect();
            histories.sort_by_key(|h| *h != flit_core::main_history());
            for h in &histories {
                let have = theirs.seals.iter().find(|(x, _)| x == h).map(|(_, n)| *n).unwrap_or(0);
                for (f, a) in chains.moments_after(h, have) {
                    out.push(SyncMsg::Moment(a.clone(), f.clone()));
                }
            }
            for (being, h, len) in chains.act_heads() {
                let have = theirs.acts.iter().find(|(b, x, _)| *b == being && *x == h).map(|(_, _, n)| *n).unwrap_or(0);
                if have < len {
                    for a in chains.acts_after(&being, &h, have) {
                        if !chains.is_sealed(&a.id) {
                            out.push(SyncMsg::Act(a.clone()));
                        }
                    }
                }
            }
            out
        })
        .unwrap_or_default()
}

/// Acts a peer asked for by id: sealed ones travel as moments.
fn by_ids(inner: &Inner, container: &SongId, ids: &[ActId]) -> Vec<SyncMsg> {
    inner
        .with_container(container, |s| {
            let mut out = Vec::new();
            for id in ids {
                let Some(a) = s.chains().get(id) else { continue };
                match s.chains().fact_for(id) {
                    Some(f) => out.push(SyncMsg::Moment(a.clone(), f.clone())),
                    None => out.push(SyncMsg::Act(a.clone())),
                }
            }
            out
        })
        .unwrap_or_default()
}

/// Run one sync session over `conn`. `initiator` = Some(container) opens
/// the stream for that container; None accepts whatever the peer names.
/// A session that never gets past the handshake still reports `PeerDown`
/// so the caller knows the attempt ended.
pub(crate) async fn run_session(inner: Arc<Inner>, conn: Connection, initiator: Option<SongId>) -> Result<()> {
    let peer = crate::id_of(&conn.remote_id());
    match handshake(&inner, &conn, peer, initiator).await {
        Ok((container, send, recv, theirs)) => session(inner, peer, container, send, recv, theirs).await,
        Err(e) => {
            inner.emit(NetEvent::PeerDown(peer));
            Err(e)
        }
    }
}

type Streams = (SongId, iroh::endpoint::SendStream, iroh::endpoint::RecvStream, Heads);

async fn handshake(inner: &Arc<Inner>, conn: &Connection, peer: Id, initiator: Option<SongId>) -> Result<Streams> {
    if inner.is_blocked(&peer) {
        conn.close(1u32.into(), b"blocked");
        return Err(Error::Refused("blocked".into()));
    }
    let (container, send, recv, theirs) = match initiator {
        Some(container) => {
            if !inner.member_of(&container, &peer) {
                conn.close(1u32.into(), b"not in crew");
                return Err(Error::Refused(format!("{peer:?} is not in the crew")));
            }
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
            let heads = my_heads(inner, &container)?;
            write_msg(&mut send, &SyncMsg::Hello { song: container, heads, addr: inner.addr_info(), peers: inner.known_peers() }).await?;
            let Some(SyncMsg::Hello { song: theirs_song, heads: theirs, addr, peers }) = read_msg::<SyncMsg>(&mut recv).await? else {
                return Err(Error::Protocol("expected Hello".into()));
            };
            if theirs_song != container {
                return Err(Error::Refused("different container".into()));
            }
            learn(inner, peer, &addr, &peers)?;
            (container, send, recv, theirs)
        }
        None => {
            let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
            let Some(SyncMsg::Hello { song: container, heads: theirs, addr, peers }) = read_msg::<SyncMsg>(&mut recv).await? else {
                return Err(Error::Protocol("expected Hello".into()));
            };
            if !inner.member_of(&container, &peer) {
                write_msg(&mut send, &SyncMsg::Bye).await.ok();
                conn.close(1u32.into(), b"not in crew");
                return Err(Error::Refused(format!("{peer:?} is not in the crew of {container:?}")));
            }
            let Ok(heads) = my_heads(inner, &container) else {
                write_msg(&mut send, &SyncMsg::Bye).await.ok();
                return Err(Error::Refused("container not here".into()));
            };
            learn(inner, peer, &addr, &peers)?;
            write_msg(&mut send, &SyncMsg::Hello { song: container, heads, addr: inner.addr_info(), peers: inner.known_peers() }).await?;
            (container, send, recv, theirs)
        }
    };
    Ok((container, send, recv, theirs))
}

fn learn(inner: &Inner, peer: Id, addr: &AddrInfo, peers: &[AddrInfo]) -> Result<()> {
    if addr.id != peer {
        return Err(Error::Protocol("address id mismatch".into()));
    }
    inner.learn(addr);
    for p in peers {
        inner.learn(p);
    }
    Ok(())
}

async fn session(inner: Arc<Inner>, peer: Id, container: SongId, mut send: iroh::endpoint::SendStream, mut recv: iroh::endpoint::RecvStream, theirs: Heads) -> Result<()> {
    // What they lack, in the keeper's order.
    for msg in missing_for(&inner, &container, &theirs) {
        write_msg(&mut send, &msg).await?;
    }

    // Register the outbox, start the writer.
    let (tx, mut rx) = mpsc::unbounded_channel::<SyncMsg>();
    let outbox = SyncOutbox { tx, container };
    inner.peers.lock().unwrap().entry(peer).or_default().push(outbox.clone());
    inner.emit(NetEvent::PeerUp(peer));
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write_msg(&mut send, &msg).await.is_err() {
                break;
            }
        }
        send.finish().ok();
    });

    // Reader loop.
    let result = async {
        loop {
            let Some(msg) = read_msg::<SyncMsg>(&mut recv).await? else { break };
            match msg {
                SyncMsg::Act(_) | SyncMsg::Moment(_, _) => {
                    let want = inner.import_from(peer, &container, vec![msg]);
                    if inner.quota.lock().unwrap().is_exceeded(&peer) {
                        return Err(Error::Refused("over quota".into()));
                    }
                    if !want.is_empty() {
                        outbox.send(SyncMsg::Want(want));
                    }
                }
                SyncMsg::Want(ids) => {
                    for m in by_ids(&inner, &container, &ids) {
                        outbox.send(m);
                    }
                }
                SyncMsg::After(heads) => {
                    for m in missing_for(&inner, &container, &heads) {
                        outbox.send(m);
                    }
                }
                SyncMsg::Hello { .. } => {}
                SyncMsg::Bye => break,
            }
        }
        Ok::<(), Error>(())
    }
    .await;

    // Tear down.
    {
        let mut peers = inner.peers.lock().unwrap();
        if let Some(v) = peers.get_mut(&peer) {
            v.retain(|o| !o.tx.same_channel(&outbox.tx));
            if v.is_empty() {
                peers.remove(&peer);
            }
        }
    }
    writer.abort();
    inner.emit(NetEvent::PeerDown(peer));
    let _ = now_ms();
    result
}
