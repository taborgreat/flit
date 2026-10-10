//! Audio chunking: cut interleaved PCM into bar-sized pieces so a small edit
//! produces few new chunks. Chunk blobs are raw interleaved f32 LE.

use crate::blob::{BlobStore, Hash};
use crate::position::{SongPos, TimeSig};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkPlan {
    pub sample_rate: u32,
    pub channels: u8,
    pub bpm: f64,
    pub time_sig: TimeSig,
}

impl ChunkPlan {
    /// Frames (per channel samples) in one bar at this tempo.
    pub fn frames_per_bar(&self) -> usize {
        let beats_per_bar = self.time_sig.ticks_per_bar() as f64 / crate::position::TICKS_PER_BEAT as f64;
        let secs = beats_per_bar * 60.0 / self.bpm;
        (secs * self.sample_rate as f64).round().max(1.0) as usize
    }

    pub fn frames_per_chunk(&self) -> usize {
        self.frames_per_bar()
    }
}

/// Split interleaved samples into bar-sized chunks (last one may be short).
pub fn split<'a>(samples: &'a [f32], plan: &ChunkPlan) -> Vec<&'a [f32]> {
    let per_chunk = plan.frames_per_chunk() * plan.channels as usize;
    if per_chunk == 0 {
        return vec![samples];
    }
    samples.chunks(per_chunk).collect()
}

pub fn encode_chunk(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn decode_chunk(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Store every chunk, return their hashes in order.
pub fn store_audio(store: &BlobStore, samples: &[f32], plan: &ChunkPlan) -> crate::Result<Vec<Hash>> {
    let mut hashes = Vec::new();
    for c in split(samples, plan) {
        hashes.push(store.put(&encode_chunk(c))?);
    }
    Ok(hashes)
}

/// Reassemble audio from chunk hashes.
pub fn load_audio(store: &BlobStore, chunks: &[Hash]) -> crate::Result<Vec<f32>> {
    let mut out = Vec::new();
    for h in chunks {
        out.extend(decode_chunk(&store.get(h)?));
    }
    Ok(out)
}

/// Where chunk `i` of a clip starting at `start` begins, in ticks.
pub fn chunk_start(start: SongPos, i: usize, plan: &ChunkPlan) -> SongPos {
    SongPos::from_ticks(start.ticks + i as i64 * plan.time_sig.ticks_per_bar())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> ChunkPlan {
        ChunkPlan { sample_rate: 48000, channels: 2, bpm: 120.0, time_sig: TimeSig::default() }
    }

    #[test]
    fn bar_size() {
        // 4/4 at 120 bpm: a bar is 2 seconds = 96000 frames
        assert_eq!(plan().frames_per_bar(), 96000);
    }

    #[test]
    fn small_edit_makes_few_new_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path()).unwrap();
        let p = plan();
        let frames = p.frames_per_bar() * 4; // 4 bars
        let mut audio: Vec<f32> = (0..frames * 2).map(|i| (i as f32 * 0.001).sin()).collect();
        let a = store_audio(&store, &audio, &p).unwrap();
        assert_eq!(a.len(), 4);
        // edit inside bar 3 only
        let off = p.frames_per_bar() * 2 * 2 + 100;
        audio[off] = 0.9;
        let b = store_audio(&store, &audio, &p).unwrap();
        assert_eq!(a[0], b[0]);
        assert_eq!(a[1], b[1]);
        assert_ne!(a[2], b[2]);
        assert_eq!(a[3], b[3]);
        assert_eq!(store.list().unwrap().len(), 5);
        let back = load_audio(&store, &b).unwrap();
        assert_eq!(back, audio);
    }

    #[test]
    fn partial_last_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path()).unwrap();
        let p = plan();
        let audio = vec![0.5f32; p.frames_per_bar() * 2 + 10];
        let h = store_audio(&store, &audio, &p).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(load_audio(&store, &h).unwrap(), audio);
        assert_eq!(chunk_start(SongPos::START, 1, &p).beats(), 4.0);
    }
}
