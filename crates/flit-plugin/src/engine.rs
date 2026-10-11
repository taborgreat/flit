//! The audio-thread side. No allocation, no locks, no IPC: rings only.

use nih_plug::prelude::*;
use rtrb::{Consumer, Producer, RingBuffer};

/// The four things an instance can be (PACK 2 §8.5). A Track or Receiver
/// instance named "master" is the session link: it sends (or hears) the
/// reference mix. The panel itself is the FLit app now.
#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Mode {
    #[id = "track"]
    #[name = "Track (send)"]
    Track,
    #[id = "receiver"]
    #[name = "Track (receive)"]
    Receiver,
    #[id = "talksend"]
    #[name = "Talk (send)"]
    TalkSend,
    #[id = "talkrecv"]
    #[name = "Talk (receive)"]
    TalkReceive,
}

/// The worker's ends of the rings.
pub struct Rings {
    /// Interleaved samples captured from the host (audio -> worker).
    pub capture: Consumer<f32>,
    /// Interleaved samples to play (worker -> audio).
    pub play: Producer<f32>,
}

pub struct Engine {
    capture: Producer<f32>,
    play: Consumer<f32>,
    channels: usize,
    /// Count of frames dropped because the worker fell behind (debug).
    pub dropped: u64,
}

impl Engine {
    /// Rings hold ~2 s each, so a slow agent never blocks the audio thread.
    pub fn new(sample_rate: u32, channels: usize, _max_block: usize) -> (Engine, Rings) {
        let cap = (sample_rate as usize * channels * 2).next_power_of_two();
        let (cap_tx, cap_rx) = RingBuffer::<f32>::new(cap);
        let (play_tx, play_rx) = RingBuffer::<f32>::new(cap);
        (Engine { capture: cap_tx, play: play_rx, channels, dropped: 0 }, Rings { capture: cap_rx, play: play_tx })
    }

    /// One block. `connected` = the worker currently has an agent; when it
    /// does not, audio keeps flowing through untouched (addendum 2 §1).
    /// `replace` = a Receiver on the master hearing the reference mix: the
    /// local mix is replaced while connected, heard as is otherwise.
    pub fn process(&mut self, buffer: &mut Buffer, mode: Mode, replace: bool, monitor: bool, gain: f32, connected: bool) {
        let channels = buffer.channels().min(self.channels);
        let sends = matches!(mode, Mode::Track | Mode::TalkSend);
        let receives = matches!(mode, Mode::Receiver | Mode::TalkReceive);
        let replace_output = mode == Mode::Receiver && replace && connected;
        for mut frame in buffer.iter_samples() {
            if sends {
                // Captured at the print point: this plugin sits after the
                // lane's inserts; the host applies fader/pan after us in most
                // DAWs, so the bridge level decides what "post-fader" means.
                for ch in 0..channels {
                    let s = *frame.get_mut(ch).unwrap();
                    if self.capture.push(s).is_err() {
                        self.dropped += 1;
                    }
                }
            }
            if receives {
                for ch in 0..channels {
                    let incoming = self.play.pop().unwrap_or(0.0) * gain;
                    let out = frame.get_mut(ch).unwrap();
                    if replace_output {
                        *out = incoming;
                    } else if monitor {
                        *out += incoming;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Addendum 2 §1-2: the audio path never blocks on the agent. With no
    //! worker draining the rings, audio still passes through untouched, and
    //! received audio only mixes in when it is there.
    use super::*;

    fn run_block(engine: &mut Engine, input: &mut [Vec<f32>], mode: Mode, replace: bool, monitor: bool, connected: bool) {
        let mut buffer = Buffer::default();
        let n = input[0].len();
        unsafe {
            buffer.set_slices(n, |slices| {
                slices.clear();
                for ch in input.iter_mut() {
                    slices.push(std::slice::from_raw_parts_mut(ch.as_mut_ptr(), n));
                }
            });
        }
        engine.process(&mut buffer, mode, replace, monitor, 1.0, connected);
    }

    #[test]
    fn track_mode_passes_audio_through_and_captures() {
        let (mut engine, mut rings) = Engine::new(48_000, 2, 512);
        let mut input = vec![vec![0.25f32; 64], vec![-0.5f32; 64]];
        run_block(&mut engine, &mut input, Mode::Track, false, true, true);
        assert!(input[0].iter().all(|s| *s == 0.25), "output untouched");
        assert!(input[1].iter().all(|s| *s == -0.5));
        let mut captured = Vec::new();
        while let Ok(s) = rings.capture.pop() {
            captured.push(s);
        }
        assert_eq!(captured.len(), 128, "interleaved frames captured for the worker");
        assert_eq!(&captured[..2], &[0.25, -0.5]);
    }

    #[test]
    fn audio_continues_when_the_agent_is_gone() {
        // nobody drains the capture ring: it fills up and we drop, never block
        let (mut engine, _rings) = Engine::new(1_000, 2, 64);
        let mut input = vec![vec![0.1f32; 64], vec![0.1f32; 64]];
        for _ in 0..200 {
            run_block(&mut engine, &mut input, Mode::Track, false, true, false);
        }
        assert!(engine.dropped > 0, "ring overflow is counted, not blocking");
        assert!(input[0].iter().all(|s| *s == 0.1), "audio still passes");
    }

    #[test]
    fn receiver_mixes_in_what_the_worker_delivered() {
        let (mut engine, mut rings) = Engine::new(48_000, 2, 64);
        for i in 0..32 {
            rings.play.push(if i % 2 == 0 { 0.5 } else { -0.5 }).unwrap();
        }
        let mut input = vec![vec![0.1f32; 32], vec![0.1f32; 32]];
        run_block(&mut engine, &mut input, Mode::Receiver, false, true, true);
        // first 16 frames: input + received; after that nothing to pop -> input only
        assert!((input[0][0] - 0.6).abs() < 1e-6);
        assert!((input[1][0] - (-0.4)).abs() < 1e-6);
        assert!((input[0][20] - 0.1).abs() < 1e-6);
        // monitor off: received audio is not mixed in
        for _ in 0..8 {
            rings.play.push(0.9).unwrap();
        }
        let mut input = vec![vec![0.0f32; 4], vec![0.0f32; 4]];
        run_block(&mut engine, &mut input, Mode::Receiver, false, false, true);
        assert!(input[0].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn master_receiver_replaces_output_only_when_connected() {
        let (mut engine, mut rings) = Engine::new(48_000, 2, 64);
        for _ in 0..8 {
            rings.play.push(0.3).unwrap();
        }
        let mut input = vec![vec![1.0f32; 4], vec![1.0f32; 4]];
        run_block(&mut engine, &mut input, Mode::Receiver, true, true, false);
        assert!(input[0].iter().all(|s| (*s - 1.3).abs() < 1e-6), "disconnected: local mix (plus whatever was left) is heard");
        let mut input = vec![vec![1.0f32; 4], vec![1.0f32; 4]];
        for _ in 0..8 {
            rings.play.push(0.3).unwrap();
        }
        run_block(&mut engine, &mut input, Mode::Receiver, true, true, true);
        assert!(input[0].iter().all(|s| (*s - 0.3).abs() < 1e-6), "connected: the reference mix replaces the local one");
    }
}
