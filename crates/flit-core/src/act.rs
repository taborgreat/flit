//! Acts and facts: the two records of every moment (update pack §0, §3).
//!
//! - A **Name** is the account: one root keypair (`Id`). It holds authority.
//! - A **Being** acts under a Name (human | ai | scripted). A stamp carries
//!   name + being.
//! - A being stamps an **Act**; the song's **Keeper** seals the matching
//!   **Fact**. One act + its fact = one moment.
//! - Order comes only from chains: `act_seq` per being per history, and the
//!   keeper's `fact_seq` (per thing) / `seal_seq` (per history). Time is
//!   information: `stamped_at` is signed evidence, never order.

use crate::blob::Hash;
use crate::id::{Id, Keypair, Signature};
use crate::plugin::{DeviceSlot, ParamId, PluginIdentity};
use crate::position::SongPos;
use serde::{Deserialize, Serialize};

pub type ActId = Hash;
pub type FactId = Hash;
/// A history (what branches were). `main` is the all-zero id; another
/// history's id is the id of the `History` act that created it.
pub type HistoryId = Hash;

/// Current on-disk / on-wire format of acts, facts and spines.
pub const FORMAT_VERSION: u32 = 1;

/// Which host produced an act ("plugin" when no bridge is involved). The
/// core never interprets this beyond display and `HostState` routing.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub struct HostTag(pub String);

impl HostTag {
    pub fn plugin_only() -> Self {
        HostTag("plugin".into())
    }
    pub fn new(s: &str) -> Self {
        HostTag(s.to_string())
    }
}

impl std::fmt::Display for HostTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A mixer / instrument / bus / master track. No owner. The id is stable
/// and host-neutral from the core's point of view; the bridge maps it to
/// its host's tracks and reports the display name.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub struct TrackId(pub String);

impl TrackId {
    pub fn new(s: &str) -> Self {
        TrackId(s.to_string())
    }
}

impl From<&str> for TrackId {
    fn from(s: &str) -> Self {
        TrackId(s.to_string())
    }
}

impl std::fmt::Display for TrackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A being under a Name: short id, unique within that Name.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub struct BeingId(pub String);

impl BeingId {
    pub fn new(s: &str) -> Self {
        BeingId(s.to_string())
    }
}

impl std::fmt::Display for BeingId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A being of a Name, as it appears everywhere a moment is credited.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub struct BeingRef {
    pub name: Id,
    pub being: BeingId,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Default)]
pub enum BeingKind {
    #[default]
    Human,
    Ai,
    Scripted,
}

impl std::fmt::Display for BeingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BeingKind::Human => "human",
            BeingKind::Ai => "ai",
            BeingKind::Scripted => "scripted",
        })
    }
}

/// Anything with a fact chain. Things nest: song > song settings / tracks
/// > devices. An act targets the most specific thing it touched.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, PartialOrd, Ord)]
pub enum Thing {
    Song,
    SongSettings,
    Track(TrackId),
    Device { track: TrackId, slot: DeviceSlot },
    /// A being's own presence thing: written only by that being.
    Presence(BeingRef),
}

impl Thing {
    pub fn track(&self) -> Option<&TrackId> {
        match self {
            Thing::Track(t) | Thing::Device { track: t, .. } => Some(t),
            _ => None,
        }
    }
    pub fn device_slot(&self) -> Option<DeviceSlot> {
        match self {
            Thing::Device { slot, .. } => Some(*slot),
            _ => None,
        }
    }
    /// A short stable path, for folders and display.
    pub fn path(&self) -> String {
        match self {
            Thing::Song => "song".into(),
            Thing::SongSettings => "song/settings".into(),
            Thing::Track(t) => format!("track/{t}"),
            Thing::Device { track, slot } => format!("track/{track}/device/{}", slot.0),
            Thing::Presence(b) => format!("presence/{}/{}", b.name.short_code(), b.being),
        }
    }
}

impl std::fmt::Display for Thing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.path())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub enum Kind {
    Setting,
    Mixer,
    Device,
    Transport,
    Notes,
    Audio,
    Version,
    Crew,
    Being,
    Grant,
    Flag,
    Keeper,
    History,
    Merge,
    Chat,
    Presence,
    HostState,
    GoBack,
    Undo,
    Settings,
}

// ----- payloads -----

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct SettingPayload {
    pub plugin: PluginIdentity,
    pub param: ParamId,
    /// Normalized 0..1.
    pub value: f32,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum MixerChange {
    /// Linear gain, 1.0 = unity.
    Volume(f32),
    /// -1 left .. 0 centre .. 1 right.
    Pan(f32),
    Mute(bool),
}

impl MixerChange {
    pub fn property(&self) -> &'static str {
        match self {
            MixerChange::Volume(_) => "volume",
            MixerChange::Pan(_) => "pan",
            MixerChange::Mute(_) => "mute",
        }
    }
}

/// A plugin placed in a device slot (add or replace) or taken out of it.
/// An act on the track; the device's settings start over when it changes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct DevicePayload {
    pub slot: DeviceSlot,
    pub plugin: Option<PluginIdentity>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum TransportChange {
    Play,
    Stop,
    Position(SongPos),
    /// Beats per minute.
    Tempo(f64),
    Loop { start: SongPos, end: SongPos },
    LoopOff,
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub struct Note {
    pub start: SongPos,
    pub length_ticks: i64,
    pub key: u8,
    pub velocity: u8,
    pub channel: u8,
}

/// A change to a clip's notes: these notes added, these removed. Two
/// beings adding different notes to one clip touch different properties,
/// so both land; the same note (start, key, channel) is one property.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct NotesPayload {
    pub clip: String,
    pub add: Vec<Note>,
    pub remove: Vec<Note>,
}

impl NotesPayload {
    pub fn property(&self) -> String {
        let mut keys: Vec<(i64, u8, u8)> = self.add.iter().chain(self.remove.iter()).map(|n| (n.start.ticks, n.key, n.channel)).collect();
        keys.sort();
        keys.dedup();
        let joined = keys.iter().map(|(s, k, c)| format!("{s}:{k}:{c}")).collect::<Vec<_>>().join(",");
        if keys.len() <= 8 {
            format!("clip/{}/{joined}", self.clip)
        } else {
            format!("clip/{}/{}", self.clip, Hash::of(joined.as_bytes()).short())
        }
    }
}

/// Where in the signal chain a track's audio was captured.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Default)]
pub enum PrintPoint {
    /// After the track's inserts, fader and pan; before sends and master.
    #[default]
    PostFaderPreSend,
    /// Raw input, before any processing (e.g. a recorded Talk stream).
    Raw,
    /// The host could not honor the standard point; treat as approximate.
    Other,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct AudioPayload {
    pub start: SongPos,
    pub sample_rate: u32,
    pub bit_depth: u8,
    pub channels: u8,
    pub print_point: PrintPoint,
    /// Listening copy: one independent Opus stream per bar, by hash.
    pub opus_chunks: Vec<Hash>,
    /// Archive copy: one independent FLAC stream per bar, same bars.
    pub flac_chunks: Vec<Hash>,
    /// Who played it (a recorded Talk stream credits the player).
    pub credit: Option<BeingRef>,
}

impl AudioPayload {
    pub fn bars(&self) -> usize {
        self.opus_chunks.len().max(self.flac_chunks.len())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct ManifestEntry {
    pub path: String,
    pub hash: Hash,
    pub size: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct VersionPayload {
    pub name: String,
    pub host: HostTag,
    /// Hash of the host's project file blob.
    pub project: Hash,
    pub manifest: Vec<ManifestEntry>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum CrewAction {
    Approve { id: Id, owner: bool },
    /// A gate: the crew closes to this Name. Its beings' act chains stop;
    /// every fact they made stays.
    Remove { id: Id },
    Promote { id: Id },
    /// A gate: one specific being of a crew Name may not act here.
    BlockBeing { being: BeingRef },
    UnblockBeing { being: BeingRef },
    /// An owner accepts a being's equivocation evidence as settled: the
    /// keeper seals its acts again.
    Pardon { being: BeingRef },
}

/// Beings are created, updated and revoked by their Name (signed by the
/// Name key, never by a delegated key).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum BeingAction {
    Create {
        kind: BeingKind,
        display_name: String,
        /// 0xRRGGBB.
        color: u32,
        /// Delegated key for a being running on another machine (opt-in).
        /// None = the Name's key signs for it.
        key: Option<Id>,
        /// Songs the delegated key is good for (empty = any).
        scope: Vec<Hash>,
    },
    Update { display_name: String, color: u32 },
    /// A gate: new acts of this being are rejected; its facts stay.
    Revoke,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct BeingPayload {
    pub being: BeingId,
    pub action: BeingAction,
}

/// An ai / scripted being may stamp main (or any history) on these
/// tracks. Revocable. Only the Name itself grants.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct GrantPayload {
    pub being: BeingRef,
    pub tracks: Vec<TrackId>,
    /// true = grant, false = revoke
    pub grant: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum FlagKind {
    /// "I think this being is not what it claims to be."
    ClaimedKind,
    /// Two validly signed acts by one being with the same act_seq.
    Equivocation { acts: [ActId; 2] },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct FlagPayload {
    pub being: BeingRef,
    pub kind: FlagKind,
    pub note: String,
}

/// Move the keeper role (an owner, on song settings).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct KeeperPayload {
    pub name: Id,
    pub machine: String,
}

/// A new history from a moment. The new history's id is this act's id.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct HistoryPayload {
    pub name: String,
    /// The moment it starts from (an act on the parent history).
    pub from: ActId,
    /// The history `from` lives on (the parent).
    pub parent: HistoryId,
}

/// Merge (merge and pull, one operation): whole history or chosen tracks
/// FROM one history (or another song) TO this one. The facts travel with
/// their original Name + being; act chains never travel.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct MergePayload {
    pub from_history: HistoryId,
    pub from_song: Option<Hash>,
    /// Latest source moment included.
    pub from_moment: ActId,
    /// Which tracks (empty = the whole history).
    pub tracks: Vec<TrackId>,
    /// Per conflicting property: true = keep FROM, false = keep TO.
    pub choices: Vec<(Thing, String, bool)>,
    /// The acts brought over, in their fact order on the source.
    pub acts: Vec<Act>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct ChatPayload {
    pub text: String,
    /// "try this from here": a moment the message points at.
    pub pinned: Option<ActId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug, Default)]
pub enum View {
    #[default]
    Other,
    Mixer,
    Arrange,
    Notes,
    Device,
    Browser,
}

/// Where a being is and what it looks at (an act on its own presence thing).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub struct PresencePayload {
    pub view: View,
    pub track: Option<TrackId>,
    pub device: Option<DeviceSlot>,
    pub region: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct HostStatePayload {
    pub host: HostTag,
    pub blob: Vec<u8>,
}

/// Song settings (an act on `Thing::SongSettings`).
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum SettingsChange {
    /// Whose DAW is the reference mix / final render.
    ReferenceDaw(BeingRef),
    /// Exact app version the keeper seals; others are held.
    RequiredVersion(String),
    TimeSignature { beats: u8, unit: u8 },
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum Payload {
    Setting(SettingPayload),
    Mixer(MixerChange),
    Device(DevicePayload),
    Transport(TransportChange),
    Notes(NotesPayload),
    Audio(AudioPayload),
    Version(VersionPayload),
    Crew(CrewAction),
    Being(BeingPayload),
    Grant(GrantPayload),
    Flag(FlagPayload),
    Keeper(KeeperPayload),
    History(HistoryPayload),
    Merge(MergePayload),
    Chat(ChatPayload),
    Presence(PresencePayload),
    HostState(HostStatePayload),
    GoBack { to: ActId },
    Undo { target: ActId },
    Settings(SettingsChange),
}

impl Payload {
    pub fn kind(&self) -> Kind {
        match self {
            Payload::Setting(_) => Kind::Setting,
            Payload::Mixer(_) => Kind::Mixer,
            Payload::Device(_) => Kind::Device,
            Payload::Transport(_) => Kind::Transport,
            Payload::Notes(_) => Kind::Notes,
            Payload::Audio(_) => Kind::Audio,
            Payload::Version(_) => Kind::Version,
            Payload::Crew(_) => Kind::Crew,
            Payload::Being(_) => Kind::Being,
            Payload::Grant(_) => Kind::Grant,
            Payload::Flag(_) => Kind::Flag,
            Payload::Keeper(_) => Kind::Keeper,
            Payload::History(_) => Kind::History,
            Payload::Merge(_) => Kind::Merge,
            Payload::Chat(_) => Kind::Chat,
            Payload::Presence(_) => Kind::Presence,
            Payload::HostState(_) => Kind::HostState,
            Payload::GoBack { .. } => Kind::GoBack,
            Payload::Undo { .. } => Kind::Undo,
            Payload::Settings(_) => Kind::Settings,
        }
    }

    /// Payloads that change what a track sounds like (the replayable ones).
    pub fn is_track_edit(&self) -> bool {
        matches!(self, Payload::Setting(_) | Payload::Mixer(_) | Payload::Device(_) | Payload::Notes(_) | Payload::Audio(_) | Payload::HostState(_))
    }

    /// A gate refuses or closes.
    pub fn is_gate(&self) -> bool {
        matches!(
            self,
            Payload::Crew(CrewAction::Remove { .. }) | Payload::Crew(CrewAction::BlockBeing { .. }) | Payload::Being(BeingPayload { action: BeingAction::Revoke, .. }) | Payload::Grant(GrantPayload { grant: false, .. })
        )
    }

    /// The property an act changes, for conflict detection (same thing +
    /// same property, stale `based_on` = collision). None = whole thing.
    pub fn property(&self) -> Option<String> {
        match self {
            Payload::Setting(s) => Some(s.param.0.clone()),
            Payload::Mixer(m) => Some(m.property().to_string()),
            Payload::Device(d) => Some(format!("slot/{}", d.slot.0)),
            Payload::Transport(TransportChange::Tempo(_)) => Some("tempo".into()),
            Payload::Notes(n) => Some(n.property()),
            Payload::Audio(a) => Some(format!("audio/{}", a.start.ticks)),
            Payload::Settings(SettingsChange::ReferenceDaw(_)) => Some("reference_daw".into()),
            Payload::Settings(SettingsChange::RequiredVersion(_)) => Some("required_version".into()),
            Payload::Settings(SettingsChange::TimeSignature { .. }) => Some("time_signature".into()),
            Payload::Keeper(_) => Some("keeper".into()),
            _ => None,
        }
    }

    /// Which thing a payload belongs on, given the track it was aimed at
    /// (None = the song). Used to check that an act targets the most
    /// specific thing.
    pub fn thing_level(&self) -> ThingLevel {
        match self {
            Payload::Setting(_) => ThingLevel::Device,
            Payload::Mixer(_) | Payload::Device(_) | Payload::Notes(_) | Payload::Audio(_) | Payload::HostState(_) => ThingLevel::Track,
            Payload::Transport(_) | Payload::Settings(_) | Payload::Keeper(_) => ThingLevel::Settings,
            Payload::Presence(_) => ThingLevel::Presence,
            Payload::Merge(_) => ThingLevel::TrackOrSong,
            _ => ThingLevel::Song,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThingLevel {
    Song,
    Settings,
    Track,
    Device,
    Presence,
    TrackOrSong,
}

// ----- act -----

/// Everything in an act except its id and signature.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct ActBody {
    /// The Name (root key).
    pub name: Id,
    /// The being that acted.
    pub being: BeingId,
    /// Previous act on this being's act chain for `history`.
    pub act_prev: Option<ActId>,
    /// Act number on this being's act chain for `history` (1, 2, 3...).
    pub act_seq: u64,
    /// Most specific thing touched.
    pub thing: Thing,
    /// The exact property changed, or None for whole-thing acts.
    pub property: Option<String>,
    /// `fact_seq` of `thing` the being last saw (None = never saw one).
    pub based_on: Option<u64>,
    pub history: HistoryId,
    pub app_version: String,
    pub format_version: u32,
    /// Which host produced it (display, Version and HostState routing).
    pub host: HostTag,
    pub gate: bool,
    pub payload: Payload,
    /// Song position (bars/beats) if relevant: placement in the music.
    pub pos: Option<SongPos>,
    /// Stamper's clock, ms UTC. Display and evidence only, never order.
    pub stamped_at: u64,
}

impl ActBody {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("act body always encodes")
    }
    pub fn hash(&self) -> ActId {
        Hash::of(&self.canonical_bytes())
    }
}

/// A being's signed act.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Act {
    pub id: ActId,
    pub sig: Signature,
    pub body: ActBody,
}

impl Act {
    /// Sign with the Name's key (or a certified being key).
    pub fn sign(body: ActBody, key: &Keypair) -> Act {
        let id = body.hash();
        let sig = key.sign(id.as_bytes());
        Act { id, sig, body }
    }

    /// Hash tier: id matches the body. The signature is checked against
    /// the Name key or the being's certified key by `verify::signature`.
    pub fn check_id(&self) -> crate::Result<()> {
        if self.body.hash() != self.id {
            return Err(crate::Error::BadId(self.id));
        }
        Ok(())
    }

    /// Does `signer` (a public key) verify this act?
    pub fn signed_by(&self, signer: &Id) -> bool {
        signer.verify(self.id.as_bytes(), &self.sig)
    }

    pub fn kind(&self) -> Kind {
        self.body.payload.kind()
    }
    pub fn name(&self) -> Id {
        self.body.name
    }
    pub fn being(&self) -> BeingRef {
        BeingRef { name: self.body.name, being: self.body.being.clone() }
    }
    pub fn history(&self) -> HistoryId {
        self.body.history
    }
    pub fn thing(&self) -> &Thing {
        &self.body.thing
    }
    pub fn track(&self) -> Option<&TrackId> {
        self.body.thing.track()
    }
    pub fn payload(&self) -> &Payload {
        &self.body.payload
    }
    pub fn pos(&self) -> Option<SongPos> {
        self.body.pos
    }
    pub fn gate(&self) -> bool {
        self.body.gate
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("act encodes to json")
    }
    pub fn from_json(s: &str) -> crate::Result<Act> {
        serde_json::from_str(s).map_err(|e| crate::Error::Decode(e.to_string()))
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("act encodes")
    }
    pub fn from_bytes(b: &[u8]) -> crate::Result<Act> {
        postcard::from_bytes(b).map_err(|e| crate::Error::Decode(e.to_string()))
    }
}

// ----- fact -----

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct FactBody {
    pub thing: Thing,
    /// The history the act was sealed into (differs from the act's when
    /// it was declined into another history).
    pub history: HistoryId,
    /// Number on this thing's fact chain in that history (1, 2, 3...).
    pub fact_seq: u64,
    /// Previous fact on this thing's chain (one; always linear).
    pub fact_prev: Option<FactId>,
    /// Keeper's song-wide seal number for that history.
    pub seal_seq: u64,
    pub act_id: ActId,
    /// Keeper Name + machine id.
    pub keeper: Id,
    pub machine: String,
    /// True when the act was declined into another history.
    pub gate: bool,
}

impl FactBody {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("fact body always encodes")
    }
    pub fn hash(&self) -> FactId {
        Hash::of(&self.canonical_bytes())
    }
}

/// The keeper's seal of an act.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Fact {
    pub id: FactId,
    pub keeper_sig: Signature,
    pub body: FactBody,
}

impl Fact {
    pub fn seal(body: FactBody, keeper_key: &Keypair) -> Fact {
        debug_assert_eq!(body.keeper, keeper_key.id());
        let id = body.hash();
        let keeper_sig = keeper_key.sign(id.as_bytes());
        Fact { id, keeper_sig, body }
    }

    /// Hash tier + signature by the keeper's Name key.
    pub fn verify(&self) -> crate::Result<()> {
        self.verify_with(&[self.body.keeper])
    }

    /// Hash tier + signature by one of `signers` (the keeper Name key or a
    /// key the Name certified for its keeper machine).
    pub fn verify_with(&self, signers: &[Id]) -> crate::Result<()> {
        if self.body.hash() != self.id {
            return Err(crate::Error::BadId(self.id));
        }
        if !signers.iter().any(|s| s.verify(self.id.as_bytes(), &self.keeper_sig)) {
            return Err(crate::Error::BadKeeperSig(self.id));
        }
        Ok(())
    }

    pub fn act_id(&self) -> ActId {
        self.body.act_id
    }
    pub fn history(&self) -> HistoryId {
        self.body.history
    }
    pub fn thing(&self) -> &Thing {
        &self.body.thing
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("fact encodes to json")
    }
    pub fn from_json(s: &str) -> crate::Result<Fact> {
        serde_json::from_str(s).map_err(|e| crate::Error::Decode(e.to_string()))
    }
}

/// A moment: an act and, once the keeper sealed it, its fact.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub struct Moment {
    pub act: Act,
    pub fact: Option<Fact>,
}

/// The main history.
pub fn main_history() -> HistoryId {
    Hash::ZERO
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::SongPos;

    fn body(key: &Keypair) -> ActBody {
        ActBody {
            name: key.id(),
            being: BeingId::new("b1"),
            act_prev: None,
            act_seq: 1,
            thing: Thing::Device { track: TrackId::new("drums"), slot: DeviceSlot(1) },
            property: Some("7".into()),
            based_on: None,
            history: main_history(),
            app_version: "0.1.0".into(),
            format_version: FORMAT_VERSION,
            host: HostTag::new("test"),
            gate: false,
            payload: Payload::Setting(SettingPayload { plugin: PluginIdentity::vst3("Acme", "Comp", "1.2.0"), param: ParamId::from_u32(7), value: 0.25 }),
            pos: Some(SongPos::START),
            stamped_at: 1_000,
        }
    }

    #[test]
    fn act_id_and_signature() {
        let key = Keypair::from_seed(1);
        let a = Act::sign(body(&key), &key);
        a.check_id().unwrap();
        assert!(a.signed_by(&key.id()));
        assert!(!a.signed_by(&Keypair::from_seed(2).id()));
        let mut t = a.clone();
        if let Payload::Setting(s) = &mut t.body.payload {
            s.value = 0.5;
        }
        assert!(matches!(t.check_id(), Err(crate::Error::BadId(_))));
        // stamped_at is inside the signature: a changed time is a changed act
        let mut t2 = a.clone();
        t2.body.stamped_at = 2_000;
        assert!(matches!(t2.check_id(), Err(crate::Error::BadId(_))));
    }

    #[test]
    fn fact_seal_and_verify() {
        let key = Keypair::from_seed(1);
        let keeper = Keypair::from_seed(7);
        let a = Act::sign(body(&key), &key);
        let f = Fact::seal(FactBody { thing: a.thing().clone(), history: main_history(), fact_seq: 1, fact_prev: None, seal_seq: 1, act_id: a.id, keeper: keeper.id(), machine: "m1".into(), gate: false }, &keeper);
        f.verify().unwrap();
        let mut bad = f.clone();
        bad.body.seal_seq = 2;
        assert!(matches!(bad.verify(), Err(crate::Error::BadId(_))));
        let other = Keypair::from_seed(8);
        let forged = Fact { id: f.id, keeper_sig: other.sign(f.id.as_bytes()), body: f.body.clone() };
        assert!(matches!(forged.verify(), Err(crate::Error::BadKeeperSig(_))));
    }

    #[test]
    fn roundtrips() {
        let key = Keypair::from_seed(3);
        let a = Act::sign(body(&key), &key);
        assert_eq!(Act::from_json(&a.to_json()).unwrap(), a);
        assert_eq!(Act::from_bytes(&a.to_bytes()).unwrap(), a);
        assert_eq!(a.payload().property().as_deref(), Some("7"));
        assert!(Payload::Crew(CrewAction::Remove { id: key.id() }).is_gate());
    }
}
