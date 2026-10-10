//! FLit plugin (VST3 + CLAP): a thin client of flit-agent.
//!
//! One plugin, several modes:
//! - Master: the panel (home, Songs/Friends/History/Chat), transport info
//!   for level 0, the reference mix (send when I own the mix lane, hear
//!   when I choose to).
//! - Lane: streams this lane out at the print point and feeds saves.
//! - Receiver: plays a friend's lane in.
//! - Talk send / Talk receive: a live track outside the song.
//!
//! Audio thread rule: `process()` only reads/writes lock-free rings and
//! atomics. A worker thread moves audio between the rings and the agent.

mod engine;
mod ui;
mod worker;

use engine::{Engine, Mode};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use worker::{Shared, Worker};

pub struct FlitPlugin {
    params: Arc<FlitParams>,
    engine: Option<Engine>,
    worker: Option<Worker>,
    shared: Arc<Shared>,
    sample_rate: f32,
    channels: usize,
}

#[derive(Params)]
pub struct FlitParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    /// Which lane this instance is (Lane/Receiver modes) or Talk name.
    #[persist = "lane"]
    pub lane: Arc<RwLock<String>>,
    /// Hear the reference mix instead of my local mix (Master mode).
    #[id = "hearref"]
    pub hear_reference: BoolParam,
    /// Monitor received audio (Receiver / Talk receive).
    #[id = "monitor"]
    pub monitor: BoolParam,
    #[id = "rxgain"]
    pub receive_gain: FloatParam,
}

impl Default for FlitParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(1200, 760),
            mode: EnumParam::new("Mode", Mode::Master).non_automatable(),
            lane: Arc::new(RwLock::new(String::new())),
            hear_reference: BoolParam::new("Hear reference mix", false).non_automatable(),
            monitor: BoolParam::new("Monitor", true),
            receive_gain: FloatParam::new("Receive gain", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_smoother(SmoothingStyle::Linear(20.0)),
        }
    }
}

impl Default for FlitPlugin {
    fn default() -> Self {
        Self { params: Arc::new(FlitParams::default()), engine: None, worker: None, shared: Arc::new(Shared::default()), sample_rate: 48_000.0, channels: 2 }
    }
}

impl Plugin for FlitPlugin {
    const NAME: &'static str = "FLit";
    const VENDOR: &'static str = "FLit";
    const URL: &'static str = "https://github.com/taborgreat/flit";
    const EMAIL: &'static str = "taborgreat@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout { main_input_channels: NonZeroU32::new(2), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_input_channels: NonZeroU32::new(1), main_output_channels: NonZeroU32::new(1), ..AudioIOLayout::const_default() },
    ];

    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        ui::create(self.params.clone(), self.shared.clone())
    }

    fn initialize(&mut self, layout: &AudioIOLayout, config: &BufferConfig, _ctx: &mut impl InitContext<Self>) -> bool {
        self.sample_rate = config.sample_rate;
        self.channels = layout.main_output_channels.map(|c| c.get() as usize).unwrap_or(2);
        let (engine, rings) = Engine::new(self.sample_rate as u32, self.channels, config.max_buffer_size as usize);
        self.engine = Some(engine);
        self.worker = Some(Worker::start(rings, self.shared.clone(), self.params.clone(), self.sample_rate as u32, self.channels as u8));
        true
    }

    fn reset(&mut self) {}

    fn deactivate(&mut self) {
        self.worker = None;
        self.engine = None;
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let Some(engine) = self.engine.as_mut() else { return ProcessStatus::Normal };
        let t = context.transport();
        self.shared.transport.store(t.playing, t.tempo.unwrap_or(120.0), t.pos_beats().unwrap_or(0.0));
        let mode = self.params.mode.value();
        let hear_ref = self.params.hear_reference.value();
        let monitor = self.params.monitor.value();
        let gain = self.params.receive_gain.smoothed.next();
        engine.process(buffer, mode, hear_ref, monitor, gain, self.shared.connected.load(Ordering::Relaxed));
        ProcessStatus::Normal
    }
}

impl ClapPlugin for FlitPlugin {
    const CLAP_ID: &'static str = "com.flit.plugin";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Work on one song together, each in your own DAW");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo, ClapFeature::Mono, ClapFeature::Utility, ClapFeature::Custom("network")];
}

impl Vst3Plugin for FlitPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"FLitTogetherP001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Network, Vst3SubCategory::Tools];
}

nih_export_clap!(FlitPlugin);
nih_export_vst3!(FlitPlugin);

#[allow(dead_code)]
fn _keep(_: &AtomicBool) {}
