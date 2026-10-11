//! The virtual MIDI port between the FLit app and the FL script.
//!
//! FL lets third-party code in only through a MIDI controller script, so
//! the bridge's frames (`sysex.rs`: request / reply / event JSON, chunked
//! into SysEx) travel over a virtual MIDI port. The port is ONLY a wire.
//! How it comes to exist is this module's business and nobody else's:
//!
//! - now (dev builds): [`LoopMidiPort`], a pair of loopMIDI ports the
//!   developer created by hand ("FLit In" / "FLit Out"). loopMIDI is
//!   dev-only and is replaced at M6.
//! - M6 (release): `CoreMidiPort` (macOS: the app creates a virtual source
//!   and destination named "FLit"), `WinMidiServicesPort` (an app-owned
//!   loopback endpoint through Windows MIDI Services) with a bundled
//!   teVirtualMIDI driver as the fallback. Same trait; nothing else changes.
//!   FLIT_SPEC.md section 25.
//!
//! Two traits, two sides of the wire: [`HostPort`] is the platform port
//! (open / send / on_receive / close); [`MidiPort`] is what the bridge
//! polls (send / recv). [`Wire`] turns one into the other. Tests implement
//! `MidiPort` directly ([`LoopbackPort`]).

use std::sync::{Arc, Mutex};

/// Base name of the port. The dev pair is "<name> In" / "<name> Out"; the
/// M6 ports are one endpoint called just this.
pub const DEFAULT_PORT: &str = "FLit";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortError(pub String);

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PortError {}

/// Called with every complete SysEx message from FL, from whatever thread
/// the port uses.
pub type ReceiveFn = Box<dyn FnMut(&[u8]) + Send>;

/// A virtual MIDI port the app owns (or, in dev builds, borrows). All
/// platform MIDI code lives behind this trait.
pub trait HostPort: Send {
    /// Open the port called `name`. Until then `send` fails and nothing is
    /// received.
    fn open(&mut self, name: &str) -> Result<(), PortError>;
    /// One complete SysEx message (F0 .. F7) to FL.
    fn send(&mut self, bytes: &[u8]) -> bool;
    /// Where complete SysEx messages from FL go. May be set before or after
    /// `open`; the latest callback wins.
    fn on_receive(&mut self, cb: ReceiveFn);
    fn close(&mut self);
}

/// The bridge's side of the wire: send one SysEx, take what arrived.
pub trait MidiPort: Send {
    fn send(&mut self, sysex: &[u8]) -> bool;
    /// Everything received since the last call (complete SysEx messages).
    fn recv(&mut self) -> Vec<Vec<u8>>;
    fn is_open(&self) -> bool {
        true
    }
}

/// A [`HostPort`] seen as a [`MidiPort`]: messages FL sends queue up until
/// the bridge polls.
pub struct Wire {
    port: Box<dyn HostPort>,
    inbox: Arc<Mutex<Vec<Vec<u8>>>>,
    open: bool,
}

impl Wire {
    pub fn open(mut port: Box<dyn HostPort>, name: &str) -> Result<Wire, PortError> {
        let inbox: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let ib = inbox.clone();
        port.on_receive(Box::new(move |msg| ib.lock().unwrap().push(msg.to_vec())));
        port.open(name)?;
        Ok(Wire { port, inbox, open: true })
    }
}

impl MidiPort for Wire {
    fn send(&mut self, sysex: &[u8]) -> bool {
        self.open && self.port.send(sysex)
    }
    fn recv(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.inbox.lock().unwrap())
    }
    fn is_open(&self) -> bool {
        self.open
    }
}

impl Drop for Wire {
    fn drop(&mut self) {
        self.open = false;
        self.port.close();
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

#[cfg(windows)]
mod loopmidi {
    //! Dev builds: two loopMIDI ports made by hand (the installer writes
    //! loopMIDI's port list), driven through midir over winmm. The only
    //! file that may say "loopMIDI".
    use super::{HostPort, PortError, ReceiveFn};
    use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct LoopMidiPort {
        inputs: Vec<MidiInputConnection<()>>,
        output: Option<MidiOutputConnection>,
        cb: Arc<Mutex<Option<ReceiveFn>>>,
    }

    impl HostPort for LoopMidiPort {
        /// `name` is the base: "<name> In" is what FL reads (we write to it),
        /// "<name> Out" is what FL writes (we read it).
        ///
        /// Verified on FL 24.1.1: FL routes `midiOutSysex` to whichever OUTPUT
        /// device carries the same port number as the script's input device.
        /// If the user enables "FLit In" as that output instead of "FLit Out",
        /// FL's answers come back on the port we send on. loopMIDI fans a port
        /// out to every reader, so we listen on BOTH (our own requests are
        /// told apart by direction in the bridge) and either mapping works.
        fn open(&mut self, name: &str) -> Result<(), PortError> {
            self.close();
            let to_fl = format!("{name} In");
            let from_fl = format!("{name} Out");
            let not_there = |what: &str| PortError(format!("no MIDI {what}: the virtual MIDI port \"{name}\" is not there (dev builds: loopMIDI must run with the ports \"{to_fl}\" and \"{from_fl}\")"));
            let midi_out = MidiOutput::new("FLit").map_err(|e| PortError(e.to_string()))?;
            let out_port = midi_out
                .ports()
                .into_iter()
                .find(|p| midi_out.port_name(p).map(|n| n.contains(&to_fl)).unwrap_or(false))
                .ok_or_else(|| not_there(&format!("output port containing \"{to_fl}\"")))?;
            for (port_name, required) in [(&from_fl, true), (&to_fl, false)] {
                let mut midi_in = MidiInput::new("FLit").map_err(|e| PortError(e.to_string()))?;
                midi_in.ignore(Ignore::None); // we want SysEx
                let port = midi_in.ports().into_iter().find(|p| midi_in.port_name(p).map(|n| n.contains(port_name.as_str())).unwrap_or(false));
                let Some(port) = port else {
                    if required {
                        return Err(not_there(&format!("input port containing \"{port_name}\"")));
                    }
                    continue;
                };
                let cb = self.cb.clone();
                // VERIFY ON WINDOWS: winmm delivers each complete SysEx in one callback (midir reassembles MIM_LONGDATA buffers).
                match midi_in.connect(&port, "flit-in", move |_ts, msg, _| {
                    if msg.first() == Some(&0xF0) {
                        if let Some(cb) = cb.lock().unwrap().as_mut() {
                            cb(msg);
                        }
                    }
                }, ()) {
                    Ok(c) => self.inputs.push(c),
                    Err(e) if required => return Err(PortError(e.to_string())),
                    Err(e) => tracing::debug!("could not also listen on {port_name}: {e}"),
                }
            }
            self.output = Some(midi_out.connect(&out_port, "flit-out").map_err(|e| PortError(e.to_string()))?);
            Ok(())
        }

        fn send(&mut self, bytes: &[u8]) -> bool {
            self.output.as_mut().map(|o| o.send(bytes).is_ok()).unwrap_or(false)
        }

        fn on_receive(&mut self, cb: ReceiveFn) {
            *self.cb.lock().unwrap() = Some(cb);
        }

        fn close(&mut self) {
            self.inputs.clear();
            self.output = None;
        }
    }
}

#[cfg(windows)]
pub use loopmidi::LoopMidiPort;

/// The port this platform has, opened under `name`. The bridge calls this
/// and never names an implementation. M6 adds the macOS and Windows
/// MIDI Services ports here.
pub fn open_platform(name: &str) -> Result<Wire, PortError> {
    #[cfg(windows)]
    {
        Wire::open(Box::new(LoopMidiPort::default()), name)
    }
    #[cfg(not(windows))]
    {
        Err(PortError(format!("no virtual MIDI port for this platform yet (port.rs); wanted \"{name}\"")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A HostPort the test holds the other end of.
    #[derive(Default)]
    struct End {
        opened: Option<String>,
        closed: bool,
        sent: Vec<Vec<u8>>,
        cb: Option<ReceiveFn>,
    }

    #[derive(Clone, Default)]
    struct FakePort(Arc<Mutex<End>>);

    impl HostPort for FakePort {
        fn open(&mut self, name: &str) -> Result<(), PortError> {
            if name.is_empty() {
                return Err(PortError("no name".into()));
            }
            self.0.lock().unwrap().opened = Some(name.to_string());
            Ok(())
        }
        fn send(&mut self, bytes: &[u8]) -> bool {
            self.0.lock().unwrap().sent.push(bytes.to_vec());
            true
        }
        fn on_receive(&mut self, cb: ReceiveFn) {
            self.0.lock().unwrap().cb = Some(cb);
        }
        fn close(&mut self) {
            self.0.lock().unwrap().closed = true;
        }
    }

    #[test]
    fn wire_queues_what_fl_sends_until_the_bridge_polls() {
        let end = FakePort::default();
        let mut wire = Wire::open(Box::new(end.clone()), DEFAULT_PORT).unwrap();
        assert_eq!(end.0.lock().unwrap().opened.as_deref(), Some("FLit"));
        assert!(wire.is_open());
        assert!(wire.send(&[0xF0, 1, 0xF7]));
        assert_eq!(end.0.lock().unwrap().sent, vec![vec![0xF0, 1, 0xF7]]);
        // FL answers on the port's own thread; the bridge sees it at its next poll
        {
            let mut e = end.0.lock().unwrap();
            let cb = e.cb.as_mut().expect("callback registered before open");
            cb(&[0xF0, 2, 0xF7]);
            cb(&[0xF0, 3, 0xF7]);
        }
        assert_eq!(wire.recv(), vec![vec![0xF0, 2, 0xF7], vec![0xF0, 3, 0xF7]]);
        assert!(wire.recv().is_empty(), "taken once");
        drop(wire);
        assert!(end.0.lock().unwrap().closed, "dropping the wire closes the port");
    }

    #[test]
    fn open_failure_is_an_error_not_a_panic() {
        assert_eq!(Wire::open(Box::new(FakePort::default()), "").err(), Some(PortError("no name".into())));
    }
}
