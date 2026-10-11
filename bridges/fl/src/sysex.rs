//! The SysEx wire protocol shared with `fl-script/device_FLit.py`.
//! Keep this file and the Python `Codec` class in lockstep; the test
//! `python_codec_matches_rust` checks them against each other.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SYSEX_START: u8 = 0xF0;
pub const SYSEX_END: u8 = 0xF7;
/// 0x7D = MIDI "non-commercial" manufacturer id, then "FLit".
pub const HEADER: [u8; 5] = [0x7D, b'F', b'L', b'i', b't'];
/// Bytes of base64 payload per SysEx message. Windows MIDI buffers are
/// typically 1024 bytes; flstudio-mcp saw 1000 OK / 2000 dropped.
pub const CHUNK_PAYLOAD: usize = 600; // VERIFY ON WINDOWS: raise if 900 survives the virtual MIDI port -> FL -> port (per port kind, FLIT_SPEC.md 25)
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Dir {
    /// agent -> FL
    Request = 1,
    /// FL -> agent, answering a request (same id)
    Reply = 2,
    /// FL -> agent, unsolicited
    Event = 3,
}

impl Dir {
    fn from_u8(b: u8) -> Option<Dir> {
        match b {
            1 => Some(Dir::Request),
            2 => Some(Dir::Reply),
            3 => Some(Dir::Event),
            _ => None,
        }
    }
}

/// One complete (reassembled) message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub dir: Dir,
    /// 14-bit request id (two 7-bit bytes). Events use 0.
    pub id: u16,
    pub json: String,
}

/// Agent -> FL requests. JSON: {"cmd": "...", ...}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum FlRequest {
    Hello { protocol: u32 },
    /// Tempo, transport, track names, which slots hold plugins.
    Snapshot,
    /// Page through a plugin's parameters. Only named ones come back.
    Params { track: i32, slot: i32, from: u32, count: u32 },
    SetParam { track: i32, slot: i32, param: u32, value: f32 },
    SetVolume { track: i32, value: f32 },
    SetPan { track: i32, value: f32 },
    SetMute { track: i32, value: bool },
    /// Channel-rack channel (global index) volume / pan / mute.
    SetChanVolume { chan: i32, value: f32 },
    SetChanPan { chan: i32, value: f32 },
    SetChanMute { chan: i32, value: bool },
    /// Ask FL to save the project (its own Save command).
    Save,
    /// Enable (true) or bypass (false) a mixer effect slot.
    SetSlotEnabled { track: i32, slot: i32, value: bool },
    SetTempo { bpm: f64 },
    Play,
    Stop,
    SetPos { ticks: i64 },
    /// What is the user looking at.
    Focus,
    /// Window title (closest thing to the project file name).
    Title,
    /// Presence tints (0xRRGGBB): restored by the bridge, never saved.
    SetTrackColor { track: i32, color: u32 },
    SetChannelColor { chan: i32, color: u32 },
    /// Follow: select a mixer track / a channel, open a window
    /// ("mixer" | "piano_roll" | "playlist" | "channel_rack").
    SelectTrack { track: i32 },
    SelectChannel { chan: i32 },
    ShowWindow { window: String },
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct FlTrack {
    pub index: i32,
    pub name: String,
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    /// (slot, plugin name) for slots that hold a plugin.
    pub slots: Vec<(i32, String)>,
    /// Track color 0xRRGGBB (FL's own, before any presence tint).
    #[serde(default)]
    pub color: u32,
}

/// A channel-rack channel. `plugin` is the generator's name (None for a
/// sampler / audio clip / layer).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FlChannel {
    pub index: i32,
    pub name: String,
    pub plugin: Option<String>,
    pub insert: i32,
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    #[serde(default)]
    pub color: u32,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct FlParam {
    pub index: u32,
    pub name: String,
    pub value: f32,
    #[serde(default)]
    pub text: String,
}

/// FL -> agent replies. JSON: {"ok": true, ...} or {"ok": false, "error": "..."}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FlReply {
    Error { ok: bool, error: String },
    Hello { ok: bool, api: i32, fl: String, python: String, ppq: i32 },
    Snapshot {
        ok: bool,
        tempo: f64,
        playing: bool,
        pos_ticks: i64,
        ppq: i32,
        tracks: Vec<FlTrack>,
        #[serde(default)]
        channels: Vec<FlChannel>,
    },
    Params { ok: bool, track: i32, slot: i32, total: u32, next: u32, params: Vec<FlParam> },
    Focus {
        ok: bool,
        focus: String,
        track: i32,
        slot: i32,
        #[serde(default)]
        channel: Option<i32>,
    },
    Title { ok: bool, title: String },
    Ok { ok: bool },
}

/// FL -> agent unsolicited events. JSON: {"ev": "...", ...}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum FlEvent {
    /// Script started (or reloaded).
    Hello { api: i32, fl: String, python: String },
    /// Every ~500 ms from OnIdle: cheap transport state, plus FL's
    /// "changed since save" flag (0 = saved) and the window title, which
    /// is how the bridge notices saves and project switches.
    Heartbeat {
        playing: bool,
        tempo: f64,
        pos_ticks: i64,
        #[serde(default)]
        changed: Option<i32>,
        #[serde(default)]
        title: Option<String>,
        /// FL's undo history (count, position): a change = a real edit
        /// happened, even one the dirty flags missed.
        #[serde(default)]
        undo_count: Option<i32>,
        #[serde(default)]
        undo_pos: Option<i32>,
        /// FL's record button. Stop while it is on = a take ended.
        #[serde(default)]
        recording: Option<bool>,
    },
    /// Something changed: FL dirty flags plus which mixer tracks / channels.
    Dirty {
        flags: u32,
        tracks: Vec<i32>,
        #[serde(default)]
        channels: Vec<i32>,
    },
    /// OnProjectLoad finished.
    ProjectLoaded { title: String },
    /// Where the user is: the focused window, the selected mixer track
    /// (-1 = none), the plugin slot (-1), the selected channel.
    Focus {
        focus: String,
        track: i32,
        slot: i32,
        #[serde(default)]
        channel: Option<i32>,
    },
    Log { text: String },
}

fn b64_encode(bytes: &[u8]) -> Vec<u8> {
    data_encoding::BASE64.encode(bytes).into_bytes()
}

fn b64_decode(bytes: &[u8]) -> Option<Vec<u8>> {
    data_encoding::BASE64.decode(bytes).ok()
}

/// Encode one frame into one or more SysEx messages (each F0..F7).
pub fn encode(frame: &Frame) -> Vec<Vec<u8>> {
    let payload = b64_encode(frame.json.as_bytes());
    let chunks: Vec<&[u8]> = if payload.is_empty() { vec![&[][..]] } else { payload.chunks(CHUNK_PAYLOAD).collect() };
    let n = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut m = Vec::with_capacity(c.len() + 12);
            m.push(SYSEX_START);
            m.extend_from_slice(&HEADER);
            m.push(frame.dir as u8);
            m.push(((frame.id >> 7) & 0x7F) as u8);
            m.push((frame.id & 0x7F) as u8);
            m.push(if i + 1 < n { 1 } else { 0 });
            m.extend_from_slice(c);
            m.push(SYSEX_END);
            m
        })
        .collect()
}

/// Parsed pieces of one SysEx message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub dir: Dir,
    pub id: u16,
    pub more: bool,
    pub payload: Vec<u8>,
}

/// Parse one SysEx message (with or without the F0/F7 framing).
pub fn parse_piece(msg: &[u8]) -> Option<Piece> {
    let mut m = msg;
    if m.first() == Some(&SYSEX_START) {
        m = &m[1..];
    }
    if m.last() == Some(&SYSEX_END) {
        m = &m[..m.len() - 1];
    }
    if m.len() < HEADER.len() + 4 || m[..HEADER.len()] != HEADER {
        return None;
    }
    let rest = &m[HEADER.len()..];
    let dir = Dir::from_u8(rest[0])?;
    if rest[1] > 0x7F || rest[2] > 0x7F {
        return None;
    }
    let id = ((rest[1] as u16) << 7) | rest[2] as u16;
    let more = match rest[3] {
        0 => false,
        1 => true,
        _ => return None,
    };
    let payload = rest[4..].to_vec();
    if payload.iter().any(|b| *b > 0x7F) {
        return None;
    }
    Some(Piece { dir, id, more, payload })
}

/// Reassembles chunked messages. One per MIDI port.
#[derive(Default, Debug)]
pub struct Chunker {
    partial: BTreeMap<(u8, u16), Vec<u8>>,
}

impl Chunker {
    /// Feed one SysEx message; get a frame when complete.
    pub fn push(&mut self, msg: &[u8]) -> Option<Frame> {
        let p = parse_piece(msg)?;
        let key = (p.dir as u8, p.id);
        let buf = self.partial.entry(key).or_default();
        buf.extend_from_slice(&p.payload);
        if buf.len() > 4 * 1024 * 1024 {
            self.partial.remove(&key);
            return None;
        }
        if p.more {
            return None;
        }
        let payload = self.partial.remove(&key).unwrap_or_default();
        let json = String::from_utf8(b64_decode(&payload)?).ok()?;
        Some(Frame { dir: p.dir, id: p.id, json })
    }

    pub fn pending(&self) -> usize {
        self.partial.len()
    }
}

pub fn request_frame(id: u16, req: &FlRequest) -> Frame {
    Frame { dir: Dir::Request, id: id & 0x3FFF, json: serde_json::to_string(req).expect("request encodes") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small_and_chunked() {
        let f = request_frame(0x1234, &FlRequest::SetParam { track: 3, slot: 1, param: 17, value: 0.42 });
        let msgs = encode(&f);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0][0], 0xF0);
        assert_eq!(*msgs[0].last().unwrap(), 0xF7);
        assert!(msgs[0][1..].iter().take(msgs[0].len() - 2).all(|b| *b < 0x80), "7-bit clean");
        let mut c = Chunker::default();
        assert_eq!(c.push(&msgs[0]), Some(f.clone()));
        let req: FlRequest = serde_json::from_str(&f.json).unwrap();
        assert!(matches!(req, FlRequest::SetParam { track: 3, slot: 1, param: 17, .. }));

        // a long reply gets chunked and reassembled, even interleaved with another id
        let params: Vec<FlParam> = (0..120).map(|i| FlParam { index: i, name: format!("Parameter number {i} with a long name"), value: i as f32 / 120.0, text: format!("{i} dB") }).collect();
        let reply = FlReply::Params { ok: true, track: 1, slot: 0, total: 4240, next: 120, params };
        let big = Frame { dir: Dir::Reply, id: 7, json: serde_json::to_string(&reply).unwrap() };
        let msgs = encode(&big);
        assert!(msgs.len() > 5, "{} chunks", msgs.len());
        assert!(msgs.iter().all(|m| m.len() <= CHUNK_PAYLOAD + 11));
        let other = encode(&Frame { dir: Dir::Event, id: 0, json: "{\"ev\":\"heartbeat\",\"playing\":false,\"tempo\":120.0,\"pos_ticks\":0}".into() });
        assert!(matches!(serde_json::from_str::<FlEvent>("{\"ev\":\"heartbeat\",\"playing\":false,\"tempo\":120.0,\"pos_ticks\":0}").unwrap(), FlEvent::Heartbeat { changed: None, title: None, .. }), "old scripts without the new fields still parse");
        let mut c = Chunker::default();
        let mut got = Vec::new();
        for (i, m) in msgs.iter().enumerate() {
            if i == 2 {
                got.extend(c.push(&other[0]));
            }
            got.extend(c.push(m));
        }
        assert_eq!(got.len(), 2);
        assert_eq!(got[1], big);
        let back: FlReply = serde_json::from_str(&got[1].json).unwrap();
        assert!(matches!(back, FlReply::Params { total: 4240, .. }));
        assert_eq!(c.pending(), 0);
    }

    #[test]
    fn garbage_is_rejected() {
        let mut c = Chunker::default();
        for m in [vec![], vec![0xF0, 0xF7], vec![0xF0, 0x7D, b'F', b'L', b'i', b't', 9, 0, 0, 0, 0xF7], vec![0xFF; 50], vec![0xF0, 0x7D, b'F', b'L', b'i', b't', 1, 0, 0, 0, 0x80, 0xF7]] {
            assert_eq!(c.push(&m), None);
        }
        // valid framing, invalid base64 -> None, nothing stuck
        let m = [&[0xF0u8][..], &HEADER, &[2, 0, 1, 0], b"!!!!", &[0xF7]].concat();
        assert_eq!(c.push(&m), None);
        assert_eq!(c.pending(), 0);
    }

    #[test]
    fn reply_variants_decode() {
        let j = r#"{"ok":false,"error":"nope"}"#;
        assert!(matches!(serde_json::from_str::<FlReply>(j).unwrap(), FlReply::Error { .. }));
        let j = r#"{"ok":true,"tempo":128.0,"playing":true,"pos_ticks":384,"ppq":96,"tracks":[{"index":1,"name":"Drums","volume":0.8,"pan":0.0,"mute":false,"slots":[[0,"Fruity Limiter"]]}]}"#;
        assert!(matches!(serde_json::from_str::<FlReply>(j).unwrap(), FlReply::Snapshot { tempo, .. } if tempo == 128.0));
        let j = r#"{"ev":"dirty","flags":4096,"tracks":[1,2]}"#;
        assert!(matches!(serde_json::from_str::<FlEvent>(j).unwrap(), FlEvent::Dirty { flags: 4096, .. }));
        let j = r#"{"ok":true}"#;
        assert!(matches!(serde_json::from_str::<FlReply>(j).unwrap(), FlReply::Ok { ok: true }));
    }
}

#[cfg(test)]
mod python_lockstep {
    //! The Python `Codec` in fl-script/device_FLit.py writes test vectors when
    //! its own tests run (`codec_vectors.json`). If the file is there, the
    //! Rust codec must decode exactly what Python encoded, and vice versa.
    use super::*;

    #[derive(serde::Deserialize)]
    struct Vectors {
        small: Vec<Vec<u8>>,
        small_json: String,
        chunked: Vec<Vec<u8>>,
        chunked_json: String,
    }

    #[test]
    fn python_codec_matches_rust() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fl-script/codec_vectors.json");
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("no codec_vectors.json (run the Python tests first); skipping");
            return;
        };
        let v: Vectors = serde_json::from_str(&text).unwrap();
        let mut c = Chunker::default();
        let f = c.push(&v.small[0]).expect("python small frame decodes");
        assert_eq!(f, Frame { dir: Dir::Request, id: 0x1234, json: v.small_json.clone() });
        // byte-for-byte: Rust encodes the same message identically
        assert_eq!(encode(&f), v.small);
        let mut got = None;
        for m in &v.chunked {
            got = c.push(m);
        }
        let f = got.expect("python chunked frame decodes");
        assert_eq!(f, Frame { dir: Dir::Event, id: 0, json: v.chunked_json.clone() });
        assert_eq!(encode(&f), v.chunked);
    }
}
