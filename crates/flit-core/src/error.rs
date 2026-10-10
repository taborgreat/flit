use crate::act::{ActId, BeingRef, FactId, HistoryId};
use crate::blob::Hash;
use crate::id::Id;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode: {0}")]
    Encode(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("bad signature on act {0}")]
    BadSignature(ActId),
    #[error("id does not match contents: {0}")]
    BadId(Hash),
    #[error("fact {0} is not signed by the keeper")]
    BadKeeperSig(FactId),
    #[error("act {0} already in the chains")]
    Duplicate(ActId),
    #[error("act {act} hangs off {prev}, which is unknown")]
    MissingPrev { act: ActId, prev: ActId },
    #[error("act {0} does not continue its being's act chain")]
    BadActChain(ActId),
    #[error("fact {0} does not continue its thing's fact chain")]
    BadFactChain(FactId),
    #[error("fact {0} does not continue the keeper's seal order")]
    BadSealChain(FactId),
    #[error("history {0} is unknown")]
    UnknownHistory(HistoryId),
    #[error("act {0} is unknown")]
    UnknownAct(ActId),
    #[error("fact {0} is unknown")]
    UnknownFact(FactId),
    #[error("{0} is not in the crew")]
    NotInCrew(Id),
    #[error("{0} is not an owner")]
    NotOwner(Id),
    #[error("being {} of {} is unknown here", .0.being, .0.name)]
    UnknownBeing(BeingRef),
    #[error("being {} of {} was revoked", .0.being, .0.name)]
    RevokedBeing(BeingRef),
    #[error("being {} of {} is blocked in this song", .0.being, .0.name)]
    BlockedBeing(BeingRef),
    #[error("being {} of {} may not {what}", being.being, being.name)]
    NotAllowed { being: BeingRef, what: String },
    #[error("genesis act is malformed: {0}")]
    BadGenesis(String),
    #[error("the chains already have a genesis")]
    AlreadyHasGenesis,
    #[error("cannot remove the last owner")]
    LastOwner,
    #[error("blob {0} not found")]
    BlobNotFound(Hash),
    #[error("blob content does not match hash {0}")]
    BlobCorrupt(Hash),
    #[error("this machine is not the keeper of the song")]
    NotKeeper,
    #[error("fact is sealed by {0}, who is not the keeper")]
    WrongKeeper(Id),
    #[error("being {} of {} stamped two acts with the same number", being.being, being.name)]
    Equivocation { being: BeingRef, acts: [ActId; 2] },
    #[error("this song requires FLit {required}, the act was made by {got}")]
    VersionMismatch { required: String, got: String },
    #[error("data format {0} is newer than this FLit understands")]
    FormatTooNew(u32),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;
