//! A song on disk: `.flit/` beside the project, holding the spines, the
//! blob store and a little metadata. This is the API the agent and the
//! network layer use; it never knows which DAW is on the other side.
//! Users see plain words (Save version, Go back, New history from here,
//! Merge); underneath every one of them is an act (see `seal`).

use crate::act::*;
use crate::blob::{BlobStore, Hash};
use crate::id::{Id, Keypair};
use crate::plugin::{DeviceSlot, ParamId, PluginIdentity};
use crate::position::SongPos;
use crate::seal::keeper::{self, Decision, KeeperKey};
use crate::seal::lens::{self, Lens, Row};
use crate::seal::spine::Spines;
use crate::seal::{AppendOutcome, Chains, HistoryInfo};
use crate::state::SongState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// A song's public id: the id of its genesis act.
pub type SongId = Hash;

/// Who is stamping: a being of a Name, on a host, with a key that speaks
/// for it (the Name key, or a certified being key).
#[derive(Clone, Debug)]
pub struct Stamper {
    pub key: Keypair,
    pub name: Id,
    pub being: BeingId,
    pub host: HostTag,
    pub app_version: String,
    pub machine: String,
}

impl Stamper {
    /// A being on the Name's own machine: the Name key signs.
    pub fn new(key: Keypair, being: &str, host: HostTag, app_version: &str, machine: &str) -> Self {
        let name = key.id();
        Self { key, name, being: BeingId::new(being), host, app_version: app_version.to_string(), machine: machine.to_string() }
    }

    /// A being on another machine with a key its Name certified.
    pub fn delegated(key: Keypair, name: Id, being: &str, host: HostTag, app_version: &str, machine: &str) -> Self {
        Self { key, name, being: BeingId::new(being), host, app_version: app_version.to_string(), machine: machine.to_string() }
    }

    pub fn being_ref(&self) -> BeingRef {
        BeingRef { name: self.name, being: self.being.clone() }
    }

    pub fn as_being(&self, being: &str) -> Stamper {
        let mut s = self.clone();
        s.being = BeingId::new(being);
        s
    }

    pub fn keeper_key(&self) -> KeeperKey {
        KeeperKey { key: self.key.clone(), name: self.name, being: self.being.clone(), machine: self.machine.clone(), host: self.host.clone(), app_version: self.app_version.clone() }
    }
}

fn default_format() -> u32 {
    FORMAT_VERSION
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SongMeta {
    id: SongId,
    name: String,
    /// File name of the DAW project this song was made for (lives in the
    /// same folder). Lets the agent link a project to its song by itself
    /// and warn when another project file is opened in that folder.
    #[serde(default)]
    project: Option<String>,
    #[serde(default = "default_format")]
    format: u32,
}

/// Something to send: our own act (ACT) or a sealed moment (MOMENT).
#[derive(Clone, Debug)]
pub enum Outbound {
    Act(Act),
    Moment(Act, Fact),
}

/// Something to tell the user.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// A moment collided with someone else's; saved as a history.
    Declined { act: ActId, being: BeingRef, history: HistoryId, history_name: String, property: String },
    /// The keeper is holding a moment.
    Held { act: ActId, reason: String },
    /// A being stamped two moments with one number.
    Equivocation { being: BeingRef, acts: [ActId; 2] },
}

#[derive(Debug)]
pub struct Song {
    dir: PathBuf,
    chains: Chains,
    blobs: BlobStore,
    meta: SongMeta,
    keeper: Option<KeeperKey>,
    outbox: Vec<Outbound>,
    notices: Vec<Notice>,
}

/// How to put a member's host back to a moment: load the nearest Version
/// of that host at or before it, then replay these edits on top.
#[derive(Clone, Debug, PartialEq)]
pub struct RestorePlan {
    pub version: Option<Act>,
    pub replay: Vec<Act>,
    /// Sound of every track at that moment.
    pub state: SongState,
}

/// One property both sides changed since the histories parted.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub thing: Thing,
    pub property: String,
    pub from_act: ActId,
    pub to_act: ActId,
    /// true = take FROM's change, false = keep TO's. Default per the
    /// keeper rule: what TO already sealed wins.
    pub keep_from: bool,
}

/// What a Merge would do, before doing it.
#[derive(Clone, Debug)]
pub struct MergePreview {
    pub from_history: HistoryId,
    pub from_song: Option<SongId>,
    pub from_moment: ActId,
    pub to_history: HistoryId,
    /// Chosen tracks (empty = the whole history).
    pub tracks: Vec<TrackId>,
    /// Source track edits the destination does not have yet, in order.
    pub acts: Vec<Act>,
    pub conflicts: Vec<Conflict>,
    /// Moments per track since the histories parted.
    pub counts: BTreeMap<TrackId, usize>,
    /// Archive (FLAC) chunks the merge needs that are not here yet. A peer
    /// MUST pull these before the Merge lands.
    pub flac_needed: Vec<Hash>,
}

impl MergePreview {
    pub fn set_choice(&mut self, thing: &Thing, property: &str, keep_from: bool) {
        for c in &mut self.conflicts {
            if c.thing == *thing && c.property == property {
                c.keep_from = keep_from;
            }
        }
    }
    pub fn keep_all(&mut self, keep_from: bool) {
        for c in &mut self.conflicts {
            c.keep_from = keep_from;
        }
    }
}

impl Song {
    // ----- lifecycle -----

    /// Create a new song in `dir` (the `.flit` folder). The creator's Name
    /// becomes the first owner and this machine the keeper.
    pub fn create(dir: impl AsRef<Path>, who: &Stamper, name: &str, now_ms: u64) -> crate::Result<Song> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        if Spines::old_layout(&dir) {
            return Err(crate::Error::Invalid(format!("{} is a song from before the spine model; it cannot be opened", dir.display())));
        }
        let chains = Chains::open(&dir)?;
        if !chains.is_empty() {
            return Err(crate::Error::Invalid(format!("{} already holds a song", dir.display())));
        }
        let blobs = BlobStore::open(dir.join("blobs"))?;
        let meta = SongMeta { id: Hash::ZERO, name: name.to_string(), project: None, format: FORMAT_VERSION };
        let mut song = Song { dir, chains, blobs, meta, keeper: Some(who.keeper_key()), outbox: Vec::new(), notices: Vec::new() };
        let genesis = song.act(who, main_history(), Thing::Song, Payload::Crew(CrewAction::Approve { id: who.name, owner: true }), None, now_ms)?;
        song.meta.id = genesis.id;
        song.write_meta()?;
        song.act(who, main_history(), Thing::SongSettings, Payload::Keeper(KeeperPayload { name: who.name, machine: who.machine.clone() }), None, now_ms)?;
        Ok(song)
    }

    /// Open an existing song folder. The keeper key, if this machine holds
    /// the role, is set afterwards with `set_keeper`.
    pub fn open(dir: impl AsRef<Path>) -> crate::Result<Song> {
        let dir = dir.as_ref().to_path_buf();
        if Spines::old_layout(&dir) {
            return Err(crate::Error::Invalid(format!("{} is a song from before the spine model; it cannot be opened", dir.display())));
        }
        let meta: SongMeta = serde_json::from_str(&std::fs::read_to_string(dir.join("song.json"))?).map_err(|e| crate::Error::Decode(e.to_string()))?;
        if meta.format > FORMAT_VERSION {
            return Err(crate::Error::FormatTooNew(meta.format));
        }
        let chains = Chains::open(&dir)?;
        let blobs = BlobStore::open(dir.join("blobs"))?;
        Ok(Song { dir, chains, blobs, meta, keeper: None, outbox: Vec::new(), notices: Vec::new() })
    }

    /// Start a local copy of someone else's song from its genesis moment.
    /// Everything else arrives by MOMENT.
    pub fn join(dir: impl AsRef<Path>, genesis: Act, genesis_fact: Option<Fact>, name: &str, now_ms: u64) -> crate::Result<Song> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let mut chains = Chains::open(&dir)?;
        if chains.is_empty() {
            chains.append_act(genesis.clone(), Some(now_ms))?;
            if let Some(f) = genesis_fact {
                chains.append_fact(f)?;
            }
        } else if chains.song_id() != Some(genesis.id) {
            return Err(crate::Error::Invalid("folder holds a different song".into()));
        }
        let meta = SongMeta { id: genesis.id, name: name.to_string(), project: None, format: FORMAT_VERSION };
        std::fs::write(dir.join("song.json"), serde_json::to_string_pretty(&meta).unwrap())?;
        let blobs = BlobStore::open(dir.join("blobs"))?;
        Ok(Song { dir, chains, blobs, meta, keeper: None, outbox: Vec::new(), notices: Vec::new() })
    }

    fn write_meta(&self) -> crate::Result<()> {
        std::fs::write(self.dir.join("song.json"), serde_json::to_string_pretty(&self.meta).unwrap())?;
        Ok(())
    }

    pub fn exists(dir: impl AsRef<Path>) -> bool {
        dir.as_ref().join("song.json").is_file()
    }

    /// The project file (name only) this song belongs to, if recorded.
    pub fn project_file(&self) -> Option<&str> {
        self.meta.project.as_deref()
    }

    /// Record which project file this song belongs to.
    pub fn set_project_file(&mut self, file_name: &str) -> crate::Result<()> {
        self.meta.project = Some(file_name.to_string());
        self.write_meta()
    }

    pub fn id(&self) -> SongId {
        self.meta.id
    }
    pub fn name(&self) -> &str {
        &self.meta.name
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn chains(&self) -> &Chains {
        &self.chains
    }
    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }
    pub fn crew(&self) -> &crate::CrewState {
        self.chains.crew()
    }
    pub fn beings(&self) -> &crate::beings::Beings {
        self.chains.beings()
    }
    pub fn genesis(&self) -> &Act {
        self.chains.genesis().expect("a song always has a genesis")
    }
    pub fn state(&self, history: &HistoryId) -> Option<&SongState> {
        self.chains.state(history)
    }
    pub fn main(&self) -> &SongState {
        self.chains.state(&main_history()).expect("main always exists")
    }
    pub fn state_at(&self, at: &ActId) -> crate::Result<SongState> {
        self.chains.state_at(at)
    }
    pub fn histories(&self) -> Vec<HistoryInfo> {
        self.chains.histories()
    }
    pub fn history_name(&self, history: &HistoryId) -> Option<&str> {
        self.chains.history_name(history)
    }
    pub fn latest(&self, history: &HistoryId) -> Option<ActId> {
        self.chains.latest(history)
    }

    // ----- keeper -----

    /// Who seals this song: (Name, machine).
    pub fn keeper(&self) -> Option<(Id, String)> {
        self.chains.keeper()
    }

    /// Hold (or drop) the keeper role on this machine. Sealing starts at
    /// once for anything waiting.
    pub fn set_keeper(&mut self, key: Option<KeeperKey>, now_ms: u64) -> crate::Result<Vec<Decision>> {
        self.keeper = key;
        self.seal_pending(now_ms)
    }

    pub fn keeper_key(&self) -> Option<&KeeperKey> {
        self.keeper.as_ref()
    }

    /// Does this machine hold the role right now?
    pub fn is_keeper(&self) -> bool {
        self.keeper.as_ref().map(|k| keeper::is_keeper(&self.chains, k)).unwrap_or(false)
    }

    /// Seal everything waiting, if this machine is the keeper.
    pub fn seal_pending(&mut self, now_ms: u64) -> crate::Result<Vec<Decision>> {
        let Some(k) = self.keeper.clone() else { return Ok(vec![]) };
        let decisions = keeper::seal_pending(&mut self.chains, &k, now_ms)?;
        for d in &decisions {
            match d {
                Decision::Sealed(f) => {
                    if let Some(a) = self.chains.get(&f.act_id()) {
                        self.outbox.push(Outbound::Moment(a.clone(), f.clone()));
                    }
                }
                Decision::Declined { fact, history, created } => {
                    if let Some((a, f)) = created {
                        self.outbox.push(Outbound::Moment(a.clone(), f.clone()));
                    }
                    if let Some(a) = self.chains.get(&fact.act_id()).cloned() {
                        self.outbox.push(Outbound::Moment(a.clone(), fact.clone()));
                        self.notices.push(Notice::Declined {
                            act: a.id,
                            being: a.being(),
                            history: *history,
                            history_name: self.chains.history_name(history).unwrap_or("").to_string(),
                            property: lens::property_label(&a),
                        });
                    }
                }
                Decision::Held { act, reason } => {
                    self.notices.push(Notice::Held { act: *act, reason: format!("{reason:?}") });
                }
            }
        }
        Ok(decisions)
    }

    /// Acts and moments to send, since the last call.
    pub fn take_outbox(&mut self) -> Vec<Outbound> {
        std::mem::take(&mut self.outbox)
    }

    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    /// Moments waiting for the keeper, song-wide.
    pub fn unsealed_count(&self) -> usize {
        self.chains.unsealed_total()
    }

    // ----- receiving -----

    /// An act that arrived from a peer (verified on the way in; applied
    /// optimistically; sealed at once if this machine is the keeper).
    pub fn import_act(&mut self, act: Act, now_ms: u64) -> crate::Result<AppendOutcome> {
        let outcome = self.chains.append_act(act, Some(now_ms))?;
        if outcome == AppendOutcome::Equivocation {
            if let Some(ev) = self.chains.evidence().last() {
                self.notices.push(Notice::Equivocation { being: ev.being.clone(), acts: ev.acts });
            }
        }
        if outcome == AppendOutcome::Appended {
            self.seal_pending(now_ms)?;
        }
        Ok(outcome)
    }

    /// A keeper's fact that arrived from a peer.
    pub fn import_fact(&mut self, fact: Fact) -> crate::Result<AppendOutcome> {
        let gate = fact.body.gate;
        let act_id = fact.act_id();
        let history = fact.history();
        let outcome = self.chains.append_fact(fact)?;
        if outcome == AppendOutcome::Appended && gate {
            if let Some(a) = self.chains.get(&act_id) {
                self.notices.push(Notice::Declined {
                    act: act_id,
                    being: a.being(),
                    history,
                    history_name: self.chains.history_name(&history).unwrap_or("").to_string(),
                    property: lens::property_label(a),
                });
            }
        }
        Ok(outcome)
    }

    /// A MOMENT: fact with its act.
    pub fn import_moment(&mut self, act: Act, fact: Fact, now_ms: u64) -> crate::Result<AppendOutcome> {
        let a = self.import_act(act, now_ms)?;
        if a == AppendOutcome::Pending || a == AppendOutcome::Equivocation {
            return Ok(a);
        }
        self.import_fact(fact)
    }

    // ----- stamping -----

    /// Stamp an act on `history`: the act goes on the being's act chain,
    /// is applied at once, and is sealed at once if this machine is the
    /// keeper. An ai / scripted being without a grant for the track lands
    /// in its own history instead.
    pub fn act(&mut self, who: &Stamper, history: HistoryId, thing: Thing, payload: Payload, pos: Option<SongPos>, now_ms: u64) -> crate::Result<Act> {
        let being = who.being_ref();
        let history = self.route(who, history, &thing, &payload, now_ms)?;
        let (act_prev, act_seq) = self.chains.act_head(&being, &history).map(|(id, n)| (Some(id), n + 1)).unwrap_or((None, 1));
        let based_on = match self.chains.fact_seq(&thing, &history) {
            0 => None,
            n => Some(n),
        };
        let body = ActBody {
            name: who.name,
            being: who.being.clone(),
            act_prev,
            act_seq,
            property: payload.property(),
            thing,
            based_on,
            history,
            app_version: who.app_version.clone(),
            format_version: FORMAT_VERSION,
            host: who.host.clone(),
            gate: payload.is_gate(),
            payload,
            pos,
            stamped_at: now_ms,
        };
        let act = Act::sign(body, &who.key);
        match self.chains.append_act(act.clone(), Some(now_ms))? {
            AppendOutcome::Appended => {}
            other => return Err(crate::Error::Invalid(format!("own act not appended: {other:?}"))),
        }
        self.outbox.push(Outbound::Act(act.clone()));
        self.seal_pending(now_ms)?;
        Ok(act)
    }

    /// Where an act lands: ai / scripted beings edit in their own history
    /// unless granted the track.
    fn route(&mut self, who: &Stamper, history: HistoryId, thing: &Thing, payload: &Payload, now_ms: u64) -> crate::Result<HistoryId> {
        let being = who.being_ref();
        if !self.chains.beings().is_ai(&being) {
            return Ok(history);
        }
        let needs_own = match payload {
            p if p.is_track_edit() => !thing.track().map(|t| self.chains.beings().granted(&being, t)).unwrap_or(false),
            Payload::Transport(_) | Payload::Version(_) | Payload::GoBack { .. } | Payload::Undo { .. } => true,
            Payload::Merge(m) => !m.acts.iter().all(|a| a.track().map(|t| self.chains.beings().granted(&being, t)).unwrap_or(false)),
            _ => false,
        };
        if !needs_own || crate::seal::verify::ai_owns_history(&self.chains, &being, &history) {
            return Ok(history);
        }
        // reuse this being's history off `history`, or start one
        let label = self.being_label(&being);
        let name = format!("{label}'s history");
        let existing = self.chains.histories().into_iter().find(|h| h.parent == Some(history) && h.created_by.as_ref() == Some(&being) && h.name == name);
        if let Some(h) = existing {
            return Ok(h.id);
        }
        let from = self.chains.latest(&history).ok_or(crate::Error::UnknownHistory(history))?;
        let act = self.act(who, history, Thing::Song, Payload::History(HistoryPayload { name, from, parent: history }), None, now_ms)?;
        Ok(act.id)
    }

    fn on_track(&mut self, who: &Stamper, history: HistoryId, track: &TrackId, payload: Payload, pos: Option<SongPos>, now_ms: u64) -> crate::Result<Act> {
        self.act(who, history, Thing::Track(track.clone()), payload, pos, now_ms)
    }

    fn on_song(&mut self, who: &Stamper, history: HistoryId, payload: Payload, now_ms: u64) -> crate::Result<Act> {
        self.act(who, history, Thing::Song, payload, None, now_ms)
    }

    fn on_settings(&mut self, who: &Stamper, history: HistoryId, payload: Payload, now_ms: u64) -> crate::Result<Act> {
        self.act(who, history, Thing::SongSettings, payload, None, now_ms)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn setting(&mut self, who: &Stamper, history: HistoryId, track: &TrackId, slot: DeviceSlot, plugin: &PluginIdentity, param: &ParamId, value: f32, pos: Option<SongPos>, now_ms: u64) -> crate::Result<Act> {
        self.act(who, history, Thing::Device { track: track.clone(), slot }, Payload::Setting(SettingPayload { plugin: plugin.clone(), param: param.clone(), value }), pos, now_ms)
    }

    pub fn mixer(&mut self, who: &Stamper, history: HistoryId, track: &TrackId, change: MixerChange, now_ms: u64) -> crate::Result<Act> {
        self.on_track(who, history, track, Payload::Mixer(change), None, now_ms)
    }

    /// A plugin placed in (`Some`) or removed from (`None`) a device slot.
    pub fn device(&mut self, who: &Stamper, history: HistoryId, track: &TrackId, slot: DeviceSlot, plugin: Option<PluginIdentity>, now_ms: u64) -> crate::Result<Act> {
        self.on_track(who, history, track, Payload::Device(DevicePayload { slot, plugin }), None, now_ms)
    }

    pub fn transport(&mut self, who: &Stamper, history: HistoryId, change: TransportChange, now_ms: u64) -> crate::Result<Act> {
        self.on_settings(who, history, Payload::Transport(change), now_ms)
    }

    pub fn tempo(&mut self, who: &Stamper, history: HistoryId, bpm: f64, now_ms: u64) -> crate::Result<Act> {
        self.transport(who, history, TransportChange::Tempo(bpm), now_ms)
    }

    /// Notes added to / removed from a clip (a delta, so parallel edits of
    /// different notes both land).
    pub fn notes(&mut self, who: &Stamper, history: HistoryId, track: &TrackId, clip: &str, add: Vec<Note>, remove: Vec<Note>, now_ms: u64) -> crate::Result<Act> {
        let pos = add.iter().chain(remove.iter()).map(|n| n.start).min();
        self.on_track(who, history, track, Payload::Notes(NotesPayload { clip: clip.to_string(), add, remove }), pos, now_ms)
    }

    pub fn audio(&mut self, who: &Stamper, history: HistoryId, track: &TrackId, audio: AudioPayload, now_ms: u64) -> crate::Result<Act> {
        let pos = Some(audio.start);
        self.on_track(who, history, track, Payload::Audio(audio), pos, now_ms)
    }

    pub fn host_state(&mut self, who: &Stamper, history: HistoryId, track: &TrackId, blob: Vec<u8>, now_ms: u64) -> crate::Result<Act> {
        let host = who.host.clone();
        self.on_track(who, history, track, Payload::HostState(HostStatePayload { host, blob }), None, now_ms)
    }

    /// Save version: the host's project file (already in the blob store)
    /// plus the files it uses.
    pub fn version(&mut self, who: &Stamper, history: HistoryId, name: &str, project: Hash, manifest: Vec<ManifestEntry>, now_ms: u64) -> crate::Result<Act> {
        let host = who.host.clone();
        self.on_song(who, history, Payload::Version(VersionPayload { name: name.to_string(), host, project, manifest }), now_ms)
    }

    /// Store a project file and record a version in one go.
    pub fn save_version_from_file(&mut self, who: &Stamper, history: HistoryId, name: &str, project_path: &Path, used_files: &[PathBuf], now_ms: u64) -> crate::Result<Act> {
        let project = self.blobs.put_file(project_path)?;
        let mut manifest = Vec::new();
        for f in used_files {
            let hash = self.blobs.put_file(f)?;
            let size = std::fs::metadata(f)?.len();
            manifest.push(ManifestEntry { path: f.to_string_lossy().into_owned(), hash, size });
        }
        self.version(who, history, name, project, manifest, now_ms)
    }

    // crew (song chain, main)
    pub fn approve(&mut self, who: &Stamper, id: Id, owner: bool, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Crew(CrewAction::Approve { id, owner }), now_ms)
    }
    /// A gate: the crew closes to `id`. Their beings' act chains stop;
    /// their facts stay.
    pub fn remove(&mut self, who: &Stamper, id: Id, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Crew(CrewAction::Remove { id }), now_ms)
    }
    pub fn promote(&mut self, who: &Stamper, id: Id, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Crew(CrewAction::Promote { id }), now_ms)
    }
    pub fn block_being(&mut self, who: &Stamper, being: BeingRef, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Crew(CrewAction::BlockBeing { being }), now_ms)
    }
    pub fn unblock_being(&mut self, who: &Stamper, being: BeingRef, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Crew(CrewAction::UnblockBeing { being }), now_ms)
    }
    /// An owner settles an equivocation: the keeper seals the being again.
    pub fn pardon(&mut self, who: &Stamper, being: BeingRef, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Crew(CrewAction::Pardon { being }), now_ms)
    }

    // beings (song chain, main; Name key only)
    #[allow(clippy::too_many_arguments)]
    pub fn declare_being(&mut self, who: &Stamper, being: &str, kind: BeingKind, display_name: &str, color: u32, key: Option<Id>, scope: Vec<Hash>, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Being(BeingPayload { being: BeingId::new(being), action: BeingAction::Create { kind, display_name: display_name.trim().to_string(), color, key, scope } }), now_ms)
    }
    pub fn update_being(&mut self, who: &Stamper, being: &str, display_name: &str, color: u32, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Being(BeingPayload { being: BeingId::new(being), action: BeingAction::Update { display_name: display_name.trim().to_string(), color } }), now_ms)
    }
    pub fn revoke_being(&mut self, who: &Stamper, being: &str, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Being(BeingPayload { being: BeingId::new(being), action: BeingAction::Revoke }), now_ms)
    }
    /// Let (or stop letting) one of your ai / scripted beings act on
    /// these tracks directly.
    pub fn grant(&mut self, who: &Stamper, being: &str, tracks: Vec<TrackId>, grant: bool, now_ms: u64) -> crate::Result<Act> {
        let being = BeingRef { name: who.name, being: BeingId::new(being) };
        self.on_song(who, main_history(), Payload::Grant(GrantPayload { being, tracks, grant }), now_ms)
    }
    pub fn flag(&mut self, who: &Stamper, being: BeingRef, kind: FlagKind, note: &str, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, main_history(), Payload::Flag(FlagPayload { being, kind, note: note.to_string() }), now_ms)
    }

    // settings (owner, Name key)
    pub fn move_keeper(&mut self, who: &Stamper, name: Id, machine: &str, now_ms: u64) -> crate::Result<Act> {
        self.on_settings(who, main_history(), Payload::Keeper(KeeperPayload { name, machine: machine.to_string() }), now_ms)
    }
    pub fn set_required_version(&mut self, who: &Stamper, version: &str, now_ms: u64) -> crate::Result<Act> {
        self.on_settings(who, main_history(), Payload::Settings(SettingsChange::RequiredVersion(version.to_string())), now_ms)
    }
    pub fn set_reference_daw(&mut self, who: &Stamper, history: HistoryId, being: BeingRef, now_ms: u64) -> crate::Result<Act> {
        self.on_settings(who, history, Payload::Settings(SettingsChange::ReferenceDaw(being)), now_ms)
    }
    pub fn set_time_signature(&mut self, who: &Stamper, history: HistoryId, beats: u8, unit: u8, now_ms: u64) -> crate::Result<Act> {
        self.on_settings(who, history, Payload::Settings(SettingsChange::TimeSignature { beats, unit }), now_ms)
    }

    // ----- histories -----

    /// New history from any moment. Returns the new history's id.
    pub fn new_history(&mut self, who: &Stamper, name: &str, from: ActId, now_ms: u64) -> crate::Result<HistoryId> {
        let parent = self.chains.history_of(&from).ok_or(crate::Error::UnknownAct(from))?;
        let act = self.on_song(who, parent, Payload::History(HistoryPayload { name: name.to_string(), from, parent }), now_ms)?;
        Ok(act.id)
    }

    /// Go back to a moment. Undoable like everything else.
    pub fn go_back(&mut self, who: &Stamper, history: HistoryId, to: ActId, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, history, Payload::GoBack { to }, now_ms)
    }

    /// Undo one moment (a Merge's whole group goes with it).
    pub fn undo(&mut self, who: &Stamper, history: HistoryId, target: ActId, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, history, Payload::Undo { target }, now_ms)
    }

    /// The tape of a history through a lens.
    pub fn rows(&self, history: &HistoryId, lens: &Lens, show_presence: bool) -> Vec<Row> {
        lens::rows(&self.chains, history, lens, show_presence)
    }

    /// The tape of a history, Song lens.
    pub fn history(&self, history: &HistoryId) -> Vec<Row> {
        self.rows(history, &Lens::Song, false)
    }

    /// How `host` gets back to moment `at`.
    pub fn restore_plan(&self, at: &ActId, host: &HostTag) -> crate::Result<RestorePlan> {
        let state = self.chains.state_at(at)?;
        let history = self.chains.history_of(at).ok_or(crate::Error::UnknownAct(*at))?;
        let effective = self.chains.effective(&history, Some(at));
        let version_idx = effective.iter().rposition(|e| matches!(e.payload(), Payload::Version(v) if v.host == *host));
        let version = version_idx.map(|i| effective[i].clone());
        let replay = effective
            .iter()
            .skip(version_idx.map(|i| i + 1).unwrap_or(0))
            .filter(|e| e.payload().is_track_edit() || matches!(e.payload(), Payload::Transport(TransportChange::Tempo(_)) | Payload::Merge(_)))
            .map(|e| (*e).clone())
            .collect();
        Ok(RestorePlan { version, replay, state })
    }

    // ----- names -----

    /// What to call a being: what its Name declared, else the being id.
    pub fn being_label(&self, being: &BeingRef) -> String {
        self.chains.beings().display_name(being).map(str::to_string).unwrap_or_else(|| being.being.0.clone())
    }

    pub fn being_color(&self, being: &BeingRef) -> Option<u32> {
        self.chains.beings().color(being)
    }

    /// What to call a Name: its first declared being's name, else the code.
    pub fn name_label(&self, name: &Id) -> String {
        self.chains.beings().of_name(name).into_iter().find(|(_, b)| !b.revoked && !b.display_name.is_empty()).map(|(_, b)| b.display_name.clone()).unwrap_or_else(|| name.short_code())
    }

    // ----- chat, presence -----

    pub fn chat(&mut self, who: &Stamper, history: HistoryId, text: &str, pinned: Option<ActId>, now_ms: u64) -> crate::Result<Act> {
        self.on_song(who, history, Payload::Chat(ChatPayload { text: text.to_string(), pinned }), now_ms)
    }

    /// Where a being is (an act on its own presence thing).
    pub fn presence(&mut self, who: &Stamper, history: HistoryId, presence: PresencePayload, pos: Option<SongPos>, now_ms: u64) -> crate::Result<Act> {
        let thing = Thing::Presence(who.being_ref());
        self.act(who, history, thing, Payload::Presence(presence), pos, now_ms)
    }

    /// Click a message: the moment to jump to.
    pub fn jump_target(&self, chat: &ActId) -> Option<(HistoryId, ActId, Option<SongPos>)> {
        let a = self.chains.get(chat)?;
        match a.payload() {
            Payload::Chat(c) => {
                let target = c.pinned.and_then(|p| self.chains.get(&p)).unwrap_or(a);
                Some((self.chains.history_of(&target.id)?, target.id, target.pos()))
            }
            _ => None,
        }
    }

    /// Every chat message in the song, main first then other histories,
    /// each in seal order, with the history's name.
    pub fn all_chat(&self) -> Vec<(Act, String)> {
        let mut v = Vec::new();
        for h in self.chains.histories() {
            for a in self.chains.sequence(&h.id) {
                if matches!(a.payload(), Payload::Chat(_)) {
                    v.push((a.clone(), h.name.clone()));
                }
            }
        }
        v
    }

    // ----- merge -----

    /// Preview a Merge: FROM `from` of `source` (this song or another one)
    /// TO `to` here, the whole history or chosen tracks. Fact chains
    /// travel (as the acts they sealed); act chains never do.
    pub fn merge_preview(&self, to: HistoryId, source: &Song, from: HistoryId, tracks: &[TrackId], upto: Option<ActId>) -> crate::Result<MergePreview> {
        let same_song = source.id() == self.id();
        let src = source.chains();
        if !src.has_history(&from) {
            return Err(crate::Error::UnknownHistory(from));
        }
        if !self.chains.has_history(&to) {
            return Err(crate::Error::UnknownHistory(to));
        }
        let from_moment = match upto {
            Some(a) => a,
            None => src.latest(&from).ok_or(crate::Error::UnknownHistory(from))?,
        };
        let src_eff = track_edits(src.effective(&from, Some(&from_moment)), tracks);
        let to_eff = track_edits(self.chains.effective(&to, None), tracks);
        let to_ids: HashSet<ActId> = to_eff.iter().map(|a| a.id).collect();
        let src_ids: HashSet<ActId> = src_eff.iter().map(|a| a.id).collect();
        let acts: Vec<Act> = src_eff.into_iter().filter(|a| !to_ids.contains(&a.id)).collect();
        let to_only: Vec<&Act> = to_eff.iter().filter(|a| !src_ids.contains(&a.id)).collect();
        let mut conflicts: Vec<Conflict> = Vec::new();
        for a in &acts {
            let Some(p) = &a.body.property else { continue };
            if let Some(t) = to_only.iter().rev().find(|x| x.thing() == a.thing() && x.body.property.as_deref() == Some(p.as_str())) {
                match conflicts.iter_mut().find(|c| c.thing == *a.thing() && c.property == *p) {
                    Some(c) => c.from_act = a.id,
                    None => conflicts.push(Conflict { thing: a.thing().clone(), property: p.clone(), from_act: a.id, to_act: t.id, keep_from: false }),
                }
            }
        }
        let mut counts: BTreeMap<TrackId, usize> = BTreeMap::new();
        for a in &acts {
            if let Some(t) = a.track() {
                *counts.entry(t.clone()).or_default() += 1;
            }
        }
        let mut flac_needed = Vec::new();
        for a in &acts {
            if let Payload::Audio(au) = a.payload() {
                for h in &au.flac_chunks {
                    if !self.blobs.has(h) && !flac_needed.contains(h) {
                        flac_needed.push(*h);
                    }
                }
            }
        }
        Ok(MergePreview { from_history: from, from_song: if same_song { None } else { Some(source.id()) }, from_moment, to_history: to, tracks: tracks.to_vec(), acts, conflicts, counts, flac_needed })
    }

    /// Land a previewed Merge as one group (one act, undoable as one).
    /// Fails if archive chunks are still missing: pull `flac_needed` first.
    pub fn merge(&mut self, who: &Stamper, preview: &MergePreview, now_ms: u64) -> crate::Result<Act> {
        let missing = self.blobs.missing(preview.flac_needed.iter());
        if !missing.is_empty() {
            return Err(crate::Error::BlobNotFound(missing[0]));
        }
        let acts: Vec<Act> = preview
            .acts
            .iter()
            .filter(|a| match &a.body.property {
                Some(p) => preview.conflicts.iter().find(|c| c.thing == *a.thing() && c.property == *p).map(|c| c.keep_from).unwrap_or(true),
                None => true,
            })
            .cloned()
            .collect();
        let choices = preview.conflicts.iter().map(|c| (c.thing.clone(), c.property.clone(), c.keep_from)).collect();
        let thing = match preview.tracks.as_slice() {
            [t] => Thing::Track(t.clone()),
            _ => Thing::Song,
        };
        let payload = Payload::Merge(MergePayload { from_history: preview.from_history, from_song: preview.from_song, from_moment: preview.from_moment, tracks: preview.tracks.clone(), choices, acts });
        self.act(who, preview.to_history, thing, payload, None, now_ms)
    }

    // ----- blobs -----

    /// Opus (listening) chunks referenced in a history that are not here.
    pub fn missing_opus(&self, history: &HistoryId) -> Vec<Hash> {
        let Some(st) = self.chains.state(history) else { return vec![] };
        self.blobs.missing(st.tracks.values().flat_map(|l| l.opus_hashes()).collect::<Vec<_>>().iter())
    }

    /// FLAC (archive) chunks referenced in a history that are not here.
    pub fn missing_flac(&self, history: &HistoryId) -> Vec<Hash> {
        let Some(st) = self.chains.state(history) else { return vec![] };
        self.blobs.missing(st.tracks.values().flat_map(|l| l.flac_hashes()).collect::<Vec<_>>().iter())
    }

    /// Blobs a Version needs (project file + manifest) that are not here.
    pub fn missing_for_version(&self, version: &ActId) -> Vec<Hash> {
        let Some(a) = self.chains.get(version) else { return vec![] };
        let Payload::Version(v) = a.payload() else { return vec![] };
        let mut wanted = vec![v.project];
        wanted.extend(v.manifest.iter().map(|m| m.hash));
        self.blobs.missing(wanted.iter())
    }
}

/// The track edits in a list of effective acts (Merge groups flattened to
/// the acts they carried), limited to `tracks` when given.
fn track_edits(effective: Vec<&Act>, tracks: &[TrackId]) -> Vec<Act> {
    let keep = |a: &Act| tracks.is_empty() || a.track().map(|t| tracks.contains(t)).unwrap_or(false);
    let mut out = Vec::new();
    for e in effective {
        match e.payload() {
            p if p.is_track_edit() => {
                if keep(e) {
                    out.push(e.clone());
                }
            }
            Payload::Merge(m) => out.extend(m.acts.iter().filter(|x| x.payload().is_track_edit() && keep(x)).cloned()),
            _ => {}
        }
    }
    out
}
