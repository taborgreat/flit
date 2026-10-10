//! L2: FakeHost sweeps produce exactly the expected events, at level 1 and
//! level 0. Receiver decisions for a mixed crew. Save renders only changed
//! lanes.
use flit_bridge_fake::FakeHost;
use flit_core::*;
use flit_host::*;

fn comp() -> PluginIdentity {
    PluginIdentity::vst3("Acme", "Comp", "1.2.0")
}
fn lane(s: &str) -> LaneId {
    LaneId::new(s)
}
fn host() -> HostTag {
    HostTag::new("fake")
}
fn kinds(evs: &[Stamp]) -> Vec<Kind> {
    evs.iter().map(|e| e.kind()).collect()
}

fn setup(level: Level) -> (tempfile::TempDir, Song, LiveLoop<FakeHost, PcmChunkEncoder>) {
    let dir = tempfile::tempdir().unwrap();
    let key = Keypair::from_seed(1);
    let song = Song::create(dir.path().join(".flit"), &key, "S", &host(), Time::at(0, SongPos::START)).unwrap();
    let mut fake = FakeHost::new(level);
    fake.add_track("drums", "Drums", LaneKind::Part, Some(comp()), 110.0);
    fake.add_track("bass", "Bass", LaneKind::Part, Some(comp()), 55.0);
    let live = LiveLoop::new(fake, PcmChunkEncoder, key, stamp::main_branch());
    (dir, song, live)
}

#[test]
fn level1_thousand_twists_one_event() {
    let (_d, mut song, mut live) = setup(Level::LiveControl);
    let mut t = 0;
    // tracks appear -> lanes claimed
    let evs = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Lane, Kind::Lane]);
    assert_eq!(song.main().owner(&lane("drums")), Some(live.id()));

    // 1000 twists over 10 seconds, ticking the loop every 10 ms: nothing
    // settles while the knob is moving
    let mut emitted = Vec::new();
    for i in 0..1000 {
        live.bridge.twist("drums", DeviceSlot(0), "thresh", (i % 100) as f32 / 100.0);
        t += 10;
        emitted.extend(live.tick(&mut song, t).unwrap());
    }
    assert!(emitted.is_empty(), "nothing while still moving");
    t += 150;
    emitted.extend(live.tick(&mut song, t).unwrap());
    assert_eq!(kinds(&emitted), vec![Kind::Setting]);
    let Payload::Setting(s) = emitted[0].payload() else { panic!() };
    assert_eq!(s.param, ParamId::new("thresh"));
    assert!((s.value - 0.99).abs() < 1e-6);
    assert_eq!(s.plugin, comp());
    assert_eq!(emitted[0].body.host, host());
    assert_eq!(song.main().lane(&lane("drums")).unwrap().setting(DeviceSlot(0), &"thresh".into()), Some(0.99));

    // knob returned to where it started = 0 events
    live.bridge.sweep("drums", DeviceSlot(0), "thresh", 0.99, 0.2, 50);
    live.bridge.sweep("drums", DeviceSlot(0), "thresh", 0.2, 0.99, 50);
    t += 200;
    assert!(live.tick(&mut song, t).unwrap().is_empty());

    // volume + tempo settle too; play/stop are instant
    live.bridge.set_volume("bass", 0.7);
    live.bridge.set_tempo(128.0);
    live.bridge.play();
    let evs = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Transport]); // Play, instantly
    t += 200;
    let evs = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Mixer, Kind::Transport]);
    assert_eq!(song.main().transport.tempo, 128.0);
    assert_eq!(song.main().lane(&lane("bass")).unwrap().mixer.volume, 0.7);
    assert_eq!(live.ignored_foreign, 0);
}

#[test]
fn level0_is_silent_but_saves_capture_audio() {
    let (_d, mut song, mut live) = setup(Level::PluginOnly);
    assert_eq!(live.bridge.capabilities().level, Level::PluginOnly);
    let mut t = 0;
    assert!(live.tick(&mut song, t).unwrap().is_empty(), "no bridge, no lane reports");
    for i in 0..1000 {
        live.bridge.twist("drums", DeviceSlot(0), "thresh", (i % 100) as f32 / 100.0);
        live.bridge.set_volume("bass", 0.3);
        live.bridge.set_tempo(90.0);
        t += 10;
        assert!(live.tick(&mut song, t).unwrap().is_empty());
    }
    t += 1000;
    assert!(live.tick(&mut song, t).unwrap().is_empty(), "level 0 never reports knobs");
    assert_eq!(song.chains().len(), 2, "genesis + mix lane only");
    // ... but tempo and transport always cross: the plugin reads them from
    // its host's transport info and reports them itself.
    live.report_transport(&mut song, TransportChange::Tempo(90.0), SongPos::START, t).unwrap();
    live.report_transport(&mut song, TransportChange::Play, SongPos::START, t).unwrap();
    t += 200;
    let evs = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Transport]);
    assert_eq!(song.main().transport.tempo, 90.0);
    assert!(song.main().transport.playing);

    // The plugin on the drums lane captured its output at the print point
    let cap = live.bridge.capture("drums", SongPos::START, SongPos::from_beats(8.0)).unwrap();
    assert_eq!(cap.print_point, PrintPoint::PostFaderPreSend);
    live.push_captured(lane("drums"), cap);
    let state_path = _d.path().join("plugin-state.json");
    std::fs::write(&state_path, b"{}").unwrap();
    let evs = live.save_version(&mut song, "Save 1", &state_path, t, SongPos::from_beats(8.0)).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Version, Kind::Lane, Kind::Audio]);
    let Payload::Audio(a) = evs[2].payload() else { panic!() };
    // 8 beats = 2 bars = 2 chunks (at the song's tempo, 90)
    assert_eq!(a.opus_chunks.len(), 2);
    assert_eq!(a.flac_chunks.len(), 2);
    assert_eq!(a.print_point, PrintPoint::PostFaderPreSend);
    assert!(a.opus_chunks.iter().all(|h| song.blobs().has(h)));
    assert_eq!(song.main().owner(&lane("drums")), Some(live.id()));
    assert_eq!(evs[1].body.host, host());
}

#[test]
fn save_renders_only_changed_lanes() {
    let (d, mut song, mut live) = setup(Level::Project);
    let mut t = 0;
    live.tick(&mut song, t).unwrap();
    live.bridge.twist("drums", DeviceSlot(0), "thresh", 0.4);
    live.bridge.write_notes("bass", "riff", vec![Note { start: SongPos::START, length_ticks: 480, key: 36, velocity: 100, channel: 0 }]);
    live.tick(&mut song, t).unwrap(); // ingest the move
    t += 200;
    live.tick(&mut song, t).unwrap(); // settle
    assert_eq!(live.dirty_lanes(), vec![lane("drums")]);
    let proj = d.path().join("song.fakeproj");
    live.bridge.seek(SongPos::from_beats(4.0));
    live.bridge.save_project(&proj).unwrap();
    let evs = live.tick(&mut song, t).unwrap();
    // notes from the project (bass changed via the file), version, audio for both changed lanes
    assert_eq!(kinds(&evs), vec![Kind::Notes, Kind::Version, Kind::Audio, Kind::Audio]);
    assert_eq!(live.bridge.renders, 2);
    assert!(live.dirty_lanes().is_empty());
    let Payload::Version(v) = evs[1].payload() else { panic!() };
    assert_eq!(v.name, "Save");
    assert_eq!(song.blobs().get(&v.project).unwrap(), std::fs::read(&proj).unwrap());
    // nothing changed -> second save renders nothing
    live.bridge.save_project(&proj).unwrap();
    let evs = live.tick(&mut song, t + 10).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Version]);
    assert_eq!(live.bridge.renders, 2);
    // restore plan uses this version
    let plan = song.restore_plan(&stamp::main_branch(), &evs[0].id, &host()).unwrap();
    assert_eq!(plan.version.as_ref().map(|e| e.id), Some(evs[0].id));
    assert!(plan.replay.is_empty());
}

#[test]
fn foreign_lane_changes_are_dropped() {
    let (_d, mut song, mut live) = setup(Level::LiveControl);
    let other = Keypair::from_seed(2);
    // the owner (seed 1 == live.id()) approves other and gives them bass
    let key = Keypair::from_seed(1);
    song.approve(&key, other.id(), false, &host(), Time::at(0, SongPos::START)).unwrap();
    song.assign_lane(&key, &stamp::main_branch(), &lane("bass"), other.id(), LaneKind::Part, None, &host(), Time::at(0, SongPos::START)).unwrap();
    live.tick(&mut song, 0).unwrap();
    live.bridge.twist("bass", DeviceSlot(0), "x", 0.9);
    live.bridge.twist("drums", DeviceSlot(0), "x", 0.9);
    live.tick(&mut song, 10).unwrap();
    let evs = live.tick(&mut song, 500).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Setting]);
    assert_eq!(evs[0].lane(), Some(&lane("drums")));
    assert_eq!(live.ignored_foreign, 1);
}

/// Dropping a plugin into a slot or removing it is one stamp at once (no
/// settle); a peer with a bridge that can place plugins applies it, a
/// level-0 peer hears the lane; Go back / preview re-place it.
#[test]
fn plugin_add_remove_is_a_stamp_and_follows_history() {
    let dir = tempfile::tempdir().unwrap();
    let alice = Keypair::from_seed(1);
    let bob = Keypair::from_seed(2);
    let main = stamp::main_branch();
    let mut sa = Song::create(dir.path().join("a"), &alice, "S", &host(), Time::at(0, SongPos::START)).unwrap();
    sa.approve(&alice, bob.id(), false, &host(), Time::at(1, SongPos::START)).unwrap();
    let mut fa = FakeHost::new(Level::LiveControl);
    fa.add_track("drums", "Drums", LaneKind::Part, Some(comp()), 110.0);
    let mut la = LiveLoop::new(fa, PcmChunkEncoder, alice.clone(), main);
    la.tick(&mut sa, 0).unwrap();
    let eq = PluginIdentity::vst3("Acme", "EQ", "2.0");
    la.bridge.add_device("drums", DeviceSlot(1), eq.clone());
    let evs = la.tick(&mut sa, 10).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Device], "at once, not settled");
    assert_eq!(sa.main().lane(&lane("drums")).unwrap().devices.get(&DeviceSlot(1)), Some(&eq));
    la.bridge.remove_device("drums", DeviceSlot(1));
    let evs2 = la.tick(&mut sa, 20).unwrap();
    assert_eq!(kinds(&evs2), vec![Kind::Device]);
    assert!(sa.main().lane(&lane("drums")).unwrap().devices.get(&DeviceSlot(1)).is_none());

    // Bob's copy: a level-1 FakeHost with the EQ installed applies the add
    let mut sb = Song::join(dir.path().join("b"), sa.genesis().clone(), "S").unwrap();
    let mut fb = FakeHost::new(Level::LiveControl);
    fb.install(eq.clone());
    for e in sa.chains().causal() {
        sb.import(e.clone()).unwrap();
    }
    assert_eq!(Receiver::apply(&mut fb, &sb, &evs[0].id), Some(ReceiverDecision::ApplyDevice));
    assert_eq!(fb.device("drums", DeviceSlot(1)), Some(&eq));
    assert_eq!(Receiver::apply(&mut fb, &sb, &evs2[0].id), Some(ReceiverDecision::ApplyDevice));
    assert_eq!(fb.device("drums", DeviceSlot(1)), None);
    // a level-0 host hears the lane instead
    let mut f0 = FakeHost::new(Level::PluginOnly);
    assert_eq!(Receiver::apply(&mut f0, &sb, &evs[0].id), Some(ReceiverDecision::HearAsAudio(lane("drums"))));

    // Go back to when the EQ was there: apply_state puts it back on Bob's host
    let st = sb.chains().state_at(&evs[0].id).unwrap();
    let applied = Receiver::apply_state(&mut fb, &st);
    assert_eq!(applied.devices, 1);
    assert!(applied.manual.is_empty());
    assert_eq!(fb.device("drums", DeviceSlot(1)), Some(&eq));
    // a host that cannot place plugins reports what to do by hand
    let mut fc = FakeHost::new(Level::LiveControl);
    let by_hand = Receiver::apply_state(&mut fc, &st);
    assert_eq!(by_hand.devices, 0);
    assert_eq!(by_hand.manual, vec!["drums: put EQ in slot 2".to_string()]);
}

#[test]
fn receiver_mixed_crew_decisions() {
    // Alice on a level-1 host with the Comp plugin, Bob at level 0, Carol on
    // a level-1 host WITHOUT the plugin.
    let dir = tempfile::tempdir().unwrap();
    let alice = Keypair::from_seed(1);
    let mut song = Song::create(dir.path(), &alice, "S", &host(), Time::at(0, SongPos::START)).unwrap();
    let main = stamp::main_branch();
    song.claim_lane(&alice, &main, &lane("drums"), LaneKind::Part, None, &host(), Time::at(0, SongPos::START)).unwrap();
    let setting = song.setting(&alice, &main, &lane("drums"), DeviceSlot(0), &comp(), &"x".into(), 0.5, &host(), Time::at(1, SongPos::START)).unwrap();
    let mixer = song.mixer(&alice, &main, &lane("drums"), MixerChange::Pan(-0.5), &host(), Time::at(2, SongPos::START)).unwrap();
    let tempo = song.tempo(&alice, &main, 100.0, &host(), Time::at(3, SongPos::START)).unwrap();
    let hs = song.host_state(&alice, &main, &lane("drums"), &host(), vec![7, 7], Time::at(4, SongPos::START)).unwrap();
    let chat = song.chat(&alice, &main, "hi", None, &host(), Time::at(5, SongPos::START)).unwrap();

    let mut with_plugin = FakeHost::new(Level::LiveControl);
    with_plugin.install(comp());
    let mut level0 = FakeHost::new(Level::PluginOnly);
    let mut without = FakeHost::new(Level::LiveControl).with_tag("otherdaw");

    assert_eq!(Receiver::apply(&mut with_plugin, &song, &setting.id), Some(ReceiverDecision::ApplySetting));
    assert_eq!(with_plugin.param("drums", DeviceSlot(0), "x"), Some(0.5));
    assert_eq!(Receiver::apply(&mut level0, &song, &setting.id), Some(ReceiverDecision::HearAsAudio(lane("drums"))));
    assert_eq!(Receiver::apply(&mut without, &song, &setting.id), Some(ReceiverDecision::HearAsAudio(lane("drums"))));
    assert!(without.applied.is_empty());

    assert_eq!(Receiver::apply(&mut with_plugin, &song, &mixer.id), Some(ReceiverDecision::ApplyMixer));
    assert_eq!(Receiver::apply(&mut level0, &song, &mixer.id), Some(ReceiverDecision::HearAsAudio(lane("drums"))));
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &tempo.id), Some(ReceiverDecision::ApplyTransport));
    assert_eq!(with_plugin.tempo, 100.0);
    assert_eq!(Receiver::apply(&mut level0, &song, &tempo.id), Some(ReceiverDecision::Nothing));
    // HostState: same host applies, other host hears audio
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &hs.id), Some(ReceiverDecision::ApplyHostState));
    assert_eq!(with_plugin.applied_host_state.len(), 1);
    assert_eq!(Receiver::apply(&mut without, &song, &hs.id), Some(ReceiverDecision::HearAsAudio(lane("drums"))));
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &chat.id), Some(ReceiverDecision::Nothing));
    // an event that is not in the log is never applied
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &Hash::of(b"nope")), None);
}

#[test]
fn two_fakehosts_level1_and_level0_share_a_song() {
    // Alice (level 1) and Bob (level 0) in one song, same process, no
    // network: Bob's copy receives Alice's events; Alice's receives Bob's
    // captured audio on save.
    let dir = tempfile::tempdir().unwrap();
    let alice = Keypair::from_seed(1);
    let bob = Keypair::from_seed(2);
    let main = stamp::main_branch();
    let mut sa = Song::create(dir.path().join("a"), &alice, "S", &host(), Time::at(0, SongPos::START)).unwrap();
    sa.approve(&alice, bob.id(), false, &host(), Time::at(1, SongPos::START)).unwrap();
    let mut sb = Song::join(dir.path().join("b"), sa.genesis().clone(), "S").unwrap();
    for e in sa.chains().causal() {
        sb.import(e.clone()).unwrap();
    }
    let mut fa = FakeHost::new(Level::LiveControl);
    fa.add_track("drums", "Drums", LaneKind::Part, Some(comp()), 110.0);
    let mut la = LiveLoop::new(fa, PcmChunkEncoder, alice.clone(), main);
    let fb = FakeHost::new(Level::PluginOnly);
    let mut lb = LiveLoop::new(fb, PcmChunkEncoder, bob.clone(), main);
    lb.bridge.add_track("vox", "Vox", LaneKind::Part, None, 220.0);

    la.tick(&mut sa, 0).unwrap();
    la.bridge.sweep("drums", DeviceSlot(0), "x", 0.0, 0.8, 100);
    la.tick(&mut sa, 10).unwrap();
    let evs = la.tick(&mut sa, 500).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Setting]);
    // "network": copy events across
    for e in sa.chains().causal() {
        sb.import(e.clone()).unwrap();
    }
    assert_eq!(sa.main(), sb.main());
    // Bob at level 0 hears drums as audio
    assert_eq!(Receiver::apply(&mut lb.bridge, &sb, &evs[0].id), Some(ReceiverDecision::HearAsAudio(lane("drums"))));
    // Bob saves: his captured vox lands as audio on his own lane
    let cap = lb.bridge.capture("vox", SongPos::START, SongPos::from_beats(4.0)).unwrap();
    lb.push_captured(lane("vox"), cap);
    let p = dir.path().join("b-state.json");
    std::fs::write(&p, b"{}").unwrap();
    let evs = lb.save_version(&mut sb, "Bob save", &p, 1000, SongPos::from_beats(4.0)).unwrap();
    assert_eq!(kinds(&evs), vec![Kind::Version, Kind::Lane, Kind::Audio]);
    for e in sb.chains().causal() {
        sa.import(e.clone()).unwrap();
    }
    assert_eq!(sa.main(), sb.main());
    assert_eq!(sa.main().owner(&lane("vox")), Some(bob.id()));
    // Alice needs Bob's chunks: they are listed as missing until fetched
    let missing = sa.missing_opus(&main);
    assert_eq!(missing.len(), 1);
    for h in &missing {
        sa.blobs().put_verified(h, &sb.blobs().get(h).unwrap()).unwrap();
    }
    assert!(sa.missing_opus(&main).is_empty());
}
