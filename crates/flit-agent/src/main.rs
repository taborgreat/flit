//! flit-agent: the headless helper. One per user per machine; every plugin
//! instance and every DAW talks to it over local IPC. `flit-app` is the same
//! helper with a window; this binary stays for homelab partners and tests.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "flit-agent", version, about = "FLit background helper (headless)")]
struct Cli {
    #[command(flatten)]
    args: flit_agent::cli::Args,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = cli.args.data_dir();
    let Some(_lock) = flit_agent::cli::try_lock(&data_dir)? else {
        eprintln!("flit-agent is already running for {}", data_dir.display());
        return Ok(());
    };
    flit_agent::cli::init_logging(&data_dir)?;
    let cfg = cli.args.config()?;
    let socket = cli.args.socket();
    tracing::info!("flit-agent {} socket {}", flit_agent::agent::VERSION, socket.0);
    flit_agent::run(cfg, socket).await
}
