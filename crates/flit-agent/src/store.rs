//! Small per-user files: the songs list, friends, settings.

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
}

impl Registry {
    pub fn load(path: &Path) -> Registry {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
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

    pub fn friend_name(&self, id: &Id) -> Option<String> {
        self.friends.iter().find(|f| f.id == *id).map(|f| f.name.clone())
    }

    pub fn add_friend(&mut self, id: Id, name: &str, addr: Option<AddrInfo>) {
        self.friends.retain(|f| f.id != id);
        self.friends.push(Friend { id, name: name.to_string(), addr });
    }
}
