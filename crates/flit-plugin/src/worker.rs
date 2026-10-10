//! The plugin's worker thread: owns the IPC client, moves audio between the
//! rings and the agent, keeps a snapshot of agent state for the GUI, and
//! reconnects (starting the agent if needed) whenever the agent is gone.

use crate::engine::{Mode, Rings};
use crate::FlitParams;
use atomic_float::AtomicF64;
use flit_core::{LaneId, SongPos, TransportChange};
use flit_ipc::*;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct TransportAtomics {
    pub playing: AtomicBool,
    pub tempo: AtomicF64,
    pub pos_beats: AtomicF64,
}

impl TransportAtomics {
    pub fn store(&self, playing: bool, tempo: f64, pos_beats: f64) {
        self.playing.store(playing, Ordering::Relaxed);
        self.tempo.store(tempo, Ordering::Relaxed);
        self.pos_beats.store(pos_beats, Ordering::Relaxed);
    }
}

/// What the GUI reads and the worker writes (and the GUI's action queue).
#[derive(Default)]
pub struct Shared {
    pub connected: AtomicBool,
    pub transport: TransportAtomics,
    pub status: Mutex<Option<Status>>,
    pub history: Mutex<Vec<flit_core::HistoryRow>>,
    pub chat: Mutex<Vec<ChatRow>>,
    pub branches: Mutex<Vec<BranchInfo>>,
    pub songs: Mutex<Vec<(String, PathBuf)>>,
    pub friends: Mutex<Vec<Friend>>,
    pub preview: Mutex<Option<PreviewInfo>>,
    pub last_error: Mutex<Option<String>>,
    pub notices: Mutex<VecDeque<String>>,
    pub actions: Mutex<VecDeque<Request>>,
    pub connect_error: Mutex<Option<String>>,
}

impl Shared {
    pub fn act(&self, r: Request) {
        self.actions.lock().push_back(r);
    }
    pub fn notice(&self, s: impl Into<String>) {
        let mut n = self.notices.lock();
        n.push_back(s.into());
        while n.len() > 50 {
            n.pop_front();
        }
    }
}

pub struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    pub fn start(rings: Rings, shared: Arc<Shared>, params: Arc<FlitParams>, sample_rate: u32, channels: u8) -> Worker {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let thread = std::thread::Builder::new()
            .name("flit-worker".into())
            .spawn(move || run(rings, shared, params, sample_rate, channels, s2))
            .ok();
        Worker { stop, thread }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Where the agent executable is: FLIT_AGENT, then the per-user install
/// location, then PATH.
pub fn agent_exe() -> PathBuf {
    if let Ok(p) = std::env::var("FLIT_AGENT") {
        return PathBuf::from(p);
    }
    let name = if cfg!(windows) { "flit-agent.exe" } else { "flit-agent" };
    if let Some(d) = dirs::data_local_dir() {
        let p = d.join("flit").join("bin").join(name);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(name)
}

fn client_kind(mode: Mode, lane: &str) -> ClientKind {
    let lane_id = LaneId::new(if lane.trim().is_empty() { "lane" } else { lane.trim() });
    match mode {
        Mode::Master => ClientKind::Panel,
        Mode::Lane => ClientKind::Lane { lane: lane_id },
        Mode::Receiver => ClientKind::Receiver { lane: lane_id },
        Mode::TalkSend => ClientKind::TalkSend { name: lane.trim().to_string(), to: vec![] },
        Mode::TalkReceive => ClientKind::TalkReceive { from: None },
    }
}

fn run(mut rings: Rings, shared: Arc<Shared>, params: Arc<FlitParams>, sample_rate: u32, channels: u8, stop: Arc<AtomicBool>) {
    let mut backoff = Duration::from_millis(250);
    let mut buf: Vec<f32> = Vec::with_capacity(sample_rate as usize * channels as usize);
    while !stop.load(Ordering::SeqCst) {
        let mode = params.mode.value();
        let lane = params.lane.read().clone();
        let kind = client_kind(mode, &lane);
        let opts = ClientOptions { kind: kind.clone(), sample_rate, channels, agent_exe: Some(agent_exe()), connect_timeout: Duration::from_secs(5), ..ClientOptions::new(ClientKind::Cli) };
        let mut client = match Client::connect_or_start(&opts) {
            Ok(c) => c,
            Err(e) => {
                shared.connected.store(false, Ordering::SeqCst);
                *shared.connect_error.lock() = Some(e.to_string());
                // Keep draining the capture ring so it never fills up.
                while rings.capture.pop().is_ok() {}
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(3));
                continue;
            }
        };
        backoff = Duration::from_millis(250);
        shared.connected.store(true, Ordering::SeqCst);
        *shared.connect_error.lock() = None;
        shared.notice("Connected to FLit helper");
        if let Ok(Response::Status(s)) = client.request(Request::Status) {
            *shared.status.lock() = Some(s);
        }
        session(&mut client, &mut rings, &shared, &params, &mut buf, sample_rate, channels, &stop, mode, &lane);
        shared.connected.store(false, Ordering::SeqCst);
        if !stop.load(Ordering::SeqCst) {
            shared.notice("Lost the FLit helper; reconnecting…");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn session(client: &mut Client, rings: &mut Rings, shared: &Shared, params: &FlitParams, buf: &mut Vec<f32>, sample_rate: u32, channels: u8, stop: &AtomicBool, mode: Mode, lane: &str) {
    let mut last_playing = shared.transport.playing.load(Ordering::Relaxed);
    let mut last_tempo = 0.0f64;
    let mut last_status = Instant::now();
    let my_lane = LaneId::new(if lane.trim().is_empty() { "lane" } else { lane.trim() });
    let sends_lane = matches!(mode, Mode::Lane | Mode::Master);
    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = client;
            return;
        }
        if !client.is_alive() {
            return;
        }
        // mode or lane changed: reconnect with the right Hello
        if params.mode.value() != mode || params.lane.read().as_str() != lane {
            return;
        }
        let pos = SongPos::from_beats(shared.transport.pos_beats.load(Ordering::Relaxed));

        // 1. audio out
        buf.clear();
        let max = (sample_rate as usize / 10) * channels as usize; // 100 ms at most per message
        while buf.len() < max {
            match rings.capture.pop() {
                Ok(s) => buf.push(s),
                Err(_) => break,
            }
        }
        if !buf.is_empty() {
            let r = if sends_lane {
                let lane_id = if mode == Mode::Master { LaneId::new(flit_core::song::MIX_LANE) } else { my_lane.clone() };
                client.send(Request::Audio { lane: lane_id, pos, pcm: buf.clone() })
            } else if mode == Mode::TalkSend {
                client.send(Request::TalkAudio { pcm: buf.clone() })
            } else {
                Ok(())
            };
            if r.is_err() {
                return;
            }
        }

        // 2. transport (Master reports its host's transport for level 0)
        if mode == Mode::Master {
            let playing = shared.transport.playing.load(Ordering::Relaxed);
            let tempo = shared.transport.tempo.load(Ordering::Relaxed);
            if playing != last_playing {
                last_playing = playing;
                let _ = client.send(Request::Transport { change: if playing { TransportChange::Play } else { TransportChange::Stop }, pos });
            }
            if (tempo - last_tempo).abs() > 0.001 && tempo > 0.0 {
                last_tempo = tempo;
                let _ = client.send(Request::Transport { change: TransportChange::Tempo(tempo), pos });
            }
        }

        // 3. pushes
        while let Some(p) = client.try_push() {
            match p {
                Push::LaneAudio { lane: l, pcm } => {
                    let wanted = match mode {
                        Mode::Receiver => l == my_lane || l.0 == "__submix",
                        Mode::Master => l.0 == flit_core::song::MIX_LANE && params.hear_reference.value(),
                        _ => false,
                    };
                    if wanted {
                        for s in pcm {
                            let _ = rings.play.push(s);
                        }
                    }
                }
                Push::TalkAudio { pcm, .. } => {
                    if mode == Mode::TalkReceive {
                        for s in pcm {
                            let _ = rings.play.push(s);
                        }
                    }
                }
                Push::Status(s) => *shared.status.lock() = Some(s),
                Push::Row(r) => {
                    let mut h = shared.history.lock();
                    if !h.iter().any(|x| x.id == r.id) {
                        h.push(r);
                    }
                }
                Push::Chat(c) => {
                    let mut ch = shared.chat.lock();
                    if !ch.iter().any(|x| x.id == c.id) {
                        ch.push(c);
                    }
                }
                Push::JoinRequest { id, name } => shared.notice(format!("{name} ({}) asks to join", id.short_code())),
                Push::Invite { from, name, .. } => shared.notice(format!("{} invited you to \"{name}\"", from.short_code())),
                Push::Rejected { from, reason } => shared.notice(format!("Rejected from {}: {reason}", from.short_code())),
                Push::Warning(w) => shared.notice(w),
            }
        }

        // 4. GUI actions
        let actions: Vec<Request> = shared.actions.lock().drain(..).collect();
        for req in actions {
            let refresh_history = matches!(req, Request::GoBack { .. } | Request::NewBranch { .. } | Request::Undo { .. } | Request::BringIn { .. } | Request::SwitchBranch { .. } | Request::SaveVersion { .. } | Request::ClaimLane { .. } | Request::OpenSong { .. } | Request::NewSong { .. } | Request::ShareProject { .. } | Request::JoinWithCode { .. } | Request::AcceptInvite { .. });
            match client.request_timeout(req, Duration::from_secs(130)) {
                Ok(Response::Error(e)) => *shared.last_error.lock() = Some(e),
                Ok(Response::Status(s)) => *shared.status.lock() = Some(s),
                Ok(Response::History(h)) => *shared.history.lock() = h,
                Ok(Response::Chat(c)) => *shared.chat.lock() = c,
                Ok(Response::Branches(l)) => *shared.branches.lock() = l,
                Ok(Response::Songs(s)) => *shared.songs.lock() = s,
                Ok(Response::Friends(f)) => *shared.friends.lock() = f,
                Ok(Response::Preview(p)) => *shared.preview.lock() = Some(p),
                Ok(Response::Code(c)) => shared.notice(format!("Song code: {c}")),
                Ok(_) => {}
                Err(e) => {
                    *shared.last_error.lock() = Some(e.to_string());
                    return;
                }
            }
            if refresh_history {
                if let Ok(Response::Status(s)) = client.request(Request::Status) {
                    let branch = s.song.as_ref().map(|x| x.branch).unwrap_or_else(flit_core::stamp::main_branch);
                    *shared.status.lock() = Some(s);
                    if let Ok(Response::History(h)) = client.request(Request::History { branch }) {
                        *shared.history.lock() = h;
                    }
                    if let Ok(Response::Branches(l)) = client.request(Request::Branches) {
                        *shared.branches.lock() = l;
                    }
                }
            }
        }

        // 5. periodic status
        if last_status.elapsed() > Duration::from_secs(3) {
            last_status = Instant::now();
            match client.request_timeout(Request::Status, Duration::from_secs(5)) {
                Ok(Response::Status(s)) => *shared.status.lock() = Some(s),
                Err(_) => return,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
