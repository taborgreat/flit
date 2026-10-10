//! Beings under Names, as a song knows them: declared by `Being` acts
//! (signed by the Name key), granted tracks by `Grant` acts, flagged by
//! `Flag` acts. Derived state; the acts are the truth.

use crate::act::{BeingAction, BeingId, BeingKind, BeingPayload, BeingRef, FlagKind, GrantPayload, TrackId, ActId};
use crate::blob::Hash;
use crate::id::Id;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeingInfo {
    pub kind: BeingKind,
    pub display_name: String,
    /// 0xRRGGBB.
    pub color: u32,
    /// Certified delegated key, if the being runs on another machine.
    pub key: Option<Id>,
    /// Songs the delegated key is good for (empty = any).
    pub scope: Vec<Hash>,
    pub revoked: bool,
}

impl BeingInfo {
    pub fn is_ai(&self) -> bool {
        matches!(self.kind, BeingKind::Ai | BeingKind::Scripted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagRecord {
    pub by: Id,
    pub act: ActId,
    pub being: BeingRef,
    pub kind: FlagKind,
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beings {
    pub by_name: BTreeMap<Id, BTreeMap<BeingId, BeingInfo>>,
    /// ai / scripted beings allowed on these tracks directly (any history).
    pub grants: BTreeMap<BeingRef, BTreeSet<TrackId>>,
    pub flags: Vec<FlagRecord>,
}

impl Beings {
    pub fn get(&self, being: &BeingRef) -> Option<&BeingInfo> {
        self.by_name.get(&being.name)?.get(&being.being)
    }

    pub fn of_name(&self, name: &Id) -> Vec<(BeingId, &BeingInfo)> {
        self.by_name.get(name).map(|m| m.iter().map(|(k, v)| (k.clone(), v)).collect()).unwrap_or_default()
    }

    /// All declared, not revoked beings in the song.
    pub fn all(&self) -> Vec<(BeingRef, &BeingInfo)> {
        let mut v = Vec::new();
        for (name, m) in &self.by_name {
            for (b, info) in m {
                if !info.revoked {
                    v.push((BeingRef { name: *name, being: b.clone() }, info));
                }
            }
        }
        v
    }

    pub fn is_ai(&self, being: &BeingRef) -> bool {
        self.get(being).map(|b| b.is_ai()).unwrap_or(false)
    }

    pub fn is_revoked(&self, being: &BeingRef) -> bool {
        self.get(being).map(|b| b.revoked).unwrap_or(false)
    }

    pub fn display_name(&self, being: &BeingRef) -> Option<&str> {
        self.get(being).map(|b| b.display_name.as_str()).filter(|s| !s.is_empty())
    }

    pub fn color(&self, being: &BeingRef) -> Option<u32> {
        self.get(being).map(|b| b.color)
    }

    pub fn granted(&self, being: &BeingRef, track: &TrackId) -> bool {
        self.grants.get(being).map(|t| t.contains(track)).unwrap_or(false)
    }

    pub fn grants_of(&self, being: &BeingRef) -> Vec<TrackId> {
        self.grants.get(being).map(|t| t.iter().cloned().collect()).unwrap_or_default()
    }

    /// Does `signer` speak for `being` in song `song`? The Name key always
    /// does; a certified being key does for that being only, within scope,
    /// until revoked.
    pub fn signer_for(&self, being: &BeingRef, signer: &Id, song: Option<&Hash>) -> bool {
        if *signer == being.name {
            return true;
        }
        let Some(info) = self.get(being) else { return false };
        if info.revoked {
            return false;
        }
        match &info.key {
            Some(k) if k == signer => info.scope.is_empty() || song.map(|s| info.scope.contains(s)).unwrap_or(false),
            _ => false,
        }
    }

    /// Certified keys of a Name (for keeper signatures from a delegated machine).
    pub fn certified_keys(&self, name: &Id) -> Vec<Id> {
        self.by_name.get(name).map(|m| m.values().filter(|b| !b.revoked).filter_map(|b| b.key).collect()).unwrap_or_default()
    }

    pub fn apply_being(&mut self, name: Id, p: &BeingPayload) {
        let m = self.by_name.entry(name).or_default();
        match &p.action {
            BeingAction::Create { kind, display_name, color, key, scope } => {
                m.insert(p.being.clone(), BeingInfo { kind: *kind, display_name: display_name.trim().to_string(), color: *color, key: *key, scope: scope.clone(), revoked: false });
            }
            BeingAction::Update { display_name, color } => {
                if let Some(b) = m.get_mut(&p.being) {
                    b.display_name = display_name.trim().to_string();
                    b.color = *color;
                }
            }
            BeingAction::Revoke => {
                if let Some(b) = m.get_mut(&p.being) {
                    b.revoked = true;
                }
            }
        }
    }

    pub fn apply_grant(&mut self, g: &GrantPayload) {
        let set = self.grants.entry(g.being.clone()).or_default();
        for t in &g.tracks {
            if g.grant {
                set.insert(t.clone());
            } else {
                set.remove(t);
            }
        }
    }

    pub fn flags_for(&self, being: &BeingRef) -> Vec<&FlagRecord> {
        self.flags.iter().filter(|f| f.being == *being).collect()
    }
}
