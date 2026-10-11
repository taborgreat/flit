//! Small per-user files: the songs list, friends, settings, this machine's id.

use flit_core::Id;
use flit_net::AddrInfo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Friend {
    pub id: Id,
    pub name: String,
    pub addr: Option<AddrInfo>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    pub songs: Vec<(String, PathBuf)>,
    pub friends: Vec<Friend>,
    pub last_song: Option<PathBuf>,
    #[serde(default)]
    pub monitoring_bitrate: Option<u32>,
    #[serde(default)]
    pub submix: bool,
    #[serde(default)]
    pub hear_reference: bool,
    #[serde(default)]
    pub relay_only: bool,
    /// Tint tracks in the DAW with the colors of beings looking at them.
    #[serde(default = "yes")]
    pub show_presence_in_daw: bool,
    /// Where the release list lives (empty = updates not configured).
    #[serde(default)]
    pub update_manifest_url: String,
    /// "stable" | "beta"
    #[serde(default = "stable")]
    pub update_channel: String,
    /// What is known of each song (by project folder): id, org, access.
    #[serde(default)]
    pub song_notes: std::collections::HashMap<String, SongNote>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SongNote {
    pub id: Option<flit_core::SongId>,
    pub org: Option<flit_core::SongId>,
    pub access: Option<flit_core::Access>,
}

fn yes() -> bool {
    true
}
fn stable() -> String {
    "stable".into()
}

impl Registry {
    pub fn load(path: &Path) -> Registry {
        let mut r: Registry = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if r.update_channel.is_empty() {
            r.update_channel = stable();
        }
        r
    }

    pub fn save(&self, path: &Path) {
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, t);
        }
    }

    pub fn remember_song(&mut self, name: &str, dir: &Path) {
        self.songs.retain(|(_, d)| d != dir);
        self.songs.insert(0, (name.to_string(), dir.to_path_buf()));
        self.last_song = Some(dir.to_path_buf());
    }

    /// Remember a song's id, org and access; true when something changed.
    pub fn note_song(&mut self, dir: &Path, note: SongNote) -> bool {
        let key = dir.to_string_lossy().into_owned();
        if self.song_notes.get(&key) == Some(&note) {
            return false;
        }
        self.song_notes.insert(key, note);
        true
    }

    pub fn song_note(&self, dir: &Path) -> Option<&SongNote> {
        self.song_notes.get(dir.to_string_lossy().as_ref())
    }

    pub fn friend_name(&self, id: &Id) -> Option<String> {
        self.friends.iter().find(|f| f.id == *id).map(|f| f.name.clone())
    }

    pub fn add_friend(&mut self, id: Id, name: &str, addr: Option<AddrInfo>) {
        self.friends.retain(|f| f.id != id);
        self.friends.push(Friend { id, name: name.to_string(), addr });
    }
}

/// A stable id for this machine (the keeper role names a machine). Made
/// once, kept in the data folder.
pub fn machine_id(data_dir: &Path) -> String {
    let p = data_dir.join("machine");
    if let Ok(s) = std::fs::read_to_string(&p) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "machine".into());
    let rnd: u64 = rand_u64();
    let id = format!("{}-{:08x}", host.to_lowercase(), rnd as u32);
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(&p, &id);
    id
}

fn rand_u64() -> u64 {
    let seed = flit_core::Keypair::generate().id();
    u64::from_le_bytes(seed.0[..8].try_into().unwrap())
}
