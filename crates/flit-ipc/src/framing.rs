//! u32 LE length prefix + postcard body. Same for sync and async.

use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Read, Write};

/// Hard cap on one frame. Audio blocks are far below this.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

pub fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    let body = postcard::to_allocvec(v).expect("ipc message encodes");
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn decode<T: DeserializeOwned>(body: &[u8]) -> io::Result<T> {
    postcard::from_bytes(body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

pub fn write_sync<W: Write, T: Serialize>(w: &mut W, v: &T) -> io::Result<()> {
    w.write_all(&encode(v))?;
    w.flush()
}

pub fn read_sync<R: Read, T: DeserializeOwned>(r: &mut R) -> io::Result<T> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    decode(&buf)
}

pub async fn write_async<W: tokio::io::AsyncWrite + Unpin, T: Serialize>(w: &mut W, v: &T) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;
    w.write_all(&encode(v)).await?;
    w.flush().await
}

pub async fn read_async<R: tokio::io::AsyncRead + Unpin, T: DeserializeOwned>(r: &mut R) -> io::Result<T> {
    use tokio::io::AsyncReadExt;
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    decode(&buf)
}
