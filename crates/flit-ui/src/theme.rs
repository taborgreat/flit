//! Design tokens (FLIT_SPEC §12). Every widget reads these; no toolkit
//! default survives. Type: IBM Plex Sans (UI) and IBM Plex Mono (values,
//! moment numbers, ids), sizes 11 / 12 / 14 / 18, regular + medium.
//! Shape: 3 px controls, 4 px panels, 1 px borders, no shadows, no
//! gradients, no pills. Spacing on a 4 px grid: gap 8, section 16, pad 12.

use std::sync::Arc;

pub const BG: egui::Color32 = egui::Color32::from_rgb(0x0E, 0x0F, 0x11);
pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(0x16, 0x18, 0x1B);
pub const RAISED: egui::Color32 = egui::Color32::from_rgb(0x1E, 0x21, 0x25);
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0x2A, 0x2E, 0x33);
pub const BORDER_HI: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3F, 0x46);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xE6, 0xE8, 0xEB);
pub const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8A, 0x90, 0x99);
pub const FAINT: egui::Color32 = egui::Color32::from_rgb(0x5A, 0x60, 0x6A);
/// The one accent.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xA3, 0x3D);
pub const OK: egui::Color32 = egui::Color32::from_rgb(0x46, 0xA7, 0x58);
pub const WARN: egui::Color32 = egui::Color32::from_rgb(0xE5, 0xA1, 0x3D);
pub const BAD: egui::Color32 = egui::Color32::from_rgb(0xE5, 0x48, 0x4D);

/// Being colors, in order of assignment.
pub const BEING_COLORS: [u32; 8] = [0x5B8DEF, 0xE07A5F, 0x4FB286, 0xC77DDB, 0xE8C547, 0x4CC3D9, 0xF08FB0, 0x9AA5B1];

pub const RADIUS_CONTROL: u8 = 3;
pub const RADIUS_PANEL: u8 = 4;
pub const GAP: f32 = 8.0;
pub const SECTION: f32 = 16.0;
pub const PAD: f32 = 12.0;
pub const BUTTON_H: f32 = 24.0;
pub const PRIMARY_H: f32 = 28.0;
pub const TOP_BAR_H: f32 = 36.0;
pub const STATUS_STRIP_H: f32 = 22.0;
pub const TIMELINE_DEFAULT_H: f32 = 96.0;
pub const RAIL_W: f32 = 140.0;

pub const S11: f32 = 11.0;
pub const S12: f32 = 12.0;
pub const S14: f32 = 14.0;
pub const S18: f32 = 18.0;

pub fn color(rgb: u32) -> egui::Color32 {
    egui::Color32::from_rgb(((rgb >> 16) & 0xFF) as u8, ((rgb >> 8) & 0xFF) as u8, (rgb & 0xFF) as u8)
}

/// Font families: "sans" (Plex Sans), "sans-medium", "mono" (Plex Mono).
pub fn sans() -> egui::FontFamily {
    egui::FontFamily::Name("sans".into())
}
pub fn medium() -> egui::FontFamily {
    egui::FontFamily::Name("sans-medium".into())
}
pub fn mono() -> egui::FontFamily {
    egui::FontFamily::Name("mono".into())
}

pub fn text(size: f32) -> egui::FontId {
    egui::FontId::new(size, sans())
}
pub fn text_medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, medium())
}
pub fn code(size: f32) -> egui::FontId {
    egui::FontId::new(size, mono())
}

/// Install the Plex fonts and every visual token.
pub fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("plex-sans".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"))));
    fonts.font_data.insert("plex-sans-medium".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"))));
    fonts.font_data.insert("plex-mono".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"))));
    fonts.font_data.insert("plex-mono-medium".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"))));
    // the egui defaults stay as fallbacks for glyphs Plex lacks (emoji are not used)
    fonts.families.insert(sans(), vec!["plex-sans".into(), "Ubuntu-Light".into(), "NotoEmoji-Regular".into()]);
    fonts.families.insert(medium(), vec!["plex-sans-medium".into(), "Ubuntu-Light".into()]);
    fonts.families.insert(mono(), vec!["plex-mono".into(), "Hack".into()]);
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        list.insert(0, "plex-sans".into());
    }
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        list.insert(0, "plex-mono".into());
    }
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = BG;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = SURFACE;
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.window_stroke = egui::Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = egui::CornerRadius::same(RADIUS_PANEL);
    v.menu_corner_radius = egui::CornerRadius::same(RADIUS_PANEL);
    v.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(0xE8, 0xA3, 0x3D, 0x40);
    v.selection.stroke = egui::Stroke::new(1.0_f32, ACCENT);
    let r = egui::CornerRadius::same(RADIUS_CONTROL);
    v.widgets.noninteractive.bg_fill = SURFACE;
    v.widgets.noninteractive.weak_bg_fill = SURFACE;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = RAISED;
    v.widgets.inactive.weak_bg_fill = SURFACE;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = RAISED;
    v.widgets.hovered.weak_bg_fill = RAISED;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, BORDER_HI);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.corner_radius = r;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = RAISED;
    v.widgets.active.weak_bg_fill = RAISED;
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    v.widgets.active.corner_radius = r;
    v.widgets.active.expansion = 0.0;
    v.widgets.open.bg_fill = RAISED;
    v.widgets.open.weak_bg_fill = RAISED;
    v.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, BORDER_HI);
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    v.widgets.open.corner_radius = r;
    v.text_cursor.stroke = egui::Stroke::new(1.0_f32, ACCENT);
    v.striped = false;
    v.indent_has_left_vline = false;
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(GAP, GAP);
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.spacing.window_margin = egui::Margin::same(PAD as i8);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.indent = 16.0;
    style.spacing.interact_size = egui::vec2(40.0, BUTTON_H);
    style.spacing.slider_width = 160.0;
    style.spacing.combo_width = 160.0;
    style.spacing.text_edit_width = 240.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();
    style.animation_time = 0.1;
    style.text_styles.insert(egui::TextStyle::Body, text(S12));
    style.text_styles.insert(egui::TextStyle::Button, text(S12));
    style.text_styles.insert(egui::TextStyle::Small, text(S11));
    style.text_styles.insert(egui::TextStyle::Heading, text_medium(S18));
    style.text_styles.insert(egui::TextStyle::Monospace, code(S12));
    style.visuals.button_frame = true;
    ctx.set_style(style);
}

/// A section label: 11 px, uppercase, letter-spaced, muted.
pub fn section_label(ui: &mut egui::Ui, text_: &str) {
    let spaced: String = text_.to_uppercase().chars().flat_map(|c| [c, '\u{200A}']).collect();
    ui.add_space(SECTION - GAP);
    ui.label(egui::RichText::new(spaced).font(text_medium(S11)).color(MUTED));
}

/// A card: surface fill, 1 px border, 4 px radius, 12 px padding.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new().fill(SURFACE).stroke(egui::Stroke::new(1.0_f32, BORDER)).corner_radius(egui::CornerRadius::same(RADIUS_PANEL)).inner_margin(egui::Margin::same(PAD as i8)).show(ui, |ui| add(ui)).inner
}

/// A raised region (a row that can be hovered, a dialog body).
pub fn raised<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new().fill(RAISED).stroke(egui::Stroke::new(1.0_f32, BORDER)).corner_radius(egui::CornerRadius::same(RADIUS_PANEL)).inner_margin(egui::Margin::same(PAD as i8)).show(ui, |ui| add(ui)).inner
}

/// The primary button: 28 px, accent fill, dark text.
pub fn primary(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(label).font(text_medium(S12)).color(BG)).fill(ACCENT).stroke(egui::Stroke::NONE).corner_radius(egui::CornerRadius::same(RADIUS_CONTROL)).min_size(egui::vec2(0.0, PRIMARY_H));
    ui.add(b)
}

/// An ordinary button: 24 px, raised fill, 1 px border.
pub fn button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(label).font(text(S12))).fill(RAISED).stroke(egui::Stroke::new(1.0_f32, BORDER)).corner_radius(egui::CornerRadius::same(RADIUS_CONTROL)).min_size(egui::vec2(0.0, BUTTON_H));
    ui.add(b)
}

/// A quiet button: no fill until hovered.
pub fn quiet(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(label).font(text(S12)).color(MUTED)).fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE).corner_radius(egui::CornerRadius::same(RADIUS_CONTROL)).min_size(egui::vec2(0.0, BUTTON_H));
    ui.add(b)
}

/// A danger button (status red border).
pub fn danger(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(label).font(text(S12)).color(BAD)).fill(RAISED).stroke(egui::Stroke::new(1.0_f32, BAD)).corner_radius(egui::CornerRadius::same(RADIUS_CONTROL)).min_size(egui::vec2(0.0, BUTTON_H));
    ui.add(b)
}

/// A segmented control: one of `items` is selected. Returns the new index.
pub fn segmented(ui: &mut egui::Ui, selected: usize, items: &[&str]) -> usize {
    let mut out = selected;
    egui::Frame::new().fill(BG).stroke(egui::Stroke::new(1.0_f32, BORDER)).corner_radius(egui::CornerRadius::same(RADIUS_CONTROL)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
        ui.horizontal(|ui| {
            for (i, item) in items.iter().enumerate() {
                let on = i == selected;
                let b = egui::Button::new(egui::RichText::new(*item).font(if on { text_medium(S12) } else { text(S12) }).color(if on { TEXT } else { MUTED }))
                    .fill(if on { RAISED } else { egui::Color32::TRANSPARENT })
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(egui::CornerRadius::same(RADIUS_CONTROL))
                    .min_size(egui::vec2(0.0, BUTTON_H - 4.0));
                if ui.add(b).clicked() {
                    out = i;
                }
            }
        });
    });
    out
}

/// A text field with a label above it.
pub fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) -> egui::Response {
    ui.label(egui::RichText::new(label).font(text(S11)).color(MUTED));
    ui.add(egui::TextEdit::singleline(value).hint_text(egui::RichText::new(hint).color(FAINT)).desired_width(f32::INFINITY).font(text(S12)))
}

/// A small status dot (status colors only).
pub fn dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// A being avatar: a filled circle in the being's color with its initial.
/// Hollow when the being is in another history or offline.
pub fn avatar(ui: &mut egui::Ui, rgb: u32, initial: char, solid: bool, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let c = color(rgb);
    let p = ui.painter();
    if solid {
        p.circle_filled(rect.center(), size / 2.0, c);
        p.text(rect.center(), egui::Align2::CENTER_CENTER, initial.to_uppercase().to_string(), text_medium(size * 0.55), BG);
    } else {
        p.circle_stroke(rect.center(), size / 2.0 - 0.5, egui::Stroke::new(1.0_f32, c));
        p.text(rect.center(), egui::Align2::CENTER_CENTER, initial.to_uppercase().to_string(), text_medium(size * 0.55), c);
    }
    resp
}

/// A numeric value: mono, tabular, right-aligned in `width`.
pub fn value(ui: &mut egui::Ui, text_: &str, width: f32) {
    ui.add_sized(egui::vec2(width, BUTTON_H - 4.0), egui::Label::new(egui::RichText::new(text_).font(code(S12))).halign(egui::Align::RIGHT));
}

pub fn muted(ui: &mut egui::Ui, text_: &str) {
    ui.label(egui::RichText::new(text_).font(text(S12)).color(MUTED));
}

pub fn small(ui: &mut egui::Ui, text_: &str) {
    ui.label(egui::RichText::new(text_).font(text(S11)).color(MUTED));
}

pub fn heading(ui: &mut egui::Ui, text_: &str) {
    ui.label(egui::RichText::new(text_).font(text_medium(S18)).color(TEXT));
}

pub fn strong(ui: &mut egui::Ui, text_: &str) {
    ui.label(egui::RichText::new(text_).font(text_medium(S12)).color(TEXT));
}

/// The hidden style gallery: every widget in every state, so the look
/// can be checked at a glance (Settings > "Style gallery").
pub fn gallery(ui: &mut egui::Ui) {
    heading(ui, "Style gallery");
    small(ui, "Every widget in every state. Tokens live in theme.rs.");
    section_label(ui, "Type");
    ui.label(egui::RichText::new("Plex Sans 18 medium").font(text_medium(S18)));
    ui.label(egui::RichText::new("Plex Sans 14 regular").font(text(S14)));
    ui.label(egui::RichText::new("Plex Sans 12 regular, the body text").font(text(S12)));
    ui.label(egui::RichText::new("Plex Sans 11 muted").font(text(S11)).color(MUTED));
    ui.label(egui::RichText::new("Plex Mono 12 #212 k3jd-9f2a 0.750").font(code(S12)));
    section_label(ui, "Buttons");
    ui.horizontal(|ui| {
        primary(ui, "Primary");
        button(ui, "Button");
        quiet(ui, "Quiet");
        danger(ui, "Danger");
        ui.add_enabled(false, egui::Button::new("Disabled"));
    });
    section_label(ui, "Segmented");
    segmented(ui, 0, &["Moments", "Time"]);
    section_label(ui, "Fields");
    let mut s = String::from("text");
    field(ui, "Label", &mut s, "hint");
    section_label(ui, "Status");
    ui.horizontal(|ui| {
        dot(ui, OK);
        muted(ui, "ok");
        dot(ui, WARN);
        muted(ui, "warn");
        dot(ui, BAD);
        muted(ui, "bad");
        dot(ui, ACCENT);
        muted(ui, "accent");
    });
    section_label(ui, "Members");
    ui.horizontal(|ui| {
        for (i, c) in BEING_COLORS.iter().enumerate() {
            avatar(ui, *c, (b'A' + i as u8) as char, i % 2 == 0, 20.0);
        }
    });
    section_label(ui, "Surfaces");
    card(ui, |ui| {
        muted(ui, "card: surface, 1 px border, 4 px radius, 12 px padding");
        raised(ui, |ui| muted(ui, "raised"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel must draw without a host: fonts resolve, every widget lays out.
    #[test]
    fn theme_and_gallery_draw_headless() {
        let ctx = egui::Context::default();
        apply(&ctx);
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 760.0))), ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| gallery(ui));
                });
            });
        }
    }
}
