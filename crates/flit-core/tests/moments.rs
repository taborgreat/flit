//! Storage and the dual seal: write, reopen, verify every frame, fold to
//! the same state; torn tails heal; the sidecar is derived; tampering is
//! caught; folding to a moment equals a full replay.
mod common;
use common::*;
use flit_core::*;

#[test]
fn write_read_verify_replay() {
    let dir = tempfile::tempdir().unwrap();
    let flit = dir.path().join(".flit");
    let alice = stamper(1, "alice");
    let drums = track("drums");
    let mut now = 10u64;
    let mut song = new_song(&flit, &alice, "Alice", now);
    for i in 0..10 {
        now += 1;
        song.setting(&alice, main(), &drums, DeviceSlot(1), &comp(), &p7(), i as f32 / 10.0, None, now).unwrap();
    }
    song.tempo(&alice, main(), 128.0, now).unwrap();
    song.mixer(&alice, main(), &drums, MixerChange::Volume(0.8), now).unwrap();
    song.host_state(&alice, main(), &drums, vec![1, 2, 3, 4], now).unwrap();
    song.notes(&alice, main(), &drums, "pat1", vec![note(0, 60), note(1, 62)], vec![], now).unwrap();
    let proj = dir.path().join("song.proj");
    std::fs::write(&proj, b"project bytes v1").unwrap();
    let sample = dir.path().join("kick.wav");
    std::fs::write(&sample, b"RIFF....").unwrap();
    let v = song.save_version_from_file(&alice, main(), "v1", &proj, &[sample.clone()], now).unwrap();

    let before = song.main().clone();
    let rows_before = tape(&song, &main());
    let n = song.chains().len();
    assert_eq!(n, 3 + 10 + 4 + 1, "genesis, keeper, being, settings, tempo/mixer/hoststate/notes, version");
    assert_eq!(song.unsealed_count(), 0, "the owner's machine is the keeper: everything sealed at once");
    drop(song);

    // on disk: one spine for Alice, the keeper spine, the format file
    assert_eq!(std::fs::read_to_string(flit.join("format")).unwrap().trim(), "1");
    let spine = flit.join("spines").join(format!("{}.tree", alice.name.to_hex()));
    assert_eq!(std::fs::read_to_string(&spine).unwrap().lines().count(), n, "every act Alice made");
    let keeper = flit.join("spines").join("keeper.tree");
    assert_eq!(std::fs::read_to_string(&keeper).unwrap().lines().count(), n, "every fact the keeper sealed");

    // read back: same fold, every act signed by its Name, every fact by the keeper
    let song2 = Song::open(&flit).unwrap();
    assert_eq!(song2.chains().len(), n);
    for a in song2.chains().all_acts() {
        a.check_id().unwrap();
        assert!(a.signed_by(&alice.name));
        let f = song2.chains().fact_for(&a.id).expect("sealed");
        f.verify().unwrap();
        assert_eq!(f.body.keeper, alice.name);
    }
    assert_eq!(&before, song2.main());
    assert_eq!(rows_before, tape(&song2, &main()));
    let d = song2.main().track(&drums).unwrap();
    assert_eq!(d.setting(DeviceSlot(1), &p7()), Some(0.9));
    assert_eq!(d.plugin(DeviceSlot(1)), Some(&comp()));
    assert_eq!(d.mixer.volume, 0.8);
    assert!(d.mixer_set.volume && !d.mixer_set.pan);
    assert_eq!(song2.main().settings.tempo, 128.0);
    assert_eq!(d.host_state.get(&host()).unwrap(), &vec![1, 2, 3, 4]);
    assert_eq!(d.clips["pat1"].len(), 2);
    assert_eq!(song2.main().versions, vec![v.id]);
    assert!(song2.missing_for_version(&v.id).is_empty());
    assert_eq!(song2.keeper(), Some((alice.name, "machine-1".into())));

    // the verify tier on a copied spine
    let acts = flit_core::seal::spine::Spines::verify_act_spine(&spine, &alice.name).unwrap();
    assert_eq!(acts.len(), n);
}

#[test]
fn torn_tail_heals_and_appends_continue() {
    let dir = tempfile::tempdir().unwrap();
    let flit = dir.path().join(".flit");
    let alice = stamper(1, "alice");
    let mut song = new_song(&flit, &alice, "Alice", 1);
    song.tempo(&alice, main(), 100.0, 2).unwrap();
    let before = song.main().clone();
    let n = song.chains().len();
    drop(song);
    let spine = flit.join("spines").join(format!("{}.tree", alice.name.to_hex()));
    let keeper = flit.join("spines").join("keeper.tree");
    use std::io::Write;
    for p in [&spine, &keeper] {
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        f.write_all(b"{\"h\":99,\"p\":\"zz\",\"act\":{\"id\":\"brok").unwrap();
    }
    let mut song = Song::open(&flit).unwrap();
    assert_eq!(song.chains().len(), n, "torn line skipped");
    assert_eq!(&before, song.main());
    song.tempo(&alice, main(), 101.0, 3).unwrap();
    drop(song);
    let song = Song::open(&flit).unwrap();
    assert_eq!(song.chains().len(), n + 1, "append after a torn tail lands cleanly");
    assert_eq!(song.main().settings.tempo, 101.0);
    assert_eq!(std::fs::read_to_string(&spine).unwrap().lines().count(), n + 1);
}

#[test]
fn sidecar_is_derived() {
    let dir = tempfile::tempdir().unwrap();
    let flit = dir.path().join(".flit");
    let alice = stamper(1, "alice");
    let mut song = new_song(&flit, &alice, "Alice", 1);
    for i in 0..5 {
        song.setting(&alice, main(), &track("bass"), DeviceSlot(0), &comp(), &p7(), i as f32 / 5.0, None, 2 + i).unwrap();
    }
    let before = song.main().clone();
    let rows = tape(&song, &main());
    assert!(song.history(&main()).iter().all(|r| r.received_at.is_some()));
    drop(song);
    std::fs::remove_dir_all(flit.join("sidecar")).unwrap();
    let song = Song::open(&flit).unwrap();
    assert_eq!(&before, song.main());
    assert_eq!(rows, tape(&song, &main()));
    assert!(song.history(&main()).iter().all(|r| r.received_at.is_none()), "witness notes gone, nothing else changed");
}

#[test]
fn tampered_frame_is_caught() {
    let dir = tempfile::tempdir().unwrap();
    let flit = dir.path().join(".flit");
    let alice = stamper(1, "alice");
    let mut song = new_song(&flit, &alice, "Alice", 1);
    song.tempo(&alice, main(), 100.0, 2).unwrap();
    song.tempo(&alice, main(), 110.0, 3).unwrap();
    drop(song);
    let spine = flit.join("spines").join(format!("{}.tree", alice.name.to_hex()));
    let text = std::fs::read_to_string(&spine).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = lines.clone();
    let changed = lines[3].replace("100.0", "140.0");
    assert_ne!(changed, lines[3]);
    out[3] = &changed;
    std::fs::write(&spine, out.join("\n") + "\n").unwrap();
    assert!(Song::open(&flit).is_err(), "a changed frame breaks the link or the act id");
    assert!(flit_core::seal::spine::Spines::verify_act_spine(&spine, &alice.name).is_err());
}

#[test]
fn dual_seal_one_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut net, alice, bob, _carol) = crew_of_three(dir.path());
    let now = net.tick();
    let act = net.song(1).mixer(&bob, main(), &track("bass"), MixerChange::Pan(-0.2), now).unwrap();
    assert!(act.signed_by(&bob.name), "the being's Name signs the act");
    assert_eq!(net.song(1).unsealed_count(), 1, "Bob's machine is not the keeper");
    net.deliver_all();
    for s in &net.songs {
        let f = s.chains().fact_for(&act.id).expect("sealed everywhere");
        f.verify().unwrap();
        assert_eq!(f.body.keeper, alice.name, "the keeper countersigns");
        assert!(!bob.name.verify(f.id.as_bytes(), &f.keeper_sig));
    }
    // every fact in every copy has exactly one writer
    for s in &net.songs {
        for f in s.chains().all_facts() {
            assert_eq!(f.body.keeper, alice.name);
            f.verify().unwrap();
        }
    }
    // Bob cannot seal his own act: Carol's copy refuses a fact he signed
    let now = net.tick();
    let act2 = net.song(1).mixer(&bob, main(), &track("bass"), MixerChange::Pan(0.3), now).unwrap();
    net.offline[0] = true;
    net.deliver_all();
    assert!(net.songs[2].chains().contains(&act2.id) && !net.songs[2].chains().is_sealed(&act2.id));
    let prev = net.songs[2].chains().fact_chain(act2.thing(), &main()).last().map(|f| f.id);
    let forged = Fact::seal(
        FactBody { thing: act2.thing().clone(), history: main(), fact_seq: 2, fact_prev: prev, seal_seq: net.songs[2].chains().seal_seq(&main()) + 1, act_id: act2.id, keeper: bob.name, machine: "machine-2".into(), gate: false },
        &bob.key,
    );
    assert!(matches!(net.song(2).import_fact(forged.clone()), Err(Error::WrongKeeper(_))));
    net.offline[0] = false;
    net.deliver_all();
    assert!(net.songs.iter().all(|s| s.chains().fact_for(&act2.id).map(|f| f.body.keeper == alice.name).unwrap_or(false)));
}

#[test]
fn fold_to_moment_equals_full_replay() {
    let dir = tempfile::tempdir().unwrap();
    let flit = dir.path().join(".flit");
    let alice = stamper(1, "alice");
    let mut song = new_song(&flit, &alice, "Alice", 1);
    let tracks = [track("drums"), track("bass"), track("keys")];
    let mut ids = Vec::new();
    for i in 0..1200u64 {
        let t = &tracks[(i % 3) as usize];
        let a = match i % 5 {
            0 => song.mixer(&alice, main(), t, MixerChange::Volume((i % 10) as f32 / 10.0), 10 + i).unwrap(),
            1 => song.tempo(&alice, main(), 100.0 + (i % 50) as f64, 10 + i).unwrap(),
            2 => song.device(&alice, main(), t, DeviceSlot((i % 3) as u32), Some(comp()), 10 + i).unwrap(),
            _ => song.setting(&alice, main(), t, DeviceSlot(1), &comp(), &ParamId::from_u32((i % 4) as u32), (i % 7) as f32 / 7.0, None, 10 + i).unwrap(),
        };
        ids.push(a.id);
    }
    assert!(song.chains().seal_seq(&main()) > CHECKPOINT_EVERY as u64 * 2, "checkpoints are in play");
    let moments: Vec<(Act, Fact)> = song.chains().moments_after(&main(), 0).into_iter().map(|(f, a)| (a.clone(), f.clone())).collect();
    for k in [0usize, 1, 7, 499, 500, 501, 999, 1000, 1150, moments.len() - 1] {
        let mut fresh = Chains::in_memory();
        for (a, f) in moments.iter().take(k + 1) {
            assert_eq!(fresh.append_moment(a.clone(), f.clone(), None).unwrap(), AppendOutcome::Appended);
        }
        let at = &moments[k].0.id;
        let folded = song.state_at(at).unwrap();
        let replayed = fresh.state(&main()).unwrap();
        assert_eq!(&folded, replayed, "fold to moment {k} equals replay of its prefix");
    }
    drop(song);
    let song2 = Song::open(&flit).unwrap();
    assert_eq!(song2.main().applied, moments.len());
}
