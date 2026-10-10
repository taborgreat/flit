//! flit-agent: the background helper. One per user per machine; every
//! plugin instance and every DAW talks to it over local IPC.

use clap::Parser;
use flit_agent::{AgentConfig, AnyBridge, NoBridge};
use flit_bridge_fake::FakeHost;
use flit_host::Level;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "flit-agent", version, about = "FLit background helper")]
struct Args {
    /// Which bridge to run: auto (FL Studio when its loopMIDI ports exist, else none),
    /// none (plugin only, level 0), fake (simulated DAW), fl (FL Studio, Windows).
    #[arg(long, default_value = "auto")]
    bridge: String,
    /// Python with pyflp, for reading FL project files (level 2). Default:
    /// FLIT_PYTHON, the installed venv, the repo .venv, then python on PATH.
    #[arg(long)]
    python: Option<PathBuf>,
    /// Folder holding the DAW's project files (FL: its recent-files list is
    /// consulted too, so this is rarely needed).
    #[arg(long)]
    projects_dir: Option<PathBuf>,
    /// FakeHost level (0-3).
    #[arg(long, default_value_t = 1)]
    fake_level: u8,
    /// Local-only networking: loopback, no relay, no discovery (tests, LAN demos).
    #[arg(long)]
    local: bool,
    /// Never expose my IP to the crew; everything via relay.
    #[arg(long)]
    relay_only: bool,
    /// Where identity, settings and the song list live.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// IPC socket name (default: per-user).
    #[arg(long)]
    socket: Option<String>,
    /// Display name shown to friends.
    #[arg(long)]
    name: Option<String>,
    /// Project folder to open at start.
    #[arg(long)]
    open: Option<PathBuf>,
    /// Use a fixed test identity seed instead of the stored key (tests only).
    #[arg(long)]
    seed: Option<u64>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(flit_agent::default_data_dir);
    std::fs::create_dir_all(&data_dir)?;
    // One agent per data folder. A plugin that gets no answer starts another
    // agent every few seconds; without this they pile up (and each one
    // rotates the log away). The OS drops the lock when the holder dies.
    let lock = std::fs::File::create(data_dir.join("agent.lock"))?;
    if lock.try_lock().is_err() {
        eprintln!("flit-agent is already running for {}", data_dir.display());
        return Ok(());
    }
    // Logs go to stderr and to <data dir>\agent.log (previous run kept as
    // agent.prev.log), whoever started the agent (a terminal or a plugin).
    // RUST_LOG overrides the default filter.
    let log_path = data_dir.join("agent.log");
    if log_path.exists() {
        let _ = std::fs::rename(&log_path, data_dir.join("agent.prev.log"));
    }
    let file = std::fs::File::create(&log_path)?;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("flit=info,flit_agent=info,flit_host=info,flit_net=info,flit_bridge_fl=debug,flit_bridge_fake=info"));
    {
        use tracing_subscriber::fmt::writer::MakeWriterExt;
        tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::io::stderr.and(std::sync::Mutex::new(file))).init();
    }
    tracing::info!("log file {}", log_path.display());
    let key = match args.seed {
        Some(s) => flit_core::Keypair::from_seed(s),
        None => flit_net::IdentityStore::at(data_dir.join("identity.key")).load_or_create()?,
    };
    let bridge = match args.bridge.as_str() {
        "none" => AnyBridge::None(NoBridge::default()),
        "fake" => {
            let level = match args.fake_level {
                0 => Level::PluginOnly,
                1 => Level::LiveControl,
                2 => Level::Project,
                _ => Level::Ai,
            };
            AnyBridge::Fake(FakeHost::new(level))
        }
        "fl" | "auto" => {
            let mut fl = flit_bridge_fl::FlConfig::default();
            if let Some(d) = args.projects_dir.clone() {
                fl.projects_dir = Some(d);
            }
            fl.python = args.python.clone().or_else(flit_bridge_fl::flp::find_python);
            match flit_bridge_fl::FlBridge::open_with_config(fl.clone()) {
                Ok(b) => {
                    tracing::info!("FL Studio bridge on MIDI ports \"{}\" / \"{}\"; project reading: {}", fl.in_port, fl.out_port, fl.python.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "off (no python with pyflp)".into()));
                    AnyBridge::Fl(b)
                }
                Err(e) if args.bridge == "auto" => {
                    tracing::info!("no DAW bridge ({e}); running plugin-only (level 0)");
                    AnyBridge::None(NoBridge::default())
                }
                Err(e) => anyhow::bail!("FL Studio bridge: {e}"),
            }
        }
        other => anyhow::bail!("unknown bridge {other}"),
    };
    let name = args.name.unwrap_or_else(|| std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "me".into()));
    let socket = args.socket.map(flit_ipc::SocketName).unwrap_or_else(flit_ipc::socket_name);
    tracing::info!("flit-agent {} id {} socket {}", flit_agent::agent::VERSION, key.id().short_code(), socket.0);
    let cfg = AgentConfig { key, display_name: name, data_dir, bridge, local_only: args.local, relay_only: args.relay_only, open_song: args.open };
    flit_agent::run(cfg, socket).await
}
