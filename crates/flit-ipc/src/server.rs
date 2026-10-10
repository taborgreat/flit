//! Tokio server scaffold: accept clients, read requests, call the handler,
//! write responses; let the agent push to any client.

use crate::framing::{read_async, write_async};
use crate::proto::*;
use crate::SocketName;
use interprocess::local_socket::tokio::{prelude::*, Stream};
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub type ConnId = u64;

/// The agent implements this.
pub trait Handler: Clone + Send + Sync + 'static {
    fn on_hello(&self, conn: ConnId, kind: ClientKind, sample_rate: u32, channels: u8) -> impl Future<Output = ()> + Send;
    fn on_request(&self, conn: ConnId, req: Request) -> impl Future<Output = Response> + Send;
    fn on_disconnect(&self, conn: ConnId) -> impl Future<Output = ()> + Send;
}

#[derive(Clone, Default)]
pub struct ServerHandle {
    clients: Arc<Mutex<HashMap<ConnId, (Option<ClientKind>, mpsc::UnboundedSender<Frame>)>>>,
}

impl ServerHandle {
    pub fn push(&self, conn: ConnId, push: Push) {
        if let Some((_, tx)) = self.clients.lock().unwrap().get(&conn) {
            let _ = tx.send(Frame { id: 0, body: Body::Push(push) });
        }
    }

    pub fn broadcast(&self, push: Push) {
        let clients = self.clients.lock().unwrap();
        for (_, tx) in clients.values() {
            let _ = tx.send(Frame { id: 0, body: Body::Push(push.clone()) });
        }
    }

    /// Push to clients of a kind (e.g. the Receiver of one lane).
    pub fn push_where(&self, push: Push, mut pred: impl FnMut(&ClientKind) -> bool) -> usize {
        let clients = self.clients.lock().unwrap();
        let mut n = 0;
        for (kind, tx) in clients.values() {
            if kind.as_ref().map(&mut pred).unwrap_or(false) {
                let _ = tx.send(Frame { id: 0, body: Body::Push(push.clone()) });
                n += 1;
            }
        }
        n
    }

    pub fn clients(&self) -> Vec<(ConnId, Option<ClientKind>)> {
        self.clients.lock().unwrap().iter().map(|(id, (k, _))| (*id, k.clone())).collect()
    }

    pub fn count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }

    fn set_kind(&self, conn: ConnId, kind: ClientKind) {
        if let Some(c) = self.clients.lock().unwrap().get_mut(&conn) {
            c.0 = Some(kind);
        }
    }
}

pub fn to_name(n: &SocketName) -> std::io::Result<Name<'static>> {
    let s = n.0.clone();
    #[cfg(windows)]
    {
        // Named pipes have no file-system names: a path-style socket name
        // (tests, --socket C:\....sock) becomes a pipe name derived from it.
        let s = if n.is_path() {
            let safe: String = s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '.') { c } else { '_' }).collect();
            format!("flit-{}", safe.trim_matches('_'))
        } else {
            s
        };
        return s.to_ns_name::<GenericNamespaced>();
    }
    #[allow(unreachable_code)]
    if n.is_path() || !GenericNamespaced::is_supported() {
        let p = if n.is_path() { std::path::PathBuf::from(s) } else { runtime_dir().join(s) };
        p.to_fs_name::<GenericFilePath>()
    } else {
        s.to_ns_name::<GenericNamespaced>()
    }
}

pub fn runtime_dir() -> std::path::PathBuf {
    dirs::runtime_dir().or_else(dirs::cache_dir).unwrap_or_else(std::env::temp_dir)
}

/// Serve forever (until the future is dropped). Returns the handle first
/// through `ready`.
pub async fn serve<H: Handler>(name: SocketName, handler: H, ready: tokio::sync::oneshot::Sender<ServerHandle>) -> std::io::Result<()> {
    let n = to_name(&name)?;
    let listener = ListenerOptions::new().name(n).try_overwrite(true).reclaim_name(true).create_tokio()?;
    let handle = ServerHandle::default();
    let _ = ready.send(handle.clone());
    let mut next: ConnId = 1;
    loop {
        let conn = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("ipc accept: {e}");
                continue;
            }
        };
        let id = next;
        next += 1;
        let (tx, rx) = mpsc::unbounded_channel();
        handle.clients.lock().unwrap().insert(id, (None, tx));
        let h = handler.clone();
        let hd = handle.clone();
        tokio::spawn(async move {
            connection(id, conn, rx, h, hd.clone()).await;
            hd.clients.lock().unwrap().remove(&id);
        });
    }
}

async fn connection<H: Handler>(id: ConnId, conn: Stream, mut rx: mpsc::UnboundedReceiver<Frame>, handler: H, handle: ServerHandle) {
    let (mut recv, mut send) = conn.split();
    let writer = tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if write_async(&mut send, &f).await.is_err() {
                break;
            }
        }
    });
    loop {
        let frame: Frame = match read_async(&mut recv).await {
            Ok(f) => f,
            Err(_) => break,
        };
        match frame.body {
            Body::Request(Request::Hello { kind, sample_rate, channels, protocol }) => {
                if protocol != PROTOCOL_VERSION {
                    let resp = Frame { id: frame.id, body: Body::Response(Response::Error(format!("protocol {protocol} != {PROTOCOL_VERSION}"))) };
                    let _ = handle.clients.lock().unwrap().get(&id).map(|(_, tx)| tx.send(resp));
                    continue;
                }
                handle.set_kind(id, kind.clone());
                handler.on_hello(id, kind, sample_rate, channels).await;
                let resp = Frame { id: frame.id, body: Body::Response(Response::Ok) };
                let _ = handle.clients.lock().unwrap().get(&id).map(|(_, tx)| tx.send(resp));
            }
            Body::Request(req) => {
                let resp = handler.on_request(id, req).await;
                let f = Frame { id: frame.id, body: Body::Response(resp) };
                let tx = handle.clients.lock().unwrap().get(&id).map(|(_, tx)| tx.clone());
                match tx {
                    Some(tx) => {
                        if tx.send(f).is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ => {}
        }
    }
    writer.abort();
    handler.on_disconnect(id).await;
}
