//! The plugin's editor: nih-plug's egui window drawing flit-ui's compact
//! panel (PACK 2 §8.5). This file only answers the panel's questions about
//! the instance (mode, name); the full UI is the FLit app.

use crate::engine::Mode as ParamMode;
use crate::FlitParams;
use flit_ui::compact::CompactState;
use flit_ui::{egui, theme as t, Host, Mode, Shared, Surface};
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

impl From<ParamMode> for Mode {
    fn from(m: ParamMode) -> Mode {
        match m {
            ParamMode::Track => Mode::Track,
            ParamMode::Receiver => Mode::Receiver,
            ParamMode::TalkSend => Mode::TalkSend,
            ParamMode::TalkReceive => Mode::TalkReceive,
        }
    }
}

/// The plugin has no Master mode any more: that word maps to Track.
impl From<Mode> for ParamMode {
    fn from(m: Mode) -> ParamMode {
        match m {
            Mode::Master | Mode::Track => ParamMode::Track,
            Mode::Receiver => ParamMode::Receiver,
            Mode::TalkSend => ParamMode::TalkSend,
            Mode::TalkReceive => ParamMode::TalkReceive,
        }
    }
}

struct PluginHost<'a, 'b> {
    params: &'a FlitParams,
    setter: &'a ParamSetter<'b>,
}

impl Host for PluginHost<'_, '_> {
    fn surface(&self) -> Surface {
        Surface::Plugin
    }
    fn mode(&self) -> Mode {
        self.params.mode.value().into()
    }
    fn set_mode(&self, m: Mode) {
        self.setter.begin_set_parameter(&self.params.mode);
        self.setter.set_parameter(&self.params.mode, m.into());
        self.setter.end_set_parameter(&self.params.mode);
    }
    fn track(&self) -> String {
        self.params.track.read().clone()
    }
    fn set_track(&self, t: String) {
        *self.params.track.write() = t;
    }
    fn main_area(&self, ctx: &egui::Context, add: &mut dyn FnMut(&mut egui::Ui)) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t::BG)).show(ctx, |ui| add(ui));
    }
}

pub fn create(params: Arc<FlitParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let egui_state: Arc<EguiState> = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        CompactState::default(),
        |ctx, _| t::apply(ctx),
        move |ctx, setter, st| {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            let host = PluginHost { params: &params, setter };
            flit_ui::compact::draw(ctx, &host, st, &shared);
        },
    )
}

#[cfg(test)]
mod tests {
    //! The compact panel through the plugin's own host, headless, in every
    //! mode: a panic here would take the DAW down with it.
    use super::*;
    use std::sync::atomic::Ordering;

    struct NoHost;
    impl GuiContext for NoHost {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn request_resize(&self) -> bool {
            true
        }
        unsafe fn raw_begin_set_parameter(&self, _p: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, _p: ParamPtr, _n: f32) {}
        unsafe fn raw_end_set_parameter(&self, _p: ParamPtr) {}
        fn get_state(&self) -> nih_plug::prelude::PluginState {
            unimplemented!()
        }
        fn set_state(&self, _s: nih_plug::prelude::PluginState) {}
    }

    #[test]
    fn plugin_host_draws_headless() {
        let params = Arc::new(FlitParams::default());
        let shared = Shared::default();
        shared.connected.store(true, Ordering::Relaxed);
        let ctx = egui::Context::default();
        let gui: Arc<dyn GuiContext> = Arc::new(NoHost);
        let mut st = CompactState::default();
        for mode in [ParamMode::Track, ParamMode::Receiver, ParamMode::TalkSend, ParamMode::TalkReceive] {
            for _ in 0..3 {
                let setter = ParamSetter::new(gui.as_ref());
                setter.set_parameter(&params.mode, mode);
                let host = PluginHost { params: &params, setter: &setter };
                let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(520.0, 230.0))), ..Default::default() }, |ctx| {
                    flit_ui::compact::draw(ctx, &host, &mut st, &shared);
                });
            }
        }
    }
}
