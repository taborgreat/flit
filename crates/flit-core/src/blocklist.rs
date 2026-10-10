//! IDs this app refuses everywhere. Private: never an event, never synced.

use crate::id::Id;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct BlockList {
    ids: BTreeSet<Id>,
    path: Option<PathBuf>,
}

impl BlockList {
    pub fn in_memory() -> Self {
        Self::default()
    }

    pub fn load(path: impl AsRef<Path>) -> crate::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut ids = BTreeSet::new();
        if path.is_file() {
            for line in std::fs::read_to_string(&path)?.lines() {
                if let Some(id) = Id::from_base32(line) {
                    ids.insert(id);
                }
            }
        }
        Ok(Self { ids, path: Some(path) })
    }

    fn save(&self) -> crate::Result<()> {
        if let Some(p) = &self.path {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let body: String = self.ids.iter().map(|i| format!("{i}\n")).collect();
            std::fs::write(p, body)?;
        }
        Ok(())
    }

    pub fn block(&mut self, id: Id) -> crate::Result<()> {
        self.ids.insert(id);
        self.save()
    }

    pub fn unblock(&mut self, id: &Id) -> crate::Result<()> {
        self.ids.remove(id);
        self.save()
    }

    pub fn is_blocked(&self, id: &Id) -> bool {
        self.ids.contains(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Id> {
        self.ids.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Keypair;

    #[test]
    fn persists() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("blocked.txt");
        let bad = Keypair::from_seed(9).id();
        let mut b = BlockList::load(&p).unwrap();
        b.block(bad).unwrap();
        let b2 = BlockList::load(&p).unwrap();
        assert!(b2.is_blocked(&bad));
        assert!(!b2.is_blocked(&Keypair::from_seed(1).id()));
    }
}
