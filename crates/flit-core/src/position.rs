//! Host-neutral musical time.

use serde::{Deserialize, Serialize};

/// Resolution of song positions. 960 ticks per quarter note.
pub const TICKS_PER_BEAT: i64 = 960;

/// A position in the song, in ticks from the start. Host-neutral.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct SongPos {
    pub ticks: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
pub struct TimeSig {
    pub numerator: u8,
    pub denominator: u8,
}

impl Default for TimeSig {
    fn default() -> Self {
        Self { numerator: 4, denominator: 4 }
    }
}

impl TimeSig {
    /// Ticks in one bar.
    pub fn ticks_per_bar(&self) -> i64 {
        // one beat of the denominator = 4/denominator quarter notes
        TICKS_PER_BEAT * 4 * self.numerator as i64 / self.denominator as i64
    }
}

impl SongPos {
    pub const START: SongPos = SongPos { ticks: 0 };

    pub fn from_ticks(ticks: i64) -> Self {
        Self { ticks }
    }

    pub fn from_beats(beats: f64) -> Self {
        Self { ticks: (beats * TICKS_PER_BEAT as f64).round() as i64 }
    }

    pub fn beats(&self) -> f64 {
        self.ticks as f64 / TICKS_PER_BEAT as f64
    }

    /// 0-based (bar, beat, tick-within-beat).
    pub fn bar_beat_tick(&self, sig: TimeSig) -> (i64, i64, i64) {
        let tpb = sig.ticks_per_bar();
        let beat_len = TICKS_PER_BEAT * 4 / sig.denominator as i64;
        let bar = self.ticks.div_euclid(tpb);
        let rem = self.ticks.rem_euclid(tpb);
        (bar, rem / beat_len, rem % beat_len)
    }

    pub fn from_bar_beat(bar: i64, beat: i64, sig: TimeSig) -> Self {
        let beat_len = TICKS_PER_BEAT * 4 / sig.denominator as i64;
        Self { ticks: bar * sig.ticks_per_bar() + beat * beat_len }
    }

    /// Human form, 1-based like most DAWs: `5.3` = bar 5 beat 3.
    pub fn display(&self, sig: TimeSig) -> String {
        let (bar, beat, _) = self.bar_beat_tick(sig);
        format!("{}.{}", bar + 1, beat + 1)
    }

    /// Seconds from start at a constant tempo.
    pub fn seconds(&self, bpm: f64) -> f64 {
        self.beats() * 60.0 / bpm
    }

    pub fn from_seconds(secs: f64, bpm: f64) -> Self {
        Self::from_beats(secs * bpm / 60.0)
    }

    /// Samples from start at a constant tempo.
    pub fn samples(&self, bpm: f64, sample_rate: u32) -> i64 {
        (self.seconds(bpm) * sample_rate as f64).round() as i64
    }
}

impl std::fmt::Debug for SongPos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SongPos({})", self.display(TimeSig::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_and_beats() {
        let sig = TimeSig::default();
        let p = SongPos::from_bar_beat(4, 2, sig);
        assert_eq!(p.bar_beat_tick(sig), (4, 2, 0));
        assert_eq!(p.display(sig), "5.3");
        assert_eq!(p.beats(), 18.0);
        assert_eq!(sig.ticks_per_bar(), 4 * TICKS_PER_BEAT);
        let waltz = TimeSig { numerator: 3, denominator: 4 };
        assert_eq!(waltz.ticks_per_bar(), 3 * TICKS_PER_BEAT);
        let six8 = TimeSig { numerator: 6, denominator: 8 };
        assert_eq!(six8.ticks_per_bar(), 3 * TICKS_PER_BEAT);
    }

    #[test]
    fn seconds_and_samples() {
        let p = SongPos::from_beats(120.0);
        assert!((p.seconds(120.0) - 60.0).abs() < 1e-9);
        assert_eq!(p.samples(120.0, 48000), 48000 * 60);
        assert_eq!(SongPos::from_seconds(60.0, 120.0), p);
    }
}
