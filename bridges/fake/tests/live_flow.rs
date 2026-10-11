//! FakeHost sweeps produce exactly the expected acts, at level 1 and
//! level 0. Receiver decisions for a mixed crew. Save renders only changed
//! tracks. Presence settles. Applying a moment is a diff (§20).
use flit_bridge_fake::FakeHost;
use flit_core::*;
use flit_host::*;

fn comp() -> PluginIdentity {
    PluginIdentity::vst3("Acme", "Comp", "1.2.0")
}
fn track(s: &str) -> TrackId {
    TrackId::new(s)
}
fn host() -> HostTag {
    HostTag::new("fake")
}
fn kinds(acts: &[Act]) -> Vec<Kind> {
    acts.iter().map(|e| e.kind()).collect()
}
fn stamper(seed: u64, being: &str) -> Stamper {
    Stamper::new(Keypair::from_seed(seed), being, host(), "0.1.0", &format!("m{seed}"))
}

fn setup(level: Level) -> (tempfile::TempDir, Song, LiveLoop<FakeHost, PcmChunkEncoder>) {
    let dir = tempfile::tempdir().unwrap();
    let who = stamper(1, "alice");
    let song = Song::create(dir.path().join(".flit"), &who, "S", 0).unwrap();
    let mut fake = FakeHost::new(level);
    fake.add_track("drums", "Drums", TrackKind::Part, Some(comp()), 110.0);
    fake.add_track("bass", "Bass", TrackKind::Part, Some(comp()), 55.0);
    let live = LiveLoop::new(fake, PcmChunkEncoder, who, main_history());
    (dir, song, live)
}

/// Carry everything one song stamped into another (same process, no net).
fn carry(from: &mut Song, to: &mut Song, now: u64) {
    for o in from.take_outbox() {
        match o {
            Outbound::Act(a) => {
                let _ = to.import_act(a, now);
            }
            Outbound::Moment(a, f) => {
                let _ = to.import_moment(a, f, now);
            }
        }
    }
}

#[test]
fn level1_thousand_twists_one_act() {
    let (_d, mut song, mut live) = setup(Level::LiveControl);
    let mut t = 0;
    // tracks appear -> names known, nothing stamped (no owners)
    let acts = live.tick(&mut song, t).unwrap();
    assert!(acts.is_empty());
    assert_eq!(live.track_name(&track("drums")), Some("Drums"));

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
    assert_eq!(emitted[0].thing(), &Thing::Device { track: track("drums"), slot: DeviceSlot(0) });
    assert_eq!(song.main().track(&track("drums")).unwrap().setting(DeviceSlot(0), &"thresh".into()), Some(0.99));
    assert!(song.chains().is_sealed(&emitted[0].id), "the owner's machine is the keeper");

    // knob returned to where it started = 0 acts
    live.bridge.sweep("drums", DeviceSlot(0), "thresh", 0.99, 0.2, 50);
    live.bridge.sweep("drums", DeviceSlot(0), "thresh", 0.2, 0.99, 50);
    t += 200;
    assert!(live.tick(&mut song, t).unwrap().is_empty());

    // volume + tempo settle too; play/stop are instant
    live.bridge.set_volume("bass", 0.7);
    live.bridge.set_tempo(128.0);
    live.bridge.play();
    let acts = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Transport]); // Play, instantly
    t += 200;
    let acts = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Mixer, Kind::Transport]);
    assert_eq!(song.main().settings.tempo, 128.0);
    assert_eq!(song.main().track(&track("bass")).unwrap().mixer.volume, 0.7);
}

#[test]
fn level0_is_silent_but_saves_capture_audio() {
    let (_d, mut song, mut live) = setup(Level::PluginOnly);
    assert_eq!(live.bridge.capabilities().level, Level::PluginOnly);
    let mut t = 0;
    assert!(live.tick(&mut song, t).unwrap().is_empty());
    for i in 0..1000 {
        live.bridge.twist("drums", DeviceSlot(0), "thresh", (i % 100) as f32 / 100.0);
        live.bridge.set_volume("bass", 0.3);
        live.bridge.set_tempo(90.0);
        t += 10;
        assert!(live.tick(&mut song, t).unwrap().is_empty());
    }
    t += 1000;
    assert!(live.tick(&mut song, t).unwrap().is_empty(), "level 0 never reports knobs");
    assert_eq!(song.chains().len(), 2, "genesis + keeper only");
    // ... but tempo and transport always cross: the plugin reads them from
    // its host's transport info and reports them itself.
    live.report_transport(&mut song, TransportChange::Tempo(90.0), SongPos::START, t).unwrap();
    live.report_transport(&mut song, TransportChange::Play, SongPos::START, t).unwrap();
    t += 200;
    let acts = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Transport]);
    assert_eq!(song.main().settings.tempo, 90.0);
    assert!(song.main().transport.playing);

    // The plugin on the drums track captured its output at the print point
    let cap = live.bridge.capture("drums", SongPos::START, SongPos::from_beats(8.0)).unwrap();
    assert_eq!(cap.print_point, PrintPoint::PostFaderPreSend);
    live.push_captured(track("drums"), cap);
    let state_path = _d.path().join("plugin-state.json");
    std::fs::write(&state_path, b"{}").unwrap();
    let acts = live.save_version(&mut song, "Save 1", &state_path, t, SongPos::from_beats(8.0)).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Version, Kind::Audio]);
    let Payload::Audio(a) = acts[1].payload() else { panic!() };
    // 8 beats = 2 bars = 2 chunks (at the song's tempo, 90)
    assert_eq!(a.opus_chunks.len(), 2);
    assert_eq!(a.flac_chunks.len(), 2);
    assert_eq!(a.print_point, PrintPoint::PostFaderPreSend);
    assert!(a.opus_chunks.iter().all(|h| song.blobs().has(h)));
    assert_eq!(acts[1].body.host, host());
}

#[test]
fn save_renders_only_changed_tracks_and_notes_are_deltas() {
    let (d, mut song, mut live) = setup(Level::Project);
    let mut t = 0;
    live.tick(&mut song, t).unwrap();
    live.bridge.twist("drums", DeviceSlot(0), "thresh", 0.4);
    let n1 = Note { start: SongPos::START, length_ticks: 480, key: 36, velocity: 100, channel: 0 };
    live.bridge.write_notes("bass", "riff", vec![n1.clone()]);
    live.tick(&mut song, t).unwrap(); // ingest the move
    t += 200;
    live.tick(&mut song, t).unwrap(); // settle
    assert_eq!(live.dirty_tracks(), vec![track("drums")]);
    let proj = d.path().join("song.fakeproj");
    live.bridge.seek(SongPos::from_beats(4.0));
    live.bridge.save_project(&proj).unwrap();
    let acts = live.tick(&mut song, t).unwrap();
    // notes from the project (bass changed via the file), version, audio for both changed tracks
    assert_eq!(kinds(&acts), vec![Kind::Notes, Kind::Version, Kind::Audio, Kind::Audio]);
    let Payload::Notes(n) = acts[0].payload() else { panic!() };
    assert_eq!((n.add.len(), n.remove.len()), (1, 0));
    assert_eq!(live.bridge.renders, 2);
    assert!(live.dirty_tracks().is_empty());
    let Payload::Version(v) = acts[1].payload() else { panic!() };
    assert_eq!(v.name, "Save");
    assert_eq!(song.blobs().get(&v.project).unwrap(), std::fs::read(&proj).unwrap());
    // nothing changed -> second save renders nothing
    live.bridge.save_project(&proj).unwrap();
    let acts = live.tick(&mut song, t + 10).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Version]);
    assert_eq!(live.bridge.renders, 2);
    // a note moved: one removed, one added
    let n2 = Note { start: SongPos::from_beats(1.0), ..n1.clone() };
    live.bridge.write_notes("bass", "riff", vec![n2.clone()]);
    live.bridge.save_project(&proj).unwrap();
    let acts = live.tick(&mut song, t + 20).unwrap();
    let Payload::Notes(n) = acts[0].payload() else { panic!("{acts:?}") };
    assert_eq!((n.add.clone(), n.remove.clone()), (vec![n2.clone()], vec![n1.clone()]));
    assert_eq!(song.main().track(&track("bass")).unwrap().clips["riff"], vec![n2]);
    // restore plan uses this version
    let plan = song.restore_plan(&acts[1].id, &host()).unwrap();
    assert_eq!(plan.version.as_ref().map(|e| e.id), Some(acts[1].id));
    assert!(plan.replay.is_empty());
}

/// Dropping a plugin into a slot or removing it is one act at once (no
/// settle); a peer with a bridge that can place plugins applies it, a
/// level-0 peer hears the track; Go back / Show in DAW re-place it.
#[test]
fn plugin_add_remove_is_an_act_and_follows_history() {
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let bob = stamper(2, "bob");
    let mut sa = Song::create(dir.path().join("a"), &alice, "S", 0).unwrap();
    sa.approve(&alice, bob.name, false, 1).unwrap();
    let mut fa = FakeHost::new(Level::LiveControl);
    fa.add_track("drums", "Drums", TrackKind::Part, Some(comp()), 110.0);
    let mut la = LiveLoop::new(fa, PcmChunkEncoder, alice.clone(), main_history());
    la.tick(&mut sa, 0).unwrap();
    let eq = PluginIdentity::vst3("Acme", "EQ", "2.0");
    la.bridge.add_device("drums", DeviceSlot(1), eq.clone());
    let acts = la.tick(&mut sa, 10).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Device], "at once, not settled");
    assert_eq!(sa.main().track(&track("drums")).unwrap().plugin(DeviceSlot(1)), Some(&eq));
    la.bridge.remove_device("drums", DeviceSlot(1));
    let acts2 = la.tick(&mut sa, 20).unwrap();
    assert_eq!(kinds(&acts2), vec![Kind::Device]);
    assert!(sa.main().track(&track("drums")).unwrap().plugin(DeviceSlot(1)).is_none());

    // Bob's copy: a level-1 FakeHost with the EQ installed applies the add
    let gf = sa.chains().fact_for(&sa.genesis().id).cloned();
    let mut sb = Song::join(dir.path().join("b"), sa.genesis().clone(), gf, "S", 0).unwrap();
    let mut fb = FakeHost::new(Level::LiveControl);
    fb.install(eq.clone());
    carry(&mut sa, &mut sb, 30);
    assert_eq!(Receiver::apply(&mut fb, &sb, &acts[0].id), Some(ReceiverDecision::ApplyDevice));
    assert_eq!(fb.device("drums", DeviceSlot(1)), Some(&eq));
    assert_eq!(Receiver::apply(&mut fb, &sb, &acts2[0].id), Some(ReceiverDecision::ApplyDevice));
    assert_eq!(fb.device("drums", DeviceSlot(1)), None);
    // a level-0 host hears the track instead
    let mut f0 = FakeHost::new(Level::PluginOnly);
    assert_eq!(Receiver::apply(&mut f0, &sb, &acts[0].id), Some(ReceiverDecision::HearAsAudio(track("drums"))));

    // Go back to when the EQ was there: apply_state puts it back on Bob's host
    let st = sb.chains().state_at(&acts[0].id).unwrap();
    let applied = Receiver::apply_state(&mut fb, &st, None);
    assert_eq!(applied.devices, 1);
    assert!(applied.manual.is_empty());
    assert_eq!(fb.device("drums", DeviceSlot(1)), Some(&eq));
    // a host that cannot place plugins reports what to do by hand
    let mut fc = FakeHost::new(Level::LiveControl);
    let by_hand = Receiver::apply_state(&mut fc, &st, None);
    assert_eq!(by_hand.devices, 0);
    assert!(by_hand.manual.iter().any(|m| m == "drums: put EQ in slot 2"), "{:?}", by_hand.manual);
}

#[test]
fn receiver_mixed_crew_decisions() {
    // Alice on a level-1 host with the Comp plugin, Bob at level 0, Carol on
    // a level-1 host WITHOUT the plugin.
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let mut song = Song::create(dir.path(), &alice, "S", 0).unwrap();
    let main = main_history();
    let setting = song.setting(&alice, main, &track("drums"), DeviceSlot(0), &comp(), &"x".into(), 0.5, None, 1).unwrap();
    let mixer = song.mixer(&alice, main, &track("drums"), MixerChange::Pan(-0.5), 2).unwrap();
    let tempo = song.tempo(&alice, main, 100.0, 3).unwrap();
    let hs = song.host_state(&alice, main, &track("drums"), vec![7, 7], 4).unwrap();
    let chat = song.chat(&alice, main, "hi", None, 5).unwrap();

    let mut with_plugin = FakeHost::new(Level::LiveControl);
    with_plugin.install(comp());
    let mut level0 = FakeHost::new(Level::PluginOnly);
    let mut without = FakeHost::new(Level::LiveControl).with_tag("otherdaw");

    assert_eq!(Receiver::apply(&mut with_plugin, &song, &setting.id), Some(ReceiverDecision::ApplySetting));
    assert_eq!(with_plugin.param("drums", DeviceSlot(0), "x"), Some(0.5));
    assert_eq!(Receiver::apply(&mut level0, &song, &setting.id), Some(ReceiverDecision::HearAsAudio(track("drums"))));
    assert_eq!(Receiver::apply(&mut without, &song, &setting.id), Some(ReceiverDecision::HearAsAudio(track("drums"))));
    assert!(without.applied.is_empty());

    assert_eq!(Receiver::apply(&mut with_plugin, &song, &mixer.id), Some(ReceiverDecision::ApplyMixer));
    assert_eq!(Receiver::apply(&mut level0, &song, &mixer.id), Some(ReceiverDecision::HearAsAudio(track("drums"))));
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &tempo.id), Some(ReceiverDecision::ApplyTransport));
    assert_eq!(with_plugin.tempo, 100.0);
    assert_eq!(Receiver::apply(&mut level0, &song, &tempo.id), Some(ReceiverDecision::Nothing));
    // HostState: same host applies, other host hears audio
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &hs.id), Some(ReceiverDecision::ApplyHostState));
    assert_eq!(with_plugin.applied_host_state.len(), 1);
    assert_eq!(Receiver::apply(&mut without, &song, &hs.id), Some(ReceiverDecision::HearAsAudio(track("drums"))));
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &chat.id), Some(ReceiverDecision::Nothing));
    // an act that is not in the chains is never applied
    assert_eq!(Receiver::apply(&mut with_plugin, &song, &Hash::of(b"nope")), None);
}

#[test]
fn two_fakehosts_level1_and_level0_share_a_song() {
    // Alice (level 1, keeper) and Bob (level 0) in one song, same process,
    // no network: Bob's copy receives Alice's moments; Alice's receives
    // Bob's act and seals it; Bob's captured audio lands on save.
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let bob = stamper(2, "bob");
    let main = main_history();
    let mut sa = Song::create(dir.path().join("a"), &alice, "S", 0).unwrap();
    sa.approve(&alice, bob.name, false, 1).unwrap();
    let gf = sa.chains().fact_for(&sa.genesis().id).cloned();
    let mut sb = Song::join(dir.path().join("b"), sa.genesis().clone(), gf, "S", 0).unwrap();
    carry(&mut sa, &mut sb, 2);
    let mut fa = FakeHost::new(Level::LiveControl);
    fa.add_track("drums", "Drums", TrackKind::Part, Some(comp()), 110.0);
    let mut la = LiveLoop::new(fa, PcmChunkEncoder, alice.clone(), main);
    let fb = FakeHost::new(Level::PluginOnly);
    let mut lb = LiveLoop::new(fb, PcmChunkEncoder, bob.clone(), main);
    lb.bridge.add_track("vox", "Vox", TrackKind::Part, None, 220.0);

    la.tick(&mut sa, 0).unwrap();
    la.bridge.sweep("drums", DeviceSlot(0), "x", 0.0, 0.8, 100);
    la.tick(&mut sa, 10).unwrap();
    let acts = la.tick(&mut sa, 500).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Setting]);
    carry(&mut sa, &mut sb, 501);
    assert_eq!(sa.main(), sb.main());
    // Bob at level 0 hears drums as audio
    assert_eq!(Receiver::apply(&mut lb.bridge, &sb, &acts[0].id), Some(ReceiverDecision::HearAsAudio(track("drums"))));
    // Bob saves: his captured vox lands as audio on his own track (unsealed until Alice seals)
    let cap = lb.bridge.capture("vox", SongPos::START, SongPos::from_beats(4.0)).unwrap();
    lb.push_captured(track("vox"), cap);
    let p = dir.path().join("b-state.json");
    std::fs::write(&p, b"{}").unwrap();
    let acts = lb.save_version(&mut sb, "Bob save", &p, 1000, SongPos::from_beats(4.0)).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Version, Kind::Audio]);
    assert_eq!(sb.unsealed_count(), 2);
    carry(&mut sb, &mut sa, 1001);
    assert_eq!(sa.unsealed_count(), 0, "Alice's machine sealed them");
    carry(&mut sa, &mut sb, 1002);
    assert_eq!(sb.unsealed_count(), 0);
    assert_eq!(sa.main(), sb.main());
    assert_eq!(sa.main().track(&track("vox")).unwrap().audio.len(), 1);
    // Alice needs Bob's chunks: they are listed as missing until fetched
    let missing = sa.missing_opus(&main);
    assert_eq!(missing.len(), 1);
    for h in &missing {
        sa.blobs().put_verified(h, &sb.blobs().get(h).unwrap()).unwrap();
    }
    assert!(sa.missing_opus(&main).is_empty());
}

/// Presence is stamped once focus has rested ~500 ms; fast clicking does
/// not flood the chain; the same spot twice stamps nothing new; other
/// beings can be shown in the DAW and followed.
#[test]
fn presence_settles_and_is_shown() {
    let (_d, mut song, mut live) = setup(Level::LiveControl);
    let mut t = 0;
    live.tick(&mut song, t).unwrap();
    for i in 0..20 {
        live.bridge.focus(View::Mixer, Some(if i % 2 == 0 { "drums" } else { "bass" }), None);
        t += 50;
        assert!(live.tick(&mut song, t).unwrap().is_empty(), "nothing while clicking around");
    }
    assert_eq!(live.presence_now().map(|p| p.track.clone()).flatten(), Some(track("bass")), "the live signal is there, unrecorded");
    t += 500;
    let acts = live.tick(&mut song, t).unwrap();
    assert_eq!(kinds(&acts), vec![Kind::Presence]);
    assert_eq!(acts[0].thing(), &Thing::Presence(live.stamper().being_ref()));
    let p = &song.main().presence[&live.stamper().being_ref()];
    assert_eq!((p.view, p.track.clone()), (View::Mixer, Some(track("bass"))));
    // resting on the same spot again: nothing
    live.bridge.focus(View::Mixer, Some("bass"), None);
    t += 600;
    assert!(live.tick(&mut song, t).unwrap().is_empty());
    // hidden from the song lens by default, shown on request
    assert!(song.history(&main_history()).iter().all(|r| r.kind != Kind::Presence));
    assert!(song.rows(&main_history(), &Lens::Song, true).iter().any(|r| r.kind == Kind::Presence));

    // show Bob on the drums track in his color, then clear
    let bob = BeingRef { name: Keypair::from_seed(2).id(), being: BeingId::new("bob") };
    let before = live.bridge.color("drums").unwrap();
    let shown = vec![Shown { being: bob.clone(), label: "Bob".into(), color: 0xE07A5F, presence: PresencePayload { view: View::Device, track: Some(track("drums")), device: Some(DeviceSlot(0)), region: None } }];
    assert_eq!(live.bridge.show_presence(&shown), ApplyOutcome::Applied);
    assert_eq!(live.bridge.color("drums"), Some(0xE07A5F));
    live.bridge.clear_presence();
    assert_eq!(live.bridge.color("drums"), Some(before), "original restored");
    assert_eq!(live.bridge.follow(&shown[0].presence), ApplyOutcome::Applied);
    assert_eq!(live.bridge.followed.len(), 1);
}

/// §20: applying moments to the DAW is a diff. Going back and forward
/// across many moments never reloads a device that is already there; a
/// parameter-only jump touches params only; a big jump takes the snapshot
/// path when a project file is at hand.
#[test]
fn apply_is_a_diff_never_a_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let mut song = Song::create(dir.path().join(".flit"), &alice, "S", 0).unwrap();
    let main = main_history();
    let eq = PluginIdentity::vst3("Acme", "EQ", "2.0");
    song.device(&alice, main, &track("drums"), DeviceSlot(0), Some(comp()), 1).unwrap();
    song.device(&alice, main, &track("drums"), DeviceSlot(1), Some(eq.clone()), 2).unwrap();
    let mut moments = Vec::new();
    let comp_plugin = comp();
    for i in 0..100u32 {
        let plugin = if i % 2 == 0 { &comp_plugin } else { &eq };
        let a = song.setting(&alice, main, &track("drums"), DeviceSlot(i % 2), plugin, &ParamId::from_u32(i % 5), (i % 7) as f32 / 7.0, None, 10 + i as u64).unwrap();
        moments.push(a.id);
    }
    let mut fb = FakeHost::new(Level::LiveControl);
    fb.install(comp());
    fb.install(eq.clone());
    // first apply: both devices placed, every param set
    let st = song.state_at(moments.last().unwrap()).unwrap();
    let r = Receiver::apply_state(&mut fb, &st, None);
    assert_eq!((r.devices, fb.device_loads), (2, 2));
    assert!(!r.snapshot_loaded);
    let params_after_first = fb.applied.len();
    // go back and forward across all 100 moments: no device is ever reloaded
    for k in (0..100).rev().chain(0..100) {
        let st = song.state_at(&moments[k]).unwrap();
        Receiver::apply_state(&mut fb, &st, None);
    }
    assert_eq!(fb.device_loads, 2, "unchanged devices are never reloaded");
    assert_eq!(fb.device_removals, 0);
    assert!(fb.applied.len() > params_after_first);
    // a parameter-only jump: exactly the params that differ are set
    let n = fb.applied.len();
    let st_now = song.state_at(&moments[99]).unwrap();
    Receiver::apply_state(&mut fb, &st_now, None);
    assert_eq!(fb.applied.len(), n, "already there: nothing touched");
    let st_prev = song.state_at(&moments[98]).unwrap();
    Receiver::apply_state(&mut fb, &st_prev, None);
    assert_eq!(fb.applied.len() - n, 1, "one param differs between neighbouring moments");

    // a big jump: a moment with six new devices; a Version project file
    // holds them. The snapshot path loads the file instead of six loads.
    let mut rich = FakeHost::new(Level::LiveControl);
    rich.add_track("drums", "Drums", TrackKind::Part, Some(comp()), 110.0);
    rich.add_device("drums", DeviceSlot(1), eq.clone());
    let many: Vec<PluginIdentity> = (0..6u32).map(|i| PluginIdentity::vst3("Acme", &format!("Fx{i}"), "1")).collect();
    rich.add_track("keys", "Keys", TrackKind::Part, None, 330.0);
    for (i, p) in many.iter().enumerate() {
        rich.add_device("keys", DeviceSlot(i as u32), p.clone());
    }
    let proj = dir.path().join("rich.fakeproj");
    rich.save_project(&proj).unwrap();
    for (i, p) in many.iter().enumerate() {
        song.device(&alice, main, &track("keys"), DeviceSlot(i as u32), Some(p.clone()), 200 + i as u64).unwrap();
    }
    let st_big = song.state(&main).unwrap().clone();
    let d = diff(&st_big, &fb.read_state().unwrap(), &host());
    assert_eq!(d.device_changes(), 6);
    assert!(fb.estimate_cost(&d) > SNAPSHOT_THRESHOLD);
    let loads_before = fb.device_loads;
    let r = Receiver::apply_state(&mut fb, &st_big, Some(&proj));
    assert!(r.snapshot_loaded, "big jump takes the snapshot path");
    assert_eq!(fb.snapshot_loads, 1);
    assert_eq!(fb.device_loads, loads_before, "the project file brought the devices; none loaded one by one");
    assert_eq!(fb.tracks[&track("keys")].devices.len(), 6);
    // the same jump without a project file loads them one by one
    let mut plain = FakeHost::new(Level::LiveControl);
    for p in &many {
        plain.install(p.clone());
    }
    plain.install(comp());
    plain.install(eq.clone());
    let r = Receiver::apply_state(&mut plain, &st_big, None);
    assert!(!r.snapshot_loaded);
    assert_eq!(plain.device_loads, 8);
}

/// Bridges may announce a device they merely see (a new track with its
/// plugin already in it, the first snapshot after connecting). Only a
/// change against the song is a moment.
#[test]
fn announcing_a_device_the_song_knows_is_not_a_moment() {
    let dir = tempfile::tempdir().unwrap();
    let alice = stamper(1, "alice");
    let mut song = Song::create(dir.path().join("a"), &alice, "S", 0).unwrap();
    let mut fa = FakeHost::new(Level::LiveControl);
    fa.add_track("drums", "Drums", TrackKind::Part, Some(comp()), 110.0);
    let mut live = LiveLoop::new(fa, PcmChunkEncoder, alice.clone(), main_history());
    live.tick(&mut song, 0).unwrap();
    let eq = PluginIdentity::vst3("Acme", "EQ", "2.0");
    live.bridge.add_device("drums", DeviceSlot(1), eq.clone());
    assert_eq!(kinds(&live.tick(&mut song, 10).unwrap()), vec![Kind::Device]);
    live.bridge.add_device("drums", DeviceSlot(1), eq.clone());
    assert!(live.tick(&mut song, 20).unwrap().is_empty(), "the song already has it");
    live.bridge.remove_device("drums", DeviceSlot(1));
    assert_eq!(kinds(&live.tick(&mut song, 30).unwrap()), vec![Kind::Device]);
    live.bridge.remove_device("drums", DeviceSlot(1));
    assert!(live.tick(&mut song, 40).unwrap().is_empty(), "already empty");
}
