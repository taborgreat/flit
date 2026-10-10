//! The MIDI port the bridge talks SysEx over. Real ports on Windows
//! (midir over winmm, through loopMIDI), an in-memory port for tests.

/// A MIDI port carrying SysEx both ways.
pub trait MidiPort: Send {
    fn send(&mut self, sysex: &[u8]) -> bool;
    /// Everything received since the last call (complete SysEx messages).
    fn recv(&mut self) -> Vec<Vec<u8>>;
    fn is_open(&self) -> bool {
        true
    }
}

/// In-memory port for tests: what the bridge sends lands in `sent`; put
/// FL's answers in `inbox`.
#[derive(Default, Debug)]
pub struct LoopbackPort {
    pub sent: Vec<Vec<u8>>,
    pub inbox: Vec<Vec<u8>>,
}

impl MidiPort for LoopbackPort {
    fn send(&mut self, sysex: &[u8]) -> bool {
        self.sent.push(sysex.to_vec());
        true
    }
    fn recv(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.inbox)
    }
}

/// Default loopMIDI port names. The installer creates them (loopMIDI is
/// dev-only; the release installer ships its own loopback driver).
pub const DEFAULT_IN_PORT: &str = "FLit Out"; // FL's output -> we read
pub const DEFAULT_OUT_PORT: &str = "FLit In"; // we write -> FL's input

#[cfg(windows)]
pub mod real {
    //! midir-backed port. Built only on Windows.
    use super::MidiPort;
    use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
    use std::sync::{Arc, Mutex};

    pub struct MidirPort {
        _inputs: Vec<MidiInputConnection<()>>,
        output: MidiOutputConnection,
        inbox: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl MidirPort {
        /// Open by (substring of) port name. `in_name` is the port FL writes
        /// to (its output), `out_name` the one FL reads (its input).
        ///
        /// Verified on FL 24.1.1: FL routes `midiOutSysex` to whichever
        /// OUTPUT device carries the same port number as the script's input
        /// device. If the user enables "FLit In" as that output instead of
        /// "FLit Out", FL's answers come back on the port we send on. loopMIDI
        /// fans a port out to every reader, so we listen on BOTH ports (our
        /// own requests are filtered by direction in the bridge) and either
        /// mapping works.
        pub fn open(in_name: &str, out_name: &str) -> Result<Self, String> {
            let midi_out = MidiOutput::new("FLit agent").map_err(|e| e.to_string())?;
            let out_port = midi_out
                .ports()
                .into_iter()
                .find(|p| midi_out.port_name(p).map(|n| n.contains(out_name)).unwrap_or(false))
                .ok_or_else(|| format!("no MIDI output port containing \"{out_name}\" (is loopMIDI running?)"))?;
            let inbox = Arc::new(Mutex::new(Vec::new()));
            let mut inputs = Vec::new();
            for (name, required) in [(in_name, true), (out_name, false)] {
                let mut midi_in = MidiInput::new("FLit agent").map_err(|e| e.to_string())?;
                midi_in.ignore(Ignore::None); // we want SysEx
                let port = midi_in.ports().into_iter().find(|p| midi_in.port_name(p).map(|n| n.contains(name)).unwrap_or(false));
                let Some(port) = port else {
                    if required {
                        return Err(format!("no MIDI input port containing \"{name}\" (is loopMIDI running?)"));
                    }
                    continue;
                };
                let ib = inbox.clone();
                // VERIFY ON WINDOWS: winmm delivers each complete SysEx in one callback (midir reassembles MIM_LONGDATA buffers).
                match midi_in.connect(&port, "flit-in", move |_ts, msg, _| {
                    if msg.first() == Some(&0xF0) {
                        ib.lock().unwrap().push(msg.to_vec());
                    }
                }, ()) {
                    Ok(c) => inputs.push(c),
                    Err(e) if required => return Err(e.to_string()),
                    Err(e) => tracing::debug!("could not also listen on {name}: {e}"),
                }
            }
            let output = midi_out.connect(&out_port, "flit-out").map_err(|e| e.to_string())?;
            Ok(Self { _inputs: inputs, output, inbox })
        }
    }

    impl MidiPort for MidirPort {
        fn send(&mut self, sysex: &[u8]) -> bool {
            self.output.send(sysex).is_ok()
        }
        fn recv(&mut self) -> Vec<Vec<u8>> {
            std::mem::take(&mut *self.inbox.lock().unwrap())
        }
    }
}
