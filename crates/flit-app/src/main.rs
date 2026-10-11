//! flit-app: FLit as a standalone app (PACK 2 §7). One process holds the
//! helper (spines, keeper, sync, identity, bridges; `flit_agent::run` on a
//! tokio thread) and, on the main thread, an eframe window drawing the
//! shared screens (`flit_ui::panel`). The window is an ordinary IPC client
//! of the helper (`ClientKind::Window`), the same way the plugin is, so one
//! source of truth stays one.
//!
//! Launch modes:
//! - plain: helper + window
//! - `--background`: helper now, the window the first time someone asks
//!   (a plugin's [Open FLit], or launching FLit again)
//! - `--headless`: helper only (homelab partner), never a window
//! A second launch while FLit runs asks the running helper to bring its
//! window forward; when that helper has no window (a bare `flit-agent`), the
//! second launch becomes its window.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod native;
mod tray;

use clap::Parser;
use eframe::egui;
use flit_ipc::{Client, ClientKind, ClientOptions, Request, Response, SocketName};
use flit_ui::{theme as t, PanelState, Shared, Surface};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "flit-app", version, about = "FLit: work on one song together, each in your own DAW")]
struct Cli {
    #[command(flatten)]
    args: flit_agent::cli::Args,
    /// Start the helper without a window; the window opens when asked for.
    #[arg(long)]
    background: bool,
    /// Helper only, never a window (always-on partner on a homelab).
    #[arg(long)]
    headless: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = cli.args.data_dir();
    let socket = cli.args.socket();
    let (open_tx, open_rx) = mpsc::channel::<Option<PathBuf>>();
    match flit_agent::cli::try_lock(&data_dir)? {
        Some(lock) => {
            flit_agent::cli::init_logging(&data_dir)?;
            let mut cfg = cli.args.config()?;
            let tx = open_tx.clone();
            cfg.window_hook = Some(Arc::new(move |song| {
                let _ = tx.send(song);
            }));
            let sock = socket.clone();
            tracing::info!("flit-app {} socket {}", flit_agent::agent::VERSION, sock.0);
            std::thread::Builder::new().name("flit-helper".into()).spawn(move || {
                let _lock = lock;
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                if let Err(e) = rt.block_on(flit_agent::run(cfg, sock)) {
                    tracing::error!("helper stopped: {e}");
                }
                // The helper was asked to shut down: the app ends with it.
                std::process::exit(0);
            })?;
            if cli.headless {
                loop {
                    std::thread::park();
                }
            }
            let first = if cli.background {
                // Wait until someone wants the window.
                open_rx.recv().unwrap_or(None)
            } else {
                cli.args.open.clone()
            };
            run_window(socket, open_rx, first, true)
        }
        None => {
            // FLit already runs for this data folder: ask it for its window.
            let opts = ClientOptions { socket: socket.clone(), ..ClientOptions::new(ClientKind::Cli) };
            if let Ok(mut c) = Client::connect(&opts) {
                if let Ok(Response::Ok) = c.request(Request::OpenWindow { song: cli.args.open.clone(), spawn: false }) {
                    return Ok(());
                }
            }
            if cli.headless || cli.background {
                return Ok(());
            }
            // The running helper has no window of its own (a bare flit-agent): be it.
            run_window(socket, open_rx, cli.args.open.clone(), false)
        }
    }
}

/// The window, on the main thread, until the user quits. `embedded`: the
/// helper runs in this process, so closing the window only hides it (the
/// plugin needs the helper); a window for someone else's helper just closes.
fn run_window(socket: SocketName, open_rx: mpsc::Receiver<Option<PathBuf>>, first_song: Option<PathBuf>, embedded: bool) -> anyhow::Result<()> {
    let shared = Arc::new(Shared::default());
    if let Some(s) = first_song {
        *shared.open_song.lock() = Some(s);
        shared.show_window.store(true, Ordering::SeqCst);
    }
    let icon = image::load_from_memory(include_bytes!("../../flit-ui/assets/flit_logo_96.png")).ok().map(|img| {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        egui::IconData { rgba: rgba.into_raw(), width: w, height: h }
    });
    let mut viewport = egui::ViewportBuilder::default().with_title("FLit").with_app_id("flit").with_inner_size([1200.0, 760.0]).with_min_inner_size([820.0, 560.0]);
    if let Some(i) = icon {
        viewport = viewport.with_icon(Arc::new(i));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let shared2 = shared.clone();
    eframe::run_native(
        "FLit",
        options,
        Box::new(move |cc| {
            t::apply(&cc.egui_ctx);
            native::remember(raw_window_handle::HasWindowHandle::window_handle(cc));
            let ctx = cc.egui_ctx.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let s3 = shared2.clone();
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                // A hidden window gets no repaints: the OS has to show it
                // before the frame that handles `show_window` can run.
                if s3.show_window.load(Ordering::SeqCst) {
                    native::show();
                }
                ctx.request_repaint();
            });
            let opts = ClientOptions { socket, ..ClientOptions::new(ClientKind::Window) };
            let (s2, st2) = (shared2.clone(), stop.clone());
            let worker = std::thread::Builder::new().name("flit-window-client".into()).spawn(move || flit_ui::client::ui_client_loop(opts, s2, st2, wake)).ok();
            // The tray icon: left click or "Open FLit" shows the window
            // (natively: a hidden window gets no frames), "Quit" ends the app.
            let quit = Arc::new(AtomicBool::new(false));
            let tray = tray::install(cc.egui_ctx.clone(), shared2.clone(), quit.clone());
            Ok(Box::new(App { shared: shared2, state: PanelState::new(), open_rx, embedded, stop, worker, hidden: false, quit, _tray: tray }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("window: {e}"))?;
    let _ = shared;
    Ok(())
}

struct App {
    shared: Arc<Shared>,
    state: PanelState,
    open_rx: mpsc::Receiver<Option<PathBuf>>,
    embedded: bool,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    hidden: bool,
    /// Set by the tray's Quit.
    quit: Arc<AtomicBool>,
    _tray: Option<tray::Tray>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Someone asked for the window (the helper's hook before our client
        // connected, or a push through it).
        let mut show = false;
        while let Ok(song) = self.open_rx.try_recv() {
            if song.is_some() {
                *self.shared.open_song.lock() = song;
            }
            show = true;
        }
        if self.shared.show_window.swap(false, Ordering::SeqCst) {
            show = true;
        }
        if show {
            if let Some(dir) = self.shared.open_song.lock().take() {
                self.shared.act(Request::OpenSong { dir });
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.hidden = false;
        }
        let quit = self.state.quit_requested() || self.quit.load(Ordering::SeqCst);
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Closing the window keeps the helper alive for the plugins; the app
        // comes back from the tray, [Open FLit] or another launch.
        if self.embedded && !quit && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        }
        if !self.hidden {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        flit_ui::panel::draw(ctx, &flit_ui::host::PlainHost(Surface::App), &mut self.state, &self.shared);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
