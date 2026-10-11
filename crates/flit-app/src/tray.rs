//! The tray icon (PACK 2 §8.3): FLit keeps running when its window is
//! closed; the tray is how it comes back. Built on the main thread inside
//! eframe's creator (the tray's own hidden window is pumped by winit's
//! loop). Events arrive on that thread too, so they may show the window
//! natively and ask for a frame.

use eframe::egui;
use flit_ui::Shared;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct Tray {
    _icon: tray_icon::TrayIcon,
}

const OPEN: &str = "open";
const QUIT: &str = "quit";

pub fn install(ctx: egui::Context, shared: Arc<Shared>, quit: Arc<AtomicBool>) -> Option<Tray> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let icon = image::load_from_memory(include_bytes!("../../flit-ui/assets/flit_logo_48.png")).ok().and_then(|img| {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        tray_icon::Icon::from_rgba(rgba.into_raw(), w, h).ok()
    })?;
    let menu = Menu::new();
    let open = MenuItem::with_id(OPEN, "Open FLit", true, None);
    let quit_item = MenuItem::with_id(QUIT, "Quit FLit", true, None);
    menu.append_items(&[&open, &PredefinedMenuItem::separator(), &quit_item]).ok()?;
    let show = {
        let ctx = ctx.clone();
        let shared = shared.clone();
        move || {
            shared.show_window.store(true, Ordering::SeqCst);
            crate::native::show();
            ctx.request_repaint();
        }
    };
    let show2 = show.clone();
    TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
            show();
        }
    }));
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        if ev.id().0 == OPEN {
            show2();
        } else if ev.id().0 == QUIT {
            quit.store(true, Ordering::SeqCst);
            crate::native::show();
            ctx.request_repaint();
        }
    }));
    let icon = TrayIconBuilder::new().with_tooltip("FLit").with_icon(icon).with_menu(Box::new(menu)).with_menu_on_left_click(false).build().ok()?;
    Some(Tray { _icon: icon })
}
