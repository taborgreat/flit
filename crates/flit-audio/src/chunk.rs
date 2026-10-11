//! One bar of audio, encoded independently: an Opus chunk (listening) and a
//! FLAC chunk (archive), both content-addressed.
//!
//! Opus chunk layout (little endian):
//!   magic "FLO1" | u32 source sample rate | u8 channels | u32 frames
//!   | u16 pre-skip | packets: (u16 len, bytes)*
//! The source sample rate is what the owner recorded at; the Opus data is
//! always 48 kHz and is resampled back on decode when asked.
//!
//! FLAC chunk: a complete FLAC stream (fLaC marker, STREAMINFO, frames).

use crate::resample::resample;
use crate::{opus_err, Error, Result};
use flacenc::component::BitRepr;
use flacenc::error::Verify;
use flit_core::chunk::ChunkPlan;
use flit_core::{BlobStore, Hash};
use flit_host::{ChunkEncoder, RenderedAudio};
use opusic_c::{Application, Bitrate, Channels, Decoder, Encoder, SampleRate};

const OPUS_MAGIC: &[u8; 4] = b"FLO1";
const OPUS_RATE: u32 = 48_000;
const FRAME: usize = 960; // 20 ms at 48 kHz
const MAX_PACKET: usize = 4000;
/// Listening copy bitrate for saved takes (stereo).
pub const ARCHIVE_OPUS_BITRATE: u32 = 192_000;

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedChunk {
    pub sample_rate: u32,
    pub channels: u8,
    pub bit_depth: u8,
    /// Interleaved.
    pub samples: Vec<f32>,
}

fn channels_of(n: u8) -> Result<Channels> {
    match n {
        1 => Ok(Channels::Mono),
        2 => Ok(Channels::Stereo),
        _ => Err(Error::Unsupported(format!("{n} channels"))),
    }
}

/// Encode one bar as an Opus chunk.
pub fn encode_opus_chunk(samples: &[f32], sample_rate: u32, channels: u8, bitrate: u32) -> Result<Vec<u8>> {
    let ch = channels_of(channels)?;
    let chn = channels as usize;
    let at48 = resample(samples, chn, sample_rate, OPUS_RATE);
    let frames = at48.len() / chn;
    let mut enc = Encoder::new(ch, SampleRate::Hz48000, Application::Audio).map_err(opus_err)?;
    enc.set_bitrate(Bitrate::Value(bitrate)).map_err(opus_err)?;
    enc.set_complexity(10).map_err(opus_err)?;
    let preskip = enc.get_look_ahead().map_err(opus_err)? as u16;

    let mut out = Vec::with_capacity(frames / 8 + 32);
    out.extend_from_slice(OPUS_MAGIC);
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.push(channels);
    out.extend_from_slice(&(frames as u32).to_le_bytes());
    out.extend_from_slice(&preskip.to_le_bytes());

    let mut pkt = vec![0u8; MAX_PACKET];
    let mut frame = vec![0f32; FRAME * chn];
    // One extra frame of silence flushes the encoder's lookahead.
    let total_frames = frames + preskip as usize;
    let mut pos = 0usize;
    while pos < total_frames {
        frame.iter_mut().for_each(|s| *s = 0.0);
        let take = FRAME.min(frames.saturating_sub(pos));
        if take > 0 {
            frame[..take * chn].copy_from_slice(&at48[pos * chn..(pos + take) * chn]);
        }
        let n = enc.encode_float_to_slice(&frame, &mut pkt).map_err(opus_err)?;
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&pkt[..n]);
        pos += FRAME;
    }
    Ok(out)
}

/// Decode an Opus chunk. `to_rate`: resample back to this rate (use the
/// chunk's own source rate by passing `None`).
pub fn decode_opus_chunk(bytes: &[u8], to_rate: Option<u32>) -> Result<DecodedChunk> {
    if bytes.len() < 15 || &bytes[..4] != OPUS_MAGIC {
        return Err(Error::BadChunk("not an opus chunk"));
    }
    let sample_rate = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let channels = bytes[8];
    let frames = u32::from_le_bytes([bytes[9], bytes[10], bytes[11], bytes[12]]) as usize;
    let preskip = u16::from_le_bytes([bytes[13], bytes[14]]) as usize;
    if frames > 48_000 * 600 || !(8000..=384_000).contains(&sample_rate) {
        return Err(Error::BadChunk("absurd header"));
    }
    let ch = channels_of(channels)?;
    let chn = channels as usize;
    let mut dec = Decoder::new(ch, SampleRate::Hz48000).map_err(opus_err)?;
    let mut pcm = Vec::with_capacity((frames + preskip + FRAME) * chn);
    let mut buf = vec![0f32; FRAME * 6 * chn];
    let mut p = 15usize;
    while p + 2 <= bytes.len() {
        let len = u16::from_le_bytes([bytes[p], bytes[p + 1]]) as usize;
        p += 2;
        if p + len > bytes.len() {
            return Err(Error::BadChunk("truncated packet"));
        }
        let n = dec.decode_float_to_slice(&bytes[p..p + len], &mut buf, false).map_err(opus_err)?;
        pcm.extend_from_slice(&buf[..n * chn]);
        p += len;
        if pcm.len() / chn >= frames + preskip {
            break;
        }
    }
    let start = preskip.min(pcm.len() / chn);
    let end = (start + frames).min(pcm.len() / chn);
    let trimmed = &pcm[start * chn..end * chn];
    let target = to_rate.unwrap_or(sample_rate);
    let samples = resample(trimmed, chn, OPUS_RATE, target);
    Ok(DecodedChunk { sample_rate: target, channels, bit_depth: 32, samples })
}

/// Encode one bar as a FLAC stream at `bit_depth` (16 or 24).
pub fn encode_flac_chunk(samples: &[f32], sample_rate: u32, channels: u8, bit_depth: u8) -> Result<Vec<u8>> {
    let bits = match bit_depth {
        16 => 16usize,
        _ => 24usize,
    };
    let scale = ((1u64 << (bits - 1)) - 1) as f32;
    let max = scale as i32;
    let ints: Vec<i32> = samples.iter().map(|s| (s.clamp(-1.0, 1.0) * scale).round().clamp(-(max + 1) as f32, max as f32) as i32).collect();
    let config = flacenc::config::Encoder::default().into_verified().map_err(|e| Error::FlacEncode(format!("{e:?}")))?;
    let source = flacenc::source::MemSource::from_samples(&ints, channels as usize, bits, sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size).map_err(|e| Error::FlacEncode(format!("{e:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| Error::FlacEncode(format!("{e:?}")))?;
    Ok(sink.as_slice().to_vec())
}

pub fn decode_flac_chunk(bytes: &[u8]) -> Result<DecodedChunk> {
    let mut reader = claxon::FlacReader::new(std::io::Cursor::new(bytes)).map_err(|e| Error::FlacDecode(e.to_string()))?;
    let info = reader.streaminfo();
    if info.channels == 0 || info.channels > 8 || info.bits_per_sample == 0 || info.bits_per_sample > 32 {
        return Err(Error::FlacDecode("bad streaminfo".into()));
    }
    if let Some(n) = info.samples {
        if n > 48_000 * 600 {
            return Err(Error::FlacDecode("absurd length".into()));
        }
    }
    let scale = ((1u64 << (info.bits_per_sample - 1)) - 1) as f32;
    let mut samples = Vec::with_capacity(info.samples.unwrap_or(0) as usize * info.channels as usize);
    for s in reader.samples() {
        let s = s.map_err(|e| Error::FlacDecode(e.to_string()))?;
        samples.push(s as f32 / scale);
        if samples.len() > 48_000 * 600 * 8 {
            return Err(Error::FlacDecode("absurd length".into()));
        }
    }
    Ok(DecodedChunk { sample_rate: info.sample_rate, channels: info.channels as u8, bit_depth: info.bits_per_sample as u8, samples })
}

/// Both copies of one bar.
pub fn encode_bar(samples: &[f32], sample_rate: u32, channels: u8, bit_depth: u8, opus_bitrate: u32) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        encode_opus_chunk(samples, sample_rate, channels, opus_bitrate)?,
        encode_flac_chunk(samples, sample_rate, channels, bit_depth)?,
    ))
}

/// Which copy to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkEncoderImpl {
    OpusAndFlac,
}

/// The real `ChunkEncoder`: Opus listening copy + FLAC archive copy per bar.
pub struct FlitChunkEncoder {
    pub bit_depth: u8,
    pub opus_bitrate: u32,
}

impl Default for FlitChunkEncoder {
    fn default() -> Self {
        Self { bit_depth: 24, opus_bitrate: ARCHIVE_OPUS_BITRATE }
    }
}

impl ChunkEncoder for FlitChunkEncoder {
    fn encode(&mut self, store: &BlobStore, audio: &RenderedAudio, plan: &ChunkPlan) -> flit_core::Result<(Vec<Hash>, Vec<Hash>)> {
        let mut opus = Vec::new();
        let mut flac = Vec::new();
        for bar in flit_core::chunk::split(&audio.samples, plan) {
            let (o, f) = encode_bar(bar, audio.sample_rate, audio.channels, self.bit_depth, self.opus_bitrate)
                .map_err(|e| flit_core::Error::Encode(e.to_string()))?;
            opus.push(store.put(&o)?);
            flac.push(store.put(&f)?);
        }
        Ok((opus, flac))
    }

    fn bit_depth(&self) -> u8 {
        self.bit_depth
    }
}

/// Load a clip's audio from the store: FLAC when every archive chunk is
/// here, else the Opus copy (resampled to the clip's rate). `None` if a
/// chunk is missing from both sets.
pub fn load_clip(store: &BlobStore, clip: &flit_core::AudioClip) -> Result<Option<DecodedChunk>> {
    let mut out = Vec::new();
    let bars = clip.opus_chunks.len().max(clip.flac_chunks.len());
    for i in 0..bars {
        let flac = clip.flac_chunks.get(i).filter(|h| store.has(h));
        let opus = clip.opus_chunks.get(i).filter(|h| store.has(h));
        let decoded = match (flac, opus) {
            (Some(h), _) => decode_flac_chunk(&store.get(h)?)?,
            (None, Some(h)) => decode_opus_chunk(&store.get(h)?, Some(clip.sample_rate))?,
            (None, None) => return Ok(None),
        };
        out.extend(decoded.samples);
    }
    Ok(Some(DecodedChunk { sample_rate: clip.sample_rate, channels: clip.channels, bit_depth: clip.bit_depth, samples: out }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, sr: u32, ch: usize, hz: f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * ch);
        for i in 0..frames {
            let s = (i as f32 * hz * std::f32::consts::TAU / sr as f32).sin() * 0.5;
            for _ in 0..ch {
                v.push(s);
            }
        }
        v
    }

    fn rms_diff(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len()).max(1);
        (a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() / n as f32).sqrt()
    }

    #[test]
    fn flac_is_lossless_at_24_bit() {
        let pcm = tone(96_000, 48_000, 2, 440.0);
        let bytes = encode_flac_chunk(&pcm, 48_000, 2, 24).unwrap();
        assert!(bytes.len() < pcm.len() * 4 * 3 / 4, "flac ({}) should be well under f32 size", bytes.len());
        let back = decode_flac_chunk(&bytes).unwrap();
        assert_eq!(back.sample_rate, 48_000);
        assert_eq!(back.channels, 2);
        assert_eq!(back.bit_depth, 24);
        assert_eq!(back.samples.len(), pcm.len());
        // 24-bit quantization only
        assert!(rms_diff(&back.samples, &pcm) < 1e-6);
    }

    #[test]
    fn opus_roundtrip_keeps_length_and_sound() {
        let pcm = tone(96_000, 48_000, 2, 440.0);
        let bytes = encode_opus_chunk(&pcm, 48_000, 2, 192_000).unwrap();
        assert!(bytes.len() < 60_000, "2 s stereo at 192k is about 48 kB, got {}", bytes.len());
        let back = decode_opus_chunk(&bytes, None).unwrap();
        assert_eq!(back.samples.len(), pcm.len());
        // lossy, but close (skip the first few ms of codec warm-up)
        let skip = 2000 * 2;
        assert!(rms_diff(&back.samples[skip..], &pcm[skip..]) < 0.05, "{}", rms_diff(&back.samples[skip..], &pcm[skip..]));
    }

    #[test]
    fn opus_resamples_non_48k_sources() {
        let pcm = tone(44_100, 44_100, 1, 1000.0);
        let bytes = encode_opus_chunk(&pcm, 44_100, 1, 128_000).unwrap();
        let back = decode_opus_chunk(&bytes, None).unwrap();
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.samples.len(), pcm.len());
        let at48 = decode_opus_chunk(&bytes, Some(48_000)).unwrap();
        assert_eq!(at48.samples.len(), 48_000);
    }

    #[test]
    fn garbage_never_panics() {
        for bytes in [vec![], vec![0u8; 3], b"FLO1".to_vec(), vec![0xff; 100], b"fLaC\0\0\0".to_vec()] {
            assert!(decode_opus_chunk(&bytes, None).is_err());
            assert!(decode_flac_chunk(&bytes).is_err());
        }
        let mut good = encode_opus_chunk(&tone(4800, 48_000, 1, 440.0), 48_000, 1, 96_000).unwrap();
        good.truncate(good.len() - 5);
        assert!(decode_opus_chunk(&good, None).is_err());
        let mut hdr = good.clone();
        hdr[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_opus_chunk(&hdr, None).is_err());
    }

    #[test]
    fn chunk_encoder_stores_both_copies_and_load_prefers_flac() {
        use flit_core::chunk::ChunkPlan;
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::open(dir.path()).unwrap();
        let plan = ChunkPlan { sample_rate: 48_000, channels: 2, bpm: 120.0, time_sig: Default::default() };
        let audio = RenderedAudio { sample_rate: 48_000, channels: 2, start: flit_core::SongPos::START, samples: tone(96_000 * 2 + 100, 48_000, 2, 220.0), print_point: Default::default() };
        let mut enc = FlitChunkEncoder::default();
        let (opus, flac) = enc.encode(&store, &audio, &plan).unwrap();
        assert_eq!(opus.len(), 3);
        assert_eq!(flac.len(), 3);
        assert_ne!(opus[0], flac[0]);
        let clip = flit_core::AudioClip { start: flit_core::SongPos::START, sample_rate: 48_000, bit_depth: 24, channels: 2, print_point: Default::default(), opus_chunks: opus.clone(), flac_chunks: flac.clone(), credit: None, act: Hash::ZERO };
        let full = load_clip(&store, &clip).unwrap().unwrap();
        assert_eq!(full.samples.len(), audio.samples.len());
        assert!(rms_diff(&full.samples, &audio.samples) < 1e-6, "flac path is lossless");
        // without the flac copies we still play from opus
        let store2 = BlobStore::open(dir.path().join("peer")).unwrap();
        for h in &opus {
            store2.put(&store.get(h).unwrap()).unwrap();
        }
        let lossy = load_clip(&store2, &clip).unwrap().unwrap();
        assert_eq!(lossy.samples.len(), audio.samples.len());
        // a missing chunk in both sets -> None
        let mut clip2 = clip.clone();
        clip2.opus_chunks[1] = Hash::of(b"missing");
        assert!(load_clip(&store2, &clip2).unwrap().is_none());
    }
}
