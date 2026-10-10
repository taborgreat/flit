//! fake-session: a command-branch client for a running flit-agent. Drives the
//! demo/test flows (new song, join, approve, twist FakeHost knobs, save,
//! history, chat) and prints what the agent says. Start an agent first:
//!   flit-agent --bridge fake --local --socket /tmp/a.sock --seed 1 --data-dir /tmp/a
//! then e.g.
//!   fake-session --socket /tmp/a.sock new "Jam" /tmp/a/song

use clap::{Parser, Subcommand};
use flit_core::{DeviceSlot, Hash, Id, LaneId, LaneKind, SongPos, TransportChange};
use flit_ipc::*;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "fake-session", about = "Talk to a flit-agent")]
struct Args {
    /// IPC socket (default: the per-user one).
    #[arg(long)]
    socket: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Status,
    New { name: String, dir: PathBuf },
    Share { name: String, dir: PathBuf, project: PathBuf },
    Open { dir: PathBuf },
    Code,
    Join { code: String, dir: PathBuf },
    Approve { id: String, #[arg(long)] owner: bool },
    Deny { id: String },
    Block { id: String },
    AddFriend { id: String, name: String },
    Friends,
    Invite { id: String },
    Accept { song: String, dir: PathBuf },
    Branches,
    History { #[arg(long)] branch: Option<String> },
    GoBack { to: String },
    Branch { name: String, from: String },
    Switch { branch: String },
    Undo { target: String },
    Lanes,
    Claim { lane: String, #[arg(long, default_value = "part")] kind: String, #[arg(long)] name: Option<String> },
    Preview { lane: String, source_branch: String },
    Bring { lane: String, source_branch: String, #[arg(long)] keep_both_as: Option<String> },
    Offer { lane: String },
    Chat { text: String, #[arg(long)] pin: Option<String> },
    Chats,
    Save { name: String },
    Transport { #[arg(value_parser = ["play", "stop"])] what: String, #[arg(long, default_value_t = 0.0)] beats: f64 },
    Tempo { bpm: f64 },
    /// Timeline Exact view: show the song as it was at a row (omit the row to come back to now).
    Show { at: Option<String> },
    /// Move the host's transport to a beat.
    Jump { beats: f64 },
    /// FakeHost: add a track with a plugin in slot 0.
    FakeTrack { lane: String, name: String, #[arg(long, default_value_t = 440.0)] tone: f32, #[arg(long)] no_plugin: bool },
    FakeTwist { lane: String, param: String, value: f32, #[arg(long, default_value_t = 0)] slot: u32 },
    FakeSweep { lane: String, param: String, from: f32, to: f32, #[arg(long, default_value_t = 100)] steps: usize, #[arg(long, default_value_t = 0)] slot: u32 },
    FakeVolume { lane: String, value: f32 },
    FakeTempo { bpm: f64 },
    FakeSave { path: PathBuf },
    FakeParam { lane: String, param: String, #[arg(long, default_value_t = 0)] slot: u32 },
    /// Send a test tone as lane audio for `seconds` (level 0 lane plugin stand-in).
    Tone { lane: String, #[arg(long, default_value_t = 2.0)] seconds: f64, #[arg(long, default_value_t = 220.0)] hz: f32 },
    /// Print pushes from the agent for `seconds`.
    Watch { #[arg(long, default_value_t = 10.0)] seconds: f64 },
    Settings { #[arg(long)] bitrate: Option<u32>, #[arg(long)] submix: Option<bool>, #[arg(long)] hear_reference: Option<bool>, #[arg(long)] download: Option<String>, #[arg(long)] relay_only: Option<bool>, #[arg(long)] name: Option<String> },
    Shutdown,
}

fn id(s: &str) -> anyhow::Result<Id> {
    Id::from_base32(s).ok_or_else(|| anyhow::anyhow!("bad id: {s}"))
}
fn hash(s: &str) -> anyhow::Result<Hash> {
    Hash::from_hex(s).ok_or_else(|| anyhow::anyhow!("bad hash: {s}"))
}
fn branch(s: &str) -> anyhow::Result<flit_core::BranchId> {
    if s == "main" { Ok(flit_core::stamp::main_branch()) } else { hash(s) }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let socket = args.socket.map(SocketName).unwrap_or_else(socket_name);
    let kind = match &args.cmd {
        Cmd::Tone { lane, .. } => ClientKind::Lane { lane: LaneId::new(lane) },
        Cmd::Watch { .. } => ClientKind::Panel,
        _ => ClientKind::Cli,
    };
    let mut client = Client::connect(&ClientOptions { socket, ..ClientOptions::new(kind) })?;
    let req = match args.cmd {
        Cmd::Status => Request::Status,
        Cmd::New { name, dir } => Request::NewSong { name, dir },
        Cmd::Share { name, dir, project } => Request::ShareProject { name, dir, project },
        Cmd::Open { dir } => Request::OpenSong { dir },
        Cmd::Code => Request::SongCode,
        Cmd::Join { code, dir } => Request::JoinWithCode { code, dir },
        Cmd::Approve { id: i, owner } => Request::Approve { id: id(&i)?, owner },
        Cmd::Deny { id: i } => Request::Deny { id: id(&i)? },
        Cmd::Block { id: i } => Request::Block { id: id(&i)? },
        Cmd::AddFriend { id, name } => Request::AddFriend { id, name },
        Cmd::Friends => Request::Friends,
        Cmd::Invite { id: i } => Request::Invite { friend: id(&i)? },
        Cmd::Accept { song, dir } => Request::AcceptInvite { song: hash(&song)?, dir },
        Cmd::Branches => Request::Branches,
        Cmd::History { branch: l } => {
            let l = match l {
                Some(l) => branch(&l)?,
                None => match client.request(Request::Status)? {
                    Response::Status(s) => s.song.map(|s| s.branch).unwrap_or_else(flit_core::stamp::main_branch),
                    _ => flit_core::stamp::main_branch(),
                },
            };
            Request::History { branch: l }
        }
        Cmd::GoBack { to } => Request::GoBack { to: hash(&to)? },
        Cmd::Branch { name, from } => Request::NewBranch { name, from: hash(&from)? },
        Cmd::Switch { branch: l } => Request::SwitchBranch { branch: branch(&l)? },
        Cmd::Undo { target } => Request::Undo { target: hash(&target)? },
        Cmd::Lanes => Request::Lanes,
        Cmd::Claim { lane, kind, name } => Request::ClaimLane {
            lane: LaneId::new(&lane),
            kind: match kind.as_str() {
                "bus" => LaneKind::Bus,
                "mix" => LaneKind::Mix,
                _ => LaneKind::Part,
            },
            name,
        },
        Cmd::Preview { lane, source_branch } => Request::BringPreview { lane: LaneId::new(&lane), source_branch: branch(&source_branch)? },
        Cmd::Bring { lane, source_branch, keep_both_as } => Request::BringIn { lane: LaneId::new(&lane), source_branch: branch(&source_branch)?, keep_both_as: keep_both_as.map(|l| LaneId::new(&l)) },
        Cmd::Offer { lane } => Request::OfferLane { lane: LaneId::new(&lane) },
        Cmd::Chat { text, pin } => Request::Chat { text, pinned: pin.map(|p| hash(&p)).transpose()? },
        Cmd::Chats => Request::ChatList,
        Cmd::Save { name } => Request::SaveVersion { name },
        Cmd::Transport { what, beats } => Request::Transport { change: if what == "play" { TransportChange::Play } else { TransportChange::Stop }, pos: SongPos::from_beats(beats) },
        Cmd::Tempo { bpm } => Request::Transport { change: TransportChange::Tempo(bpm), pos: SongPos::START },
        Cmd::Show { at } => Request::Preview { at: at.map(|a| hash(&a)).transpose()? },
        Cmd::Jump { beats } => Request::Jump { pos: SongPos::from_beats(beats) },
        Cmd::FakeTrack { lane, name, tone, no_plugin } => Request::Fake(FakeCmd::AddTrack {
            lane,
            name,
            kind: LaneKind::Part,
            plugin: if no_plugin { None } else { Some(flit_core::PluginIdentity::vst3("Acme", "Comp", "1.2.0")) },
            tone_hz: tone,
        }),
        Cmd::FakeTwist { lane, param, value, slot } => Request::Fake(FakeCmd::Twist { lane, slot, param, value }),
        Cmd::FakeSweep { lane, param, from, to, steps, slot } => Request::Fake(FakeCmd::Sweep { lane, slot, param, from, to, steps }),
        Cmd::FakeVolume { lane, value } => Request::Fake(FakeCmd::Volume { lane, value }),
        Cmd::FakeTempo { bpm } => Request::Fake(FakeCmd::Tempo { bpm }),
        Cmd::FakeSave { path } => Request::Fake(FakeCmd::SaveProject { path }),
        Cmd::FakeParam { lane, param, slot } => Request::Fake(FakeCmd::Param { lane, slot, param }),
        Cmd::Tone { lane, seconds, hz } => {
            let lane = LaneId::new(&lane);
            let sr = 48_000u32;
            let block = 480usize;
            let blocks = (seconds * sr as f64 / block as f64) as usize;
            let mut n = 0usize;
            for b in 0..blocks {
                let pcm: Vec<f32> = (0..block).flat_map(|i| {
                    let s = ((n + i) as f32 * hz * std::f32::consts::TAU / sr as f32).sin() * 0.5;
                    [s, s]
                }).collect();
                n += block;
                let pos = SongPos::from_seconds(b as f64 * block as f64 / sr as f64, 120.0);
                client.send(Request::Audio { lane: lane.clone(), pos, pcm })?;
                std::thread::sleep(Duration::from_millis(10));
            }
            println!("sent {seconds}s of tone on {lane}");
            return Ok(());
        }
        Cmd::Watch { seconds } => {
            let end = std::time::Instant::now() + Duration::from_secs_f64(seconds);
            while std::time::Instant::now() < end {
                if let Some(p) = client.push_timeout(Duration::from_millis(200)) {
                    match p {
                        Push::LaneAudio { lane, pcm } => println!("audio {lane}: {} samples", pcm.len()),
                        Push::TalkAudio { from, name, pcm } => println!("talk {name} from {}: {} samples", from.short_code(), pcm.len()),
                        other => println!("{other:?}"),
                    }
                }
            }
            return Ok(());
        }
        Cmd::Settings { bitrate, submix, hear_reference, download, relay_only, name } => Request::Settings(SettingsPatch {
            download_full_quality: download.map(|d| match d.as_str() {
                "on-demand" => flit_core::DownloadPolicy::OnDemand,
                "wifi" => flit_core::DownloadPolicy::WifiOnly,
                _ => flit_core::DownloadPolicy::Always,
            }),
            monitoring_bitrate: bitrate,
            submix,
            hear_reference,
            relay_only,
            display_name: name,
        }),
        Cmd::Shutdown => Request::Shutdown,
    };
    let resp = client.request_timeout(req, Duration::from_secs(130))?;
    print_response(resp);
    Ok(())
}

fn print_response(r: Response) {
    match r {
        Response::Ok => println!("ok"),
        Response::Pong => println!("pong"),
        Response::Error(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        Response::Status(s) => {
            println!("FLit agent {} | ID {} | host {} level {}", s.agent_version, s.short_code, s.host, s.level);
            if !s.bridge.is_empty() {
                println!("bridge: {}{}", s.bridge, match s.bridge_up { Some(true) => " [UP]", Some(false) => " [DOWN]", None => "" });
            }
            if let Some(p) = &s.project {
                println!("project: {}", p.display());
            }
            if let Some(p) = s.previewing {
                println!("SHOWING {p} (fake-session show  to come back to now)");
            }
            println!("friend code: {}", s.id_text);
            match &s.song {
                Some(song) => {
                    println!("song: {} ({}) {} events, tempo {:.1}, branch {} ", song.name, song.id.short(), song.events, song.tempo, song.branch_name);
                    println!("code: {}", song.code);
                    println!("crew: {} members, owners {:?}", song.members.len(), song.owners.iter().map(|o| o.short_code()).collect::<Vec<_>>());
                    println!("mix owner: {:?} (me: {})", song.mix_owner.map(|o| o.short_code()), song.i_own_mix);
                    for l in &song.lanes {
                        println!("  lane {:<12} {:?} owner {:?}{}", l.lane, l.kind, l.owner.map(|o| o.short_code()), if l.mine { " (mine)" } else { "" });
                    }
                    if !song.heard_as_audio.is_empty() {
                        println!("heard as audio: {:?}", song.heard_as_audio);
                    }
                    println!("missing chunks: {} opus, {} flac", song.missing_opus, song.missing_flac);
                }
                None => println!("no song open"),
            }
            for p in &s.peers {
                println!("peer {} {} {}{}", p.id.short_code(), p.name.clone().unwrap_or_default(), if p.connected { "connected" } else { "offline" }, if p.over_quota { " OVER QUOTA" } else { "" });
            }
            for (id, name) in &s.pending_joins {
                println!("JOIN REQUEST from {} ({}) -> approve {}", name, id.short_code(), id);
            }
            for (from, song, name) in &s.invites {
                println!("INVITE from {} to \"{}\" -> accept {}", from.short_code(), name, song);
            }
            for w in &s.warnings {
                println!("warning: {w}");
            }
            println!("settings: {:?}", s.settings);
        }
        Response::Songs(v) => {
            for (n, d) in v {
                println!("{n}\t{}", d.display());
            }
        }
        Response::Code(c) => println!("{c}"),
        Response::Branches(v) => {
            for l in v {
                println!("{}{} {}", if l.current { "* " } else { "  " }, l.name, l.branch);
            }
        }
        Response::History(rows) => {
            for r in rows {
                println!("{} {:>4} {} {:<9} {:<10} {}{}", r.id, r.order, r.name.short_code(), format!("{:?}", r.kind), r.lane.map(|l| l.0).unwrap_or_default(), r.summary, if r.undone { " (undone)" } else { "" });
            }
        }
        Response::Lanes(v) => {
            for l in v {
                println!("{:<12} {:?} owner {:?}{}", l.lane, l.kind, l.owner.map(|o| o.short_code()), if l.mine { " (mine)" } else { "" });
            }
        }
        Response::Chat(v) => {
            for c in v {
                println!("[{}] {}: {}{}", c.branch_name, c.from_name.unwrap_or_else(|| c.from.short_code()), c.text, c.pinned.map(|p| format!(" (pinned {})", p)).unwrap_or_default());
            }
        }
        Response::Preview(p) => println!("{} changes, conflict {}, flac needed {}, allowed {}", p.changes, p.conflict, p.flac_needed, p.allowed),
        Response::Friends(v) => {
            for f in v {
                println!("{} {}", f.name, f.id);
            }
        }
        Response::Stamped(e) => println!("{e}"),
        Response::Branch(l) => println!("{l}"),
        Response::Value(v) => println!("{v:?}"),
    }
}

#[allow(dead_code)]
fn _unused(_: DeviceSlot) {}
