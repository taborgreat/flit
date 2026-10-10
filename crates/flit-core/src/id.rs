//! IDs: a user's (or machine's) identity is an ed25519 public key.
//! The same 32 bytes are also its network address (see flit-net).

use ed25519_dalek::{Signer, Verifier};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A public identity. Safe to share. Shown to users as a short code + QR.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Id(pub [u8; 32]);

/// Signature over an event id.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature(#[serde(with = "serde_bytes64")] pub [u8; 64]);

/// A private key. Never leaves the machine (OS keychain, or a 0600 file).
#[derive(Clone)]
pub struct Keypair {
    signing: ed25519_dalek::SigningKey,
}

impl Keypair {
    pub fn generate() -> Self {
        let seed: [u8; 32] = rand::random();
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        Self { signing }
    }

    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        Self { signing: ed25519_dalek::SigningKey::from_bytes(bytes) }
    }

    /// Deterministic keypair from a seed. Tests only; real keys use `generate`.
    pub fn from_seed(seed: u64) -> Self {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&seed.to_le_bytes());
        let h = blake3::hash(&bytes);
        Self::from_secret_bytes(h.as_bytes())
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    pub fn id(&self) -> Id {
        Id(self.signing.verifying_key().to_bytes())
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.signing.sign(msg).to_bytes())
    }
}

impl fmt::Debug for Keypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keypair({})", self.id())
    }
}

impl Id {
    pub const ZERO: Id = Id([0u8; 32]);

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&self.0) else {
            return false;
        };
        let sig = ed25519_dalek::Signature::from_bytes(&sig.0);
        vk.verify(msg, &sig).is_ok()
    }

    /// Full textual form: lowercase base32, no padding (52 chars).
    pub fn to_base32(&self) -> String {
        data_encoding::BASE32_NOPAD.encode(&self.0).to_lowercase()
    }

    pub fn from_base32(s: &str) -> Option<Id> {
        let s = s.trim().to_uppercase();
        let bytes = data_encoding::BASE32_NOPAD.decode(s.as_bytes()).ok()?;
        let arr: [u8; 32] = bytes.try_into().ok()?;
        Some(Id(arr))
    }

    /// Short code for the UI, e.g. `k3jd-9f2a-...` (first 16 chars grouped).
    pub fn short_code(&self) -> String {
        let b = self.to_base32();
        b[..16]
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_base32())
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.short_code())
    }
}

impl std::str::FromStr for Id {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Id::from_base32(s).ok_or_else(|| crate::Error::Decode(format!("bad id: {s}")))
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sig({}..)", hex::encode(&self.0[..4]))
    }
}

mod serde_bytes64 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            hex::encode(v).serialize(s)
        } else {
            v.as_slice().serialize(s)
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        if d.is_human_readable() {
            let s = String::deserialize(d)?;
            let v = hex::decode(s).map_err(serde::de::Error::custom)?;
            v.try_into().map_err(|_| serde::de::Error::custom("sig must be 64 bytes"))
        } else {
            let v: Vec<u8> = Vec::deserialize(d)?;
            v.try_into().map_err(|_| serde::de::Error::custom("sig must be 64 bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let kp = Keypair::generate();
        let sig = kp.sign(b"hello");
        assert!(kp.id().verify(b"hello", &sig));
        assert!(!kp.id().verify(b"hellp", &sig));
        assert!(!Keypair::generate().id().verify(b"hello", &sig));
    }

    #[test]
    fn base32_roundtrip() {
        let id = Keypair::from_seed(7).id();
        let s = id.to_base32();
        assert_eq!(s.len(), 52);
        assert_eq!(Id::from_base32(&s), Some(id));
        assert_eq!(Id::from_base32(&s.to_uppercase()), Some(id));
        assert_eq!(id.short_code().len(), 19);
    }

    #[test]
    fn seeded_is_deterministic() {
        assert_eq!(Keypair::from_seed(1).id(), Keypair::from_seed(1).id());
        assert_ne!(Keypair::from_seed(1).id(), Keypair::from_seed(2).id());
    }
}
