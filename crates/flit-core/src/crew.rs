//! Who is allowed in a song. Derived from `Crew` acts on the song thing of
//! main, every one signed by an owner Name at the time it was written.
//! Membership is per Name; approving a Name admits all its beings. An owner
//! may block one being without removing its Name.

use crate::act::{BeingRef, CrewAction};
use crate::id::Id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewState {
    pub members: BTreeSet<Id>,
    pub owners: BTreeSet<Id>,
    /// Beings of crew Names that may not act in this song.
    pub blocked: BTreeSet<BeingRef>,
}

impl CrewState {
    pub fn is_member(&self, id: &Id) -> bool {
        self.members.contains(id)
    }

    pub fn is_owner(&self, id: &Id) -> bool {
        self.owners.contains(id)
    }

    pub fn is_blocked(&self, being: &BeingRef) -> bool {
        self.blocked.contains(being)
    }

    /// Check a crew action signed by `signer` is allowed right now.
    pub fn check(&self, signer: &Id, action: &CrewAction) -> crate::Result<()> {
        if !self.is_owner(signer) {
            return Err(crate::Error::NotOwner(*signer));
        }
        match action {
            CrewAction::Approve { .. } => Ok(()),
            CrewAction::Remove { id } => {
                if !self.is_member(id) {
                    return Err(crate::Error::NotInCrew(*id));
                }
                if self.is_owner(id) && self.owners.len() == 1 {
                    return Err(crate::Error::LastOwner);
                }
                Ok(())
            }
            CrewAction::Promote { id } => {
                if !self.is_member(id) {
                    return Err(crate::Error::NotInCrew(*id));
                }
                Ok(())
            }
            CrewAction::BlockBeing { being } | CrewAction::UnblockBeing { being } | CrewAction::Pardon { being } => {
                if !self.is_member(&being.name) {
                    return Err(crate::Error::NotInCrew(being.name));
                }
                Ok(())
            }
        }
    }

    /// Apply an action that already passed `check` (or is the genesis).
    pub fn apply(&mut self, action: &CrewAction) {
        match action {
            CrewAction::Approve { id, owner } => {
                self.members.insert(*id);
                if *owner {
                    self.owners.insert(*id);
                }
            }
            CrewAction::Remove { id } => {
                self.members.remove(id);
                self.owners.remove(id);
                self.blocked.retain(|b| b.name != *id);
            }
            CrewAction::Promote { id } => {
                self.owners.insert(*id);
            }
            CrewAction::BlockBeing { being } => {
                self.blocked.insert(being.clone());
            }
            CrewAction::UnblockBeing { being } => {
                self.blocked.remove(being);
            }
            CrewAction::Pardon { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::act::BeingId;
    use crate::id::Keypair;

    #[test]
    fn rules() {
        let a = Keypair::from_seed(1).id();
        let b = Keypair::from_seed(2).id();
        let c = Keypair::from_seed(3).id();
        let mut crew = CrewState::default();
        crew.apply(&CrewAction::Approve { id: a, owner: true });
        assert!(crew.check(&b, &CrewAction::Approve { id: c, owner: false }).is_err());
        crew.check(&a, &CrewAction::Approve { id: b, owner: false }).unwrap();
        crew.apply(&CrewAction::Approve { id: b, owner: false });
        assert!(matches!(crew.check(&a, &CrewAction::Remove { id: a }), Err(crate::Error::LastOwner)));
        assert!(crew.check(&a, &CrewAction::Promote { id: c }).is_err());
        let bb = BeingRef { name: b, being: BeingId::new("bot") };
        crew.check(&a, &CrewAction::BlockBeing { being: bb.clone() }).unwrap();
        crew.apply(&CrewAction::BlockBeing { being: bb.clone() });
        assert!(crew.is_blocked(&bb));
        crew.check(&a, &CrewAction::Promote { id: b }).unwrap();
        crew.apply(&CrewAction::Promote { id: b });
        crew.check(&b, &CrewAction::Remove { id: a }).unwrap();
        crew.apply(&CrewAction::Remove { id: a });
        assert!(!crew.is_member(&a));
        assert!(crew.is_owner(&b));
        crew.apply(&CrewAction::Remove { id: b });
        assert!(!crew.is_blocked(&bb), "blocks go with the Name");
    }
}
