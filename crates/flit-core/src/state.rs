//! The folded song model: what every track sounds like right now, the song
//! settings, where every being is. Host-neutral, cheap data. Computed by
//! replaying sealed facts (and unsealed acts on top); never truth itself.

use crate::act::*;
use crate::blob::Hash;
use crate::id::Id;
use crate::plugin::{DeviceSlot, ParamId, PluginIdentity};
use crate::position::{SongPos, TimeSig};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingValue {
    pub plugin: PluginIdentity,
    pub value: f32,
    /// The act that set it.
    pub act: ActId,
}

/// One device slot on a track: which plugin sits there and its settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceState {
    pub plugin: Option<PluginIdentity>,
    pub settings: BTreeMap<ParamId, SettingValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerState {
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
}

impl Default for MixerState {
    fn default() -> Self {
        Self { volume: 1.0, pan: 0.0, mute: false }
    }
}

/// Which mixer fields the song has actually recorded for a track. A track
/// whose fader was never touched keeps the host's own value when a moment
/// is shown, instead of being reset to the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixerSet {
    pub volume: bool,
    pub pan: bool,
    pub mute: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioClip {
    pub start: SongPos,
    pub sample_rate: u32,
    pub bit_depth: u8,
    pub channels: u8,
    pub print_point: PrintPoint,
    pub opus_chunks: Vec<Hash>,
    pub flac_chunks: Vec<Hash>,
    pub credit: Option<BeingRef>,
    pub act: ActId,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackState {
    pub devices: BTreeMap<DeviceSlot, DeviceState>,
    pub mixer: MixerState,
    /// Which of `mixer`'s fields were set by an act (vs. still default).
    pub mixer_set: MixerSet,
    pub clips: BTreeMap<String, Vec<Note>>,
    /// Keyed by start position so a re-render at the same spot replaces.
    pub audio: BTreeMap<SongPos, AudioClip>,
    /// Opaque per-host blobs, keyed by host tag.
    pub host_state: BTreeMap<HostTag, Vec<u8>>,
    /// The act that last touched the track's sound.
    pub last_edit: Option<ActId>,
}

impl TrackState {
    /// Apply a track-edit payload aimed at `thing` (the track or one of its
    /// devices). Other payloads are ignored.
    pub fn apply(&mut self, act_id: ActId, thing: &Thing, payload: &Payload) {
        match payload {
            Payload::Setting(s) => {
                let Some(slot) = thing.device_slot() else { return };
                let dev = self.devices.entry(slot).or_default();
                if dev.plugin.is_none() {
                    dev.plugin = Some(s.plugin.clone());
                }
                dev.settings.insert(s.param.clone(), SettingValue { plugin: s.plugin.clone(), value: s.value, act: act_id });
            }
            Payload::Device(d) => {
                // the slot's settings belonged to the plugin that was there
                match &d.plugin {
                    Some(p) => {
                        self.devices.insert(d.slot, DeviceState { plugin: Some(p.clone()), settings: BTreeMap::new() });
                    }
                    None => {
                        self.devices.remove(&d.slot);
                    }
                }
            }
            Payload::Mixer(m) => match m {
                MixerChange::Volume(v) => {
                    self.mixer.volume = *v;
                    self.mixer_set.volume = true;
                }
                MixerChange::Pan(p) => {
                    self.mixer.pan = *p;
                    self.mixer_set.pan = true;
                }
                MixerChange::Mute(b) => {
                    self.mixer.mute = *b;
                    self.mixer_set.mute = true;
                }
            },
            Payload::Notes(n) => {
                let clip = self.clips.entry(n.clip.clone()).or_default();
                let same = |x: &Note, y: &Note| x.start == y.start && x.key == y.key && x.channel == y.channel;
                for r in &n.remove {
                    clip.retain(|x| !same(x, r));
                }
                for a in &n.add {
                    clip.retain(|x| !same(x, a));
                    clip.push(a.clone());
                }
                clip.sort_by_key(|x| (x.start, x.key, x.channel));
            }
            Payload::Audio(a) => {
                self.audio.insert(
                    a.start,
                    AudioClip {
                        start: a.start,
                        sample_rate: a.sample_rate,
                        bit_depth: a.bit_depth,
                        channels: a.channels,
                        print_point: a.print_point,
                        opus_chunks: a.opus_chunks.clone(),
                        flac_chunks: a.flac_chunks.clone(),
                        credit: a.credit.clone(),
                        act: act_id,
                    },
                );
            }
            Payload::HostState(h) => {
                self.host_state.insert(h.host.clone(), h.blob.clone());
            }
            _ => return,
        }
        self.last_edit = Some(act_id);
    }

    pub fn device(&self, slot: DeviceSlot) -> Option<&DeviceState> {
        self.devices.get(&slot)
    }

    /// The plugin in a slot, if any.
    pub fn plugin(&self, slot: DeviceSlot) -> Option<&PluginIdentity> {
        self.devices.get(&slot).and_then(|d| d.plugin.as_ref())
    }

    pub fn setting(&self, slot: DeviceSlot, param: &ParamId) -> Option<f32> {
        self.devices.get(&slot)?.settings.get(param).map(|v| v.value)
    }

    /// Every (slot, param, value) on the track.
    pub fn settings(&self) -> Vec<(DeviceSlot, ParamId, &SettingValue)> {
        self.devices.iter().flat_map(|(slot, d)| d.settings.iter().map(move |(p, v)| (*slot, p.clone(), v))).collect()
    }

    /// Listening copies (Opus) this track needs to play.
    pub fn opus_hashes(&self) -> Vec<Hash> {
        self.audio.values().flat_map(|c| c.opus_chunks.iter().copied()).collect()
    }

    /// Archive copies (FLAC) needed before export, processing, Merge or
    /// editing this track's audio.
    pub fn flac_hashes(&self) -> Vec<Hash> {
        self.audio.values().flat_map(|c| c.flac_chunks.iter().copied()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransportState {
    pub playing: bool,
    pub position: SongPos,
    pub loop_range: Option<(SongPos, SongPos)>,
}

impl Default for TransportState {
    fn default() -> Self {
        Self { playing: false, position: SongPos::START, loop_range: None }
    }
}

/// Song settings: the fold of the song-settings thing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SongSettings {
    pub tempo: f64,
    pub time_sig: TimeSig,
    /// Whose DAW is the reference mix / final render.
    pub reference_daw: Option<BeingRef>,
    /// Exact app version the keeper seals (None = the genesis act's).
    pub required_version: Option<String>,
    /// Keeper Name + machine (None = the song owner's machine, unnamed).
    pub keeper: Option<(Id, String)>,
}

impl Default for SongSettings {
    fn default() -> Self {
        Self { tempo: 120.0, time_sig: TimeSig::default(), reference_daw: None, required_version: None, keeper: None }
    }
}

/// Where a being is, folded from its presence thing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceState {
    pub view: View,
    pub track: Option<TrackId>,
    pub device: Option<DeviceSlot>,
    pub region: Option<String>,
    pub pos: Option<SongPos>,
    pub act: ActId,
}

/// Everything a history is right now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SongState {
    pub tracks: BTreeMap<TrackId, TrackState>,
    pub settings: SongSettings,
    pub transport: TransportState,
    /// Latest Version act per host.
    pub latest_version: BTreeMap<HostTag, ActId>,
    /// All Version acts in this history, in order.
    pub versions: Vec<ActId>,
    /// Histories started from this one (History act ids).
    pub histories: Vec<ActId>,
    pub chat: Vec<ActId>,
    pub presence: BTreeMap<BeingRef, PresenceState>,
    /// How many acts have been folded in (for incremental updates).
    pub applied: usize,
}

impl SongState {
    pub fn track(&self, track: &TrackId) -> Option<&TrackState> {
        self.tracks.get(track)
    }

    pub fn track_mut(&mut self, track: &TrackId) -> &mut TrackState {
        self.tracks.entry(track.clone()).or_default()
    }

    pub fn tempo(&self) -> f64 {
        self.settings.tempo
    }

    /// Replace the sound of every track with another moment's (Go back).
    /// Versions, histories, chat and presence are organisational and stay.
    pub fn restore_sound_from(&mut self, past: &SongState) {
        self.tracks = past.tracks.clone();
        self.settings.tempo = past.settings.tempo;
        self.settings.time_sig = past.settings.time_sig;
    }

    /// Apply the payload of one act aimed at `thing` (used for an act
    /// itself and for the acts a Merge carries).
    pub fn apply_payload(&mut self, act_id: ActId, being: &BeingRef, thing: &Thing, payload: &Payload) {
        match payload {
            p if p.is_track_edit() => {
                if let Some(track) = thing.track() {
                    self.track_mut(track).apply(act_id, thing, p);
                }
            }
            Payload::Transport(t) => match t {
                TransportChange::Play => self.transport.playing = true,
                TransportChange::Stop => self.transport.playing = false,
                TransportChange::Position(p) => self.transport.position = *p,
                TransportChange::Tempo(bpm) => self.settings.tempo = *bpm,
                TransportChange::Loop { start, end } => self.transport.loop_range = Some((*start, *end)),
                TransportChange::LoopOff => self.transport.loop_range = None,
            },
            Payload::Settings(s) => match s {
                SettingsChange::ReferenceDaw(b) => self.settings.reference_daw = Some(b.clone()),
                SettingsChange::RequiredVersion(v) => self.settings.required_version = Some(v.clone()),
                SettingsChange::TimeSignature { beats, unit } => self.settings.time_sig = TimeSig { numerator: *beats, denominator: *unit },
            },
            Payload::Keeper(k) => self.settings.keeper = Some((k.name, k.machine.clone())),
            Payload::Version(v) => {
                self.latest_version.insert(v.host.clone(), act_id);
                self.versions.push(act_id);
            }
            Payload::History(_) => self.histories.push(act_id),
            Payload::Chat(_) => self.chat.push(act_id),
            Payload::Presence(p) => {
                self.presence.insert(being.clone(), PresenceState { view: p.view, track: p.track.clone(), device: p.device, region: p.region.clone(), pos: None, act: act_id });
            }
            Payload::Merge(m) => {
                for inner in &m.acts {
                    if inner.payload().is_track_edit() {
                        let b = inner.being();
                        self.apply_payload(inner.id, &b, inner.thing(), inner.payload());
                    }
                }
            }
            Payload::Crew(_) | Payload::Being(_) | Payload::Grant(_) | Payload::Flag(_) | Payload::GoBack { .. } | Payload::Undo { .. } => {}
            _ => {}
        }
    }

    /// Fold one act that is known to be live (not undone). `GoBack` is
    /// handled by the caller because it needs the chains.
    pub fn apply(&mut self, act: &Act) {
        let being = act.being();
        self.apply_payload(act.id, &being, act.thing(), act.payload());
        if let Payload::Presence(_) = act.payload() {
            if let Some(p) = self.presence.get_mut(&being) {
                p.pos = act.pos();
            }
        }
    }
}
