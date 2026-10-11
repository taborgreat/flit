use flit_ipc::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct Echo {
    hellos: Arc<Mutex<Vec<ClientKind>>>,
    handle: Arc<Mutex<Option<ServerHandle>>>,
}

impl Handler for Echo {
    async fn on_hello(&self, _conn: ConnId, kind: ClientKind, _sr: u32, _ch: u8) {
        self.hellos.lock().unwrap().push(kind);
    }
    async fn on_request(&self, conn: ConnId, req: Request) -> Response {
        match req {
            Request::Ping => Response::Pong,
            Request::Chat { text, .. } => {
                if let Some(h) = self.handle.lock().unwrap().as_ref() {
                    h.push(conn, Push::Warning(format!("echo: {text}")));
                }
                Response::Ok
            }
            Request::Audio { track, pcm, .. } => {
                if let Some(h) = self.handle.lock().unwrap().as_ref() {
                    h.push_where(Push::TrackAudio { track: track.clone(), pcm }, |k| matches!(k, ClientKind::Receiver { track: t } if *t == track));
                }
                Response::Ok
            }
            _ => Response::Error("unsupported".into()),
        }
    }
    async fn on_disconnect(&self, _conn: ConnId) {}
}

/// Dropping a client must take its reader thread with it, at once: a DAW
/// unloads the plugin DLL right after, and a thread that wakes later runs
/// unloaded code. The server only sees the disconnect once every handle of
/// the pipe is closed, so a quick disconnect proves the thread is gone.
#[test]
fn dropping_a_client_ends_its_reader_thread() {
    let dir = tempfile::tempdir().unwrap();
    let sock = SocketName(dir.path().join("drop.sock").to_string_lossy().into_owned());
    let echo = Echo::default();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (e2, s2) = (echo.clone(), sock.clone());
    rt.spawn(async move { serve(s2, e2, ready_tx).await.unwrap() });
    let handle = rt.block_on(ready_rx).unwrap();
    for _ in 0..3 {
        let mut c = Client::connect(&ClientOptions { socket: sock.clone(), ..ClientOptions::new(ClientKind::Panel) }).unwrap();
        assert_eq!(c.request(Request::Ping).unwrap(), Response::Pong);
        assert_eq!(handle.count(), 1);
        let t = std::time::Instant::now();
        drop(c);
        assert!(t.elapsed() < Duration::from_secs(1), "drop took {:?}", t.elapsed());
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while handle.count() != 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(handle.count(), 0, "server still sees the dropped client: its reader thread is alive");
    }
}

#[test]
fn client_server_roundtrip_and_push_routing() {
    let dir = tempfile::tempdir().unwrap();
    let sock = SocketName(dir.path().join("test.sock").to_string_lossy().into_owned());
    let echo = Echo::default();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let e2 = echo.clone();
    let s2 = sock.clone();
    rt.spawn(async move { serve(s2, e2, ready_tx).await.unwrap() });
    let handle = rt.block_on(ready_rx).unwrap();
    *echo.handle.lock().unwrap() = Some(handle.clone());

    let mut panel = Client::connect(&ClientOptions { socket: sock.clone(), ..ClientOptions::new(ClientKind::Panel) }).unwrap();
    let track = flit_core::TrackId::new("gtr");
    let mut rx = Client::connect(&ClientOptions { socket: sock.clone(), ..ClientOptions::new(ClientKind::Receiver { track: track.clone() }) }).unwrap();
    assert_eq!(panel.request(Request::Ping).unwrap(), Response::Pong);
    assert_eq!(handle.count(), 2);
    assert_eq!(panel.request(Request::Chat { text: "hi".into(), pinned: None }).unwrap(), Response::Ok);
    assert_eq!(panel.push_timeout(Duration::from_secs(2)), Some(Push::Warning("echo: hi".into())));
    // audio pushed only to the receiver of that track
    panel.send(Request::Audio { track: track.clone(), pos: flit_core::SongPos::START, pcm: vec![0.1, 0.2] }).unwrap();
    assert_eq!(rx.push_timeout(Duration::from_secs(2)), Some(Push::TrackAudio { track: track.clone(), pcm: vec![0.1, 0.2] }));
    assert!(panel.try_push().is_none());
    // a bad protocol version is refused
    let bad = Client::connect(&ClientOptions { socket: sock.clone(), ..ClientOptions::new(ClientKind::Cli) });
    assert!(bad.is_ok());
    // server goes away: client notices
    drop(rt);
    std::thread::sleep(Duration::from_millis(200));
    assert!(panel.request_timeout(Request::Ping, Duration::from_millis(500)).is_err());
    assert!(!panel.is_alive() || panel.request_timeout(Request::Ping, Duration::from_millis(200)).is_err());
    // no agent + no exe configured -> clear error
    let err = Client::connect_or_start(&ClientOptions { socket: sock, ..ClientOptions::new(ClientKind::Cli) }).err().unwrap();
    assert!(err.to_string().contains("no agent") || err.kind() == std::io::ErrorKind::ConnectionRefused || err.kind() == std::io::ErrorKind::NotFound, "{err}");
}
