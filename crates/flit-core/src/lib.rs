//! flit-core: the DAW-neutral heart of FLit.
//!
//! A song is a set of signed acts and keeper-sealed facts on chains (the
//! act model, FLIT_SPEC.md) plus a store of files addressed by BLAKE3
//! hash. A Name (one keypair) holds authority; its beings (human, ai,
//! scripted) stamp acts; the song's keeper seals each act into a fact on
//! the touched thing's chain. One act + its fact = one moment. Everything
//! else (histories, Go back, Merge, chat, crew, presence) is a fold over
//! those chains. Time is information, never order.
//!
//! RULE: nothing in this crate knows about any particular DAW. Hosts are
//! reached through the `HostBridge` trait in `flit-host`; this crate only
//! carries an opaque `HostTag` string so acts can say where they came from.

pub mod act;
pub mod beings;
pub mod blob;
pub mod blocklist;
pub mod chunk;
pub mod clock;
pub mod crew;
pub mod error;
pub mod id;
pub mod names;
pub mod org;
pub mod plugin;
pub mod position;
pub mod seal;
pub mod settings;
pub mod song;
pub mod state;

pub use act::{
    main_history, Act, ActBody, ActId, AudioPayload, BeingAction, BeingId, BeingKind, BeingPayload, BeingRef, ChatPayload, CrewAction, DevicePayload, Fact, FactBody, FactId, FlagKind, FlagPayload, GrantPayload, HistoryId,
    HistoryPayload, HostStatePayload, HostTag, KeeperPayload, Kind, ManifestEntry, MergePayload, MixerChange, Moment, Note, NotesPayload, Payload, PresencePayload, PrintPoint, SettingPayload, SettingsChange, Thing,
    TrackId, TransportChange, VersionPayload, View, FORMAT_VERSION,
};
pub use beings::{BeingInfo, Beings, FlagRecord};
pub use blob::{BlobStore, Hash};
pub use blocklist::BlockList;
pub use clock::now_ms;
pub use crew::CrewState;
pub use error::{Error, Result};
pub use id::{Id, Keypair, Signature};
pub use names::{BeingProfile, NameProfile, NameStore, BEING_COLORS};
pub use org::{Access, AiPolicy, AlbumAction, AlbumId, AlbumState, Albums, ContainerKind, OrgDefaults};
pub use plugin::{DeviceSlot, ParamId, PluginFormat, PluginIdentity};
pub use position::{SongPos, TimeSig, TICKS_PER_BEAT};
pub use seal::keeper::{Decision, HoldReason, KeeperKey};
pub use seal::lens::{Lens, Row};
pub use seal::{AppendOutcome, Chains, Equivocation, HistoryInfo, CHECKPOINT_EVERY};
pub use settings::{DownloadPolicy, UserSettings};
pub use song::{Conflict, MergePreview, Notice, Outbound, RestorePlan, Song, SongId, Stamper};
pub use state::{AudioClip, DeviceState, MixerSet, MixerState, PresenceState, SettingValue, SongSettings, SongState, TrackState, TransportState};
