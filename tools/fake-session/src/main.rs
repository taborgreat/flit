//! fake-session: a command-line client for a running flit-agent. Drives
//! the demo/test flows (new song, join, approve, twist FakeHost knobs,
//! save, timeline, chat, histories, merge, beings) and prints what the
//! agent says. Start an agent first:
//!   flit-agent --bridge fake --local --socket /tmp/a.sock --seed 1 --data-dir /tmp/a
//! then e.g.
//!   fake-session --socket /tmp/a.sock new "Jam" /tmp/a/song

use clap::{Parser, Subcommand};
use flit_core::{BeingId, BeingKind, BeingRef, Hash, Id, Lens, SongPos, TrackId, TransportChange, View};
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
    Histories,
    /// The tape of a history through a lens: song (default), settings, track <id>, me, team.
    Timeline { #[arg(long)] history: Option<String>, #[arg(long, default_value = "song")] lens: String, #[arg(long)] track: Option<String>, #[arg(long)] presence: bool },
    GoBack { to: String },
    History { name: String, from: String },
    Switch { history: String },
    Undo { target: String },
    Tracks,
    MergePreview { from_history: String, #[arg(long)] track: Vec<String> },
    Merge { from_history: String, #[arg(long)] track: Vec<String>, #[arg(long)] keep_from: bool },
    Chat { text: String, #[arg(long)] pin: Option<String> },
    Chats,
    Save { name: String },
    Transport { #[arg(value_parser = ["play", "stop"])] what: String, #[arg(long, default_value_t = 0.0)] beats: f64 },
    Tempo { bpm: f64 },
    /// Show in DAW: the song as it was at a moment (omit the moment to come back to now).
    Show { at: Option<String> },
    /// Move the host's transport to a beat.
    Jump { beats: f64 },
    Beings,
    ActAs { being: String },
    NewBeing { id: String, #[arg(long, default_value = "human")] kind: String, #[arg(long)] name: Option<String> },
    Grant { being: String, #[arg(long)] track: Vec<String>, #[arg(long)] revoke: bool },
    Flag { name: String, being: String, #[arg(long, default_value = "")] note: String },
    Follow { name: String, being: String, #[arg(long)] keep: bool },
    Names,
    SwitchName { id: String },
    NewName { label: String, being: String },
    RecoveryCode,
    Backup { passphrase: String, path: PathBuf },
    MoveKeeper { name: String, machine: String },
    RequireVersion { version: String },
    Updates,
    /// FakeHost: add a track with a plugin in slot 0.
    FakeTrack { track: String, name: String, #[arg(long, default_value_t = 440.0)] tone: f32, #[arg(long)] no_plugin: bool },
    FakeTwist { track: String, param: String, value: f32, #[arg(long, default_value_t = 0)] slot: u32 },
    FakeSweep { track: String, param: String, from: f32, to: f32, #[arg(long, default_value_t = 100)] steps: usize, #[arg(long, default_value_t = 0)] slot: u32 },
    FakeVolume { track: String, value: f32 },
    FakeTempo { bpm: f64 },
    FakeSave { path: PathBuf },
    FakeParam { track: String, param: String, #[arg(long, default_value_t = 0)] slot: u32 },
    FakeFocus { #[arg(value_parser = ["mixer", "arrange", "notes", "device", "browser", "other"])] view: String, #[arg(long)] track: Option<String>, #[arg(long)] slot: Option<u32> },
    /// Send a test tone as track audio for `seconds` (level 0 track plugin stand-in).
    Tone { track: String, #[arg(long, default_value_t = 2.0)] seconds: f64, #[arg(long, default_value_t = 220.0)] hz: f32 },
    /// Print pushes from the agent for `seconds`.
    Watch { #[arg(long, default_value_t = 10.0)] seconds: f64 },
    Settings { #[arg(long)] bitrate: Option<u32>, #[arg(long)] submix: Option<bool>, #[arg(long)] hear_reference: Option<bool>, #[arg(long)] download: Option<String>, #[arg(long)] relay_only: Option<bool>, #[arg(long)] presence_in_daw: Option<bool>, #[arg(long)] manifest: Option<String>, #[arg(long)] channel: Option<String> },
    Shutdown,
}

fn id(s: &str) -> anyhow::Result<Id> {
    Id::from_base32(s).ok_or_else(|| anyhow::anyhow!("bad id: {s}"))
}
fn hash(s: &str) -> anyhow::Result<Hash> {
    Hash::from_hex(s).ok_or_else(|| anyhow::anyhow!("bad hash: {s}"))
}
fn history(s: &str) -> anyhow::Result<flit_core::HistoryId> {
    if s == "main" { Ok(flit_core::main_history()) } else { hash(s) }
}
fn being(name: &str, being: &str) -> anyhow::Result<BeingRef> {
    Ok(BeingRef { name: id(name)?, being: BeingId::new(being) })
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let socket = args.socket.map(SocketName).unwrap_or_else(socket_name);
    let kind = match &args.cmd {
        Cmd::Tone { track, .. } => ClientKind::Track { track: TrackId::new(track) },
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
        Cmd::Histories => Request::Histories,
        Cmd::Timeline { history: h, lens, track, presence } => {
            let h = match h {
                Some(h) => history(&h)?,
                None => match client.request(Request::Status)? {
                    Response::Status(s) => s.song.map(|s| s.history).unwrap_or_else(flit_core::main_history),
                    _ => flit_core::main_history(),
                },
            };
            let lens = match lens.as_str() {
                "settings" => Lens::SongSettings,
                "track" => Lens::Track(TrackId::new(track.as_deref().unwrap_or("drums"))),
                "me" => match client.request(Request::Status)? {
                    Response::Status(s) => Lens::Me(BeingRef { name: s.name, being: s.being }),
                    _ => Lens::Song,
                },
                "team" => Lens::Team,
                _ => Lens::Song,
            };
            Request::Timeline { history: h, lens, show_presence: presence }
        }
        Cmd::GoBack { to } => Request::GoBack { to: hash(&to)? },
        Cmd::History { name, from } => Request::NewHistory { name, from: hash(&from)? },
        Cmd::Switch { history: h } => Request::SwitchHistory { history: history(&h)? },
        Cmd::Undo { target } => Request::Undo { target: hash(&target)? },
        Cmd::Tracks => Request::Tracks,
        Cmd::MergePreview { from_history, track } => Request::MergePreview { from_history: history(&from_history)?, tracks: track.iter().map(|t| TrackId::new(t)).collect() },
        Cmd::Merge { from_history, track, keep_from } => {
            let from = history(&from_history)?;
            let tracks: Vec<TrackId> = track.iter().map(|t| TrackId::new(t)).collect();
            let choices = match client.request(Request::MergePreview { from_history: from, tracks: tracks.clone() })? {
                Response::MergePreview(p) => p.conflicts.into_iter().map(|c| (c.thing, c.property, keep_from)).collect(),
                _ => vec![],
            };
            Request::Merge { from_history: from, tracks, choices }
        }
        Cmd::Chat { text, pin } => Request::Chat { text, pinned: pin.map(|p| hash(&p)).transpose()? },
        Cmd::Chats => Request::ChatList,
        Cmd::Save { name } => Request::SaveVersion { name },
        Cmd::Transport { what, beats } => Request::Transport { change: if what == "play" { TransportChange::Play } else { TransportChange::Stop }, pos: SongPos::from_beats(beats) },
        Cmd::Tempo { bpm } => Request::Transport { change: TransportChange::Tempo(bpm), pos: SongPos::START },
        Cmd::Show { at } => Request::ShowMoment { at: at.map(|a| hash(&a)).transpose()? },
        Cmd::Jump { beats } => Request::Jump { pos: SongPos::from_beats(beats) },
        Cmd::Beings => Request::Beings,
        Cmd::ActAs { being } => Request::ActAs { being: BeingId::new(&being) },
        Cmd::NewBeing { id, kind, name } => Request::NewBeing {
            display_name: name.unwrap_or_else(|| id.clone()),
            id: BeingId::new(&id),
            kind: match kind.as_str() {
                "ai" => BeingKind::Ai,
                "scripted" => BeingKind::Scripted,
                _ => BeingKind::Human,
            },
            color: None,
        },
        Cmd::Grant { being, track, revoke } => Request::Grant { being: BeingId::new(&being), tracks: track.iter().map(|t| TrackId::new(t)).collect(), grant: !revoke },
        Cmd::Flag { name, being: b, note } => Request::Flag { being: being(&name, &b)?, note },
        Cmd::Follow { name, being: b, keep } => Request::Follow { being: Some(being(&name, &b)?), keep },
        Cmd::Names => Request::Names,
        Cmd::SwitchName { id: i } => Request::SwitchName { id: id(&i)? },
        Cmd::NewName { label, being: b } => Request::NewName { label: label.clone(), being: BeingId::new(&b), display_name: b.clone(), kind: BeingKind::Human },
        Cmd::RecoveryCode => Request::RecoveryCode,
        Cmd::Backup { passphrase, path } => Request::BackupName { passphrase, path },
        Cmd::MoveKeeper { name, machine } => Request::MoveKeeper { name: id(&name)?, machine },
        Cmd::RequireVersion { version } => Request::SetRequiredVersion { version },
        Cmd::Updates => Request::CheckUpdates,
        Cmd::FakeTrack { track, name, tone, no_plugin } => Request::Fake(FakeCmd::AddTrack {
            track,
            name,
            kind: "part".into(),
            plugin: if no_plugin { None } else { Some(flit_core::PluginIdentity::vst3("Acme", "Comp", "1.2.0")) },
            tone_hz: tone,
        }),
        Cmd::FakeTwist { track, param, value, slot } => Request::Fake(FakeCmd::Twist { track, slot, param, value }),
        Cmd::FakeSweep { track, param, from, to, steps, slot } => Request::Fake(FakeCmd::Sweep { track, slot, param, from, to, steps }),
        Cmd::FakeVolume { track, value } => Request::Fake(FakeCmd::Volume { track, value }),
        Cmd::FakeTempo { bpm } => Request::Fake(FakeCmd::Tempo { bpm }),
        Cmd::FakeSave { path } => Request::Fake(FakeCmd::SaveProject { path }),
        Cmd::FakeParam { track, param, slot } => Request::Fake(FakeCmd::Param { track, slot, param }),
        Cmd::FakeFocus { view, track, slot } => Request::Fake(FakeCmd::Focus {
            view: match view.as_str() {
                "mixer" => View::Mixer,
                "arrange" => View::Arrange,
                "notes" => View::Notes,
                "device" => View::Device,
                "browser" => View::Browser,
                _ => View::Other,
            },
            track,
            slot,
        }),
        Cmd::Tone { track, seconds, hz } => {
            let track = TrackId::new(&track);
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
                client.send(Request::Audio { track: track.clone(), pos, pcm })?;
                std::thread::sleep(Duration::from_millis(10));
            }
            println!("sent {seconds}s of tone on {track}");
            return Ok(());
        }
        Cmd::Watch { seconds } => {
            let end = std::time::Instant::now() + Duration::from_secs_f64(seconds);
            while std::time::Instant::now() < end {
                if let Some(p) = client.push_timeout(Duration::from_millis(200)) {
                    match p {
                        Push::TrackAudio { track, pcm } => println!("audio {track}: {} samples", pcm.len()),
                        Push::TalkAudio { from, name, pcm } => println!("talk {name} from {}: {} samples", from.short_code(), pcm.len()),
                        Push::Presence(rows) => {
                            for r in rows {
                                println!("presence {} ({}) {:?} {:?}", r.display_name, r.being.being, r.presence.as_ref().map(|p| p.view), r.presence.as_ref().and_then(|p| p.track.clone()));
                            }
                        }
                        other => println!("{other:?}"),
                    }
                }
            }
            return Ok(());
        }
        Cmd::Settings { bitrate, submix, hear_reference, download, relay_only, presence_in_daw, manifest, channel } => Request::Settings(SettingsPatch {
            download_full_quality: download.map(|d| match d.as_str() {
                "on-demand" => flit_core::DownloadPolicy::OnDemand,
                "wifi" => flit_core::DownloadPolicy::WifiOnly,
                _ => flit_core::DownloadPolicy::Always,
            }),
            monitoring_bitrate: bitrate,
            submix,
            hear_reference,
            relay_only,
            show_presence_in_daw: presence_in_daw,
            update_manifest_url: manifest,
            update_channel: channel,
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
            println!("FLit agent {} | Name {} ({}) acting as {} ({:?}) | host {} level {} | {:?}", s.agent_version, s.name_label, s.short_code, s.being_label, s.being_kind, s.host, s.level, s.connection);
            if !s.bridge.is_empty() {
                println!("bridge: {}{}", s.bridge, match s.bridge_up { Some(true) => " [UP]", Some(false) => " [DOWN]", None => "" });
            }
            if let Some(p) = &s.project {
                println!("project: {}", p.display());
            }
            if let Some(p) = s.previewing {
                println!("SHOWING {p} (fake-session show  to come back to now)");
            }
            if let Some(v) = &s.update_available {
                println!("update available: {v}");
            }
            println!("friend code: {}", s.id_text);
            match &s.song {
                Some(song) => {
                    println!("song: {} ({}) {} moments ({} unsealed, {} unsent), tempo {:.1}, history {}", song.name, song.id.short(), song.moments, song.unsealed, song.unsent, song.tempo, song.history_name);
                    println!("code: {}", song.code);
                    println!("crew: {} members, owners {:?}", song.members.len(), song.owners.iter().map(|o| o.short_code()).collect::<Vec<_>>());
                    println!("keeper: {:?} (me: {}, online: {}); requires FLit {:?}{}", song.keeper.as_ref().map(|(n, m)| format!("{} on {m}", n.short_code())), song.i_am_keeper, song.keeper_online, song.required_version, song.version_mismatch.as_ref().map(|v| format!(" MISMATCH ({v})")).unwrap_or_default());
                    for t in &song.tracks {
                        println!("  track {:<14} {:<8} {:<6} {} moments, {} devices", t.track, t.name, t.kind, t.moments, t.devices);
                    }
                    for b in &song.beings {
                        println!("  being {:<10} {:<8} {:?} {}{}{} {:?}", b.display_name, b.being.being, b.kind, if b.online { "online" } else { "offline" }, if b.acting { " (acting)" } else if b.mine { " (mine)" } else { "" }, if b.revoked { " revoked" } else { "" }, b.presence.as_ref().map(|p| (p.view, p.track.clone())));
                    }
                    for h in &song.histories {
                        println!("  history {}{} {} ({} moments, {} unsealed, {} here)", if h.current { "* " } else { "  " }, h.name, h.history, h.moments, h.unsealed, h.beings.len());
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
        Response::Histories(v) => {
            for h in v {
                println!("{}{} {} parent {:?} ({} moments)", if h.current { "* " } else { "  " }, h.name, h.history, h.parent, h.moments);
            }
        }
        Response::Timeline(rows) => {
            for r in rows {
                println!("{} #{:<4} {:<10} {:<9} {:<20} {}{}{}{}", r.act, r.number, r.being.being, format!("{:?}", r.kind), r.thing, r.summary, if r.sealed { "" } else { " (unsealed)" }, if r.gate { " (gate)" } else { "" }, if r.undone { " (undone)" } else { "" });
            }
        }
        Response::Tracks(v) => {
            for t in v {
                println!("{:<14} {:<8} {:<6} {} moments, {} devices", t.track, t.name, t.kind, t.moments, t.devices);
            }
        }
        Response::Chat(v) => {
            for c in v {
                println!("[{}] {}: {}{}", c.history_name, c.from_label, c.text, c.pinned.map(|p| format!(" (pinned {})", p)).unwrap_or_default());
            }
        }
        Response::MergePreview(p) => {
            println!("{} changes from {} into {}; flac needed {}", p.acts, p.from_history, p.to_history, p.flac_needed);
            for (t, n) in &p.counts {
                println!("  {t}: {n} moments");
            }
            for c in &p.conflicts {
                println!("  conflict {}: FROM {} ({}) | TO {} ({}) -> keep {}", c.label, c.from_summary, c.from_being.being, c.to_summary, c.to_being.being, if c.keep_from { "FROM" } else { "TO" });
            }
        }
        Response::Friends(v) => {
            for f in v {
                println!("{} {}", f.name, f.id);
            }
        }
        Response::Beings(v) => {
            for b in v {
                println!("{:<10} {:<8} {:?} {} {}{}", b.display_name, b.being.being, b.kind, b.being.name.short_code(), if b.acting { "(acting) " } else { "" }, if b.revoked { "revoked" } else { "" });
            }
        }
        Response::Names(v) => {
            for n in v {
                println!("{}{} {} beings {:?}{}", if n.current { "* " } else { "  " }, n.label, n.id, n.beings.iter().map(|(b, _)| b.0.clone()).collect::<Vec<_>>(), if n.backed_up { "" } else { " (not backed up)" });
            }
        }
        Response::Stamped(e) => println!("{e}"),
        Response::History(h) => println!("{h}"),
        Response::Value(v) => println!("{v:?}"),
        Response::Text(t) => println!("{t}"),
        Response::Updates(u) => {
            println!("{} (current {}, channel {})", u.message, u.current, u.channel);
            for r in u.releases {
                println!("  {} {}{} {}", r.version, r.channel, if r.installed { " [installed]" } else { "" }, r.url);
            }
        }
        Response::Library(l) => {
            println!("songs: {}", l.songs.len());
            for s in &l.songs {
                println!("  {:<20} {} org {:?} access {:?}", s.name, s.dir.display(), s.org.map(|o| o.short()), s.access);
            }
            println!("orgs: {}", l.orgs.len());
            for o in &l.orgs {
                println!("  {} {:<20} {} members{}{} keeper {:?} songs {} albums {}", o.org.short(), o.name, o.members, if o.owner { " [owner]" } else { "" }, if o.archived { " [archived]" } else { "" }, o.keeper.as_ref().map(|(n, m)| format!("{} on {m}", n.short_code())), o.songs.len(), o.albums);
            }
            println!("invites: {}", l.invites.len());
            for i in &l.invites {
                println!("  {:?} {} \"{}\" from {} ({})", i.kind, i.container.short(), i.name, i.from_label, i.from.short_code());
            }
            if let Some((o, name)) = &l.song_org {
                println!("open song: org {} \"{name}\" access {:?} ai {:?}", o.short(), l.access, l.ai_policy);
            }
        }
        Response::OrgMembers(m) => {
            for r in m {
                println!("  {} {:<16}{}{}{} beings {:?}", r.id.short_code(), r.label, if r.owner { " owner" } else { "" }, if r.invited { " (invited)" } else { "" }, if r.connected { " online" } else { "" }, r.beings.iter().map(|b| b.1.clone()).collect::<Vec<_>>());
            }
        }
        Response::Albums(a) => {
            for r in a {
                println!("  {} [{}] {:<20}{} {} songs{}", r.album.short(), r.scope_name, r.name, if r.archived { " archived" } else { "" }, r.songs.len(), if r.can_curate { " (curate)" } else { "" });
                for s in &r.songs {
                    println!("      {} {}{}", s.song.short(), s.name.clone().unwrap_or_default(), if s.locked { " [locked]" } else { "" });
                }
            }
        }
        Response::Album(id) => println!("album {}", id.to_hex()),
        Response::Bytes(b) => println!("{} bytes", b.len()),
    }
}
