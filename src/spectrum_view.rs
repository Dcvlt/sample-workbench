use crate::{
    audio::AudioClip,
    spectrum::{self, Spectrum},
    theme,
};
use eframe::egui;
use std::{
    ops::Range,
    sync::{Arc, mpsc},
};

#[derive(Default, PartialEq, Clone, Copy)]
pub(crate) enum Mode {
    #[default]
    Waveform,
    Spectrogram,
    Overlay,
}

type Pending = (Arc<[f32]>, usize, mpsc::Receiver<Spectrum>);
type ImageKey = (Range<usize>, usize, usize, u32, u32);

pub(crate) struct SpectrumView {
    pub(crate) mode: Mode,
    fft_size: usize,
    floor_db: f32,
    opacity: f32,
    source: Option<Arc<[f32]>>,
    pub(crate) data: Option<Arc<Spectrum>>,
    pub(crate) required: bool,
    pending: Option<Pending>,
    texture: Option<egui::TextureHandle>,
    image_key: Option<ImageKey>,
}

impl Default for SpectrumView {
    fn default() -> Self {
        Self {
            mode: Mode::Waveform,
            fft_size: 2048,
            floor_db: 80.0,
            opacity: 0.75,
            source: None,
            data: None,
            required: false,
            pending: None,
            texture: None,
            image_key: None,
        }
    }
}

impl SpectrumView {
    pub(crate) fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            crate::theme::segments(ui, |ui| {
                for (mode, label) in [
                    (Mode::Waveform, "Waveform"),
                    (Mode::Overlay, "Overlay"),
                    (Mode::Spectrogram, "Spectrogram"),
                ] {
                    if ui
                        .add(crate::theme::segment(label, self.mode == mode))
                        .clicked()
                    {
                        self.mode = mode;
                    }
                }
            });
            if self.mode != Mode::Waveform {
                egui::ComboBox::from_id_salt("fft_resolution")
                    .selected_text(format!("{} samples", self.fft_size))
                    .show_ui(ui, |ui| {
                        for (size, label) in [
                            (512, "512 · attack detail"),
                            (2048, "2048 · balanced"),
                            (4096, "4096 · bass detail"),
                        ] {
                            ui.selectable_value(&mut self.fft_size, size, label);
                        }
                    });
                ui.label("Range");
                ui.add(
                    egui::DragValue::new(&mut self.floor_db)
                        .range(30.0..=120.0)
                        .suffix(" dB"),
                );
                if self.mode == Mode::Overlay {
                    ui.label("Opacity");
                    ui.add(
                        egui::DragValue::new(&mut self.opacity)
                            .range(0.05..=1.0)
                            .speed(0.01),
                    );
                }
            }
        });
    }

    pub(crate) fn update(&mut self, clip: &AudioClip, ctx: &egui::Context) {
        let changed = self
            .source
            .as_ref()
            .is_some_and(|old| !Arc::ptr_eq(old, &clip.samples))
            || self
                .data
                .as_ref()
                .is_some_and(|d| d.fft_size != self.fft_size);
        if changed {
            self.source = None;
            self.data = None;
            self.texture = None;
            self.image_key = None;
        }
        if let Some((source, size, receiver)) = &self.pending {
            match receiver.try_recv() {
                Ok(data) => {
                    if Arc::ptr_eq(source, &clip.samples) && *size == self.fft_size {
                        self.source = Some(source.clone());
                        self.data = Some(Arc::new(data));
                        self.image_key = None;
                    }
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        if self.mode == Mode::Waveform && !self.required {
            return;
        }
        if self.data.is_none() && self.pending.is_none() {
            let (tx, rx) = mpsc::channel();
            let input = clip.clone();
            let size = self.fft_size;
            let repaint = ctx.clone();
            self.pending = Some((clip.samples.clone(), size, rx));
            std::thread::spawn(move || {
                let result = spectrum::analyze(&input, size);
                let _ = tx.send(result);
                repaint.request_repaint();
            });
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub(crate) fn paint(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: egui::Rect,
        view: &Range<usize>,
        gain: f32,
    ) {
        if self.mode == Mode::Waveform {
            return;
        }
        let Some(data) = &self.data else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Analyzing spectrum…",
                egui::FontId::proportional(14.0),
                theme::MUTED,
            );
            return;
        };
        let width = (rect.width() as usize).clamp(1, 1024);
        let height = (rect.height() as usize).clamp(1, 256);
        let key = (
            view.clone(),
            width,
            height,
            gain.to_bits(),
            self.floor_db.to_bits(),
        );
        let nyquist = data.sample_rate as f32 / 2.0;
        let low = 20.0_f32.min(nyquist / 2.0);
        let frequency = |y: f32| low * (nyquist / low).powf(1.0 - y);
        if self.image_key.as_ref() != Some(&key) {
            let gain_db = if gain > 0.0 {
                20.0 * gain.log10()
            } else {
                -200.0
            };
            let mut pixels = vec![egui::Color32::BLACK; width * height];
            let bins = data.fft_size / 2 + 1;
            for y in 0..height {
                let lower = (frequency((y + 1) as f32 / height as f32) * data.fft_size as f32
                    / data.sample_rate as f32)
                    .floor() as usize;
                let upper = (frequency(y as f32 / height as f32) * data.fft_size as f32
                    / data.sample_rate as f32)
                    .ceil() as usize;
                for x in 0..width {
                    let start = (view.start + x * view.len() / width) / data.hop;
                    let end = (view.start + (x + 1) * view.len() / width).div_ceil(data.hop);
                    let mut db = -120.0_f32;
                    for column in start.min(data.columns.saturating_sub(1))
                        ..end.max(start + 1).min(data.columns)
                    {
                        for bin in lower.min(bins - 1)..=upper.min(bins - 1) {
                            db = db.max(data.db[column * bins + bin]);
                        }
                    }
                    pixels[y * width + x] =
                        color(((db + gain_db + self.floor_db) / self.floor_db).clamp(0.0, 1.0));
                }
            }
            let image = egui::ColorImage::new([width, height], pixels);
            if let Some(texture) = &mut self.texture {
                texture.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.texture = Some(ui.ctx().load_texture(
                    "spectrogram",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
            self.image_key = Some(key);
        }
        if let Some(texture) = &self.texture {
            let opacity = if self.mode == Mode::Overlay {
                self.opacity
            } else {
                1.0
            };
            painter.image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE.linear_multiply(opacity),
            );
        }
        let mut last_label_y = rect.top() - 20.0;
        for hz in [20000.0, 10000.0, 5000.0, 1000.0, 500.0, 100.0, 50.0] {
            if hz < low || hz > nyquist {
                continue;
            }
            let y = rect.bottom() - (hz / low).ln() / (nyquist / low).ln() * rect.height();
            if y - last_label_y < 15.0 {
                continue;
            }
            last_label_y = y;
            painter.text(
                egui::pos2(rect.left() + 4.0, y),
                egui::Align2::LEFT_CENTER,
                format!("{hz:.0} Hz"),
                egui::FontId::monospace(10.0),
                egui::Color32::WHITE,
            );
        }
        if rect.height() > 60.0 {
            painter.text(
                rect.right_bottom() - egui::vec2(4.0, 4.0),
                egui::Align2::RIGHT_BOTTOM,
                format!(
                    "−{:.0}…0 dBFS · {:.1} ms window / {:.1} ms hop",
                    self.floor_db,
                    1000.0 * data.fft_size as f32 / data.sample_rate as f32,
                    1000.0 * data.hop as f32 / data.sample_rate as f32
                ),
                egui::FontId::monospace(10.0),
                egui::Color32::WHITE,
            );
        }
        if let Some(pointer) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) {
            let hz = frequency((pointer.y - rect.top()) / rect.height());
            let frame = view.start
                + (((pointer.x - rect.left()) / rect.width()) * view.len() as f32) as usize;
            let db = data.magnitude_db(frame, hz)
                + if gain > 0.0 {
                    20.0 * gain.log10()
                } else {
                    -200.0
                };
            painter.text(
                egui::pos2(rect.right() - 4.0, rect.top() + 4.0),
                egui::Align2::RIGHT_TOP,
                format!("{hz:.0} Hz · {db:.1} dBFS"),
                egui::FontId::monospace(11.0),
                egui::Color32::WHITE,
            );
        }
    }
}

fn color(value: f32) -> egui::Color32 {
    let stops = [
        [8., 12., 24.],
        [37., 26., 89.],
        [138., 36., 113.],
        [238., 103., 60.],
        [255., 239., 158.],
    ];
    let position = value * 4.0;
    let index = (position as usize).min(3);
    let fraction = position - index as f32;
    let channel = |i| (stops[index][i] * (1.0 - fraction) + stops[index + 1][i] * fraction) as u8;
    egui::Color32::from_rgb(channel(0), channel(1), channel(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_background_result_cannot_replace_new_audio() {
        let old = AudioClip {
            channels: 1,
            sample_rate: 48000,
            duration_seconds: 0.01,
            samples: vec![0.0; 480].into(),
        };
        let new = AudioClip {
            samples: vec![0.5; 480].into(),
            ..old.clone()
        };
        let (tx, rx) = mpsc::channel();
        tx.send(spectrum::analyze(&old, 2048)).unwrap();
        let mut view = SpectrumView {
            pending: Some((old.samples.clone(), 2048, rx)),
            ..Default::default()
        };
        view.update(&new, &egui::Context::default());
        assert!(view.data.is_none());
        assert!(view.pending.is_none());
    }
}
