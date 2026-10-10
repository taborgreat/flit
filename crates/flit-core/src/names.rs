//! Name management on one machine (update pack §23): several Names, each
//! with its own key and beings; switch, log out, back up, import, remove.
//! Names never mix data; song folders hold every crew Name's spine.
//!
//! ```text
//! <data>/names/<name-hex>/secret        the Name key (hex)
//! <data>/names/<name-hex>/profile.json  label, beings, active being
//! <data>/names/current                  which Name is in use
//! ```
//! Keys sit in files here until the OS keychain is wired in.

use crate::act::{BeingId, BeingKind};
use crate::id::{Id, Keypair};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeingProfile {
    pub id: BeingId,
    pub kind: BeingKind,
    pub display_name: String,
    /// 0xRRGGBB.
    pub color: u32,
    /// Runs on another machine with its own certified key.
    #[serde(default)]
    pub delegated_key: Option<Id>,
}

/// The being colors of the design (update pack §12), in order.
pub const BEING_COLORS: [u32; 8] = [0x5B8DEF, 0xE07A5F, 0x4FB286, 0xC77DDB, 0xE8C547, 0x4CC3D9, 0xF08FB0, 0x9AA5B1];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameProfile {
    pub id: Id,
    pub label: String,
    #[serde(default)]
    pub beings: Vec<BeingProfile>,
    #[serde(default)]
    pub active_being: Option<BeingId>,
    /// The user confirmed a backup exists.
    #[serde(default)]
    pub backed_up: bool,
}

impl NameProfile {
    pub fn being(&self, id: &BeingId) -> Option<&BeingProfile> {
        self.beings.iter().find(|b| b.id == *id)
    }

    pub fn active(&self) -> Option<&BeingProfile> {
        self.active_being.as_ref().and_then(|b| self.being(b)).or_else(|| self.beings.first())
    }

    /// Next unused being color.
    pub fn next_color(&self) -> u32 {
        BEING_COLORS[self.beings.len() % BEING_COLORS.len()]
    }
}

#[derive(Debug, Clone)]
pub struct NameStore {
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Backup {
    id: Id,
    secret: String,
    profile: NameProfile,
}

impl NameStore {
    pub fn open(root: impl AsRef<Path>) -> crate::Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("names"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, id: &Id) -> PathBuf {
        self.root.join("names").join(id.to_hex())
    }

    fn current_path(&self) -> PathBuf {
        self.root.join("names").join("current")
    }

    /// Every Name on this machine, by label.
    pub fn list(&self) -> crate::Result<Vec<NameProfile>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.root.join("names"))? {
            let e = e?;
            if !e.file_type()?.is_dir() {
                continue;
            }
            let p = e.path().join("profile.json");
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(profile) = serde_json::from_str::<NameProfile>(&text) {
                    out.push(profile);
                }
            }
        }
        out.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    pub fn profile(&self, id: &Id) -> crate::Result<NameProfile> {
        let p = self.dir(id).join("profile.json");
        serde_json::from_str(&std::fs::read_to_string(&p)?).map_err(|e| crate::Error::Decode(e.to_string()))
    }

    pub fn save_profile(&self, profile: &NameProfile) -> crate::Result<()> {
        let d = self.dir(&profile.id);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("profile.json"), serde_json::to_string_pretty(profile).map_err(|e| crate::Error::Encode(e.to_string()))?)?;
        Ok(())
    }

    /// The Name in use, if any (logged out = None).
    pub fn current(&self) -> crate::Result<Option<NameProfile>> {
        let p = self.current_path();
        if !p.is_file() {
            return Ok(None);
        }
        let Some(id) = Id::from_base32(&std::fs::read_to_string(&p)?) else { return Ok(None) };
        if !self.dir(&id).join("secret").is_file() {
            return Ok(None);
        }
        self.profile(&id).map(Some)
    }

    /// Use this Name from now on.
    pub fn switch(&self, id: &Id) -> crate::Result<()> {
        if !self.dir(id).join("secret").is_file() {
            return Err(crate::Error::Invalid(format!("no key for {} on this machine", id.short_code())));
        }
        std::fs::write(self.current_path(), id.to_base32())?;
        Ok(())
    }

    /// Stop using any Name; keys stay so logging back in is instant.
    pub fn logout(&self) -> crate::Result<()> {
        let p = self.current_path();
        if p.is_file() {
            std::fs::remove_file(p)?;
        }
        Ok(())
    }

    /// A fresh Name with its first being. Becomes current.
    pub fn create(&self, label: &str, first_being: Option<BeingProfile>) -> crate::Result<(Keypair, NameProfile)> {
        let key = Keypair::generate();
        let profile = self.adopt(&key, label, first_being.into_iter().collect())?;
        Ok((key, profile))
    }

    /// Keep an existing key as a Name here (import, migration). Becomes current.
    pub fn adopt(&self, key: &Keypair, label: &str, beings: Vec<BeingProfile>) -> crate::Result<NameProfile> {
        let id = key.id();
        let d = self.dir(&id);
        std::fs::create_dir_all(&d)?;
        std::fs::write(d.join("secret"), hex::encode(key.secret_bytes()))?;
        let profile = match self.profile(&id) {
            Ok(mut p) => {
                if !label.is_empty() {
                    p.label = label.to_string();
                }
                for b in beings {
                    if p.being(&b.id).is_none() {
                        p.beings.push(b);
                    }
                }
                p
            }
            Err(_) => {
                let active = beings.first().map(|b| b.id.clone());
                NameProfile { id, label: label.to_string(), beings, active_being: active, backed_up: false }
            }
        };
        self.save_profile(&profile)?;
        self.switch(&id)?;
        Ok(profile)
    }

    pub fn key(&self, id: &Id) -> crate::Result<Keypair> {
        let text = std::fs::read_to_string(self.dir(id).join("secret"))?;
        let bytes = hex::decode(text.trim()).map_err(|e| crate::Error::Decode(e.to_string()))?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| crate::Error::Decode("secret must be 32 bytes".into()))?;
        let key = Keypair::from_secret_bytes(&arr);
        if key.id() != *id {
            return Err(crate::Error::Invalid("secret does not match the Name".into()));
        }
        Ok(key)
    }

    pub fn add_being(&self, id: &Id, being: BeingProfile) -> crate::Result<NameProfile> {
        let mut p = self.profile(id)?;
        if p.being(&being.id).is_some() {
            return Err(crate::Error::Invalid(format!("being {} already exists", being.id)));
        }
        p.beings.push(being);
        if p.active_being.is_none() {
            p.active_being = p.beings.first().map(|b| b.id.clone());
        }
        self.save_profile(&p)?;
        Ok(p)
    }

    pub fn set_active_being(&self, id: &Id, being: &BeingId) -> crate::Result<NameProfile> {
        let mut p = self.profile(id)?;
        if p.being(being).is_none() {
            return Err(crate::Error::Invalid(format!("no being {being}")));
        }
        p.active_being = Some(being.clone());
        self.save_profile(&p)?;
        Ok(p)
    }

    pub fn remove_being(&self, id: &Id, being: &BeingId) -> crate::Result<NameProfile> {
        let mut p = self.profile(id)?;
        p.beings.retain(|b| b.id != *being);
        if p.active_being.as_ref() == Some(being) {
            p.active_being = p.beings.first().map(|b| b.id.clone());
        }
        self.save_profile(&p)?;
        Ok(p)
    }

    /// Delete this Name's key and profile here. Songs on disk stay.
    pub fn remove(&self, id: &Id) -> crate::Result<()> {
        let d = self.dir(id);
        if d.is_dir() {
            std::fs::remove_dir_all(&d)?;
        }
        if let Ok(Some(cur)) = self.current() {
            if cur.id == *id {
                self.logout()?;
            }
        }
        Ok(())
    }

    // ----- backup -----

    /// The recovery code: the key itself, typed as 13 groups of 4.
    pub fn recovery_code(key: &Keypair) -> String {
        let b = data_encoding::BASE32_NOPAD.encode(&key.secret_bytes()).to_lowercase();
        b.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
    }

    pub fn from_recovery_code(code: &str) -> crate::Result<Keypair> {
        let s: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase();
        let bytes = data_encoding::BASE32_NOPAD.decode(s.as_bytes()).map_err(|_| crate::Error::Decode("not a recovery code".into()))?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| crate::Error::Decode("recovery code has the wrong length".into()))?;
        Ok(Keypair::from_secret_bytes(&arr))
    }

    /// An encrypted backup file of a Name (key + profile).
    pub fn export_backup(&self, id: &Id, passphrase: &str) -> crate::Result<Vec<u8>> {
        let key = self.key(id)?;
        let profile = self.profile(id)?;
        let plain = serde_json::to_vec(&Backup { id: *id, secret: hex::encode(key.secret_bytes()), profile }).map_err(|e| crate::Error::Encode(e.to_string()))?;
        Ok(backup::seal(&plain, passphrase))
    }

    /// Restore a Name from a backup file. Becomes current.
    pub fn import_backup(&self, bytes: &[u8], passphrase: &str) -> crate::Result<NameProfile> {
        let plain = backup::open(bytes, passphrase)?;
        let b: Backup = serde_json::from_slice(&plain).map_err(|e| crate::Error::Decode(e.to_string()))?;
        let raw = hex::decode(&b.secret).map_err(|e| crate::Error::Decode(e.to_string()))?;
        let arr: [u8; 32] = raw.try_into().map_err(|_| crate::Error::Decode("bad secret".into()))?;
        let key = Keypair::from_secret_bytes(&arr);
        if key.id() != b.id {
            return Err(crate::Error::Invalid("backup key does not match its Name".into()));
        }
        let mut p = self.adopt(&key, &b.profile.label, b.profile.beings.clone())?;
        p.active_being = b.profile.active_being.clone().or(p.active_being);
        p.backed_up = true;
        self.save_profile(&p)?;
        Ok(p)
    }

    pub fn mark_backed_up(&self, id: &Id) -> crate::Result<()> {
        let mut p = self.profile(id)?;
        p.backed_up = true;
        self.save_profile(&p)
    }
}

/// Passphrase-sealed bytes: BLAKE3-stretched key, BLAKE3 XOF keystream,
/// keyed-BLAKE3 tag (encrypt-then-MAC). Format: magic, salt, nonce,
/// ciphertext, tag.
mod backup {
    const MAGIC: &[u8] = b"FLITBK1\n";
    const ROUNDS: u32 = 60_000;

    fn stretch(passphrase: &str, salt: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
        let mut k = blake3::derive_key("flit backup v1 passphrase", passphrase.as_bytes());
        for _ in 0..ROUNDS {
            k = *blake3::keyed_hash(&k, salt).as_bytes();
        }
        let enc = blake3::derive_key("flit backup v1 enc", &k);
        let mac = blake3::derive_key("flit backup v1 mac", &k);
        (enc, mac)
    }

    fn keystream(enc: &[u8; 32], nonce: &[u8; 16], len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        let mut h = blake3::Hasher::new_keyed(enc);
        h.update(nonce);
        h.finalize_xof().fill(&mut out);
        out
    }

    pub fn seal(plain: &[u8], passphrase: &str) -> Vec<u8> {
        let salt: [u8; 16] = rand::random();
        let nonce: [u8; 16] = rand::random();
        let (enc, mac) = stretch(passphrase, &salt);
        let ks = keystream(&enc, &nonce, plain.len());
        let cipher: Vec<u8> = plain.iter().zip(ks).map(|(p, k)| p ^ k).collect();
        let mut out = Vec::with_capacity(MAGIC.len() + 32 + cipher.len() + 32);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&salt);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&cipher);
        let mut m = blake3::Hasher::new_keyed(&mac);
        m.update(&salt);
        m.update(&nonce);
        m.update(&cipher);
        out.extend_from_slice(m.finalize().as_bytes());
        out
    }

    pub fn open(bytes: &[u8], passphrase: &str) -> crate::Result<Vec<u8>> {
        if bytes.len() < MAGIC.len() + 64 || &bytes[..MAGIC.len()] != MAGIC {
            return Err(crate::Error::Decode("not a FLit backup file".into()));
        }
        let rest = &bytes[MAGIC.len()..];
        let salt: [u8; 16] = rest[..16].try_into().unwrap();
        let nonce: [u8; 16] = rest[16..32].try_into().unwrap();
        let cipher = &rest[32..rest.len() - 32];
        let tag = &rest[rest.len() - 32..];
        let (enc, mac) = stretch(passphrase, &salt);
        let mut m = blake3::Hasher::new_keyed(&mac);
        m.update(&salt);
        m.update(&nonce);
        m.update(cipher);
        if m.finalize().as_bytes() != tag {
            return Err(crate::Error::Invalid("wrong passphrase or damaged backup".into()));
        }
        let ks = keystream(&enc, &nonce, cipher.len());
        Ok(cipher.iter().zip(ks).map(|(c, k)| c ^ k).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_switch_logout_backup() {
        let dir = tempfile::tempdir().unwrap();
        let store = NameStore::open(dir.path()).unwrap();
        assert!(store.current().unwrap().is_none());
        let (k1, p1) = store.create("Tabor", Some(BeingProfile { id: BeingId::new("tabor"), kind: BeingKind::Human, display_name: "Tabor".into(), color: BEING_COLORS[0], delegated_key: None })).unwrap();
        assert_eq!(store.current().unwrap().unwrap().id, k1.id());
        let (k2, _) = store.create("Second", None).unwrap();
        assert_eq!(store.current().unwrap().unwrap().id, k2.id());
        assert_eq!(store.list().unwrap().len(), 2);
        store.switch(&k1.id()).unwrap();
        assert_eq!(store.current().unwrap().unwrap().active().unwrap().display_name, "Tabor");
        store.logout().unwrap();
        assert!(store.current().unwrap().is_none());
        store.switch(&k1.id()).unwrap();
        assert_eq!(store.key(&k1.id()).unwrap().id(), k1.id());

        // recovery code round trip
        let code = NameStore::recovery_code(&k1);
        assert_eq!(code.split('-').count(), 13);
        assert_eq!(NameStore::from_recovery_code(&code).unwrap().id(), k1.id());

        // backup to another machine
        let bytes = store.export_backup(&k1.id(), "hunter2").unwrap();
        let other = tempfile::tempdir().unwrap();
        let store2 = NameStore::open(other.path()).unwrap();
        assert!(store2.import_backup(&bytes, "wrong").is_err());
        let restored = store2.import_backup(&bytes, "hunter2").unwrap();
        assert_eq!(restored.id, p1.id);
        assert_eq!(restored.beings, p1.beings);
        assert_eq!(store2.key(&k1.id()).unwrap().id(), k1.id());

        store.remove(&k1.id()).unwrap();
        assert!(store.key(&k1.id()).is_err());
        assert!(store.current().unwrap().is_none());
    }
}
