//! Two agents in one process (two Names on one box), one on FakeHost
//! level 1 (the owner: its machine is the keeper), one with no bridge
//! (level 0), talking to each other over iroh and to "plugins" over IPC.
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
    let cfg = AgentConfig { seed: Some(seed), display_name: name.to_string(), data_dir: dir.join(format!("{name}-data")), bridge, local_only: true, relay_only: false, open_song: None, window_hook: None };
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

fn timeline(c: &mut Client, history: HistoryId) -> Vec<Row> {
    match c.request(Request::Timeline { history, lens: Lens::Song, show_presence: false }).unwrap() {
        Response::Timeline(h) => h,
        o => panic!("{o:?}"),
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

    // Alice: a Name with one human being, acting as it
    let st = status(&mut ca);
    assert_eq!(st.being, BeingId::new("alice"));
    assert_eq!(st.being_label, "alice");
    assert_eq!(st.names_on_machine, 1);

    // Alice: new song, FakeHost gets a track; her machine is the keeper
    assert_eq!(ca.request(Request::NewSong { name: "Jam".into(), dir: dir.path().join("alice-proj") }).unwrap(), Response::Ok);
    let st = status(&mut ca);
    assert_eq!(st.level, 1);
    let song = st.song.clone().unwrap();
    assert_eq!(song.name, "Jam");
    assert!(song.i_am_keeper);
    assert_eq!(song.keeper.as_ref().map(|k| k.0), Some(st.name));
    assert!(song.beings.iter().any(|b| b.being.being == BeingId::new("alice") && b.acting));
    let code = song.code.clone();
    assert!(code.starts_with("flit1"));
    assert_eq!(ca.request(Request::Fake(FakeCmd::AddTrack { track: "drums".into(), name: "Drums".into(), kind: "part".into(), plugin: Some(PluginIdentity::vst3("Acme", "Comp", "1.2.0")), tone_hz: 110.0 })).unwrap(), Response::Ok);

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
    assert!(!sb.song.as_ref().unwrap().i_am_keeper);
    // Bob's being was declared in the song and sealed by Alice's keeper
    wait_until(|| status(&mut ca).song.unwrap().beings.iter().any(|b| b.being.name == bob_id && b.display_name == "bob"), "bob's being at alice");
    wait_until(|| status(&mut cb).song.unwrap().unsealed == 0, "bob's acts sealed");

    // Alice twists a knob on FakeHost: one Setting; Bob hears drums as audio
    let n_before = status(&mut cb).song.unwrap().moments;
    assert_eq!(ca.request(Request::Fake(FakeCmd::Sweep { track: "drums".into(), slot: 0, param: "thresh".into(), from: 0.0, to: 0.7, steps: 200 })).unwrap(), Response::Ok);
    wait_until(|| status(&mut cb).song.unwrap().moments > n_before, "setting arrives at bob");
    let hist = timeline(&mut cb, main_history());
    let settings: Vec<&Row> = hist.iter().filter(|r| r.kind == Kind::Setting).collect();
    assert_eq!(settings.len(), 1, "1000 twists = 1 act");
    assert!(settings[0].summary.contains("thresh"));
    assert!(settings[0].sealed);
    wait_until(|| status(&mut cb).song.unwrap().heard_as_audio.contains(&TrackId::new("drums")), "bob hears drums as audio");

    // Chat: Bob -> Alice (and Alice's panel gets a push)
    let mut panel_a = cli(&a, ClientKind::Panel);
    let ev = cb.request(Request::Chat { text: "nice drums".into(), pinned: None }).unwrap();
    assert!(matches!(ev, Response::Stamped(_)));
    wait_until(|| matches!(ca.request(Request::ChatList).unwrap(), Response::Chat(v) if v.len() == 1 && v[0].text == "nice drums" && v[0].from_label == "bob"), "chat at alice");
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

    // Bob's track plugin (level 0) streams audio; Alice's receiver gets it
    let vox = TrackId::new("vox");
    let mut track_b = Client::connect(&ClientOptions { socket: b.socket.clone(), sample_rate: 48_000, channels: 2, ..ClientOptions::new(ClientKind::Track { track: vox.clone() }) }).unwrap();
    let mut recv_a = Client::connect(&ClientOptions { socket: a.socket.clone(), sample_rate: 48_000, channels: 2, ..ClientOptions::new(ClientKind::Receiver { track: vox.clone() }) }).unwrap();
    let mut n = 0usize;
    let mut got_audio = false;
    for blk in 0..300 {
        let pcm: Vec<f32> = (0..480).flat_map(|i| {
            let s = ((n + i) as f32 * 220.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5;
            [s, s]
        }).collect();
        n += 480;
        track_b.send(Request::Audio { track: vox.clone(), pos: SongPos::from_seconds(blk as f64 * 0.01, 120.0), pcm }).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        if let Some(Push::TrackAudio { track, pcm }) = recv_a.try_push() {
            assert_eq!(track, vox);
            assert!(!pcm.is_empty());
            got_audio = true;
            if blk > 100 {
                break;
            }
        }
    }
    assert!(got_audio, "alice's receiver heard bob's track live");

    // Bob saves (level 0): his captured vox becomes Opus+FLAC chunks; Alice fetches them
    let r = cb.request_timeout(Request::SaveVersion { name: "take 1".into() }, Duration::from_secs(60)).unwrap();
    assert!(matches!(r, Response::Stamped(_)), "{r:?}");
    wait_until(|| {
        let s = status(&mut ca).song.unwrap();
        s.tracks.iter().any(|t| t.track == vox) && s.missing_opus == 0 && s.missing_flac == 0
    }, "alice has bob's take, opus and flac");
    let hist_a = timeline(&mut ca, main_history());
    assert!(hist_a.iter().any(|r| r.kind == Kind::Audio && r.being.name == bob_id && r.thing.track() == Some(&vox) && r.sealed));
    assert!(hist_a.iter().any(|r| r.kind == Kind::Version && r.being.name == bob_id));

    // Presence: Alice looks at the drums; Bob's status shows it
    assert_eq!(ca.request(Request::Fake(FakeCmd::Focus { view: View::Mixer, track: Some("drums".into()), slot: None })).unwrap(), Response::Ok);
    wait_until(|| status(&mut cb).song.unwrap().beings.iter().any(|b| b.being.being == BeingId::new("alice") && b.presence.as_ref().and_then(|p| p.track.clone()) == Some(TrackId::new("drums"))), "alice's presence at bob");
    assert!(timeline(&mut cb, main_history()).iter().all(|r| r.kind != Kind::Presence), "hidden by default");

    // Histories + go back through the agent, from Bob
    let first_setting = settings[0].act;
    let Response::History(history) = cb.request(Request::NewHistory { name: "quieter".into(), from: first_setting }).unwrap() else { panic!() };
    assert_eq!(status(&mut cb).song.unwrap().history, history);
    assert_eq!(cb.request(Request::SwitchHistory { history: main_history() }).unwrap(), Response::Ok);
    wait_until(|| matches!(ca.request(Request::Histories).unwrap(), Response::Histories(v) if v.len() == 2), "history reaches alice");

    // A second being under Alice's Name, acting as it: separate act chain
    assert_eq!(ca.request(Request::NewBeing { id: BeingId::new("kade"), kind: BeingKind::Human, display_name: "Kade".into(), color: None }).unwrap(), Response::Ok);
    assert_eq!(ca.request(Request::ActAs { being: BeingId::new("kade") }).unwrap(), Response::Ok);
    assert_eq!(status(&mut ca).being_label, "Kade");
    let Response::Stamped(k) = ca.request(Request::Chat { text: "kade here".into(), pinned: None }).unwrap() else { panic!() };
    wait_until(|| timeline(&mut cb, main_history()).iter().any(|r| r.act == k && r.being.being == BeingId::new("kade") && r.act_seq == 1), "kade's first act at bob");
    assert_eq!(ca.request(Request::ActAs { being: BeingId::new("alice") }).unwrap(), Response::Ok);

    // Both agents shut down cleanly
    assert_eq!(ca.request(Request::Shutdown).unwrap(), Response::Ok);
    assert_eq!(cb.request(Request::Shutdown).unwrap(), Response::Ok);
    std::thread::sleep(Duration::from_millis(300));
    drop(a.rt);
    drop(b.rt);
}

fn library(c: &mut Client) -> LibraryView {
    match c.request(Request::Library).unwrap() {
        Response::Library(l) => l,
        o => panic!("{o:?}"),
    }
}

fn albums(c: &mut Client) -> Vec<AlbumRow> {
    match c.request(Request::Albums).unwrap() {
        Response::Albums(a) => a,
        o => panic!("{o:?}"),
    }
}

/// PACK 2 over IPC: Alice makes an org and invites Bob; Bob accepts (a
/// member on both machines); a song under the org with access = org lets
/// Bob in by code without an approval; org albums carry the song to Bob;
/// Bob's personal album never shows up at Alice.
#[test]
fn orgs_albums_and_org_access_over_ipc() {
    let dir = tempfile::tempdir().unwrap();
    // seeds of their own: the first test's Names run in parallel on this box
    let a = start_agent(dir.path(), 7, "alice", AnyBridge::None(NoBridge::default()));
    let b = start_agent(dir.path(), 8, "bob", AnyBridge::None(NoBridge::default()));
    let mut ca = cli(&a, ClientKind::Cli);
    let mut cb = cli(&b, ClientKind::Cli);
    let bob_id = Keypair::from_seed(8).id();

    // an org, kept by Alice's machine, with a personal shelf made on first run
    assert_eq!(ca.request(Request::NewOrg { name: "Tigard Beats".into() }).unwrap(), Response::Ok);
    let lib = library(&mut ca);
    assert_eq!(lib.orgs.len(), 1);
    let org = lib.orgs[0].org;
    assert!(lib.orgs[0].owner && lib.orgs[0].i_am_keeper && lib.orgs[0].members == 1);
    assert_eq!(lib.orgs[0].name, "Tigard Beats");

    // invite Bob (a friend by his ID text); Bob's inbox shows an org invite
    let sb = status(&mut cb);
    assert_eq!(ca.request(Request::OrgInvite { org, friend: sb.id_text.clone() }).unwrap(), Response::Ok);
    wait_until(|| library(&mut cb).invites.iter().any(|i| i.container == org && i.kind == ContainerKind::Org && i.name == "Tigard Beats"), "org invite at bob");
    assert!(library(&mut cb).orgs.is_empty(), "an invite is not membership");
    assert_eq!(cb.request_timeout(Request::AcceptOrgInvite { org }, Duration::from_secs(30)).unwrap(), Response::Ok);
    wait_until(|| library(&mut ca).orgs[0].members == 2, "bob is a member at alice");
    wait_until(|| library(&mut cb).orgs.first().map(|o| o.member && !o.owner && o.members == 2).unwrap_or(false), "bob is a member at bob");
    assert!(library(&mut cb).invites.is_empty());
    let Response::OrgMembers(rows) = cb.request(Request::OrgMembers { org }).unwrap() else { panic!() };
    assert!(rows.iter().any(|r| r.owner && r.id == Keypair::from_seed(7).id()));
    assert!(rows.iter().any(|r| r.id == bob_id && !r.owner && !r.invited));

    // a song under the org, access = org: Bob gets in by code, no approval
    assert_eq!(ca.request(Request::NewSongIn { name: "Jam".into(), dir: dir.path().join("alice-jam"), org: Some(org), access: Some(Access::Org) }).unwrap(), Response::Ok);
    let lib = library(&mut ca);
    assert_eq!(lib.song_org.as_ref().map(|(o, n)| (*o, n.as_str())), Some((org, "Tigard Beats")));
    assert_eq!(lib.access, Access::Org);
    wait_until(|| library(&mut ca).orgs[0].songs.iter().any(|s| s.name == "Jam" && s.access == Some(Access::Org)), "the org lists its song");
    let song = status(&mut ca).song.unwrap();
    let r = cb.request_timeout(Request::JoinWithCode { code: song.code.clone(), dir: dir.path().join("bob-jam") }, Duration::from_secs(30)).unwrap();
    assert_eq!(r, Response::Ok, "org members join without an approval");
    assert!(status(&mut ca).pending_joins.is_empty());
    wait_until(|| status(&mut cb).song.as_ref().map(|s| s.name == "Jam").unwrap_or(false), "bob has the song");
    // the access = org setting travels by sync after the genesis; act once it is here
    wait_until(|| library(&mut cb).access == Access::Org, "the song's access reaches bob");
    let Response::Stamped(hi) = cb.request(Request::Chat { text: "hi from the org".into(), pinned: None }).unwrap() else { panic!() };
    wait_until(|| timeline(&mut ca, main_history()).iter().any(|r| r.act == hi && r.sealed), "bob's act sealed by the song keeper");

    // org album: Alice makes it and adds the song; Bob sees it with the song he can open
    let Response::Album(album) = ca.request(Request::NewAlbum { scope: AlbumScope::Org(org), name: "EP".into() }).unwrap() else { panic!() };
    assert_eq!(ca.request(Request::AlbumAct { scope: AlbumScope::Org(org), album, action: AlbumAction::AddSong { song: song.id, after: None } }).unwrap(), Response::Ok);
    wait_until(|| albums(&mut cb).iter().any(|r| r.album == album && r.name == "EP" && r.songs.len() == 1 && !r.songs[0].locked && r.songs[0].name.as_deref() == Some("Jam")), "org album at bob");
    assert!(!albums(&mut cb).iter().find(|r| r.album == album).unwrap().can_curate, "a member who did not make it does not curate");
    assert!(matches!(cb.request(Request::AlbumAct { scope: AlbumScope::Org(org), album, action: AlbumAction::Rename("LP".into()) }).unwrap(), Response::Error(_)));
    let Response::Timeline(rows) = ca.request(Request::AlbumTimeline { scope: AlbumScope::Org(org), album }).unwrap() else { panic!() };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.kind == Kind::Album) && rows[0].act == album);

    // personal album: Bob's, never at Alice
    let Response::Album(mine) = cb.request(Request::NewAlbum { scope: AlbumScope::Personal, name: "sketches".into() }).unwrap() else { panic!() };
    assert!(albums(&mut cb).iter().any(|r| r.album == mine && r.scope == AlbumScope::Personal && r.can_curate));
    assert_eq!(library(&mut cb).personal_albums, 1);
    std::thread::sleep(Duration::from_millis(500));
    assert!(albums(&mut ca).iter().all(|r| r.album != mine), "a personal album stays on its Name's machines");

    // back to private: Bob's next act is refused on his own copy
    assert_eq!(ca.request(Request::SetSongOrg { org: Some(org), access: Access::Private }).unwrap(), Response::Ok);
    wait_until(|| matches!(cb.request(Request::Chat { text: "closed?".into(), pinned: None }).unwrap(), Response::Error(_)), "private closes the song to non-crew");

    assert_eq!(ca.request(Request::Shutdown).unwrap(), Response::Ok);
    assert_eq!(cb.request(Request::Shutdown).unwrap(), Response::Ok);
    std::thread::sleep(Duration::from_millis(300));
    drop(a.rt);
    drop(b.rt);
}

/// Two Track instances in one DAW both report the transport (there is no
/// Master instance any more): a change is stamped once.
#[test]
fn transport_from_many_track_instances_is_stamped_once() {
    let dir = tempfile::tempdir().unwrap();
    let a = start_agent(dir.path(), 9, "solo", AnyBridge::None(NoBridge::default()));
    let mut c = cli(&a, ClientKind::Cli);
    assert_eq!(c.request(Request::NewSong { name: "Loop".into(), dir: dir.path().join("loop") }).unwrap(), Response::Ok);
    let mut t1 = cli(&a, ClientKind::Track { track: TrackId::new("master") });
    let mut t2 = cli(&a, ClientKind::Track { track: TrackId::new("keys") });
    let pos = SongPos::START;
    for change in [TransportChange::Play, TransportChange::Stop, TransportChange::Play] {
        assert_eq!(t1.request(Request::Transport { change: change.clone(), pos }).unwrap(), Response::Ok);
        assert_eq!(t2.request(Request::Transport { change, pos }).unwrap(), Response::Ok);
    }
    let rows = timeline(&mut c, main_history());
    let transport: Vec<&Row> = rows.iter().filter(|r| r.kind == Kind::Transport).collect();
    assert_eq!(transport.len(), 3, "play, stop, play: {:?}", transport.iter().map(|r| r.summary.clone()).collect::<Vec<_>>());
    assert_eq!(c.request(Request::Shutdown).unwrap(), Response::Ok);
}

#[test]
fn plugin_client_reconnects_after_agent_restart() {
    // If the agent dies, the plugin shows "reconnecting" and reconnects
    // when it comes back. Audio is never routed through here.
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
    assert_eq!(status(&mut c2).name, Keypair::from_seed(5).id());
    // Names on this machine: the one, current, with its being
    let Response::Names(names) = c2.request(Request::Names).unwrap() else { panic!() };
    assert_eq!(names.len(), 1);
    assert!(names[0].current);
    assert_eq!(names[0].beings[0].0, BeingId::new("solo"));
    let Response::Text(code) = c2.request(Request::RecoveryCode).unwrap() else { panic!() };
    assert_eq!(code.split('-').count(), 13);
    assert_eq!(c2.request(Request::Shutdown).unwrap(), Response::Ok);
}
