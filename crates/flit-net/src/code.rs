//! Song code: the song's public id plus how to reach its owner. Lets
//! someone ASK to join; never lets them in.

use crate::proto::AddrInfo;
use flit_core::{Id, SongId};
use serde::{Deserialize, Serialize};

const PREFIX: &str = "flit1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SongCode {
    pub song: SongId,
    pub owner: Id,
    pub owner_addr: AddrInfo,
}

impl SongCode {
    pub fn encode(&self) -> String {
        let bytes = postcard::to_allocvec(self).expect("code encodes");
        format!("{PREFIX}{}", data_encoding::BASE32_NOPAD.encode(&bytes).to_lowercase())
    }

    pub fn decode(s: &str) -> Option<SongCode> {
        let s = s.trim().to_lowercase();
        let body = s.strip_prefix(PREFIX)?;
        let bytes = data_encoding::BASE32_NOPAD.decode(body.to_uppercase().as_bytes()).ok()?;
        let code: SongCode = postcard::from_bytes(&bytes).ok()?;
        if code.owner_addr.id != code.owner {
            return None;
        }
        Some(code)
    }
}

impl std::fmt::Display for SongCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.encode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_reject_garbage() {
        let owner = flit_core::Keypair::from_seed(1).id();
        let code = SongCode {
            song: flit_core::Hash::of(b"song"),
            owner,
            owner_addr: AddrInfo { id: owner, ips: vec!["192.168.1.2:4433".parse().unwrap()], relay: Some("https://relay.example".into()) },
        };
        let s = code.encode();
        assert!(s.starts_with("flit1"));
        assert_eq!(SongCode::decode(&s), Some(code.clone()));
        assert_eq!(SongCode::decode(&s.to_uppercase()), Some(code));
        assert_eq!(SongCode::decode("flit1zzzz"), None);
        assert_eq!(SongCode::decode("nope"), None);
    }
}

