//! Who is allowed in a song. Derived from `Crew` acts on the song thing of
//! main, every one signed by an owner Name at the time it was written.
//! Membership is per Name; approving a Name admits all its beings. An owner
//! may block one being without removing its Name.

use crate::act::{BeingRef, CrewAction};
use crate::id::Id;
use crate::org::ContainerKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewState {
    pub members: BTreeSet<Id>,
    pub owners: BTreeSet<Id>,
    /// Beings of crew Names that may not act in this song.
    pub blocked: BTreeSet<BeingRef>,
    /// Orgs: Names an owner invited that have not answered yet.
    #[serde(default)]
    pub invited: BTreeSet<Id>,
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

    pub fn is_invited(&self, id: &Id) -> bool {
        self.invited.contains(id)
    }

    /// Check a crew action signed by `signer` is allowed right now in a
    /// container of `kind`. Songs approve directly; orgs invite and the
    /// invited Name accepts; a personal shelf admits nobody.
    pub fn check(&self, signer: &Id, action: &CrewAction, kind: ContainerKind) -> crate::Result<()> {
        // the invited Name's answer and a member's leave are not owner acts
        match action {
            CrewAction::Accept | CrewAction::Decline => {
                return if kind != ContainerKind::Org {
                    Err(crate::Error::Invalid("only orgs invite".into()))
                } else if !self.is_invited(signer) {
                    Err(crate::Error::Invalid("not invited".into()))
                } else {
                    Ok(())
                };
            }
            CrewAction::Leave => {
                if !self.is_member(signer) {
                    return Err(crate::Error::NotInCrew(*signer));
                }
                if self.is_owner(signer) && self.owners.len() == 1 {
                    return Err(crate::Error::LastOwner);
                }
                return Ok(());
            }
            _ => {}
        }
        if !self.is_owner(signer) {
            return Err(crate::Error::NotOwner(*signer));
        }
        match action {
            CrewAction::Approve { .. } => match kind {
                ContainerKind::Song => Ok(()),
                ContainerKind::Org => Err(crate::Error::Invalid("orgs invite; the Name accepts".into())),
                ContainerKind::Personal => Err(crate::Error::Invalid("a personal shelf has one Name".into())),
            },
            CrewAction::Invite { id } => {
                if kind != ContainerKind::Org {
                    return Err(crate::Error::Invalid("only orgs invite".into()));
                }
                if self.is_member(id) {
                    return Err(crate::Error::Invalid("already a member".into()));
                }
                Ok(())
            }
            CrewAction::Demote { id } => {
                if !self.is_owner(id) {
                    return Err(crate::Error::NotOwner(*id));
                }
                if self.owners.len() == 1 {
                    return Err(crate::Error::LastOwner);
                }
                Ok(())
            }
            CrewAction::Accept | CrewAction::Decline | CrewAction::Leave => unreachable!(),
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
            CrewAction::Invite { id } => {
                self.invited.insert(*id);
            }
            CrewAction::Accept => {}
            CrewAction::Decline => {}
            CrewAction::Leave => {}
            CrewAction::Demote { id } => {
                self.owners.remove(id);
            }
        }
    }

    /// Accept, Decline and Leave act on the signer, which `apply` does not
    /// know: the chains call this with the act's Name.
    pub fn apply_by(&mut self, signer: &Id, action: &CrewAction) {
        match action {
            CrewAction::Accept => {
                self.invited.remove(signer);
                self.members.insert(*signer);
            }
            CrewAction::Decline => {
                self.invited.remove(signer);
            }
            CrewAction::Leave => {
                self.members.remove(signer);
                self.owners.remove(signer);
                self.blocked.retain(|b| b.name != *signer);
            }
            other => self.apply(other),
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
        assert!(crew.check(&b, &CrewAction::Approve { id: c, owner: false }, ContainerKind::Song).is_err());
        crew.check(&a, &CrewAction::Approve { id: b, owner: false }, ContainerKind::Song).unwrap();
        crew.apply(&CrewAction::Approve { id: b, owner: false });
        assert!(matches!(crew.check(&a, &CrewAction::Remove { id: a }, ContainerKind::Song), Err(crate::Error::LastOwner)));
        assert!(crew.check(&a, &CrewAction::Promote { id: c }, ContainerKind::Song).is_err());
        let bb = BeingRef { name: b, being: BeingId::new("bot") };
        crew.check(&a, &CrewAction::BlockBeing { being: bb.clone() }, ContainerKind::Song).unwrap();
        crew.apply(&CrewAction::BlockBeing { being: bb.clone() });
        assert!(crew.is_blocked(&bb));
        crew.check(&a, &CrewAction::Promote { id: b }, ContainerKind::Song).unwrap();
        crew.apply(&CrewAction::Promote { id: b });
        crew.check(&b, &CrewAction::Remove { id: a }, ContainerKind::Song).unwrap();
        crew.apply(&CrewAction::Remove { id: a });
        assert!(!crew.is_member(&a));
        assert!(crew.is_owner(&b));
        crew.apply(&CrewAction::Remove { id: b });
        assert!(!crew.is_blocked(&bb), "blocks go with the Name");
    }

    #[test]
    fn org_rules() {
        let a = Keypair::from_seed(1).id();
        let b = Keypair::from_seed(2).id();
        let mut crew = CrewState::default();
        crew.apply(&CrewAction::Approve { id: a, owner: true });
        let org = ContainerKind::Org;
        assert!(crew.check(&a, &CrewAction::Approve { id: b, owner: false }, org).is_err(), "orgs do not approve");
        assert!(crew.check(&b, &CrewAction::Accept, org).is_err(), "not invited yet");
        crew.check(&a, &CrewAction::Invite { id: b }, org).unwrap();
        crew.apply(&CrewAction::Invite { id: b });
        assert!(!crew.is_member(&b));
        crew.check(&b, &CrewAction::Accept, org).unwrap();
        crew.apply_by(&b, &CrewAction::Accept);
        assert!(crew.is_member(&b) && !crew.is_invited(&b));
        assert!(matches!(crew.check(&a, &CrewAction::Leave, org), Err(crate::Error::LastOwner)));
        assert!(crew.check(&a, &CrewAction::Demote { id: a }, org).is_err());
        crew.apply(&CrewAction::Promote { id: b });
        crew.check(&a, &CrewAction::Leave, org).unwrap();
        crew.check(&b, &CrewAction::Demote { id: a }, org).unwrap();
        crew.apply_by(&a, &CrewAction::Leave);
        assert!(!crew.is_member(&a) && crew.is_owner(&b));
        assert!(crew.check(&a, &CrewAction::Invite { id: b }, ContainerKind::Personal).is_err());
    }
}
