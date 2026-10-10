//! Peers: order and fold never depend on arrival order or clocks; the
//! keeper seals, declines and holds; beings and keys; equivocation.
mod common;
use common::*;
use flit_core::*;

fn drums() -> TrackId {
    track("drums")
}
fn bass() -> TrackId {
    track("bass")
}

#[test]
fn same_acts_any_arrival_order_same_song() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, carol) = crew_of_three(dir.path());
    // Bob and Carol work without hearing each other (offline from the net)
    net.offline = vec![true, true, true];
    for i in 0..12u32 {
        let now = net.tick();
        net.song(1).setting(&bob, main(), &drums(), DeviceSlot(1), &comp(), &ParamId::from_u32(i), 0.5, None, now).unwrap();
        net.song(2).mixer(&carol, main(), &bass(), MixerChange::Volume(0.1 * i as f32), now).unwrap();
        net.song(0).tempo(&alice, main(), 100.0 + i as f64, now).unwrap();
    }
    net.offline = vec![false, false, false];
    net.deliver_shuffled(42);
    assert_eq!(net.song(0).unsealed_count(), 0);
    assert_same_song(&net, &main());
    // and the keeper's order is what every lens shows
    for lens in [Lens::Track(drums()), Lens::Track(bass()), Lens::Me(bob.being_ref()), Lens::Name(carol.name), Lens::SongSettings, Lens::Team] {
        let a: Vec<ActId> = net.songs[0].rows(&main(), &lens, false).iter().map(|r| r.act).collect();
        for s in &net.songs[1..] {
            let b: Vec<ActId> = s.rows(&main(), &lens, false).iter().map(|r| r.act).collect();
            assert_eq!(a, b, "lens {lens:?} differs between peers");
        }
    }
    // a fourth peer that receives the same moments in a different order
    let genesis = net.songs[0].genesis().clone();
    let gf = net.songs[0].chains().fact_for(&genesis.id).cloned();
    let mut late = Song::join(dir.path().join("d").join(".flit"), genesis, gf, "Test song", 1).unwrap();
    let mut moments: Vec<(Act, Fact)> = net.songs[0].chains().moments_after(&main(), 1).into_iter().map(|(f, a)| (a.clone(), f.clone())).collect();
    shuffle(&mut moments, 7);
    let mut pending = 0;
    for (a, f) in &moments {
        if late.import_moment(a.clone(), f.clone(), 5).unwrap() == AppendOutcome::Pending {
            pending += 1;
        }
    }
    assert!(pending > 0, "the shuffle really did reorder");
    assert_eq!(late.chains().pending_count(), 0, "everything resolved once its prev arrived");
    assert_eq!(tape(&late, &main()), tape(&net.songs[0], &main()));
    assert_eq!(late.main(), net.songs[0].main());
}

#[test]
fn a_clock_years_off_changes_nothing_but_the_time_axis() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, carol) = crew_of_three(dir.path());
    let far_past = 10u64;
    let far_future = 4_000_000_000_000u64;
    for i in 0..6u32 {
        let now = net.tick();
        net.song(0).tempo(&alice, main(), 100.0 + i as f64, now).unwrap();
        net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.5), far_future + i as u64).unwrap();
        net.song(2).mixer(&carol, main(), &bass(), MixerChange::Volume(0.5), far_past + i as u64).unwrap();
        net.deliver_all();
    }
    assert_same_song(&net, &main());
    let rows = net.songs[0].history(&main());
    let bob_rows: Vec<&Row> = rows.iter().filter(|r| r.being == bob.being_ref()).collect();
    let carol_rows: Vec<&Row> = rows.iter().filter(|r| r.being == carol.being_ref()).collect();
    assert!(bob_rows.iter().all(|r| r.stamped_at >= far_future));
    assert!(carol_rows.iter().all(|r| r.stamped_at < 100));
    // seal order interleaves them as they arrived at the keeper, not by clock
    let seal: Vec<u64> = rows.iter().filter(|r| r.being != alice.being_ref()).map(|r| r.seal_seq.unwrap()).collect();
    assert!(seal.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(bob_rows[0].seal_seq.unwrap() + 1, carol_rows[0].seal_seq.unwrap(), "round by round, whatever the clocks said");
}

#[test]
fn keeper_offline_then_back() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, _alice, bob, carol) = crew_of_three(dir.path());
    net.offline[0] = true;
    let now = net.tick();
    let b = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.3), now).unwrap();
    let c = net.song(2).mixer(&carol, main(), &bass(), MixerChange::Volume(0.6), now).unwrap();
    net.deliver_all();
    for i in [1, 2] {
        let s = &net.songs[i];
        assert_eq!(s.main().track(&drums()).unwrap().mixer.volume, 0.3, "applied live on peer {i}");
        assert_eq!(s.main().track(&bass()).unwrap().mixer.volume, 0.6);
        assert_eq!(s.unsealed_count(), 2, "waiting to be sealed");
        assert!(!s.chains().is_sealed(&b.id) && !s.chains().is_sealed(&c.id));
    }
    net.offline[0] = false;
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.unsealed_count(), 0);
        assert!(s.chains().is_sealed(&b.id) && s.chains().is_sealed(&c.id));
    }
    assert_same_song(&net, &main());
}

#[test]
fn keeper_moves_to_another_machine() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, carol) = crew_of_three(dir.path());
    let now = net.tick();
    net.song(0).promote(&alice, bob.name, now).unwrap();
    net.song(0).move_keeper(&alice, bob.name, "machine-2", now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.keeper(), Some((bob.name, "machine-2".into())));
    }
    assert!(!net.song(0).is_keeper());
    let now = net.tick();
    net.song(1).set_keeper(Some(bob.keeper_key()), now).unwrap();
    assert!(net.song(1).is_keeper());
    let now = net.tick();
    let c = net.song(2).mixer(&carol, main(), &bass(), MixerChange::Mute(true), now).unwrap();
    let a = net.song(0).tempo(&alice, main(), 90.0, now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        let f = s.chains().fact_for(&c.id).expect("sealed by the new keeper");
        assert_eq!(f.body.keeper, bob.name);
        assert_eq!(f.body.machine, "machine-2");
        assert_eq!(s.chains().fact_for(&a.id).unwrap().body.keeper, bob.name);
    }
    assert_same_song(&net, &main());
    // the old keeper's key no longer seals anything
    let now = net.tick();
    net.song(0).tempo(&alice, main(), 91.0, now).unwrap();
    assert_eq!(net.song(0).unsealed_count(), 1);
    net.deliver_all();
    assert_eq!(net.song(0).unsealed_count(), 0);
}

#[test]
fn two_beings_under_one_name_keep_separate_chains() {
    let dir = tempfile::tempdir().unwrap();
    let tabor = stamper(1, "tabor");
    let kade = tabor.as_being("kade");
    let mut song = new_song(&dir.path().join(".flit"), &tabor, "Tabor", 1);
    song.declare_being(&tabor, "kade", BeingKind::Human, "Kade", 0xE07A5F, None, vec![], 2).unwrap();
    for i in 0..3 {
        song.mixer(&tabor, main(), &drums(), MixerChange::Volume(0.1 * i as f32), 3 + i).unwrap();
        song.mixer(&kade, main(), &bass(), MixerChange::Volume(0.2 * i as f32), 3 + i).unwrap();
    }
    let t: Vec<u64> = song.chains().act_chain(&tabor.being_ref(), &main()).iter().map(|a| a.body.act_seq).collect();
    let k: Vec<u64> = song.chains().act_chain(&kade.being_ref(), &main()).iter().map(|a| a.body.act_seq).collect();
    assert_eq!(k, vec![1, 2, 3]);
    assert_eq!(t.len(), 3 + 3 + 1, "genesis, keeper, being, being Kade... Tabor's chain counts his own acts only");
    assert!(t.windows(2).all(|w| w[1] == w[0] + 1));
    assert_eq!(song.rows(&main(), &Lens::Me(kade.being_ref()), false).iter().map(|r| r.number).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(song.being_label(&kade.being_ref()), "Kade");
}

#[test]
fn delegated_being_key() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    let eng_key = Keypair::from_seed(50);
    let now = net.tick();
    // Alice certifies a key for her being "engineer" (ai, on a homelab)
    net.song(0).declare_being(&alice, "engineer", BeingKind::Ai, "Engineer", 0xC77DDB, Some(eng_key.id()), vec![], now).unwrap();
    net.deliver_all();
    let engineer = Stamper::delegated(eng_key.clone(), alice.name, "engineer", host(), "0.1.0", "homelab");
    let impostor = Stamper::delegated(eng_key.clone(), alice.name, "alice", host(), "0.1.0", "homelab");
    // the engineer's key verifies for the engineer, in Bob's copy too
    let now = net.tick();
    let a = net.song(1).chat(&engineer, main(), "hello from the homelab", None, now).unwrap();
    assert!(a.signed_by(&eng_key.id()));
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.chains().is_sealed(&a.id)));
    // but not for Alice's human being
    let now = net.tick();
    assert!(matches!(net.song(1).chat(&impostor, main(), "x", None, now), Err(Error::BadSignature(_))));
    // a being never grants, approves or declares
    let now = net.tick();
    assert!(matches!(net.song(1).approve(&engineer, bob.name, true, now), Err(Error::NotAllowed { .. })));
    assert!(matches!(net.song(1).grant(&engineer, "engineer", vec![drums()], true, now), Err(Error::NotAllowed { .. })));
    assert!(matches!(net.song(1).declare_being(&engineer, "evil", BeingKind::Human, "Evil", 0, None, vec![], now), Err(Error::NotAllowed { .. })));
    // revoked: rejected everywhere, facts stay
    let now = net.tick();
    net.song(0).revoke_being(&alice, "engineer", now).unwrap();
    net.deliver_all();
    let now = net.tick();
    assert!(matches!(net.song(1).chat(&engineer, main(), "still here?", None, now), Err(Error::BadSignature(_)) | Err(Error::RevokedBeing(_))));
    assert!(net.songs.iter().all(|s| s.chains().is_sealed(&a.id)));
}

#[test]
fn ai_being_lands_in_its_own_history_unless_granted() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, _bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    net.song(0).declare_being(&alice, "bot", BeingKind::Ai, "Bot", 0x4CC3D9, None, vec![], now).unwrap();
    net.deliver_all();
    let bot = alice.as_being("bot");
    let now = net.tick();
    let a = net.song(0).setting(&bot, main(), &drums(), DeviceSlot(1), &comp(), &p7(), 0.9, None, now).unwrap();
    assert_ne!(a.history(), main(), "no grant: the bot's change lands in its own history");
    let h = net.songs[0].histories().into_iter().find(|h| h.id == a.history()).unwrap();
    assert_eq!(h.name, "Bot's history");
    assert_eq!(h.parent, Some(main()));
    assert_eq!(h.created_by, Some(bot.being_ref()));
    net.deliver_all();
    assert!(net.songs[0].main().track(&drums()).is_none());
    assert_eq!(net.songs[0].state(&a.history()).unwrap().track(&drums()).unwrap().setting(DeviceSlot(1), &p7()), Some(0.9));
    assert_same_song(&net, &a.history());
    // a peer rejects a bot act aimed straight at main
    let forged = Act::sign(ActBody { history: main(), act_prev: Some(a.id), act_seq: 3, ..a.body.clone() }, &alice.key);
    assert!(matches!(net.song(1).import_act(forged, 9), Err(Error::NotAllowed { .. }) | Err(Error::BadActChain(_))));
    // granted: straight into main
    let now = net.tick();
    net.song(0).grant(&alice, "bot", vec![drums()], true, now).unwrap();
    net.deliver_all();
    let now = net.tick();
    let g = net.song(0).setting(&bot, main(), &drums(), DeviceSlot(1), &comp(), &p7(), 0.7, None, now).unwrap();
    assert_eq!(g.history(), main());
    net.deliver_all();
    assert_eq!(net.songs[2].main().track(&drums()).unwrap().setting(DeviceSlot(1), &p7()), Some(0.7));
    // still not on bass
    let now = net.tick();
    let b = net.song(0).mixer(&bot, main(), &bass(), MixerChange::Mute(true), now).unwrap();
    assert_ne!(b.history(), main());
}

#[test]
fn different_properties_both_land_same_property_declined() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, _alice, bob, carol) = crew_of_three(dir.path());
    // different properties on one track, in parallel
    net.offline = vec![true, true, true];
    let now = net.tick();
    let b = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.4), now).unwrap();
    let c = net.song(2).mixer(&carol, main(), &drums(), MixerChange::Pan(0.5), now).unwrap();
    net.offline = vec![false, false, false];
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.chains().fact_for(&b.id).unwrap().history(), main());
        assert_eq!(s.chains().fact_for(&c.id).unwrap().history(), main());
        let d = s.main().track(&drums()).unwrap();
        assert_eq!((d.mixer.volume, d.mixer.pan), (0.4, 0.5));
    }
    // the same property, both based on what they last saw
    net.offline = vec![true, true, true];
    let now = net.tick();
    let b2 = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.8), now).unwrap();
    let c2 = net.song(2).mixer(&carol, main(), &drums(), MixerChange::Volume(0.2), now).unwrap();
    assert_eq!(b2.body.based_on, c2.body.based_on);
    net.offline = vec![false, false, false];
    // Bob's reaches the keeper first
    net.deliver_all();
    let declined: Vec<HistoryInfo> = net.songs[0].histories().into_iter().filter(|h| h.id != main()).collect();
    assert_eq!(declined.len(), 1);
    assert_eq!(declined[0].name, "Carol's version of volume");
    assert_eq!(declined[0].parent, Some(main()));
    for s in &net.songs {
        assert_eq!(s.chains().fact_for(&b2.id).unwrap().history(), main(), "first sealed wins");
        let f = s.chains().fact_for(&c2.id).expect("nothing dropped");
        assert!(f.body.gate);
        assert_eq!(f.history(), declined[0].id);
        assert_eq!(s.main().track(&drums()).unwrap().mixer.volume, 0.8);
        assert_eq!(s.state(&declined[0].id).unwrap().track(&drums()).unwrap().mixer.volume, 0.2);
        assert_eq!(s.state(&declined[0].id).unwrap().track(&drums()).unwrap().mixer.pan, 0.5, "the history starts from what Carol saw");
    }
    assert_same_song(&net, &main());
    assert_same_song(&net, &declined[0].id);
    // Carol was told
    let notices = net.song(2).take_notices();
    assert!(notices.iter().any(|n| matches!(n, Notice::Declined { act, property, .. } if *act == c2.id && property == "volume")), "{notices:?}");
    // the song lens shows her moment as a gate in its own history, not on main
    assert!(net.songs[1].history(&main()).iter().all(|r| r.act != c2.id));
    let row = net.songs[1].history(&declined[0].id).into_iter().find(|r| r.act == c2.id).unwrap();
    assert!(row.gate && row.sealed && !row.inherited);
}

#[test]
fn notes_added_in_parallel_are_both_present() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, _alice, bob, carol) = crew_of_three(dir.path());
    net.offline = vec![true, true, true];
    let now = net.tick();
    net.song(1).notes(&bob, main(), &drums(), "pat1", vec![note(0, 36)], vec![], now).unwrap();
    net.song(2).notes(&carol, main(), &drums(), "pat1", vec![note(1, 38)], vec![], now).unwrap();
    net.offline = vec![false, false, false];
    net.deliver_all();
    assert_eq!(net.songs[0].histories().len(), 1, "different notes are different properties");
    for s in &net.songs {
        let clip = &s.main().track(&drums()).unwrap().clips["pat1"];
        assert_eq!(clip.iter().map(|n| n.key).collect::<Vec<_>>(), vec![36, 38]);
    }
    // the same note moved by both: one wins, the other is kept aside
    net.offline = vec![true, true, true];
    let now = net.tick();
    net.song(1).notes(&bob, main(), &drums(), "pat1", vec![Note { velocity: 50, ..note(0, 36) }], vec![], now).unwrap();
    net.song(2).notes(&carol, main(), &drums(), "pat1", vec![], vec![note(0, 36)], now).unwrap();
    net.offline = vec![false, false, false];
    net.deliver_all();
    assert_eq!(net.songs[0].histories().len(), 2);
    assert_eq!(net.songs[0].main().track(&drums()).unwrap().clips["pat1"][0].velocity, 50);
}

#[test]
fn equivocation_is_flagged_and_held_until_pardoned() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    let real = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.4), now).unwrap();
    net.deliver_all();
    // Bob (or someone with his key) makes a second act with the same number
    let mut body = real.body.clone();
    body.payload = Payload::Mixer(MixerChange::Volume(0.9));
    let twin = Act::sign(body, &bob.key);
    assert_ne!(twin.id, real.id);
    assert_eq!(net.song(0).import_act(twin.clone(), 9).unwrap(), AppendOutcome::Equivocation);
    assert_eq!(net.song(2).import_act(twin.clone(), 9).unwrap(), AppendOutcome::Equivocation);
    let ev = net.songs[0].chains().evidence()[0].clone();
    assert_eq!(ev.acts, [real.id, twin.id]);
    assert!(net.songs[0].chains().held().contains(&bob.being_ref()));
    // the keeper's owner flags it with the evidence; every peer sees the flag
    let now = net.tick();
    net.song(0).flag(&alice, bob.being_ref(), FlagKind::Equivocation { acts: ev.acts }, "", now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.beings().flags_for(&bob.being_ref()).len(), 1);
    }
    // Bob's next act is held, not sealed; applied live on his peer only
    let now = net.tick();
    let next = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.5), now).unwrap();
    net.deliver_all();
    assert!(!net.songs[0].chains().is_sealed(&next.id));
    assert_eq!(net.songs[0].main().track(&drums()).unwrap().mixer.volume, 0.5, "optimistic on the keeper too");
    // an owner decides: accept
    let now = net.tick();
    net.song(0).pardon(&alice, bob.being_ref(), now).unwrap();
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.chains().is_sealed(&next.id)));
    assert_same_song(&net, &main());
}

#[test]
fn offline_acts_seal_when_no_overlap_otherwise_aside() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, _alice, bob, carol) = crew_of_three(dir.path());
    net.offline[1] = true;
    let now = net.tick();
    let b1 = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.4), now).unwrap();
    let b2 = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Pan(-0.5), now + 1).unwrap();
    let b3 = net.song(1).tempo(&bob, main(), 140.0, now + 2).unwrap();
    // meanwhile Carol changes the tempo and it is sealed
    let c = net.song(2).tempo(&carol, main(), 90.0, now).unwrap();
    net.deliver_all();
    assert!(net.songs[0].chains().is_sealed(&c.id));
    net.offline[1] = false;
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.chains().fact_for(&b1.id).unwrap().history(), main());
        assert_eq!(s.chains().fact_for(&b2.id).unwrap().history(), main());
        let f = s.chains().fact_for(&b3.id).unwrap();
        assert!(f.body.gate && f.history() != main(), "the overlapping tempo change is kept aside");
        assert_eq!(s.main().settings.tempo, 90.0);
    }
    let aside = net.songs[0].histories().into_iter().find(|h| h.id != main()).unwrap();
    assert_eq!(aside.name, "Bob's version of tempo");
    assert_eq!(net.songs[0].state(&aside.id).unwrap().settings.tempo, 140.0);
    assert_same_song(&net, &main());
}

#[test]
fn required_version_holds_then_seals_aside_after_update() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    assert_eq!(net.songs[0].chains().required_version().as_deref(), Some("0.1.0"), "default: the creator's version");
    let now = net.tick();
    net.song(0).set_required_version(&alice, "0.2.0", now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.main().settings.required_version.as_deref(), Some("0.2.0"), "every peer sees the notice");
    }
    // Bob is still on 0.1.0: his act applies live but is held
    let now = net.tick();
    let old = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.3), now).unwrap();
    net.deliver_all();
    assert!(!net.songs[0].chains().is_sealed(&old.id));
    assert!(net.song(0).take_notices().iter().any(|n| matches!(n, Notice::Held { act, .. } if *act == old.id)));
    // the keeper's own machine must run the required version too
    let now = net.tick();
    let a_old = net.song(0).tempo(&alice, main(), 77.0, now).unwrap();
    assert!(!net.songs[0].chains().is_sealed(&a_old.id));
    let alice2 = Stamper { app_version: "0.2.0".into(), ..alice.clone() };
    let now = net.tick();
    let a_new = net.song(0).tempo(&alice2, main(), 78.0, now).unwrap();
    assert!(net.songs[0].chains().is_sealed(&a_new.id));
    // Bob updates and resends: the new act is sealed; the old one is sealed aside
    let bob2 = Stamper { app_version: "0.2.0".into(), ..bob.clone() };
    let now = net.tick();
    let new = net.song(1).mixer(&bob2, main(), &drums(), MixerChange::Volume(0.3), now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.chains().fact_for(&new.id).unwrap().history(), main());
        let f = s.chains().fact_for(&old.id).expect("held act sealed once Bob updated");
        assert!(f.body.gate && f.history() != main());
        assert_eq!(s.unsealed_count(), 0);
    }
    let aside = net.songs[0].histories().into_iter().find(|h| h.name == "Bob on FLit 0.1.0").expect("kept, never applied to main");
    assert_eq!(net.songs[0].chains().fact_for(&old.id).unwrap().history(), aside.id);
    assert_same_song(&net, &main());
}

#[test]
fn late_peer_catches_up_with_moments_only() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    for i in 0..5 {
        let now = net.tick();
        net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.1 * i as f32), now).unwrap();
        net.song(0).tempo(&alice, main(), 100.0 + i as f64, now).unwrap();
    }
    let now = net.tick();
    let from = net.songs[0].latest(&main()).unwrap();
    let h = net.song(0).new_history(&alice, "take 2", from, now).unwrap();
    net.song(0).tempo(&alice, h, 150.0, now).unwrap();
    net.deliver_all();
    let genesis = net.songs[0].genesis().clone();
    let gf = net.songs[0].chains().fact_for(&genesis.id).cloned();
    let dave = Song::join(dir.path().join("d").join(".flit"), genesis, gf, "Test song", 1).unwrap();
    net.songs.push(dave);
    net.offline.push(false);
    net.catch_up(0, 3);
    assert_eq!(tape(&net.songs[3], &main()), tape(&net.songs[0], &main()));
    assert_eq!(tape(&net.songs[3], &h), tape(&net.songs[0], &h));
    assert_eq!(net.songs[3].state(&h), net.songs[0].state(&h));
    assert_eq!(net.songs[3].chains().pending_count(), 0);
}

#[test]
fn flags_presence_and_removal() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, carol) = crew_of_three(dir.path());
    // a flag is just a moment everyone sees
    let now = net.tick();
    net.song(2).flag(&carol, bob.being_ref(), FlagKind::ClaimedKind, "too fast to be human", now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        let f = s.beings().flags_for(&bob.being_ref());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].by, carol.name);
    }
    // presence is recorded, hidden by default, folds with the moment
    let now = net.tick();
    let before = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.2), now).unwrap();
    let p = net.song(1).presence(&bob, main(), PresencePayload { view: View::Mixer, track: Some(drums()), device: None, region: None }, Some(SongPos::from_beats(8.0)), now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        assert!(s.history(&main()).iter().all(|r| r.act != p.id), "hidden by default");
        assert!(s.rows(&main(), &Lens::Song, true).iter().any(|r| r.act == p.id));
        let ps = &s.main().presence[&bob.being_ref()];
        assert_eq!((ps.view, ps.track.as_ref(), ps.pos), (View::Mixer, Some(&drums()), Some(SongPos::from_beats(8.0))));
        assert!(s.state_at(&before.id).unwrap().presence.get(&bob.being_ref()).is_none(), "scrubbing shows where everyone was");
    }
    // a blocked being may not act; the Name's other beings may
    let now = net.tick();
    net.song(0).block_being(&alice, bob.being_ref(), now).unwrap();
    net.deliver_all();
    let now = net.tick();
    assert!(matches!(net.song(1).tempo(&bob, main(), 99.0, now), Err(Error::BlockedBeing(_))));
    net.song(0).unblock_being(&alice, bob.being_ref(), now).unwrap();
    net.deliver_all();
    let now = net.tick();
    net.song(1).tempo(&bob, main(), 99.0, now).unwrap();
    net.deliver_all();
    // removed from the crew: nothing new, everything old stays
    let now = net.tick();
    net.song(0).remove(&alice, bob.name, now).unwrap();
    net.deliver_all();
    let now = net.tick();
    assert!(matches!(net.song(1).tempo(&bob, main(), 98.0, now), Err(Error::NotInCrew(_))));
    for s in &net.songs {
        assert_eq!(s.main().track(&drums()).unwrap().mixer.volume, 0.2);
        assert_eq!(s.main().settings.tempo, 99.0);
        assert!(!s.crew().is_member(&bob.name));
    }
    assert_same_song(&net, &main());
}

#[test]
fn validation_rejects_bad_acts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, _alice, bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    let good = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.4), now).unwrap();
    // bad signature
    let mut bad = good.clone();
    bad.sig = Keypair::from_seed(99).sign(bad.id.as_bytes());
    assert!(matches!(net.song(0).import_act(bad, 1), Err(Error::BadSignature(_))));
    // changed body
    let mut bad = good.clone();
    bad.body.stamped_at += 1;
    assert!(matches!(net.song(0).import_act(bad, 1), Err(Error::BadId(_))));
    // a stranger
    let eve = stamper(66, "eve");
    let mut body = good.body.clone();
    body.name = eve.name;
    body.being = BeingId::new("eve");
    assert!(matches!(net.song(0).import_act(Act::sign(body, &eve.key), 1), Err(Error::NotInCrew(_))));
    // out of range
    assert!(net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(99.0), now).is_err());
    // unknown history
    let mut body = good.body.clone();
    body.history = Hash::of(b"nope");
    body.act_prev = None;
    body.act_seq = 1;
    assert!(matches!(net.song(0).import_act(Act::sign(body, &bob.key), 1), Err(Error::UnknownHistory(_))));
    // wrong thing for the payload
    let mut body = good.body.clone();
    body.thing = Thing::Song;
    body.act_prev = Some(good.id);
    body.act_seq += 1;
    assert!(matches!(net.song(0).import_act(Act::sign(body, &bob.key), 1), Err(Error::Invalid(_))));
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.chains().is_sealed(&good.id)));
}
