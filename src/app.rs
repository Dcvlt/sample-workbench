use eframe::egui;
use std::path::PathBuf;
use std::time::Duration;

use crate::audio::{AudioClip, load_audio};
use crate::playback::Playback;
use crate::waveform::draw_waveform;

pub(crate) struct SampleWorkbench {
    gain: f32,
    selected_file: Option<PathBuf>,
    audio_info: Option<AudioClip>,
    error: Option<String>,
    playback: Option<Playback>,
    looping: bool,
    position_seconds: f64,
}

impl Default for SampleWorkbench {
    fn default() -> Self {
        Self {
            gain: 1.0,
            selected_file: None,
            audio_info: None,
            error: None,
            playback: None,
            looping: false,
            position_seconds: 0.0,
        }
    }
}

impl eframe::App for SampleWorkbench {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut finished = false;

        if let Some(playback) = &self.playback
            && let Some(clip) = &self.audio_info
        {
            finished = playback.sink.empty();

            let elapsed = playback.sink.get_pos().as_secs_f64();
            let duration = clip.duration_seconds;

            self.position_seconds = if finished {
                duration
            } else if self.looping && duration > 0.0 {
                elapsed % duration
            } else {
                elapsed.min(duration)
            };
        }

        if finished {
            self.playback = None;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Sample Workbench");
            if ui.button("Open WAV…").clicked() {
                let selection = rfd::FileDialog::new()
                    .add_filter("WAV audio", &["wav"])
                    .pick_file();
                if let Some(path) = selection {
                    match load_audio(&path) {
                        Ok(info) => {
                            self.playback = None;
                            self.selected_file = Some(path);
                            self.audio_info = Some(info);
                            self.error = None;
                        }
                        Err(error) => {
                            self.error =
                                Some(format!("Could not open {}: {error}", path.display(),));
                        }
                    }
                }
            }

            match &self.selected_file {
                Some(path) => {
                    ui.label(format!("Selected: {}", path.display()));
                }
                None => {
                    ui.label("No file selected");
                }
            }

            if let Some(info) = &self.audio_info {
                ui.label(format!(
                    "{} channel(s) · {} Hz · {:.2} seconds",
                    info.channels, info.sample_rate, info.duration_seconds,
                ));
                ui.label(format!("{} sample values loaded", info.samples.len()));
            }

            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }

            ui.add(egui::Slider::new(&mut self.gain, 0.0..=1.0).text("Gain"));

            ui.label(format!("Gain: {:.0}%", self.gain * 100.0));

            if ui.button("Reset gain").clicked() {
                self.gain = 1.0;
            }
            ui.add_enabled(
                self.playback.is_none(),
                egui::Checkbox::new(&mut self.looping, "Loop"),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.audio_info.is_some(), egui::Button::new("Play"))
                    .clicked()
                {
                    let can_resume = self
                        .playback
                        .as_ref()
                        .is_some_and(|playback| !playback.sink.empty());

                    if can_resume {
                        if let Some(playback) = &self.playback {
                            playback.sink.play();
                        }
                    } else if let Some(clip) = &self.audio_info {
                        self.playback = None;

                        match Playback::new(clip, self.gain, self.looping) {
                            Ok(playback) => {
                                self.position_seconds = 0.0;
                                self.playback = Some(playback);
                                self.error = None;
                            }
                            Err(error) => {
                                self.error = Some(format!("Could not start audio: {error}"));
                            }
                        }
                    }
                }

                if let Some(playback) = &self.playback
                    && ui.button("Pause").clicked()
                {
                    playback.sink.pause();
                }

                if ui.button("Stop").clicked() {
                    self.playback = None;
                    self.position_seconds = 0.0;
                }
            });

            if let Some(playback) = &self.playback {
                playback.sink.set_volume(self.gain);
            }
            if let Some(clip) = &self.audio_info {
                let progress = if clip.duration_seconds > 0.0 {
                    (self.position_seconds / clip.duration_seconds).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };

                let status = match &self.playback {
                    Some(playback) if playback.sink.is_paused() => "Paused",
                    Some(_) => "Playing",
                    None => "Stopped",
                };

                ui.label(format!(
                    "{status} · {:.2} / {:.2} s",
                    self.position_seconds, clip.duration_seconds,
                ));

                ui.add(egui::ProgressBar::new(progress));

                ui.add_space(12.0);
                draw_waveform(ui, clip, self.gain, progress);
            }
        });
        if let Some(playback) = &self.playback
            && !playback.sink.is_paused()
        {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}
