//! The tutorial: first run and the Help button.
//!
//! First run it fills the panel and cannot be skipped until a Name and a
//! first Being exist. From Help it is the same walk with the making steps
//! already done. One idea per screen, plain words.
//!
//! Users read "Studio" for Name and "member" for Being (FLIT_SPEC.md
//! section 0); the code keeps the code names.
//!
//! Making the Studio opens a card with its public key and its private key
//! (the recovery code) to copy, once. Making a member shows no keys: a
//! Studio can get them later from the Studio screen.

use crate::theme as t;
use crate::client::Shared;
use flit_core::{BeingId, BeingKind};
use flit_ipc::*;

// ----- the seam to the helper -----

fn has_name(s: &Status) -> bool {
    !s.name_label.trim().is_empty()
}

fn make_name(shared: &Shared, name: &str) {
    shared.act(Request::SetNameLabel { label: name.to_string() });
    // the private key, shown once on the card that opens next
    *shared.text.lock() = None;
    shared.act(Request::RecoveryCode);
}

#[derive(Clone)]
struct Being {
    name: String,
    kind: BeingKind,
}

/// The Beings of this Name.
fn beings(s: &Status) -> Vec<Being> {
    s.my_beings.iter().map(|(_, name, kind)| Being { name: name.clone(), kind: *kind }).collect()
}

fn being_id(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "me".into() } else { out }
}

fn make_being(shared: &Shared, name: &str, kind: BeingKind) {
    shared.act(Request::NewBeing { id: BeingId::new(&being_id(name)), kind, display_name: name.to_string(), color: None });
    shared.act(Request::Beings);
}

// ----- state -----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Name,
    Being,
    Moments,
    Timeline,
    ActsFacts,
    Machines,
    Trust,
    Songs,
    Ready,
    Together,
    Rooms,
    Bridges,
}

const STEPS: [Step; 13] = [Step::Welcome, Step::Name, Step::Being, Step::Moments, Step::Timeline, Step::ActsFacts, Step::Machines, Step::Trust, Step::Songs, Step::Ready, Step::Together, Step::Rooms, Step::Bridges];

/// The card that opens right after the Name is made.
struct Reveal {
    label: String,
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
    /// Beings made in this sitting (Status catches up a moment later).
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
            self.onboarding = !has_name(s) || beings(s).is_empty();
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
            Some(s.name_label.trim().to_string())
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
    ui.label(egui::RichText::new(text).font(t::text_medium(24.0)).color(t::TEXT));
    ui.add_space(t::PAD);
}

fn p(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(t::text(t::S14)).color(t::TEXT));
    ui.add_space(t::GAP);
}

fn soft(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(t::text(t::S12)).color(t::MUTED));
    ui.add_space(t::GAP);
}

fn bullets(ui: &mut egui::Ui, lines: &[&str]) {
    for line in lines {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            t::dot(ui, t::ACCENT);
            ui.label(egui::RichText::new(*line).font(t::text(t::S14)).color(t::TEXT));
        });
    }
    ui.add_space(t::PAD);
}

fn panel<R>(ui: &mut egui::Ui, stroke: egui::Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new().fill(t::SURFACE).stroke(egui::Stroke::new(1.0_f32, stroke)).corner_radius(egui::CornerRadius::same(t::RADIUS_PANEL)).inner_margin(egui::Margin::same(20)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    })
}

fn big_button(ui: &mut egui::Ui, enabled: bool, text: &str, width: f32) -> bool {
    ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(text).font(t::text_medium(t::S14)).color(t::BG)).fill(t::ACCENT).corner_radius(egui::CornerRadius::same(t::RADIUS_CONTROL)).min_size(egui::vec2(width, 32.0))).clicked()
}

fn plain_button(ui: &mut egui::Ui, text: &str, width: f32) -> bool {
    ui.add(egui::Button::new(egui::RichText::new(text).font(t::text(t::S14))).fill(t::RAISED).stroke(egui::Stroke::new(1.0_f32, t::BORDER)).corner_radius(egui::CornerRadius::same(t::RADIUS_CONTROL)).min_size(egui::vec2(width, 32.0))).clicked()
}

fn kind_word(k: BeingKind) -> &'static str {
    match k {
        BeingKind::Human => "Human",
        BeingKind::Ai => "AI",
        BeingKind::Scripted => "Scripted",
    }
}

fn kind_color(k: BeingKind) -> egui::Color32 {
    match k {
        BeingKind::Human => t::color(t::BEING_COLORS[0]),
        BeingKind::Ai => t::color(t::BEING_COLORS[3]),
        BeingKind::Scripted => t::color(t::BEING_COLORS[4]),
    }
}

/// Three cards to pick from, one line of meaning each.
fn kind_picker(ui: &mut egui::Ui, kind: &mut BeingKind) {
    let about = [(BeingKind::Human, "A person who plays, records and mixes."), (BeingKind::Ai, "An AI partner that can mix, clean up or try ideas."), (BeingKind::Scripted, "A program that does the repeat jobs for you.")];
    ui.columns(3, |cols| {
        for (i, (col, (k, text))) in cols.iter_mut().zip(about).enumerate() {
            let selected = *kind == k;
            let out = egui::Frame::new().fill(if selected { t::RAISED } else { t::BG }).stroke(egui::Stroke::new(1.0_f32, if selected { t::ACCENT } else { t::BORDER })).corner_radius(egui::CornerRadius::same(t::RADIUS_PANEL)).inner_margin(egui::Margin::same(12)).show(col, |ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(72.0);
                ui.label(egui::RichText::new(kind_word(k)).font(t::text_medium(t::S14)).color(if selected { t::TEXT } else { t::MUTED }));
                ui.label(egui::RichText::new(text).font(t::text(t::S12)).color(t::MUTED));
            });
            if col.interact(out.response.rect, col.id().with(("kind", i)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *kind = k;
            }
        }
    });
    ui.add_space(t::PAD);
}

/// A strip of moments, the way the Timeline shows them.
fn tape(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 52.0), egui::Sense::hover());
    let paint = ui.painter();
    paint.rect_filled(rect, 3.0, t::BG);
    paint.rect_stroke(rect, 3.0, egui::Stroke::new(1.0_f32, t::BORDER), egui::StrokeKind::Inside);
    let gap = 12.0;
    let count = ((rect.width() - 40.0) / gap).floor().max(0.0) as usize;
    for i in 0..count {
        let x = rect.left() + 18.0 + i as f32 * gap;
        let tall = i % 11 == 7;
        let h = if tall { 30.0 } else { 15.0 };
        let mark = egui::Rect::from_min_max(egui::pos2(x - 1.5, rect.bottom() - 8.0 - h), egui::pos2(x + 1.5, rect.bottom() - 8.0));
        paint.rect_filled(mark, 0.0, t::color(t::BEING_COLORS[(i * 7 + i / 3) % 5]));
    }
    paint.vline(rect.right() - 12.0, rect.shrink(6.0).y_range(), egui::Stroke::new(1.0_f32, t::ACCENT));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        t::small(ui, "Each mark is one moment, in the color of who made it.");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| t::small(ui, "now"));
    });
    ui.add_space(t::PAD);
}

/// One history, read from one side.
fn reading(ui: &mut egui::Ui, name: &str, question: &str, subject: &str, lines: &[&str]) {
    panel(ui, t::BORDER, |ui| {
        ui.label(egui::RichText::new(name).font(t::text_medium(t::S18)).color(t::ACCENT));
        ui.label(egui::RichText::new(question).font(t::text(t::S12)).color(t::MUTED));
        ui.add_space(t::GAP);
        ui.label(egui::RichText::new(subject).font(t::text_medium(t::S14)));
        for line in lines {
            ui.label(egui::RichText::new(*line).font(t::text(t::S14)).color(t::TEXT));
        }
    });
}

/// Main splits into three rooms and comes back together.
fn rooms(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 190.0), egui::Sense::hover());
    let paint = ui.painter();
    paint.rect_filled(rect, 3.0, t::BG);
    let line = egui::Stroke::new(1.0_f32, t::BORDER_HI);
    let top = egui::pos2(rect.center().x, rect.top() + 26.0);
    let bottom = egui::pos2(rect.center().x, rect.bottom() - 26.0);
    let mid_y = rect.center().y;
    let rooms = [("Alice", "softer vocals", t::BEING_COLORS[0]), ("Bob", "heavier drums", t::BEING_COLORS[1]), ("Chris", "half time", t::BEING_COLORS[2])];
    let reach = (rect.width() / 2.0 - 90.0).max(60.0);
    for (i, (who, what, color)) in rooms.iter().enumerate() {
        let x = rect.center().x + (i as f32 - 1.0) * reach;
        let box_rect = egui::Rect::from_center_size(egui::pos2(x, mid_y), egui::vec2(132.0, 46.0));
        paint.line_segment([egui::pos2(top.x, top.y + 12.0), egui::pos2(x, box_rect.top())], line);
        paint.line_segment([egui::pos2(x, box_rect.bottom()), egui::pos2(bottom.x, bottom.y - 12.0)], line);
        paint.rect_filled(box_rect, 4.0, t::SURFACE);
        paint.rect_stroke(box_rect, 4.0, egui::Stroke::new(1.0_f32, t::color(*color)), egui::StrokeKind::Inside);
        paint.text(egui::pos2(x, mid_y - 9.0), egui::Align2::CENTER_CENTER, *who, t::text_medium(t::S14), t::TEXT);
        paint.text(egui::pos2(x, mid_y + 10.0), egui::Align2::CENTER_CENTER, *what, t::text(t::S11), t::MUTED);
    }
    paint.text(top, egui::Align2::CENTER_CENTER, "main", t::text_medium(t::S14), t::TEXT);
    paint.text(bottom, egui::Align2::CENTER_CENTER, "main, with the best of each", t::text_medium(t::S14), t::TEXT);
    ui.add_space(t::PAD);
}

/// Label, a line of meaning, the key in a box, and Copy.
fn key_row(ui: &mut egui::Ui, label: &str, color: egui::Color32, about: &str, shown: &str, copy: Option<&str>, extra: impl FnOnce(&mut egui::Ui)) -> bool {
    let mut copied = false;
    ui.label(egui::RichText::new(label).font(t::text_medium(t::S11)).color(color));
    ui.label(egui::RichText::new(about).font(t::text(t::S12)).color(t::TEXT));
    ui.add_space(4.0);
    egui::Frame::new().fill(t::BG).stroke(egui::Stroke::new(1.0_f32, t::BORDER)).corner_radius(egui::CornerRadius::same(t::RADIUS_CONTROL)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(egui::RichText::new(shown).font(t::code(t::S12))).wrap());
    });
    ui.horizontal(|ui| {
        if let Some(text) = copy {
            if t::button(ui, "Copy").clicked() {
                ui.ctx().copy_text(text.to_string());
                copied = true;
            }
        }
        extra(ui);
    });
    ui.add_space(t::PAD);
    copied
}

/// The opening. Returns true when the user moves on.
fn reveal_screen(ui: &mut egui::Ui, r: &mut Reveal, s: &Status, shared: &Shared) -> bool {
    let private = shared.text.lock().clone();
    let now = ui.input(|i| i.time);
    let tt = (((now - r.at) / 0.9) as f32).clamp(0.0, 1.0);
    let open = 1.0 - (1.0 - tt).powi(3);
    if tt < 1.0 {
        ui.ctx().request_repaint();
    }
    let mut go = false;

    let out = panel(ui, t::ACCENT, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new("YOUR STUDIO").font(t::text_medium(t::S11)).color(t::MUTED));
            ui.label(egui::RichText::new(&r.label).font(t::text_medium(32.0)).color(t::ACCENT));
        });
        ui.add_space(t::SECTION);
        p(ui, "It is yours now. Every Studio is a pair of keys, a lot like a crypto wallet.");
        key_row(ui, "PUBLIC KEY", t::OK, "Safe to share. It is how friends find you and invite you.", &s.short_code, Some(&s.id_text), |_| {});

        let about = "This proves you are you. Never share it. Anyone who has it can act as you.";
        match &private {
            Some(key) => {
                let shown = if r.unmasked { key.clone() } else { "\u{2022}".repeat(40) };
                let (mut unmasked, recently) = (r.unmasked, r.copied_at.map(|at| now - at < 2.0).unwrap_or(false));
                if key_row(ui, "PRIVATE KEY", t::WARN, about, &shown, Some(key), |ui| {
                    if t::button(ui, if unmasked { "Hide" } else { "Show" }).clicked() {
                        unmasked = !unmasked;
                    }
                    if recently {
                        ui.label(egui::RichText::new("Copied").font(t::text(t::S12)).color(t::OK));
                    }
                }) {
                    r.copied_at = Some(now);
                }
                r.unmasked = unmasked;
                soft(ui, "Copy it now and keep it somewhere safe. If you lose it, nobody can get it back for you.");
                ui.checkbox(&mut r.saved, "I saved my private key");
            }
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    t::muted(ui, "Getting your keys");
                });
            }
        }
        ui.add_space(4.0);
        soft(ui, "These keys are also why FLit needs no account and no server. Your computer talks straight to your friends' computers.");
        ui.add_space(t::GAP);
        if big_button(ui, private.is_some() && r.saved, "Continue", 140.0) {
            go = true;
        }
    });

    // two doors over the card, sliding apart from the middle
    if tt < 1.0 {
        let rect = out.response.rect.expand(2.0);
        let half = rect.height() / 2.0 * open;
        let mid = rect.center().y;
        let paint = ui.painter();
        paint.rect_filled(egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), mid - half)), 0.0, t::BG);
        paint.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), mid + half), rect.max), 0.0, t::BG);
        let seam = egui::Stroke::new(1.0_f32, t::ACCENT);
        paint.hline(rect.x_range(), mid - half, seam);
        paint.hline(rect.x_range(), mid + half, seam);
    }
    if go {
        // the key is not kept on screen after this
        *shared.text.lock() = None;
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
            ui.add_space(t::GAP);

            // where we are, and the way out
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{} of {}", st.step + 1, STEPS.len())).font(t::code(t::S11)).color(t::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if set_up && !last && st.reveal.is_none() && !st.confirm_skip && t::quiet(ui, "Skip").clicked() {
                        st.confirm_skip = true;
                    }
                });
            });
            if st.confirm_skip {
                ui.add_space(4.0);
                panel(ui, t::WARN, |ui| {
                    ui.label(egui::RichText::new("Are you sure?").font(t::text_medium(t::S14)));
                    soft(ui, "You can open this again any time from Help.");
                    ui.horizontal(|ui| {
                        if t::button(ui, "Yes, skip").clicked() {
                            done = true;
                        }
                        if t::primary(ui, "Keep going").clicked() {
                            st.confirm_skip = false;
                        }
                    });
                });
            }
            ui.add_space(24.0);

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
                    if let Some(tex) = logo {
                        ui.image((tex.id(), egui::vec2(84.0, 84.0)));
                        ui.add_space(t::GAP);
                    }
                    title(ui, "Welcome to FLit");
                    p(ui, "FLit lets people, AI and scripts work on the same song together, each in their own DAW.");
                    p(ui, "Everything you do is saved as you go. You can go back to any moment, try ideas without losing your work, and stop sending project files around.");
                    if set_up {
                        soft(ui, "This takes about three minutes.");
                    } else {
                        soft(ui, "This takes about three minutes. First we make your Studio.");
                    }
                }
                Step::Name => {
                    title(ui, "Your Studio");
                    p(ui, "The first thing you make is a Studio. It is you, or your studio, as one identity. It holds your songs, your friends, and a record of everything made under it.");
                    match &named {
                        Some(n) => {
                            panel(ui, t::BORDER, |ui| {
                                ui.label(egui::RichText::new("YOUR STUDIO").font(t::text_medium(t::S11)).color(t::MUTED));
                                ui.label(egui::RichText::new(n).font(t::text_medium(24.0)).color(t::ACCENT));
                                ui.add_space(t::GAP);
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&s.short_code).font(t::code(t::S12)));
                                    if t::button(ui, "Copy public key").clicked() {
                                        ui.ctx().copy_text(s.id_text.clone());
                                    }
                                });
                            });
                            ui.add_space(t::PAD);
                            p(ui, "Every Studio is a pair of keys. The public key is safe to share, and it is how friends find you. The private key proves you are you, so never share it.");
                            soft(ui, "These keys are why FLit needs no account and no server. Your computer talks straight to your friends' computers. Your private key is on the Studio screen under Back up this Studio.");
                        }
                        None => {
                            nav = false;
                            t::field(ui, "Studio", &mut st.name_input, "Your name, or your studio's name");
                            ui.add_space(t::PAD);
                            let name = st.name_input.trim().to_string();
                            let ok = !name.is_empty() && name.chars().count() <= 64;
                            if big_button(ui, ok, "Create my Studio", 180.0) {
                                make_name(shared, &name);
                                st.made_name = Some(name.clone());
                                if st.being_input.is_empty() {
                                    st.being_input = name.clone();
                                }
                                st.reveal = Some(Reveal { label: name, at: now, unmasked: false, saved: false, copied_at: None });
                            }
                            if st.step > 0 {
                                ui.add_space(t::GAP);
                                if t::quiet(ui, "Back").clicked() {
                                    st.step -= 1;
                                }
                            }
                        }
                    }
                }
                Step::Being => {
                    title(ui, "Members");
                    p(ui, "A Studio needs someone to do the work. That is a member.");
                    p(ui, "A member can be a human, an AI, or a script. A Studio can have one member or many.");
                    let first = all_beings.is_empty();
                    if !first {
                        panel(ui, t::BORDER, |ui| {
                            ui.label(egui::RichText::new("YOUR MEMBERS").font(t::text_medium(t::S11)).color(t::MUTED));
                            for b in &all_beings {
                                ui.horizontal(|ui| {
                                    t::dot(ui, kind_color(b.kind));
                                    ui.label(egui::RichText::new(format!("{}, {}", b.name, kind_word(b.kind))).font(t::text(t::S14)));
                                });
                            }
                        });
                        ui.add_space(t::PAD);
                        soft(ui, "Each member has its own place in every song. Your Studio manages them on the Studio screen.");
                        if !st.adding_being && t::button(ui, "Add another member").clicked() {
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
                        t::field(ui, "Name", &mut st.being_input, "What to call this member");
                        ui.add_space(t::GAP);
                        ui.label(egui::RichText::new("Type").font(t::text(t::S11)).color(t::MUTED));
                        kind_picker(ui, &mut st.being_kind);
                        let name = st.being_input.trim().to_string();
                        let ok = !name.is_empty() && name.chars().count() <= 64 && !all_beings.iter().any(|b| b.name == name);
                        if big_button(ui, ok, "Create member", 180.0) {
                            make_being(shared, &name, st.being_kind);
                            st.made_beings.push(Being { name, kind: st.being_kind });
                            st.being_input.clear();
                            st.adding_being = false;
                            // no key screen for a member: straight on
                            if first {
                                next = true;
                            }
                        }
                        if first {
                            ui.add_space(t::GAP);
                            if t::quiet(ui, "Back").clicked() {
                                st.step -= 1;
                            }
                        }
                    }
                }
                Step::Moments => {
                    title(ui, "Moments");
                    p(ui, "Every time something changes, FLit makes a new moment.");
                    bullets(ui, &["Record a vocal.", "Turn a volume knob.", "Move a note.", "Rename a track.", "Change the tempo."]);
                    p(ui, "It is like a timeline, but it does not keep rolling forward with the clock. A new moment only happens when something changes.");
                    soft(ui, "Every moment made by one of your members is kept under your Studio.");
                }
                Step::Timeline => {
                    title(ui, "The Timeline bar");
                    p(ui, "At the bottom of FLit is the Timeline bar. It shows every change made by everyone on the song, and everyone's copy stays up to date on its own.");
                    tape(ui);
                    bullets(ui, &["Click a moment to see the song as it was then.", "Go back if you liked it better before.", "Start a new history to try something on the side.", "Merge that history into main when it works."]);
                    soft(ui, "Nothing is ever lost.");
                }
                Step::ActsFacts => {
                    title(ui, "Two ways to read moments");
                    p(ui, "Every moment can be read two ways.");
                    ui.columns(2, |cols| {
                        reading(&mut cols[0], "By member", "Who did what?", "Alice", &["Recorded vocals", "Changed the EQ", "Muted the guitar"]);
                        reading(&mut cols[1], "By track", "What happened to this?", "Vocal track", &["Recorded", "Trimmed", "EQ changed"]);
                    });
                    ui.add_space(t::SECTION);
                    p(ui, "Same moments, seen from two sides. Pick a member in the Timeline to follow what they did. Pick a track, a plugin or the song settings to follow what happened to it.");
                }
                Step::Machines => {
                    title(ui, "AI and scripted members");
                    p(ui, "An AI member works just like a human one. It has its own identity, it makes its own moments, and everything it does can be seen and undone.");
                    p(ui, "AI members can work your DAW and your plugins. They can even build new plugins that your whole crew can share.");
                    p(ui, "Scripted members take care of the repeat jobs.");
                    bullets(ui, &["Rename files.", "Render stems.", "Sort samples."]);
                }
                Step::Trust => {
                    title(ui, "Trust comes first");
                    panel(ui, t::WARN, |ui| {
                        ui.label(egui::RichText::new("Off until you turn it on").font(t::text_medium(t::S18)).color(t::WARN));
                        ui.add_space(4.0);
                        p(ui, "Controlling a DAW and building plugins means running code on your computer. A bad script or a careless AI could change files, delete work, or do almost anything else.");
                        p(ui, "So an AI or scripted member works in its own history until you grant it tracks.");
                        ui.label(egui::RichText::new("Only grant members of Studios you trust completely.").font(t::text_medium(t::S14)));
                    });
                }
                Step::Songs => {
                    title(ui, "Songs and who gets in");
                    p(ui, "A song holds its crew, its histories, its files and its chat.");
                    p(ui, "You invite Studios to a song, then choose which of their members are allowed in.");
                    panel(ui, t::BORDER, |ui| {
                        ui.label(egui::RichText::new("Studio A").font(t::text_medium(t::S14)));
                        ui.add_space(4.0);
                        for (who, allowed) in [("Sarah, Human", true), ("Vocal cleaner, AI", true), ("Plugin builder, AI", false), ("Stem render, Scripted", false)] {
                            ui.horizontal(|ui| {
                                t::dot(ui, if allowed { t::OK } else { t::FAINT });
                                ui.label(egui::RichText::new(who).font(t::text(t::S14)));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(egui::RichText::new(if allowed { "allowed" } else { "blocked" }).font(t::text(t::S12)).color(if allowed { t::OK } else { t::MUTED }));
                                });
                            });
                        }
                    });
                    ui.add_space(t::PAD);
                    p(ui, "Want humans only? Block the AI members. Either way you can always see what a human did and what an AI did, and undo what you need to.");
                }
                Step::Ready => {
                    nav = false;
                    title(ui, "That is the heart of it");
                    p(ui, "You know enough to start.");
                    p(ui, "Or take two more minutes to see how crews like to work together.");
                    ui.add_space(t::GAP);
                    ui.horizontal(|ui| {
                        if big_button(ui, true, "Start using FLit", 170.0) {
                            done = true;
                        }
                        if plain_button(ui, "Show me how crews work", 200.0) {
                            next = true;
                        }
                    });
                    ui.add_space(t::GAP);
                    if t::quiet(ui, "Back").clicked() {
                        st.step -= 1;
                    }
                }
                Step::Together => {
                    title(ui, "Together on main");
                    p(ui, "Everyone joins the main history at once. It is like a band around one mixing desk.");
                    p(ui, "Press play and everyone hears it. Someone says the bass is too loud, someone turns it down, and everyone hears the change. Often one person leads, like a producer.");
                    p(ui, "This is great for listening and deciding together.");
                    soft(ui, "It is less great when five people want to do five different things, because every play and every stop is shared.");
                }
                Step::Rooms => {
                    title(ui, "Each in your own room");
                    p(ui, "Everyone starts a history from the same moment and works without interrupting anyone.");
                    rooms(ui);
                    p(ui, "Then the crew listens to each history, picks the best parts, and merges them into main. You can step into someone's history any time to listen or to help.");
                    soft(ui, "Histories are not for fixing mistakes. They are rooms in a studio.");
                }
                Step::Bridges => {
                    title(ui, "One more thing");
                    p(ui, "FLit does not belong to any one DAW. Each DAW connects through a Bridge.");
                    p(ui, "So far there is one Bridge, for FL Studio, and we are still building it. Ableton, Logic, Reaper, Bitwig and others can join the same way.");
                    p(ui, "If you write code, we would love your help. Every new Bridge brings another music community into the same songs.");
                }
            }

            if nav {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    if st.step > 0 && plain_button(ui, "Back", 90.0) {
                        st.step -= 1;
                        st.adding_being = false;
                    }
                    let word = if last { "Start using FLit" } else if step == Step::Welcome { "Let's go" } else { "Continue" };
                    if big_button(ui, true, word, 140.0) {
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
