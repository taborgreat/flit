//! The settle rule: wait until a control has been still for ~150 ms and
//! keep only the final value. 1000 twists = 1 event. A knob returned to
//! where it started = 0 events.

use flit_core::{DeviceSlot, TrackId, MixerChange, ParamId, PluginIdentity, SongPos, TransportChange};
use std::collections::HashMap;

pub const DEFAULT_SETTLE_MS: u64 = 150;

/// What is being turned.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ControlKey {
    Param { track: TrackId, slot: DeviceSlot, param: ParamId },
    Volume(TrackId),
    Pan(TrackId),
    Mute(TrackId),
    Tempo,
}

/// A settled change ready to become one event.
#[derive(Clone, Debug, PartialEq)]
pub enum Settled {
    Param { track: TrackId, slot: DeviceSlot, plugin: PluginIdentity, param: ParamId, value: f32, pos: SongPos },
    Mixer { track: TrackId, change: MixerChange, pos: SongPos },
    Tempo { bpm: f64, pos: SongPos },
}

#[derive(Clone, Debug)]
struct Gesture {
    /// Value before the gesture began (the last committed one, if known).
    start: Option<f64>,
    latest: f64,
    last_move_ms: u64,
    plugin: Option<PluginIdentity>,
    pos: SongPos,
}

#[derive(Debug)]
pub struct Settler {
    settle_ms: u64,
    committed: HashMap<ControlKey, f64>,
    active: HashMap<ControlKey, Gesture>,
}

impl Default for Settler {
    fn default() -> Self {
        Self::new(DEFAULT_SETTLE_MS)
    }
}

impl Settler {
    pub fn new(settle_ms: u64) -> Self {
        Self { settle_ms, committed: HashMap::new(), active: HashMap::new() }
    }

    /// Tell the settler the value a control already has (from the song
    /// state) so a gesture that returns there produces nothing.
    pub fn prime(&mut self, key: ControlKey, value: f64) {
        self.committed.insert(key, value);
    }

    pub fn committed(&self, key: &ControlKey) -> Option<f64> {
        self.committed.get(key).copied()
    }

    /// A control moved.
    pub fn input(&mut self, key: ControlKey, value: f64, plugin: Option<PluginIdentity>, pos: SongPos, now_ms: u64) {
        let start = self.committed.get(&key).copied();
        let g = self.active.entry(key).or_insert(Gesture { start, latest: value, last_move_ms: now_ms, plugin: None, pos });
        g.latest = value;
        g.last_move_ms = now_ms;
        g.pos = pos;
        if plugin.is_some() {
            g.plugin = plugin;
        }
    }

    pub fn has_pending(&self) -> bool {
        !self.active.is_empty()
    }

    /// Emit every gesture that has been still long enough.
    pub fn flush(&mut self, now_ms: u64) -> Vec<Settled> {
        let settle = self.settle_ms;
        let done: Vec<ControlKey> = self
            .active
            .iter()
            .filter(|(_, g)| now_ms.saturating_sub(g.last_move_ms) >= settle)
            .map(|(k, _)| k.clone())
            .collect();
        let mut out = Vec::new();
        for key in done {
            let g = self.active.remove(&key).unwrap();
            let unchanged = g.start.map(|s| approx_eq(s, g.latest)).unwrap_or(false);
            self.committed.insert(key.clone(), g.latest);
            if unchanged {
                continue;
            }
            let settled = match key {
                ControlKey::Param { track, slot, param } => {
                    let Some(plugin) = g.plugin else { continue };
                    Settled::Param { track, slot, plugin, param, value: g.latest as f32, pos: g.pos }
                }
                ControlKey::Volume(track) => Settled::Mixer { track, change: MixerChange::Volume(g.latest as f32), pos: g.pos },
                ControlKey::Pan(track) => Settled::Mixer { track, change: MixerChange::Pan(g.latest as f32), pos: g.pos },
                ControlKey::Mute(track) => Settled::Mixer { track, change: MixerChange::Mute(g.latest >= 0.5), pos: g.pos },
                ControlKey::Tempo => Settled::Tempo { bpm: g.latest, pos: g.pos },
            };
            out.push(settled);
        }
        out.sort_by_key(|s| match s {
            Settled::Param { track, param, .. } => format!("0{track}{param}"),
            Settled::Mixer { track, .. } => format!("1{track}"),
            Settled::Tempo { .. } => "2".into(),
        });
        out
    }

    /// Settled form of a transport change (tempo settles, the rest is instant).
    pub fn is_settled_kind(change: &TransportChange) -> bool {
        matches!(change, TransportChange::Tempo(_))
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ControlKey {
        ControlKey::Param { track: TrackId::new("d"), slot: DeviceSlot(0), param: ParamId::new("x") }
    }
    fn plug() -> Option<PluginIdentity> {
        Some(PluginIdentity::vst3("A", "B", "1"))
    }

    #[test]
    fn thousand_twists_one_event() {
        let mut s = Settler::new(150);
        for i in 0..1000u64 {
            s.input(key(), (i % 100) as f64 / 100.0, plug(), SongPos::START, i * 10);
        }
        assert!(s.flush(1000 * 10 + 100).is_empty(), "not still yet");
        let out = s.flush(1000 * 10 + 150);
        assert_eq!(out.len(), 1);
        match &out[0] {
            Settled::Param { value, .. } => assert!((value - 0.99).abs() < 1e-6),
            _ => panic!(),
        }
        assert!(!s.has_pending());
    }

    #[test]
    fn returned_to_start_zero_events() {
        let mut s = Settler::new(150);
        s.prime(key(), 0.5);
        s.input(key(), 0.7, plug(), SongPos::START, 0);
        s.input(key(), 0.9, plug(), SongPos::START, 50);
        s.input(key(), 0.5, plug(), SongPos::START, 100);
        assert!(s.flush(300).is_empty());
        // and the committed value is still 0.5
        assert_eq!(s.committed(&key()), Some(0.5));
        // a real change after that works
        s.input(key(), 0.6, plug(), SongPos::START, 400);
        assert_eq!(s.flush(600).len(), 1);
        // moving away and back inside one gesture, with the start learned
        // from the previous settle, is also nothing
        s.input(key(), 0.1, plug(), SongPos::START, 700);
        s.input(key(), 0.6, plug(), SongPos::START, 750);
        assert!(s.flush(1000).is_empty());
    }

    #[test]
    fn two_controls_settle_independently() {
        let mut s = Settler::new(150);
        s.input(ControlKey::Volume(TrackId::new("a")), 0.8, None, SongPos::START, 0);
        s.input(ControlKey::Tempo, 130.0, None, SongPos::START, 100);
        let first = s.flush(200);
        assert_eq!(first.len(), 1);
        assert!(matches!(first[0], Settled::Mixer { .. }));
        let second = s.flush(260);
        assert_eq!(second.len(), 1);
        assert!(matches!(second[0], Settled::Tempo { bpm, .. } if bpm == 130.0));
    }
}
