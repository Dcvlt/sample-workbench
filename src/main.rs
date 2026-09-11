mod app;
mod audio;
mod playback;
mod waveform;

use app::SampleWorkbench;

fn main() -> eframe::Result {
    eframe::run_native(
        "Sample Workbench",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(SampleWorkbench::default()))),
    )
}
