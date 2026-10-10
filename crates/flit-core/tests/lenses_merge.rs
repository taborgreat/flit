//! Lenses, histories, Go back, Undo and Merge.
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
fn lenses_read_their_own_chains() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    let d1 = net.song(0).device(&alice, main(), &drums(), DeviceSlot(1), Some(comp()), now).unwrap();
    net.deliver_all();
    let now = net.tick();
    let s1 = net.song(1).setting(&bob, main(), &drums(), DeviceSlot(1), &comp(), &p7(), 0.2, None, now).unwrap();
    let m1 = net.song(1).mixer(&bob, main(), &drums(), MixerChange::Volume(0.5), now).unwrap();
    let b1 = net.song(1).mixer(&bob, main(), &bass(), MixerChange::Volume(0.6), now).unwrap();
    net.deliver_all();
    let now = net.tick();
    let s2 = net.song(0).setting(&alice, main(), &drums(), DeviceSlot(1), &comp(), &p7(), 0.3, None, now).unwrap();
    net.deliver_all();
    let song = &net.songs[2];
    // Track lens: the track and its devices, fact order, numbered by fact_seq
    let t: Vec<ActId> = song.rows(&main(), &Lens::Track(drums()), false).iter().map(|r| r.act).collect();
    assert_eq!(t, vec![d1.id, s1.id, m1.id, s2.id]);
    // Device lens: that device's chain only
    let d: Vec<(ActId, u64)> = song.rows(&main(), &Lens::Device { track: drums(), slot: DeviceSlot(1) }, false).iter().map(|r| (r.act, r.number)).collect();
    assert_eq!(d, vec![(s1.id, 1), (s2.id, 2)]);
    // Me lens: act order, numbered by act_seq
    let me: Vec<u64> = song.rows(&main(), &Lens::Me(bob.being_ref()), false).iter().map(|r| r.number).collect();
    assert_eq!(me, vec![1, 2, 3, 4], "being, setting, volume, bass volume");
    let me_acts: Vec<ActId> = song.rows(&main(), &Lens::Me(bob.being_ref()), false).iter().map(|r| r.act).collect();
    assert_eq!(&me_acts[1..], &[s1.id, m1.id, b1.id]);
    // Combine: Me x Track drums
    let combo: Vec<ActId> = song.rows(&main(), &Lens::Combine(Box::new(Lens::Me(bob.being_ref())), Box::new(Lens::Track(drums()))), false).iter().map(|r| r.act).collect();
    assert_eq!(combo, vec![s1.id, m1.id]);
    // Song lens: seal order
    let seal: Vec<u64> = song.history(&main()).iter().map(|r| r.number).collect();
    assert!(seal.windows(2).all(|w| w[0] < w[1]));
    // scrubbing to a moment is the same from any lens
    for lens in [Lens::Song, Lens::Track(drums()), Lens::Me(bob.being_ref()), Lens::Name(bob.name)] {
        let r = song.rows(&main(), &lens, false).into_iter().find(|r| r.act == m1.id).unwrap();
        let st = song.state_at(&r.act).unwrap();
        assert_eq!(st.track(&drums()).unwrap().mixer.volume, 0.5);
        assert_eq!(st.track(&drums()).unwrap().setting(DeviceSlot(1), &p7()), Some(0.2));
        assert!(st.track(&bass()).is_none(), "bass volume came after");
    }
    // summaries name people by their declared names
    let row = song.history(&main()).into_iter().find(|r| r.act == s2.id).unwrap();
    assert_eq!(row.being, alice.being_ref());
    assert_eq!(song.being_label(&row.being), "Alice");
    assert!(row.summary.contains("Comp 7"));
}

#[test]
fn histories_go_back_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    net.song(0).tempo(&alice, main(), 100.0, now).unwrap();
    let mid = net.song(0).mixer(&alice, main(), &drums(), MixerChange::Volume(0.5), now).unwrap();
    net.song(0).mixer(&alice, main(), &drums(), MixerChange::Volume(0.9), now).unwrap();
    net.deliver_all();
    // a history from a moment before the last change
    let now = net.tick();
    let h = net.song(1).new_history(&bob, "quieter", mid.id, now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.state(&h).unwrap().track(&drums()).unwrap().mixer.volume, 0.5, "starts from that moment");
        assert_eq!(s.main().track(&drums()).unwrap().mixer.volume, 0.9);
        let rows = s.history(&h);
        assert!(rows.iter().filter(|r| r.inherited).any(|r| r.act == mid.id));
        assert!(rows.iter().all(|r| r.inherited), "nothing of its own yet");
        assert_eq!(s.histories().len(), 2);
        assert_eq!(s.history_name(&h), Some("quieter"));
    }
    let now = net.tick();
    let in_h = net.song(1).mixer(&bob, h, &drums(), MixerChange::Volume(0.1), now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.state(&h).unwrap().track(&drums()).unwrap().mixer.volume, 0.1);
        assert_eq!(s.main().track(&drums()).unwrap().mixer.volume, 0.9, "main untouched");
        let rows = s.history(&h);
        let own: Vec<&Row> = rows.iter().filter(|r| !r.inherited).collect::<Vec<_>>();
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].act, in_h.id);
    }
    assert_same_song(&net, &h);
    // Go back on main, then undo the Go back
    let now = net.tick();
    let gb = net.song(0).go_back(&alice, main(), mid.id, now).unwrap();
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.main().track(&drums()).unwrap().mixer.volume == 0.5));
    let now = net.tick();
    net.song(0).undo(&alice, main(), gb.id, now).unwrap();
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.main().track(&drums()).unwrap().mixer.volume == 0.9));
    assert!(net.songs[1].history(&main()).iter().any(|r| r.act == gb.id && r.undone));
    assert_same_song(&net, &main());
    // a history cannot start from a moment outside this history's past
    let now = net.tick();
    assert!(net.song(0).new_history(&alice, "x", Hash::of(b"nope"), now).is_err());
}

#[test]
fn merge_whole_history_and_chosen_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    net.song(0).mixer(&alice, main(), &bass(), MixerChange::Volume(0.5), now).unwrap();
    net.deliver_all();
    let now = net.tick();
    let from = net.songs[1].latest(&main()).unwrap();
    let h = net.song(1).new_history(&bob, "idea", from, now).unwrap();
    net.deliver_all();
    let now = net.tick();
    let hk = net.song(1).setting(&bob, h, &drums(), DeviceSlot(1), &comp(), &p7(), 0.8, None, now).unwrap();
    let hb = net.song(1).mixer(&bob, h, &bass(), MixerChange::Volume(0.9), now).unwrap();
    // meanwhile on main: bass pan (no conflict) and bass volume (conflict)
    let mp = net.song(0).mixer(&alice, main(), &bass(), MixerChange::Pan(0.4), now).unwrap();
    let mv = net.song(0).mixer(&alice, main(), &bass(), MixerChange::Volume(0.2), now).unwrap();
    net.deliver_all();

    let preview = net.songs[0].merge_preview(main(), &net.songs[0], h, &[], None).unwrap();
    assert_eq!(preview.acts.iter().map(|a| a.id).collect::<Vec<_>>(), vec![hk.id, hb.id]);
    assert_eq!(preview.conflicts.len(), 1);
    let c = &preview.conflicts[0];
    assert_eq!((c.thing.clone(), c.property.as_str(), c.from_act, c.to_act, c.keep_from), (Thing::Track(bass()), "volume", hb.id, mv.id, false));
    assert_eq!(preview.counts[&drums()], 1);
    assert_eq!(preview.counts[&bass()], 1);
    assert!(preview.acts.iter().all(|a| a.id != mp.id));

    // default: TO keeps its volume; the knob comes over
    let now = net.tick();
    let merged = net.song(0).merge(&alice, &preview, now).unwrap();
    net.deliver_all();
    for s in &net.songs {
        let st = s.main();
        assert_eq!(st.track(&drums()).unwrap().setting(DeviceSlot(1), &p7()), Some(0.8));
        assert_eq!(st.track(&bass()).unwrap().mixer.volume, 0.2);
        assert_eq!(st.track(&bass()).unwrap().mixer.pan, 0.4);
        let row = s.history(&main()).into_iter().find(|r| r.act == merged.id).unwrap();
        assert_eq!(row.group_size, 1, "one act carried after the conflict was decided for TO");
        assert_eq!(st.track(&drums()).unwrap().setting(DeviceSlot(1), &p7()).map(|_| st.track(&drums()).unwrap().device(DeviceSlot(1)).unwrap().settings[&p7()].act), Some(hk.id), "credit stays with Bob's act");
    }
    // undo the merge as one
    let now = net.tick();
    let u = net.song(0).undo(&alice, main(), merged.id, now).unwrap();
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.main().track(&drums()).is_none()));
    let now = net.tick();
    net.song(0).undo(&alice, main(), u.id, now).unwrap();
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.main().track(&drums()).unwrap().setting(DeviceSlot(1), &p7()) == Some(0.8)));

    // flip the conflict and merge only bass
    let mut preview = net.songs[0].merge_preview(main(), &net.songs[0], h, &[bass()], None).unwrap();
    assert_eq!(preview.acts.len(), 1);
    preview.keep_all(true);
    let now = net.tick();
    let m2 = net.song(0).merge(&alice, &preview, now).unwrap();
    assert_eq!(m2.thing(), &Thing::Track(bass()));
    net.deliver_all();
    for s in &net.songs {
        assert_eq!(s.main().track(&bass()).unwrap().mixer.volume, 0.9);
        assert_eq!(s.history(&main()).into_iter().find(|r| r.act == m2.id).unwrap().group_size, 1);
    }
    assert_same_song(&net, &main());
    // nothing left to merge
    let again = net.songs[0].merge_preview(main(), &net.songs[0], h, &[], None).unwrap();
    assert!(again.acts.is_empty());
}

#[test]
fn pull_from_another_song_brings_facts_not_acts_and_keeps_credit() {
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let bob = stamper(2, "bob");
    let mut a = new_song(&dir.path().join("a").join(".flit"), &alice, "Alice", 1);
    let mut b = new_song(&dir.path().join("b").join(".flit"), &bob, "Bob", 1);
    let k1 = b.setting(&bob, main(), &drums(), DeviceSlot(1), &comp(), &p7(), 0.6, None, 2).unwrap();
    b.mixer(&bob, main(), &drums(), MixerChange::Volume(0.7), 3).unwrap();
    b.mixer(&bob, main(), &bass(), MixerChange::Volume(0.1), 3).unwrap();
    let preview = a.merge_preview(main(), &b, main(), &[drums()], None).unwrap();
    assert_eq!(preview.from_song, Some(b.id()));
    assert_eq!(preview.acts.len(), 2);
    assert!(preview.conflicts.is_empty());
    let m = a.merge(&alice, &preview, 4).unwrap();
    let st = a.main();
    assert_eq!(st.track(&drums()).unwrap().setting(DeviceSlot(1), &p7()), Some(0.6));
    assert_eq!(st.track(&drums()).unwrap().mixer.volume, 0.7);
    assert!(st.track(&bass()).is_none(), "only the chosen track");
    assert_eq!(st.track(&drums()).unwrap().device(DeviceSlot(1)).unwrap().settings[&p7()].act, k1.id);
    assert!(a.chains().get(&k1.id).is_none(), "Bob's act chain did not travel");
    let Payload::Merge(mp) = m.payload() else { panic!() };
    assert!(mp.acts.iter().all(|x| x.name() == bob.name && x.body.being.0 == "bob"), "credit kept on every imported fact");
    assert!(!a.crew().is_member(&bob.name), "Bob is not in Alice's crew; his work is");
    // reopen folds it the same
    drop(a);
    let a2 = Song::open(dir.path().join("a").join(".flit")).unwrap();
    assert_eq!(a2.main().track(&drums()).unwrap().mixer.volume, 0.7);
}

#[test]
fn chat_jump_and_version_restore_plan() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    let proj = dir.path().join("p.flp");
    std::fs::write(&proj, b"flp v1").unwrap();
    let v = net.song(0).save_version_from_file(&alice, main(), "v1", &proj, &[], now).unwrap();
    let k = net.song(0).setting(&alice, main(), &drums(), DeviceSlot(1), &comp(), &p7(), 0.4, Some(SongPos::from_beats(16.0)), now).unwrap();
    net.deliver_all();
    let now = net.tick();
    let c = net.song(1).chat(&bob, main(), "try this from here", Some(k.id), now).unwrap();
    net.deliver_all();
    let song = &net.songs[2];
    assert_eq!(song.jump_target(&c.id), Some((main(), k.id, Some(SongPos::from_beats(16.0)))));
    assert_eq!(song.all_chat().len(), 1);
    let plan = song.restore_plan(&k.id, &host()).unwrap();
    assert_eq!(plan.version.as_ref().map(|a| a.id), Some(v.id));
    assert_eq!(plan.replay.iter().map(|a| a.id).collect::<Vec<_>>(), vec![k.id]);
    assert_eq!(plan.state.track(&drums()).unwrap().setting(DeviceSlot(1), &p7()), Some(0.4));
    assert!(song.missing_for_version(&v.id).len() == 1, "the project blob has not been fetched by this peer yet");
    assert!(net.songs[0].missing_for_version(&v.id).is_empty());
}
