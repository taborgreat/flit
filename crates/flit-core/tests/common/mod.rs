#![allow(dead_code)]
//! Test harness: Names, beings, a little network that carries ACT and
//! MOMENT between songs in whatever order a test asks for.

use flit_core::*;
use std::path::Path;

pub fn host() -> HostTag {
    HostTag::new("test")
}

pub fn comp() -> PluginIdentity {
    PluginIdentity::vst3("Acme", "Comp", "1.2.0")
}

pub fn track(s: &str) -> TrackId {
    TrackId::new(s)
}

pub fn p7() -> ParamId {
    ParamId::from_u32(7)
}

pub fn main() -> HistoryId {
    main_history()
}

/// A being on the Name's own machine, FLit 0.1.0.
pub fn stamper(seed: u64, being: &str) -> Stamper {
    Stamper::new(Keypair::from_seed(seed), being, host(), "0.1.0", &format!("machine-{seed}"))
}

pub fn note(bar: i64, key: u8) -> Note {
    Note { start: SongPos::from_bar_beat(bar, 0, TimeSig::default()), length_ticks: TICKS_PER_BEAT, key, velocity: 100, channel: 0 }
}

/// A new song owned by `who`, with `who`'s being declared.
pub fn new_song(dir: &Path, who: &Stamper, display: &str, now: u64) -> Song {
    let mut s = Song::create(dir, who, "Test song", now).unwrap();
    s.declare_being(who, &who.being.0, BeingKind::Human, display, 0x5B8DEF, None, vec![], now).unwrap();
    s
}

/// Deterministic shuffle (LCG), so "a different arrival order" is repeatable.
pub fn shuffle<T>(v: &mut [T], seed: u64) {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    for i in (1..v.len()).rev() {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (x >> 33) as usize % (i + 1);
        v.swap(i, j);
    }
}

/// Songs on different machines, plus the wire between them.
pub struct Net {
    pub songs: Vec<Song>,
    /// Peers that do not receive anything right now (offline).
    pub offline: Vec<bool>,
    /// What each peer still has to send.
    queued: Vec<Vec<Outbound>>,
    pub now: u64,
}

impl Net {
    pub fn new(songs: Vec<Song>) -> Self {
        let n = songs.len();
        Self { songs, offline: vec![false; n], queued: vec![Vec::new(); n], now: 1_000 }
    }

    pub fn tick(&mut self) -> u64 {
        self.now += 1;
        self.now
    }

    pub fn song(&mut self, i: usize) -> &mut Song {
        &mut self.songs[i]
    }

    /// A late peer joins the wire.
    pub fn add(&mut self, song: Song) -> usize {
        self.songs.push(song);
        self.offline.push(false);
        self.queued.push(Vec::new());
        self.songs.len() - 1
    }

    fn collect(&mut self) {
        for i in 0..self.songs.len() {
            let out = self.songs[i].take_outbox();
            self.queued[i].extend(out);
        }
    }

    fn deliver_one(song: &mut Song, item: &Outbound, now: u64) {
        let _ = match item {
            Outbound::Act(a) => song.import_act(a.clone(), now),
            Outbound::Moment(a, f) => song.import_moment(a.clone(), f.clone(), now),
        };
    }

    /// Carry everything queued to every online peer (sender's order).
    /// Repeats until nothing is left (keeper seals cause new moments).
    pub fn deliver_all(&mut self) {
        self.deliver_with(None);
    }

    /// Same, but each recipient gets each batch in a shuffled order.
    pub fn deliver_shuffled(&mut self, seed: u64) {
        self.deliver_with(Some(seed));
    }

    fn deliver_with(&mut self, seed: Option<u64>) {
        for _round in 0..64 {
            self.collect();
            let n = self.songs.len();
            let mut any = false;
            let now = self.now;
            for i in 0..n {
                let batch = std::mem::take(&mut self.queued[i]);
                if batch.is_empty() {
                    continue;
                }
                let mut undelivered: Vec<Outbound> = Vec::new();
                for j in 0..n {
                    if j == i {
                        continue;
                    }
                    if self.offline[j] || self.offline[i] {
                        if undelivered.is_empty() {
                            undelivered = batch.clone();
                        }
                        continue;
                    }
                    any = true;
                    let mut order: Vec<usize> = (0..batch.len()).collect();
                    if let Some(s) = seed {
                        shuffle(&mut order, s.wrapping_add(j as u64 * 7919));
                    }
                    for k in order {
                        Self::deliver_one(&mut self.songs[j], &batch[k], now);
                    }
                }
                if !undelivered.is_empty() {
                    self.queued[i] = undelivered;
                }
            }
            if !any {
                break;
            }
        }
    }

    /// A late peer fetches every moment it lacks from `from` with MOMENT
    /// only ("moments after head X"), then unsealed acts after act heads.
    pub fn catch_up(&mut self, from: usize, to: usize) {
        let now = self.now;
        let histories: Vec<HistoryId> = self.songs[from].histories().iter().map(|h| h.id).collect();
        // histories must come in creation order: main first, then those whose
        // History act is already here
        let mut done = std::collections::HashSet::new();
        for _ in 0..8 {
            for h in &histories {
                if done.contains(h) || !self.songs[to].chains().has_history(h) {
                    continue;
                }
                let have = self.songs[to].chains().seal_seq(h);
                let items: Vec<(Act, Fact)> = self.songs[from].chains().moments_after(h, have).into_iter().map(|(f, a)| (a.clone(), f.clone())).collect();
                for (a, f) in items {
                    let _ = self.songs[to].import_moment(a, f, now);
                }
                done.insert(*h);
            }
        }
        for (being, h, len) in self.songs[from].chains().act_heads() {
            let have = self.songs[to].chains().act_head(&being, &h).map(|(_, n)| n).unwrap_or(0);
            if have < len {
                let acts: Vec<Act> = self.songs[from].chains().acts_after(&being, &h, have).into_iter().cloned().collect();
                for a in acts {
                    let _ = self.songs[to].import_act(a, now);
                }
            }
        }
    }
}

/// Owner `alice` (keeper) on machine 1, member `bob` on machine 2, member
/// `carol` on machine 3, all beings declared, everyone in sync.
pub fn crew_of_three(root: &Path) -> (Net, Stamper, Stamper, Stamper) {
    let alice = stamper(1, "alice");
    let bob = stamper(2, "bob");
    let carol = stamper(3, "carol");
    let a = new_song(&root.join("a").join(".flit"), &alice, "Alice", 1);
    let genesis = a.genesis().clone();
    let gf = a.chains().fact_for(&genesis.id).cloned();
    let b = Song::join(root.join("b").join(".flit"), genesis.clone(), gf.clone(), "Test song", 1).unwrap();
    let c = Song::join(root.join("c").join(".flit"), genesis, gf, "Test song", 1).unwrap();
    let mut net = Net::new(vec![a, b, c]);
    let now = net.tick();
    net.song(0).approve(&alice, bob.name, false, now).unwrap();
    net.song(0).approve(&alice, carol.name, false, now).unwrap();
    net.catch_up(0, 1);
    net.catch_up(0, 2);
    net.deliver_all();
    let now = net.tick();
    net.song(1).declare_being(&bob, "bob", BeingKind::Human, "Bob", 0xE07A5F, None, vec![], now).unwrap();
    net.song(2).declare_being(&carol, "carol", BeingKind::Human, "Carol", 0x4FB286, None, vec![], now).unwrap();
    net.deliver_all();
    (net, alice, bob, carol)
}

/// Every peer's view of a history, as the Song lens tape (without the
/// received-at witness, which differs per machine by design).
pub fn tape(song: &Song, history: &HistoryId) -> Vec<Row> {
    song.history(history).into_iter().map(|mut r| {
        r.received_at = None;
        r
    }).collect()
}

pub fn assert_same_song(net: &Net, history: &HistoryId) {
    let first = &net.songs[0];
    for (i, s) in net.songs.iter().enumerate().skip(1) {
        assert_eq!(tape(first, history), tape(s, history), "peer {i} tape differs");
        assert_eq!(first.state(history), s.state(history), "peer {i} fold differs");
    }
}
