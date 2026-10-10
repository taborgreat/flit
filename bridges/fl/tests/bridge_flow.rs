//! FlBridge against a scripted fake FL on an in-memory MIDI port: the
//! protocol flow (hello, snapshot, paginated params, dirty -> re-read ->
//! ParamChanged, set_param echo suppression, transport).
use flit_bridge_fl::sysex::*;
use flit_bridge_fl::*;
use flit_core::{DeviceSlot, LaneId, MixerChange, ParamId, TransportChange};
use flit_host::{ApplyOutcome, Baseline, HostBridge, HostEvent, Level};
use std::sync::{Arc, Mutex};

/// A MidiPort whose other end is a tiny FL stand-in.
#[derive(Default)]
struct FakeFl {
    inbox: Vec<Vec<u8>>,
    chunker: Chunker,
    pub requests: Vec<FlRequest>,
    pub tempo: f64,
    pub playing: bool,
    pub params: Vec<(u32, String, f32)>,
    pub volume: f32,
    /// Slots of track 1 as the snapshot reports them.
    pub slots: Option<Vec<(i32, String)>>,
    /// Extra inserts (index, name, volume) beyond Master and Drums.
    pub extra: Vec<(i32, String, f32)>,
    /// Answer every Params request with this error.
    pub params_error: Option<String>,
    /// Channel-rack channels as the snapshot reports them.
    pub channels: Vec<FlChannel>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<FakeFl>>);

impl MidiPort for Shared {
    fn send(&mut self, sysex: &[u8]) -> bool {
        let mut fl = self.0.lock().unwrap();
        let Some(frame) = fl.chunker.push(sysex) else { return true };
        let req: FlRequest = serde_json::from_str(&frame.json).unwrap();
        fl.requests.push(req.clone());
        let reply = match &req {
            FlRequest::Hello { .. } => FlReply::Hello { ok: true, api: 40, fl: "FL 25.2.5".into(), python: "3.12.1".into(), ppq: 96 },
            FlRequest::Snapshot => FlReply::Snapshot {
                ok: true,
                tempo: fl.tempo,
                playing: fl.playing,
                pos_ticks: 384,
                ppq: 96,
                channels: fl.channels.clone(),
                tracks: {
                    let mut v = vec![
                        FlTrack { index: 0, name: "Master".into(), volume: 0.8, pan: 0.0, mute: false, slots: vec![] },
                        FlTrack { index: 1, name: "Drums".into(), volume: fl.volume, pan: 0.0, mute: false, slots: fl.slots.clone().unwrap_or_else(|| vec![(0, "Fruity Limiter".into())]) },
                    ];
                    for (i, n, vol) in &fl.extra {
                        v.push(FlTrack { index: *i, name: n.clone(), volume: *vol, pan: 0.0, mute: false, slots: vec![] });
                    }
                    v
                },
            },
            FlRequest::Params { .. } if fl.params_error.is_some() => FlReply::Error { ok: false, error: fl.params_error.clone().unwrap() },
            FlRequest::Params { track, slot, from, count } => {
                let page: Vec<FlParam> = fl.params.iter().filter(|(i, _, _)| *i >= *from && *i < from + count).map(|(i, n, v)| FlParam { index: *i, name: n.clone(), value: *v, text: String::new() }).collect();
                let total = 4240u32;
                FlReply::Params { ok: true, track: *track, slot: *slot, total, next: (from + count).min(total), params: page }
            }
            FlRequest::SetParam { param, value, .. } => {
                if let Some(p) = fl.params.iter_mut().find(|(i, _, _)| i == param) {
                    p.2 = *value;
                }
                FlReply::Ok { ok: true }
            }
            FlRequest::SetVolume { value, .. } => {
                fl.volume = *value;
                FlReply::Ok { ok: true }
            }
            FlRequest::Play => {
                fl.playing = true;
                FlReply::Ok { ok: true }
            }
            FlRequest::SetTempo { bpm } => {
                fl.tempo = *bpm;
                FlReply::Ok { ok: true }
            }
            _ => FlReply::Ok { ok: true },
        };
        let out = encode(&Frame { dir: Dir::Reply, id: frame.id, json: serde_json::to_string(&reply).unwrap() });
        fl.inbox.extend(out);
        true
    }
    fn recv(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.0.lock().unwrap().inbox)
    }
}

fn event(fl: &Shared, ev: &FlEvent) {
    let out = encode(&Frame { dir: Dir::Event, id: 0, json: serde_json::to_string(ev).unwrap() });
    fl.0.lock().unwrap().inbox.extend(out);
}

/// Poll until the bridge has nothing in flight.
fn settle(b: &mut FlBridge) -> Vec<HostEvent> {
    let mut out = Vec::new();
    for _ in 0..50 {
        out.extend(b.poll());
    }
    out
}

#[test]
fn hello_snapshot_scan_dirty_and_apply() {
    let fl = Shared::default();
    {
        let mut f = fl.0.lock().unwrap();
        f.tempo = 128.0;
        f.volume = 0.8;
        f.params = vec![(0, "Threshold".into(), 0.5), (1, "Ratio".into(), 0.25), (300, "Deep".into(), 0.9)];
    }
    let mut b = FlBridge::new(Box::new(fl.clone()));
    assert_eq!(b.capabilities().level, Level::LiveControl);
    assert!(!b.capabilities().honors_print_point, "FL cannot print post-fader without rerouting (VERIFY ON WINDOWS)");
    let evs = settle(&mut b);
    // first pass: lanes seen (mix + drums), tempo learned; the first param
    // scan is a baseline and produces no ParamChanged
    assert!(evs.iter().any(|e| matches!(e, HostEvent::LaneSeen { lane, kind: flit_core::LaneKind::Mix, .. } if lane.0 == "mix")));
    assert!(evs.iter().any(|e| matches!(e, HostEvent::LaneSeen { lane, name, .. } if lane.0 == "fl-track-1" && name == "Drums")));
    assert!(evs.iter().any(|e| matches!(e, HostEvent::Transport { change: TransportChange::Tempo(t), .. } if *t == 128.0)));
    assert!(!evs.iter().any(|e| matches!(e, HostEvent::ParamChanged { .. })), "{evs:?}");
    assert!(b.script.is_some());
    let n_param_pages = fl.0.lock().unwrap().requests.iter().filter(|r| matches!(r, FlRequest::Params { .. })).count();
    assert!(n_param_pages >= 7 && n_param_pages <= 30, "scanned to MAX_SCAN_PARAM in pages: {n_param_pages}");

    // the user turns Ratio: FL only says "dirty"; the bridge re-reads and finds it
    fl.0.lock().unwrap().params[1].2 = 0.6;
    event(&fl, &FlEvent::Dirty { flags: 4096, tracks: vec![1], channels: vec![] });
    let evs = settle(&mut b);
    let changed: Vec<&HostEvent> = evs.iter().filter(|e| matches!(e, HostEvent::ParamChanged { .. })).collect();
    assert_eq!(changed.len(), 1, "{evs:?}");
    let HostEvent::ParamChanged { lane, slot, plugin, param, value, .. } = changed[0] else { panic!() };
    assert_eq!(lane, &LaneId::new("fl-track-1"));
    assert_eq!(*slot, DeviceSlot(0));
    assert_eq!(plugin.name, "Fruity Limiter");
    assert_eq!(param, &ParamId::from_u32(1));
    assert!((value - 0.6).abs() < 1e-6);
    // the first change of a control is preceded by its value from before,
    // so Go back can restore a knob to where it was
    let bi = evs.iter().position(|e| matches!(e, HostEvent::Baseline { baseline: Baseline::Param { param, value, .. }, .. } if *param == ParamId::from_u32(1) && (value - 0.25).abs() < 1e-6)).expect("baseline with the old value: {evs:?}");
    let ci = evs.iter().position(|e| matches!(e, HostEvent::ParamChanged { .. })).unwrap();
    assert!(bi < ci, "baseline comes first");
    // a second change of the same knob has no second baseline
    fl.0.lock().unwrap().params[1].2 = 0.7;
    event(&fl, &FlEvent::Dirty { flags: 4096, tracks: vec![1], channels: vec![] });
    let evs = settle(&mut b);
    assert_eq!(evs.iter().filter(|e| matches!(e, HostEvent::ParamChanged { .. })).count(), 1);
    assert!(!evs.iter().any(|e| matches!(e, HostEvent::Baseline { .. })), "{evs:?}");

    // fader move shows up as a mixer change in gain (0.8 = unity)
    fl.0.lock().unwrap().volume = 0.4;
    event(&fl, &FlEvent::Dirty { flags: 4, tracks: vec![1], channels: vec![] });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::MixerChanged { change: MixerChange::Volume(g), .. } if (g - 0.5).abs() < 1e-6)), "{evs:?}");
    let bi = evs.iter().position(|e| matches!(e, HostEvent::Baseline { baseline: Baseline::Mixer { change: MixerChange::Volume(g), .. }, .. } if (g - 1.0).abs() < 1e-6)).expect("fader baseline at unity");
    let ci = evs.iter().position(|e| matches!(e, HostEvent::MixerChanged { .. })).unwrap();
    assert!(bi < ci);

    // heartbeat: play state and tempo
    event(&fl, &FlEvent::Heartbeat { playing: true, tempo: 128.0, pos_ticks: 960, changed: None, title: None, undo_count: None, undo_pos: None });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::Transport { change: TransportChange::Play, pos } if pos.beats() == 10.0)));
    assert!(b.is_connected());

    // a peer's setting is applied through plugins.setParamValue and does
    // not echo back as our own change when FL reports dirty afterwards
    let plug = flit_core::PluginIdentity::new(flit_core::PluginFormat::Other("fl-wrapper".into()), "", "Fruity Limiter", "fl");
    assert_eq!(b.set_param(&LaneId::new("fl-track-1"), DeviceSlot(0), &plug, &ParamId::from_u32(0), 0.75), ApplyOutcome::Applied);
    let other = flit_core::PluginIdentity::vst3("Acme", "Comp", "1");
    assert_eq!(b.set_param(&LaneId::new("fl-track-1"), DeviceSlot(0), &other, &ParamId::from_u32(0), 0.75), ApplyOutcome::NotAvailable);
    settle(&mut b);
    assert!((fl.0.lock().unwrap().params[0].2 - 0.75).abs() < 1e-6);
    event(&fl, &FlEvent::Dirty { flags: 4096, tracks: vec![1], channels: vec![] });
    let evs = settle(&mut b);
    assert!(!evs.iter().any(|e| matches!(e, HostEvent::ParamChanged { .. })), "applied setting must not echo: {evs:?}");

    // transport/mixer/tempo out
    assert_eq!(b.set_transport(&TransportChange::Tempo(100.0)), ApplyOutcome::Applied);
    assert_eq!(b.set_mixer(&LaneId::new("fl-track-1"), &MixerChange::Volume(1.0)), ApplyOutcome::Applied);
    settle(&mut b);
    let f = fl.0.lock().unwrap();
    assert_eq!(f.tempo, 100.0);
    assert!((f.volume - 0.8).abs() < 1e-6, "unity gain -> 0.8");
    assert!(f.requests.iter().any(|r| matches!(r, FlRequest::SetTempo { bpm } if *bpm == 100.0)));
}

/// Saves and project switches come from the heartbeat: FL's changed flag
/// dropping to 0 on a freshly written file = Save; a new title = another
/// project. The path comes from the title + (projects dir | FL's recent
/// files | default folder).
#[test]
fn heartbeat_detects_saves_and_project_switches() {
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("my song.flp");
    let other = dir.path().join("other.flp");
    std::fs::write(&song, b"flp").unwrap();
    std::fs::write(&other, b"flp").unwrap();
    let fl = Shared::default();
    fl.0.lock().unwrap().tempo = 120.0;
    // keep this machine's real FL recent-files list out of the test
    std::env::set_var("FLIT_FL_RECENT", dir.path().join("not-there.flp").display().to_string());
    let cfg = FlConfig { projects_dir: Some(dir.path().to_path_buf()), ..FlConfig::default() };
    let mut b = FlBridge::with_config(Box::new(fl.clone()), cfg);
    settle(&mut b);
    assert_eq!(b.current_project(), None, "no title yet and no recent file on disk");

    // first title: just learn it, no Opened (the agent already knows the song)
    event(&fl, &FlEvent::Heartbeat { playing: false, tempo: 120.0, pos_ticks: 0, changed: Some(1), title: Some("my song.flp * - FL Studio 2024".into()), undo_count: None, undo_pos: None });
    let evs = settle(&mut b);
    assert!(!evs.iter().any(|e| matches!(e, HostEvent::ProjectSaved { .. } | HostEvent::ProjectOpened { .. })), "{evs:?}");
    assert_eq!(b.current_project(), Some(song.clone()));

    // changed 1 -> 0 on a fresh file = the user saved
    event(&fl, &FlEvent::Heartbeat { playing: false, tempo: 120.0, pos_ticks: 0, changed: Some(0), title: Some("my song.flp - FL Studio 2024".into()), undo_count: None, undo_pos: None });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::ProjectSaved { path, .. } if *path == song)), "{evs:?}");
    // staying at 0 is not another save
    event(&fl, &FlEvent::Heartbeat { playing: false, tempo: 120.0, pos_ticks: 0, changed: Some(0), title: Some("my song.flp - FL Studio 2024".into()), undo_count: None, undo_pos: None });
    let evs = settle(&mut b);
    assert!(!evs.iter().any(|e| matches!(e, HostEvent::ProjectSaved { .. })), "{evs:?}");

    // another project: tracks forgotten, re-snapshot, ProjectOpened
    let before = fl.0.lock().unwrap().requests.iter().filter(|r| matches!(r, FlRequest::Snapshot)).count();
    event(&fl, &FlEvent::Heartbeat { playing: false, tempo: 120.0, pos_ticks: 0, changed: Some(0), title: Some("FL Studio 2024 - other.flp".into()), undo_count: None, undo_pos: None });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::ProjectOpened { path, .. } if *path == other)), "{evs:?}");
    assert!(fl.0.lock().unwrap().requests.iter().filter(|r| matches!(r, FlRequest::Snapshot)).count() > before);
    assert_eq!(b.current_project(), Some(other.clone()));

    // the explicit project_loaded event does the same
    event(&fl, &FlEvent::ProjectLoaded { title: "my song.flp - FL Studio 2024".into() });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::ProjectOpened { path, .. } if *path == song)), "{evs:?}");

    // title parsing and the recent-files override
    assert_eq!(bridge::title_to_filename("FL Studio 2024 - a b.flp *"), Some("a b.flp".into()));
    assert_eq!(bridge::title_to_filename("Untitled - FL Studio 2024"), None);
    std::env::set_var("FLIT_FL_RECENT", format!("{};{}", other.display(), song.display()));
    assert_eq!(bridge::recent_projects(), vec![other.clone(), song.clone()]);
    // with no projects dir the recent-files list resolves the title
    let cfg = FlConfig { projects_dir: None, ..FlConfig::default() };
    let fl2 = Shared::default();
    let mut b2 = FlBridge::with_config(Box::new(fl2.clone()), cfg);
    event(&fl2, &FlEvent::Heartbeat { playing: false, tempo: 120.0, pos_ticks: 0, changed: Some(0), title: Some("other.flp - FL Studio 2024".into()), undo_count: None, undo_pos: None });
    settle(&mut b2);
    assert_eq!(b2.current_project(), Some(other.clone()));
    std::env::remove_var("FLIT_FL_RECENT");
}

/// The agent may start before the script is assigned in FL: it keeps
/// knocking (Hello every 2 s) and connects as soon as the script answers.
#[test]
fn hello_is_retried_until_the_script_answers() {
    #[derive(Default)]
    struct Silent(Vec<Vec<u8>>);
    impl MidiPort for Silent {
        fn send(&mut self, sysex: &[u8]) -> bool {
            self.0.push(sysex.to_vec());
            true
        }
        fn recv(&mut self) -> Vec<Vec<u8>> {
            vec![]
        }
    }
    let cfg = FlConfig { request_timeout: std::time::Duration::from_millis(50), ..FlConfig::default() };
    let mut b = FlBridge::with_config(Box::new(Silent::default()), cfg);
    for _ in 0..6 {
        b.poll();
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
    assert!(!b.is_connected());
    assert_eq!(b.capabilities().level, Level::LiveControl);
    assert!(b.describe().contains("waiting for the FL script"), "{}", b.describe());
    // the retry fires after 2 s of silence (the first hello was at construction)
    std::thread::sleep(std::time::Duration::from_millis(2100));
    b.poll();
    // we cannot see the port from here, but the bridge now has a request in flight again
    assert!(b.describe().contains("waiting"), "{}", b.describe());
}

/// Plugins appearing in / leaving a slot become DeviceChanged (verified
/// need on FL 24.1.1: the user adds a compressor and nothing was stamped);
/// unused inserts are not claimed as lanes until something happens on
/// them; a scan that hits "Plugin not valid" simply ends; set_device only
/// says Applied when the slot already matches.
#[test]
fn device_changes_unused_tracks_and_scan_errors() {
    let fl = Shared::default();
    {
        let mut f = fl.0.lock().unwrap();
        f.tempo = 120.0;
        f.volume = 0.8;
        f.params = vec![(0, "Threshold".into(), 0.5)];
        f.extra = vec![(5, "Insert 5".into(), 0.8), (7, "Vox".into(), 0.8)];
    }
    let mut b = FlBridge::new(Box::new(fl.clone()));
    let evs = settle(&mut b);
    let seen: Vec<String> = evs.iter().filter_map(|e| match e { HostEvent::LaneSeen { lane, .. } => Some(lane.0.clone()), _ => None }).collect();
    assert_eq!(seen, vec!["mix", "fl-track-1", "fl-track-7"], "Insert 5 (default name, unity, no plugins) is not a lane yet: {seen:?}");
    assert!(!evs.iter().any(|e| matches!(e, HostEvent::DeviceChanged { .. })), "the first snapshot is the baseline");

    // the user drops a compressor into slot 2 of Drums
    fl.0.lock().unwrap().slots = Some(vec![(0, "Fruity Limiter".into()), (1, "Fruity Compressor".into())]);
    event(&fl, &FlEvent::Dirty { flags: 4, tracks: vec![1], channels: vec![] });
    let evs = settle(&mut b);
    let dev: Vec<&HostEvent> = evs.iter().filter(|e| matches!(e, HostEvent::DeviceChanged { .. })).collect();
    assert_eq!(dev.len(), 1, "{evs:?}");
    let HostEvent::DeviceChanged { lane, slot, plugin, .. } = dev[0] else { panic!() };
    assert_eq!(lane, &LaneId::new("fl-track-1"));
    assert_eq!(*slot, DeviceSlot(1));
    assert_eq!(plugin.as_ref().map(|p| p.name.as_str()), Some("Fruity Compressor"));
    // set_device: matching slot = nothing to do; "removed" = the slot is switched
    // off (FL cannot delete plugins) with a notice; back again = switched on;
    // a plugin FL does not have is a step for the user
    let comp = plugin.clone().unwrap();
    assert_eq!(b.set_device(&LaneId::new("fl-track-1"), DeviceSlot(1), Some(&comp)), ApplyOutcome::Applied);
    assert_eq!(b.set_device(&LaneId::new("fl-track-1"), DeviceSlot(1), None), ApplyOutcome::Applied, "bypassed instead");
    let evs = settle(&mut b);
    assert!(fl.0.lock().unwrap().requests.iter().any(|r| matches!(r, FlRequest::SetSlotEnabled { track: 1, slot: 1, value: false })), "slot switched off");
    assert!(evs.iter().any(|e| matches!(e, HostEvent::Notice(t) if t.contains("switched off"))), "{evs:?}");
    assert_eq!(b.set_device(&LaneId::new("fl-track-1"), DeviceSlot(1), Some(&comp)), ApplyOutcome::Applied, "and on again");
    settle(&mut b);
    assert!(fl.0.lock().unwrap().requests.iter().any(|r| matches!(r, FlRequest::SetSlotEnabled { track: 1, slot: 1, value: true })));
    assert_eq!(b.set_device(&LaneId::new("fl-track-1"), DeviceSlot(4), None), ApplyOutcome::Applied, "empty stays empty");
    let other = flit_core::PluginIdentity::new(flit_core::PluginFormat::Other("fl-wrapper".into()), "", "Fruity Reeverb 2", "fl");
    assert_eq!(b.set_device(&LaneId::new("fl-track-1"), DeviceSlot(4), Some(&other)), ApplyOutcome::Unsupported, "FL cannot load it");
    assert!(!b.capabilities().set_devices);

    // the user removes it again
    fl.0.lock().unwrap().slots = Some(vec![(0, "Fruity Limiter".into())]);
    event(&fl, &FlEvent::Dirty { flags: 4, tracks: vec![1], channels: vec![] });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::DeviceChanged { slot, plugin: None, .. } if *slot == DeviceSlot(1))), "{evs:?}");

    // Insert 5 becomes a lane the moment its fader moves (announced before the change)
    fl.0.lock().unwrap().extra[0].2 = 0.5;
    event(&fl, &FlEvent::Dirty { flags: 4, tracks: vec![5], channels: vec![] });
    let evs = settle(&mut b);
    let li = evs.iter().position(|e| matches!(e, HostEvent::LaneSeen { lane, .. } if lane.0 == "fl-track-5")).expect("announced: {evs:?}");
    let mi = evs.iter().position(|e| matches!(e, HostEvent::MixerChanged { lane, .. } if lane.0 == "fl-track-5")).expect("fader change");
    assert!(li < mi);

    // a scan that errors ends instead of hanging
    fl.0.lock().unwrap().params_error = Some("RuntimeError: Plugin not valid".into());
    fl.0.lock().unwrap().slots = Some(vec![(0, "Fruity Limiter".into()), (2, "Ghost".into())]);
    event(&fl, &FlEvent::Dirty { flags: 4, tracks: vec![1], channels: vec![] });
    let _ = settle(&mut b);
    let n_before = fl.0.lock().unwrap().requests.iter().filter(|r| matches!(r, FlRequest::Params { slot: 2, .. })).count();
    event(&fl, &FlEvent::Heartbeat { playing: false, tempo: 120.0, pos_ticks: 0, changed: None, title: None, undo_count: None, undo_pos: None });
    let _ = settle(&mut b);
    let n_after = fl.0.lock().unwrap().requests.iter().filter(|r| matches!(r, FlRequest::Params { slot: 2, .. })).count();
    assert_eq!(n_before, n_after, "no retry storm after an error");
    assert!(n_before >= 1);
}

/// Channel-rack channels are lanes of their own: the generator is device
/// slot 0 (params scanned with FL slot -1), a swapped generator is a Device
/// fact, the channel's own volume is the lane's fader.
#[test]
fn channels_are_lanes_with_their_generator_as_slot_zero() {
    let fl = Shared::default();
    {
        let mut f = fl.0.lock().unwrap();
        f.tempo = 120.0;
        f.volume = 0.8;
        f.params = vec![(0, "Cutoff".into(), 0.3)];
        f.channels = vec![
            FlChannel { index: 0, name: "Kick".into(), plugin: None, insert: 1, volume: 0.78125, pan: 0.0, mute: false },
            FlChannel { index: 1, name: "Lead".into(), plugin: Some("Sytrus".into()), insert: 2, volume: 0.78125, pan: 0.0, mute: false },
        ];
    }
    let mut b = FlBridge::new(Box::new(fl.clone()));
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::LaneSeen { lane, name, .. } if lane.0 == "fl-channel-1" && name == "Lead")), "{evs:?}");
    assert!(evs.iter().any(|e| matches!(e, HostEvent::LaneSeen { lane, .. } if lane.0 == "fl-channel-0")));
    assert!(fl.0.lock().unwrap().requests.iter().any(|r| matches!(r, FlRequest::Params { track: 1, slot: -1, .. })), "generator params scanned with slot -1");
    assert!(!fl.0.lock().unwrap().requests.iter().any(|r| matches!(r, FlRequest::Params { track: 0, slot: -1, .. })), "a sampler has no plugin to scan");
    assert!(b.lanes().contains(&LaneId::new("fl-channel-1")));

    // a knob on the generator
    fl.0.lock().unwrap().params[0].2 = 0.9;
    event(&fl, &FlEvent::Dirty { flags: 4096, tracks: vec![], channels: vec![1] });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::ParamChanged { lane, slot, plugin, .. } if lane.0 == "fl-channel-1" && *slot == DeviceSlot(0) && plugin.name == "Sytrus")), "{evs:?}");
    // a peer's setting for it goes to FL slot -1
    let plug = flit_core::PluginIdentity::new(flit_core::PluginFormat::Other("fl-wrapper".into()), "", "Sytrus", "fl");
    assert_eq!(b.set_param(&LaneId::new("fl-channel-1"), DeviceSlot(0), &plug, &ParamId::from_u32(0), 0.4), ApplyOutcome::Applied);
    settle(&mut b);
    assert!(fl.0.lock().unwrap().requests.iter().any(|r| matches!(r, FlRequest::SetParam { track: 1, slot: -1, param: 0, .. })));

    // the user swaps the instrument and turns the channel down
    {
        let mut f = fl.0.lock().unwrap();
        f.channels[1].plugin = Some("Serum".into());
        f.channels[1].volume = 0.390625;
    }
    event(&fl, &FlEvent::Dirty { flags: 4096, tracks: vec![], channels: vec![1] });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::DeviceChanged { lane, slot, plugin: Some(p), .. } if lane.0 == "fl-channel-1" && *slot == DeviceSlot(0) && p.name == "Serum")), "{evs:?}");
    assert!(evs.iter().any(|e| matches!(e, HostEvent::MixerChanged { lane, change: MixerChange::Volume(g), .. } if lane.0 == "fl-channel-1" && (g - 0.5).abs() < 1e-6)), "{evs:?}");
    assert_eq!(b.set_mixer(&LaneId::new("fl-channel-1"), &MixerChange::Volume(1.0)), ApplyOutcome::Applied);
    settle(&mut b);
    assert!(fl.0.lock().unwrap().requests.iter().any(|r| matches!(r, FlRequest::SetChanVolume { chan: 1, value } if (value - 0.78125).abs() < 1e-6)));
    // deleting the channel empties its generator slot
    fl.0.lock().unwrap().channels.truncate(1);
    event(&fl, &FlEvent::Dirty { flags: 4096, tracks: vec![], channels: vec![] });
    let evs = settle(&mut b);
    assert!(evs.iter().any(|e| matches!(e, HostEvent::DeviceChanged { lane, plugin: None, .. } if lane.0 == "fl-channel-1")), "{evs:?}");
    // save goes through FL's own Save command
    assert_eq!(b.request_save(), ApplyOutcome::Applied);
    settle(&mut b);
    assert!(fl.0.lock().unwrap().requests.iter().any(|r| matches!(r, FlRequest::Save)));
}

#[test]
fn lane_mapping_and_gain_law() {
    assert_eq!(FlBridge::lane_for_track(0), LaneId::new("mix"));
    assert_eq!(FlBridge::lane_for_track(7), LaneId::new("fl-track-7"));
    assert_eq!(FlBridge::track_for_lane(&LaneId::new("fl-track-7")), Some(7));
    assert_eq!(FlBridge::track_for_lane(&LaneId::new("mix")), Some(0));
    assert_eq!(FlBridge::track_for_lane(&LaneId::new("vox")), None);
    assert!((bridge::fl_to_gain(0.8) - 1.0).abs() < 1e-6);
    assert!((bridge::gain_to_fl(0.5) - 0.4).abs() < 1e-6);
}

#[test]
fn flp_snapshot_maps_to_host_neutral() {
    let s = flp::FlpSnapshot {
        version: "20.8.4".into(),
        ppq: 96,
        tempo: 140.0,
        channels: vec![flp::FlpChannel { iid: 0, name: Some("Kick".into()), kind: "Sampler".into(), insert: Some(1), plugin: None }],
        patterns: vec![flp::FlpPattern { iid: 1, name: Some("Beat".into()), notes: vec![flp::FlpNote { position: 96, length: 48, key: 60, velocity: 100, channel: 0 }] }],
        inserts: vec![],
        files: vec![],
        warnings: vec![],
    };
    let snap = flp::to_snapshot(&s);
    assert_eq!(snap.tempo, Some(140.0));
    assert_eq!(snap.notes.len(), 1);
    let (lane, clip, notes) = &snap.notes[0];
    assert_eq!(lane, &LaneId::new("fl-channel-0"), "notes belong to the channel's own lane");
    assert_eq!(clip, "Beat");
    assert_eq!(notes[0].start.beats(), 1.0);
    assert_eq!(notes[0].length_ticks, 480);
    assert_eq!(notes[0].key, 60);
}

/// Level 2 through the real PyFLP helper, when a venv with pyflp is around.
#[test]
fn flp_helper_on_sample_file_if_python_available() {
    let python = std::env::var("FLIT_TEST_PYTHON").ok().map(std::path::PathBuf::from);
    let Some(python) = python else {
        eprintln!("FLIT_TEST_PYTHON not set; skipping PyFLP helper test");
        return;
    };
    let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("flp/samples/multi-channel.flp");
    let snap = flp::read_project(&python, &sample).expect("helper runs");
    assert_eq!(snap.tempo, Some(140.0));
    assert_eq!(snap.notes.iter().map(|(_, _, n)| n.len()).sum::<usize>(), 2);
}
