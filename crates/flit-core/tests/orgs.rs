//! PACK 2 §5 at the core: orgs (countersigned membership, last owner,
//! removed members), song access through an org, albums as facts.

mod common;

use common::*;
use flit_core::*;
use std::collections::BTreeSet;

fn org_of_two(root: &std::path::Path) -> (Net, Stamper, Stamper, SongId) {
    let alice = stamper(1, "alice");
    let bob = stamper(2, "bob");
    let a = Song::create_kind(root.join("a-org"), &alice, "Tigard Beats", ContainerKind::Org, 1).unwrap();
    assert_eq!(a.kind(), ContainerKind::Org);
    let genesis = a.genesis().clone();
    let gf = a.chains().fact_for(&genesis.id).cloned();
    let b = Song::join(root.join("b-org"), genesis, gf, "Tigard Beats", 1).unwrap();
    assert_eq!(b.kind(), ContainerKind::Org, "the kind travels in the genesis");
    let id = a.id();
    (Net::new(vec![a, b]), alice, bob, id)
}

#[test]
fn invite_without_accept_is_not_membership() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _) = org_of_two(dir.path());
    // bob cannot act before an invite
    let now = net.tick();
    assert!(net.song(1).accept(&bob, now).is_err(), "no invite yet");
    net.catch_up(0, 1);
    net.song(0).invite(&alice, bob.name, now).unwrap();
    net.deliver_all();
    assert!(net.song(1).crew().is_invited(&bob.name));
    assert!(!net.song(0).crew().is_member(&bob.name), "invite alone is not membership");
    assert!(!net.song(1).crew().is_member(&bob.name));
    // the invited Name accepts: a member on every peer, sealed by the org keeper
    let now = net.tick();
    let accept = net.song(1).accept(&bob, now).unwrap();
    net.deliver_all();
    assert!(net.song(0).crew().is_member(&bob.name));
    assert!(net.song(1).crew().is_member(&bob.name));
    assert!(net.song(1).chains().is_sealed(&accept.id), "the keeper sealed the Accept");
    assert_same_song(&net, &main());
    // decline is a gate; invite + decline = not a member
    let carol = stamper(3, "carol");
    let now = net.tick();
    net.song(0).invite(&alice, carol.name, now).unwrap();
    let genesis = net.song(0).genesis().clone();
    let gf = net.song(0).chains().fact_for(&genesis.id).cloned();
    let c = Song::join(dir.path().join("c-org"), genesis, gf, "x", now).unwrap();
    net.add(c);
    net.catch_up(0, 2);
    let d = net.song(2).decline(&carol, now).unwrap();
    assert!(d.gate());
    net.deliver_all();
    assert!(!net.song(0).crew().is_member(&carol.name) && !net.song(0).crew().is_invited(&carol.name));
}

#[test]
fn last_owner_cannot_leave_without_promoting() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _) = org_of_two(dir.path());
    let now = net.tick();
    net.catch_up(0, 1);
    net.song(0).invite(&alice, bob.name, now).unwrap();
    net.deliver_all();
    net.song(1).accept(&bob, now).unwrap();
    net.deliver_all();
    assert!(matches!(net.song(0).leave(&alice, now), Err(Error::LastOwner)));
    assert!(net.song(0).demote(&alice, alice.name, now).is_err());
    net.song(0).promote(&alice, bob.name, now).unwrap();
    net.deliver_all();
    let left = net.song(0).leave(&alice, now).unwrap();
    assert!(left.gate());
    net.deliver_all();
    assert!(!net.song(1).crew().is_member(&alice.name));
    assert!(net.song(1).crew().is_owner(&bob.name));
    assert_same_song(&net, &main());
}

#[test]
fn removed_member_is_rejected_but_facts_stay() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _) = org_of_two(dir.path());
    let now = net.tick();
    net.catch_up(0, 1);
    net.song(0).invite(&alice, bob.name, now).unwrap();
    net.deliver_all();
    net.song(1).accept(&bob, now).unwrap();
    net.deliver_all();
    let hi = net.song(1).chat(&bob, main(), "hi", None, now).unwrap();
    net.deliver_all();
    assert!(net.song(0).chains().is_sealed(&hi.id));
    net.song(0).remove(&alice, bob.name, now).unwrap();
    net.deliver_all();
    assert!(matches!(net.song(1).chat(&bob, main(), "still here?", None, now), Err(Error::NotInCrew(_))));
    assert!(net.song(0).chains().contains(&hi.id), "old facts stay");
    assert_same_song(&net, &main());
}

#[test]
fn org_member_may_act_in_an_access_org_song_but_not_a_private_one() {
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let bob = stamper(2, "bob");
    let org = Keypair::from_seed(9).id(); // any id stands for the org here; membership comes from the agent
    let a = new_song(&dir.path().join("a").join(".flit"), &alice, "Alice", 1);
    let genesis = a.genesis().clone();
    let gf = a.chains().fact_for(&genesis.id).cloned();
    let b = Song::join(dir.path().join("b").join(".flit"), genesis, gf, "Test song", 1).unwrap();
    let mut net = Net::new(vec![a, b]);
    net.catch_up(0, 1);
    let now = net.tick();
    // bob is not in the crew: nothing lands
    assert!(matches!(net.song(1).chat(&bob, main(), "hello?", None, now), Err(Error::NotInCrew(_))));
    // the song joins an org with access = org, and both copies know bob is an org member
    let org_hash = Hash::of(org.as_bytes());
    let set = net.song(0).set_org(&alice, Some(org_hash), Access::Org, now).unwrap();
    net.deliver_all();
    assert_eq!(net.song(1).access(), Access::Org);
    let members: BTreeSet<Id> = [alice.name, bob.name].into_iter().collect();
    net.song(0).set_org_members(Some(members.clone()));
    net.song(1).set_org_members(Some(members));
    let hi = net.song(1).chat(&bob, main(), "hello", None, now).unwrap();
    net.deliver_all();
    assert!(net.song(0).chains().is_sealed(&hi.id), "an org member's act is sealed by the song keeper");
    // the access change is in the song's History with who changed it
    let rows = net.song(1).rows(&main(), &Lens::SongSettings, false);
    let row = rows.iter().find(|r| r.act == set.id).expect("access change on the tape");
    assert_eq!(row.being, alice.being_ref());
    assert!(row.summary.contains("access org"), "{}", row.summary);
    // back to private: the org member is out again, old facts stay
    net.song(0).set_org(&alice, Some(org_hash), Access::Private, now).unwrap();
    net.deliver_all();
    assert!(matches!(net.song(1).chat(&bob, main(), "closed?", None, now), Err(Error::NotInCrew(_))));
    assert!(net.song(1).chains().contains(&hi.id));
    assert_same_song(&net, &main());
}

#[test]
fn songs_refuse_album_facts_and_orgs_refuse_sound() {
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let mut song = new_song(&dir.path().join("s").join(".flit"), &alice, "Alice", 1);
    assert!(song.create_album(&alice, "nope", 2).is_err(), "albums live in orgs and shelves");
    let mut org = Song::create_kind(dir.path().join("o"), &alice, "Band", ContainerKind::Org, 1).unwrap();
    assert!(org.tempo(&alice, main(), 90.0, 2).is_err(), "an org holds no sound");
    assert!(org.set_org(&alice, None, Access::Org, 2).is_err(), "org/access is a song setting");
    let mut shelf = Song::create_kind(dir.path().join("p"), &alice, "", ContainerKind::Personal, 1).unwrap();
    assert!(shelf.invite(&alice, Keypair::from_seed(2).id(), 2).is_err(), "a personal shelf admits nobody");
    assert!(shelf.create_album(&alice, "sketches", 2).is_ok());
}

#[test]
fn album_facts_rewind_and_curation_rights() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _) = org_of_two(dir.path());
    let now = net.tick();
    net.catch_up(0, 1);
    net.song(0).invite(&alice, bob.name, now).unwrap();
    net.deliver_all();
    net.song(1).accept(&bob, now).unwrap();
    net.deliver_all();
    let s1 = Hash::of(b"song-1");
    let s2 = Hash::of(b"song-2");
    let s3 = Hash::of(b"song-3");
    // alice makes the album; bob (a member) adds songs and reorders
    let album = net.song(0).create_album(&alice, "EP", now).unwrap();
    net.deliver_all();
    net.song(1).album(&bob, album, AlbumAction::AddSong { song: s1, after: None }, now).unwrap();
    net.song(1).album(&bob, album, AlbumAction::AddSong { song: s2, after: None }, now).unwrap();
    net.song(1).album(&bob, album, AlbumAction::AddSong { song: s3, after: Some(s1) }, now).unwrap();
    net.deliver_all();
    assert_eq!(net.song(0).albums()[&album].songs, vec![s1, s3, s2]);
    let before = net.song(1).album(&bob, album, AlbumAction::Reorder(vec![s2, s1]), now).unwrap();
    net.deliver_all();
    assert_eq!(net.song(0).albums()[&album].songs, vec![s2, s1, s3], "songs left out of a reorder keep their place after");
    // curation: a member who did not create it may not rename or archive; the owner may
    assert!(matches!(net.song(1).album(&bob, album, AlbumAction::Rename("LP".into()), now), Err(Error::NotAllowed { .. })));
    assert!(net.song(1).album(&bob, album, AlbumAction::Archive(true), now).is_err());
    net.song(0).album(&alice, album, AlbumAction::Rename("LP".into()), now).unwrap();
    net.song(0).album(&alice, album, AlbumAction::Archive(true), now).unwrap();
    net.deliver_all();
    let st = &net.song(1).albums()[&album];
    assert_eq!(st.name, "LP");
    assert!(st.archived);
    // rewind: before the reorder the order was s1 s3 s2
    let past = net.song(1).state_at(&before.id).unwrap();
    assert_eq!(past.albums[&album].songs, vec![s2, s1, s3]);
    let prev = net.song(1).rows(&main(), &Lens::Album(album), false);
    let i = prev.iter().position(|r| r.act == before.id).unwrap();
    let at_add = net.song(1).state_at(&prev[i - 1].act).unwrap();
    assert_eq!(at_add.albums[&album].songs, vec![s1, s3, s2]);
    // the album lens shows only the album's facts, create first
    assert_eq!(prev[0].act, album);
    assert!(prev.iter().all(|r| r.kind == Kind::Album));
    assert!(prev.iter().any(|r| r.summary.contains("renamed")));
    assert_same_song(&net, &main());
    // a member's own album: the creator curates it without being an owner
    let mine = net.song(1).create_album(&bob, "bob's picks", now).unwrap();
    net.deliver_all();
    net.song(1).album(&bob, mine, AlbumAction::SetDescription("late nights".into()), now).unwrap();
    net.deliver_all();
    assert_eq!(net.song(0).albums()[&mine].description, "late nights");
}
