//! The panel. Plain words only: ID, Song, Crew, Branch, Save version,
//! Go back, New branch, Bring in. Never repo/commit/key.
//!
//! Look: the FLit logo's palette (charcoal mosaic, burgundy script):
//! dark charcoal base, lavender-grey text, burgundy as the one accent,
//! rust for warnings, green for "connected". Spacious cards, a left
//! navigation rail, one idea per card.

use crate::engine::Mode;
use crate::worker::Shared;
use crate::FlitParams;
use flit_core::{BranchId, Kind, LaneId, LaneKind, SongPos, StampId};
use flit_ipc::*;
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, EguiState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

mod tutorial;

// ----- palette (from flit_logo.png) -----
const C_BG: egui::Color32 = egui::Color32::from_rgb(0x15, 0x12, 0x18);
const C_PANEL: egui::Color32 = egui::Color32::from_rgb(0x1d, 0x19, 0x21);
const C_CARD: egui::Color32 = egui::Color32::from_rgb(0x25, 0x20, 0x2a);
const C_FIELD: egui::Color32 = egui::Color32::from_rgb(0x12, 0x0f, 0x15);
const C_BORDER: egui::Color32 = egui::Color32::from_rgb(0x3b, 0x35, 0x40);
const C_TEXT: egui::Color32 = egui::Color32::from_rgb(0xdc, 0xd4, 0xe0);
const C_MUTED: egui::Color32 = egui::Color32::from_rgb(0x99, 0x91, 0x93);
const C_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x83, 0x3e, 0x51);
const C_ACCENT_HOVER: egui::Color32 = egui::Color32::from_rgb(0xa0, 0x4e, 0x66);
const C_ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xd0, 0x8a, 0xa0);
const C_OK: egui::Color32 = egui::Color32::from_rgb(0x6f, 0xa0, 0x72);
const C_WARN: egui::Color32 = egui::Color32::from_rgb(0xc8, 0x7a, 0x44);
const C_BAD: egui::Color32 = egui::Color32::from_rgb(0xd2, 0x5a, 0x5a);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Home,
    Crew,
    Timeline,
    Chat,
    Settings,
    Help,
}

#[derive(Default)]
struct UiState {
    view: View,
    themed: bool,
    logo: Option<egui::TextureHandle>,
    tutorial: tutorial::State,
    new_name: String,
    new_dir: String,
    join_dir: String,
    share_project: String,
    join_code: String,
    friend_code: String,
    friend_name: String,
    chat_input: String,
    pin: Option<StampId>,
    bring_lane: String,
    bring_source: Option<BranchId>,
    keep_both: bool,
    version_name: String,
    branch_name: String,
    loaded: bool,
    qr: Option<(String, Vec<Vec<bool>>)>,
    // timeline lenses (filters on what is shown, never on what is folded)
    lens_name: Option<flit_core::Id>,
    /// None = everything; Some(None) = the song chain; Some(Some(lane)) = one lane
    lens_thing: Option<Option<LaneId>>,
    lens_knobs: bool,
    lens_takes: bool,
    lens_saves: bool,
    lens_chat: bool,
    lens_org: bool,
    lens_init: bool,
    /// scrubber position (index into the filtered rows) and the row it last previewed
    scrub: usize,
    scrubbed: Option<StampId>,
}

pub fn create(params: Arc<FlitParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let egui_state: Arc<EguiState> = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        UiState::default(),
        |_, _| {},
        move |ctx, setter, st| {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            draw(ctx, setter, st, &params, &shared);
        },
    )
}

// ----- theme -----

fn apply_theme(ctx: &egui::Context) {
    // A system font with real weights when there is one (Windows: Segoe UI).
    let mut fonts = egui::FontDefinitions::default();
    for candidate in ["C:\\Windows\\Fonts\\segoeui.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf"] {
        if let Ok(bytes) = std::fs::read(candidate) {
            fonts.font_data.insert("system".into(), Arc::new(egui::FontData::from_owned(bytes)));
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                list.insert(0, "system".into());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = C_BG;
    v.window_fill = C_PANEL;
    v.extreme_bg_color = C_FIELD;
    v.faint_bg_color = C_CARD;
    v.code_bg_color = C_FIELD;
    v.hyperlink_color = C_ACCENT_LIGHT;
    v.override_text_color = Some(C_TEXT);
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.selection.bg_fill = C_ACCENT;
    v.selection.stroke = egui::Stroke::new(1.0_f32, C_ACCENT_LIGHT);
    let r = egui::CornerRadius::same(6);
    v.widgets.noninteractive.bg_fill = C_CARD;
    v.widgets.noninteractive.weak_bg_fill = C_CARD;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, C_BORDER);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, C_TEXT);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x33, 0x2c, 0x38);
    v.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(0x2b, 0x25, 0x30);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, C_BORDER);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, C_TEXT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = C_ACCENT_HOVER;
    v.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0x3d, 0x33, 0x42);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, C_ACCENT_LIGHT);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = C_ACCENT;
    v.widgets.active.weak_bg_fill = C_ACCENT;
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, C_ACCENT_LIGHT);
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
    v.widgets.active.corner_radius = r;
    v.widgets.open.bg_fill = C_CARD;
    v.widgets.open.weak_bg_fill = C_CARD;
    v.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, C_ACCENT);
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, C_TEXT);
    v.widgets.open.corner_radius = r;
    v.window_corner_radius = egui::CornerRadius::same(10);
    v.window_stroke = egui::Stroke::new(1.0_f32, C_BORDER);

    let mut style = (*ctx.style()).clone();
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size = egui::vec2(40.0, 28.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(10);
    style.spacing.indent = 20.0;
    style.spacing.slider_width = 260.0;
    style.spacing.combo_width = 140.0;
    style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(22.0));
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
    style.text_styles.insert(egui::TextStyle::Small, egui::FontId::proportional(12.5));
    style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::monospace(14.0));
    ctx.set_style(style);
}

fn logo(ctx: &egui::Context, st: &mut UiState) -> Option<egui::TextureHandle> {
    if st.logo.is_none() {
        let bytes = include_bytes!("../assets/flit_logo_256.png");
        if let Ok(img) = image::load_from_memory(bytes) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
            st.logo = Some(ctx.load_texture("flit-logo", ci, egui::TextureOptions::LINEAR));
        }
    }
    st.logo.clone()
}

/// A card: one idea, a title, roomy padding.
fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(C_CARD)
        .stroke(egui::Stroke::new(1.0_f32, C_BORDER))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(16))
        .outer_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 12 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(egui::RichText::new(title).size(17.0).strong().color(C_ACCENT_LIGHT));
                ui.add_space(6.0);
            }
            add(ui)
        })
        .inner
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(13.0).color(C_MUTED));
}

fn pill(ui: &mut egui::Ui, color: egui::Color32, text: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.5, color);
        ui.label(egui::RichText::new(text).size(13.0).color(C_TEXT))
    })
    .response
}

fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).color(egui::Color32::WHITE).strong()).fill(C_ACCENT))
}

/// A "Paste" button next to a field: DAWs do not always hand the keyboard
/// to a plugin window; the clipboard always works.
fn paste_button(ui: &mut egui::Ui, target: &mut String) {
    if ui.small_button("Paste").on_hover_text("Paste from the clipboard").clicked() {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(t) = cb.get_text() {
                *target = t.trim().to_string();
            }
        }
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint_text: &str) {
    ui.label(egui::RichText::new(label).size(13.0).color(C_MUTED));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(value).hint_text(hint_text).desired_width(ui.available_width() - 70.0));
        paste_button(ui, value);
    });
}

/// What to call a Name in this song: a nickname or self-set name, else the short code.
fn who(song: Option<&SongStatus>, id: flit_core::Id) -> String {
    song.and_then(|s| s.names.iter().find(|(i, _)| *i == id).map(|(_, n)| n.clone())).unwrap_or_else(|| id.short_code())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// When a moment happened, for people: relative time, with the bar only
/// when the DAW's playhead had moved (stopped at the start it reads 1.1).
fn moment_text(wall: &flit_core::Time) -> String {
    let age = now_ms().saturating_sub(wall.wall_ms) / 1000;
    let when = if age < 5 {
        "just now".to_string()
    } else if age < 60 {
        format!("{age}s ago")
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86_400 {
        format!("{}h {}m ago", age / 3600, (age % 3600) / 60)
    } else {
        format!("{}d ago", age / 86_400)
    };
    if wall.pos.ticks > 0 {
        format!("{when} · bar {}", wall.pos.display(Default::default()))
    } else {
        when
    }
}

fn moment_exact(wall: &flit_core::Time) -> String {
    let secs = wall.wall_ms / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let days = secs / 86_400;
    // civil date from days since 1970 (Howard Hinnant's algorithm)
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC · bar {}", wall.pos.display(Default::default()))
}

fn qr_matrix(text: &str) -> Vec<Vec<bool>> {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else { return vec![] };
    let w = code.width();
    let colors = code.to_colors();
    (0..w).map(|y| (0..w).map(|x| colors[y * w + x] == qrcode::Color::Dark).collect()).collect()
}

fn draw_qr(ui: &mut egui::Ui, m: &[Vec<bool>], size: f32) {
    if m.is_empty() {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, egui::Color32::from_rgb(0xdc, 0xd4, 0xe0));
    let n = m.len() as f32;
    let cell = size / (n + 2.0);
    for (y, row) in m.iter().enumerate() {
        for (x, dark) in row.iter().enumerate() {
            if *dark {
                let min = rect.min + egui::vec2((x as f32 + 1.0) * cell, (y as f32 + 1.0) * cell);
                p.rect_filled(egui::Rect::from_min_size(min, egui::vec2(cell, cell)), 0.0, C_BG);
            }
        }
    }
}

// ----- the frame -----

fn draw(ctx: &egui::Context, setter: &ParamSetter, st: &mut UiState, params: &FlitParams, shared: &Shared) {
    if !st.themed {
        st.themed = true;
        apply_theme(ctx);
    }
    let connected = shared.connected.load(Ordering::Relaxed);
    let status = shared.status.lock().clone();
    if connected && !st.loaded {
        st.loaded = true;
        shared.act(Request::Songs);
        shared.act(Request::Friends);
        shared.act(Request::Branches);
        shared.act(Request::ChatList);
        if let Some(s) = status.as_ref().and_then(|s| s.song.as_ref()) {
            shared.act(Request::History { branch: s.branch });
        }
    }
    if !connected {
        st.loaded = false;
    }
    let logo_tex = logo(ctx, st);

    // top bar: logo, song, branch, link state, mode
    egui::TopBottomPanel::top("top").frame(egui::Frame::new().fill(C_PANEL).inner_margin(egui::Margin::symmetric(16, 10))).show(ctx, |ui| {
        ui.horizontal(|ui| {
            if let Some(t) = &logo_tex {
                ui.image((t.id(), egui::vec2(30.0, 30.0)));
            }
            ui.label(egui::RichText::new("FLit").size(20.0).strong().color(C_ACCENT_LIGHT));
            ui.add_space(12.0);
            if let Some(song) = status.as_ref().and_then(|s| s.song.as_ref()) {
                ui.label(egui::RichText::new(&song.name).size(16.0).strong());
                ui.label(egui::RichText::new(format!("branch: {}", song.branch_name)).size(13.0).color(C_MUTED));
                if let Some(p) = status.as_ref().and_then(|s| s.paused.as_ref()) {
                    pill(ui, C_BAD, "edits paused").on_hover_text(p);
                }
                if song.missing_opus + song.missing_flac > 0 {
                    ui.label(egui::RichText::new(format!("downloading {} chunks", song.missing_opus + song.missing_flac)).size(13.0).color(C_WARN));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("Mode");
                egui::ComboBox::from_id_salt("mode").selected_text(mode_name(params.mode.value())).show_ui(ui, |ui| {
                    for m in [Mode::Master, Mode::Lane, Mode::Receiver, Mode::TalkSend, Mode::TalkReceive] {
                        if ui.selectable_label(params.mode.value() == m, mode_name(m)).clicked() {
                            setter.begin_set_parameter(&params.mode);
                            setter.set_parameter(&params.mode, m);
                            setter.end_set_parameter(&params.mode);
                        }
                    }
                });
                if params.mode.value() != Mode::Master {
                    let mut lane = params.lane.read().clone();
                    if ui.add(egui::TextEdit::singleline(&mut lane).desired_width(120.0).hint_text("name")).changed() {
                        *params.lane.write() = lane;
                    }
                    ui.label(egui::RichText::new(if matches!(params.mode.value(), Mode::TalkSend | Mode::TalkReceive) { "Name" } else { "Lane" }).color(C_MUTED));
                }
                ui.add_space(8.0);
                if connected {
                    if let Some(s) = status.as_ref() {
                        match s.bridge_up {
                            Some(true) => {
                                pill(ui, C_OK, &format!("{} linked", host_name(&s.host))).on_hover_text(&s.bridge);
                            }
                            Some(false) => {
                                pill(ui, C_WARN, &format!("{} not answering", host_name(&s.host))).on_hover_text(&s.bridge);
                            }
                            None => {
                                pill(ui, C_MUTED, "plugin only");
                            }
                        }
                    }
                    pill(ui, C_OK, "helper");
                } else {
                    pill(ui, C_WARN, "starting the helper…");
                    if let Some(e) = shared.connect_error.lock().as_ref() {
                        ui.label(egui::RichText::new(e).size(12.0).color(C_MUTED));
                    }
                }
            });
        });
    });

    // bottom: the last error, dismissable
    if let Some(e) = shared.last_error.lock().clone() {
        egui::TopBottomPanel::bottom("err").frame(egui::Frame::new().fill(C_PANEL).inner_margin(egui::Margin::symmetric(16, 8))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(C_BAD, &e);
                if ui.small_button("✕").clicked() {
                    *shared.last_error.lock() = None;
                }
            });
        });
    }

    let Some(s) = status.as_ref() else {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(C_BG).inner_margin(egui::Margin::same(24))).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                if let Some(t) = &logo_tex {
                    ui.image((t.id(), egui::vec2(96.0, 96.0)));
                }
                ui.add_space(12.0);
                ui.spinner();
                ui.label("Starting the FLit helper…");
                hint(ui, "Audio keeps playing while we connect.");
            });
        });
        return;
    };

    if params.mode.value() != Mode::Master {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(C_BG).inner_margin(egui::Margin::same(20))).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| lane_mode_view(ui, params, s, shared));
        });
        return;
    }

    // First run: the tutorial fills the panel until a Name and a first Being exist.
    let onboarding = st.tutorial.onboarding(s);

    // navigation rail
    egui::SidePanel::left("nav").frame(egui::Frame::new().fill(C_PANEL).inner_margin(egui::Margin::symmetric(12, 16))).resizable(false).exact_width(150.0).show_animated(ctx, !onboarding, |ui| {
        let has_song = s.song.is_some();
        for (view, name, enabled) in [
            (View::Home, "Home", true),
            (View::Crew, "Crew", true),
            (View::Timeline, "Timeline", has_song),
            (View::Chat, "Chat", has_song),
            (View::Settings, "Settings", true),
            (View::Help, "Help", true),
        ] {
            let selected = st.view == view;
            let text = egui::RichText::new(name).size(15.0).color(if selected { egui::Color32::WHITE } else if enabled { C_TEXT } else { C_MUTED });
            let b = egui::Button::new(text).fill(if selected { C_ACCENT } else { egui::Color32::TRANSPARENT }).min_size(egui::vec2(ui.available_width(), 32.0));
            if ui.add_enabled(enabled, b).clicked() {
                st.view = view;
                match view {
                    View::Home => shared.act(Request::Songs),
                    View::Crew => shared.act(Request::Friends),
                    View::Timeline => {
                        shared.act(Request::Branches);
                        if let Some(sg) = &s.song {
                            shared.act(Request::History { branch: sg.branch });
                        }
                    }
                    View::Chat => shared.act(Request::ChatList),
                    View::Settings => {}
                    View::Help => st.tutorial.open(),
                }
            }
        }
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        hint(ui, "You");
        if !s.settings.display_name.trim().is_empty() {
            ui.label(egui::RichText::new(s.settings.display_name.trim()).size(15.0).strong());
        }
        ui.label(egui::RichText::new(&s.short_code).monospace().size(13.0).color(C_MUTED));
        if ui.small_button("Copy ID").clicked() {
            ui.ctx().copy_text(s.id_text.clone());
        }
    });

    // Resizable from inside the plugin window (drag the bottom-right corner):
    // nih-plug asks the host to resize. Wide enough for the timeline by default.
    nih_plug_egui::resizable_window::ResizableWindow::new("flit-window").min_size(egui::vec2(760.0, 520.0)).show(ctx, &params.editor_state, |ui| {
        egui::Frame::new().fill(C_BG).inner_margin(egui::Margin::same(20)).show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match st.view {
                _ if onboarding => {
                    if tutorial::show(ui, &mut st.tutorial, s, shared, logo_tex.as_ref()) {
                        st.view = View::Home;
                    }
                }
                View::Home => home_view(ui, s, shared, st, logo_tex.as_ref()),
                View::Crew => crew_view(ui, s, shared, st),
                View::Timeline => timeline_view(ui, s, shared, st),
                View::Chat => chat_view(ui, s, shared, st),
                View::Settings => settings_view(ui, setter, params, s, shared),
                View::Help => {
                    if tutorial::show(ui, &mut st.tutorial, s, shared, logo_tex.as_ref()) {
                        st.view = View::Home;
                    }
                }
            });
        });
    });
}

/// Side by side when there is room, stacked when there is not.
fn two_columns(ui: &mut egui::Ui, left: impl FnOnce(&mut egui::Ui), right: impl FnOnce(&mut egui::Ui)) {
    if ui.available_width() > 900.0 {
        ui.columns(2, |cols| {
            left(&mut cols[0]);
            right(&mut cols[1]);
        });
    } else {
        left(ui);
        right(ui);
    }
}

fn three_columns(ui: &mut egui::Ui, a: impl FnOnce(&mut egui::Ui), b: impl FnOnce(&mut egui::Ui), c: impl FnOnce(&mut egui::Ui)) {
    if ui.available_width() > 1000.0 {
        ui.columns(3, |cols| {
            a(&mut cols[0]);
            b(&mut cols[1]);
            c(&mut cols[2]);
        });
    } else {
        a(ui);
        b(ui);
        c(ui);
    }
}

/// The DAW project and the song: linked, not yet a song, or a mismatch.
fn link_card(ui: &mut egui::Ui, s: &Status, shared: &Shared) {
    match &s.link {
        ProjectLink::Unknown => {}
        ProjectLink::Linked { project } => {
            card(ui, "", |ui| {
                pill(ui, C_OK, &format!("{} is this song", project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                hint(ui, "The song lives in the project's folder and opens with it.");
            });
        }
        ProjectLink::Unlinked { project } => {
            let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            card(ui, "This project isn't a song yet", |ui| {
                pill(ui, C_WARN, &format!("{} is open in your DAW", file));
                hint(ui, "Make it a song and every knob turn, save and take from now on is kept, in the project's own folder.");
                if primary(ui, &format!("Make {file} a song")).clicked() {
                    let dir = project.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                    let name = project.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Song".into());
                    shared.act(Request::ShareProject { name, dir, project: project.clone() });
                }
            });
        }
        ProjectLink::Mismatch { project, song_dir, song_name } => {
            let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            card(ui, "Edits are paused", |ui| {
                pill(ui, C_BAD, &format!("{} is open in your DAW, but the song \"{}\" belongs to {}", file, song_name, song_dir.display()));
                hint(ui, "Nothing is recorded until the project and the song match, so the wrong song never gets another project's edits.");
                ui.horizontal(|ui| {
                    if primary(ui, &format!("Make {file} a song")).clicked() {
                        let dir = project.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                        let name = project.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Song".into());
                        shared.act(Request::CloseSong);
                        shared.act(Request::ShareProject { name, dir, project: project.clone() });
                    }
                    if ui.button(format!("Close \"{}\"", song_name)).clicked() {
                        shared.act(Request::CloseSong);
                    }
                });
            });
        }
        ProjectLink::OtherFile { project, song_from } => {
            let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            card(ui, "Edits are paused", |ui| {
                pill(ui, C_BAD, &format!("The song in this folder was made from {}, but {} is open", song_from, file));
                hint(ui, "Save it under the song's file name, or put this project in its own folder and make it a song there.");
            });
        }
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Master => "Panel (master track)",
        Mode::Lane => "Send this lane",
        Mode::Receiver => "Hear a friend's lane",
        Mode::TalkSend => "Talk: send this track",
        Mode::TalkReceive => "Talk: receive",
    }
}

fn host_name(tag: &str) -> &str {
    match tag {
        "fl" => "FL Studio",
        "plugin" => "plugin",
        other => other,
    }
}

// ----- views -----

fn home_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut UiState, logo_tex: Option<&egui::TextureHandle>) {
    if let Some(p) = &s.project {
        if st.share_project.is_empty() {
            st.share_project = p.display().to_string();
        }
        if st.new_dir.is_empty() {
            if let Some(d) = p.parent() {
                st.new_dir = d.display().to_string();
            }
        }
    }
    if st.join_dir.is_empty() {
        if let Some(d) = s.project.as_ref().and_then(|p| p.parent()).and_then(|d| d.parent()) {
            st.join_dir = d.display().to_string();
        }
    }
    link_card(ui, s, shared);
    if let Some(song) = &s.song {
        song_card(ui, s, song, shared, st);
    } else {
        ui.horizontal(|ui| {
            if let Some(t) = logo_tex {
                ui.image((t.id(), egui::vec2(72.0, 72.0)));
            }
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Work on one song together, each in your own DAW.").size(18.0));
                hint(ui, "Open a song below, or share the project you have open. Nothing leaves this computer until someone joins.");
            });
        });
        ui.add_space(12.0);
    }

    two_columns(ui, |ui| {
        card(ui, "Your ID", |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&s.short_code).monospace().size(18.0));
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(s.id_text.clone());
                }
            });
            hint(ui, "Safe to share. A friend adds you with it; nobody can join a song without your approval.");
            if st.qr.as_ref().map(|(t, _)| t != &s.id_text).unwrap_or(true) {
                st.qr = Some((s.id_text.clone(), qr_matrix(&s.id_text)));
            }
            if let Some((_, m)) = &st.qr {
                ui.add_space(6.0);
                draw_qr(ui, m, 132.0);
            }
        });
    }, |ui| {
        card(ui, "Songs on this computer", |ui| {
            let songs = shared.songs.lock().clone();
            if songs.is_empty() {
                hint(ui, "None yet.");
            }
            for (name, dir) in songs {
                ui.horizontal(|ui| {
                    if ui.button("Open").clicked() {
                        shared.act(Request::OpenSong { dir: dir.clone() });
                    }
                    ui.label(egui::RichText::new(&name).strong());
                    ui.label(egui::RichText::new(dir.display().to_string()).size(12.0).color(C_MUTED));
                });
            }
        });
    });

    three_columns(ui, |ui| {
        card(ui, "New song", |ui| {
            hint(ui, "Start from nothing. You own it and its mix.");
            field(ui, "Name", &mut st.new_name, "Untitled");
            field(ui, "Folder", &mut st.new_dir, "C:\\...\\my song");
            if primary(ui, "Create").clicked() && !st.new_dir.is_empty() {
                shared.act(Request::NewSong { name: if st.new_name.is_empty() { "Untitled".into() } else { st.new_name.clone() }, dir: st.new_dir.clone().into() });
            }
        });
    }, |ui| {
        card(ui, "Share this project", |ui| {
            hint(ui, "The project you have open becomes a song, version 1.");
            field(ui, "Project file", &mut st.share_project, "C:\\...\\song.flp");
            if primary(ui, "Share").clicked() && !st.share_project.is_empty() {
                let p = std::path::PathBuf::from(&st.share_project);
                let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
                let name = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Song".into());
                shared.act(Request::ShareProject { name, dir, project: p });
            }
        });
    }, |ui| {
        card(ui, "Join with a song code", |ui| {
            hint(ui, "Paste a friend's code. They approve you, then everything syncs.");
            field(ui, "Song code", &mut st.join_code, "flit1…");
            field(ui, "Folder for your copy", &mut st.join_dir, "C:\\...\\their song");
            if primary(ui, "Ask to join").clicked() && !st.join_code.is_empty() && !st.join_dir.is_empty() {
                shared.act(Request::JoinWithCode { code: st.join_code.trim().to_string(), dir: st.join_dir.clone().into() });
                shared.notice("Asked to join. Waiting for the owner…");
            }
        });
    });
    notices(ui, shared);
}

fn song_card(ui: &mut egui::Ui, _s: &Status, song: &SongStatus, shared: &Shared, st: &mut UiState) {
    card(ui, "", |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&song.name).size(24.0).strong());
            ui.add_space(8.0);
            ui.label(egui::RichText::new(format!("{} in the crew", song.members.len())).color(C_MUTED));
            ui.label(egui::RichText::new(format!("branch: {}", song.branch_name)).color(C_MUTED));
            if song.i_own_mix {
                ui.label(egui::RichText::new("you own the mix").color(C_MUTED));
            } else if let Some(m) = song.mix_owner {
                ui.label(egui::RichText::new(format!("mix: {}", m.short_code())).color(C_MUTED));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    shared.act(Request::CloseSong);
                }
                if ui.button("Copy song code").on_hover_text("Give this to someone so they can ask to join").clicked() {
                    ui.ctx().copy_text(song.code.clone());
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.version_name).hint_text("What changed? (version name)").desired_width(300.0));
            paste_button(ui, &mut st.version_name);
            if primary(ui, "Save version").on_hover_text("Saves the project in your DAW and keeps this moment forever").clicked() {
                shared.act(Request::SaveVersion { name: if st.version_name.is_empty() { "Save".into() } else { st.version_name.clone() } });
            }
        });
        if !song.heard_as_audio.is_empty() {
            hint(ui, &format!("Heard as audio (this DAW can't reproduce their settings): {}", song.heard_as_audio.iter().map(|l| l.0.clone()).collect::<Vec<_>>().join(", ")));
        }
    });
}

fn lane_mode_view(ui: &mut egui::Ui, params: &FlitParams, s: &Status, shared: &Shared) {
    let lane = params.lane.read().clone();
    match params.mode.value() {
        Mode::Lane => card(ui, &format!("Sending lane \"{lane}\""), |ui| {
            hint(ui, "Whatever reaches this track is streamed to the crew and captured for every Save version. Put this plugin last on the track: after inserts, before sends.");
            if let Some(song) = &s.song {
                let l = LaneId::new(&lane);
                match song.lanes.iter().find(|x| x.lane == l) {
                    Some(info) if info.mine => {
                        pill(ui, C_OK, "You own this lane");
                    }
                    Some(info) => {
                        pill(ui, C_WARN, &format!("Owned by {}. Your audio is ignored until it is handed to you.", info.owner.map(|o| o.short_code()).unwrap_or_default()));
                    }
                    None => {
                        hint(ui, "Not in the song yet: the first Save version claims it for you.");
                        if ui.button("Take this lane now").clicked() {
                            shared.act(Request::ClaimLane { lane: l.clone(), kind: LaneKind::Part, name: Some(lane.clone()) });
                        }
                    }
                }
            } else {
                hint(ui, "No song open on the FLit panel (master track).");
            }
        }),
        Mode::Receiver => card(ui, &format!("Hearing lane \"{lane}\""), |ui| {
            hint(ui, "A friend's lane plays here when this DAW can't reproduce their settings. Add effects, level it, solo it like any track.");
            if let Some(song) = &s.song {
                if song.heard_as_audio.iter().any(|x| x.0 == lane) {
                    pill(ui, C_OK, "Live");
                }
            }
        }),
        Mode::TalkSend => card(ui, &format!("Talk: sending \"{lane}\""), |ui| {
            hint(ui, "Outside the song: never rendered into versions, no lane owner. Lowest-latency settings. Friends hear it on a Talk receiver in their own mixer.");
        }),
        Mode::TalkReceive => card(ui, "Talk: receiving", |ui| {
            hint(ui, "A friend's live track arrives here. Record it into a lane you own and it becomes a normal take, credited to the player.");
            ui.horizontal(|ui| {
                if primary(ui, "● Record into lane").clicked() {
                    shared.act(Request::RecordTalk { lane: LaneId::new(if lane.is_empty() { "talk take" } else { &lane }), start: true });
                }
                if ui.button("■ Stop").clicked() {
                    shared.act(Request::RecordTalk { lane: LaneId::new(if lane.is_empty() { "talk take" } else { &lane }), start: false });
                }
            });
        }),
        Mode::Master => {}
    }
    notices(ui, shared);
}

fn crew_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut UiState) {
    if !s.pending_joins.is_empty() || !s.invites.is_empty() {
        card(ui, "Waiting for you", |ui| {
            for (id, name) in &s.pending_joins {
                ui.horizontal(|ui| {
                    ui.label(format!("{} ({}) asks to join", if name.trim().is_empty() { id.short_code() } else { name.clone() }, id.short_code()));
                    if primary(ui, "Approve").clicked() {
                        shared.act(Request::Approve { id: *id, owner: false });
                    }
                    if ui.button("Deny").clicked() {
                        shared.act(Request::Deny { id: *id });
                    }
                    if ui.button("Block").clicked() {
                        shared.act(Request::Block { id: *id });
                    }
                });
            }
            for (from, song_id, name) in &s.invites {
                ui.horizontal(|ui| {
                    ui.label(format!("{} invited you to \"{name}\"", from.short_code()));
                    ui.add(egui::TextEdit::singleline(&mut st.new_dir).hint_text("Folder for your copy").desired_width(220.0));
                    paste_button(ui, &mut st.new_dir);
                    if primary(ui, "Accept").clicked() && !st.new_dir.is_empty() {
                        shared.act(Request::AcceptInvite { song: *song_id, dir: st.new_dir.clone().into() });
                    }
                });
            }
        });
    }
    if let Some(song) = &s.song {
        card(ui, &format!("Crew of \"{}\"", song.name), |ui| {
            hint(ui, "Everyone in the song. Owners approve and remove people.");
            for p in &s.peers {
                ui.horizontal(|ui| {
                    pill(ui, if p.connected { C_OK } else { C_MUTED }, &format!("{}  {}{}{}", p.name.clone().unwrap_or_else(|| p.id.short_code()), p.id.short_code(), if p.owner { " · owner" } else { "" }, if p.over_quota { " · OVER QUOTA" } else { "" }));
                    if !p.owner && ui.small_button("Make owner").clicked() {
                        shared.act(Request::Promote { id: p.id });
                    }
                    if ui.small_button("Remove").clicked() {
                        shared.act(Request::RemoveFromCrew { id: p.id });
                    }
                });
            }
            if ui.button("Leave this song").clicked() {
                shared.act(Request::LeaveSong);
            }
        });
    }
    card(ui, "Friends", |ui| {
        hint(ui, "People you know. Add them to a song's crew with one click.");
        for f in shared.friends.lock().clone() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&f.name).strong());
                ui.label(egui::RichText::new(f.id.short_code()).monospace().color(C_MUTED));
                if s.song.is_some() && ui.small_button("Add to crew").clicked() {
                    shared.act(Request::Invite { friend: f.id });
                }
                if ui.small_button("Block").clicked() {
                    shared.act(Request::Block { id: f.id });
                }
            });
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.friend_code).hint_text("Their ID").desired_width(300.0));
            paste_button(ui, &mut st.friend_code);
            ui.add(egui::TextEdit::singleline(&mut st.friend_name).hint_text("Name").desired_width(140.0));
            if primary(ui, "Add friend").clicked() && !st.friend_code.is_empty() {
                shared.act(Request::AddFriend { id: st.friend_code.trim().to_string(), name: st.friend_name.clone() });
                shared.act(Request::Friends);
                st.friend_code.clear();
            }
        });
    });
    notices(ui, shared);
}

/// Which rows a lens keeps. Lenses never change the fold (addendum 4 §1).
fn lens_keeps(st: &UiState, r: &flit_core::HistoryRow) -> bool {
    if let Some(n) = st.lens_name {
        if r.name != n {
            return false;
        }
    }
    match &st.lens_thing {
        None => {}
        Some(None) => {
            if r.lane.is_some() {
                return false;
            }
        }
        Some(Some(l)) => {
            if r.lane.as_ref() != Some(l) {
                return false;
            }
        }
    }
    match r.kind {
        Kind::Setting | Kind::Mixer | Kind::Device | Kind::HostState | Kind::Transport => st.lens_knobs,
        Kind::Notes | Kind::Audio => st.lens_takes,
        Kind::Version => st.lens_saves,
        Kind::Chat => st.lens_chat,
        Kind::Crew | Kind::Lane | Kind::Branch | Kind::Bring | Kind::GoBack | Kind::Undo | Kind::Profile => st.lens_org,
    }
}

fn kind_word(k: Kind) -> &'static str {
    match k {
        Kind::Setting => "knob",
        Kind::Mixer => "fader",
        Kind::Device => "plugin",
        Kind::Transport => "transport",
        Kind::Notes => "notes",
        Kind::Audio => "take",
        Kind::Version => "save",
        Kind::Crew => "crew",
        Kind::Lane => "lane",
        Kind::Branch => "branch",
        Kind::Bring => "bring in",
        Kind::Chat => "chat",
        Kind::HostState => "DAW state",
        Kind::GoBack => "go back",
        Kind::Undo => "undo",
        Kind::Profile => "name",
    }
}

/// The branch tree: Main at the root, every branch under the branch it
/// forked from. Click to switch what you are live on.
fn branch_tree(ui: &mut egui::Ui, branches: &[BranchInfo], rows_for_tree: &[flit_core::HistoryRow], parent: Option<BranchId>, depth: usize, shared: &Shared) {
    for b in branches.iter().filter(|b| b.parent == parent) {
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 22.0);
            let glyph = if depth == 0 { "●" } else { "└" };
            ui.label(egui::RichText::new(glyph).color(if b.current { C_ACCENT_LIGHT } else { C_MUTED }));
            let text = egui::RichText::new(&b.name).size(15.0).color(if b.current { egui::Color32::WHITE } else { C_TEXT });
            let btn = egui::Button::new(text).fill(if b.current { C_ACCENT } else { egui::Color32::TRANSPARENT });
            if ui.add(btn).on_hover_text(if b.current { "You are live here" } else { "Switch to this branch" }).clicked() && !b.current {
                shared.act(Request::SwitchBranch { branch: b.branch });
            }
            if b.from.is_some() {
                let from_row = rows_for_tree.iter().find(|r| Some(r.id) == b.from);
                let text = from_row.map(|r| format!("from {}", moment_text(&r.wall))).unwrap_or_else(|| "from an earlier moment".into());
                ui.label(egui::RichText::new(text).size(12.0).color(C_MUTED));
            }
        });
        branch_tree(ui, branches, rows_for_tree, Some(b.branch), depth + 1, shared);
    }
}

fn timeline_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut UiState) {
    let Some(song) = &s.song else {
        card(ui, "Timeline", |ui| hint(ui, "Open a song to see its timeline."));
        return;
    };
    if !st.lens_init {
        st.lens_init = true;
        st.lens_knobs = true;
        st.lens_takes = true;
        st.lens_saves = true;
        st.lens_chat = true;
        st.lens_org = true;
    }
    let rows = shared.history.lock().clone();
    let branches = shared.branches.lock().clone();
    let shown: Vec<&flit_core::HistoryRow> = rows.iter().filter(|r| lens_keeps(st, r)).collect();
    let previewing = s.previewing;

    two_columns(ui, |ui| {
        card(ui, "Branches", |ui| {
            hint(ui, "Main is the song. A branch is a safe place to try something; bring it in when it works.");
            branch_tree(ui, &branches, &rows, None, 0, shared);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.branch_name).hint_text("name for a new branch").desired_width(180.0));
                paste_button(ui, &mut st.branch_name);
                let from = st.scrubbed.or_else(|| shown.last().map(|r| r.id));
                if let Some(from) = from {
                    if primary(ui, "New branch from this moment").on_hover_text("Starts a branch at the scrubbed row (or the latest)").clicked() {
                        let name = if st.branch_name.trim().is_empty() { format!("branch from {}", shown.iter().find(|r| r.id == from).map(|r| moment_exact(&r.wall)).unwrap_or_default()) } else { st.branch_name.trim().to_string() };
                        shared.act(Request::NewBranch { name, from });
                        st.branch_name.clear();
                    }
                }
            });
        });
    }, |ui| {
        card(ui, "Lanes on this branch", |ui| {
            hint(ui, "One owner per lane. Click a lane to pick it for Bring in.");
            for l in &song.lanes {
                let txt = format!("{}{}  {}", song.lanes.iter().find(|x| x.lane == l.lane).and_then(|x| x.name.clone()).unwrap_or_else(|| l.lane.0.clone()), if l.mine { " ✓" } else { "" }, l.owner.map(|o| who(Some(song), o)).unwrap_or_else(|| "(free)".into()));
                if ui.selectable_label(st.bring_lane == l.lane.0, txt).clicked() {
                    st.bring_lane = l.lane.0.clone();
                }
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Bring in").strong());
                ui.add(egui::TextEdit::singleline(&mut st.bring_lane).hint_text("lane").desired_width(110.0));
                ui.label("from");
                for b in &branches {
                    if b.branch != song.branch && ui.selectable_label(st.bring_source == Some(b.branch), &b.name).clicked() {
                        st.bring_source = Some(b.branch);
                        shared.act(Request::BringPreview { lane: LaneId::new(&st.bring_lane), source_branch: b.branch });
                    }
                }
            });
            if let Some(p) = shared.preview.lock().clone() {
                hint(ui, &format!("{} changes{}{}", p.changes, if p.conflict { " · both sides changed: pick one, or keep both" } else { "" }, if p.flac_needed > 0 { " · fetching full quality first" } else { "" }));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut st.keep_both, "keep both");
                    if p.allowed {
                        if primary(ui, "Bring in").clicked() {
                            if let Some(src) = st.bring_source {
                                shared.act(Request::BringIn { lane: LaneId::new(&st.bring_lane), source_branch: src, keep_both_as: if st.keep_both { Some(LaneId::new(&format!("{} 2", st.bring_lane))) } else { None } });
                                *shared.preview.lock() = None;
                            }
                        }
                    } else if ui.button("Offer it").on_hover_text("You don't own this lane here; let the owner listen and bring it in.").clicked() {
                        shared.act(Request::OfferLane { lane: LaneId::new(&st.bring_lane) });
                    }
                });
            }
        });
    });

    card(ui, "Moments", |ui| {
        // lenses
        let mut names: Vec<flit_core::Id> = Vec::new();
        for r in &rows {
            if !names.contains(&r.name) {
                names.push(r.name);
            }
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Show").color(C_MUTED));
            egui::ComboBox::from_id_salt("lens-name").selected_text(st.lens_name.map(|n| who(Some(song), n)).unwrap_or_else(|| "everyone".into())).show_ui(ui, |ui| {
                if ui.selectable_label(st.lens_name.is_none(), "everyone").clicked() {
                    st.lens_name = None;
                }
                for n in &names {
                    if ui.selectable_label(st.lens_name == Some(*n), who(Some(song), *n)).clicked() {
                        st.lens_name = Some(*n);
                    }
                }
            });
            let thing_text = match &st.lens_thing {
                None => "everything".to_string(),
                Some(None) => "the song".to_string(),
                Some(Some(l)) => l.0.clone(),
            };
            egui::ComboBox::from_id_salt("lens-thing").selected_text(thing_text).show_ui(ui, |ui| {
                if ui.selectable_label(st.lens_thing.is_none(), "everything").clicked() {
                    st.lens_thing = None;
                }
                if ui.selectable_label(st.lens_thing == Some(None), "the song").clicked() {
                    st.lens_thing = Some(None);
                }
                for l in &song.lanes {
                    if ui.selectable_label(st.lens_thing.as_ref() == Some(&Some(l.lane.clone())), &l.lane.0).clicked() {
                        st.lens_thing = Some(Some(l.lane.clone()));
                    }
                }
            });
            ui.checkbox(&mut st.lens_knobs, "knobs & plugins");
            ui.checkbox(&mut st.lens_takes, "takes");
            ui.checkbox(&mut st.lens_saves, "saves");
            ui.checkbox(&mut st.lens_chat, "chat");
            ui.checkbox(&mut st.lens_org, "crew & lanes");
        });
        let shown: Vec<&flit_core::HistoryRow> = rows.iter().filter(|r| lens_keeps(st, r)).collect();

        // scrubber (Exact view on release)
        if !shown.is_empty() {
            let last = shown.len() - 1;
            if st.scrub > last {
                st.scrub = last;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Scrub").color(C_MUTED));
                let resp = ui.add(egui::Slider::new(&mut st.scrub, 0..=last).show_value(false));
                let row = shown[st.scrub];
                ui.label(egui::RichText::new(format!("{} · {} · {}", moment_text(&row.wall), who(Some(song), row.name), row.summary)).size(13.0)).on_hover_text(moment_exact(&row.wall));
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    if st.scrubbed != Some(row.id) {
                        st.scrubbed = Some(row.id);
                        shared.act(Request::Preview { at: Some(row.id) });
                    }
                }
                if previewing.is_some() && primary(ui, "Back to now").clicked() {
                    st.scrubbed = None;
                    shared.act(Request::Preview { at: None });
                }
            });
            if let Some(p) = previewing {
                ui.colored_label(C_WARN, format!("Showing the song as it was {}. Your live work is untouched until you choose Go back or New branch.", shown.iter().find(|r| r.id == p).map(|r| moment_text(&r.wall)).unwrap_or_else(|| p.short())));
            }
        } else {
            hint(ui, "Nothing here yet. Turn a knob, save a version, or say something in Chat.");
        }
        ui.add_space(8.0);

        // rows, newest first
        egui::Grid::new("history").striped(true).num_columns(6).spacing(egui::vec2(14.0, 6.0)).show(ui, |ui| {
            for r in shown.iter().rev() {
                let dim = r.undone;
                let here = previewing == Some(r.id);
                let txt = |s: String| {
                    let t = egui::RichText::new(s);
                    let t = if dim { t.weak().strikethrough() } else { t };
                    if here { t.strong().color(C_ACCENT_LIGHT) } else { t }
                };
                ui.label(txt(kind_word(r.kind).to_string()));
                ui.label(txt(r.lane.as_ref().map(|l| l.0.clone()).unwrap_or_default()));
                if ui.add(egui::Label::new(txt(who(Some(song), r.name))).sense(egui::Sense::click())).on_hover_text(format!("{}\nShow only what this person did", r.name.short_code())).clicked() {
                    st.lens_name = Some(r.name);
                }
                ui.label(txt(r.summary.clone()));
                ui.label(txt(moment_text(&r.wall))).on_hover_text(moment_exact(&r.wall));
                ui.horizontal(|ui| {
                    if r.kind != Kind::Crew && ui.small_button("Go back to here").clicked() {
                        shared.act(Request::GoBack { to: r.id });
                    }
                    if ui.small_button("New branch").clicked() {
                        shared.act(Request::NewBranch { name: format!("branch from {}", moment_exact(&r.wall)), from: r.id });
                    }
                    if !r.undone && !matches!(r.kind, Kind::Crew | Kind::Branch) && ui.small_button("Undo").clicked() {
                        shared.act(Request::Undo { target: r.id });
                    }
                    if ui.small_button("Pin").on_hover_text("Pin a chat message to this row").clicked() {
                        st.pin = Some(r.id);
                        st.view = View::Chat;
                    }
                });
                ui.end_row();
            }
        });
    });
    notices(ui, shared);
}

fn chat_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut UiState) {
    let Some(_song) = &s.song else {
        card(ui, "Chat", |ui| hint(ui, "Open a song to chat with its crew."));
        return;
    };
    card(ui, "Chat", |ui| {
        hint(ui, "Everyone in the crew. Every message is kept beside the edits on the timeline.");
        for c in shared.chat.lock().clone() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("[{}]", c.branch_name)).size(12.0).color(C_MUTED));
                ui.label(egui::RichText::new(c.from_name.clone().unwrap_or_else(|| c.from.short_code())).strong().color(C_ACCENT_LIGHT)).on_hover_text(c.from.short_code());
                ui.label(&c.text);
                ui.label(egui::RichText::new(moment_text(&flit_core::Time::at(c.wall_ms, c.pos))).size(12.0).color(C_MUTED));
                if c.pos.ticks > 0 && ui.small_button(format!("↪ bar {}", c.pos.display(Default::default()))).on_hover_text("Move the DAW to this moment").clicked() {
                    shared.act(Request::Jump { pos: c.pos });
                    shared.notice(format!("Jump to {} on {}", c.pos.display(Default::default()), c.branch_name));
                }
            });
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if let Some(p) = st.pin {
                ui.label(egui::RichText::new(format!("pinned to {}", p.short())).size(12.0).color(C_MUTED));
                if ui.small_button("✕").clicked() {
                    st.pin = None;
                }
            }
            let r = ui.add(egui::TextEdit::singleline(&mut st.chat_input).hint_text("Say something…").desired_width(ui.available_width() - 140.0));
            paste_button(ui, &mut st.chat_input);
            let send = primary(ui, "Send").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if send && !st.chat_input.trim().is_empty() {
                shared.act(Request::Chat { text: st.chat_input.trim().to_string(), pinned: st.pin.take() });
                st.chat_input.clear();
            }
        });
    });
    notices(ui, shared);
}

fn settings_view(ui: &mut egui::Ui, setter: &ParamSetter, params: &FlitParams, s: &Status, shared: &Shared) {
    card(ui, "Listening", |ui| {
        let mut hear = params.hear_reference.value();
        ui.horizontal(|ui| {
            ui.label("Hear:");
            if ui.selectable_label(hear, "Reference mix").clicked() {
                hear = true;
            }
            if ui.selectable_label(!hear, "My local mix").clicked() {
                hear = false;
            }
        });
        if hear != params.hear_reference.value() {
            setter.begin_set_parameter(&params.hear_reference);
            setter.set_parameter(&params.hear_reference, hear);
            setter.end_set_parameter(&params.hear_reference);
            shared.act(Request::Settings(SettingsPatch { hear_reference: Some(hear), ..Default::default() }));
        }
        hint(ui, "The mix owner's master is the song's reference sound. Only you hear your own master chain.");
        let mut kbps = s.settings.monitoring_bitrate / 1000;
        if ui.add(egui::Slider::new(&mut kbps, 64..=256).text("monitoring kbps")).drag_stopped() {
            shared.act(Request::Settings(SettingsPatch { monitoring_bitrate: Some(kbps * 1000), ..Default::default() }));
        }
        let mut submix = s.settings.submix;
        if ui.checkbox(&mut submix, "Send my lanes as one submix").changed() {
            shared.act(Request::Settings(SettingsPatch { submix: Some(submix), ..Default::default() }));
        }
    });
    card(ui, "Downloads", |ui| {
        let mut dl = s.settings.download_full_quality;
        ui.horizontal(|ui| {
            ui.label("Full-quality audio:");
            for (v, n) in [(flit_core::DownloadPolicy::Always, "always"), (flit_core::DownloadPolicy::OnDemand, "on demand"), (flit_core::DownloadPolicy::WifiOnly, "on Wi-Fi only")] {
                if ui.selectable_label(dl == v, n).clicked() {
                    dl = v;
                }
            }
        });
        if dl != s.settings.download_full_quality {
            shared.act(Request::Settings(SettingsPatch { download_full_quality: Some(dl), ..Default::default() }));
        }
        hint(ui, "Takes arrive as a small listening copy first; the lossless copy follows in the background.");
    });
    card(ui, "You", |ui| {
        let mut name = s.settings.display_name.clone();
        ui.horizontal(|ui| {
            ui.label("Your name");
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(200.0).hint_text("how the crew should call you")).lost_focus() && name != s.settings.display_name {
                shared.act(Request::Settings(SettingsPatch { display_name: Some(name.clone()), ..Default::default() }));
            }
            paste_button(ui, &mut name);
        });
        hint(ui, "Written into every song you open as a fact on your own chain, so the crew gets it from your ID. Anyone can still give you a nickname of their own (Crew > Friends).");
        let mut relay = s.settings.relay_only;
        if ui.checkbox(&mut relay, "Relay only (hide my IP from the crew; takes effect on restart)").changed() {
            shared.act(Request::Settings(SettingsPatch { relay_only: Some(relay), ..Default::default() }));
        }
        hint(ui, "Headphones recommended for Talk and voice: nothing leaks into the song or a render.");
    });
    card(ui, "Under the hood", |ui| {
        ui.label(format!("Helper {} · DAW: {} · bridge level {} · {} plugin instances", s.agent_version, host_name(&s.host), s.level, s.connected_clients));
        if !s.bridge.is_empty() {
            hint(ui, &s.bridge);
        }
        if let Some(p) = &s.project {
            hint(ui, &format!("Project: {}", p.display()));
        }
        for w in &s.warnings {
            ui.colored_label(C_WARN, w);
        }
    });
    notices(ui, shared);
}

fn notices(ui: &mut egui::Ui, shared: &Shared) {
    let n = shared.notices.lock();
    if n.is_empty() {
        return;
    }
    card(ui, "Recent", |ui| {
        for line in n.iter().rev().take(6) {
            hint(ui, line);
        }
    });
}

#[allow(dead_code)]
fn _pos(_: SongPos) {}
