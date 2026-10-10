//! Live Opus streaming. One encoder for lanes, Talk tracks and voice: the
//! same packets, the same transport; `StreamKind` says whether the sound is
//! part of the song or outside it (Talk is never logged).

use crate::resample::resample;
use crate::{opus_err, Result};
use flit_core::LaneId;
use opusic_c::{Application, Bitrate, Channels, Decoder, Encoder, SampleRate};
use serde::{Deserialize, Serialize};

const RATE: u32 = 48_000;
const MAX_PACKET: usize = 4000;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StreamKind {
    /// A lane of the song (or the owner's lanes as one submix).
    InSong { lane: LaneId },
    /// My lanes mixed to one stream, for listeners who don't need them apart.
    Submix,
    /// The mix owner's master output: the reference mix.
    ReferenceMix,
    /// Talk: any mixer track sent live, outside the song. Never logged.
    Talk { name: String },
    /// Voice chat (OS audio). Never logged.
    Voice,
}

impl StreamKind {
    pub fn in_song(&self) -> bool {
        matches!(self, StreamKind::InSong { .. } | StreamKind::Submix | StreamKind::ReferenceMix)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamConfig {
    pub kind: StreamKind,
    pub channels: u8,
    /// Monitoring bitrate, 64 000..=256 000.
    pub bitrate: u32,
    /// Lesson mode: 10 ms frames, Opus restricted low delay, smallest buffers.
    pub low_delay: bool,
    /// Sample rate of the audio pushed in / wanted out.
    pub sample_rate: u32,
}

impl StreamConfig {
    pub fn lane(lane: &LaneId, sample_rate: u32, channels: u8) -> Self {
        Self { kind: StreamKind::InSong { lane: lane.clone() }, channels, bitrate: 128_000, low_delay: false, sample_rate }
    }

    pub fn talk(name: &str, sample_rate: u32, channels: u8) -> Self {
        Self { kind: StreamKind::Talk { name: name.to_string() }, channels, bitrate: 128_000, low_delay: true, sample_rate }
    }

    pub fn voice(sample_rate: u32) -> Self {
        Self { kind: StreamKind::Voice, channels: 1, bitrate: 48_000, low_delay: true, sample_rate }
    }

    pub fn with_bitrate(mut self, bps: u32) -> Self {
        self.bitrate = bps.clamp(64_000, 256_000);
        self
    }

    /// Frames per packet at 48 kHz.
    pub fn frame(&self) -> usize {
        if self.low_delay { 480 } else { 960 }
    }
}

/// One packet on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamPacket {
    pub seq: u32,
    /// Position of the first sample in 48 kHz frames since the stream start.
    pub frame_pos: u64,
    pub opus: Vec<u8>,
}

impl StreamPacket {
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("packet encodes")
    }
    pub fn from_bytes(b: &[u8]) -> Option<StreamPacket> {
        let p: StreamPacket = postcard::from_bytes(b).ok()?;
        if p.opus.len() > MAX_PACKET {
            return None;
        }
        Some(p)
    }
}

pub struct StreamEncoder {
    cfg: StreamConfig,
    enc: Encoder,
    pending: Vec<f32>,
    seq: u32,
    frame_pos: u64,
}

impl StreamEncoder {
    pub fn new(cfg: StreamConfig) -> Result<Self> {
        let ch = match cfg.channels {
            1 => Channels::Mono,
            _ => Channels::Stereo,
        };
        let app = if cfg.low_delay { Application::LowDelay } else { Application::Audio };
        let mut enc = Encoder::new(ch, SampleRate::Hz48000, app).map_err(opus_err)?;
        enc.set_bitrate(Bitrate::Value(cfg.bitrate.clamp(64_000, 256_000))).map_err(opus_err)?;
        enc.set_complexity(if cfg.low_delay { 5 } else { 8 }).map_err(opus_err)?;
        enc.set_inband_fec(opusic_c::InbandFec::Mode1).map_err(opus_err)?;
        enc.set_packet_loss(5).map_err(opus_err)?;
        Ok(Self { cfg, enc, pending: Vec::new(), seq: 0, frame_pos: 0 })
    }

    pub fn config(&self) -> &StreamConfig {
        &self.cfg
    }

    /// Push interleaved samples at `cfg.sample_rate`; get every full packet.
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<StreamPacket>> {
        let chn = self.cfg.channels as usize;
        let at48 = resample(samples, chn, self.cfg.sample_rate, RATE);
        self.pending.extend_from_slice(&at48);
        let frame = self.cfg.frame();
        let mut out = Vec::new();
        let mut pkt = vec![0u8; MAX_PACKET];
        while self.pending.len() >= frame * chn {
            let chunk: Vec<f32> = self.pending.drain(..frame * chn).collect();
            let n = self.enc.encode_float_to_slice(&chunk, &mut pkt).map_err(opus_err)?;
            out.push(StreamPacket { seq: self.seq, frame_pos: self.frame_pos, opus: pkt[..n].to_vec() });
            self.seq = self.seq.wrapping_add(1);
            self.frame_pos += frame as u64;
        }
        Ok(out)
    }

    pub fn latency_frames(&mut self) -> u32 {
        self.enc.get_look_ahead().unwrap_or(0) + self.cfg.frame() as u32
    }
}

pub struct StreamDecoder {
    cfg: StreamConfig,
    dec: Decoder,
    next_seq: Option<u32>,
    pub lost: u64,
}

impl StreamDecoder {
    pub fn new(cfg: StreamConfig) -> Result<Self> {
        let ch = match cfg.channels {
            1 => Channels::Mono,
            _ => Channels::Stereo,
        };
        let dec = Decoder::new(ch, SampleRate::Hz48000).map_err(opus_err)?;
        Ok(Self { cfg, dec, next_seq: None, lost: 0 })
    }

    /// Decode one packet into interleaved samples at `cfg.sample_rate`.
    /// Gaps are concealed (one PLC frame per lost packet, up to 5).
    pub fn decode(&mut self, p: &StreamPacket) -> Result<Vec<f32>> {
        let chn = self.cfg.channels as usize;
        let frame = self.cfg.frame();
        let mut out48 = Vec::with_capacity(frame * chn * 2);
        let mut buf = vec![0f32; frame * 6 * chn];
        if let Some(expected) = self.next_seq {
            let gap = p.seq.wrapping_sub(expected);
            if gap > 0 && gap < 1000 {
                self.lost += gap as u64;
                for _ in 0..gap.min(5) {
                    let n = self.dec.decode_float_to_slice(&[], &mut buf[..frame * chn], false).map_err(opus_err)?;
                    out48.extend_from_slice(&buf[..n * chn]);
                }
            }
        }
        let n = self.dec.decode_float_to_slice(&p.opus, &mut buf, false).map_err(opus_err)?;
        out48.extend_from_slice(&buf[..n * chn]);
        self.next_seq = Some(p.seq.wrapping_add(1));
        Ok(resample(&out48, chn, RATE, self.cfg.sample_rate))
    }

    /// Nothing arrived in time: conceal one frame.
    pub fn conceal(&mut self) -> Result<Vec<f32>> {
        let chn = self.cfg.channels as usize;
        let frame = self.cfg.frame();
        let mut buf = vec![0f32; frame * chn];
        let n = self.dec.decode_float_to_slice(&[], &mut buf, false).map_err(opus_err)?;
        Ok(resample(&buf[..n * chn], chn, RATE, self.cfg.sample_rate))
    }
}

/// Sum several lanes into one stream ("my lanes as one submix").
#[derive(Default)]
pub struct Submix {
    acc: Vec<f32>,
}

impl Submix {
    pub fn add(&mut self, lane: &[f32]) {
        if self.acc.len() < lane.len() {
            self.acc.resize(lane.len(), 0.0);
        }
        for (a, s) in self.acc.iter_mut().zip(lane) {
            *a += s;
        }
    }

    /// Take the mixed block, soft-clipped to stay in range.
    pub fn take(&mut self) -> Vec<f32> {
        let mut v = std::mem::take(&mut self.acc);
        for s in &mut v {
            *s = s.clamp(-1.0, 1.0);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frames: usize, sr: u32, ch: usize) -> Vec<f32> {
        (0..frames).flat_map(|i| std::iter::repeat((i as f32 * 440.0 * std::f32::consts::TAU / sr as f32).sin() * 0.5).take(ch)).collect()
    }

    #[test]
    fn lane_stream_roundtrip_with_loss() {
        let cfg = StreamConfig::lane(&LaneId::new("gtr"), 48_000, 2);
        let mut enc = StreamEncoder::new(cfg.clone()).unwrap();
        let mut dec = StreamDecoder::new(cfg).unwrap();
        let pcm = tone(48_000, 48_000, 2);
        let mut pkts = Vec::new();
        for block in pcm.chunks(512 * 2) {
            pkts.extend(enc.push(block).unwrap());
        }
        assert_eq!(pkts.len(), 50, "1 s = 50 x 20 ms");
        assert!(pkts.iter().all(|p| p.opus.len() < 600));
        // drop packet 10
        let mut out = Vec::new();
        for (i, p) in pkts.iter().enumerate() {
            if i == 10 {
                continue;
            }
            let bytes = p.to_bytes();
            let back = StreamPacket::from_bytes(&bytes).unwrap();
            out.extend(dec.decode(&back).unwrap());
        }
        assert_eq!(dec.lost, 1);
        assert_eq!(out.len(), pcm.len(), "concealment keeps timing");
    }

    #[test]
    fn talk_is_low_delay_and_outside_the_song() {
        let cfg = StreamConfig::talk("guitar", 44_100, 1);
        assert!(!cfg.kind.in_song());
        assert_eq!(cfg.frame(), 480);
        let mut enc = StreamEncoder::new(cfg.clone()).unwrap();
        let mut dec = StreamDecoder::new(cfg).unwrap();
        let pcm = tone(4410, 44_100, 1);
        let pkts = enc.push(&pcm).unwrap();
        assert_eq!(pkts.len(), 10);
        let out: Vec<f32> = pkts.iter().flat_map(|p| dec.decode(p).unwrap()).collect();
        assert!((out.len() as i64 - 4410).abs() <= 10);
        assert!(enc.latency_frames() < 48 * 20, "under 20 ms of codec delay");
        assert!(StreamConfig::lane(&LaneId::new("x"), 48_000, 2).kind.in_song());
    }

    #[test]
    fn submix_sums_lanes() {
        let mut m = Submix::default();
        m.add(&[0.5, 0.5]);
        m.add(&[0.25, -0.5, 1.0]);
        assert_eq!(m.take(), vec![0.75, 0.0, 1.0]);
        assert!(m.take().is_empty());
    }

    #[test]
    fn bad_packets_are_rejected_not_panicking() {
        assert!(StreamPacket::from_bytes(&[]).is_none());
        assert!(StreamPacket::from_bytes(&[0xff; 64]).is_none());
        let cfg = StreamConfig::voice(48_000);
        let mut dec = StreamDecoder::new(cfg).unwrap();
        let junk = StreamPacket { seq: 0, frame_pos: 0, opus: vec![0xff; 100] };
        let _ = dec.decode(&junk); // Err or silence, never a panic
    }
}
