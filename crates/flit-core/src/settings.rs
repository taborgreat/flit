//! Per-user settings. Private to this machine, never events.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// When to pull the full-quality (FLAC) copy of other people's audio.
/// Playback alone only needs the Opus copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum DownloadPolicy {
    /// In the background, right after the Opus copy. The default.
    #[default]
    Always,
    /// Only when something needs it (export, processing, Bring in, editing).
    OnDemand,
    /// Background download only on an unmetered connection.
    WifiOnly,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub struct UserSettings {
    pub download_full_quality: DownloadPolicy,
    /// Display name shown to friends (not an identity).
    pub display_name: Option<String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl UserSettings {
    pub fn load(path: impl AsRef<Path>) -> crate::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut s: UserSettings = if path.is_file() {
            serde_json::from_str(&std::fs::read_to_string(&path)?)
                .map_err(|e| crate::Error::Decode(e.to_string()))?
        } else {
            UserSettings::default()
        };
        s.path = Some(path);
        Ok(s)
    }

    pub fn save(&self) -> crate::Result<()> {
        if let Some(p) = &self.path {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(p, serde_json::to_string_pretty(self).map_err(|e| crate::Error::Encode(e.to_string()))?)?;
        }
        Ok(())
    }

    /// Should the archive copy be fetched right away when new audio arrives?
    pub fn fetch_flac_in_background(&self, on_unmetered_network: bool) -> bool {
        match self.download_full_quality {
            DownloadPolicy::Always => true,
            DownloadPolicy::OnDemand => false,
            DownloadPolicy::WifiOnly => on_unmetered_network,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_policy() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = UserSettings::load(dir.path().join("settings.json")).unwrap();
        assert!(s.fetch_flac_in_background(false));
        s.download_full_quality = DownloadPolicy::WifiOnly;
        s.save().unwrap();
        let s2 = UserSettings::load(dir.path().join("settings.json")).unwrap();
        assert_eq!(s2.download_full_quality, DownloadPolicy::WifiOnly);
        assert!(!s2.fetch_flac_in_background(false));
        assert!(s2.fetch_flac_in_background(true));
    }
}
