//! Two agents in one process (two users on one Linux box), one on FakeHost
//! level 1, one with no bridge (level 0), talking to each other over iroh
//! and to "plugins" over IPC. Addendum 2: the agent owns iroh + the log.
use flit_agent::{AgentConfig, AnyBridge, NoBridge};
use flit_bridge_fake::FakeHost;
use flit_core::*;
use flit_host::Level;
use flit_ipc::*;
use std::time::{Duration, Instant};

struct Running {
    rt: tokio::runtime::Runtime,
    socket: SocketName,
}

fn start_agent(dir: &std::path::Path, seed: u64, name: &str, bridge: AnyBridge) -> Running {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let socket = SocketName(dir.join(format!("{name}.sock")).to_string_lossy().into_owned());
    let cfg = AgentConfig { key: Keypair::from_seed(seed), display_name: name.to_string(), data_dir: dir.join(format!("{name}-data")), bridge, local_only: true, relay_only: false, open_song: None };
    let s2 = socket.clone();
    rt.spawn(async move {
        if let Err(e) = flit_agent::run(cfg, s2).await {
            eprintln!("agent {e}");
        }
    });
    // wait for the socket
    let opts = ClientOptions { socket: socket.clone(), ..ClientOptions::new(ClientKind::Cli) };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Client::connect(&opts).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "agent did not come up");
        std::thread::sleep(Duration::from_millis(50));
    }
    Running { rt, socket }
}

fn cli(r: &Running, kind: ClientKind) -> Client {
    Client::connect(&ClientOptions { socket: r.socket.clone(), ..ClientOptions::new(kind) }).unwrap()
}

fn status(c: &mut Client) -> Status {
    match c.request(Request::Status).unwrap() {
        Response::Status(s) => s,
        other => panic!("{other:?}"),
    }
}

fn wait_until(mut f: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn two_agents_join_sync_save_and_stream() {
    let dir = tempfile::tempdir().unwrap();
    let a = start_agent(dir.path(), 1, "alice", AnyBridge::Fake(FakeHost::new(Level::LiveControl)));
    let b = start_agent(dir.path(), 2, "bob", AnyBridge::None(NoBridge::default()));
    let mut ca = cli(&a, ClientKind::Cli);
    let mut cb = cli(&b, ClientKind::Cli);

    // Alice: new song, FakeHost gets a track
    assert_eq!(ca.request(Request::NewSong { name: "Jam".into(), dir: dir.path().join("alice-proj") }).unwrap(), Response::Ok);
    let st = status(&mut ca);
    assert_eq!(st.level, 1);
    let song = st.song.clone().unwrap();
    assert_eq!(song.name, "Jam");
    assert!(song.i_own_mix);
    let code = song.code.clone();
    assert!(code.starts_with("flit1"));
    assert_eq!(ca.request(Request::Fake(FakeCmd::AddTrack { lane: "drums".into(), name: "Drums".into(), kind: LaneKind::Part, plugin: Some(PluginIdentity::vst3("Acme", "Comp", "1.2.0")), tone_hz: 110.0 })).unwrap(), Response::Ok);

    // Bob joins with the code (blocks until Alice approves)
    let bsock = b.socket.clone();
    let bdir = dir.path().join("bob-proj");
    let joiner = std::thread::spawn(move || {
        let mut c = Client::connect(&ClientOptions { socket: bsock, ..ClientOptions::new(ClientKind::Cli) }).unwrap();
        c.request_timeout(Request::JoinWithCode { code, dir: bdir }, Duration::from_secs(60)).unwrap()
    });
    let bob_id = Keypair::from_seed(2).id();
    wait_until(|| status(&mut ca).pending_joins.iter().any(|(id, _)| *id == bob_id), "join request");
    assert_eq!(status(&mut ca).pending_joins[0].1, "bob");
    assert_eq!(ca.request(Request::Approve { id: bob_id, owner: false }).unwrap(), Response::Ok);
    assert_eq!(joiner.join().unwrap(), Response::Ok);
    wait_until(|| status(&mut cb).peers.iter().any(|p| p.connected), "bob connected");
    wait_until(|| status(&mut ca).peers.iter().any(|p| p.connected), "alice connected");
    let sb = status(&mut cb);
    assert_eq!(sb.level, 0);
    assert_eq!(sb.song.as_ref().unwrap().name, "Jam");
    assert_eq!(sb.song.as_ref().unwrap().members.len(), 2);

    // Alice twists a knob on FakeHost: one Setting; Bob hears drums as audio
    let n_before = status(&mut cb).song.unwrap().events;
    assert_eq!(ca.request(Request::Fake(FakeCmd::Sweep { lane: "drums".into(), slot: 0, param: "thresh".into(), from: 0.0, to: 0.7, steps: 200 })).unwrap(), Response::Ok);
    wait_until(|| status(&mut cb).song.unwrap().events > n_before, "setting arrives at bob");
    let hist = match cb.request(Request::History { branch: stamp::main_branch() }).unwrap() {
        Response::History(h) => h,
        o => panic!("{o:?}"),
    };
    let settings: Vec<_> = hist.iter().filter(|r| r.kind == Kind::Setting).collect();
    assert_eq!(settings.len(), 1, "1000 twists = 1 event");
    assert!(settings[0].summary.contains("thresh"));
    wait_until(|| status(&mut cb).song.unwrap().heard_as_audio.contains(&LaneId::new("drums")), "bob hears drums as audio");

    // Chat: Bob -> Alice (and Alice's panel gets a push)
    let mut panel_a = cli(&a, ClientKind::Panel);
    let ev = cb.request(Request::Chat { text: "nice drums".into(), pinned: None }).unwrap();
    assert!(matches!(ev, Response::Stamped(_)));
    wait_until(|| matches!(ca.request(Request::ChatList).unwrap(), Response::Chat(v) if v.len() == 1 && v[0].text == "nice drums"), "chat at alice");
    let mut got_push = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(Push::Chat(c)) = panel_a.push_timeout(Duration::from_millis(200)) {
            assert_eq!(c.text, "nice drums");
            got_push = true;
            break;
        }
    }
    assert!(got_push, "panel got the chat push");

    // Bob's lane plugin (level 0) streams audio; Alice's receiver gets it
    let vox = LaneId::new("vox");
    let mut lane_b = Client::connect(&ClientOptions { socket: b.socket.clone(), sample_rate: 48_000, channels: 2, ..ClientOptions::new(ClientKind::Lane { lane: vox.clone() }) }).unwrap();
    let mut recv_a = Client::connect(&ClientOptions { socket: a.socket.clone(), sample_rate: 48_000, channels: 2, ..ClientOptions::new(ClientKind::Receiver { lane: vox.clone() }) }).unwrap();
    let mut n = 0usize;
    let mut got_audio = false;
    for blk in 0..300 {
        let pcm: Vec<f32> = (0..480).flat_map(|i| {
            let s = ((n + i) as f32 * 220.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5;
            [s, s]
        }).collect();
        n += 480;
        lane_b.send(Request::Audio { lane: vox.clone(), pos: SongPos::from_seconds(blk as f64 * 0.01, 120.0), pcm }).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        if let Some(Push::LaneAudio { lane, pcm }) = recv_a.try_push() {
            assert_eq!(lane, vox);
            assert!(!pcm.is_empty());
            got_audio = true;
            if blk > 100 {
                break;
            }
        }
    }
    assert!(got_audio, "alice's receiver heard bob's lane live");

    // Bob saves (level 0): his captured vox becomes Opus+FLAC chunks; Alice fetches them
    let r = cb.request_timeout(Request::SaveVersion { name: "take 1".into() }, Duration::from_secs(60)).unwrap();
    assert!(matches!(r, Response::Stamped(_)), "{r:?}");
    wait_until(|| {
        let s = status(&mut ca).song.unwrap();
        s.lanes.iter().any(|l| l.lane == vox && l.owner == Some(bob_id)) && s.missing_opus == 0 && s.missing_flac == 0 && s.events >= 2
    }, "alice has bob's take, opus and flac");
    let hist_a = match ca.request(Request::History { branch: stamp::main_branch() }).unwrap() {
        Response::History(h) => h,
        o => panic!("{o:?}"),
    };
    assert!(hist_a.iter().any(|r| r.kind == Kind::Audio && r.name == bob_id && r.lane.as_ref() == Some(&vox)));
    assert!(hist_a.iter().any(|r| r.kind == Kind::Version && r.name == bob_id));

    // Branches + go back through the agent, from Bob
    let first_setting = settings[0].id;
    let Response::Branch(branch) = cb.request(Request::NewBranch { name: "quieter".into(), from: first_setting }).unwrap() else { panic!() };
    assert_eq!(status(&mut cb).song.unwrap().branch, branch);
    assert_eq!(cb.request(Request::SwitchBranch { branch: stamp::main_branch() }).unwrap(), Response::Ok);
    wait_until(|| matches!(ca.request(Request::Branches).unwrap(), Response::Branches(v) if v.len() == 2), "branch branch reaches alice");

    // Both agents shut down cleanly
    assert_eq!(ca.request(Request::Shutdown).unwrap(), Response::Ok);
    assert_eq!(cb.request(Request::Shutdown).unwrap(), Response::Ok);
    std::thread::sleep(Duration::from_millis(300));
    drop(a.rt);
    drop(b.rt);
}

#[test]
fn plugin_client_reconnects_after_agent_restart() {
    // Addendum 2 §1: if the agent dies, the plugin shows "reconnecting" and
    // reconnects when it comes back. Audio is never routed through here.
    let dir = tempfile::tempdir().unwrap();
    let a = start_agent(dir.path(), 5, "solo", AnyBridge::None(NoBridge::default()));
    let mut c = cli(&a, ClientKind::Panel);
    assert_eq!(c.request(Request::Ping).unwrap(), Response::Pong);
    // kill the agent
    assert_eq!(c.request(Request::Shutdown).unwrap(), Response::Ok);
    std::thread::sleep(Duration::from_millis(400));
    a.rt.shutdown_timeout(Duration::from_secs(2));
    let err = c.request_timeout(Request::Ping, Duration::from_millis(500));
    assert!(err.is_err(), "client must notice the agent is gone");
    assert!(Client::connect(&ClientOptions { socket: a.socket.clone(), ..ClientOptions::new(ClientKind::Panel) }).is_err());
    // the agent restarts (same identity, same data dir) and the client reconnects
    let a2 = start_agent(dir.path(), 5, "solo", AnyBridge::None(NoBridge::default()));
    let mut c2 = cli(&a2, ClientKind::Panel);
    assert_eq!(c2.request(Request::Ping).unwrap(), Response::Pong);
    assert_eq!(status(&mut c2).id, Keypair::from_seed(5).id());
    assert_eq!(c2.request(Request::Shutdown).unwrap(), Response::Ok);
}
