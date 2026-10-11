//! The UI side of the helper connection: `Shared` is what the screens read
//! (and the queue of requests they write); the functions below move pushes
//! and actions between an IPC `Client` and it. The plugin's worker adds
//! audio on top; the app window runs `ui_session` as is.

use atomic_float::AtomicF64;
use flit_core::{HistoryId, Lens, PresencePayload, Row};
use flit_ipc::*;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The key the panel sends its master audio under (the reference mix).
pub const REFERENCE_KEY: &str = "__reference";

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
    /// This instance is named "master" (the session link; audio thread reads it).
    pub master: AtomicBool,
    pub transport: TransportAtomics,
    pub status: Mutex<Option<Status>>,
    /// The tape of the history + lens the panel asked for last.
    pub timeline: Mutex<Vec<Row>>,
    pub timeline_key: Mutex<Option<(HistoryId, Lens, bool)>>,
    pub chat: Mutex<Vec<ChatRow>>,
    pub histories: Mutex<Vec<HistoryRow>>,
    pub tracks: Mutex<Vec<TrackInfo>>,
    pub songs: Mutex<Vec<(String, PathBuf)>>,
    pub friends: Mutex<Vec<Friend>>,
    pub beings: Mutex<Vec<BeingRow>>,
    pub names: Mutex<Vec<NameRow>>,
    pub merge: Mutex<Option<MergeInfo>>,
    pub updates: Mutex<Option<UpdateInfo>>,
    pub text: Mutex<Option<String>>,
    pub last_error: Mutex<Option<String>>,
    pub notices: Mutex<VecDeque<String>>,
    pub declined: Mutex<VecDeque<(flit_core::ActId, HistoryId, String, String)>>,
    pub actions: Mutex<VecDeque<Request>>,
    pub connect_error: Mutex<Option<String>>,
    /// The panel's own presence (what it looks at); sent when it changes.
    pub presence: Mutex<Option<PresencePayload>>,
    /// The helper asked for the window (a plugin's [Open FLit], a second
    /// launch): the app shows itself and clears this.
    pub show_window: AtomicBool,
    /// The song the window was asked to open, if any.
    pub open_song: Mutex<Option<PathBuf>>,
    /// PACK 2: the Library (songs, orgs, invites, the open song's org).
    pub library: Mutex<Option<LibraryView>>,
    pub org_members: Mutex<Vec<OrgMemberRow>>,
    pub albums: Mutex<Vec<AlbumRow>>,
    /// The tape of the album asked for last (AlbumTimeline).
    pub album_timeline: Mutex<Vec<Row>>,
    /// Cover bytes by hash, as they arrive (ContainerBlob).
    pub covers: Mutex<std::collections::HashMap<flit_core::Hash, Vec<u8>>>,
    /// Hashes asked for and not answered yet.
    pub covers_wanted: Mutex<std::collections::HashSet<flit_core::Hash>>,
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
    /// Ask for the tape of a history through a lens (refreshed on pushes).
    pub fn want_timeline(&self, history: HistoryId, lens: Lens, show_presence: bool) {
        let key = (history, lens.clone(), show_presence);
        let mut k = self.timeline_key.lock();
        if k.as_ref() != Some(&key) {
            *k = Some(key);
            self.act(Request::Timeline { history, lens, show_presence });
        }
    }
}

pub fn refresh_timeline(client: &mut Client, shared: &Shared) {
    let key = shared.timeline_key.lock().clone();
    if let Some((history, lens, show_presence)) = key {
        if let Ok(Response::Timeline(rows)) = client.request_timeout(Request::Timeline { history, lens, show_presence }, Duration::from_secs(5)) {
            *shared.timeline.lock() = rows;
        }
    }
}

/// Apply one non-audio push. Returns true when the tape should be refetched.
/// Audio pushes are the caller's (the plugin feeds its rings; the app has
/// no audio path yet and drops them).
pub fn handle_push(shared: &Shared, p: Push) -> bool {
    match p {
        Push::TrackAudio { .. } | Push::TalkAudio { .. } => false,
        Push::Status(s) => {
            *shared.status.lock() = Some(s);
            false
        }
        Push::Row(r) => {
            let mut h = shared.timeline.lock();
            match h.iter_mut().find(|x| x.act == r.act) {
                Some(x) => *x = r,
                None => h.push(r),
            }
            true
        }
        Push::Chat(c) => {
            let mut ch = shared.chat.lock();
            if !ch.iter().any(|x| x.id == c.id) {
                ch.push(c);
            }
            false
        }
        Push::JoinRequest { id, name } => {
            shared.notice(format!("{name} ({}) asks to join", id.short_code()));
            false
        }
        Push::Invite { from, name, .. } => {
            shared.notice(format!("{} invited you to \"{name}\"", from.short_code()));
            false
        }
        Push::Rejected { from, reason } => {
            shared.notice(format!("Rejected from {}: {reason}", from.short_code()));
            false
        }
        Push::Warning(w) => {
            shared.notice(w);
            false
        }
        Push::Declined { act, history, history_name, property } => {
            shared.declined.lock().push_back((act, history, history_name.clone(), property.clone()));
            shared.notice(format!("Your change to {property} collided with someone else's. Saved as the history \"{history_name}\"."));
            true
        }
        Push::Presence(rows) => {
            *shared.beings.lock() = rows;
            false
        }
        Push::OpenWindow { song } => {
            *shared.open_song.lock() = song;
            shared.show_window.store(true, Ordering::SeqCst);
            false
        }
    }
}

/// Requests after which the status, tape, histories and tracks are stale.
fn refreshes(req: &Request) -> bool {
    matches!(req, Request::GoBack { .. } | Request::NewHistory { .. } | Request::Undo { .. } | Request::Merge { .. } | Request::SwitchHistory { .. } | Request::SaveVersion { .. } | Request::OpenSong { .. } | Request::NewSong { .. } | Request::ShareProject { .. } | Request::JoinWithCode { .. } | Request::AcceptInvite { .. } | Request::ActAs { .. } | Request::NewBeing { .. } | Request::SwitchName { .. } | Request::NewName { .. } | Request::ImportName { .. } | Request::ImportRecovery { .. } | Request::Grant { .. } | Request::Flag { .. } | Request::Pardon { .. } | Request::BlockBeing { .. } | Request::UnblockBeing { .. } | Request::MoveKeeper { .. } | Request::SetRequiredVersion { .. } | Request::CloseSong | Request::LeaveSong)
}

/// Requests after which the Library and the albums are stale.
fn refreshes_library(req: &Request) -> bool {
    matches!(req, Request::NewOrg { .. } | Request::OrgInvite { .. } | Request::AcceptOrgInvite { .. } | Request::DeclineInvite { .. } | Request::OrgLeave { .. } | Request::OrgRemove { .. } | Request::OrgPromote { .. } | Request::OrgDemote { .. } | Request::OrgSettings { .. } | Request::OrgMoveKeeper { .. } | Request::SetSongOrg { .. } | Request::SetAiPolicy { .. } | Request::NewSongIn { .. } | Request::NewAlbum { .. } | Request::AlbumAct { .. } | Request::AlbumCover { .. } | Request::AcceptInvite { .. } | Request::OpenSong { .. } | Request::NewSong { .. } | Request::ShareProject { .. } | Request::JoinWithCode { .. })
}

/// Send every queued GUI action and file the answers. Err = connection gone.
pub fn run_actions(client: &mut Client, shared: &Shared) -> std::io::Result<()> {
    let actions: Vec<Request> = shared.actions.lock().drain(..).collect();
    for req in actions {
        let refresh = refreshes(&req);
        let refresh_library = refreshes_library(&req);
        let album_tape = matches!(req, Request::AlbumTimeline { .. });
        let org_members = matches!(req, Request::OrgMembers { .. });
        let blob = match &req {
            Request::ContainerBlob { hash, .. } => Some(*hash),
            _ => None,
        };
        match client.request_timeout(req, Duration::from_secs(130)) {
            Ok(Response::Bytes(b)) => {
                if let Some(h) = blob {
                    shared.covers_wanted.lock().remove(&h);
                    shared.covers.lock().insert(h, b);
                }
            }
            Ok(Response::Error(e)) => *shared.last_error.lock() = Some(e),
            Ok(Response::Library(l)) => *shared.library.lock() = Some(l),
            Ok(Response::OrgMembers(m)) => *shared.org_members.lock() = m,
            Ok(Response::Albums(a)) => *shared.albums.lock() = a,
            Ok(Response::Timeline(h)) if album_tape => *shared.album_timeline.lock() = h,
            Ok(Response::Album(_)) => {}
            Ok(Response::Ok) if org_members => {}
            Ok(Response::Status(s)) => *shared.status.lock() = Some(s),
            Ok(Response::Timeline(h)) => *shared.timeline.lock() = h,
            Ok(Response::Chat(c)) => *shared.chat.lock() = c,
            Ok(Response::Histories(h)) => *shared.histories.lock() = h,
            Ok(Response::Tracks(t)) => *shared.tracks.lock() = t,
            Ok(Response::Songs(s)) => *shared.songs.lock() = s,
            Ok(Response::Friends(f)) => *shared.friends.lock() = f,
            Ok(Response::Beings(b)) => *shared.beings.lock() = b,
            Ok(Response::Names(n)) => *shared.names.lock() = n,
            Ok(Response::MergePreview(m)) => *shared.merge.lock() = Some(m),
            Ok(Response::Updates(u)) => *shared.updates.lock() = Some(u),
            Ok(Response::Text(t)) => *shared.text.lock() = Some(t),
            Ok(Response::Code(c)) => shared.notice(format!("Song code: {c}")),
            Ok(_) => {}
            Err(e) => {
                *shared.last_error.lock() = Some(e.to_string());
                return Err(e);
            }
        }
        if refresh_library {
            if let Ok(Response::Library(l)) = client.request(Request::Library) {
                *shared.library.lock() = Some(l);
            }
            if let Ok(Response::Albums(a)) = client.request(Request::Albums) {
                *shared.albums.lock() = a;
            }
        }
        if refresh {
            if let Ok(Response::Status(s)) = client.request(Request::Status) {
                let history = s.song.as_ref().map(|x| x.history).unwrap_or_else(flit_core::main_history);
                *shared.status.lock() = Some(s);
                // follow the current history in the tape unless the panel pinned another
                let mut key = shared.timeline_key.lock();
                let (lens, show) = key.as_ref().map(|(_, l, s)| (l.clone(), *s)).unwrap_or((Lens::Song, false));
                *key = Some((history, lens, show));
                drop(key);
                refresh_timeline(client, shared);
                if let Ok(Response::Histories(h)) = client.request(Request::Histories) {
                    *shared.histories.lock() = h;
                }
                if let Ok(Response::Tracks(t)) = client.request(Request::Tracks) {
                    *shared.tracks.lock() = t;
                }
            }
        }
    }
    Ok(())
}

/// Ask for the status every few seconds (a dead connection shows up here).
pub struct StatusPoll(Instant);

impl Default for StatusPoll {
    fn default() -> Self {
        StatusPoll(Instant::now())
    }
}

impl StatusPoll {
    pub fn tick(&mut self, client: &mut Client, shared: &Shared) -> std::io::Result<()> {
        if self.0.elapsed() > Duration::from_secs(3) {
            self.0 = Instant::now();
            match client.request_timeout(Request::Status, Duration::from_secs(5)) {
                Ok(Response::Status(s)) => *shared.status.lock() = Some(s),
                Err(e) => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }
}

/// First things after a connect: mark it, fetch the status, forget the tape key.
pub fn on_connected(client: &mut Client, shared: &Shared) {
    shared.connected.store(true, Ordering::SeqCst);
    *shared.connect_error.lock() = None;
    shared.notice("Connected to the FLit helper");
    if let Ok(Response::Status(s)) = client.request(Request::Status) {
        *shared.status.lock() = Some(s);
    }
    *shared.timeline_key.lock() = None;
}

/// One connected session for a UI-only client (the app window): pushes,
/// actions, status, until the connection dies or `stop` is set. `wake` is
/// called whenever something changed (the window repaints).
pub fn ui_session(client: &mut Client, shared: &Shared, stop: &AtomicBool, wake: &dyn Fn()) {
    let mut poll = StatusPoll::default();
    let mut last_presence: Option<PresencePayload> = None;
    loop {
        if stop.load(Ordering::SeqCst) || !client.is_alive() {
            return;
        }
        let mut changed = false;
        let mut timeline_dirty = false;
        while let Some(p) = client.try_push() {
            changed = true;
            timeline_dirty |= handle_push(shared, p);
        }
        if timeline_dirty {
            refresh_timeline(client, shared);
        }
        let p = shared.presence.lock().clone();
        if p != last_presence {
            if let Some(pp) = &p {
                let _ = client.send(Request::Presence { presence: pp.clone(), pos: None });
            }
            last_presence = p;
        }
        if !shared.actions.lock().is_empty() {
            changed = true;
            if run_actions(client, shared).is_err() {
                return;
            }
        }
        if poll.tick(client, shared).is_err() {
            return;
        }
        if changed {
            wake();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Keep a UI-only client connected to the helper (no agent start: the app
/// embeds its own or someone else runs one). Runs until `stop` is set.
pub fn ui_client_loop(opts: ClientOptions, shared: Arc<Shared>, stop: Arc<AtomicBool>, wake: Arc<dyn Fn() + Send + Sync>) {
    let mut backoff = Duration::from_millis(100);
    while !stop.load(Ordering::SeqCst) {
        let mut client = match Client::connect(&opts) {
            Ok(c) => c,
            Err(e) => {
                shared.connected.store(false, Ordering::SeqCst);
                *shared.connect_error.lock() = Some(e.to_string());
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(2));
                continue;
            }
        };
        backoff = Duration::from_millis(100);
        on_connected(&mut client, &shared);
        wake();
        ui_session(&mut client, &shared, &stop, wake.as_ref());
        shared.connected.store(false, Ordering::SeqCst);
        if !stop.load(Ordering::SeqCst) {
            shared.notice("Lost the FLit helper; reconnecting");
            wake();
        }
    }
}
