//! flit-agent as a library (so tests and tools can run agents in-process).
pub mod agent;
pub mod bridge;
pub mod store;

pub use agent::{Agent, AgentConfig};
pub use bridge::{AnyBridge, NoBridge};

use std::path::PathBuf;

/// Default per-user data folder.
pub fn default_data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("flit")
}

/// Start an agent and serve IPC on `socket`. Returns once the agent is
/// asked to shut down.
pub async fn run(cfg: AgentConfig, socket: flit_ipc::SocketName) -> anyhow::Result<()> {
    let agent = Agent::start(cfg).await?;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let a2 = agent.clone();
    let sock_name = socket.0.clone();
    let server = tokio::spawn(async move { flit_ipc::serve(socket, a2, ready_tx).await });
    let handle = match ready_rx.await {
        Ok(h) => h,
        Err(_) => {
            // serve() returned before it was ready: say why.
            let why = match server.await {
                Ok(Err(e)) => e.to_string(),
                Ok(Ok(())) => "stopped".into(),
                Err(e) => e.to_string(),
            };
            anyhow::bail!("could not listen on {sock_name}: {why}");
        }
    };
    agent.set_handle(handle);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if agent.state.lock().unwrap().shutdown {
            break;
        }
        if server.is_finished() {
            anyhow::bail!("ipc server stopped");
        }
    }
    server.abort();
    let node = agent.state.lock().unwrap().node.handle();
    node.shutdown().await;
    Ok(())
}
