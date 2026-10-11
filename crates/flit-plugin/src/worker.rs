//! The plugin's worker thread: owns the IPC client, moves audio between the
//! rings and the helper, keeps `flit_ui::Shared` current for the compact
//! panel, and reconnects (starting the FLit app if needed) whenever it is
//! gone. The non-audio half (pushes, actions, status) is `flit_ui::client`.

use crate::engine::{Mode, Rings};
use crate::FlitParams;
use flit_core::{SongPos, TrackId, TransportChange};
use flit_ipc::*;
use flit_ui::client::{handle_push, on_connected, refresh_timeline, run_actions, StatusPoll, REFERENCE_KEY};
use flit_ui::compact::is_master;
pub use flit_ui::Shared;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

/// What to start when no helper answers: FLIT_AGENT, else the installed
/// FLit app (in the background: its window opens on demand), else the
/// headless helper, else whatever PATH finds.
pub fn agent_exe() -> (PathBuf, Vec<String>) {
    if let Ok(p) = std::env::var("FLIT_AGENT") {
        return (PathBuf::from(p), vec![]);
    }
    let (app, agent) = if cfg!(windows) { ("flit-app.exe", "flit-agent.exe") } else { ("flit-app", "flit-agent") };
    if let Some(d) = dirs::data_local_dir() {
        let bin = d.join("flit").join("bin");
        if bin.join(app).is_file() {
            return (bin.join(app), vec!["--background".into()]);
        }
        if bin.join(agent).is_file() {
            return (bin.join(agent), vec![]);
        }
    }
    (PathBuf::from(agent), vec![])
}

/// The track id this instance speaks for. "master" (any case) on a Track or
/// Receiver is the session link: the reference mix.
fn track_id(track: &str) -> TrackId {
    let t = track.trim();
    if t.is_empty() {
        TrackId::new("track")
    } else if is_master(t) {
        TrackId::new("master")
    } else {
        TrackId::new(t)
    }
}

fn client_kind(mode: Mode, track: &str) -> ClientKind {
    match mode {
        Mode::Track => ClientKind::Track { track: track_id(track) },
        Mode::Receiver => ClientKind::Receiver { track: track_id(track) },
        Mode::TalkSend => ClientKind::TalkSend { name: track.trim().to_string(), to: vec![] },
        Mode::TalkReceive => ClientKind::TalkReceive { from: None },
    }
}

fn run(mut rings: Rings, shared: Arc<Shared>, params: Arc<FlitParams>, sample_rate: u32, channels: u8, stop: Arc<AtomicBool>) {
    let mut backoff = Duration::from_millis(250);
    let mut buf: Vec<f32> = Vec::with_capacity(sample_rate as usize * channels as usize);
    while !stop.load(Ordering::SeqCst) {
        let mode = params.mode.value();
        let track = params.track.read().clone();
        shared.master.store(is_master(&track), Ordering::Relaxed);
        let kind = client_kind(mode, &track);
        let (exe, args) = agent_exe();
        let opts = ClientOptions { kind: kind.clone(), sample_rate, channels, agent_exe: Some(exe), agent_args: args, connect_timeout: Duration::from_secs(5), ..ClientOptions::new(ClientKind::Cli) };
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
        on_connected(&mut client, &shared);
        session(&mut client, &mut rings, &shared, &params, &mut buf, sample_rate, channels, &stop, mode, &track);
        shared.connected.store(false, Ordering::SeqCst);
        if !stop.load(Ordering::SeqCst) {
            shared.notice("Lost FLit; reconnecting");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn session(client: &mut Client, rings: &mut Rings, shared: &Shared, params: &FlitParams, buf: &mut Vec<f32>, sample_rate: u32, channels: u8, stop: &AtomicBool, mode: Mode, track: &str) {
    let mut last_playing = shared.transport.playing.load(Ordering::Relaxed);
    let mut last_tempo = 0.0f64;
    let mut poll = StatusPoll::default();
    let my_track = track_id(track);
    let master = is_master(track) && matches!(mode, Mode::Track | Mode::Receiver);
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if !client.is_alive() {
            return;
        }
        // mode or track changed: reconnect with the right Hello
        if params.mode.value() != mode || params.track.read().as_str() != track {
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
            let r = match mode {
                Mode::Track => {
                    let id = if master { TrackId::new(REFERENCE_KEY) } else { my_track.clone() };
                    client.send(Request::Audio { track: id, pos, pcm: buf.clone() })
                }
                Mode::TalkSend => client.send(Request::TalkAudio { pcm: buf.clone() }),
                _ => Ok(()),
            };
            if r.is_err() {
                return;
            }
        }

        // 2. transport: Track instances report the host's transport for
        // level 0 (the helper stamps a change once, whoever says it first)
        if mode == Mode::Track {
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

        // 3. pushes: audio into the rings, the rest into Shared
        let mut timeline_dirty = false;
        while let Some(p) = client.try_push() {
            match p {
                Push::TrackAudio { track: t, pcm } => {
                    let wanted = mode == Mode::Receiver && (t == my_track || t.0 == "__submix" || (master && t.0 == REFERENCE_KEY));
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
                other => timeline_dirty |= handle_push(shared, other),
            }
        }
        if timeline_dirty {
            refresh_timeline(client, shared);
        }

        // 4. GUI actions
        if run_actions(client, shared).is_err() {
            return;
        }

        // 5. periodic status
        if poll.tick(client, shared).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
