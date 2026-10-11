//! The panel. Plain words only: Moment, History, Track, Being, Merge,
//! Timeline, Go back, New history from here, Save version, Crew, Song code.
//! Layout (FLIT_SPEC §8, §12): top bar 36 px (Name, acting as, song,
//! link), left rail, main, the Timeline bar docked at the bottom (history
//! switcher, lens, Acts | Facts | Time, the tape, Live, Merge), status
//! strip 22 px. Everything reads theme.rs; the tape is the one signature.

use crate::client::Shared;
use crate::host::{Host, Mode, Surface};
use crate::theme::{self as t, color as being_color};
use flit_core::{ActId, BeingId, BeingKind, BeingRef, HistoryId, Kind, Lens, PresencePayload, Row, SongPos, Thing, TrackId, View as DawView};
use flit_ipc::*;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

mod help;
pub mod library;
mod tutorial;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Screen {
    #[default]
    Home,
    Library,
    Crew,
    Beings,
    Chat,
    Settings,
    Help,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Axis {
    /// One mark per moment, in the lens's own order (act order on member lenses, seal order on things).
    #[default]
    Facts,
    Time,
}

/// Merge dialog state.
#[derive(Default)]
struct MergeDialog {
    open: bool,
    from: Option<HistoryId>,
    to: Option<HistoryId>,
    whole: bool,
    tracks: Vec<TrackId>,
    /// (thing, property) -> keep FROM
    choices: BTreeMap<(Thing, String), bool>,
    requested: Option<(HistoryId, Vec<TrackId>)>,
}

#[derive(Default)]
pub struct PanelState {
    screen: Screen,
    themed: bool,
    logo: Option<egui::TextureHandle>,
    loaded: bool,
    gallery: bool,
    tutorial: tutorial::State,
    /// The Library: songs, albums, orgs, invites (PACK 2 §3).
    library: library::State,
    /// Help shows the walk by default; this flips it to the concept list.
    help_reference: bool,
    // forms
    new_name: String,
    new_dir: String,
    join_dir: String,
    share_project: String,
    join_code: String,
    friend_code: String,
    friend_name: String,
    chat_input: String,
    pin: Option<ActId>,
    version_name: String,
    history_name: String,
    being_id: String,
    being_display: String,
    being_kind: usize,
    grant_being: String,
    grant_tracks: String,
    flag_note: String,
    name_label: String,
    backup_pass: String,
    backup_path: String,
    import_path: String,
    recovery: String,
    keeper_machine: String,
    required_version: String,
    manifest_url: String,
    qr: Option<(String, Vec<Vec<bool>>)>,
    // timeline bar
    tape_h: f32,
    axis: Axis,
    axis_received: bool,
    lens: Lens,
    show_presence: bool,
    /// Selected moment (Exact view shown in the DAW); None = live.
    selected: Option<ActId>,
    /// Scroll offset (in marks) from the right edge; 0 = live edge.
    scroll: f32,
    /// The moment the view holds on to while scrolled back or showing the
    /// past, so arriving moments do not move it.
    anchor: Option<ActId>,
    zoom: f32,
    detached: bool,
    history_menu: bool,
    lens_menu: bool,
    merge: MergeDialog,
    /// Pinned chat target from the tape's context menu.
    chat_pin_from_tape: Option<ActId>,
    /// The app's Settings > Quit FLit was pressed (the app closes for real).
    quit: bool,
}

impl PanelState {
    pub fn new() -> Self {
        PanelState { tape_h: t::TIMELINE_DEFAULT_H, zoom: 1.0, lens: Lens::Song, ..Default::default() }
    }
    pub fn quit_requested(&self) -> bool {
        self.quit
    }
}

// ----- helpers -----

fn logo(ctx: &egui::Context, st: &mut PanelState) -> Option<egui::TextureHandle> {
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

/// A "Paste" button next to a field: DAWs do not always hand the keyboard
/// to a plugin window; the clipboard always works.
fn paste_button(ui: &mut egui::Ui, target: &mut String) {
    if t::quiet(ui, "Paste").on_hover_text("Paste from the clipboard").clicked() {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if let Ok(text) = cb.get_text() {
                *target = text.trim().to_string();
            }
        }
    }
}

fn field_paste(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.label(egui::RichText::new(label).font(t::text(t::S11)).color(t::MUTED));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(value).hint_text(egui::RichText::new(hint).color(t::FAINT)).desired_width(ui.available_width() - 64.0).font(t::text(t::S12)));
        paste_button(ui, value);
    });
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// "3m ago" (witness time, display only).
fn ago(ms: u64) -> String {
    let age = now_ms().saturating_sub(ms) / 1000;
    if age < 5 {
        "just now".to_string()
    } else if age < 60 {
        format!("{age}s ago")
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86_400 {
        format!("{}h {}m ago", age / 3600, (age % 3600) / 60)
    } else {
        format!("{}d ago", age / 86_400)
    }
}

/// "2026-10-10 14:03:22 UTC" (witness time).
fn exact(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let days = secs / 86_400;
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
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

fn bar_of(pos: Option<SongPos>) -> String {
    pos.map(|p| format!("bar {}", p.display(Default::default()))).unwrap_or_default()
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
    p.rect_filled(rect, 3.0, t::TEXT);
    let n = m.len() as f32;
    let cell = size / (n + 2.0);
    for (y, row) in m.iter().enumerate() {
        for (x, dark) in row.iter().enumerate() {
            if *dark {
                let min = rect.min + egui::vec2((x as f32 + 1.0) * cell, (y as f32 + 1.0) * cell);
                p.rect_filled(egui::Rect::from_min_size(min, egui::vec2(cell, cell)), 0.0, t::BG);
            }
        }
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
        Kind::Being => "member",
        Kind::Grant => "grant",
        Kind::Flag => "flag",
        Kind::Keeper => "keeper",
        Kind::History => "history",
        Kind::Merge => "merge",
        Kind::Chat => "chat",
        Kind::Presence => "presence",
        Kind::HostState => "state",
        Kind::GoBack => "go back",
        Kind::Undo => "undo",
        Kind::Settings => "setting",
        Kind::Album => "album",
    }
}

fn being_kind_word(k: BeingKind) -> &'static str {
    match k {
        BeingKind::Human => "human",
        BeingKind::Ai => "ai",
        BeingKind::Scripted => "scripted",
    }
}

fn host_name(tag: &str) -> &str {
    match tag {
        "fl" => "FL Studio",
        "fake" => "the fake host",
        "plugin" => "plugin only",
        other => other,
    }
}

/// Who is who in the song, for colors and labels.
struct People<'a> {
    beings: &'a [BeingRow],
}

impl People<'_> {
    fn color(&self, b: &BeingRef) -> u32 {
        self.beings.iter().find(|x| x.being == *b).map(|x| x.color).unwrap_or(0x9AA5B1)
    }
    fn label(&self, b: &BeingRef) -> String {
        self.beings.iter().find(|x| x.being == *b).map(|x| x.display_name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| b.being.0.clone())
    }
    fn kind(&self, b: &BeingRef) -> BeingKind {
        self.beings.iter().find(|x| x.being == *b).map(|x| x.kind).unwrap_or_default()
    }
    fn initial(&self, b: &BeingRef) -> char {
        self.label(b).chars().next().unwrap_or('?')
    }
}

fn track_label(tracks: &[TrackInfo], id: &TrackId) -> String {
    tracks.iter().find(|x| x.track == *id).map(|x| x.name.clone()).unwrap_or_else(|| id.0.clone())
}

fn thing_label(tracks: &[TrackInfo], thing: &Thing) -> String {
    match thing {
        Thing::Song => "Song".into(),
        Thing::SongSettings => "Song settings".into(),
        Thing::Track(tr) => track_label(tracks, tr),
        Thing::Device { track, slot } => format!("{} / slot {}", track_label(tracks, track), slot.0 + 1),
        Thing::Presence(_) => "presence".into(),
        Thing::Album(a) => format!("album {}", a.short()),
    }
}

// ----- the frame -----

/// One frame of the whole panel. Draws nothing on a context's first
/// frame (the theme's fonts exist from the next one).
pub fn draw(ctx: &egui::Context, host: &dyn Host, st: &mut PanelState, shared: &Shared) {
    // Fonts set on a context only exist from the next frame on, and using a
    // font family before it exists panics (which would take the DAW down).
    // So the first frame only installs the theme and draws nothing.
    // The flag lives on the context itself: a new context (the window was
    // closed and opened again) needs the theme again.
    let themed_id = egui::Id::new("flit-themed");
    if !ctx.data(|d| d.get_temp::<bool>(themed_id).unwrap_or(false)) {
        ctx.data_mut(|d| d.insert_temp(themed_id, true));
        st.themed = true;
        t::apply(ctx);
        ctx.request_repaint();
        return;
    }
    let connected = shared.connected.load(Ordering::Relaxed);
    let status = shared.status.lock().clone();
    if connected && !st.loaded {
        st.loaded = true;
        shared.act(Request::Songs);
        shared.act(Request::Friends);
        shared.act(Request::Histories);
        shared.act(Request::Tracks);
        shared.act(Request::ChatList);
        shared.act(Request::Beings);
        shared.act(Request::Names);
        if let Some(s) = status.as_ref().and_then(|s| s.song.as_ref()) {
            shared.want_timeline(s.history, st.lens.clone(), st.show_presence);
        }
    }
    if !connected {
        st.loaded = false;
    }
    let logo_tex = logo(ctx, st);
    // the panel's own presence at level 0: which screen the user is on
    {
        let view = match st.screen {
            Screen::Chat => DawView::Other,
            _ => DawView::Other,
        };
        let p = PresencePayload { view, track: None, device: None, region: Some(match st.screen {
            Screen::Home => "home",
            Screen::Library => "library",
            Screen::Crew => "crew",
            Screen::Beings => "beings",
            Screen::Chat => "chat",
            Screen::Settings => "settings",
            Screen::Help => "help",
        }.into()) };
        if status.as_ref().map(|s| s.level == 0).unwrap_or(false) {
            *shared.presence.lock() = Some(p);
        }
    }

    top_bar(ctx, host, st, shared, status.as_ref(), connected, logo_tex.as_ref());

    let Some(s) = status.as_ref() else {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t::BG).inner_margin(egui::Margin::same(24))).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                if let Some(tex) = &logo_tex {
                    ui.image((tex.id(), egui::vec2(96.0, 96.0)));
                }
                ui.add_space(12.0);
                ui.spinner();
                t::muted(ui, "Starting the FLit helper");
                t::small(ui, "Audio keeps playing while we connect.");
                if let Some(e) = shared.connect_error.lock().as_ref() {
                    t::small(ui, e);
                }
            });
        });
        return;
    };

    if host.mode() != Mode::Master {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t::BG).inner_margin(egui::Margin::same(20))).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| track_mode_view(ui, host, s, shared));
        });
        return;
    }

    // First run: the tutorial fills the panel until a Name and a first Being exist.
    if st.tutorial.onboarding(s) {
        host.main_area(ctx, &mut |ui| {
            egui::Frame::new().fill(t::BG).inner_margin(egui::Margin::same(t::SECTION as i8)).show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if tutorial::show(ui, &mut st.tutorial, s, shared, logo_tex.as_ref()) {
                        st.screen = Screen::Home;
                        shared.act(Request::Beings);
                    }
                });
            });
        });
        return;
    }

    status_strip(ctx, s, shared);
    timeline_bar(ctx, st, s, shared);
    merge_dialog(ctx, st, s, shared);

    // navigation rail
    egui::SidePanel::left("nav").frame(egui::Frame::new().fill(t::SURFACE).inner_margin(egui::Margin::symmetric(8, 12))).resizable(false).exact_width(t::RAIL_W).show(ctx, |ui| {
        let has_song = s.song.is_some();
        for (screen, name, enabled) in [(Screen::Home, "Home", true), (Screen::Library, "Library", true), (Screen::Crew, "Crew", true), (Screen::Beings, "Studio", true), (Screen::Chat, "Chat", has_song), (Screen::Settings, "Settings", true), (Screen::Help, "Help", true)] {
            let selected = st.screen == screen;
            let text = egui::RichText::new(name).font(if selected { t::text_medium(t::S12) } else { t::text(t::S12) }).color(if selected { t::TEXT } else if enabled { t::MUTED } else { t::FAINT });
            let b = egui::Button::new(text).fill(if selected { t::RAISED } else { egui::Color32::TRANSPARENT }).stroke(egui::Stroke::NONE).corner_radius(egui::CornerRadius::same(t::RADIUS_CONTROL)).min_size(egui::vec2(ui.available_width(), t::BUTTON_H));
            if ui.add_enabled(enabled, b).clicked() {
                st.screen = screen;
                match screen {
                    Screen::Home => {
                        shared.act(Request::Songs);
                        shared.act(Request::Tracks);
                    }
                    Screen::Library => st.library.refresh(shared),
                    Screen::Crew => {
                        shared.act(Request::Friends);
                        shared.act(Request::Beings);
                    }
                    Screen::Beings => {
                        shared.act(Request::Beings);
                        shared.act(Request::Names);
                    }
                    Screen::Chat => shared.act(Request::ChatList),
                    Screen::Settings => {}
                    Screen::Help => {
                        st.tutorial.open();
                        st.help_reference = false;
                    }
                }
            }
        }
        ui.add_space(t::SECTION);
        // declined notices with their actions
        let declined: Vec<_> = shared.declined.lock().iter().cloned().collect();
        for (i, (_, history, history_name, property)) in declined.iter().enumerate().take(3) {
            t::card(ui, |ui| {
                ui.label(egui::RichText::new(format!("Your change to {property} collided with someone else's. Saved as \"{history_name}\".")).font(t::text(t::S11)).color(t::WARN));
                ui.horizontal(|ui| {
                    if t::button(ui, "Merge").clicked() {
                        st.merge.open = true;
                        st.merge.from = Some(*history);
                        st.merge.to = s.song.as_ref().map(|x| x.history);
                        st.merge.whole = true;
                        st.merge.requested = None;
                        shared.declined.lock().remove(i);
                    }
                    if t::quiet(ui, "Dismiss").clicked() {
                        shared.declined.lock().remove(i);
                    }
                });
            });
        }
        let notices: Vec<String> = shared.notices.lock().iter().rev().take(3).cloned().collect();
        if !notices.is_empty() {
            t::section_label(ui, "Notices");
            for n in notices.iter() {
                let short: String = if n.chars().count() > 90 { n.chars().take(88).collect::<String>() + "..." } else { n.clone() };
                ui.add(egui::Label::new(egui::RichText::new(short).font(t::text(t::S11)).color(t::MUTED)).wrap()).on_hover_text(n);
            }
        }
    });

    // Resizable from inside the plugin window (drag the bottom-right corner):
    // nih-plug asks the host to resize.
    host.main_area(ctx, &mut |ui| {
        egui::Frame::new().fill(t::BG).inner_margin(egui::Margin::same(t::SECTION as i8)).show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if st.gallery {
                    if t::quiet(ui, "Close the style gallery").clicked() {
                        st.gallery = false;
                    }
                    t::gallery(ui);
                    return;
                }
                match st.screen {
                    Screen::Home => home_view(ui, s, shared, st, logo_tex.as_ref()),
                    Screen::Library => library::show(ui, &mut st.library, s, shared),
                    Screen::Crew => crew_view(ui, s, shared, st),
                    Screen::Beings => beings_view(ui, s, shared, st),
                    Screen::Chat => chat_view(ui, s, shared, st),
                    Screen::Settings => settings_view(ui, host, s, shared, st),
                    Screen::Help => {
                        let idx = t::segmented(ui, if st.help_reference { 1 } else { 0 }, &["Walk through", "Every concept"]);
                        st.help_reference = idx == 1;
                        if st.help_reference {
                            help::show(ui, s);
                        } else if tutorial::show(ui, &mut st.tutorial, s, shared, logo_tex.as_ref()) {
                            st.screen = Screen::Home;
                        }
                    }
                }
            });
        });
    });
}

#[allow(clippy::too_many_arguments)]
fn top_bar(ctx: &egui::Context, host: &dyn Host, st: &mut PanelState, shared: &Shared, status: Option<&Status>, connected: bool, logo_tex: Option<&egui::TextureHandle>) {
    egui::TopBottomPanel::top("top").exact_height(t::TOP_BAR_H).frame(egui::Frame::new().fill(t::SURFACE).inner_margin(egui::Margin::symmetric(12, 0)).stroke(egui::Stroke::new(1.0_f32, t::BORDER))).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            if let Some(tex) = logo_tex {
                ui.image((tex.id(), egui::vec2(22.0, 22.0)));
            }
            ui.label(egui::RichText::new("FLit").font(t::text_medium(t::S14)));
            ui.add_space(t::GAP);
            if let Some(s) = status {
                // Studio (click: Studio screen = Name management) and Acting as
                let name_btn = egui::Button::new(egui::RichText::new(&s.name_label).font(t::text(t::S12))).fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE);
                if ui.add(name_btn).on_hover_text(format!("Studio {}\nClick for Studios on this machine", s.short_code)).clicked() {
                    st.screen = Screen::Beings;
                    shared.act(Request::Names);
                }
                t::muted(ui, "acting as");
                let beings = shared.beings.lock().clone();
                let mine: Vec<&BeingRow> = beings.iter().filter(|b| b.mine && !b.revoked).collect();
                let label = format!("{} ({})", s.being_label, being_kind_word(s.being_kind));
                egui::ComboBox::from_id_salt("acting").selected_text(egui::RichText::new(label).font(t::text(t::S12))).width(170.0).show_ui(ui, |ui| {
                    for b in &mine {
                        let on = b.being.being == s.being;
                        if ui.selectable_label(on, format!("{} ({})", b.display_name, being_kind_word(b.kind))).clicked() && !on {
                            shared.act(Request::ActAs { being: b.being.being.clone() });
                        }
                    }
                    if mine.is_empty() {
                        let _ = ui.selectable_label(true, format!("{} ({})", s.being_label, being_kind_word(s.being_kind)));
                    }
                    ui.separator();
                    if ui.selectable_label(false, "Add a member").clicked() {
                        st.screen = Screen::Beings;
                    }
                });
                ui.add_space(t::SECTION);
                if let Some(song) = &s.song {
                    ui.label(egui::RichText::new(&song.name).font(t::text_medium(t::S12)));
                    t::muted(ui, &format!("history: {}", song.history_name));
                    if let Some(p) = &s.paused {
                        t::dot(ui, t::BAD);
                        ui.label(egui::RichText::new("edits paused").font(t::text(t::S11)).color(t::BAD)).on_hover_text(p);
                    }
                    if let Some(v) = &song.version_mismatch {
                        t::dot(ui, t::WARN);
                        ui.label(egui::RichText::new(format!("this song requires FLit {v}")).font(t::text(t::S11)).color(t::WARN)).on_hover_text("Your moments wait until you update.");
                    }
                }
                if s.update_available.is_some() {
                    t::dot(ui, t::ACCENT);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if host.surface() == Surface::Plugin {
                    let mode = host.mode();
                    egui::ComboBox::from_id_salt("mode").selected_text(egui::RichText::new(mode.name()).font(t::text(t::S12))).width(130.0).show_ui(ui, |ui| {
                        for m in Mode::ALL {
                            if ui.selectable_label(mode == m, m.name()).clicked() {
                                host.set_mode(m);
                            }
                        }
                    });
                    if mode != Mode::Master {
                        let mut track = host.track();
                        if ui.add(egui::TextEdit::singleline(&mut track).desired_width(110.0).hint_text("name").font(t::text(t::S12))).changed() {
                            host.set_track(track);
                        }
                        t::muted(ui, if matches!(mode, Mode::TalkSend | Mode::TalkReceive) { "name" } else { "track" });
                    }
                }
                ui.add_space(t::GAP);
                if connected {
                    if let Some(s) = status {
                        match s.bridge_up {
                            Some(true) => {
                                t::muted(ui, &format!("{} linked", host_name(&s.host)));
                                t::dot(ui, t::OK);
                            }
                            Some(false) => {
                                ui.label(egui::RichText::new(format!("{} not answering", host_name(&s.host))).font(t::text(t::S12)).color(t::WARN)).on_hover_text(&s.bridge);
                                t::dot(ui, t::WARN);
                            }
                            None => {
                                t::muted(ui, "plugin only");
                                t::dot(ui, t::FAINT);
                            }
                        }
                    }
                } else {
                    t::muted(ui, "starting the helper");
                    t::dot(ui, t::WARN);
                }
            });
        });
    });
}

/// The status strip under the Timeline bar: connection, beings online
/// grouped by history, pending work.
fn status_strip(ctx: &egui::Context, s: &Status, shared: &Shared) {
    egui::TopBottomPanel::bottom("strip").exact_height(t::STATUS_STRIP_H).frame(egui::Frame::new().fill(t::SURFACE).inner_margin(egui::Margin::symmetric(12, 0)).stroke(egui::Stroke::new(1.0_f32, t::BORDER))).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            let (c, word) = match s.connection {
                Connection::Direct => (t::OK, "direct"),
                Connection::Relayed => (t::WARN, "relayed"),
                Connection::Offline => (t::FAINT, "offline"),
            };
            t::dot(ui, c);
            t::small(ui, word);
            if let Some(song) = &s.song {
                ui.add_space(t::GAP);
                let beings = shared.beings.lock().clone();
                let people = People { beings: &beings };
                for h in &song.histories {
                    let here: Vec<&BeingRow> = beings.iter().filter(|b| b.online && b.history == Some(h.history)).collect();
                    if here.is_empty() {
                        continue;
                    }
                    let resp = ui.horizontal(|ui| {
                        t::small(ui, &format!("{}:", h.name));
                        for b in &here {
                            let r = t::avatar(ui, b.color, people.initial(&b.being), h.current, 14.0);
                            r.on_hover_ui(|ui| being_card(ui, b, &people, s));
                        }
                    }).response;
                    resp.on_hover_ui(|ui| {
                        for b in &here {
                            ui.horizontal(|ui| {
                                t::avatar(ui, b.color, people.initial(&b.being), true, 14.0);
                                ui.label(format!("{} ({}) {}", b.display_name, being_kind_word(b.kind), b.being.name.short_code()));
                            });
                        }
                        if !h.current && t::button(ui, "Switch to this history").clicked() {
                            shared.act(Request::SwitchHistory { history: h.history });
                        }
                    });
                    ui.add_space(t::GAP);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if song.unsealed > 0 {
                        t::small(ui, &format!("{} moments waiting to be sealed", song.unsealed));
                    }
                    if song.unsent > 0 {
                        t::small(ui, &format!("{} not yet sent", song.unsent));
                    }
                    if song.missing_opus + song.missing_flac > 0 {
                        t::small(ui, &format!("downloading {} chunks", song.missing_opus + song.missing_flac));
                    }
                    if !song.keeper_online {
                        ui.label(egui::RichText::new("keeper offline").font(t::text(t::S11)).color(t::WARN)).on_hover_text("Everyone keeps working; moments wait to be sealed. An owner can move the keeper role.");
                    } else if song.i_am_keeper {
                        t::small(ui, "keeper: this machine");
                    }
                });
            }
        });
    });
}

fn being_card(ui: &mut egui::Ui, b: &BeingRow, people: &People, s: &Status) {
    ui.horizontal(|ui| {
        t::avatar(ui, b.color, people.initial(&b.being), true, 18.0);
        t::strong(ui, &b.display_name);
        t::muted(ui, being_kind_word(b.kind));
    });
    t::small(ui, &format!("Studio {}", b.being.name.short_code()));
    if let Some(h) = b.history.and_then(|h| s.song.as_ref().and_then(|sg| sg.histories.iter().find(|x| x.history == h).map(|x| x.name.clone()))) {
        t::small(ui, &format!("in {h}"));
    }
    if let Some(p) = &b.presence {
        let tracks = s.song.as_ref().map(|sg| sg.tracks.clone()).unwrap_or_default();
        t::small(ui, &format!("{:?}{}", p.view, p.track.as_ref().map(|tr| format!(" · {}", track_label(&tracks, tr))).unwrap_or_default()).to_lowercase());
    }
    if b.flags > 0 {
        ui.label(egui::RichText::new(format!("{} flag(s) on this member", b.flags)).font(t::text(t::S11)).color(t::WARN));
    }
}

// ----- the Timeline bar -----

fn lens_label(lens: &Lens, s: &Status, people: &People) -> String {
    let tracks = s.song.as_ref().map(|sg| sg.tracks.clone()).unwrap_or_default();
    match lens {
        Lens::Song => "Song".into(),
        Lens::SongSettings => "Song settings".into(),
        Lens::Track(tr) => track_label(&tracks, tr),
        Lens::Device { track, slot } => format!("{} / slot {}", track_label(&tracks, track), slot.0 + 1),
        Lens::Me(_) => "Me".into(),
        Lens::Name(_) => "My Studio".into(),
        Lens::Being(b) => people.label(b),
        Lens::Team => "Team".into(),
        Lens::Combine(a, b) => format!("{} x {}", lens_label(a, s, people), lens_label(b, s, people)),
        Lens::Album(a) => format!("Album {}", a.short()),
    }
}

fn timeline_bar(ctx: &egui::Context, st: &mut PanelState, s: &Status, shared: &Shared) {
    let beings = shared.beings.lock().clone();
    let people = People { beings: &beings };
    let Some(song) = &s.song else {
        egui::TopBottomPanel::bottom("timeline").exact_height(40.0).frame(egui::Frame::new().fill(t::SURFACE).inner_margin(egui::Margin::symmetric(12, 0)).stroke(egui::Stroke::new(1.0_f32, t::BORDER))).show(ctx, |ui| {
            ui.horizontal_centered(|ui| t::muted(ui, "Timeline: open or make a song"));
        });
        return;
    };
    shared.want_timeline(song.history, st.lens.clone(), st.show_presence);
    let rows = shared.timeline.lock().clone();
    let me = BeingRef { name: s.name, being: s.being.clone() };
    egui::TopBottomPanel::bottom("timeline").resizable(true).default_height(st.tape_h).min_height(64.0).max_height(260.0).frame(egui::Frame::new().fill(t::SURFACE).inner_margin(egui::Margin::symmetric(12, 6)).stroke(egui::Stroke::new(1.0_f32, t::BORDER))).show(ctx, |ui| {
        st.tape_h = ui.available_height();
        // controls row
        ui.horizontal(|ui| {
            // history switcher
            let cur = song.histories.iter().find(|h| h.current);
            let label = cur.map(|h| h.name.clone()).unwrap_or_else(|| "main".into());
            egui::ComboBox::from_id_salt("history").selected_text(egui::RichText::new(format!("History: {label}")).font(t::text(t::S12))).width(200.0).show_ui(ui, |ui| {
                for h in &song.histories {
                    ui.horizontal(|ui| {
                        let resp = ui.selectable_label(h.current, &h.name);
                        for b in beings.iter().filter(|b| b.online && b.history == Some(h.history)).take(6) {
                            t::avatar(ui, b.color, people.initial(&b.being), true, 12.0);
                        }
                        if resp.clicked() && !h.current {
                            shared.act(Request::SwitchHistory { history: h.history });
                            st.selected = None;
                            st.detached = false;
                        }
                        resp.on_hover_ui(|ui| {
                            t::small(ui, &format!("{} moments, {} unsealed", h.moments, h.unsealed));
                            if let Some(c) = &h.created_by {
                                t::small(ui, &format!("started by {}", people.label(c)));
                            }
                        });
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.history_name).hint_text("name").desired_width(110.0));
                    let from = st.selected.or_else(|| rows.last().map(|r| r.act));
                    if t::button(ui, "New history from this moment").clicked() {
                        if let Some(from) = from {
                            let name = if st.history_name.trim().is_empty() { "new history".to_string() } else { st.history_name.trim().to_string() };
                            shared.act(Request::NewHistory { name, from });
                            st.history_name.clear();
                            st.selected = None;
                            st.detached = false;
                        }
                    }
                });
            });
            // lens
            egui::ComboBox::from_id_salt("lens").selected_text(egui::RichText::new(format!("Lens: {}", lens_label(&st.lens, s, &people))).font(t::text(t::S12))).width(190.0).show_ui(ui, |ui| {
                if ui.selectable_label(st.lens == Lens::Song, "Song").clicked() {
                    st.lens = Lens::Song;
                }
                if ui.selectable_label(st.lens == Lens::SongSettings, "    Song settings").clicked() {
                    st.lens = Lens::SongSettings;
                }
                for tr in &song.tracks {
                    let l = Lens::Track(tr.track.clone());
                    if ui.selectable_label(st.lens == l, format!("    {} ({})", tr.name, tr.moments)).clicked() {
                        st.lens = l;
                    }
                }
                ui.separator();
                if ui.selectable_label(matches!(st.lens, Lens::Me(_)), "Me").clicked() {
                    st.lens = Lens::Me(me.clone());
                }
                if ui.selectable_label(matches!(st.lens, Lens::Name(_)), "My Studio").clicked() {
                    st.lens = Lens::Name(s.name);
                }
                if ui.selectable_label(st.lens == Lens::Team, "Team").clicked() {
                    st.lens = Lens::Team;
                }
                for b in beings.iter().filter(|b| !b.mine) {
                    let l = Lens::Being(b.being.clone());
                    if ui.selectable_label(st.lens == l, format!("    {}", b.display_name)).clicked() {
                        st.lens = l;
                    }
                }
                ui.separator();
                ui.checkbox(&mut st.show_presence, "Show presence");
            });
            // Users see moments; act order and fact order both put one mark per moment.
            let idx = t::segmented(ui, if st.axis == Axis::Time { 1 } else { 0 }, &["Moments", "Time"]);
            st.axis = match (idx, st.axis) {
                (1, _) => Axis::Time,
                (_, Axis::Time) => Axis::Facts,
                (_, keep) => keep,
            };
            if st.axis == Axis::Time {
                let w = if st.axis_received { "received" } else { "stamped" };
                if t::quiet(ui, w).on_hover_text("Time axis by the stamper's clock, or by when this machine received each moment").clicked() {
                    st.axis_received = !st.axis_received;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if t::button(ui, "Merge").clicked() {
                    st.merge.open = true;
                    st.merge.to = Some(song.history);
                    st.merge.from = song.histories.iter().find(|h| h.current).and_then(|h| h.parent).or_else(|| song.histories.iter().find(|h| !h.current).map(|h| h.history));
                    st.merge.whole = true;
                    st.merge.requested = None;
                }
                let live = st.selected.is_none() && !st.detached;
                let b = egui::Button::new(egui::RichText::new("Live").font(t::text_medium(t::S12)).color(if live { t::BG } else { t::TEXT })).fill(if live { t::ACCENT } else { t::RAISED }).stroke(egui::Stroke::new(1.0_f32, if live { t::ACCENT } else { t::BORDER })).corner_radius(egui::CornerRadius::same(t::RADIUS_CONTROL)).min_size(egui::vec2(0.0, t::BUTTON_H));
                if ui.add(b).clicked() {
                    st.selected = None;
                    st.detached = false;
                    st.scroll = 0.0;
                    if s.previewing.is_some() {
                        shared.act(Request::ShowMoment { at: None });
                    }
                }
                if let Some(sel) = st.selected {
                    if t::button(ui, "Go back to here").clicked() {
                        shared.act(Request::GoBack { to: sel });
                        st.selected = None;
                        st.detached = false;
                    }
                    if let Some(r) = rows.iter().find(|r| r.act == sel) {
                        t::small(ui, &format!("#{} {}", r.number, r.summary));
                    }
                }
            });
        });
        ui.add_space(2.0);
        tape(ui, st, s, &rows, &people, shared, &me);
    });
}

/// The tape: moments as small marks in the acting being's color, in the
/// lens's order. Versions taller with a seal glyph, chat as dots above,
/// gates hollow, unsealed outlined, undone faint. Live at the right edge.
fn tape(ui: &mut egui::Ui, st: &mut PanelState, s: &Status, rows: &[Row], people: &People, shared: &Shared, me: &BeingRef) {
    let avail = ui.available_size();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail.x, avail.y.max(24.0)), egui::Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, t::BG);
    p.rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, t::BORDER), egui::StrokeKind::Inside);
    if rows.is_empty() {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "No moments yet", t::text(t::S12), t::FAINT);
        return;
    }
    // zoom / scroll
    let (scroll_x, ctrl) = ui.input(|i| (i.raw_scroll_delta.x + i.raw_scroll_delta.y, i.modifiers.ctrl));
    if resp.hovered() && scroll_x != 0.0 {
        if ctrl {
            st.zoom = (st.zoom * (1.0 + scroll_x * 0.002)).clamp(0.25, 8.0);
        } else {
            st.scroll = (st.scroll - scroll_x * 0.05).max(0.0);
            st.detached = st.scroll > 0.0;
            st.anchor = None;
        }
    }
    if resp.dragged() {
        let d = resp.drag_delta().x;
        if d.abs() > 0.0 {
            let per = spacing(st, rect.width(), rows.len());
            st.scroll = (st.scroll + d / per).max(0.0);
            st.detached = st.scroll > 0.0;
            st.anchor = None;
        }
    }
    let n = rows.len();
    // The live edge: with nothing selected and no scroll, the newest moment
    // sits at the right and new ones keep landing there. Scrolled back, or
    // showing a past moment, the view holds still on a moment while new
    // ones arrive, until Live or a scroll back to the edge.
    if st.detached || st.selected.is_some() {
        match st.anchor.and_then(|a| rows.iter().position(|r| r.act == a)) {
            Some(i) => st.scroll = (n - 1 - i) as f32,
            None => {
                let i = (n as f32 - 1.0 - st.scroll).round().clamp(0.0, (n - 1) as f32) as usize;
                st.anchor = rows.get(i).map(|r| r.act);
            }
        }
    } else {
        st.anchor = None;
    }
    let per = spacing(st, rect.width(), n);
    let base_y = rect.bottom() - 6.0;
    // positions
    let xs: Vec<f32> = match st.axis {
        Axis::Time => {
            let key = |r: &Row| if st.axis_received { r.received_at.unwrap_or(r.stamped_at) } else { r.stamped_at };
            let min = rows.iter().map(key).min().unwrap_or(0) as f64;
            let max = rows.iter().map(key).max().unwrap_or(0).max(min as u64 + 1) as f64;
            let w = (rect.width() - 24.0) as f64 * st.zoom as f64;
            rows.iter().map(|r| rect.right() - 12.0 - (((max - key(r) as f64) / (max - min)) * w) as f32 + st.scroll * per).collect()
        }
        _ => rows.iter().enumerate().map(|(i, _)| rect.right() - 12.0 - (n - 1 - i) as f32 * per + st.scroll * per).collect(),
    };
    let mut hovered: Option<usize> = None;
    let pointer = resp.hover_pos();
    for (i, r) in rows.iter().enumerate() {
        let x = xs[i];
        if x < rect.left() + 4.0 || x > rect.right() - 4.0 {
            continue;
        }
        let c = being_color(people.color(&r.being));
        let c = if r.undone { c.gamma_multiply(0.35) } else if r.inherited { c.gamma_multiply(0.6) } else { c };
        let h = match r.kind {
            Kind::Version => rect.height() - 16.0,
            Kind::Merge | Kind::GoBack | Kind::History => rect.height() * 0.6,
            _ => rect.height() * 0.4,
        };
        let mark = egui::Rect::from_min_max(egui::pos2(x - 1.5, base_y - h), egui::pos2(x + 1.5, base_y));
        match r.kind {
            Kind::Chat => {
                p.circle_filled(egui::pos2(x, rect.top() + 6.0), 2.0, c);
            }
            Kind::Presence => {
                p.circle_stroke(egui::pos2(x, rect.top() + 6.0), 2.0, egui::Stroke::new(1.0_f32, c));
            }
            _ => {
                if r.gate || !r.sealed {
                    p.rect_stroke(mark, 0.0, egui::Stroke::new(1.0_f32, c), egui::StrokeKind::Inside);
                } else {
                    p.rect_filled(mark, 0.0, c);
                }
                if r.kind == Kind::Version {
                    // seal glyph: a small diamond at the top
                    let top = egui::pos2(x, base_y - h - 3.0);
                    p.add(egui::Shape::convex_polygon(vec![top + egui::vec2(0.0, -3.0), top + egui::vec2(3.0, 0.0), top + egui::vec2(0.0, 3.0), top + egui::vec2(-3.0, 0.0)], c, egui::Stroke::NONE));
                }
                if r.group_size > 1 {
                    p.text(egui::pos2(x, base_y - h - 10.0), egui::Align2::CENTER_CENTER, format!("{}", r.group_size), t::code(t::S11), c);
                }
            }
        }
        if Some(r.act) == st.selected {
            p.line_segment([egui::pos2(x, rect.top() + 2.0), egui::pos2(x, rect.bottom() - 2.0)], egui::Stroke::new(1.0_f32, t::ACCENT));
        }
        if let Some(pt) = pointer {
            if (pt.x - x).abs() < per.max(3.0) / 2.0 {
                hovered = Some(i);
            }
        }
    }
    // live edge
    if st.selected.is_none() && !st.detached {
        p.line_segment([egui::pos2(rect.right() - 6.0, rect.top() + 2.0), egui::pos2(rect.right() - 6.0, rect.bottom() - 2.0)], egui::Stroke::new(1.0_f32, t::ACCENT));
    }
    // hover card
    if let Some(i) = hovered {
        let r = &rows[i];
        let tracks = s.song.as_ref().map(|sg| sg.tracks.clone()).unwrap_or_default();
        let r2 = r.clone();
        resp.clone().on_hover_ui_at_pointer(|ui| {
            ui.horizontal(|ui| {
                t::avatar(ui, people.color(&r2.being), people.initial(&r2.being), true, 16.0);
                t::strong(ui, &people.label(&r2.being));
                t::muted(ui, being_kind_word(people.kind(&r2.being)));
            });
            t::small(ui, &format!("Studio {}", r2.being.name.short_code()));
            ui.label(egui::RichText::new(format!("{} · {}", kind_word(r2.kind), r2.summary)).font(t::text(t::S12)));
            t::small(ui, &format!("{} · {}", thing_label(&tracks, &r2.thing), s.song.as_ref().and_then(|sg| sg.histories.iter().find(|h| h.history == r2.history)).map(|h| h.name.clone()).unwrap_or_default()));
            ui.label(egui::RichText::new(format!("#{}  {}'s #{}  seal {}{}", r2.number, people.label(&r2.being), r2.act_seq, r2.seal_seq.map(|x| x.to_string()).unwrap_or_else(|| "-".into()), if r2.sealed { "" } else { "  (waiting to be sealed)" })).font(t::code(t::S11)).color(t::MUTED));
            t::small(ui, &format!("stamped {} ({}){}", ago(r2.stamped_at), exact(r2.stamped_at), r2.received_at.map(|m| format!(" · received {}", ago(m))).unwrap_or_default()));
            if let Some(pos) = r2.pos {
                t::small(ui, &bar_of(Some(pos)));
            }
            t::small(ui, &format!("FLit {} · {}", r2.app_version, host_name(&r2.host.0)));
            if r2.gate {
                ui.label(egui::RichText::new("gate: refused or closed").font(t::text(t::S11)).color(t::WARN));
            }
            if r2.undone {
                t::small(ui, "undone");
            }
        });
        if resp.clicked() {
            st.selected = Some(r.act);
            st.detached = true;
            shared.act(Request::ShowMoment { at: Some(r.act) });
        }
        let r3 = r.clone();
        let me2 = me.clone();
        resp.context_menu(|ui| {
            if ui.button("Go back to here").clicked() {
                shared.act(Request::GoBack { to: r3.act });
                st.selected = None;
                st.detached = false;
                ui.close_menu();
            }
            if ui.button("New history from here").clicked() {
                shared.act(Request::NewHistory { name: format!("from #{}", r3.number), from: r3.act });
                ui.close_menu();
            }
            if ui.button("Pin chat here").clicked() {
                st.chat_pin_from_tape = Some(r3.act);
                st.pin = Some(r3.act);
                st.screen = Screen::Chat;
                ui.close_menu();
            }
            if ui.button("Show this member's moments").clicked() {
                st.lens = if r3.being == me2 { Lens::Me(me2.clone()) } else { Lens::Being(r3.being.clone()) };
                ui.close_menu();
            }
            if !matches!(r3.kind, Kind::Crew | Kind::Being | Kind::Grant | Kind::Keeper | Kind::History | Kind::Flag | Kind::Presence | Kind::Settings) && !r3.inherited {
                if ui.button(if r3.undone { "Undo the undo" } else { "Undo" }).clicked() {
                    shared.act(Request::Undo { target: r3.act });
                    ui.close_menu();
                }
            }
        });
    }
}

fn spacing(st: &PanelState, width: f32, n: usize) -> f32 {
    let fit = ((width - 24.0) / n.max(1) as f32).clamp(2.0, 14.0);
    (fit * st.zoom).clamp(1.5, 40.0)
}

// ----- the Merge dialog -----

fn merge_dialog(ctx: &egui::Context, st: &mut PanelState, s: &Status, shared: &Shared) {
    if !st.merge.open {
        return;
    }
    let Some(song) = &s.song else {
        st.merge.open = false;
        return;
    };
    let histories = song.histories.clone();
    let name_of = |h: Option<HistoryId>| h.and_then(|h| histories.iter().find(|x| x.history == h)).map(|x| x.name.clone()).unwrap_or_else(|| "-".into());
    let mut open = st.merge.open;
    egui::Window::new("Merge").collapsible(false).resizable(true).default_width(560.0).open(&mut open).show(ctx, |ui| {
        ui.horizontal(|ui| {
            t::muted(ui, "FROM");
            egui::ComboBox::from_id_salt("merge-from").selected_text(name_of(st.merge.from)).show_ui(ui, |ui| {
                for h in &histories {
                    if ui.selectable_label(st.merge.from == Some(h.history), &h.name).clicked() {
                        st.merge.from = Some(h.history);
                        st.merge.requested = None;
                    }
                }
            });
            if t::quiet(ui, "swap").clicked() {
                std::mem::swap(&mut st.merge.from, &mut st.merge.to);
                st.merge.requested = None;
            }
            t::muted(ui, "TO");
            egui::ComboBox::from_id_salt("merge-to").selected_text(name_of(st.merge.to)).show_ui(ui, |ui| {
                for h in &histories {
                    if ui.selectable_label(st.merge.to == Some(h.history), &h.name).clicked() {
                        st.merge.to = Some(h.history);
                        st.merge.requested = None;
                    }
                }
            });
        });
        if st.merge.to != Some(song.history) {
            ui.label(egui::RichText::new("Switch to the TO history first (you merge into the history you are in).").font(t::text(t::S11)).color(t::WARN));
            if let Some(to) = st.merge.to {
                if t::button(ui, "Switch").clicked() {
                    shared.act(Request::SwitchHistory { history: to });
                }
            }
        }
        ui.add_space(t::GAP);
        let whole = t::segmented(ui, if st.merge.whole { 0 } else { 1 }, &["Whole history", "Chosen tracks"]) == 0;
        if whole != st.merge.whole {
            st.merge.whole = whole;
            st.merge.requested = None;
        }
        let info = shared.merge.lock().clone();
        if !st.merge.whole {
            for tr in &song.tracks {
                let mut on = st.merge.tracks.contains(&tr.track);
                let count = info.as_ref().and_then(|i| i.counts.iter().find(|(x, _)| *x == tr.track).map(|(_, n)| *n)).unwrap_or(tr.moments);
                if ui.checkbox(&mut on, format!("{} ({} moments)", tr.name, count)).changed() {
                    if on {
                        st.merge.tracks.push(tr.track.clone());
                    } else {
                        st.merge.tracks.retain(|x| *x != tr.track);
                    }
                    st.merge.requested = None;
                }
            }
        }
        // preview
        if let (Some(from), Some(_)) = (st.merge.from, st.merge.to) {
            let tracks = if st.merge.whole { vec![] } else { st.merge.tracks.clone() };
            if st.merge.requested.as_ref() != Some(&(from, tracks.clone())) {
                st.merge.requested = Some((from, tracks.clone()));
                shared.act(Request::MergePreview { from_history: from, tracks });
            }
        }
        if let Some(i) = &info {
            t::section_label(ui, "What comes over");
            t::muted(ui, &format!("{} moments{}", i.acts, if i.flac_needed > 0 { format!(" · {} archive chunks still to download (they are pulled before the merge lands)", i.flac_needed) } else { String::new() }));
            if !i.conflicts.is_empty() {
                t::section_label(ui, "Conflicts");
                t::small(ui, "Both sides changed these. Pick a side for each; what TO already has wins by default.");
                ui.horizontal(|ui| {
                    if t::quiet(ui, "Keep all FROM").clicked() {
                        for c in &i.conflicts {
                            st.merge.choices.insert((c.thing.clone(), c.property.clone()), true);
                        }
                    }
                    if t::quiet(ui, "Keep all TO").clicked() {
                        for c in &i.conflicts {
                            st.merge.choices.insert((c.thing.clone(), c.property.clone()), false);
                        }
                    }
                });
                let beings = shared.beings.lock().clone();
                let people = People { beings: &beings };
                for c in &i.conflicts {
                    let key = (c.thing.clone(), c.property.clone());
                    let keep_from = *st.merge.choices.get(&key).unwrap_or(&c.keep_from);
                    t::raised(ui, |ui| {
                        t::strong(ui, &c.label);
                        ui.horizontal(|ui| {
                            let a = egui::RichText::new(format!("A  {}  ({})", c.from_summary, people.label(&c.from_being))).font(t::text(t::S12)).color(being_color(people.color(&c.from_being)));
                            let b = egui::RichText::new(format!("B  {}  ({})", c.to_summary, people.label(&c.to_being))).font(t::text(t::S12)).color(being_color(people.color(&c.to_being)));
                            ui.label(a);
                            ui.add_space(t::SECTION);
                            ui.label(b);
                        });
                        ui.horizontal(|ui| {
                            let idx = t::segmented(ui, if keep_from { 0 } else { 1 }, &["Keep FROM (A)", "Keep TO (B)"]);
                            st.merge.choices.insert(key.clone(), idx == 0);
                            ui.add_enabled(false, egui::Button::new("Listen A")).on_disabled_hover_text("Listen scrub from stored renders is coming");
                            ui.add_enabled(false, egui::Button::new("Listen B"));
                        });
                    });
                }
            }
            ui.add_space(t::GAP);
            ui.horizontal(|ui| {
                let can = st.merge.to == Some(song.history) && i.acts > 0;
                if ui.add_enabled(can, egui::Button::new(egui::RichText::new("Confirm merge").font(t::text_medium(t::S12)).color(t::BG)).fill(t::ACCENT).min_size(egui::vec2(0.0, t::PRIMARY_H))).clicked() {
                    let choices: Vec<(Thing, String, bool)> = i.conflicts.iter().map(|c| (c.thing.clone(), c.property.clone(), *st.merge.choices.get(&(c.thing.clone(), c.property.clone())).unwrap_or(&c.keep_from))).collect();
                    shared.act(Request::Merge { from_history: i.from_history, tracks: if st.merge.whole { vec![] } else { st.merge.tracks.clone() }, choices });
                    st.merge.open = false;
                    *shared.merge.lock() = None;
                }
                ui.add_enabled(false, egui::Button::new("Listen")).on_disabled_hover_text("Listen scrub from stored renders is coming");
                t::small(ui, "Lands as one moment, undoable as one.");
            });
        } else {
            t::muted(ui, "Working out what differs");
        }
    });
    st.merge.open = open;
}

// ----- screens -----

/// Two columns when there is room, stacked when there is not. One closure,
/// called with side 0 then side 1, so it may borrow the UI state once.
fn columns2(ui: &mut egui::Ui, mut body: impl FnMut(&mut egui::Ui, usize)) {
    if ui.available_width() > 860.0 {
        ui.columns(2, |cols| {
            body(&mut cols[0], 0);
            body(&mut cols[1], 1);
        });
    } else {
        body(ui, 0);
        body(ui, 1);
    }
}

/// The DAW project and the song: linked, not yet a song, or a mismatch.
fn link_card(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut PanelState) {
    match &s.link {
        ProjectLink::Unknown => {}
        ProjectLink::Linked { project } => {
            t::card(ui, |ui| {
                ui.horizontal(|ui| {
                    t::dot(ui, t::OK);
                    ui.label(egui::RichText::new(project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).font(t::text_medium(t::S12)));
                    t::muted(ui, "is this song");
                });
            });
        }
        ProjectLink::Unlinked { project } => {
            t::card(ui, |ui| {
                let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.horizontal(|ui| {
                    t::dot(ui, t::WARN);
                    ui.label(egui::RichText::new(&file).font(t::text_medium(t::S12)));
                    t::muted(ui, "is not a song yet");
                });
                t::small(ui, "Make it one: the song lives beside the project file, and every moment from now on is kept.");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.new_name).hint_text("song name").desired_width(200.0));
                    if t::primary(ui, "Make it a song").clicked() {
                        let name = if st.new_name.trim().is_empty() { project.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Song".into()) } else { st.new_name.trim().to_string() };
                        if let Some(dir) = project.parent() {
                            shared.act(Request::ShareProject { name, dir: dir.to_path_buf(), project: project.clone() });
                        }
                    }
                });
            });
        }
        ProjectLink::Mismatch { project, song_dir, song_name } => {
            t::card(ui, |ui| {
                let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.horizontal(|ui| {
                    t::dot(ui, t::BAD);
                    ui.label(egui::RichText::new("Edits are paused").font(t::text_medium(t::S12)).color(t::BAD));
                });
                t::small(ui, &format!("{file} is open, but the song \"{song_name}\" belongs to {}.", song_dir.display()));
                ui.horizontal(|ui| {
                    if t::button(ui, &format!("Make {file} its own song")).clicked() {
                        if let Some(dir) = project.parent() {
                            shared.act(Request::ShareProject { name: project.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Song".into()), dir: dir.to_path_buf(), project: project.clone() });
                        }
                    }
                    if t::quiet(ui, "Close the song").clicked() {
                        shared.act(Request::CloseSong);
                    }
                });
            });
        }
        ProjectLink::OtherFile { project, song_from } => {
            t::card(ui, |ui| {
                let file = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.horizontal(|ui| {
                    t::dot(ui, t::BAD);
                    ui.label(egui::RichText::new("Edits are paused").font(t::text_medium(t::S12)).color(t::BAD));
                });
                t::small(ui, &format!("This folder's song was made from {song_from}, but {file} is open. Save it as {song_from}, or make {file} its own song in another folder."));
            });
        }
    }
}

fn home_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut PanelState, logo_tex: Option<&egui::TextureHandle>) {
    ui.horizontal(|ui| {
        if let Some(tex) = logo_tex {
            ui.image((tex.id(), egui::vec2(40.0, 40.0)));
        }
        ui.vertical(|ui| {
            t::heading(ui, "Work on one song together, each in your own DAW.");
            t::small(ui, "Every knob, fader, plugin, note, take and save is a moment. Scrub back any time; nothing is ever lost.");
        });
    });
    link_card(ui, s, shared, st);
    columns2(ui, |ui, side| if side == 0 {
            match &s.song {
                Some(song) => song_card(ui, s, song, shared, st),
                None => {
                    t::card(ui, |ui| {
                        t::section_label(ui, "New song");
                        field_paste(ui, "Name", &mut st.new_name, "My song");
                        field_paste(ui, "Folder", &mut st.new_dir, r"C:\Music\MySong");
                        if t::primary(ui, "Create").clicked() && !st.new_dir.trim().is_empty() {
                            shared.act(Request::NewSong { name: st.new_name.trim().to_string(), dir: st.new_dir.trim().into() });
                        }
                        t::section_label(ui, "Join with a song code");
                        field_paste(ui, "Song code", &mut st.join_code, "flit1...");
                        field_paste(ui, "Folder for your copy", &mut st.join_dir, r"C:\Music\TheirSong");
                        if t::primary(ui, "Join").clicked() && !st.join_code.trim().is_empty() && !st.join_dir.trim().is_empty() {
                            shared.act(Request::JoinWithCode { code: st.join_code.trim().to_string(), dir: st.join_dir.trim().into() });
                        }
                    });
                }
            }
            let songs = shared.songs.lock().clone();
            if !songs.is_empty() {
                t::card(ui, |ui| {
                    t::section_label(ui, "Your songs");
                    for (name, dir) in songs {
                        ui.horizontal(|ui| {
                            if t::quiet(ui, &name).on_hover_text(dir.display().to_string()).clicked() {
                                shared.act(Request::OpenSong { dir: dir.clone() });
                            }
                        });
                    }
                });
            }
        } else {
            if let Some(song) = &s.song {
                tracks_card(ui, s, song, shared, st);
            }
        });
}

fn song_card(ui: &mut egui::Ui, s: &Status, song: &SongStatus, shared: &Shared, st: &mut PanelState) {
    t::card(ui, |ui| {
        t::section_label(ui, "This song");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&song.name).font(t::text_medium(t::S18)));
            t::muted(ui, &format!("{} moments · {} histories", song.moments, song.histories.len()));
        });
        t::small(ui, &song.dir.display().to_string());
        ui.horizontal(|ui| {
            t::muted(ui, "keeper:");
            t::small(ui, &song.keeper.as_ref().map(|(n, m)| format!("{} on {}{}", if *n == s.name { "you".to_string() } else { n.short_code() }, m, if song.i_am_keeper { " (this machine)" } else { "" })).unwrap_or_else(|| "-".into()));
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut st.version_name).hint_text("version name").desired_width(180.0));
            if t::primary(ui, "Save version").clicked() {
                let name = if st.version_name.trim().is_empty() { "Save".to_string() } else { st.version_name.trim().to_string() };
                shared.act(Request::SaveVersion { name });
                st.version_name.clear();
            }
        });
        t::section_label(ui, "Song code");
        t::small(ui, "Give this to someone to let them ask to join. You approve them in Crew.");
        if st.qr.as_ref().map(|(c, _)| c != &song.code).unwrap_or(true) {
            st.qr = Some((song.code.clone(), qr_matrix(&song.code)));
        }
        ui.horizontal(|ui| {
            if let Some((_, m)) = &st.qr {
                draw_qr(ui, m, 112.0);
            }
            ui.vertical(|ui| {
                ui.add(egui::TextEdit::multiline(&mut song.code.clone()).desired_rows(3).desired_width(f32::INFINITY).font(t::code(t::S11)));
                if t::button(ui, "Copy code").clicked() {
                    ui.ctx().copy_text(song.code.clone());
                }
            });
        });
        ui.horizontal(|ui| {
            if t::quiet(ui, "Close song").clicked() {
                shared.act(Request::CloseSong);
            }
        });
    });
}

/// Tracks with who is looking at them (the presence map).
fn tracks_card(ui: &mut egui::Ui, s: &Status, song: &SongStatus, shared: &Shared, st: &mut PanelState) {
    let beings = shared.beings.lock().clone();
    let people = People { beings: &beings };
    t::card(ui, |ui| {
        t::section_label(ui, "Tracks");
        if song.tracks.is_empty() {
            t::muted(ui, "Nothing yet. Tracks appear as your DAW reports them or as moments land on them.");
        }
        for tr in &song.tracks {
            ui.horizontal(|ui| {
                let on = st.lens == Lens::Track(tr.track.clone());
                if ui.selectable_label(on, egui::RichText::new(&tr.name).font(t::text(t::S12))).clicked() {
                    st.lens = if on { Lens::Song } else { Lens::Track(tr.track.clone()) };
                }
                t::small(ui, &format!("{} · {} moments · {} plugins", tr.kind, tr.moments, tr.devices));
                for b in beings.iter().filter(|b| b.presence.as_ref().and_then(|p| p.track.as_ref()) == Some(&tr.track)) {
                    let in_my_history = b.history == Some(song.history);
                    let r = t::avatar(ui, b.color, people.initial(&b.being), in_my_history && b.online, 16.0);
                    if r.clicked() && !b.acting {
                        shared.act(Request::Follow { being: Some(b.being.clone()), keep: false });
                    }
                    r.on_hover_ui(|ui| {
                        being_card(ui, b, &people, s);
                        t::small(ui, "click: follow once");
                    });
                }
            });
        }
        if let Some(f) = &s.following {
            ui.horizontal(|ui| {
                t::small(ui, &format!("following {}", people.label(f)));
                if t::quiet(ui, "stop").clicked() {
                    shared.act(Request::Follow { being: None, keep: false });
                }
            });
        }
    });
}

fn track_mode_view(ui: &mut egui::Ui, host: &dyn Host, s: &Status, shared: &Shared) {
    let mode = host.mode();
    t::heading(ui, mode.name());
    match mode {
        Mode::Track => t::muted(ui, "This instance sends its track out at the print point (after inserts, fader and pan; before sends) and feeds saves. Put it last on the track."),
        Mode::Receiver => t::muted(ui, "This instance plays a friend's track in. Name it after their track."),
        Mode::TalkSend => t::muted(ui, "Talk: a live track outside the song. Everyone in the crew who listens hears it."),
        Mode::TalkReceive => {
            t::muted(ui, "Talk: hear what someone plays live. Record it into a track with the button below.");
            ui.horizontal(|ui| {
                let mut track = host.track();
                if ui.add(egui::TextEdit::singleline(&mut track).hint_text("track to record into").desired_width(200.0)).changed() {
                    host.set_track(track.clone());
                }
                let tr = TrackId::new(track.trim());
                if t::button(ui, "Start recording").clicked() {
                    shared.act(Request::RecordTalk { track: tr.clone(), start: true });
                }
                if t::button(ui, "Stop and keep").clicked() {
                    shared.act(Request::RecordTalk { track: tr, start: false });
                }
            });
        }
        Mode::Master => {}
    }
    ui.add_space(t::SECTION);
    t::small(ui, &format!("Helper: {} · song: {}", if shared.connected.load(Ordering::Relaxed) { "connected" } else { "reconnecting" }, s.song.as_ref().map(|x| x.name.clone()).unwrap_or_else(|| "none open".into())));
}

fn crew_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut PanelState) {
    let beings = shared.beings.lock().clone();
    let people = People { beings: &beings };
    let i_own = s.song.as_ref().map(|sg| sg.owners.contains(&s.name)).unwrap_or(false);
    columns2(ui, |ui, side| if side == 0 {
            if let Some(song) = &s.song {
                t::card(ui, |ui| {
                    t::section_label(ui, "Crew");
                    for m in &song.members {
                        let owner = song.owners.contains(m);
                        let online = s.peers.iter().any(|p| p.id == *m && p.connected) || *m == s.name;
                        ui.horizontal(|ui| {
                            t::dot(ui, if online { t::OK } else { t::FAINT });
                            let label = beings.iter().find(|b| b.being.name == *m).map(|b| b.display_name.clone()).unwrap_or_else(|| m.short_code());
                            ui.label(egui::RichText::new(label).font(t::text(t::S12)));
                            t::small(ui, &m.short_code());
                            if owner {
                                t::small(ui, "owner");
                            }
                            if i_own && *m != s.name {
                                if !owner && t::quiet(ui, "Make owner").clicked() {
                                    shared.act(Request::Promote { id: *m });
                                }
                                if t::quiet(ui, "Remove").clicked() {
                                    shared.act(Request::RemoveFromCrew { id: *m });
                                }
                            }
                        });
                    }
                    t::section_label(ui, "Members in this song");
                    for b in beings.iter() {
                        ui.horizontal(|ui| {
                            t::avatar(ui, b.color, people.initial(&b.being), b.online, 16.0);
                            ui.label(egui::RichText::new(&b.display_name).font(t::text(t::S12)));
                            t::small(ui, being_kind_word(b.kind));
                            if b.revoked {
                                t::small(ui, "revoked");
                            }
                            if b.flags > 0 {
                                ui.label(egui::RichText::new(format!("{} flag(s)", b.flags)).font(t::text(t::S11)).color(t::WARN));
                            }
                            if !b.granted.is_empty() {
                                t::small(ui, &format!("granted: {}", b.granted.iter().map(|x| track_label(&song.tracks, x)).collect::<Vec<_>>().join(", ")));
                            }
                            if !b.mine {
                                if t::quiet(ui, "Follow").clicked() {
                                    shared.act(Request::Follow { being: Some(b.being.clone()), keep: true });
                                }
                                if t::quiet(ui, "Flag").clicked() {
                                    shared.act(Request::Flag { being: b.being.clone(), note: st.flag_note.trim().to_string() });
                                }
                                if i_own {
                                    if t::quiet(ui, "Block").clicked() {
                                        shared.act(Request::BlockBeing { being: b.being.clone() });
                                    }
                                    if t::quiet(ui, "Pardon").on_hover_text("Accept this member's moments again after an equivocation").clicked() {
                                        shared.act(Request::Pardon { being: b.being.clone() });
                                    }
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut st.flag_note).hint_text("note for a flag (optional)").desired_width(260.0));
                    });
                });
            }
            if !s.pending_joins.is_empty() {
                t::card(ui, |ui| {
                    t::section_label(ui, "Asking to join");
                    for (id, name) in &s.pending_joins {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(name).font(t::text_medium(t::S12)));
                            t::small(ui, &id.short_code());
                            if t::primary(ui, "Approve").clicked() {
                                shared.act(Request::Approve { id: *id, owner: false });
                            }
                            if t::quiet(ui, "Deny").clicked() {
                                shared.act(Request::Deny { id: *id });
                            }
                            if t::danger(ui, "Block").clicked() {
                                shared.act(Request::Block { id: *id });
                            }
                        });
                    }
                });
            }
            if !s.invites.is_empty() {
                t::card(ui, |ui| {
                    t::section_label(ui, "Invites");
                    for (from, song, name) in &s.invites {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} invited you to \"{name}\"", from.short_code()));
                            ui.add(egui::TextEdit::singleline(&mut st.join_dir).hint_text("folder for your copy").desired_width(180.0));
                            if t::primary(ui, "Accept").clicked() && !st.join_dir.trim().is_empty() {
                                shared.act(Request::AcceptInvite { song: *song, dir: st.join_dir.trim().into() });
                            }
                        });
                    }
                });
            }
        } else {
            t::card(ui, |ui| {
                t::section_label(ui, "Friends");
                t::small(ui, "Add a friend by their friend code. Then invite them to the open song.");
                field_paste(ui, "Friend code", &mut st.friend_code, "flitid1...");
                field_paste(ui, "Nickname", &mut st.friend_name, "Sam");
                if t::button(ui, "Add friend").clicked() && !st.friend_code.trim().is_empty() {
                    shared.act(Request::AddFriend { id: st.friend_code.trim().to_string(), name: st.friend_name.trim().to_string() });
                    shared.act(Request::Friends);
                }
                let friends = shared.friends.lock().clone();
                for f in friends {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&f.name).font(t::text(t::S12)));
                        t::small(ui, &f.id.short_code());
                        if s.song.is_some() && i_own && t::quiet(ui, "Invite").clicked() {
                            shared.act(Request::Invite { friend: f.id });
                        }
                    });
                }
                t::section_label(ui, "Your friend code");
                ui.add(egui::TextEdit::multiline(&mut s.id_text.clone()).desired_rows(2).desired_width(f32::INFINITY).font(t::code(t::S11)));
                if t::button(ui, "Copy").clicked() {
                    ui.ctx().copy_text(s.id_text.clone());
                }
            });
        });
}

fn beings_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut PanelState) {
    let beings = shared.beings.lock().clone();
    let people = People { beings: &beings };
    columns2(ui, |ui, side| if side == 0 {
            t::card(ui, |ui| {
                t::section_label(ui, "Members of your Studio");
                t::small(ui, "You act as one member at a time. Each has its own color, moments and presence.");
                for b in beings.iter().filter(|b| b.mine) {
                    ui.horizontal(|ui| {
                        t::avatar(ui, b.color, people.initial(&b.being), !b.revoked, 16.0);
                        ui.label(egui::RichText::new(&b.display_name).font(if b.acting { t::text_medium(t::S12) } else { t::text(t::S12) }));
                        t::small(ui, &format!("{} · {}", b.being.being, being_kind_word(b.kind)));
                        if b.acting {
                            t::small(ui, "acting");
                        } else if !b.revoked && t::quiet(ui, "Act as").clicked() {
                            shared.act(Request::ActAs { being: b.being.being.clone() });
                        }
                        if !b.revoked && !b.acting && t::quiet(ui, "Revoke").clicked() {
                            shared.act(Request::RevokeBeing { id: b.being.being.clone() });
                        }
                    });
                }
                t::section_label(ui, "Add a member");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.being_display).hint_text("display name").desired_width(140.0));
                    ui.add(egui::TextEdit::singleline(&mut st.being_id).hint_text("id (short, no spaces)").desired_width(140.0));
                    st.being_kind = t::segmented(ui, st.being_kind, &["human", "ai", "scripted"]);
                    if t::button(ui, "Add").clicked() && !st.being_display.trim().is_empty() {
                        let id = if st.being_id.trim().is_empty() { st.being_display.trim().to_lowercase().replace(' ', "-") } else { st.being_id.trim().to_string() };
                        let kind = match st.being_kind {
                            1 => BeingKind::Ai,
                            2 => BeingKind::Scripted,
                            _ => BeingKind::Human,
                        };
                        shared.act(Request::NewBeing { id: BeingId::new(&id), kind, display_name: st.being_display.trim().to_string(), color: None });
                        st.being_display.clear();
                        st.being_id.clear();
                    }
                });
                t::small(ui, "AI and scripted members land in their own history unless you grant them tracks.");
                if s.song.is_some() {
                    t::section_label(ui, "Grant tracks to an AI or scripted member");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut st.grant_being).hint_text("member id").desired_width(120.0));
                        ui.add(egui::TextEdit::singleline(&mut st.grant_tracks).hint_text("track ids, comma separated").desired_width(220.0));
                        if t::button(ui, "Grant").clicked() {
                            let tracks: Vec<TrackId> = st.grant_tracks.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).map(TrackId::new).collect();
                            shared.act(Request::Grant { being: BeingId::new(st.grant_being.trim()), tracks, grant: true });
                        }
                        if t::quiet(ui, "Revoke").clicked() {
                            let tracks: Vec<TrackId> = st.grant_tracks.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()).map(TrackId::new).collect();
                            shared.act(Request::Grant { being: BeingId::new(st.grant_being.trim()), tracks, grant: false });
                        }
                    });
                }
            });
        } else {
            t::card(ui, |ui| {
                t::section_label(ui, "Studios on this machine");
                t::small(ui, "A Studio is one key that holds authority. Each Studio has its own members, songs and crews.");
                let names = shared.names.lock().clone();
                for n in &names {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&n.label).font(if n.current { t::text_medium(t::S12) } else { t::text(t::S12) }));
                        t::small(ui, &n.id.short_code());
                        if !n.backed_up {
                            ui.label(egui::RichText::new("not backed up").font(t::text(t::S11)).color(t::WARN));
                        }
                        if n.current {
                            t::small(ui, "current");
                        } else {
                            if t::quiet(ui, "Switch").clicked() {
                                shared.act(Request::SwitchName { id: n.id });
                            }
                            if t::danger(ui, "Remove from this machine").clicked() {
                                shared.act(Request::RemoveName { id: n.id });
                                shared.act(Request::Names);
                            }
                        }
                    });
                }
                t::section_label(ui, "Back up this Studio");
                t::small(ui, "Losing the key with no backup loses this Studio's authority. The songs stay on every crew machine.");
                ui.horizontal(|ui| {
                    if t::button(ui, "Show recovery code").clicked() {
                        shared.act(Request::RecoveryCode);
                    }
                    if let Some(code) = shared.text.lock().clone() {
                        ui.add(egui::TextEdit::singleline(&mut code.clone()).desired_width(300.0).font(t::code(t::S11)));
                        if t::quiet(ui, "Copy").clicked() {
                            ui.ctx().copy_text(code.clone());
                        }
                        if t::quiet(ui, "Hide").clicked() {
                            *shared.text.lock() = None;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.backup_pass).password(true).hint_text("passphrase").desired_width(140.0));
                    ui.add(egui::TextEdit::singleline(&mut st.backup_path).hint_text(r"C:\...\flit-studio.backup").desired_width(220.0));
                    if t::button(ui, "Write backup file").clicked() && !st.backup_path.trim().is_empty() {
                        shared.act(Request::BackupName { passphrase: st.backup_pass.clone(), path: st.backup_path.trim().into() });
                    }
                });
                t::section_label(ui, "New or imported Studio");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.name_label).hint_text("label").desired_width(140.0));
                    if t::button(ui, "New Studio").clicked() && !st.name_label.trim().is_empty() {
                        let being = st.name_label.trim().to_lowercase().replace(' ', "-");
                        shared.act(Request::NewName { label: st.name_label.trim().to_string(), being: BeingId::new(&being), display_name: st.name_label.trim().to_string(), kind: BeingKind::Human });
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.import_path).hint_text("backup file").desired_width(220.0));
                    if t::button(ui, "Import file").clicked() && !st.import_path.trim().is_empty() {
                        shared.act(Request::ImportName { path: st.import_path.trim().into(), passphrase: st.backup_pass.clone() });
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut st.recovery).hint_text("recovery code").desired_width(300.0));
                    paste_button(ui, &mut st.recovery);
                    if t::button(ui, "Import code").clicked() && !st.recovery.trim().is_empty() {
                        let label = if st.name_label.trim().is_empty() { "Restored".to_string() } else { st.name_label.trim().to_string() };
                        let being = label.to_lowercase().replace(' ', "-");
                        shared.act(Request::ImportRecovery { code: st.recovery.trim().to_string(), label, being: BeingId::new(&being) });
                    }
                });
                ui.horizontal(|ui| {
                    if t::quiet(ui, "Log out").on_hover_text("Stop using this Studio here; the key stays so logging back in is instant").clicked() {
                        shared.act(Request::Logout);
                    }
                });
            });
        });
}

fn chat_view(ui: &mut egui::Ui, s: &Status, shared: &Shared, st: &mut PanelState) {
    let Some(song) = &s.song else { return };
    let beings = shared.beings.lock().clone();
    let people = People { beings: &beings };
    t::card(ui, |ui| {
        t::section_label(ui, "Chat");
        let chat = shared.chat.lock().clone();
        egui::ScrollArea::vertical().max_height(ui.available_height() - 80.0).stick_to_bottom(true).show(ui, |ui| {
            for c in &chat {
                ui.horizontal_wrapped(|ui| {
                    t::avatar(ui, c.color, people.initial(&c.from), true, 14.0);
                    ui.label(egui::RichText::new(&c.from_label).font(t::text_medium(t::S12)).color(being_color(c.color)));
                    if c.history != song.history {
                        t::small(ui, &format!("[{}]", c.history_name));
                    }
                    ui.label(egui::RichText::new(&c.text).font(t::text(t::S12)));
                    t::small(ui, &ago(c.stamped_at));
                    if let Some(p) = c.pinned {
                        if t::quiet(ui, "go to moment").clicked() {
                            st.selected = Some(p);
                            st.detached = true;
                            shared.act(Request::ShowMoment { at: Some(p) });
                        }
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut st.chat_input).hint_text("say something").desired_width(ui.available_width() - 200.0));
            paste_button(ui, &mut st.chat_input);
            if let Some(p) = st.pin {
                if t::quiet(ui, &format!("pinned #{}", p.short())).on_hover_text("click to unpin").clicked() {
                    st.pin = None;
                }
            } else if let Some(sel) = st.selected {
                if t::quiet(ui, "pin the selected moment").clicked() {
                    st.pin = Some(sel);
                }
            }
            let send = t::primary(ui, "Send").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if send && !st.chat_input.trim().is_empty() {
                shared.act(Request::Chat { text: st.chat_input.trim().to_string(), pinned: st.pin });
                st.chat_input.clear();
                st.pin = None;
            }
        });
    });
}

fn settings_view(ui: &mut egui::Ui, host: &dyn Host, s: &Status, shared: &Shared, st: &mut PanelState) {
    columns2(ui, |ui, side| if side == 0 {
            if host.surface() == Surface::App {
                t::card(ui, |ui| {
                    t::section_label(ui, "FLit app");
                    t::small(ui, "Closing the window keeps FLit running for your DAW's plugins. Quit stops it.");
                    if t::danger(ui, "Quit FLit").clicked() {
                        st.quit = true;
                    }
                });
            }
            t::card(ui, |ui| {
                t::section_label(ui, "Audio");
                let mut hear = if host.surface() == Surface::Plugin { host.hear_reference() } else { s.settings.hear_reference };
                if ui.checkbox(&mut hear, "Hear the reference mix instead of my local mix").changed() {
                    host.set_hear_reference(hear);
                    shared.act(Request::Settings(SettingsPatch { hear_reference: Some(hear), ..Default::default() }));
                }
                let mut submix = s.settings.submix;
                if ui.checkbox(&mut submix, "Send my tracks as one submix").changed() {
                    shared.act(Request::Settings(SettingsPatch { submix: Some(submix), ..Default::default() }));
                }
                ui.horizontal(|ui| {
                    t::muted(ui, "Monitoring bitrate");
                    let mut br = s.settings.monitoring_bitrate as f32 / 1000.0;
                    if ui.add(egui::Slider::new(&mut br, 64.0..=256.0).suffix(" kbps")).drag_stopped() {
                        shared.act(Request::Settings(SettingsPatch { monitoring_bitrate: Some((br * 1000.0) as u32), ..Default::default() }));
                    }
                });
                ui.horizontal(|ui| {
                    t::muted(ui, "Download full-quality audio");
                    let cur = match s.settings.download_full_quality {
                        flit_core::DownloadPolicy::Always => 0,
                        flit_core::DownloadPolicy::OnDemand => 1,
                        flit_core::DownloadPolicy::WifiOnly => 2,
                    };
                    let idx = t::segmented(ui, cur, &["always", "on demand", "Wi-Fi only"]);
                    if idx != cur {
                        let p = match idx {
                            1 => flit_core::DownloadPolicy::OnDemand,
                            2 => flit_core::DownloadPolicy::WifiOnly,
                            _ => flit_core::DownloadPolicy::Always,
                        };
                        shared.act(Request::Settings(SettingsPatch { download_full_quality: Some(p), ..Default::default() }));
                    }
                });
                t::section_label(ui, "Presence");
                let mut tint = s.settings.show_presence_in_daw;
                if ui.checkbox(&mut tint, "Show where others are inside my DAW (track tints, restored afterwards)").changed() {
                    shared.act(Request::Settings(SettingsPatch { show_presence_in_daw: Some(tint), ..Default::default() }));
                }
                t::section_label(ui, "Network");
                let mut relay = s.settings.relay_only;
                if ui.checkbox(&mut relay, "Relay only (never expose my IP; takes effect on restart)").changed() {
                    shared.act(Request::Settings(SettingsPatch { relay_only: Some(relay), ..Default::default() }));
                }
                t::small(ui, &format!("Helper {} · format {} · {}", s.agent_version, s.format_version, s.bridge));
            });
            if let Some(song) = &s.song {
                let i_own = song.owners.contains(&s.name);
                t::card(ui, |ui| {
                    t::section_label(ui, "This song (owners)");
                    ui.horizontal(|ui| {
                        t::muted(ui, "Keeper");
                        t::small(ui, &song.keeper.as_ref().map(|(n, m)| format!("{} on {m}", n.short_code())).unwrap_or_else(|| "-".into()));
                        if i_own && !song.i_am_keeper && t::button(ui, "Move the keeper role here").clicked() {
                            shared.act(Request::MoveKeeper { name: s.name, machine: String::new() });
                        }
                    });
                    if i_own {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut st.keeper_machine).hint_text("Studio public key (base32) [machine]").desired_width(260.0));
                            if t::quiet(ui, "Move to").clicked() {
                                let mut parts = st.keeper_machine.split_whitespace();
                                if let Some(id) = parts.next().and_then(flit_core::Id::from_base32) {
                                    shared.act(Request::MoveKeeper { name: id, machine: parts.next().unwrap_or("").to_string() });
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            t::muted(ui, "Required FLit version");
                            ui.add(egui::TextEdit::singleline(&mut st.required_version).hint_text(song.required_version.clone().unwrap_or_default()).desired_width(100.0));
                            if t::button(ui, "Set").clicked() && !st.required_version.trim().is_empty() {
                                shared.act(Request::SetRequiredVersion { version: st.required_version.trim().to_string() });
                            }
                            if t::quiet(ui, "this one").clicked() {
                                shared.act(Request::SetRequiredVersion { version: s.agent_version.clone() });
                            }
                        });
                        ui.horizontal(|ui| {
                            t::muted(ui, "Reference DAW");
                            t::small(ui, &song.reference_daw.as_ref().map(|b| b.being.0.clone()).unwrap_or_else(|| "-".into()));
                            if t::quiet(ui, "me").clicked() {
                                shared.act(Request::SetReferenceDaw { being: BeingRef { name: s.name, being: s.being.clone() } });
                            }
                        });
                    }
                    t::small(ui, &format!("Requires FLit {} · you run {}", song.required_version.clone().unwrap_or_else(|| "-".into()), s.agent_version));
                });
            }
        } else {
            t::card(ui, |ui| {
                t::section_label(ui, "Updates");
                t::small(ui, "Releases are read from a manifest (GitHub Releases). Installing a version and switching is done by the installer; songs can require an exact version.");
                ui.horizontal(|ui| {
                    if st.manifest_url.is_empty() {
                        st.manifest_url = s.settings.update_manifest_url.clone();
                    }
                    ui.add(egui::TextEdit::singleline(&mut st.manifest_url).hint_text("https://.../releases.json").desired_width(260.0));
                    if t::quiet(ui, "Save").clicked() {
                        shared.act(Request::Settings(SettingsPatch { update_manifest_url: Some(st.manifest_url.trim().to_string()), ..Default::default() }));
                    }
                });
                ui.horizontal(|ui| {
                    t::muted(ui, "Channel");
                    let cur = if s.settings.update_channel == "beta" { 1 } else { 0 };
                    let idx = t::segmented(ui, cur, &["Stable", "Beta"]);
                    if idx != cur {
                        shared.act(Request::Settings(SettingsPatch { update_channel: Some(if idx == 1 { "beta".into() } else { "stable".into() }), ..Default::default() }));
                    }
                    if t::button(ui, "Check now").clicked() {
                        shared.act(Request::CheckUpdates);
                    }
                });
                if let Some(u) = shared.updates.lock().clone() {
                    t::small(ui, &u.message);
                    for r in &u.releases {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&r.version).font(t::code(t::S12)));
                            t::small(ui, &r.channel);
                            if r.installed {
                                t::small(ui, "installed");
                            }
                            if !r.url.is_empty() && t::quiet(ui, "copy link").clicked() {
                                ui.ctx().copy_text(r.url.clone());
                            }
                        });
                        if !r.notes.is_empty() {
                            t::small(ui, &r.notes);
                        }
                    }
                }
                t::small(ui, "The plugin file is locked while the DAW is open: plugin updates apply on the next DAW restart. The helper restarts on its own.");
                t::section_label(ui, "Look");
                if t::quiet(ui, "Style gallery").clicked() {
                    st.gallery = true;
                }
            });
        });
}

#[cfg(test)]
mod tests {
    //! The whole panel drawn headless, in every state a first-time user can
    //! reach. A panic here would take the DAW down with it.
    use super::*;
    use crate::host::PlainHost;
    use flit_core::{main_history, Keypair};

    fn status(named: bool, song: bool) -> Status {
        let name = Keypair::from_seed(1).id();
        let me = BeingRef { name, being: BeingId::new("tabor") };
        let row = BeingRow { being: me.clone(), kind: BeingKind::Human, display_name: "Tabor".into(), color: 0x5B8DEF, revoked: false, delegated: false, granted: vec![], flags: 0, online: true, history: Some(main_history()), presence: None, mine: true, acting: true };
        Status {
            agent_version: "0.1.0".into(),
            format_version: 1,
            name,
            short_code: name.short_code(),
            id_text: "flitid1abc".into(),
            name_label: if named { "Tabor".into() } else { String::new() },
            being: BeingId::new(if named { "tabor" } else { "me" }),
            being_label: "Tabor".into(),
            being_kind: BeingKind::Human,
            my_beings: if named { vec![(BeingId::new("tabor"), "Tabor".into(), BeingKind::Human)] } else { vec![] },
            names_on_machine: 1,
            host: "fl".into(),
            level: 2,
            project: None,
            previewing: None,
            bridge: "x".into(),
            bridge_up: Some(true),
            link: ProjectLink::Unlinked { project: "C:/a/b.flp".into() },
            paused: None,
            connection: Connection::Offline,
            song: song.then(|| SongStatus {
                id: flit_core::Hash::ZERO,
                name: "Song".into(),
                dir: "C:/a".into(),
                code: "flit1code".into(),
                owners: vec![name],
                members: vec![name],
                history: main_history(),
                history_name: "main".into(),
                tracks: vec![TrackInfo { track: TrackId::new("fl-track-1"), name: "Drums".into(), kind: "part".into(), moments: 2, devices: 1 }],
                reference_daw: None,
                moments: 3,
                unsealed: 1,
                unsent: 0,
                tempo: 120.0,
                heard_as_audio: vec![],
                missing_opus: 0,
                missing_flac: 0,
                keeper: Some((name, "m".into())),
                i_am_keeper: true,
                keeper_online: true,
                required_version: Some("0.1.0".into()),
                version_mismatch: None,
                beings: vec![row.clone()],
                histories: vec![HistoryRow { history: main_history(), name: "main".into(), current: true, parent: None, from: None, from_pos: None, created_by: None, beings: vec![me.clone()], moments: 3, unsealed: 1 }],
            }),
            peers: vec![],
            pending_joins: vec![],
            invites: vec![],
            settings: SettingsView { download_full_quality: flit_core::DownloadPolicy::Always, monitoring_bitrate: 128_000, submix: false, hear_reference: false, relay_only: false, show_presence_in_daw: true, update_manifest_url: String::new(), update_channel: "stable".into() },
            warnings: vec![],
            connected_clients: 1,
            following: None,
            update_available: None,
        }
    }

    fn rows() -> Vec<Row> {
        let name = Keypair::from_seed(1).id();
        let me = BeingRef { name, being: BeingId::new("tabor") };
        (0..5u64)
            .map(|i| Row {
                act: flit_core::Hash::of(&i.to_le_bytes()),
                fact: None,
                number: i,
                act_seq: i + 1,
                fact_seq: Some(i + 1),
                seal_seq: Some(i + 1),
                history: main_history(),
                inherited: false,
                being: me.clone(),
                kind: [Kind::Setting, Kind::Version, Kind::Chat, Kind::Merge, Kind::Presence][i as usize],
                thing: Thing::Song,
                property: None,
                summary: "x".into(),
                gate: i == 3,
                sealed: i != 4,
                undone: false,
                group_size: if i == 3 { 4 } else { 1 },
                stamped_at: 1000 + i,
                received_at: Some(1000 + i),
                app_version: "0.1.0".into(),
                host: flit_core::HostTag::new("fl"),
                pos: None,
            })
            .collect()
    }

    fn run(st: &mut PanelState, host: &dyn Host, shared: &Shared, frames: usize) {
        let ctx = egui::Context::default();
        for _ in 0..frames {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))), ..Default::default() }, |ctx| {
                draw(ctx, host, st, shared);
            });
        }
    }

    fn fresh() -> PanelState {
        PanelState::new()
    }

    fn every_state(host: &dyn Host) {
        // no helper yet
        let shared = Shared::default();
        run(&mut fresh(), host, &shared, 3);
        // helper up, unnamed: the first-run walk, every step
        shared.connected.store(true, Ordering::Relaxed);
        *shared.status.lock() = Some(status(false, false));
        run(&mut fresh(), host, &shared, 3);
        // named, no song: every screen
        *shared.status.lock() = Some(status(true, false));
        for screen in [Screen::Home, Screen::Library, Screen::Crew, Screen::Beings, Screen::Chat, Screen::Settings, Screen::Help] {
            let mut st = fresh();
            st.screen = screen;
            run(&mut st, host, &shared, 3);
        }
        // a song open, moments on the tape, merge dialog open, gallery
        *shared.status.lock() = Some(status(true, true));
        *shared.timeline.lock() = rows();
        *shared.beings.lock() = status(true, true).song.unwrap().beings;
        for screen in [Screen::Home, Screen::Library, Screen::Crew, Screen::Beings, Screen::Chat, Screen::Settings, Screen::Help] {
            let mut st = fresh();
            st.screen = screen;
            st.merge.open = true;
            st.merge.whole = true;
            run(&mut st, host, &shared, 3);
            st.axis = Axis::Time;
            st.gallery = true;
            st.help_reference = true;
            run(&mut st, host, &shared, 2);
        }
    }

    /// The whole panel drawn headless, in every state a first-time user can
    /// reach, on both surfaces. A panic here would take a DAW down with it.
    #[test]
    fn panel_draws_in_every_first_run_state() {
        every_state(&PlainHost(Surface::App));
        every_state(&PlainHost(Surface::Plugin));
    }
}
