//! Sample-rate conversion for the Opus path (Opus only runs at 48 kHz).
//! Cubic Hermite interpolation: good enough for a listening copy and a
//! live monitor. The FLAC archive copy is never resampled.

/// Resample interleaved audio. Returns the input untouched when rates match.
pub fn resample(input: &[f32], channels: usize, from_hz: u32, to_hz: u32) -> Vec<f32> {
    if from_hz == to_hz || input.is_empty() || channels == 0 {
        return input.to_vec();
    }
    let in_frames = input.len() / channels;
    let ratio = from_hz as f64 / to_hz as f64;
    let out_frames = ((in_frames as f64) / ratio).round() as usize;
    let mut out = Vec::with_capacity(out_frames * channels);
    let sample = |frame: isize, ch: usize| -> f32 {
        let f = frame.clamp(0, in_frames as isize - 1) as usize;
        input[f * channels + ch]
    };
    for i in 0..out_frames {
        let pos = i as f64 * ratio;
        let f = pos.floor() as isize;
        let t = (pos - f as f64) as f32;
        for ch in 0..channels {
            let p0 = sample(f - 1, ch);
            let p1 = sample(f, ch);
            let p2 = sample(f + 1, ch);
            let p3 = sample(f + 2, ch);
            out.push(hermite(p0, p1, p2, p3, t));
        }
    }
    out
}

#[inline]
fn hermite(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let c0 = p1;
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
    ((c3 * t + c2) * t + c1) * t + c0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_tone_survive() {
        let sr = 44100;
        let n = 44100;
        let input: Vec<f32> = (0..n).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / sr as f32).sin()).collect();
        let out = resample(&input, 1, sr, 48000);
        assert_eq!(out.len(), 48000);
        // the 440 Hz tone is still a 440 Hz tone: check a sample mid-way
        let i = 24000usize;
        let expected = (i as f32 / 48000.0 * 440.0 * std::f32::consts::TAU).sin();
        assert!((out[i] - expected).abs() < 0.01, "{} vs {}", out[i], expected);
        assert_eq!(resample(&input, 1, sr, sr), input);
    }
}
