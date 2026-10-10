//! flit-audio: how audio is encoded on the wire and on disk.
//!
//! Two formats, never MP3:
//! - Listening copy: Opus (live streams, and sent first for every take).
//! - Archive copy: FLAC, the true copy in history.
//! Every saved bar is encoded independently so chunks stay addressable and
//! resumable. Live lanes and Talk share one streaming encoder; the only
//! difference is a flag (in-song vs outside).

pub mod chunk;
pub mod resample;
pub mod stream;

pub use chunk::{decode_flac_chunk, decode_opus_chunk, encode_bar, ChunkEncoderImpl, DecodedChunk, FlitChunkEncoder};
pub use stream::{StreamConfig, StreamDecoder, StreamEncoder, StreamKind, StreamPacket, Submix};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("opus: {0}")]
    Opus(&'static str),
    #[error("flac encode: {0}")]
    FlacEncode(String),
    #[error("flac decode: {0}")]
    FlacDecode(String),
    #[error("bad chunk: {0}")]
    BadChunk(&'static str),
    #[error("unsupported format: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Core(#[from] flit_core::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

fn opus_err(e: opusic_c::ErrorCode) -> Error {
    Error::Opus(e.message())
}
