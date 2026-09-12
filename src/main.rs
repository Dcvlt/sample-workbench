mod app;
mod audio;
mod edits;
mod export;
mod playback;
mod project;
mod spectrum;
mod spectrum_view;
mod theme;
mod transients;
mod waveform;

use app::SampleWorkbench;

fn main() -> eframe::Result {
    eframe::run_native(
        "SampleForge",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 780.0])
                .with_min_inner_size([820.0, 600.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(SampleWorkbench::new(&cc.egui_ctx)))),
    )
}
