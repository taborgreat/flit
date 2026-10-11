//! Two FLit instances on one box, each on FakeHost (one level 1, one level
//! 0), join by song code, stay in sync by ACT and MOMENT, chat crosses,
//! blobs flow Opus-first then FLAC, blocked IDs are refused, quotas bite,
//! the keeper's seals reach everyone.
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
fn track(s: &str) -> TrackId {
    TrackId::new(s)
}
fn host() -> HostTag {
    HostTag::new("fake")
}
fn stamper(seed: u64, being: &str) -> Stamper {
    Stamper::new(Keypair::from_seed(seed), being, host(), "0.1.0", &format!("m{seed}"))
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

async fn node(seed: u64, name: &str) -> (Node, Stamper) {
    let who = stamper(seed, &name.to_lowercase());
    let n = Node::bind(NodeConfig::new(Keypair::from_seed(seed), name).local()).await.unwrap();
    n.set_stamper(who.clone());
    (n, who)
}

/// Alice creates a song and shares the code; Bob joins with it.
async fn alice_and_bob(dir: &std::path::Path) -> (Node, Stamper, Node, Stamper) {
    let (alice, a) = node(1, "Alice").await;
    let (bob, b) = node(2, "Bob").await;
    let mut song = Song::create(dir.join("alice/.flit"), &a, "Jam", 0).unwrap();
    song.declare_being(&a, "alice", BeingKind::Human, "Alice", 0x5B8DEF, None, vec![], 1).unwrap();
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
    (alice, a, bob, b)
}

fn tape(n: &Node, h: &HistoryId) -> Vec<(ActId, bool)> {
    n.with_song(|s| s.history(h).into_iter().map(|r| (r.act, r.sealed)).collect()).unwrap()
}

async fn settle(alice: &Node, bob: &Node) {
    let main = main_history();
    for _ in 0..200 {
        let a = tape(alice, &main);
        let b = tape(bob, &main);
        let unsealed = alice.with_song(|s| s.unsealed_count()).unwrap() + bob.with_song(|s| s.unsealed_count()).unwrap();
        if a == b && unsealed == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("copies did not converge");
}

#[tokio::test]
async fn join_sync_chat_and_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let (alice, a, bob, b) = alice_and_bob(dir.path()).await;
    settle(&alice, &bob).await;
    let main = main_history();
    assert!(bob.with_song(|s| s.crew().is_member(&bob.id())).unwrap());
    assert_eq!(bob.with_song(|s| s.name().to_string()).unwrap(), "Jam");
    assert_eq!(alice.peers().len(), 1);
    assert!(alice.peers()[0].connected);
    assert!(alice.with_song(|s| s.is_keeper()).unwrap(), "the owner's machine seals");
    assert!(!bob.with_song(|s| s.is_keeper()).unwrap());

    // Bob declares his being (an act of his, sealed by Alice's keeper)
    let mut arx = alice.subscribe();
    let mut brx = bob.subscribe();
    let decl = bob.with_song(|s| s.declare_being(&b, "bob", BeingKind::Human, "Bob", 0xE07A5F, None, vec![], 2)).unwrap().unwrap();
    bob.publish();
    wait_for(&mut arx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&decl.id))).await;
    wait_for(&mut brx, |e| matches!(e, NetEvent::Sealed { ids, .. } if ids.contains(&decl.id))).await;
    assert!(bob.with_song(|s| s.chains().is_sealed(&decl.id)).unwrap(), "the keeper's seal came back to Bob");

    // Alice: level-1 FakeHost; a sweep becomes one Setting that Bob gets.
    let mut fa = FakeHost::new(Level::LiveControl);
    fa.add_track("drums", "Drums", TrackKind::Part, Some(comp()), 110.0);
    let mut la = LiveLoop::new(fa, FlitChunkEncoder::default(), a.clone(), main);
    alice.with_song(|s| la.tick(s, 0)).unwrap().unwrap();
    la.bridge.sweep("drums", DeviceSlot(0), "thresh", 0.0, 0.8, 300);
    alice.with_song(|s| la.tick(s, 10)).unwrap().unwrap();
    let acts = alice.with_song(|s| la.tick(s, 400)).unwrap().unwrap();
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].kind(), Kind::Setting);
    alice.publish();
    wait_for(&mut brx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&acts[0].id))).await;
    let v = bob.with_song(|s| s.main().track(&track("drums")).unwrap().setting(DeviceSlot(0), &"thresh".into())).unwrap();
    assert_eq!(v, Some(0.8));
    assert!(bob.with_song(|s| s.chains().is_sealed(&acts[0].id)).unwrap(), "a MOMENT arrived, not just an act");
    // Bob at level 0 hears it as audio
    let mut lb = LiveLoop::new(FakeHost::new(Level::PluginOnly), FlitChunkEncoder::default(), b.clone(), main);
    let d = bob.with_song(|s| Receiver::apply(&mut lb.bridge, s, &acts[0].id)).unwrap();
    assert_eq!(d, Some(ReceiverDecision::HearAsAudio(track("drums"))));

    // Chat both ways
    let m = bob.with_song(|s| s.chat(&b, main, "sounds great", None, 1000)).unwrap().unwrap();
    bob.publish();
    wait_for(&mut arx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&m.id))).await;
    let got = alice.with_song(|s| s.all_chat()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0.name(), bob.id());
    let m2 = alice.with_song(|s| s.chat(&a, main, "thanks", Some(acts[0].id), 2000)).unwrap().unwrap();
    alice.publish();
    wait_for(&mut brx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&m2.id))).await;
    assert_eq!(bob.with_song(|s| s.all_chat().len()).unwrap(), 2);
    assert_eq!(bob.with_song(|s| s.jump_target(&m2.id)).unwrap(), Some((main, acts[0].id, acts[0].pos())));

    // Bob saves at level 0: real Opus + FLAC chunks. Alice fetches Opus
    // first and FLAC in the background (default policy: always).
    lb.bridge.add_track("vox", "Vox", TrackKind::Part, None, 220.0);
    let cap = lb.bridge.capture("vox", SongPos::START, SongPos::from_beats(8.0)).unwrap();
    lb.push_captured(track("vox"), cap);
    let state = dir.path().join("bob-state.json");
    std::fs::write(&state, b"{}").unwrap();
    let acts = bob.with_song(|s| lb.save_version(s, "take 1", &state, 3000, SongPos::from_beats(8.0))).unwrap().unwrap();
    let audio = acts.iter().find(|e| e.kind() == Kind::Audio).unwrap().clone();
    let Payload::Audio(au) = audio.payload() else { panic!() };
    assert_eq!(au.opus_chunks.len(), 2);
    assert_eq!(au.flac_chunks.len(), 2);
    assert_ne!(au.opus_chunks[0], au.flac_chunks[0]);
    bob.publish();
    wait_for(&mut arx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&audio.id))).await;
    // the Version's project blob fetches in parallel; among the audio
    // chunks, Opus arrives before FLAC
    let mut fetched: Vec<Hash> = Vec::new();
    while fetched.len() < 4 {
        let e = wait_for(&mut arx, |e| matches!(e, NetEvent::BlobFetched { .. })).await;
        if let NetEvent::BlobFetched { hash, .. } = e {
            if au.opus_chunks.contains(&hash) || au.flac_chunks.contains(&hash) {
                fetched.push(hash);
            }
        }
    }
    let first_two: Vec<Hash> = fetched[..2].to_vec();
    assert!(first_two.iter().all(|h| au.opus_chunks.contains(h)), "opus first: {fetched:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(alice.with_song(|s| s.missing_opus(&main).is_empty() && s.missing_flac(&main).is_empty()).unwrap());
    // and Alice can decode Bob's take losslessly from FLAC
    let clip = alice.with_song(|s| s.main().track(&track("vox")).unwrap().audio.values().next().unwrap().clone()).unwrap();
    let decoded = alice.with_song(|s| flit_audio::chunk::load_clip(s.blobs(), &clip).unwrap().unwrap()).unwrap();
    assert_eq!(decoded.samples.len(), 2 * 96_000 * 2);
    settle(&alice, &bob).await;
    assert_eq!(alice.with_song(|s| s.main().clone()).unwrap(), bob.with_song(|s| s.main().clone()).unwrap());

    // A reconnecting peer catches up by heads: Bob stamps while Alice is
    // away, then asks for what he missed.
    let late = bob.with_song(|s| s.chat(&b, main, "while away", None, 4000)).unwrap().unwrap();
    bob.publish();
    wait_for(&mut brx, |e| matches!(e, NetEvent::Sealed { ids, .. } if ids.contains(&late.id))).await;
    bob.catch_up();
    settle(&alice, &bob).await;

    alice.shutdown().await;
    bob.shutdown().await;
}

#[tokio::test]
async fn strangers_and_blocked_ids_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (alice, _a, bob, b) = alice_and_bob(dir.path()).await;
    settle(&alice, &bob).await;
    // Carol is not in the crew: her sync attempt is refused
    let (carol, _c) = node(3, "Carol").await;
    let (genesis, gf) = alice.with_song(|s| (s.genesis().clone(), s.chains().fact_for(&s.genesis().id).cloned())).unwrap();
    let carol_song = Song::join(dir.path().join("carol/.flit"), genesis, gf, "Jam", 0).unwrap();
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

    // A forged act (Bob, not an owner, approving Carol) is rejected on
    // Alice's side and never applied
    let main = main_history();
    let forged = bob
        .with_song(|s| {
            let being = b.being_ref();
            let (prev, seq) = s.chains().act_head(&being, &main).map(|(id, n)| (Some(id), n + 1)).unwrap_or((None, 1));
            let body = ActBody {
                name: b.name,
                being: b.being.clone(),
                act_prev: prev,
                act_seq: seq,
                thing: Thing::Song,
                property: None,
                based_on: None,
                history: main,
                app_version: "0.1.0".into(),
                format_version: FORMAT_VERSION,
                host: HostTag::plugin_only(),
                gate: false,
                payload: Payload::Crew(CrewAction::Approve { id: carol.id(), owner: false }),
                pos: None,
                stamped_at: 5,
            };
            Act::sign(body, &b.key)
        })
        .unwrap();
    let mut arx = alice.subscribe();
    // hand it straight to Alice's node as if Bob's node had sent it
    let _ = flit_net::node::inject_for_test(&alice, bob.id(), forged.clone());
    let e = wait_for(&mut arx, |e| matches!(e, NetEvent::Rejected { .. })).await;
    let NetEvent::Rejected { from, reason } = e else { unreachable!() };
    assert_eq!(from, bob.id());
    assert!(reason.contains("not an owner"), "{reason}");
    assert!(!alice.with_song(|s| s.chains().contains(&forged.id)).unwrap());
    alice.shutdown().await;
    bob.shutdown().await;
    carol.shutdown().await;
}

#[tokio::test]
async fn door_two_invite_and_quota() {
    let dir = tempfile::tempdir().unwrap();
    let a = stamper(1, "alice");
    let alice = Node::bind(NodeConfig { quota: Quota { events_per_sec: 5.0, burst: 10.0, storage_bytes: 1 << 20 }, ..NodeConfig::new(Keypair::from_seed(1), "Alice") }.local()).await.unwrap();
    alice.set_stamper(a.clone());
    let song = Song::create(dir.path().join("alice/.flit"), &a, "Jam", 0).unwrap();
    alice.open_song(song);
    let (carol, c) = node(3, "Carol").await;
    let mut crx = carol.subscribe();
    let mut arx = alice.subscribe();
    // Door 2: Alice adds Carol's ID; Carol taps Accept.
    alice.invite(&carol.addr_info(), false).await.unwrap();
    let e = wait_for(&mut crx, |e| matches!(e, NetEvent::Invite { .. })).await;
    let NetEvent::Invite { from, song, name, kind } = e else { unreachable!() };
    assert_eq!(from, alice.id());
    assert_eq!(name, "Jam");
    assert_eq!(kind, ContainerKind::Song);
    assert_eq!(carol.pending_invites().len(), 1);
    carol.accept_invite(song, &dir.path().join("carol/.flit")).await.unwrap();
    wait_for(&mut arx, |e| matches!(e, NetEvent::PeerUp(_))).await;
    settle(&alice, &carol).await;
    assert!(carol.with_song(|s| s.crew().is_member(&carol.id())).unwrap());

    // Quota: Carol floods 100 chat messages; Alice stops accepting at the burst.
    let main = main_history();
    for i in 0..100 {
        carol.with_song(|s| s.chat(&c, main, &format!("spam {i}"), None, i)).unwrap().unwrap();
    }
    carol.publish();
    wait_for(&mut arx, |e| matches!(e, NetEvent::QuotaExceeded(_))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let n = alice.with_song(|s| s.all_chat().len()).unwrap();
    assert!(n <= 10, "alice accepted {n} of the flood");
    assert!(alice.peers().iter().any(|p| p.id == carol.id() && p.over_quota));
    alice.shutdown().await;
    carol.shutdown().await;
}

/// PACK 2: an org is a container synced alongside the song. Alice invites
/// Bob to her org; Bob accepts (his act, sealed by the org keeper) and is a
/// member on both sides. Alice's song has access = org, so Bob gets in by
/// code without an approval and his acts are sealed; private closes it.
#[tokio::test]
async fn org_invite_accept_and_song_access() {
    let dir = tempfile::tempdir().unwrap();
    let (alice, a) = node(1, "Alice").await;
    let (bob, b) = node(2, "Bob").await;
    let org = Song::create_kind(dir.path().join("alice-org"), &a, "Band", ContainerKind::Org, 0).unwrap();
    let org_id = org.id();
    alice.add_container(org);
    let mut song = Song::create(dir.path().join("alice/.flit"), &a, "Jam", 0).unwrap();
    song.set_org(&a, Some(org_id), Access::Org, 1).unwrap();
    alice.open_song(song);
    assert_eq!(alice.containers_of(ContainerKind::Org), vec![org_id]);
    let mut arx = alice.subscribe();
    let mut brx = bob.subscribe();

    // invite: an act on the org, and a knock on Bob's door
    alice.invite_to(&org_id, &bob.addr_info(), false).await.unwrap();
    let e = wait_for(&mut brx, |e| matches!(e, NetEvent::Invite { .. })).await;
    let NetEvent::Invite { from, song: invited_to, name, kind } = e else { unreachable!() };
    assert_eq!((from, invited_to, name.as_str(), kind), (alice.id(), org_id, "Band", ContainerKind::Org));
    bob.accept_invite_container(org_id, &dir.path().join("bob-org")).await.unwrap();
    wait_for(&mut arx, |e| matches!(e, NetEvent::PeerUp(_))).await;
    // invited, not a member, until Bob's own Accept
    for _ in 0..100 {
        if bob.with_container(&org_id, |s| s.crew().is_invited(&bob.id())).unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(bob.with_container(&org_id, |s| s.crew().is_invited(&bob.id()) && !s.crew().is_member(&bob.id())).unwrap());
    let accept = bob.with_container(&org_id, |s| s.accept(&b, 2)).unwrap().unwrap();
    bob.publish();
    wait_for(&mut arx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&accept.id))).await;
    wait_for(&mut brx, |e| matches!(e, NetEvent::Sealed { ids, .. } if ids.contains(&accept.id))).await;
    assert!(alice.with_container(&org_id, |s| s.crew().is_member(&bob.id())).unwrap());
    assert!(bob.with_container(&org_id, |s| s.crew().is_member(&bob.id()) && s.chains().is_sealed(&accept.id)).unwrap());

    // the song: not in its crew, but the org opens it
    let code = alice.song_code().unwrap();
    let reply = bob.request_join(&code, &dir.path().join("bob/.flit"), Duration::from_secs(10)).await.unwrap();
    assert!(matches!(reply, flit_net::proto::JoinReply::Welcome { .. }), "org members get in without an approval");
    settle(&alice, &bob).await;
    assert!(!bob.with_song(|s| s.crew().is_member(&bob.id())).unwrap(), "still not in the song's own crew");
    assert_eq!(bob.with_song(|s| s.access()).unwrap(), Access::Org);
    let m = bob.with_song(|s| s.chat(&b, main_history(), "hi from the org", None, 3)).unwrap().unwrap();
    bob.publish();
    wait_for(&mut brx, |e| matches!(e, NetEvent::Sealed { ids, .. } if ids.contains(&m.id))).await;
    assert!(alice.with_song(|s| s.chains().is_sealed(&m.id)).unwrap());

    // private again: Bob's next act is refused on his own copy already
    let closed = alice.with_song(|s| s.set_org(&a, Some(org_id), Access::Private, 4)).unwrap().unwrap();
    alice.publish();
    wait_for(&mut brx, |e| matches!(e, NetEvent::Imported { ids, .. } if ids.contains(&closed.id))).await;
    assert!(bob.with_song(|s| s.chat(&b, main_history(), "closed?", None, 5)).unwrap().is_err());
    assert!(bob.with_song(|s| s.chains().contains(&m.id)).unwrap(), "what landed stays");

    // an org album with a cover: the blob is fetched into Bob's org store
    let png = vec![0x89u8, b'P', b'N', b'G', 1, 2, 3, 4, 5, 6, 7, 8];
    let (album, cover) = alice
        .with_container(&org_id, |s| {
            let album = s.create_album(&a, "EP", 6).unwrap();
            let cover = s.blobs().put(&png).unwrap();
            s.album(&a, album, AlbumAction::SetCover(Some(cover)), 7).unwrap();
            (album, cover)
        })
        .unwrap();
    alice.publish();
    wait_for(&mut brx, |e| matches!(e, NetEvent::BlobFetched { hash, .. } if *hash == cover)).await;
    assert!(bob.with_container(&org_id, |s| s.albums()[&album].cover == Some(cover) && s.blobs().get(&cover).ok() == Some(png.clone())).unwrap());
    alice.shutdown().await;
    bob.shutdown().await;
}
