//! Blocking client for the plugin's worker thread and the CLI. Reconnects
//! with backoff and can start the agent when none is running.

use crate::framing::{read_sync, write_sync};
use crate::proto::*;
use crate::SocketName;
use interprocess::local_socket::{prelude::*, Stream};
use interprocess::TryClone;
use std::collections::HashMap;
use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ClientOptions {
    pub socket: SocketName,
    pub kind: ClientKind,
    pub sample_rate: u32,
    pub channels: u8,
    /// Executable to start when no agent answers (None = don't start).
    pub agent_exe: Option<std::path::PathBuf>,
    pub agent_args: Vec<String>,
    pub connect_timeout: Duration,
}

impl ClientOptions {
    pub fn new(kind: ClientKind) -> Self {
        Self { socket: crate::socket_name(), kind, sample_rate: 48_000, channels: 2, agent_exe: None, agent_args: vec![], connect_timeout: Duration::from_secs(5) }
    }
}

type Pending = Arc<Mutex<HashMap<u32, mpsc::Sender<Response>>>>;

pub struct Client {
    stream: Stream,
    next_id: u32,
    pending: Pending,
    push_rx: mpsc::Receiver<Push>,
    reader: Option<std::thread::JoinHandle<()>>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

impl Client {
    /// Connect once (no agent start).
    pub fn connect(opts: &ClientOptions) -> io::Result<Client> {
        let name = crate::server::to_name(&opts.socket)?;
        let stream = Stream::connect(name)?;
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (push_tx, push_rx) = mpsc::channel();
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let rs = stream.try_clone()?;
        let p2 = pending.clone();
        let a2 = alive.clone();
        let stop = alive.clone();
        let reader = std::thread::Builder::new().name("flit-ipc-reader".into()).spawn(move || {
            let mut rs = rs;
            loop {
                if !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let frame: io::Result<Frame> = read_sync(&mut rs);
                match frame {
                    Ok(Frame { id, body: Body::Response(r) }) => {
                        if let Some(tx) = p2.lock().unwrap().remove(&id) {
                            let _ = tx.send(r);
                        }
                    }
                    Ok(Frame { body: Body::Push(p), .. }) => {
                        if push_tx.send(p).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            a2.store(false, std::sync::atomic::Ordering::SeqCst);
            // wake every waiter
            p2.lock().unwrap().clear();
        })?;
        let mut c = Client { stream, next_id: 1, pending, push_rx, reader: Some(reader), alive };
        let hello = Request::Hello { kind: opts.kind.clone(), sample_rate: opts.sample_rate, channels: opts.channels, protocol: PROTOCOL_VERSION };
        match c.request_timeout(hello, opts.connect_timeout)? {
            Response::Ok => Ok(c),
            Response::Error(e) => Err(io::Error::new(io::ErrorKind::ConnectionRefused, e)),
            other => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unexpected hello reply {other:?}"))),
        }
    }

    /// Connect, starting the agent if nothing answers. Blocks up to
    /// `opts.connect_timeout` after a start attempt.
    pub fn connect_or_start(opts: &ClientOptions) -> io::Result<Client> {
        if let Ok(c) = Self::connect(opts) {
            return Ok(c);
        }
        let Some(exe) = &opts.agent_exe else {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "no agent running and no agent executable configured"));
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.args(&opts.agent_args);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        }
        cmd.spawn()?;
        let deadline = Instant::now() + opts.connect_timeout;
        loop {
            match Self::connect(opts) {
                Ok(c) => return Ok(c),
                Err(e) if Instant::now() >= deadline => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Send a request and wait for its response.
    pub fn request(&mut self, req: Request) -> io::Result<Response> {
        self.request_timeout(req, Duration::from_secs(30))
    }

    pub fn request_timeout(&mut self, req: Request, timeout: Duration) -> io::Result<Response> {
        if !self.is_alive() {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "agent connection lost"));
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        write_sync(&mut self.stream, &Frame { id, body: Body::Request(req) })?;
        match rx.recv_timeout(timeout) {
            Ok(r) => Ok(r),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(io::Error::new(io::ErrorKind::TimedOut, "agent did not answer"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(io::ErrorKind::NotConnected, "agent connection lost")),
        }
    }

    /// Fire and forget (audio blocks). The response is dropped when it comes.
    pub fn send(&mut self, req: Request) -> io::Result<()> {
        if !self.is_alive() {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "agent connection lost"));
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        write_sync(&mut self.stream, &Frame { id, body: Body::Request(req) })
    }

    /// Next push, if any, without blocking.
    pub fn try_push(&mut self) -> Option<Push> {
        self.push_rx.try_recv().ok()
    }

    pub fn push_timeout(&mut self, timeout: Duration) -> Option<Push> {
        self.push_rx.recv_timeout(timeout).ok()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.stream.flush();
        // The reader blocks in a read on its own handle of the pipe, so
        // closing ours does not wake it. It must be gone before we return:
        // a DAW unloads the plugin DLL right after dropping the plugin, and
        // a thread that wakes up afterwards runs unloaded code (FL Studio:
        // "FLit.clap_unloaded", c0000005). Cancel the read, then join.
        if let Some(r) = self.reader.take() {
            self.alive.store(false, std::sync::atomic::Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                cancel_blocking_io(&r, &self.stream);
                if r.is_finished() {
                    let _ = r.join();
                    break;
                }
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// Make the reader thread's blocking read return an error. Windows: cancel
/// the pending I/O on the pipe (every handle of the pipe shares the file
/// object, so our handle reaches the reader's read) and the reader thread's
/// synchronous I/O. Elsewhere nothing is done: the bounded wait in `Drop`
/// still applies.
#[allow(unused_variables)]
fn cancel_blocking_io(reader: &std::thread::JoinHandle<()>, pipe: &Stream) {
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsHandle, AsRawHandle};
        use windows_sys::Win32::Foundation::HANDLE;
        // SAFETY: valid handles owned by `pipe` and the JoinHandle; a null
        // OVERLAPPED cancels every request on the file object.
        unsafe {
            #[allow(irrefutable_let_patterns)]
            if let Stream::NamedPipe(np) = pipe {
                windows_sys::Win32::System::IO::CancelIoEx(np.as_handle().as_raw_handle() as HANDLE, std::ptr::null());
            }
            windows_sys::Win32::System::IO::CancelSynchronousIo(reader.as_raw_handle() as HANDLE);
        }
    }
}

use std::io::Write;
