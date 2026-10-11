//! Command line and start-up shared by `flit-agent` (headless helper) and
//! `flit-app` (the helper with its window): the arguments, the one-per-data-
//! folder lock, the log file, the bridge choice.

use crate::{AgentConfig, AnyBridge, NoBridge};
use flit_bridge_fake::FakeHost;
use flit_host::Level;
use std::path::PathBuf;

#[derive(clap::Args, Debug, Clone)]
pub struct Args {
    /// Which bridge to run: auto (FL Studio when its virtual MIDI port exists, else none),
    /// none (plugin only, level 0), fake (simulated DAW), fl (FL Studio, Windows).
    #[arg(long, default_value = "auto")]
    pub bridge: String,
    /// Python with pyflp, for reading FL project files (level 2). Default:
    /// FLIT_PYTHON, the installed venv, the repo .venv, then python on PATH.
    #[arg(long)]
    pub python: Option<PathBuf>,
    /// Folder holding the DAW's project files (FL: its recent-files list is
    /// consulted too, so this is rarely needed).
    #[arg(long)]
    pub projects_dir: Option<PathBuf>,
    /// FakeHost level (0-3).
    #[arg(long, default_value_t = 1)]
    pub fake_level: u8,
    /// Local-only networking: loopback, no relay, no discovery (tests, LAN demos).
    #[arg(long)]
    pub local: bool,
    /// Never expose my IP to the crew; everything via relay.
    #[arg(long)]
    pub relay_only: bool,
    /// Where Names, settings and the song list live.
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    /// IPC socket name (default: per-user).
    #[arg(long)]
    pub socket: Option<String>,
    /// Display name for the first being when a Name is created on first run.
    #[arg(long)]
    pub name: Option<String>,
    /// Project folder to open at start.
    #[arg(long)]
    pub open: Option<PathBuf>,
    /// Use a fixed test identity seed instead of the stored Name (tests only).
    #[arg(long)]
    pub seed: Option<u64>,
}

impl Args {
    pub fn data_dir(&self) -> PathBuf {
        self.data_dir.clone().unwrap_or_else(crate::default_data_dir)
    }

    pub fn socket(&self) -> flit_ipc::SocketName {
        self.socket.clone().map(flit_ipc::SocketName).unwrap_or_else(flit_ipc::socket_name)
    }

    /// The agent config for these arguments (opens the bridge).
    pub fn config(&self) -> anyhow::Result<AgentConfig> {
        let bridge = match self.bridge.as_str() {
            "none" => AnyBridge::None(NoBridge::default()),
            "fake" => {
                let level = match self.fake_level {
                    0 => Level::PluginOnly,
                    1 => Level::LiveControl,
                    2 => Level::Project,
                    _ => Level::Ai,
                };
                AnyBridge::Fake(FakeHost::new(level))
            }
            "fl" | "auto" => {
                let mut fl = flit_bridge_fl::FlConfig::default();
                if let Some(d) = self.projects_dir.clone() {
                    fl.projects_dir = Some(d);
                }
                fl.python = self.python.clone().or_else(flit_bridge_fl::flp::find_python);
                match flit_bridge_fl::FlBridge::open_with_config(fl.clone()) {
                    Ok(b) => {
                        tracing::info!("FL Studio bridge on virtual MIDI port \"{}\"; project reading: {}", fl.port, fl.python.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "off (no python with pyflp)".into()));
                        AnyBridge::Fl(b)
                    }
                    Err(e) if self.bridge == "auto" => {
                        tracing::info!("no DAW bridge ({e}); running plugin-only (level 0)");
                        AnyBridge::None(NoBridge::default())
                    }
                    Err(e) => anyhow::bail!("FL Studio bridge: {e}"),
                }
            }
            other => anyhow::bail!("unknown bridge {other}"),
        };
        let name = self.name.clone().unwrap_or_else(|| std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "me".into()));
        Ok(AgentConfig { seed: self.seed, display_name: name, data_dir: self.data_dir(), bridge, local_only: self.local, relay_only: self.relay_only, open_song: self.open.clone(), window_hook: None })
    }
}

/// One helper per data folder. A plugin that gets no answer starts another
/// helper every few seconds; without this they pile up (and each one rotates
/// the log away). The OS drops the lock when the holder dies. `None` = another
/// helper already runs for this folder.
pub fn try_lock(data_dir: &std::path::Path) -> std::io::Result<Option<std::fs::File>> {
    std::fs::create_dir_all(data_dir)?;
    let lock = std::fs::File::create(data_dir.join("agent.lock"))?;
    Ok(if lock.try_lock().is_ok() { Some(lock) } else { None })
}

/// Logs go to stderr and to <data dir>\agent.log (previous run kept as
/// agent.prev.log), whoever started the helper (a terminal, a plugin, the
/// app). RUST_LOG overrides the default filter.
pub fn init_logging(data_dir: &std::path::Path) -> std::io::Result<()> {
    let log_path = data_dir.join("agent.log");
    if log_path.exists() {
        let _ = std::fs::rename(&log_path, data_dir.join("agent.prev.log"));
    }
    let file = std::fs::File::create(&log_path)?;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("flit=info,flit_agent=info,flit_app=info,flit_host=info,flit_net=info,flit_bridge_fl=debug,flit_bridge_fake=info"));
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::io::stderr.and(std::sync::Mutex::new(file))).init();
    tracing::info!("log file {}", log_path.display());
    Ok(())
}

/// The FLit app executable: next to this one, then the per-user install.
pub fn app_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "flit-app.exe" } else { "flit-app" };
    if let Ok(me) = std::env::current_exe() {
        let p = me.with_file_name(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let p = dirs::data_local_dir()?.join("flit").join("bin").join(name);
    p.is_file().then_some(p)
}

/// Start the FLit app (detached) so its window shows `song`.
pub fn spawn_app(song: Option<PathBuf>) -> std::io::Result<()> {
    let Some(exe) = app_exe() else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "flit-app is not installed next to the helper"));
    };
    let mut cmd = std::process::Command::new(exe);
    if let Some(s) = song {
        cmd.arg("--open").arg(s);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    }
    cmd.spawn().map(|_| ())
}
