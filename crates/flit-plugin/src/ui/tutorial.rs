//! The tutorial: first run and the Help button.
//!
//! First run it fills the panel and cannot be skipped until a Name and a
//! first Being exist. From Help it is the same walk with the making steps
//! already done. One idea per screen, plain words.

use super::{field, hint, pill, primary, C_ACCENT, C_ACCENT_LIGHT, C_BG, C_BORDER, C_CARD, C_FIELD, C_MUTED, C_OK, C_TEXT, C_WARN};
use crate::worker::Shared;
use flit_ipc::*;
use nih_plug_egui::egui;

// ----- the seam to the helper -----
//
// Everything the tutorial needs from the backend goes through this block.
// Names work today (the display name in Settings). Keys and Beings are not
// in the IPC yet: fill in the three TODOs and set WIRED to true. Nothing
// below the block needs to change.

/// False while keys and Beings are not in the IPC. The tutorial can still
/// be walked: Beings made here live in the panel only, and the private key
/// box says it cannot show the key yet.
const WIRED: bool = false;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum BeingKind {
    #[default]
    Human,
    Ai,
    Script,
}

#[derive(Clone)]
struct Being {
    name: String,
    kind: BeingKind,
}

/// A fresh pair of keys, shown once right after a Name or a Being is made.
struct Keys {
    public: String,
    private: String,
}

fn has_name(s: &Status) -> bool {
    !s.settings.display_name.trim().is_empty()
}

fn make_name(shared: &Shared, name: &str) {
    shared.act(Request::Settings(SettingsPatch { display_name: Some(name.to_string()), ..Default::default() }));
    // TODO(backend): ask the helper for the Name's keys, so take_keys() returns them.
}

/// The Beings of this Name.
fn beings(_s: &Status) -> Vec<Being> {
    // TODO(backend): read them from Status.
    Vec::new()
}

fn make_being(_shared: &Shared, _name: &str, _kind: BeingKind) {
    // TODO(backend): send the request; its keys come back through take_keys().
}

/// The keys of the Name or Being that was just made. Hands them over once.
fn take_keys(_shared: &Shared) -> Option<Keys> {
    // TODO(backend): take them out of Shared.
    None
}

// ----- state -----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Name,
    Being,
    Moments,
    MomentBar,
    ActsFacts,
    Machines,
    Trust,
    Songs,
    Ready,
    Together,
    Rooms,
    Bridges,
}

const STEPS: [Step; 13] = [
    Step::Welcome,
    Step::Name,
    Step::Being,
    Step::Moments,
    Step::MomentBar,
    Step::ActsFacts,
    Step::Machines,
    Step::Trust,
    Step::Songs,
    Step::Ready,
    Step::Together,
    Step::Rooms,
    Step::Bridges,
];

/// The screen that opens right after a Name or a Being is made.
struct Reveal {
    /// None = the Name.
    being: Option<BeingKind>,
    label: String,
    keys: Option<Keys>,
    at: f64,
    unmasked: bool,
    saved: bool,
    copied_at: Option<f64>,
}

#[derive(Default)]
pub(super) struct State {
    step: usize,
    checked: bool,
    onboarding: bool,
    name_input: String,
    being_input: String,
    being_kind: BeingKind,
    /// The Name made in this sitting (Status catches up a moment later).
    made_name: Option<String>,
    /// Beings made in this sitting (all of them while WIRED is false).
    made_beings: Vec<Being>,
    adding_being: bool,
    reveal: Option<Reveal>,
    confirm_skip: bool,
}

impl State {
    /// First run: the tutorial fills the panel until it is finished or skipped.
    pub(super) fn onboarding(&mut self, s: &Status) -> bool {
        if !self.checked {
            self.checked = true;
            self.onboarding = !has_name(s) || (WIRED && beings(s).is_empty());
        }
        self.onboarding
    }

    /// Opened from the Help button: from the top.
    pub(super) fn open(&mut self) {
        self.step = 0;
        self.reveal = None;
        self.confirm_skip = false;
        self.adding_being = false;
    }

    fn named(&self, s: &Status) -> Option<String> {
        if has_name(s) {
            Some(s.settings.display_name.trim().to_string())
        } else {
            self.made_name.clone()
        }
    }

    fn all_beings(&self, s: &Status) -> Vec<Being> {
        let mut all = beings(s);
        for b in &self.made_beings {
            if !all.iter().any(|x| x.name == b.name) {
                all.push(b.clone());
            }
        }
        all
    }
}

// ----- small pieces -----

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(30.0).strong().color(egui::Color32::WHITE));
    ui.add_space(14.0);
}

fn p(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(16.5).color(C_TEXT));
    ui.add_space(10.0);
}

fn soft(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(14.5).color(C_MUTED));
    ui.add_space(8.0);
}

fn bullets(ui: &mut egui::Ui, lines: &[&str]) {
    for line in lines {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.0, C_ACCENT_LIGHT);
            ui.label(egui::RichText::new(*line).size(16.0).color(C_TEXT));
        });
    }
    ui.add_space(12.0);
}

fn panel<R>(ui: &mut egui::Ui, stroke: egui::Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new().fill(C_CARD).stroke(egui::Stroke::new(1.0_f32, stroke)).corner_radius(12).inner_margin(egui::Margin::same(22)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    })
}

fn kind_word(k: BeingKind) -> &'static str {
    match k {
        BeingKind::Human => "Human",
        BeingKind::Ai => "AI",
        BeingKind::Script => "Script",
    }
}

fn kind_color(k: BeingKind) -> egui::Color32 {
    match k {
        BeingKind::Human => C_ACCENT_LIGHT,
        BeingKind::Ai => egui::Color32::from_rgb(0x7a, 0x9b, 0xd0),
        BeingKind::Script => C_WARN,
    }
}

/// Three cards to pick from, one line of meaning each.
fn kind_picker(ui: &mut egui::Ui, kind: &mut BeingKind) {
    let about = [
        (BeingKind::Human, "A person who plays, records and mixes."),
        (BeingKind::Ai, "An AI partner that can mix, clean up or try ideas."),
        (BeingKind::Script, "A program that does the repeat jobs for you."),
    ];
    ui.columns(3, |cols| {
        for (col, (k, text)) in cols.iter_mut().zip(about) {
            let selected = *kind == k;
            let out = egui::Frame::new()
                .fill(if selected { C_CARD } else { C_FIELD })
                .stroke(egui::Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, if selected { C_ACCENT_LIGHT } else { C_BORDER }))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(14))
                .show(col, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(86.0);
                    ui.label(egui::RichText::new(kind_word(k)).size(17.0).strong().color(if selected { egui::Color32::WHITE } else { C_TEXT }));
                    ui.label(egui::RichText::new(text).size(13.5).color(C_MUTED));
                });
            if col.interact(out.response.rect, col.id().with(("kind", k as u8)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *kind = k;
            }
        }
    });
    ui.add_space(12.0);
}

/// A strip of moments, the way the Moment Bar shows them.
fn tape(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::hover());
    let paint = ui.painter();
    paint.rect_filled(rect, 8.0, C_FIELD);
    let colors = [C_ACCENT_LIGHT, C_ACCENT_LIGHT, C_OK, C_ACCENT_LIGHT, egui::Color32::from_rgb(0x7a, 0x9b, 0xd0), C_OK, C_OK, C_WARN, C_ACCENT_LIGHT];
    let gap = 15.0;
    let count = ((rect.width() - 40.0) / gap).floor().max(0.0) as usize;
    for i in 0..count {
        let x = rect.left() + 18.0 + i as f32 * gap;
        let tall = i % 11 == 7;
        let h = if tall { 30.0 } else { 15.0 };
        let mark = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(if tall { 5.0 } else { 4.0 }, h));
        paint.rect_filled(mark, 2.0, colors[(i * 7 + i / 3) % colors.len()]);
    }
    paint.vline(rect.right() - 12.0, rect.shrink(8.0).y_range(), egui::Stroke::new(2.0_f32, egui::Color32::WHITE));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        hint(ui, "Each mark is one Moment, in the color of who made it.");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| hint(ui, "now"));
    });
    ui.add_space(12.0);
}

/// One history, read from one side.
fn reading(ui: &mut egui::Ui, name: &str, question: &str, subject: &str, lines: &[&str]) {
    panel(ui, C_BORDER, |ui| {
        ui.label(egui::RichText::new(name).size(20.0).strong().color(C_ACCENT_LIGHT));
        ui.label(egui::RichText::new(question).size(14.5).color(C_MUTED));
        ui.add_space(10.0);
        ui.label(egui::RichText::new(subject).size(16.0).strong());
        for line in lines {
            ui.label(egui::RichText::new(*line).size(15.0).color(C_TEXT));
        }
    });
}

/// Main splits into three rooms and comes back together.
fn rooms(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 190.0), egui::Sense::hover());
    let paint = ui.painter();
    paint.rect_filled(rect, 8.0, C_FIELD);
    let line = egui::Stroke::new(1.5_f32, C_BORDER);
    let top = egui::pos2(rect.center().x, rect.top() + 26.0);
    let bottom = egui::pos2(rect.center().x, rect.bottom() - 26.0);
    let mid_y = rect.center().y;
    let rooms = [("Alice", "softer vocals", C_ACCENT_LIGHT), ("Bob", "heavier drums", C_OK), ("Chris", "half time", egui::Color32::from_rgb(0x7a, 0x9b, 0xd0))];
    let reach = (rect.width() / 2.0 - 90.0).max(60.0);
    for (i, (who, what, color)) in rooms.iter().enumerate() {
        let x = rect.center().x + (i as f32 - 1.0) * reach;
        let box_rect = egui::Rect::from_center_size(egui::pos2(x, mid_y), egui::vec2(132.0, 46.0));
        paint.line_segment([egui::pos2(top.x, top.y + 12.0), egui::pos2(x, box_rect.top())], line);
        paint.line_segment([egui::pos2(x, box_rect.bottom()), egui::pos2(bottom.x, bottom.y - 12.0)], line);
        paint.rect_filled(box_rect, 8.0, C_CARD);
        paint.rect_stroke(box_rect, 8.0, egui::Stroke::new(1.0_f32, *color), egui::StrokeKind::Inside);
        paint.text(egui::pos2(x, mid_y - 9.0), egui::Align2::CENTER_CENTER, *who, egui::FontId::proportional(15.0), egui::Color32::WHITE);
        paint.text(egui::pos2(x, mid_y + 10.0), egui::Align2::CENTER_CENTER, *what, egui::FontId::proportional(12.5), C_MUTED);
    }
    paint.text(top, egui::Align2::CENTER_CENTER, "Main", egui::FontId::proportional(15.0), egui::Color32::WHITE);
    paint.text(bottom, egui::Align2::CENTER_CENTER, "Main, with the best of each", egui::FontId::proportional(15.0), egui::Color32::WHITE);
    ui.add_space(14.0);
}

/// Label, a line of meaning, the key in a box, and Copy.
fn key_row(ui: &mut egui::Ui, label: &str, color: egui::Color32, about: &str, shown: &str, copy: Option<&str>, extra: impl FnOnce(&mut egui::Ui)) -> bool {
    let mut copied = false;
    ui.label(egui::RichText::new(label).size(13.0).strong().color(color));
    ui.label(egui::RichText::new(about).size(14.5).color(C_TEXT));
    ui.add_space(4.0);
    egui::Frame::new().fill(C_FIELD).stroke(egui::Stroke::new(1.0_f32, C_BORDER)).corner_radius(8).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(egui::RichText::new(shown).monospace().size(14.0)).wrap());
    });
    ui.horizontal(|ui| {
        if let Some(text) = copy {
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(text.to_string());
                copied = true;
            }
        }
        extra(ui);
    });
    ui.add_space(14.0);
    copied
}

/// The opening. Returns true when the user moves on.
fn reveal_screen(ui: &mut egui::Ui, r: &mut Reveal, s: &Status, shared: &Shared) -> bool {
    if r.keys.is_none() {
        r.keys = take_keys(shared);
    }
    let now = ui.input(|i| i.time);
    let t = (((now - r.at) / 0.9) as f32).clamp(0.0, 1.0);
    let open = 1.0 - (1.0 - t).powi(3);
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    let mut go = false;
    let is_name = r.being.is_none();

    let out = panel(ui, C_ACCENT, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(if is_name { "YOUR NAME" } else { "YOUR BEING" }).size(12.0).color(C_MUTED));
            ui.label(egui::RichText::new(&r.label).size(36.0).strong().color(C_ACCENT_LIGHT));
            if let Some(k) = r.being {
                ui.label(egui::RichText::new(kind_word(k)).size(14.0).color(kind_color(k)));
            }
        });
        ui.add_space(16.0);
        if is_name {
            p(ui, "It is yours now. Every Name is a pair of keys, a lot like a crypto wallet.");
        } else {
            p(ui, "Ready to work. Every Being gets its own pair of keys too.");
        }

        let public = if is_name { Some((s.short_code.clone(), s.id_text.clone())) } else { r.keys.as_ref().map(|k| (k.public.clone(), k.public.clone())) };
        if let Some((shown, copy)) = public {
            key_row(ui, "PUBLIC KEY", C_OK, "Safe to share. It is how friends find you and invite you.", &shown, Some(&copy), |_| {});
        }

        let about = "This proves you are you. Never share it. Anyone who has it can act as you.";
        match &r.keys {
            Some(k) => {
                let shown = if r.unmasked { k.private.clone() } else { "\u{2022}".repeat(40) };
                let private = k.private.clone();
                let (mut unmasked, recently) = (r.unmasked, r.copied_at.map(|at| now - at < 2.0).unwrap_or(false));
                if key_row(ui, "PRIVATE KEY", C_WARN, about, &shown, Some(&private), |ui| {
                    if ui.button(if unmasked { "Hide" } else { "Show" }).clicked() {
                        unmasked = !unmasked;
                    }
                    if recently {
                        ui.label(egui::RichText::new("Copied").size(13.0).color(C_OK));
                    }
                }) {
                    r.copied_at = Some(now);
                }
                r.unmasked = unmasked;
                soft(ui, "Copy it now and keep it somewhere safe. If you lose it, nobody can get it back for you.");
                ui.checkbox(&mut r.saved, "I saved my private key");
            }
            None if WIRED => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Getting your keys…");
                });
            }
            None => {
                key_row(ui, "PRIVATE KEY", C_WARN, about, "This build cannot show the private key yet. It will appear here, once, to copy.", None, |_| {});
            }
        }
        if is_name {
            ui.add_space(6.0);
            soft(ui, "These keys are also why FLit needs no account and no server. Your computer talks straight to your friends' computers.");
        }
        ui.add_space(10.0);
        let ready = match &r.keys {
            Some(_) => r.saved,
            None => !WIRED,
        };
        if ui.add_enabled(ready, egui::Button::new(egui::RichText::new("Continue").color(egui::Color32::WHITE).strong()).fill(C_ACCENT).min_size(egui::vec2(140.0, 36.0))).clicked() {
            go = true;
        }
    });

    // two doors over the card, sliding apart from the middle
    if t < 1.0 {
        let rect = out.response.rect.expand(2.0);
        let half = rect.height() / 2.0 * open;
        let mid = rect.center().y;
        let paint = ui.painter();
        paint.rect_filled(egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), mid - half)), 0.0, C_BG);
        paint.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), mid + half), rect.max), 0.0, C_BG);
        let seam = egui::Stroke::new(2.0_f32, C_ACCENT_LIGHT);
        paint.hline(rect.x_range(), mid - half, seam);
        paint.hline(rect.x_range(), mid + half, seam);
    }
    go
}

// ----- the walk -----

/// Draws the tutorial. True when it is finished or skipped.
pub(super) fn show(ui: &mut egui::Ui, st: &mut State, s: &Status, shared: &Shared, logo: Option<&egui::TextureHandle>) -> bool {
    let named = st.named(s);
    let all_beings = st.all_beings(s);
    let set_up = named.is_some() && !all_beings.is_empty();
    let step = STEPS[st.step.min(STEPS.len() - 1)];
    let last = st.step + 1 >= STEPS.len();
    let now = ui.input(|i| i.time);
    let mut done = false;
    let mut next = false;

    let full = ui.available_width();
    let width = full.min(680.0);
    ui.horizontal_top(|ui| {
        ui.add_space(((full - width) / 2.0 - 10.0).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.add_space(8.0);

            // where we are, and the way out
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{} of {}", st.step + 1, STEPS.len())).size(13.0).color(C_MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if set_up && !last && st.reveal.is_none() && !st.confirm_skip && ui.button("Skip").clicked() {
                        st.confirm_skip = true;
                    }
                });
            });
            if st.confirm_skip {
                ui.add_space(6.0);
                panel(ui, C_WARN, |ui| {
                    ui.label(egui::RichText::new("Are you sure?").size(17.0).strong());
                    soft(ui, "You can open this again any time from Help.");
                    ui.horizontal(|ui| {
                        if ui.button("Yes, skip").clicked() {
                            done = true;
                        }
                        if primary(ui, "Keep going").clicked() {
                            st.confirm_skip = false;
                        }
                    });
                });
            }
            ui.add_space(26.0);

            if let Some(r) = st.reveal.as_mut() {
                if reveal_screen(ui, r, s, shared) {
                    st.reveal = None;
                    st.adding_being = false;
                    next = true;
                }
                return;
            }

            // false on the screens that bring their own way forward
            let mut nav = true;
            match step {
                Step::Welcome => {
                    if let Some(t) = logo {
                        ui.image((t.id(), egui::vec2(84.0, 84.0)));
                        ui.add_space(10.0);
                    }
                    title(ui, "Welcome to FLit");
                    p(ui, "FLit lets people, AI and scripts work on the same song together, each in their own DAW.");
                    p(ui, "Everything you do is saved as you go. You can go back to any moment, try ideas without losing your work, and stop sending project files around.");
                    if set_up {
                        soft(ui, "This takes about three minutes.");
                    } else {
                        soft(ui, "This takes about three minutes. First we make your Name.");
                    }
                }
                Step::Name => {
                    title(ui, "Your Name");
                    p(ui, "The first thing you make is a Name. Think of it as your studio's title. It holds your songs, your friends, and the history of everything made under it.");
                    match &named {
                        Some(n) => {
                            panel(ui, C_BORDER, |ui| {
                                ui.label(egui::RichText::new("YOUR NAME").size(12.0).color(C_MUTED));
                                ui.label(egui::RichText::new(n).size(28.0).strong().color(C_ACCENT_LIGHT));
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&s.short_code).monospace().size(15.0));
                                    if ui.button("Copy public key").clicked() {
                                        ui.ctx().copy_text(s.id_text.clone());
                                    }
                                });
                            });
                            ui.add_space(14.0);
                            p(ui, "Every Name is a pair of keys. The public key is safe to share, and it is how friends find you. The private key proves you are you, so never share it.");
                            soft(ui, "These keys are why FLit needs no account and no server. Your computer talks straight to your friends' computers.");
                        }
                        None => {
                            nav = false;
                            field(ui, "Name", &mut st.name_input, "Your name, or your studio's name");
                            ui.add_space(12.0);
                            let name = st.name_input.trim().to_string();
                            let ok = !name.is_empty() && name.chars().count() <= 64;
                            if ui.add_enabled(ok, egui::Button::new(egui::RichText::new("Create my Name").color(egui::Color32::WHITE).strong()).fill(C_ACCENT).min_size(egui::vec2(180.0, 38.0))).clicked() {
                                make_name(shared, &name);
                                st.made_name = Some(name.clone());
                                if st.being_input.is_empty() {
                                    st.being_input = name.clone();
                                }
                                st.reveal = Some(Reveal { being: None, label: name, keys: None, at: now, unmasked: false, saved: false, copied_at: None });
                            }
                            if st.step > 0 {
                                ui.add_space(8.0);
                                if ui.button("Back").clicked() {
                                    st.step -= 1;
                                }
                            }
                        }
                    }
                }
                Step::Being => {
                    title(ui, "Beings");
                    p(ui, "A Name needs someone to do the work. That is a Being.");
                    p(ui, "A Being can be a human, an AI, or a script. A Name can have one Being or many.");
                    let first = all_beings.is_empty();
                    if !first {
                        panel(ui, C_BORDER, |ui| {
                            ui.label(egui::RichText::new("YOUR BEINGS").size(12.0).color(C_MUTED));
                            for b in &all_beings {
                                pill(ui, kind_color(b.kind), &format!("{}, {}", b.name, kind_word(b.kind)));
                            }
                        });
                        ui.add_space(12.0);
                        if !st.adding_being && ui.button("Add another Being").clicked() {
                            st.adding_being = true;
                            st.being_input.clear();
                        }
                    }
                    if first || st.adding_being {
                        nav = !first;
                        if first {
                            p(ui, "Let's make your first one.");
                            if st.being_input.is_empty() {
                                if let Some(n) = &named {
                                    st.being_input = n.clone();
                                }
                            }
                        }
                        field(ui, "Name", &mut st.being_input, "What to call this Being");
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Type").size(13.0).color(C_MUTED));
                        kind_picker(ui, &mut st.being_kind);
                        let name = st.being_input.trim().to_string();
                        let ok = !name.is_empty() && name.chars().count() <= 64 && !all_beings.iter().any(|b| b.name == name);
                        if ui.add_enabled(ok, egui::Button::new(egui::RichText::new("Create Being").color(egui::Color32::WHITE).strong()).fill(C_ACCENT).min_size(egui::vec2(180.0, 38.0))).clicked() {
                            make_being(shared, &name, st.being_kind);
                            st.made_beings.push(Being { name: name.clone(), kind: st.being_kind });
                            st.reveal = Some(Reveal { being: Some(st.being_kind), label: name, keys: None, at: now, unmasked: false, saved: false, copied_at: None });
                            st.being_input.clear();
                        }
                        if first {
                            ui.add_space(8.0);
                            if ui.button("Back").clicked() {
                                st.step -= 1;
                            }
                        }
                    }
                }
                Step::Moments => {
                    title(ui, "Moments");
                    p(ui, "Every time something changes, FLit makes a new Moment.");
                    bullets(ui, &["Record a vocal.", "Turn a volume knob.", "Move a note.", "Rename a track.", "Change the tempo."]);
                    p(ui, "It is like a timeline, but it does not keep rolling forward with the clock. A new Moment only happens when something changes.");
                    soft(ui, "Every Moment made by one of your Beings becomes part of your Name's history.");
                }
                Step::MomentBar => {
                    title(ui, "The Moment Bar");
                    p(ui, "At the bottom of FLit is the Moment Bar. It shows every change made by everyone on the song, and everyone's copy stays up to date on its own.");
                    tape(ui);
                    bullets(ui, &["Click a Moment to hear the song as it was then.", "Go back if you liked it better before.", "Make a branch to try something on the side.", "Merge the branch into the main song when it works."]);
                    soft(ui, "Nothing is ever lost.");
                }
                Step::ActsFacts => {
                    title(ui, "Acts and Facts");
                    p(ui, "Every Moment can be read two ways.");
                    ui.columns(2, |cols| {
                        reading(&mut cols[0], "Acts", "Who did what?", "Alice", &["Recorded vocals", "Changed the EQ", "Muted the guitar"]);
                        reading(&mut cols[1], "Facts", "What happened to this?", "Vocal track", &["Recorded", "Trimmed", "EQ changed"]);
                    });
                    ui.add_space(16.0);
                    p(ui, "Same Moments, seen from two sides. Look at Acts to follow a Name or a Being. Look at Facts to follow a track, a plugin, or the tempo.");
                }
                Step::Machines => {
                    title(ui, "AI and Script Beings");
                    p(ui, "An AI Being works just like a human one. It has its own identity, it makes its own Moments, and everything it does can be seen and undone.");
                    p(ui, "AI Beings can work your DAW and your plugins. They can even build new plugins that your whole crew can share.");
                    p(ui, "Script Beings take care of the repeat jobs.");
                    bullets(ui, &["Rename files.", "Render stems.", "Sort samples."]);
                }
                Step::Trust => {
                    title(ui, "Trust comes first");
                    panel(ui, C_WARN, |ui| {
                        ui.label(egui::RichText::new("Off until you turn it on").size(18.0).strong().color(C_WARN));
                        ui.add_space(6.0);
                        p(ui, "Controlling a DAW and building plugins means running code on your computer. A bad script or a careless AI could change files, delete work, or do almost anything else.");
                        p(ui, "So these powers are off for every Name until you turn them on.");
                        ui.label(egui::RichText::new("Only turn them on for Names you trust completely.").size(16.5).strong());
                    });
                }
                Step::Songs => {
                    title(ui, "Songs and who gets in");
                    p(ui, "A Song holds its crew, its history, its files and its chat.");
                    p(ui, "You invite Names to a Song, then choose which of their Beings are allowed in.");
                    panel(ui, C_BORDER, |ui| {
                        ui.label(egui::RichText::new("Studio A").size(17.0).strong());
                        ui.add_space(4.0);
                        for (who, allowed) in [("Sarah, Human", true), ("Vocal cleaner, AI", true), ("Plugin builder, AI", false), ("Stem render, Script", false)] {
                            ui.horizontal(|ui| {
                                pill(ui, if allowed { C_OK } else { C_MUTED }, who);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(egui::RichText::new(if allowed { "allowed" } else { "blocked" }).size(13.0).color(if allowed { C_OK } else { C_MUTED }));
                                });
                            });
                        }
                    });
                    ui.add_space(14.0);
                    p(ui, "Want humans only? Block the AI Beings. Either way you can always see what a human did and what an AI did, and undo what you need to.");
                }
                Step::Ready => {
                    nav = false;
                    title(ui, "That is the heart of it");
                    p(ui, "You know enough to start.");
                    p(ui, "Or take two more minutes to see how crews like to work together.");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(egui::RichText::new("Start using FLit").color(egui::Color32::WHITE).strong()).fill(C_ACCENT).min_size(egui::vec2(170.0, 38.0))).clicked() {
                            done = true;
                        }
                        if ui.add(egui::Button::new("Show me how crews work").min_size(egui::vec2(200.0, 38.0))).clicked() {
                            next = true;
                        }
                    });
                    ui.add_space(8.0);
                    if ui.button("Back").clicked() {
                        st.step -= 1;
                    }
                }
                Step::Together => {
                    title(ui, "Together on Main");
                    p(ui, "Everyone joins the main song at once. It is like a band around one mixing desk.");
                    p(ui, "Press play and everyone hears it. Someone says the bass is too loud, someone turns it down, and everyone hears the change. Often one person leads, like a producer.");
                    p(ui, "This is great for listening and deciding together.");
                    soft(ui, "It is less great when five people want to do five different things, because every play and every stop is shared.");
                }
                Step::Rooms => {
                    title(ui, "Each in your own room");
                    p(ui, "Everyone starts a branch from the same Moment and works without interrupting anyone.");
                    rooms(ui);
                    p(ui, "Then the crew listens to each branch, picks the best parts, and merges them into Main. You can step into someone's branch any time to listen or to help.");
                    soft(ui, "Branches are not for fixing mistakes. They are rooms in a studio.");
                }
                Step::Bridges => {
                    title(ui, "One more thing");
                    p(ui, "FLit does not belong to any one DAW. Each DAW connects through a Bridge.");
                    p(ui, "So far there is one Bridge, for FL Studio, and we are still building it. Ableton, Logic, Reaper, Bitwig and others can join the same way.");
                    p(ui, "If you write code, we would love your help. Every new Bridge brings another music community into the same songs.");
                }
            }

            if nav {
                ui.add_space(22.0);
                ui.horizontal(|ui| {
                    if st.step > 0 && ui.add(egui::Button::new("Back").min_size(egui::vec2(90.0, 36.0))).clicked() {
                        st.step -= 1;
                        st.adding_being = false;
                    }
                    let word = if last { "Start using FLit" } else if step == Step::Welcome { "Let's go" } else { "Continue" };
                    if ui.add(egui::Button::new(egui::RichText::new(word).color(egui::Color32::WHITE).strong()).fill(C_ACCENT).min_size(egui::vec2(140.0, 36.0))).clicked() {
                        if last {
                            done = true;
                        } else {
                            next = true;
                        }
                    }
                });
            }
            ui.add_space(24.0);
        });
    });

    if next {
        st.step = (st.step + 1).min(STEPS.len() - 1);
        st.confirm_skip = false;
    }
    if done {
        st.onboarding = false;
        st.open();
    }
    done
}
