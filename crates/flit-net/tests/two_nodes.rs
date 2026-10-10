//! L4: two FLit instances on one Linux box, each on FakeHost (one level 1,
//! one level 0), join by song code, stay in sync, chat crosses, blobs flow
//! Opus-first then FLAC, blocked IDs are refused, quotas bite.
use flit_audio::FlitChunkEncoder;
use flit_bridge_fake::FakeHost;
use flit_core::*;
use flit_host::*;
use flit_net::*;
use std::time::Duration;
use tokio::sync::broadcast;

fn comp() -> PluginIdentity {
    PluginIdentity::vst3("Acme", "Comp", "1.2.0")
}
fn lane(s: &str) -> LaneId {
    LaneId::new(s)
}
fn host() -> HostTag {
    HostTag::new("fake")
}

async fn wait_for(rx: &mut broadcast::Receiver<NetEvent>, mut pred: impl FnMut(&NetEvent) -> bool) -> NetEvent {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if pred(&ev) {
                        return ev;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("channel closed: {e}"),
            }
        }
    })
    .await
    .expect("timed out waiting for a net event")
}

async fn node(seed: u64, name: &str) -> Node {
    Node::bind(NodeConfig::new(Keypair::from_seed(seed), name).local()).await.unwrap()
}

/// Alice creates a song and shares the code; Bob joins with it.
async fn alice_and_bob(dir: &std::path::Path) -> (Node, Node) {
    let alice = node(1, "Alice").await;
    let bob = node(2, "Bob").await;
    let song = Song::create(dir.join("alice/.flit"), alice.key(), "Jam", &host(), Time::at(0, SongPos::START)).unwrap();
    alice.open_song(song);
    let code = alice.song_code().unwrap();
    let code_str = code.encode();
    assert!(code_str.starts_with("flit1"));
    let code2 = SongCode::decode(&code_str).unwrap();
    assert_eq!(code2, code);

    let mut arx = alice.subscribe();
    let mut brx = bob.subscribe();
    let bob_dir = dir.join("bob/.flit");
    let join = {
        let bob_ref = &bob;
        async move { bob_ref.request_join(&code2, &bob_dir, Duration::from_secs(20)).await }
    };
    let approve = async {
        let ev = wait_for(&mut arx, |e| matches!(e, NetEvent::JoinRequest { .. })).await;
        let NetEvent::JoinRequest { from, display_name } = ev else { unreachable!() };
        assert_eq!(from, bob.id());
        assert_eq!(display_name, "Bob");
        // nothing joins automatically: Bob is not in the crew until Approve
        assert!(!alice.with_song(|s| s.crew().is_member(&from)).unwrap());
        alice.approve(from, false).unwrap();
    };
    let (reply, _) = tokio::join!(join, approve);
    assert!(matches!(reply.unwrap(), flit_net::proto::JoinReply::Welcome { .. }));
    wait_for(&mut brx, |e| matches!(e, NetEvent::PeerUp(_))).await;
    wait_for(&mut arx, |e| matches!(e, NetEvent::PeerUp(_))).await;
    (alice, bob)
}

async fn settle(alice: &Node, bob: &Node) {
    for _ in 0..200 {
        let a = alice.with_song(|s| s.chains().all_ids()).unwrap();
        let b = bob.with_song(|s| s.chains().all_ids()).unwrap();
        if a == b {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("logs did not converge");
}

#[tokio::test]
async fn join_sync_chat_and_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let (alice, bob) = alice_and_bob(dir.path()).await;
    settle(&alice, &bob).await;
    let main = stamp::main_branch();
    assert!(bob.with_song(|s| s.crew().is_member(&bob.id())).unwrap());
    assert_eq!(bob.with_song(|s| s.name().to_string()).unwrap(), "Jam");
    assert_eq!(alice.peers().len(), 1);
    assert!(alice.peers()[0].connected);

    // Alice: level-1 FakeHost; a sweep becomes one Setting that Bob gets.
    let mut fa = FakeHost::new(Level::LiveControl);
    fa.add_track("drums", "Drums", LaneKind::Part, Some(comp()), 110.0);
    let mut la = LiveLoop::new(fa, FlitChunkEncoder::default(), alice.key().clone(), main);
    let evs = alice.with_song(|s| la.tick(s, 0)).unwrap().unwrap();
    alice.publish(&evs);
    la.bridge.sweep("drums", DeviceSlot(0), "thresh", 0.0, 0.8, 300);
    alice.with_song(|s| la.tick(s, 10)).unwrap().unwrap();
    let evs = alice.with_song(|s| la.tick(s, 400)).unwrap().unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind(), Kind::Setting);
    let mut brx = bob.subscribe();
    alice.publish(&evs);
    wait_for(&mut brx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&evs[0].id))).await;
    let v = bob.with_song(|s| s.main().lane(&lane("drums")).unwrap().setting(DeviceSlot(0), &"thresh".into())).unwrap();
    assert_eq!(v, Some(0.8));
    // Bob at level 0 hears it as audio
    let mut lb = LiveLoop::new(FakeHost::new(Level::PluginOnly), FlitChunkEncoder::default(), bob.key().clone(), main);
    let d = bob.with_song(|s| Receiver::apply(&mut lb.bridge, s, &evs[0].id)).unwrap();
    assert_eq!(d, Some(ReceiverDecision::HearAsAudio(lane("drums"))));

    // Chat both ways
    let mut arx = alice.subscribe();
    let m = bob.with_song(|s| s.chat(bob.key(), &main, "sounds great", None, &HostTag::plugin_only(), Time::at(1000, SongPos::from_beats(4.0)))).unwrap().unwrap();
    bob.publish(&[m.clone()]);
    wait_for(&mut arx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&m.id))).await;
    let got = alice.with_song(|s| s.all_chat()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0.name(), bob.id());
    let m2 = alice.with_song(|s| s.chat(alice.key(), &main, "thanks", Some(evs[0].id), &host(), Time::at(2000, SongPos::START))).unwrap().unwrap();
    alice.publish(&[m2.clone()]);
    wait_for(&mut brx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&m2.id))).await;
    assert_eq!(bob.with_song(|s| s.all_chat().len()).unwrap(), 2);
    assert_eq!(bob.with_song(|s| s.jump_target(&m2.id)).unwrap(), Some((main, SongPos::START)));

    // Bob saves at level 0: real Opus + FLAC chunks. Alice fetches Opus
    // first and FLAC in the background (default policy: always).
    lb.bridge.add_track("vox", "Vox", LaneKind::Part, None, 220.0);
    let cap = lb.bridge.capture("vox", SongPos::START, SongPos::from_beats(8.0)).unwrap();
    lb.push_captured(lane("vox"), cap);
    let state = dir.path().join("bob-state.json");
    std::fs::write(&state, b"{}").unwrap();
    let evs = bob.with_song(|s| lb.save_version(s, "take 1", &state, 3000, SongPos::from_beats(8.0))).unwrap().unwrap();
    let audio = evs.iter().find(|e| e.kind() == Kind::Audio).unwrap().clone();
    let Payload::Audio(a) = audio.payload() else { panic!() };
    assert_eq!(a.opus_chunks.len(), 2);
    assert_eq!(a.flac_chunks.len(), 2);
    assert_ne!(a.opus_chunks[0], a.flac_chunks[0]);
    bob.publish(&evs);
    wait_for(&mut arx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&audio.id))).await;
    let mut fetched = Vec::new();
    while fetched.len() < 5 {
        let e = wait_for(&mut arx, |e| matches!(e, NetEvent::BlobFetched { .. })).await;
        if let NetEvent::BlobFetched { hash, .. } = e {
            fetched.push(hash);
        }
    }
    // Opus arrived before FLAC
    let first_two: Vec<Hash> = fetched[..2].to_vec();
    assert!(first_two.iter().all(|h| a.opus_chunks.contains(h)), "opus first: {fetched:?}");
    assert!(alice.with_song(|s| s.missing_opus(&main).is_empty() && s.missing_flac(&main).is_empty()).unwrap());
    // and Alice can decode Bob's take losslessly from FLAC
    let clip = alice.with_song(|s| s.main().lane(&lane("vox")).unwrap().audio.values().next().unwrap().clone()).unwrap();
    let decoded = alice.with_song(|s| flit_audio::chunk::load_clip(s.blobs(), &clip).unwrap().unwrap()).unwrap();
    assert_eq!(decoded.samples.len(), 2 * 96_000 * 2);
    settle(&alice, &bob).await;
    assert_eq!(alice.with_song(|s| s.main().clone()).unwrap(), bob.with_song(|s| s.main().clone()).unwrap());

    alice.shutdown().await;
    bob.shutdown().await;
}

#[tokio::test]
async fn strangers_and_blocked_ids_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (alice, bob) = alice_and_bob(dir.path()).await;
    settle(&alice, &bob).await;
    // Carol is not in the crew: her sync attempt is refused
    let carol = node(3, "Carol").await;
    let carol_song = Song::join(dir.path().join("carol/.flit"), alice.with_song(|s| s.genesis().clone()).unwrap(), "Jam").unwrap();
    carol.open_song(carol_song);
    let mut crx = carol.subscribe();
    carol.connect_peer(&alice.addr_info()).await.unwrap();
    let e = wait_for(&mut crx, |e| matches!(e, NetEvent::PeerDown(_) | NetEvent::PeerUp(_))).await;
    assert!(matches!(e, NetEvent::PeerDown(_)), "carol must not get a session");
    assert_eq!(alice.connected_peers(), vec![bob.id()]);
    // Carol asks to join with the code; Alice blocks her instead
    let code = alice.song_code().unwrap();
    let mut arx = alice.subscribe();
    let carol_dir = dir.path().join("carol2/.flit");
    let ask = carol.request_join(&code, &carol_dir, Duration::from_secs(10));
    let decide = async {
        wait_for(&mut arx, |e| matches!(e, NetEvent::JoinRequest { .. })).await;
        alice.block(carol.id());
        alice.deny(carol.id());
    };
    let (reply, _) = tokio::join!(ask, decide);
    assert!(!matches!(reply, Ok(flit_net::proto::JoinReply::Welcome { .. })));
    assert!(alice.is_blocked(&carol.id()));
    assert!(!alice.with_song(|s| s.crew().is_member(&carol.id())).unwrap());
    // blocked: even a second ask is refused at the door
    let r = carol.request_join(&code, &carol_dir, Duration::from_secs(5)).await;
    assert!(!matches!(r, Ok(flit_net::proto::JoinReply::Welcome { .. })));

    // A forged event (Bob editing Alice's lane) is rejected on Alice's side
    let main = stamp::main_branch();
    let claim = alice.with_song(|s| s.claim_lane(alice.key(), &main, &lane("keys"), LaneKind::Part, None, &host(), Time::at(0, SongPos::START))).unwrap().unwrap();
    alice.publish(&[claim.clone()]);
    settle(&alice, &bob).await;
    let forged = bob
        .with_song(|s| {
            let thing = Thing::Lane(lane("keys"));
            let body = StampBody {
                name: bob.id(),
                act_prev: s.chains().act_head(&bob.id(), &main),
                thing: thing.clone(),
                fact_prev: s.chains().fact_heads(&thing, &main),
                branch: main,
                gate: false,
                host: HostTag::plugin_only(),
                wall: Time::at(5, SongPos::START),
                payload: Payload::Setting(SettingPayload { slot: DeviceSlot(0), plugin: comp(), param: "x".into(), value: 0.5 }),
            };
            Stamp::sign(body, bob.key())
        })
        .unwrap();
    let mut arx = alice.subscribe();
    bob.publish(&[forged.clone()]);
    let e = wait_for(&mut arx, |e| matches!(e, NetEvent::Rejected { .. })).await;
    let NetEvent::Rejected { from, reason } = e else { unreachable!() };
    assert_eq!(from, bob.id());
    assert!(reason.contains("may not change lane"), "{reason}");
    assert!(!alice.with_song(|s| s.chains().contains(&forged.id)).unwrap());
    alice.shutdown().await;
    bob.shutdown().await;
    carol.shutdown().await;
}

#[tokio::test]
async fn door_two_invite_and_quota() {
    let dir = tempfile::tempdir().unwrap();
    let alice = Node::bind(NodeConfig { quota: Quota { events_per_sec: 5.0, burst: 10.0, storage_bytes: 1 << 20 }, ..NodeConfig::new(Keypair::from_seed(1), "Alice") }.local()).await.unwrap();
    let song = Song::create(dir.path().join("alice/.flit"), alice.key(), "Jam", &host(), Time::at(0, SongPos::START)).unwrap();
    alice.open_song(song);
    let carol = node(3, "Carol").await;
    let mut crx = carol.subscribe();
    let mut arx = alice.subscribe();
    // Door 2: Alice adds Carol's ID; Carol taps Accept.
    alice.invite(&carol.addr_info(), false).await.unwrap();
    let e = wait_for(&mut crx, |e| matches!(e, NetEvent::Invite { .. })).await;
    let NetEvent::Invite { from, song, name } = e else { unreachable!() };
    assert_eq!(from, alice.id());
    assert_eq!(name, "Jam");
    assert_eq!(carol.pending_invites().len(), 1);
    carol.accept_invite(song, &dir.path().join("carol/.flit")).await.unwrap();
    wait_for(&mut arx, |e| matches!(e, NetEvent::PeerUp(_))).await;
    settle(&alice, &carol).await;
    assert!(carol.with_song(|s| s.crew().is_member(&carol.id())).unwrap());

    // Quota: Carol floods 100 chat messages; Alice stops accepting at the burst.
    let main = stamp::main_branch();
    let mut evs = Vec::new();
    for i in 0..100 {
        evs.push(carol.with_song(|s| s.chat(carol.key(), &main, &format!("spam {i}"), None, &HostTag::plugin_only(), Time::at(i, SongPos::START))).unwrap().unwrap());
    }
    carol.publish(&evs);
    wait_for(&mut arx, |e| matches!(e, NetEvent::QuotaExceeded(_))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let n = alice.with_song(|s| s.all_chat().len()).unwrap();
    assert!(n <= 10, "alice accepted {n} of the flood");
    assert!(alice.peers().iter().any(|p| p.id == carol.id() && p.over_quota));
    alice.shutdown().await;
    carol.shutdown().await;
}
