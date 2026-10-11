//! Organizations and albums (PACK 2 §1, §2): the types. An org is a
//! chained container like a song (same acts, facts, keeper, spines, sync),
//! told apart by its genesis act (`ActBody.property = "container:org"`),
//! holding no tracks: its facts are membership, settings and albums. A
//! Name's personal container (`container:personal`) holds that Name's own
//! albums and syncs only to its own machines.
//!
//! Membership of an org is countersigned: an owner stamps
//! `CrewAction::Invite`, the invited Name stamps `CrewAction::Accept`; the
//! org keeper seals the Accept and that is "<Name> is a member". `Decline`
//! and `Leave` are gates. The org never controls a Name's beings.

use crate::act::{ActId, BeingRef};
use crate::blob::Hash;
use crate::id::Id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a chained container is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ContainerKind {
    #[default]
    Song,
    Org,
    Personal,
}

impl ContainerKind {
    /// The marker in the genesis act's `property` (None for a song).
    pub fn genesis_property(self) -> Option<String> {
        match self {
            ContainerKind::Song => None,
            ContainerKind::Org => Some("container:org".into()),
            ContainerKind::Personal => Some("container:personal".into()),
        }
    }

    pub fn from_genesis_property(p: Option<&str>) -> Option<ContainerKind> {
        match p {
            None => Some(ContainerKind::Song),
            Some("container:org") => Some(ContainerKind::Org),
            Some("container:personal") => Some(ContainerKind::Personal),
            Some(_) => None,
        }
    }

    pub fn is_song(self) -> bool {
        self == ContainerKind::Song
    }
}

/// Who may act in a song besides its own crew.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Access {
    /// Only the song's own crew, even inside an org.
    #[default]
    Private,
    /// Every current member of the song's org, including songs that
    /// existed before they joined.
    Org,
}

impl Access {
    pub fn word(self) -> &'static str {
        match self {
            Access::Private => "private",
            Access::Org => "org",
        }
    }
}

/// What ai beings may do in a song. An org sets a default for its songs; a
/// song owner can override it per song.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AiPolicy {
    /// Grants work: a granted ai being may act in main on its tracks.
    #[default]
    Grants,
    /// ai beings act in their own history only, grants or not.
    OwnHistoryOnly,
}

/// Defaults an org gives its new songs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OrgDefaults {
    pub access: Access,
    pub required_version: Option<String>,
    /// Keeper for new songs: Name + machine (None = the creator's machine).
    pub keeper: Option<(Id, String)>,
    pub ai_policy: AiPolicy,
    /// Org albums: show a private song you cannot open as a locked row
    /// (true) or not at all.
    pub show_locked: bool,
}

/// An album is the `Create` act's id.
pub type AlbumId = ActId;

/// Album facts. `Create` lands on the container thing (its id becomes the
/// album's); everything else on `Thing::Album(id)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlbumAction {
    Create { name: String },
    Rename(String),
    SetDescription(String),
    /// A blob in the container's blob store (PNG), or none.
    SetCover(Option<Hash>),
    /// `after` = None puts it last.
    AddSong { song: Hash, after: Option<Hash> },
    RemoveSong { song: Hash },
    /// The whole order; songs left out stay, after the given ones.
    Reorder(Vec<Hash>),
    Archive(bool),
}

impl AlbumAction {
    /// Rename, description, cover and archive are the owner's (or the
    /// album creator's); adding, removing and ordering songs is for every
    /// member who can open the song.
    pub fn is_curation(&self) -> bool {
        matches!(self, AlbumAction::Create { .. } | AlbumAction::Rename(_) | AlbumAction::SetDescription(_) | AlbumAction::SetCover(_) | AlbumAction::Archive(_))
    }

    pub fn property(&self) -> String {
        match self {
            AlbumAction::Create { .. } => "album".into(),
            AlbumAction::Rename(_) => "name".into(),
            AlbumAction::SetDescription(_) => "description".into(),
            AlbumAction::SetCover(_) => "cover".into(),
            AlbumAction::AddSong { song, .. } | AlbumAction::RemoveSong { song } => format!("song/{}", song.short()),
            AlbumAction::Reorder(_) => "order".into(),
            AlbumAction::Archive(_) => "archived".into(),
        }
    }
}

/// The fold of one album.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumState {
    pub name: String,
    pub description: String,
    pub cover: Option<Hash>,
    pub songs: Vec<Hash>,
    pub archived: bool,
    pub created_by: Option<BeingRef>,
    /// The act that last changed it.
    pub act: ActId,
}

impl Default for AlbumState {
    fn default() -> Self {
        Self { name: String::new(), description: String::new(), cover: None, songs: Vec::new(), archived: false, created_by: None, act: Hash::ZERO }
    }
}

impl AlbumState {
    pub fn apply(&mut self, act: ActId, being: &BeingRef, a: &AlbumAction) {
        match a {
            AlbumAction::Create { name } => {
                self.name = name.clone();
                self.created_by = Some(being.clone());
            }
            AlbumAction::Rename(n) => self.name = n.clone(),
            AlbumAction::SetDescription(d) => self.description = d.clone(),
            AlbumAction::SetCover(c) => self.cover = *c,
            AlbumAction::AddSong { song, after } => {
                self.songs.retain(|s| s != song);
                let at = after.and_then(|a| self.songs.iter().position(|s| *s == a).map(|i| i + 1)).unwrap_or(self.songs.len());
                self.songs.insert(at, *song);
            }
            AlbumAction::RemoveSong { song } => self.songs.retain(|s| s != song),
            AlbumAction::Reorder(order) => {
                let mut next: Vec<Hash> = order.iter().filter(|s| self.songs.contains(s)).copied().collect();
                for s in &self.songs {
                    if !next.contains(s) {
                        next.push(*s);
                    }
                }
                self.songs = next;
            }
            AlbumAction::Archive(on) => self.archived = *on,
        }
        self.act = act;
    }
}

/// Every album of a container, by id.
pub type Albums = BTreeMap<AlbumId, AlbumState>;
