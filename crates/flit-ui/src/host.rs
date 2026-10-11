//! What a surface gives the screens. The plugin panel answers with its
//! parameters (mode, track name, hear-reference) and wraps the main area in
//! nih-plug's resizable window; the app window has no modes and uses a
//! plain central panel.

/// Which plugin instance this is. The app is always `Master` (the panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Master,
    Track,
    Receiver,
    TalkSend,
    TalkReceive,
}

impl Mode {
    pub const ALL: [Mode; 5] = [Mode::Master, Mode::Track, Mode::Receiver, Mode::TalkSend, Mode::TalkReceive];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Master => "Master (panel)",
            Mode::Track => "Track (send)",
            Mode::Receiver => "Track (receive)",
            Mode::TalkSend => "Talk (send)",
            Mode::TalkReceive => "Talk (receive)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// Inside a DAW: nih-plug window, modes, the host's audio.
    Plugin,
    /// The FLit app window.
    App,
}

pub trait Host {
    fn surface(&self) -> Surface;
    fn mode(&self) -> Mode {
        Mode::Master
    }
    fn set_mode(&self, _m: Mode) {}
    /// Track name (Track/Receiver modes) or Talk name.
    fn track(&self) -> String {
        String::new()
    }
    fn set_track(&self, _t: String) {}
    /// Hear the reference mix instead of my local mix (Master mode).
    fn hear_reference(&self) -> bool {
        false
    }
    fn set_hear_reference(&self, _on: bool) {}
    /// Lay out the main area (everything between the rail and the Timeline bar).
    fn main_area(&self, ctx: &egui::Context, add: &mut dyn FnMut(&mut egui::Ui));
}

/// The app's host, and the one tests use: no modes, a central panel.
pub struct PlainHost(pub Surface);

impl Host for PlainHost {
    fn surface(&self) -> Surface {
        self.0
    }
    fn main_area(&self, ctx: &egui::Context, add: &mut dyn FnMut(&mut egui::Ui)) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(crate::theme::BG)).show(ctx, |ui| add(ui));
    }
}
