//! Wire messages. Everything is postcard, length-prefixed (u32 LE).
//!
//! Two verbs carry a song between machines (FLIT_SPEC §21): ACT (a
//! being's signed act, live and optimistic) and MOMENT (the keeper's
//! sealed fact with its act). Catching up is "moments after head X" per
//! history, then acts after each being's act head.

use flit_audio::StreamPacket;
use flit_core::{Act, ActId, BeingRef, Fact, Hash, HistoryId, Id, Signature, SongId};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// How to reach an ID: direct sockets and/or a relay URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrInfo {
    pub id: Id,
    pub ips: Vec<SocketAddr>,
    pub relay: Option<String>,
}

const FRIEND_PREFIX: &str = "flitid1";

impl AddrInfo {
    /// "Friend code": an ID plus how to reach it. Longer than a bare ID but
    /// works without any discovery service (LAN, tests).
    pub fn encode(&self) -> String {
        let bytes = postcard::to_allocvec(self).expect("addr encodes");
        format!("{FRIEND_PREFIX}{}", data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
    }

    /// Accepts a friend code or a bare ID (no address: reachable only
    /// through discovery/relay).
    pub fn decode(s: &str) -> Option<AddrInfo> {
        let t = s.trim().to_lowercase();
        if let Some(body) = t.strip_prefix(FRIEND_PREFIX) {
            let bytes = data_encoding::BASE32_NOPAD.decode(body.to_uppercase().as_bytes()).ok()?;
            return postcard::from_bytes(&bytes).ok();
        }
        let id = Id::from_base32(&t)?;
        Some(AddrInfo { id, ips: vec![], relay: None })
    }

    pub fn to_endpoint_addr(&self) -> Option<iroh::EndpointAddr> {
        let id = crate::endpoint_id(&self.id)?;
        let mut a = iroh::EndpointAddr::new(id);
        for ip in &self.ips {
            a = a.with_ip_addr(*ip);
        }
        if let Some(r) = &self.relay {
            if let Ok(url) = r.parse::<iroh::RelayUrl>() {
                a = a.with_relay_url(url);
            }
        }
        Some(a)
    }

    pub fn from_endpoint_addr(a: &iroh::EndpointAddr) -> Self {
        Self {
            id: crate::id_of(&a.id),
            ips: a.ip_addrs().copied().collect(),
            relay: a.relay_urls().next().map(|u| u.to_string()),
        }
    }
}

/// Ask to join. Signed by the asker so the owner knows the ID is real.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    pub song: SongId,
    pub from: Id,
    pub display_name: String,
    pub addr: AddrInfo,
    /// Signature over (song || from || display_name).
    pub sig: Signature,
}

impl JoinRequest {
    pub fn signing_bytes(song: &SongId, from: &Id, name: &str) -> Vec<u8> {
        let mut v = Vec::with_capacity(64 + name.len());
        v.extend_from_slice(song.as_bytes());
        v.extend_from_slice(from.as_bytes());
        v.extend_from_slice(name.as_bytes());
        v
    }
    pub fn verify(&self) -> bool {
        self.display_name.len() <= 64 && self.from.verify(&Self::signing_bytes(&self.song, &self.from, &self.display_name), &self.sig)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum JoinReply {
    /// Approved: here is the song to start from (its genesis moment).
    Welcome { name: String, genesis: Act, genesis_fact: Option<Fact>, peers: Vec<AddrInfo> },
    /// Owner will decide later; ask again (or wait for them to add you).
    Pending,
    Refused(String),
}

/// Where a peer's copy stands: per being per history act counts, and per
/// history seal counts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Heads {
    pub acts: Vec<(BeingRef, HistoryId, u64)>,
    pub seals: Vec<(HistoryId, u64)>,
}

/// Sync session messages (both directions).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SyncMsg {
    Hello { song: SongId, heads: Heads, addr: AddrInfo, peers: Vec<AddrInfo> },
    /// ACT: a being's signed act (live, optimistic).
    Act(Act),
    /// MOMENT: the keeper's sealed fact with its act.
    Moment(Act, Fact),
    /// Acts I am missing (an act_prev I do not have).
    Want(Vec<ActId>),
    /// Send me everything after these heads.
    After(Heads),
    Bye,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlobMsg {
    Get(Vec<Hash>),
    /// One per requested hash, in order.
    Found { hash: Hash, len: u64 },
    NotFound(Hash),
}

/// A live audio frame: which stream of the sender, and the packet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamFrame {
    pub stream: u16,
    pub packet: StreamPacket,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamCtl {
    Open { stream: u16, config: flit_audio::StreamConfig },
    Close { stream: u16 },
}

pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let body = postcard::to_allocvec(msg).expect("message encodes");
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub async fn write_msg<T: Serialize>(send: &mut iroh::endpoint::SendStream, msg: &T) -> crate::Result<()> {
    send.write_all(&encode(msg)).await.map_err(|e| crate::Error::Protocol(e.to_string()))
}

pub async fn read_msg<T: for<'de> Deserialize<'de>>(recv: &mut iroh::endpoint::RecvStream) -> crate::Result<Option<T>> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(_) => return Ok(None),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > crate::MAX_MESSAGE {
        return Err(crate::Error::Protocol(format!("message too large: {n}")));
    }
    let mut buf = vec![0u8; n];
    recv.read_exact(&mut buf).await.map_err(|e| crate::Error::Protocol(e.to_string()))?;
    let msg = postcard::from_bytes(&buf).map_err(|e| crate::Error::Protocol(format!("decode: {e}")))?;
    Ok(Some(msg))
}

pub async fn read_exact_bytes(recv: &mut iroh::endpoint::RecvStream, n: usize) -> crate::Result<Vec<u8>> {
    if n > crate::MAX_MESSAGE {
        return Err(crate::Error::Protocol(format!("blob too large: {n}")));
    }
    let mut buf = vec![0u8; n];
    recv.read_exact(&mut buf).await.map_err(|e| crate::Error::Protocol(e.to_string()))?;
    Ok(buf)
}
