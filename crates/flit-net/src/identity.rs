//! Where the private key lives: the OS keychain on Windows/macOS, a 0600
//! file on Linux (and as a fallback everywhere).

use flit_core::Keypair;
use std::path::{Path, PathBuf};

const SERVICE: &str = "flit";
const ACCOUNT: &str = "identity";

#[derive(Debug, Clone)]
pub struct IdentityStore {
    file: PathBuf,
    use_keychain: bool,
}

impl IdentityStore {
    /// Default location: `<config dir>/flit/identity.key`.
    pub fn default_location() -> Self {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::at(base.join("flit").join("identity.key"))
    }

    pub fn at(file: impl AsRef<Path>) -> Self {
        Self { file: file.as_ref().to_path_buf(), use_keychain: cfg!(any(windows, target_os = "macos")) }
    }

    /// Force the file store (tests).
    pub fn file_only(mut self) -> Self {
        self.use_keychain = false;
        self
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    /// Load the key, creating one on first run.
    pub fn load_or_create(&self) -> std::io::Result<Keypair> {
        if let Some(k) = self.load()? {
            return Ok(k);
        }
        let k = Keypair::generate();
        self.save(&k)?;
        Ok(k)
    }

    pub fn load(&self) -> std::io::Result<Option<Keypair>> {
        if self.use_keychain {
            if let Some(k) = keychain::load() {
                return Ok(Some(k));
            }
        }
        if !self.file.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&self.file)?;
        let bytes = data_encoding::HEXLOWER
            .decode(text.trim().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "key must be 32 bytes"))?;
        Ok(Some(Keypair::from_secret_bytes(&arr)))
    }

    pub fn save(&self, key: &Keypair) -> std::io::Result<()> {
        if self.use_keychain && keychain::save(key) {
            return Ok(());
        }
        if let Some(d) = self.file.parent() {
            std::fs::create_dir_all(d)?;
        }
        let hex = data_encoding::HEXLOWER.encode(&key.secret_bytes());
        std::fs::write(&self.file, hex)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.file, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Export for "ID backup": the secret as hex. Show once, never log.
    pub fn export_backup(&self) -> std::io::Result<Option<String>> {
        Ok(self.load()?.map(|k| data_encoding::HEXLOWER.encode(&k.secret_bytes())))
    }

    pub fn import_backup(&self, hex: &str) -> std::io::Result<Keypair> {
        let bytes = data_encoding::HEXLOWER
            .decode(hex.trim().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "key must be 32 bytes"))?;
        let k = Keypair::from_secret_bytes(&arr);
        self.save(&k)?;
        Ok(k)
    }
}

#[cfg(any(windows, target_os = "macos"))]
mod keychain {
    use super::*;
    pub fn load() -> Option<Keypair> {
        let e = keyring::Entry::new(SERVICE, ACCOUNT).ok()?;
        let secret = e.get_secret().ok()?;
        let arr: [u8; 32] = secret.try_into().ok()?;
        Some(Keypair::from_secret_bytes(&arr))
    }
    pub fn save(key: &Keypair) -> bool {
        keyring::Entry::new(SERVICE, ACCOUNT).and_then(|e| e.set_secret(&key.secret_bytes())).is_ok()
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod keychain {
    use super::*;
    pub fn load() -> Option<Keypair> {
        let _ = (SERVICE, ACCOUNT);
        None
    }
    pub fn save(_key: &Keypair) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = IdentityStore::at(dir.path().join("sub").join("identity.key")).file_only();
        let k1 = store.load_or_create().unwrap();
        let k2 = store.load_or_create().unwrap();
        assert_eq!(k1.id(), k2.id());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(store.path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let backup = store.export_backup().unwrap().unwrap();
        let store2 = IdentityStore::at(dir.path().join("other.key")).file_only();
        assert_eq!(store2.import_backup(&backup).unwrap().id(), k1.id());
    }
}
