//! Blob transfer: ask a peer for hashes, get bytes back, verify by BLAKE3
//! before storing. Chunks are small (one bar), so a lost connection costs
//! at most one chunk: resumable by nature.

use crate::node::{now_ms, Inner, NetEvent};
use crate::proto::*;
use crate::{Error, Result, ALPN_BLOB};
use flit_core::{Hash, Id};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct BlobHandler(pub Arc<Inner>);

impl std::fmt::Debug for BlobHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BlobHandler")
    }
}

impl ProtocolHandler for BlobHandler {
    async fn accept(&self, conn: Connection) -> std::result::Result<(), AcceptError> {
        let inner = self.0.clone();
        let peer = crate::id_of(&conn.remote_id());
        let member = inner.container_ids().iter().any(|id| inner.member_of(id, &peer));
        if inner.is_blocked(&peer) || !member {
            conn.close(1u32.into(), b"not in crew");
            return Ok(());
        }
        while let Ok((mut send, mut recv)) = conn.accept_bi().await {
            let inner = inner.clone();
            tokio::spawn(async move {
                let Ok(Some(BlobMsg::Get(hashes))) = read_msg::<BlobMsg>(&mut recv).await else { return };
                for h in hashes.iter().take(10_000) {
                    let data = inner.container_ids().iter().find_map(|id| inner.with_container(id, |s| s.blobs().get(h).ok()).flatten());
                    match data {
                        Some(bytes) => {
                            if write_msg(&mut send, &BlobMsg::Found { hash: *h, len: bytes.len() as u64 }).await.is_err() {
                                return;
                            }
                            if send.write_all(&bytes).await.is_err() {
                                return;
                            }
                        }
                        None => {
                            if write_msg(&mut send, &BlobMsg::NotFound(*h)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                send.finish().ok();
            });
        }
        Ok(())
    }
}

/// Fetch `hashes` from `from` into the open song's store. Already-present
/// ones are skipped. Each blob is verified against its hash and counted
/// against the peer's quota.
pub(crate) async fn fetch(inner: Arc<Inner>, from: Id, hashes: Vec<Hash>) -> Result<Vec<Hash>> {
    let song = inner.song.lock().unwrap().as_ref().map(|s| s.id()).ok_or(Error::NoSong)?;
    fetch_into(inner, from, song, hashes).await
}

/// The same into any container's store (an org's album covers).
pub(crate) async fn fetch_into(inner: Arc<Inner>, from: Id, container: flit_core::SongId, hashes: Vec<Hash>) -> Result<Vec<Hash>> {
    let wanted: Vec<Hash> = inner.with_container(&container, |s| s.blobs().missing(hashes.iter())).ok_or(Error::NoSong)?;
    if wanted.is_empty() {
        return Ok(vec![]);
    }
    let ep_id = crate::endpoint_id(&from).ok_or(Error::BadCode)?;
    let addr = inner.known.lock().unwrap().get(&from).and_then(|a| a.to_endpoint_addr()).unwrap_or_else(|| iroh::EndpointAddr::new(ep_id));
    let conn = inner.ep.connect(addr, ALPN_BLOB).await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| Error::Net(anyhow::anyhow!(e)))?;
    write_msg(&mut send, &BlobMsg::Get(wanted.clone())).await?;
    send.finish().ok();
    let mut got = Vec::new();
    for _ in 0..wanted.len() {
        let Some(msg) = read_msg::<BlobMsg>(&mut recv).await? else { break };
        match msg {
            BlobMsg::Found { hash, len } => {
                if !inner.quota.lock().unwrap().allow_storage(&from, len, now_ms()) {
                    inner.emit(NetEvent::QuotaExceeded(from));
                    break;
                }
                let bytes = read_exact_bytes(&mut recv, len as usize).await?;
                let ok = inner.with_container(&container, |s| s.blobs().put_verified(&hash, &bytes).is_ok()).unwrap_or(false);
                if ok {
                    got.push(hash);
                    inner.emit(NetEvent::BlobFetched { hash, from });
                } else {
                    inner.emit(NetEvent::BlobFailed { hash, from });
                }
            }
            BlobMsg::NotFound(hash) => inner.emit(NetEvent::BlobFailed { hash, from }),
            BlobMsg::Get(_) => break,
        }
    }
    conn.close(0u32.into(), b"done");
    Ok(got)
}
