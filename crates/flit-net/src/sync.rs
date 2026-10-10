//! Live sync of a song between two peers. Symmetric: both sides say Hello
//! with the event ids they hold, each sends what the other lacks, then
//! every new event is pushed as it happens. Out-of-order arrivals are held
//! by the log and the missing parents are asked for.

use crate::node::{now_ms, Inner, NetEvent};
use crate::proto::*;
use crate::{Error, Result};
use flit_core::{Stamp, StampId, Id};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Where a session's outgoing messages go.
#[derive(Clone, Debug)]
pub struct SyncOutbox {
    tx: mpsc::UnboundedSender<SyncMsg>,
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
        if let Err(e) = run_session(inner, conn, false).await {
            tracing::debug!("sync session ended: {e}");
        }
        Ok(())
    }
}

fn snapshot(inner: &Inner) -> Result<(flit_core::SongId, Vec<StampId>)> {
    let g = inner.song.lock().unwrap();
    let s = g.as_ref().ok_or(Error::NoSong)?;
    Ok((s.id(), s.chains().all_ids()))
}

fn events_by_ids(inner: &Inner, ids: &[StampId]) -> Vec<Stamp> {
    let g = inner.song.lock().unwrap();
    let Some(s) = g.as_ref() else { return vec![] };
    // dependencies first, so the receiver folds instead of holding
    let wanted: std::collections::HashSet<flit_core::StampId> = ids.iter().copied().collect();
    let order = flit_core::seal::fold::merge_order(|id| s.chains().get(id), &wanted);
    order.iter().filter_map(|id| s.chains().get(id).cloned()).collect()
}

fn is_member(inner: &Inner, id: &Id) -> bool {
    let g = inner.song.lock().unwrap();
    g.as_ref().map(|s| s.crew().is_member(id)).unwrap_or(false)
}

/// Run one sync session over `conn`. `initiator` opens the stream.
/// A session that never gets past the handshake still reports `PeerDown`
/// so the caller knows the attempt ended.
pub(crate) async fn run_session(inner: Arc<Inner>, conn: Connection, initiator: bool) -> Result<()> {
    let peer = crate::id_of(&conn.remote_id());
    match handshake(&inner, &conn, peer, initiator).await {
        Ok((send, recv, my_ids, their_ids)) => session(inner, peer, send, recv, my_ids, their_ids).await,
        Err(e) => {
            inner.emit(NetEvent::PeerDown(peer));
            Err(e)
        }
    }
}

type Streams = (iroh::endpoint::SendStream, iroh::endpoint::RecvStream, Vec<StampId>, Vec<StampId>);

async fn handshake(inner: &Arc<Inner>, conn: &Connection, peer: Id, initiator: bool) -> Result<Streams> {
    if inner.is_blocked(&peer) {
        conn.close(1u32.into(), b"blocked");
        return Err(Error::Refused("blocked".into()));
    }
    if !is_member(inner, &peer) {
        conn.close(1u32.into(), b"not in crew");
        return Err(Error::Refused(format!("{peer:?} is not in the crew")));
    }
    let (mut send, mut recv) = if initiator {
        conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?
    } else {
        conn.accept_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?
    };

    // Hello both ways.
    let (song, my_ids) = snapshot(inner)?;
    let hello = SyncMsg::Hello { song, ids: my_ids.clone(), addr: inner.addr_info(), peers: inner.known_peers() };
    write_msg(&mut send, &hello).await?;
    let Some(SyncMsg::Hello { song: their_song, ids: their_ids, addr, peers }) = read_msg::<SyncMsg>(&mut recv).await? else {
        return Err(Error::Protocol("expected Hello".into()));
    };
    if their_song != song {
        write_msg(&mut send, &SyncMsg::Bye).await.ok();
        return Err(Error::Refused("different song".into()));
    }
    if addr.id != peer {
        return Err(Error::Protocol("address id mismatch".into()));
    }
    inner.learn(&addr);
    for p in &peers {
        inner.learn(p);
    }
    Ok((send, recv, my_ids, their_ids))
}

async fn session(inner: Arc<Inner>, peer: Id, mut send: iroh::endpoint::SendStream, mut recv: iroh::endpoint::RecvStream, my_ids: Vec<StampId>, their_ids: Vec<StampId>) -> Result<()> {

    // What they lack, in causal order.
    let theirs: HashSet<StampId> = their_ids.iter().copied().collect();
    let missing: Vec<StampId> = my_ids.iter().filter(|id| !theirs.contains(id)).copied().collect();
    if !missing.is_empty() {
        let evs = events_by_ids(&inner, &missing);
        for chunk in evs.chunks(500) {
            write_msg(&mut send, &SyncMsg::Stamps(chunk.to_vec())).await?;
        }
    }

    // Register the outbox, start the writer.
    let (tx, mut rx) = mpsc::unbounded_channel::<SyncMsg>();
    let outbox = SyncOutbox { tx };
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
                SyncMsg::Stamps(evs) => {
                    let (_imported, want) = inner.import_from(peer, evs);
                    if inner.quota.lock().unwrap().is_exceeded(&peer) {
                        return Err(Error::Refused("over quota".into()));
                    }
                    if !want.is_empty() {
                        outbox.send(SyncMsg::Want(want));
                    }
                }
                SyncMsg::Want(ids) => {
                    let evs = events_by_ids(&inner, &ids);
                    if !evs.is_empty() {
                        outbox.send(SyncMsg::Stamps(evs));
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
