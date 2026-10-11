//! The plugin's compact panel (PACK 2 §8.5). The plugin is a thin
//! connector now: this shows the connection, the song and history, who is
//! here, a slim strip of the latest moments, the instance's mode and name,
//! and [Open FLit], which brings the app window forward. Everything else
//! lives in the app (`panel`).

use crate::client::Shared;
use crate::host::{Host, Mode};
use crate::theme as t;
use flit_core::{ActId, Lens, Row};
use flit_ipc::*;
use std::sync::atomic::Ordering;

#[derive(Default)]
pub struct CompactState {
    loaded: bool,
}

/// The four modes a plugin instance can be in.
pub const PLUGIN_MODES: [Mode; 4] = [Mode::Track, Mode::Receiver, Mode::TalkSend, Mode::TalkReceive];

/// A Track or Receiver instance named "master" is the session link: it
/// sends (or hears) the reference mix instead of a track.
pub fn is_master(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("master")
}

fn hint(mode: Mode) -> &'static str {
    match mode {
        Mode::Track | Mode::Master => "track name, or master",
        Mode::Receiver => "their track, or master",
        Mode::TalkSend => "your name",
        Mode::TalkReceive => "track to record into",
    }
}

pub fn draw(ctx: &egui::Context, host: &dyn Host, st: &mut CompactState, shared: &Shared) {
    // Fonts set on a context exist from the next frame on (see panel::draw).
    let themed_id = egui::Id::new("flit-themed");
    if !ctx.data(|d| d.get_temp::<bool>(themed_id).unwrap_or(false)) {
        ctx.data_mut(|d| d.insert_temp(themed_id, true));
        t::apply(ctx);
        ctx.request_repaint();
        return;
    }
    let connected = shared.connected.load(Ordering::Relaxed);
    let status = shared.status.lock().clone();
    if connected && !st.loaded {
        st.loaded = true;
        shared.act(Request::Beings);
    }
    if !connected {
        st.loaded = false;
    }
    if let Some(song) = status.as_ref().and_then(|s| s.song.as_ref()) {
        shared.want_timeline(song.history, Lens::Song, false);
    }
    let mode = host.mode();
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t::BG).inner_margin(egui::Margin::same(t::PAD as i8))).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(t::GAP, 6.0);
        // 1. connection, song, history, [Open FLit]
        ui.horizontal(|ui| {
            let (color, word) = match (&status, connected) {
                (_, false) => (t::WARN, "starting FLit".to_string()),
                (Some(s), true) => match s.bridge_up {
                    Some(true) => (t::OK, format!("{} linked", s.host)),
                    Some(false) => (t::WARN, "DAW not answering".to_string()),
                    None => (t::OK, "connected".to_string()),
                },
                (None, true) => (t::OK, "connected".to_string()),
            };
            t::dot(ui, color);
            ui.label(egui::RichText::new("FLit").font(t::text_medium(t::S14)));
            match status.as_ref().and_then(|s| s.song.as_ref()) {
                Some(song) => {
                    ui.label(egui::RichText::new(&song.name).font(t::text_medium(t::S12)));
                    t::muted(ui, &format!("· {}", song.history_name));
                }
                None => t::muted(ui, "no song open"),
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if t::primary(ui, "Open FLit").on_hover_text("The FLit app: songs, albums, orgs, the full timeline").clicked() {
                    shared.act(Request::OpenWindow { song: None, spawn: true });
                }
                t::small(ui, &word);
            });
        });
        // 2. mode and name
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("compact-mode").selected_text(egui::RichText::new(mode.name()).font(t::text(t::S12))).width(140.0).show_ui(ui, |ui| {
                for m in PLUGIN_MODES {
                    if ui.selectable_label(mode == m, m.name()).clicked() {
                        host.set_mode(m);
                    }
                }
            });
            let mut name = host.track();
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(170.0).hint_text(egui::RichText::new(hint(mode)).color(t::FAINT)).font(t::text(t::S12))).changed() {
                host.set_track(name.clone());
            }
            if matches!(mode, Mode::Track | Mode::Receiver) && is_master(&name) {
                t::small(ui, if mode == Mode::Receiver { "hears the reference mix" } else { "session link: sends the reference mix" });
            }
        });
        // 3. who is here
        let beings = shared.beings.lock().clone();
        ui.horizontal(|ui| {
            t::muted(ui, "here");
            let mut any = false;
            for b in beings.iter().filter(|b| b.online && !b.revoked) {
                any = true;
                let initial = b.display_name.chars().next().unwrap_or('?');
                let mut tip = format!("{} ({})", b.display_name, b.kind);
                if let Some(p) = &b.presence {
                    if let Some(tr) = &p.track {
                        tip.push_str(&format!("\nlooking at {tr}"));
                    }
                }
                t::avatar(ui, b.color, initial, b.mine, 18.0).on_hover_text(tip);
            }
            if !any {
                t::small(ui, if connected { "nobody else right now" } else { "" });
            }
        });
        // 4. the slim strip
        let rows = shared.timeline.lock().clone();
        strip(ui, &rows, status.as_ref().and_then(|s| s.previewing));
        // 5. this DAW's project vs the song: only ever the current one;
        // a project that is not a song yet can become one from here
        if let Some(s) = &status {
            match &s.link {
                ProjectLink::Unlinked { project } => {
                    ui.horizontal(|ui| {
                        let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        t::small(ui, &format!("{file} is not a song yet."));
                        if t::button(ui, "Make it a song").clicked() {
                            if let Some(dir) = project.parent() {
                                let name = project.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Song".into());
                                shared.act(Request::ShareProject { name, dir: dir.to_path_buf(), project: project.clone() });
                            }
                        }
                    });
                }
                ProjectLink::Mismatch { project, song_name, .. } => {
                    let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(egui::RichText::new(format!("{file} is open here, but FLit has \"{song_name}\" open. Edits pause until they match (switch in the app).")).font(t::text(t::S11)).color(t::WARN));
                }
                _ => {}
            }
        }
        // 6. one line of state
        if let Some(s) = &status {
            if let Some(song) = &s.song {
                let unsealed = if song.unsealed > 0 { format!(" · {} unsealed", song.unsealed) } else { String::new() };
                t::small(ui, &format!("{} moments{} · {}", song.moments, unsealed, match s.connection {
                    Connection::Direct => "direct",
                    Connection::Relayed => "relayed",
                    Connection::Offline => "offline",
                }));
            } else if connected {
                t::small(ui, "Open or make a song in the FLit app.");
            }
            if let Some(e) = shared.connect_error.lock().as_ref().filter(|_| !connected) {
                t::small(ui, e);
            }
        }
    });
}

/// Live marks only: the latest moments as ticks, the live edge at the right.
fn strip(ui: &mut egui::Ui, rows: &[Row], previewing: Option<ActId>) {
    let h = 26.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, t::RADIUS_PANEL as f32, t::SURFACE);
    let base_y = rect.center().y;
    let label_w = 42.0;
    let left = rect.left() + 8.0;
    let right = rect.right() - label_w;
    p.line_segment([egui::pos2(left, base_y), egui::pos2(right, base_y)], egui::Stroke::new(1.0_f32, t::BORDER_HI));
    let n = rows.len().min(60);
    let shown = &rows[rows.len() - n..];
    let step = if n > 1 { (right - left) / (n as f32 - 1.0) } else { 0.0 };
    for (i, r) in shown.iter().enumerate() {
        let x = if n > 1 { left + i as f32 * step } else { right };
        let color = if Some(r.act) == previewing { t::ACCENT } else if r.gate { t::BAD } else if !r.sealed { t::WARN } else { t::TEXT };
        let half = if r.undone { 3.0 } else { 6.0 };
        p.line_segment([egui::pos2(x, base_y - half), egui::pos2(x, base_y + half)], egui::Stroke::new(if Some(r.act) == previewing { 2.0_f32 } else { 1.0_f32 }, color));
    }
    let (word, color) = match previewing {
        None => ("Live", t::OK),
        Some(_) => ("Showing", t::ACCENT),
    };
    p.text(egui::pos2(rect.right() - 6.0, base_y), egui::Align2::RIGHT_CENTER, word, t::text_medium(t::S11), color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{PlainHost, Surface};

    #[test]
    fn compact_panel_draws_headless() {
        let shared = Shared::default();
        let host = PlainHost(Surface::Plugin);
        let ctx = egui::Context::default();
        let mut st = CompactState::default();
        for connected in [false, true] {
            shared.connected.store(connected, Ordering::Relaxed);
            for _ in 0..3 {
                let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 260.0))), ..Default::default() }, |ctx| draw(ctx, &host, &mut st, &shared));
            }
        }
    }
}
