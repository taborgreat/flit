//! Content-addressed file store. Everything big (audio chunks, project
//! files, plugin source) lives here under its BLAKE3 hash.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub const ZERO: Hash = Hash([0u8; 32]);

    pub fn of(bytes: &[u8]) -> Hash {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Option<Hash> {
        let v = hex::decode(s.trim()).ok()?;
        let arr: [u8; 32] = v.try_into().ok()?;
        Some(Hash(arr))
    }

    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.short())
    }
}

impl std::str::FromStr for Hash {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Hash::from_hex(s).ok_or_else(|| crate::Error::Decode(format!("bad hash: {s}")))
    }
}

/// Files on disk under `<dir>/<first two hex>/<hex>`.
#[derive(Clone, Debug)]
pub struct BlobStore {
    dir: PathBuf,
}

impl BlobStore {
    pub fn open(dir: impl AsRef<Path>) -> crate::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, hash: &Hash) -> PathBuf {
        let hex = hash.to_hex();
        self.dir.join(&hex[..2]).join(hex)
    }

    pub fn has(&self, hash: &Hash) -> bool {
        self.path_for(hash).is_file()
    }

    /// Store bytes, return their hash. Idempotent.
    pub fn put(&self, bytes: &[u8]) -> crate::Result<Hash> {
        let hash = Hash::of(bytes);
        let path = self.path_for(&hash);
        if path.is_file() {
            return Ok(hash);
        }
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(hash)
    }

    /// Store bytes that arrived from a peer claiming `expected`. Verified.
    pub fn put_verified(&self, expected: &Hash, bytes: &[u8]) -> crate::Result<()> {
        if Hash::of(bytes) != *expected {
            return Err(crate::Error::BlobCorrupt(*expected));
        }
        self.put(bytes)?;
        Ok(())
    }

    pub fn put_file(&self, path: impl AsRef<Path>) -> crate::Result<Hash> {
        let bytes = std::fs::read(path)?;
        self.put(&bytes)
    }

    pub fn get(&self, hash: &Hash) -> crate::Result<Vec<u8>> {
        let path = self.path_for(hash);
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                crate::Error::BlobNotFound(*hash)
            } else {
                crate::Error::Io(e)
            }
        })?;
        if Hash::of(&bytes) != *hash {
            return Err(crate::Error::BlobCorrupt(*hash));
        }
        Ok(bytes)
    }

    pub fn size(&self, hash: &Hash) -> Option<u64> {
        std::fs::metadata(self.path_for(hash)).ok().map(|m| m.len())
    }

    /// Every hash in the store.
    pub fn list(&self) -> crate::Result<Vec<Hash>> {
        let mut out = Vec::new();
        for sub in std::fs::read_dir(&self.dir)? {
            let sub = sub?;
            if !sub.file_type()?.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(sub.path())? {
                let f = f?;
                if let Some(h) = f.file_name().to_str().and_then(Hash::from_hex) {
                    out.push(h);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Which of `wanted` are missing here.
    pub fn missing<'a>(&self, wanted: impl IntoIterator<Item = &'a Hash>) -> Vec<Hash> {
        wanted.into_iter().filter(|h| !self.has(h)).copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_verify() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path().join("blobs")).unwrap();
        let h = store.put(b"hello world").unwrap();
        assert_eq!(h, Hash::of(b"hello world"));
        assert!(store.has(&h));
        assert_eq!(store.get(&h).unwrap(), b"hello world");
        assert_eq!(store.put(b"hello world").unwrap(), h);
        assert_eq!(store.list().unwrap(), vec![h]);
        assert!(matches!(store.get(&Hash::of(b"nope")), Err(crate::Error::BlobNotFound(_))));
        assert!(matches!(
            store.put_verified(&Hash::of(b"a"), b"b"),
            Err(crate::Error::BlobCorrupt(_))
        ));
        assert_eq!(store.missing([&h, &Hash::of(b"x")]), vec![Hash::of(b"x")]);
    }

    #[test]
    fn corrupt_file_detected() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path()).unwrap();
        let h = store.put(b"data").unwrap();
        std::fs::write(store.path_for(&h), b"tampered").unwrap();
        assert!(matches!(store.get(&h), Err(crate::Error::BlobCorrupt(_))));
    }
}
