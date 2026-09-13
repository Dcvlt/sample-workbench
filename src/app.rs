use eframe::egui::{self, RichText};
use std::{ops::Range, path::PathBuf, time::Duration};

use crate::audio::{AudioClip, load_audio};
use crate::edits::{self, EditHistory};
use crate::export::export_region;
use crate::playback::Playback;
use crate::project::{self, ProjectData};
use crate::theme;
use crate::waveform::{WaveformAction, WaveformCache};

type DetectionWork = (
    std::sync::Arc<[f32]>,
    bool,
    std::sync::mpsc::Receiver<crate::transients::Detection>,
);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Workflow {
    Analyze,
    Edit,
    Export,
}

pub(crate) struct SampleWorkbench {
    workflow: Workflow,
    gain: f32,
    selected_file: Option<PathBuf>,
    audio_info: Option<AudioClip>,
    error: Option<String>,
    playback: Option<Playback>,
    looping: bool,
    position_seconds: f64,
    waveform: WaveformCache,
    selection: Option<Range<usize>>,
    export_status: Option<String>,
    selection_pending: bool,
    original_audio: Option<AudioClip>,
    edits: EditHistory,
    bypass_edits: bool,
    fade_ms: f32,
    project_path: Option<PathBuf>,
    attack_markers: Vec<usize>,
    selected_attack_markers: Vec<usize>,
    active_attack: Option<usize>,
    sensitivity: f32,
    attack_spacing_ms: f32,
    spectral_detector: bool,
    detect_requested: bool,
    detection_work: Option<DetectionWork>,
    grid_enabled: bool,
    grid_bpm: f32,
    grid_subdivision: u32,
    preview_timing: bool,
    audio_clipboard: Option<Vec<f32>>,
    distortion_enabled: bool,
    distortion_drive: f32,
    audition_audio: Option<AudioClip>,
    recent_files: Vec<PathBuf>,
}

impl Default for SampleWorkbench {
    fn default() -> Self {
        Self {
            workflow: Workflow::Analyze,
            gain: 1.0,
            selected_file: None,
            audio_info: None,
            error: None,
            playback: None,
            looping: false,
            position_seconds: 0.0,
            waveform: WaveformCache::default(),
            selection: None,
            export_status: None,
            selection_pending: false,
            original_audio: None,
            edits: EditHistory::default(),
            bypass_edits: false,
            fade_ms: 5.0,
            project_path: None,
            attack_markers: Vec::new(),
            selected_attack_markers: Vec::new(),
            active_attack: None,
            sensitivity: 0.65,
            attack_spacing_ms: 60.0,
            spectral_detector: true,
            detect_requested: false,
            detection_work: None,
            grid_enabled: false,
            grid_bpm: 120.0,
            grid_subdivision: 1,
            preview_timing: false,
            audio_clipboard: None,
            distortion_enabled: false,
            distortion_drive: 30.0,
            audition_audio: None,
            recent_files: Self::load_recent(),
        }
    }
}

impl SampleWorkbench {
    fn recent_path() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .map(|path| PathBuf::from(path).join("SampleForge").join("recent.txt"))
    }
    fn load_recent() -> Vec<PathBuf> {
        Self::recent_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| text.lines().take(10).map(PathBuf::from).collect())
            .unwrap_or_default()
    }
    fn remember_recent(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        self.recent_files.retain(|entry| entry != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(10);
        if let Some(path) = Self::recent_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(
                path,
                self.recent_files
                    .iter()
                    .map(|p| p.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
    }
    fn rebuild_audition(&mut self) {
        self.audition_audio = if self.distortion_enabled {
            self.audio_info
                .as_ref()
                .map(|clip| crate::playback::distortion_preview(clip, self.distortion_drive))
        } else {
            None
        };
    }
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        theme::apply(ctx);
        let mut app = Self::default();
        // A WAV path is optional when launching from the terminal.
        if let Some(path) = std::env::args_os().nth(1) {
            app.open_path(path.into());
        }
        app
    }

    fn open_path(&mut self, path: PathBuf) {
        match load_audio(&path) {
            Ok(clip) => {
                self.audio_clipboard = None;
                self.playback = None;
                self.position_seconds = 0.0;
                self.waveform.clear();
                self.preview_timing = false;
                self.selection = None;
                self.selection_pending = false;
                self.export_status = None;
                self.selected_file = Some(path);
                self.original_audio = Some(clip.clone());
                self.edits = EditHistory::default();
                self.bypass_edits = false;
                self.fade_ms = 5.0;
                self.project_path = None;
                self.attack_markers.clear();
                self.selected_attack_markers.clear();
                self.detect_requested = false;
                self.active_attack = None;
                self.audio_info = Some(clip);
                self.rebuild_audition();
                if let Some(path) = self.selected_file.clone() {
                    self.remember_recent(path);
                }
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Could not open {}: {error}", path.display())),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .pick_file()
        {
            self.open_path(path);
        }
    }

    fn save_project(&mut self) {
        let Some(source) = &self.selected_file else {
            self.error = Some("Load a WAV before saving a project".into());
            return;
        };
        let path = self.project_path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("SampleForge project", &["swp"])
                .set_file_name("untitled.swp")
                .save_file()
        });
        let Some(path) = path else {
            return;
        };
        let data = ProjectData {
            source: source.clone(),
            state: self.edits.current.clone(),
            selection: self.selection.clone(),
            gain: self.gain,
            looping: self.looping,
            bypass: self.bypass_edits,
            markers: self.attack_markers.clone(),
        };
        match project::save(&path, &data) {
            Ok(()) => {
                self.project_path = Some(path.clone());
                self.remember_recent(path.clone());
                self.export_status = Some(format!("Saved {}", path.display()));
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn open_project(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("SampleForge project", &["swp"])
            .pick_file()
        else {
            return;
        };
        self.open_project_path(path);
    }

    fn open_project_path(&mut self, path: PathBuf) {
        match project::load(&path) {
            Ok(data) => match load_audio(&data.source) {
                Ok(clip) => {
                    self.audio_clipboard = None;
                    let frames = clip.samples.len() / usize::from(clip.channels);
                    let pool_frames =
                        frames + data.state.inserted.len() / usize::from(clip.channels);
                    if let Some(pieces) = &data.state.timeline {
                        let total = pieces
                            .iter()
                            .try_fold(0usize, |sum, piece| sum.checked_add(piece.len));
                        if total.is_none()
                            || data.state.inserted.len() % usize::from(clip.channels) != 0
                            || pieces.iter().any(|piece| {
                                piece.source.is_some_and(|start| {
                                    start
                                        .checked_add(piece.len)
                                        .is_none_or(|end| end > pool_frames)
                                })
                            })
                        {
                            self.error =
                                Some("Project timing slices exceed the source audio.".into());
                            return;
                        }
                    }
                    self.attack_markers = data
                        .markers
                        .into_iter()
                        .filter(|&frame| frame < pool_frames)
                        .collect();
                    self.attack_markers.sort_unstable();
                    self.attack_markers.dedup();
                    self.active_attack = None;
                    self.selected_attack_markers.clear();
                    self.playback = None;
                    self.selected_file = Some(data.source);
                    self.original_audio = Some(clip.clone());
                    self.edits = project::history(data.state);
                    self.gain = data.gain.clamp(0.0, 1.0);
                    self.looping = data.looping;
                    self.bypass_edits = data.bypass;
                    self.audio_info = Some(clip);
                    let restored_selection = data.selection;
                    self.selection = None;
                    self.remember_recent(path.clone());
                    self.project_path = Some(path);
                    self.waveform.clear();
                    self.position_seconds = self.range_start();
                    self.error = None;
                    self.export_status = None;
                    self.refresh_edits();
                    let total = self
                        .audio_info
                        .as_ref()
                        .map_or(0, |c| c.samples.len() / usize::from(c.channels));
                    self.selection =
                        restored_selection.filter(|r| r.start < r.end && r.end <= total);
                    self.position_seconds = self.range_start();
                    self.selection_pending = false;
                }
                Err(e) => self.error = Some(format!("Could not open project source: {e}")),
            },
            Err(e) => self.error = Some(e),
        }
    }

    fn prepare_marker_coordinates(&mut self) {
        if self.bypass_edits {
            return;
        }
        if let Some(source) = &self.original_audio {
            let channels = usize::from(source.channels);
            let mut next = self.edits.current.clone();
            if next.materialize_gaps(source.samples.len() / channels, channels) {
                self.edits.commit(next);
            }
        }
    }

    fn add_marker_at_playhead(&mut self) {
        self.prepare_marker_coordinates();
        let Some(clip) = &self.audio_info else {
            return;
        };
        let frames = clip.samples.len() / usize::from(clip.channels);
        if frames == 0 {
            return;
        }
        let position = ((self.position_seconds * f64::from(clip.sample_rate)).round() as usize)
            .min(frames - 1);
        if let Some(source) = self.source_frame(position) {
            self.attack_markers.push(source);
            self.attack_markers.sort_unstable();
            self.attack_markers.dedup();
            self.active_attack = Some(source);
            self.selected_attack_markers = vec![source];
        }
    }

    fn source_frame(&self, frame: usize) -> Option<usize> {
        let clip = self.original_audio.as_ref()?;
        let total = clip.samples.len() / usize::from(clip.channels);
        if self.bypass_edits {
            return (frame < total).then_some(frame);
        }
        self.edits
            .current
            .source_ranges(total, frame..frame.saturating_add(1))
            .first()
            .map(|r| r.start)
    }

    fn visible_attacks(&self) -> Vec<(usize, usize)> {
        let Some(clip) = &self.original_audio else {
            return Vec::new();
        };
        let frames = clip.samples.len() / usize::from(clip.channels);
        let mut attacks: Vec<_> = self
            .attack_markers
            .iter()
            .filter_map(|&source| {
                let visible = if self.bypass_edits {
                    Some(source)
                } else {
                    self.edits.current.visible_frame(frames, source)
                };
                visible.map(|frame| (source, frame))
            })
            .collect();
        attacks.sort_by_key(|&(_, frame)| frame);
        attacks
    }

    fn jump_to_attack(&mut self, source: usize, frame: usize) {
        self.active_attack = Some(source);
        if let Some(clip) = &self.audio_info {
            self.position_seconds = frame as f64 / f64::from(clip.sample_rate);
            self.waveform
                .reveal(frame, clip.samples.len() / usize::from(clip.channels));
        }
        // Navigation leaves a selection loop so attacks outside it are reachable.
        self.selection = None;
        self.selection_pending = false;
        self.reconfigure_playback(false);
    }

    fn attack_controls(&mut self, ui: &mut egui::Ui) {
        if self.audio_info.is_none() {
            return;
        }
        ui.vertical(|ui| {
            let busy=self.detect_requested || self.detection_work.is_some();
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    theme::segments(ui, |ui| {
                        for (spectral, compare, label) in [(true,false,"Spectral"),(true,true,"Both"),(false,false,"Envelope")] {
                            let active = self.spectral_detector == spectral && self.waveform.compare_legacy == compare;
                            if ui.add(theme::segment(label, active)).clicked() {
                                self.spectral_detector = spectral;
                                self.waveform.compare_legacy = compare;
                            }
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(theme::icon_button("Generate markers", theme::Icon::Add))
                            .on_hover_text("Replaces existing markers. Both generates spectral markers and displays envelope candidates for comparison.")
                            .clicked() { self.detect_requested = true; }
                    });
                });
                ui.horizontal(|ui| { ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.label(format!("{} attacks", self.visible_attacks().len())); }); });
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Sensitivity: {:.0}%", self.sensitivity * 100.0));
                    ui.add(egui::Slider::new(&mut self.sensitivity, 0.0..=1.0).show_value(false));
                    ui.add_space(12.0);
                    ui.label("Min spacing");
                    ui.add(egui::DragValue::new(&mut self.attack_spacing_ms).range(5.0..=500.0).suffix(" ms"));
                });
            });
            if busy { ui.horizontal(|ui| { ui.spinner(); ui.label("Analyzing attacks…"); }); }
            ui.horizontal_wrapped(|ui| {            if let Some(result)=&self.waveform.detection {
                ui.label(format!("Spectral: {} · Envelope: {}",result.spectral.len(),result.legacy.len()));
                ui.label(RichText::new("Last analysis").small().color(theme::MUTED)).on_hover_text("Generate again after changing detector settings.");
            }
            });
        });
    }

    fn update_detection(&mut self, ctx: &egui::Context) {
        let Some(clip) = &self.audio_info else {
            return;
        };
        if let Some((source, spectral, receiver)) = &self.detection_work {
            match receiver.try_recv() {
                Ok(result) => {
                    if std::sync::Arc::ptr_eq(source, &clip.samples) {
                        let markers = if *spectral {
                            &result.spectral
                        } else {
                            &result.legacy
                        };
                        self.prepare_marker_coordinates();
                        let mut detected: Vec<usize> = markers
                            .iter()
                            .filter_map(|&frame| self.source_frame(frame))
                            .collect();
                        detected.sort_unstable();
                        detected.dedup();
                        self.attack_markers = detected;
                        self.active_attack = None;
                        self.export_status = None;
                        self.waveform.detection = Some(std::sync::Arc::new(result));
                    }
                    self.detection_work = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.detection_work = None;
                    self.error =
                        Some("Attack analysis did not complete. Try detecting again.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => (),
            }
        }
        if self.detect_requested && self.detection_work.is_none() {
            let clip = self.audio_info.as_ref().unwrap();
            if let Some(spectrum) = self.waveform.analysis(clip, ctx, true) {
                let (tx, rx) = std::sync::mpsc::channel();
                let input = clip.clone();
                let repaint = ctx.clone();
                let sensitivity = self.sensitivity;
                let spacing = self.attack_spacing_ms;
                self.detection_work = Some((clip.samples.clone(), self.spectral_detector, rx));
                self.detect_requested = false;
                std::thread::spawn(move || {
                    let result =
                        crate::transients::contextual(&spectrum, &input, sensitivity, spacing);
                    let _ = tx.send(result);
                    repaint.request_repaint();
                });
            }
        }
        if self.detection_work.is_some() || self.detect_requested {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn refresh_edits(&mut self) {
        self.preview_timing = false;
        self.detect_requested = false;
        let old_len = self
            .audio_info
            .as_ref()
            .map_or(0, |clip| clip.samples.len());
        let Some(source) = &self.original_audio else {
            return;
        };
        self.audio_info = Some(if self.bypass_edits {
            source.clone()
        } else {
            edits::render(source, &self.edits.current)
        });
        let clip = self.audio_info.as_ref().unwrap();
        if clip.samples.len() != old_len {
            self.playback = None;
            self.selection = None;
            self.selection_pending = false;
            self.position_seconds = self.position_seconds.min(clip.duration_seconds);
            self.waveform.clear();
        } else {
            self.waveform.invalidate_audio();
        }
        self.export_status = None;
        self.rebuild_audition();
        self.sync_playback();
        self.reconfigure_playback(false);
    }

    fn delete_selection(&mut self) {
        if self.bypass_edits {
            return;
        }
        let (Some(selection), Some(source)) = (self.selection.clone(), &self.original_audio) else {
            return;
        };
        let frames = source.samples.len() / usize::from(source.channels);
        let ranges = self.edits.current.source_ranges(frames, selection.clone());
        if ranges.is_empty() && self.edits.current.timeline.is_none() {
            return;
        }
        let target = selection.start as f64 / f64::from(source.sample_rate);
        let mut next = self.edits.current.clone();
        next.remove_time(frames, selection);
        if self.edits.commit(next) {
            self.refresh_edits();
            self.position_seconds = target.min(self.audio_info.as_ref().unwrap().duration_seconds);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Leave keys alone while editing a text/numeric field. Ignore auto-repeat.
        let has_clipboard_event = ctx.input(|i| {
            i.events
                .iter()
                .any(|event| matches!(event, egui::Event::Copy | egui::Event::Paste(_)))
        });
        if ctx.wants_keyboard_input() && !has_clipboard_event {
            return;
        }
        let keys = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Copy => Some((egui::Key::C, true, false)),
                    egui::Event::Paste(_) => Some((egui::Key::V, true, false)),
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if (!modifiers.ctrl && !modifiers.alt && !modifiers.command)
                        || (*key == egui::Key::Backspace && modifiers.ctrl)
                        || (matches!(key, egui::Key::Z | egui::Key::C | egui::Key::V)
                            && modifiers.ctrl) =>
                    {
                        Some((*key, modifiers.ctrl, modifiers.shift))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let mut handled = Vec::new();
        for (key, ctrl, shift) in keys {
            if handled.contains(&(key, ctrl, shift)) {
                continue;
            }
            handled.push((key, ctrl, shift));
            if matches!(
                key,
                egui::Key::Space | egui::Key::Escape | egui::Key::Backspace
            ) {
                ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, key);
                });
            }
            if key == egui::Key::Z && ctrl {
                ctx.input_mut(|i| {
                    i.consume_key(
                        egui::Modifiers {
                            ctrl: true,
                            shift,
                            ..Default::default()
                        },
                        key,
                    )
                });
            }
            match key {
                egui::Key::C if ctrl => {
                    self.copy_audio();
                    if self.audio_clipboard.is_some() {
                        ctx.copy_text("SampleForge audio selection".to_owned());
                    }
                }
                egui::Key::V if ctrl => self.paste_audio(),
                egui::Key::Space => {
                    if self.playback.as_ref().is_some_and(|p| !p.sink.is_paused()) {
                        self.playback = None;
                        self.position_seconds = self.range_start();
                    } else if self
                        .audio_info
                        .as_ref()
                        .is_some_and(|c| !c.samples.is_empty())
                    {
                        self.play();
                    }
                }
                egui::Key::Escape => self.set_selection(None),
                egui::Key::Backspace if ctrl => self.delete_selected_markers(),
                egui::Key::Backspace => self.delete_selection(),
                egui::Key::Z if ctrl && shift => {
                    self.redo_edit();
                }
                egui::Key::Z if ctrl => {
                    self.undo_edit();
                }
                _ => (),
            }
        }
    }

    fn copy_audio(&mut self) {
        if let (Some(clip), Some(range)) = (&self.audio_info, &self.selection) {
            let channels = usize::from(clip.channels);
            self.audio_clipboard =
                Some(clip.samples[range.start * channels..range.end * channels].to_vec());
        }
    }

    fn paste_audio(&mut self) {
        if self.bypass_edits {
            return;
        }
        let (Some(audio), Some(source), Some(clip)) = (
            &self.audio_clipboard,
            &self.original_audio,
            &self.audio_info,
        ) else {
            return;
        };
        if audio.is_empty() {
            return;
        }
        let channels = usize::from(clip.channels);
        let length = audio.len() / channels;
        let position = ((self.position_seconds * f64::from(clip.sample_rate)).round() as usize)
            .min(clip.samples.len() / channels);
        let range = self.selection.clone().unwrap_or(position..position);
        let start = range.start;
        let mut next = self.edits.current.clone();
        next.paste(source.samples.len() / channels, channels, range, audio);
        if self.edits.commit(next) {
            self.refresh_edits();
            self.set_selection(Some(start..start + length));
        }
    }

    fn remove_all_markers(&mut self) {
        self.attack_markers.clear();
        self.selected_attack_markers.clear();
        self.active_attack = None;
        self.preview_timing = false;
        self.detect_requested = false;
        self.detection_work = None;
        self.waveform.compare_legacy = false;
    }

    fn delete_selected_markers(&mut self) {
        if self.selected_attack_markers.is_empty() {
            return;
        }
        let selected = self.selected_attack_markers.clone();
        self.attack_markers
            .retain(|marker| !selected.contains(marker));
        self.selected_attack_markers.clear();
        self.active_attack = None;
        self.export_status = None;
    }

    fn undo_edit(&mut self) {
        if self.edits.undo() {
            self.refresh_edits();
        }
    }
    fn redo_edit(&mut self) {
        if self.edits.redo() {
            self.refresh_edits();
        }
    }

    fn edit_controls(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut history_changed = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.selection.is_some(),
                    theme::icon_button("Copy", theme::Icon::Copy),
                )
                .on_hover_text("Ctrl+C: copy selected audio")
                .clicked()
            {
                self.copy_audio();
            }
            if ui
                .add_enabled(
                    self.audio_clipboard.is_some() && !self.bypass_edits,
                    theme::icon_button("Paste", theme::Icon::Paste),
                )
                .on_hover_text("Ctrl+V: replace selection, or insert at playhead")
                .clicked()
            {
                self.paste_audio();
            }
            if ui
                .add_enabled(
                    self.selection.is_some() && !self.bypass_edits,
                    theme::icon_button("Silence selection", theme::Icon::Silence),
                )
                .clicked()
                && let Some(range) = &self.selection
            {
                let mut next = self.edits.current.clone();
                if let Some(source) = &self.original_audio {
                    let frames = source.samples.len() / usize::from(source.channels);
                    for part in next.source_ranges(frames, range.clone()) {
                        if !next.silences.contains(&part) {
                            next.silences.push(part);
                        }
                    }
                }
                if let Some(clip) = &self.audio_info {
                    next.fade_frames =
                        (self.fade_ms * clip.sample_rate as f32 / 1000.0).round() as usize;
                }
                changed = self.edits.commit(next);
                changed |= self.bypass_edits;
                self.bypass_edits = false;
            }
            if ui
                .add_enabled(
                    self.selection.is_some() && !self.bypass_edits,
                    theme::icon_button("Delete selection", theme::Icon::Delete),
                )
                .on_hover_text("Backspace: remove time and close the gap")
                .clicked()
            {
                self.delete_selection();
            }
            if ui
                .add_enabled(
                    self.edits.can_undo(),
                    theme::icon_button("Undo", theme::Icon::Undo),
                )
                .clicked()
            {
                changed |= self.edits.undo();
                history_changed = true;
            }
            if ui
                .add_enabled(
                    self.edits.can_redo(),
                    theme::icon_button("Redo", theme::Icon::Redo),
                )
                .clicked()
            {
                changed |= self.edits.redo();
                history_changed = true;
            }
        });
        if history_changed && let Some(clip) = &self.audio_info {
            self.fade_ms = self.edits.current.fade_frames as f32 * 1000.0 / clip.sample_rate as f32;
        }
        if self.edits.current.silences.is_empty()
            && self.edits.current.deletions.is_empty()
            && self.edits.current.timeline.is_none()
        {
            self.bypass_edits = false;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Edge fades");
            ui.add(
                egui::DragValue::new(&mut self.fade_ms)
                    .range(0.0..=20.0)
                    .speed(0.1)
                    .suffix(" ms"),
            );
            if ui
                .add_enabled(
                    !self.edits.current.silences.is_empty()
                        || self.edits.current.timeline.is_some()
                        || !self.edits.current.deletions.is_empty(),
                    theme::icon_button("Apply to all edits", theme::Icon::Save),
                )
                .clicked()
                && let Some(clip) = &self.audio_info
            {
                let mut next = self.edits.current.clone();
                next.fade_frames =
                    (self.fade_ms * clip.sample_rate as f32 / 1000.0).round() as usize;
                let frames = self.original_audio.as_ref().map_or(0, |source| {
                    source.samples.len() / usize::from(source.channels)
                });
                next.apply_edge_fades(frames, next.fade_frames);
                changed |= self.edits.commit(next);
            }
            changed |= ui
                .add_enabled(
                    !self.edits.current.silences.is_empty()
                        || !self.edits.current.deletions.is_empty()
                        || self.edits.current.timeline.is_some(),
                    theme::toggle(&mut self.bypass_edits, "Bypass edits"),
                )
                .changed();
        });
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("BPM");
                ui.add(
                    egui::DragValue::new(&mut self.grid_bpm)
                        .range(30.0..=300.0)
                        .speed(0.5),
                );
                ui.label("Subdivision");
                egui::ComboBox::from_id_salt("grid_subdivision")
                    .selected_text(format!("1/{}", self.grid_subdivision))
                    .show_ui(ui, |ui| {
                        for value in [1, 2, 4, 8, 16] {
                            ui.selectable_value(
                                &mut self.grid_subdivision,
                                value,
                                format!("1/{value}"),
                            );
                        }
                    });
                ui.add(theme::toggle(&mut self.grid_enabled, "Grid"));
                if ui
                    .add_enabled(
                        self.grid_enabled
                            && !self.bypass_edits
                            && !self.selected_attack_markers.is_empty(),
                        theme::icon_button("Preview timing", theme::Icon::Marker),
                    )
                    .on_hover_text(
                        "Show grid targets for selected attacks. Audio changes only when applied.",
                    )
                    .clicked()
                {
                    self.preview_timing = true;
                }
                if self.preview_timing {
                    if ui
                        .add(theme::icon_button("Apply timing", theme::Icon::Select))
                        .clicked()
                    {
                        self.apply_timing();
                    }
                    if ui
                        .add(theme::icon_button("Cancel", theme::Icon::Clear))
                        .clicked()
                    {
                        self.preview_timing = false;
                    }
                }
            });
            if self.preview_timing {
                let resolved = self.timing_targets();
                let skipped = self
                    .requested_timing_targets()
                    .iter()
                    .zip(&resolved)
                    .filter(|(requested, resolved)| requested.1 != resolved.1)
                    .count();
                let moved = self
                    .timing_targets()
                    .iter()
                    .filter(|(from, to)| from != to)
                    .count();
                ui.label(
                    RichText::new(format!(
                        "{moved} attacks targeted · {skipped} kept in place · amber = destination"
                    ))
                    .small()
                    .color(theme::AMBER),
                );
            }
        });
        if self.bypass_edits {
            ui.label(
                RichText::new(
                    "ORIGINAL AUDIO — turn bypass off to edit; export also bypasses edits",
                )
                .small()
                .color(theme::AMBER),
            );
        }
        if changed {
            self.refresh_edits();
        }
    }

    fn timing_targets(&self) -> Vec<(usize, usize)> {
        let frames = self
            .audio_info
            .as_ref()
            .map_or(0, |clip| clip.samples.len() / usize::from(clip.channels));
        edits::resolve_targets(&self.requested_timing_targets(), frames)
    }

    fn requested_timing_targets(&self) -> Vec<(usize, usize)> {
        let Some(clip) = &self.audio_info else {
            return Vec::new();
        };
        let step = 60.0 * f64::from(clip.sample_rate)
            / f64::from(self.grid_bpm.max(1.0))
            / f64::from(self.grid_subdivision.max(1));
        self.visible_attacks()
            .into_iter()
            .map(|(source, frame)| {
                let target = if self.selected_attack_markers.contains(&source) {
                    (frame as f64 / step).round() as usize
                } else {
                    0
                };
                (
                    frame,
                    if self.selected_attack_markers.contains(&source) {
                        (target as f64 * step).round() as usize
                    } else {
                        frame
                    },
                )
            })
            .collect()
    }

    fn apply_timing(&mut self) {
        if self.bypass_edits || !self.grid_enabled {
            return;
        }
        let Some(source) = &self.original_audio else {
            return;
        };
        let frames = source.samples.len() / usize::from(source.channels);
        let fade = (self.fade_ms * source.sample_rate as f32 / 1000.0).round() as usize;
        let targets = self.timing_targets();
        let moved = targets.iter().filter(|(from, to)| from != to).count();
        let skipped = self
            .requested_timing_targets()
            .iter()
            .zip(&targets)
            .filter(|(requested, resolved)| requested.1 != resolved.1)
            .count();
        match edits::quantize(&self.edits.current, frames, &targets, fade) {
            Ok(next) => {
                if self.edits.commit(next) {
                    self.refresh_edits();
                }
                self.preview_timing = false;
                self.error = None;
                self.export_status = Some(format!(
                    "{moved} attacks moved · {skipped} left in place to avoid collisions or clip edges"
                ));
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn play(&mut self) {
        if let Some(playback) = &self.playback
            && !playback.sink.empty()
        {
            playback.sink.play();
            return;
        }
        let Some(clip) = &self.audio_info else {
            return;
        };
        self.playback = None;
        let range = self.playback_range();
        let range_start = range.start as f64 / f64::from(clip.sample_rate);
        let range_end = range.end as f64 / f64::from(clip.sample_rate);
        let start = if self.position_seconds < range_start || self.position_seconds >= range_end {
            range_start
        } else {
            self.position_seconds
        };
        let clip = self.audition_audio.as_ref().unwrap_or(clip);
        match Playback::new(clip, self.gain, self.looping, start, range) {
            Ok(playback) => {
                self.position_seconds = start;
                self.playback = Some(playback);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Could not start audio: {error}")),
        }
    }

    fn export(&mut self) {
        let Some(clip) = &self.audio_info else {
            return;
        };
        let stem = self
            .selected_file
            .as_ref()
            .and_then(|path| path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "sample".to_owned());
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .set_file_name(format!("{stem}-selection.wav"))
            .save_file()
        {
            let frames = clip.samples.len() / usize::from(clip.channels);
            let range = self.selection.clone().unwrap_or(0..frames);
            match export_region(&path, clip, range, self.gain) {
                Ok(()) => {
                    self.error = None;
                    self.export_status = Some(format!("Exported {}", path.display()));
                }
                Err(error) => {
                    self.error = Some(error);
                    self.export_status = None;
                }
            }
        }
    }

    fn progress(&self) -> f32 {
        self.audio_info
            .as_ref()
            .filter(|clip| clip.duration_seconds > 0.0)
            .map_or(0.0, |clip| {
                (self.position_seconds / clip.duration_seconds).clamp(0.0, 1.0) as f32
            })
    }

    fn playback_range(&self) -> Range<usize> {
        self.selection.clone().unwrap_or_else(|| {
            0..self
                .audio_info
                .as_ref()
                .map_or(0, |clip| clip.samples.len() / usize::from(clip.channels))
        })
    }

    fn range_start(&self) -> f64 {
        self.audio_info.as_ref().map_or(0.0, |clip| {
            self.playback_range().start as f64 / f64::from(clip.sample_rate)
        })
    }

    fn set_selection(&mut self, selection: Option<Range<usize>>) {
        self.selection = selection;
        self.export_status = None;
        self.selection_pending = true;
        if self.playback.is_none() {
            self.position_seconds = self.range_start();
        }
    }

    fn reconfigure_playback(&mut self, restart_region: bool) {
        let range = self.playback_range();
        let target = if restart_region {
            self.range_start()
        } else {
            self.position_seconds
        };
        if let Some(playback) = &mut self.playback
            && let Some(clip) = self.audition_audio.as_ref().or(self.audio_info.as_ref())
        {
            match playback.reconfigure(clip, range, self.looping, self.gain, target) {
                Ok(()) => {
                    self.position_seconds = playback.position();
                    self.error = None;
                }
                Err(error) => {
                    self.error = Some(error);
                    self.playback = None;
                }
            }
        }
    }

    fn sync_playback(&mut self) {
        if let Some(playback) = &self.playback {
            self.position_seconds = playback.position();
            if playback.sink.empty() {
                self.playback = None;
            }
        }
    }
    fn header(&mut self, ctx: &egui::Context) {
        // Recent-file access is shared across every workflow step.
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(18))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    theme::mark(ui, 28.0);
                    ui.label(RichText::new("SAMPLEFORGE").size(17.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized(
                                [118.0, 36.0],
                                theme::icon_button("Open WAV…", theme::Icon::Add),
                            )
                            .clicked()
                        {
                            self.open_dialog();
                        }
                        if ui
                            .add(theme::icon_button("Open project", theme::Icon::Open))
                            .clicked()
                        {
                            self.open_project();
                        }
                        if ui
                            .add(theme::icon_button("Save project", theme::Icon::Save))
                            .clicked()
                        {
                            self.save_project();
                        }
                        let recent = ui.add(theme::icon_button("Open recent", theme::Icon::Open));
                        egui::Popup::menu(&recent).show(|ui| {
                            if self.recent_files.is_empty() {
                                ui.label("No recent files");
                            }
                            for path in self.recent_files.clone() {
                                let name = path.file_name().unwrap_or_default().to_string_lossy();
                                if ui
                                    .button(name)
                                    .on_hover_text(path.display().to_string())
                                    .clicked()
                                {
                                    if path
                                        .extension()
                                        .is_some_and(|ext| ext.eq_ignore_ascii_case("swp"))
                                    {
                                        self.open_project_path(path);
                                    } else {
                                        self.open_path(path);
                                    }
                                    ui.close();
                                }
                            }
                        });
                    });
                });
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    for (step, label, hint) in [
                        (
                            Workflow::Analyze,
                            "01  ANALYZE",
                            "Find attacks and frequency changes",
                        ),
                        (Workflow::Edit, "02  EDIT", "Shape silence and timing"),
                        (Workflow::Export, "03  EXPORT", "Render the finished DI"),
                    ] {
                        let active = self.workflow == step;
                        let text = RichText::new(label).size(11.0).strong().color(if active {
                            theme::ACCENT
                        } else {
                            theme::MUTED
                        });
                        let response = ui
                            .add(egui::Button::new(text.monospace()).frame(false))
                            .on_hover_text(hint);
                        if response.clicked() {
                            self.workflow = step;
                        }
                        if active {
                            let rect = response.rect;
                            ui.painter().line_segment(
                                [rect.left_bottom(), rect.right_bottom()],
                                egui::Stroke::new(2.0_f32, theme::ACCENT),
                            );
                        }
                        if step != Workflow::Export {
                            ui.label(RichText::new("›").color(theme::LINE));
                        }
                    }
                });
            });
    }

    fn transport(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("transport")
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(18))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let playing = self.playback.as_ref().is_some_and(|p| !p.sink.is_paused());
                    let has_audio = self
                        .audio_info
                        .as_ref()
                        .is_some_and(|clip| !clip.samples.is_empty());
                    if ui
                        .add_enabled(
                            has_audio,
                            theme::icon_button(if playing { "Pause" } else { "Play" }, if playing { theme::Icon::Pause } else { theme::Icon::Play }),
                        )
                        .clicked()
                    {
                        if playing {
                            if let Some(p) = &self.playback {
                                p.sink.pause();
                            }
                        } else {
                            self.play();
                        }
                    }
                    if ui
                        .add_enabled(
                            has_audio,
                            theme::icon_button("Stop", theme::Icon::Stop),
                        )
                        .clicked()
                    {
                        self.playback = None;
                        self.position_seconds = self.range_start();
                    }
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(theme::time(self.position_seconds))
                                    .monospace()
                                    .size(22.0)
                                    .color(theme::TEXT),
                            );
                            let total = self
                                .audio_info
                                .as_ref()
                                .map_or(0.0, |clip| clip.duration_seconds);
                            ui.label(
                                RichText::new(format!("/ {}", theme::time(total)))
                                    .monospace()
                                    .size(13.0)
                                    .color(theme::MUTED),
                            );
                        });
                        let (label, color) = match &self.playback {
                            Some(p) if p.sink.is_paused() => ("PAUSED", theme::AMBER),
                            Some(_) => ("PLAYING", theme::ACCENT),
                            None => ("STOPPED", theme::MUTED),
                        };
                        ui.label(RichText::new(label).size(10.0).strong().color(color));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.selection.is_some() { "Loop selection" } else { "Loop clip" };
                        if ui.add(theme::toggle(&mut self.looping, label))
                            .on_hover_text("Repeat the selected region, or the whole clip when no region is selected. Can be changed during playback.")
                            .changed()
                        {
                            self.reconfigure_playback(false);
                        }
                    });
                });
                ui.add_space(6.0);
                ui.add(
                    egui::ProgressBar::new(self.progress())
                        .desired_height(3.0)
                        .fill(theme::ACCENT),
                );
            });
    }

    fn inspector(&mut self, ctx: &egui::Context) {
        let mut audition_changed = false;
        egui::SidePanel::left("inspector")
            .exact_width(242.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(20))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 7.0;
                ui.spacing_mut().interact_size.y = 22.0;
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        theme::eyebrow(ui, "SOURCE");
                        ui.add_space(4.0);
                        let name = self
                            .selected_file
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "No recording loaded".to_owned());
                        let response = ui.add(
                            egui::Label::new(RichText::new(name).strong().size(15.0)).truncate(),
                        );
                        if let Some(path) = &self.selected_file {
                            response.on_hover_text(path.display().to_string());
                        }
                        ui.add_space(8.0);
                        let (rate, channels, length) = self.audio_info.as_ref().map_or(
                            ("—".to_owned(), "—".to_owned(), "—".to_owned()),
                            |clip| {
                                (
                                    format!("{} Hz", clip.sample_rate),
                                    match clip.channels {
                                        1 => "Mono".to_owned(),
                                        2 => "Stereo".to_owned(),
                                        n => format!("{n} channels"),
                                    },
                                    theme::time(clip.duration_seconds),
                                )
                            },
                        );
                        egui::Grid::new("source_specs")
                            .num_columns(2)
                            .min_row_height(16.0)
                            .spacing([16.0, 7.0])
                            .show(ui, |ui| {
                                for (label, value) in [
                                    ("Sample rate", rate),
                                    ("Channels", channels),
                                    ("Duration", length),
                                ] {
                                    ui.label(RichText::new(label).size(12.0).color(theme::MUTED));
                                    ui.label(RichText::new(value).size(12.0));
                                    ui.end_row();
                                }
                            });
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                        theme::eyebrow(ui, "PLAYBACK / EXPORT REGION");
                        if let Some(clip) = &self.audio_info {
                            let frames = clip.samples.len() / usize::from(clip.channels);
                            let range = self.selection.clone().unwrap_or(0..frames);
                            let rate = f64::from(clip.sample_rate);
                            ui.label(
                                RichText::new(if self.selection.is_some() {
                                    "Selection"
                                } else {
                                    "Whole clip"
                                })
                                .color(theme::AMBER)
                                .strong(),
                            );
                            egui::Grid::new("region_specs")
                                .num_columns(2)
                                .min_row_height(16.0)
                                .spacing([26.0, 7.0])
                                .show(ui, |ui| {
                                    for (label, time) in [
                                        ("Start", range.start as f64 / rate),
                                        ("End", range.end as f64 / rate),
                                        ("Length", (range.end - range.start) as f64 / rate),
                                    ] {
                                        ui.label(
                                            RichText::new(label).size(12.0).color(theme::MUTED),
                                        );
                                        ui.label(
                                            RichText::new(theme::time(time)).monospace().size(12.0),
                                        );
                                        ui.end_row();
                                    }
                                });
                        } else {
                            ui.label(RichText::new("Select a WAV to begin.").color(theme::MUTED));
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                        theme::eyebrow(ui, "OUTPUT LEVEL");
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", self.gain * 100.0))
                                    .size(26.0)
                                    .strong(),
                            );
                            let db = if self.gain > 0.0 {
                                format!("{:.1} dB", 20.0 * self.gain.log10())
                            } else {
                                "−∞ dB".to_owned()
                            };
                            ui.label(RichText::new(db).monospace().size(12.0).color(theme::MUTED));
                        });
                        ui.add(
                            egui::Slider::new(&mut self.gain, 0.0..=1.0)
                                .show_value(false)
                                .trailing_fill(true),
                        );
                        if ui
                            .add(theme::icon_button("Reset to unity", theme::Icon::Undo))
                            .clicked()
                        {
                            self.gain = 1.0;
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                        theme::eyebrow(ui, "DISTORTION PREVIEW");
                        audition_changed |= ui
                            .add(theme::toggle(
                                &mut self.distortion_enabled,
                                "Distortion + cab",
                            ))
                            .changed();
                        ui.spacing_mut().slider_width = 65.0;
                        let drive = ui.add(
                            egui::Slider::new(&mut self.distortion_drive, 0.0..=48.0)
                                .text("Drive")
                                .suffix(" dB"),
                        );
                        audition_changed |=
                            drive.drag_stopped() || (drive.changed() && !drive.dragged());
                        ui.label(
                            RichText::new("Audition only · WAV export stays clean")
                                .small()
                                .color(theme::MUTED),
                        );
                    });
            });
        if audition_changed {
            self.rebuild_audition();
            self.reconfigure_playback(false);
        }
    }

    fn marker_toolbar(&mut self, ui: &mut egui::Ui) {
        let visible = self.visible_attacks();
        let rate = f64::from(self.audio_info.as_ref().unwrap().sample_rate);
        let position = (self.position_seconds * rate).round() as usize;
        ui.horizontal(|ui| {
            let previous = visible
                .iter()
                .rev()
                .find(|(_, frame)| *frame < position)
                .copied();
            let next = visible.iter().find(|(_, frame)| *frame > position).copied();
            if ui
                .add_enabled(
                    previous.is_some(),
                    theme::icon_button("Previous", theme::Icon::Previous),
                )
                .clicked()
                && let Some((source, frame)) = previous
            {
                self.jump_to_attack(source, frame);
            }
            if ui
                .add_enabled(
                    next.is_some(),
                    theme::icon_button("Next", theme::Icon::Next),
                )
                .clicked()
                && let Some((source, frame)) = next
            {
                self.jump_to_attack(source, frame);
            }
            if ui
                .add(theme::icon_button("Add at playhead", theme::Icon::Marker))
                .clicked()
            {
                self.add_marker_at_playhead();
            }
            egui::ComboBox::from_id_salt("workspace_attack_picker")
                .selected_text(
                    self.active_attack
                        .and_then(|source| {
                            visible
                                .iter()
                                .position(|&(candidate, _)| candidate == source)
                        })
                        .map_or_else(
                            || "Choose attack".to_owned(),
                            |index| format!("Attack {}", index + 1),
                        ),
                )
                .show_ui(ui, |ui| {
                    for (index, &(source, frame)) in visible.iter().enumerate() {
                        if ui
                            .selectable_label(
                                self.active_attack == Some(source),
                                format!("Attack {}", index + 1),
                            )
                            .clicked()
                        {
                            self.jump_to_attack(source, frame);
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            if ui
                .add(theme::icon_button("Clear", theme::Icon::Clear))
                .clicked()
            {
                self.selected_attack_markers.clear();
                self.active_attack = None;
            }
            if ui
                .add(theme::icon_button("Select all", theme::Icon::Select))
                .clicked()
            {
                self.selected_attack_markers = visible.iter().map(|&(source, _)| source).collect();
                self.active_attack = self.selected_attack_markers.first().copied();
            }
            ui.label(
                RichText::new("Marker selection")
                    .small()
                    .color(theme::MUTED),
            );
            if ui
                .add(theme::icon_button("Remove all", theme::Icon::Delete))
                .on_hover_text("Remove all attack markers; audio is unchanged")
                .clicked()
            {
                self.remove_all_markers();
            }
        });
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        if self.audio_info.is_none() {
            ui.add_space((ui.available_height() * 0.22).max(20.0));
            ui.vertical_centered(|ui| {
                theme::mark(ui, 72.0);
                ui.add_space(18.0);
                ui.label(RichText::new("Forge your sample").size(28.0).strong());
                ui.add_space(3.0);

                ui.add_space(18.0);
                if ui
                    .add_sized([156.0, 42.0], theme::primary("Open a WAV file"))
                    .clicked()
                {
                    self.open_dialog();
                }
                ui.add_space(14.0);
                ui.label(
                    RichText::new("PCM & floating-point WAV")
                        .size(12.0)
                        .color(theme::MUTED),
                );
            });
            return;
        }
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                let name = self
                    .selected_file
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                ui.add(egui::Label::new(RichText::new(name).size(18.0).strong()).truncate());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.selection.is_some(),
                        theme::icon_button("Clear", theme::Icon::Clear),
                    )
                    .clicked()
                {
                    self.set_selection(None);
                }
                if ui
                    .add(theme::icon_button("Select all", theme::Icon::Select))
                    .clicked()
                    && let Some(clip) = &self.audio_info
                {
                    let frames = clip.samples.len() / usize::from(clip.channels);
                    if frames > 0 {
                        self.set_selection(Some(0..frames));
                    }
                }
            });
        });
        if self.workflow == Workflow::Edit {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 30.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} silences · {} cuts",
                            self.edits.current.silences.len(),
                            self.edits.current.deletions.len()
                        ))
                        .small()
                        .color(theme::MUTED),
                    );
                },
            );
        }
        if self.workflow == Workflow::Export {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 30.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add(theme::icon_button("Export WAV…", theme::Icon::Export))
                        .clicked()
                    {
                        self.export();
                    }
                },
            );
        }
        ui.add_space(12.0);
        let progress = self.progress();
        egui::Frame::new()
            .fill(theme::BG)
            .inner_margin(0)
            .show(ui, |ui| {
                match self.workflow {
                    Workflow::Analyze => self.attack_controls(ui),
                    Workflow::Edit => self.edit_controls(ui),
                    Workflow::Export => {
                        theme::eyebrow(ui, "EXPORT REVIEW");
                        ui.label(RichText::new("Your rendered file will include the edits, fades, gain, and selected region.").color(theme::MUTED));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Format").color(theme::MUTED));
                            ui.label(RichText::new("WAV / 32-bit float").strong());

                        });
                    }
                }
            });
        ui.add_space(8.0);
        ui.separator();
        let has_markers = !self.visible_attacks().is_empty();
        let available = ui.available_width();
        if has_markers && available >= 1100.0 {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(available - 490.0 - ui.spacing().item_spacing.x, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(available - 490.0 - ui.spacing().item_spacing.x);
                        self.waveform.controls(
                            ui,
                            self.audio_info.as_ref().unwrap(),
                            self.selection.as_ref(),
                        );
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(490.0, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.marker_toolbar(ui),
                );
            });
        } else {
            self.waveform.controls(
                ui,
                self.audio_info.as_ref().unwrap(),
                self.selection.as_ref(),
            );
            if has_markers {
                self.marker_toolbar(ui);
            }
        }
        self.waveform.timing_targets =
            if self.preview_timing && self.grid_enabled && !self.bypass_edits {
                self.timing_targets()
            } else {
                Vec::new()
            };
        let clip = self
            .audio_info
            .as_ref()
            .expect("editor requires a loaded clip");
        let duration = clip.duration_seconds;
        let attacks = self.visible_attacks();
        self.waveform.markers = attacks.iter().map(|&(_, frame)| frame).collect();
        self.waveform.selected_markers = self
            .selected_attack_markers
            .iter()
            .filter_map(|source| {
                attacks
                    .iter()
                    .find(|&&(candidate, _)| candidate == *source)
                    .map(|&(_, frame)| frame)
            })
            .collect();
        self.waveform.selected_marker = attacks
            .iter()
            .find(|&&(source, _)| Some(source) == self.active_attack)
            .map(|&(_, frame)| frame);
        let action = self.waveform.show(
            ui,
            clip,
            self.gain,
            progress,
            self.selection.as_ref(),
            self.grid_bpm,
            self.grid_subdivision,
            self.grid_enabled,
        );
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            for (keys, action, color) in [
                ("CLICK", "Seek", theme::ACCENT),
                ("DRAG", "Select audio", theme::AMBER),
                ("CTRL + CLICK / DRAG", "Select markers", theme::TEXT),
                ("MIDDLE / RIGHT DRAG", "Pan", theme::MUTED),
                ("SPACE", "Play / stop", theme::ACCENT),
                ("CTRL+C / V", "Copy / paste audio", theme::TEXT),
                ("BACKSPACE", "Delete audio", theme::MUTED),
            ] {
                ui.label(RichText::new(keys).monospace().small().color(color));
                ui.label(RichText::new(action).small().color(theme::MUTED));
                ui.add_space(6.0);
            }
        });
        if let Some(action) = action {
            match action {
                WaveformAction::Seek(fraction) => {
                    self.selection = None;
                    self.selection_pending = true;
                    self.reconfigure_playback(false);
                    let target = fraction * duration;
                    if let Some(playback) = &self.playback {
                        match playback.seek(target) {
                            Ok(position) => self.position_seconds = position,
                            Err(error) => self.error = Some(format!("Could not seek: {error}")),
                        }
                    } else if let Some(clip) = &self.audio_info {
                        let range = self.playback_range();
                        let rate = f64::from(clip.sample_rate);
                        self.position_seconds =
                            target.clamp(range.start as f64 / rate, range.end as f64 / rate);
                    }
                }
                WaveformAction::Select(range) => {
                    self.set_selection(Some(range));
                }
                WaveformAction::SelectMarkers(frames) => {
                    self.selected_attack_markers = attacks
                        .iter()
                        .filter_map(|&(source, frame)| frames.contains(&frame).then_some(source))
                        .collect();
                    self.active_attack = self.selected_attack_markers.first().copied();
                }
                WaveformAction::AddMarkers(frames) => {
                    for source in attacks
                        .iter()
                        .filter_map(|&(source, frame)| frames.contains(&frame).then_some(source))
                    {
                        if !self.selected_attack_markers.contains(&source) {
                            self.selected_attack_markers.push(source);
                        }
                    }
                    self.active_attack = self.selected_attack_markers.first().copied();
                }
            }
            ui.ctx().request_repaint();
        }
    }
}

impl eframe::App for SampleWorkbench {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_playback();
        self.update_detection(ctx);
        self.shortcuts(ctx);
        self.header(ctx);
        self.transport(ctx);
        self.inspector(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(24))
            .show(ctx, |ui| {
                if self.error.is_some() || self.export_status.is_some() {
                    let (message, color) = if let Some(error) = &self.error {
                        (error.clone(), egui::Color32::LIGHT_RED)
                    } else {
                        (
                            self.export_status.clone().unwrap_or_default(),
                            theme::ACCENT,
                        )
                    };
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .inner_margin(12)
                        .corner_radius(6)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if ui.small_button("Dismiss").clicked() {
                                    self.error = None;
                                    self.export_status = None;
                                }
                                ui.label(RichText::new(message).size(12.0).color(color));
                            });
                        });
                    ui.add_space(10.0);
                }
                self.editor(ui);
            });
        // Commit on release: dragging updates the overlay without rebuilding audio every frame.
        if self.selection_pending && !ctx.input(|input| input.pointer.primary_down()) {
            self.reconfigure_playback(self.selection.is_some());
            self.selection_pending = false;
        }
        if let Some(playback) = &self.playback {
            playback.sink.set_volume(self.gain);
            if !playback.sink.is_paused() {
                ctx.request_repaint_after(Duration::from_millis(33));
            }
        }
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    #[test]
    fn add_marker_at_playhead_uses_rendered_position_after_edits() {
        let mut app = app();
        app.position_seconds = 0.6;
        app.add_marker_at_playhead();
        assert_eq!(app.attack_markers, vec![6]);
        assert_eq!(app.selected_attack_markers, vec![6]);
        app.selection = Some(2..4);
        app.delete_selection();
        app.position_seconds = 0.6;
        app.add_marker_at_playhead();
        assert_eq!(app.attack_markers, vec![6, 8]);
    }
    #[test]
    fn distortion_preview_does_not_change_export_clip() {
        let mut app = app();
        let clean = app.audio_info.as_ref().unwrap().samples.clone();
        app.distortion_enabled = true;
        app.rebuild_audition();
        assert_eq!(app.audio_info.as_ref().unwrap().samples, clean);
        assert!(app.audition_audio.is_some());
        app.distortion_enabled = false;
        app.rebuild_audition();
        assert!(app.audition_audio.is_none());
    }
    #[test]
    fn native_clipboard_events_do_not_double_paste() {
        let mut app = app();
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Copy],
                ..Default::default()
            },
            |ctx| app.shortcuts(ctx),
        );
        app.selection = None;
        let _ = ctx.run(
            egui::RawInput {
                events: vec![
                    egui::Event::Key {
                        key: egui::Key::V,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::CTRL,
                    },
                    egui::Event::Paste(String::new()),
                ],
                ..Default::default()
            },
            |ctx| app.shortcuts(ctx),
        );
        assert_eq!(app.audio_info.as_ref().unwrap().samples.len(), 12);
    }
    #[test]
    fn copy_paste_insert_replace_delete_and_undo_preserve_snapshot() {
        let mut app = app();
        app.copy_audio(); // [2, 3]
        app.selection = None;
        app.position_seconds = 0.5;
        app.paste_audio();
        assert_eq!(
            app.audio_info.as_ref().unwrap().samples.as_ref(),
            &[0., 1., 2., 3., 4., 2., 3., 5., 6., 7., 8., 9.]
        );
        assert_eq!(app.selection, Some(5..7));
        app.paste_audio(); // replacement, not another insertion
        assert_eq!(app.audio_info.as_ref().unwrap().samples.len(), 12);
        app.delete_selection();
        assert_eq!(
            app.audio_info.as_ref().unwrap().samples.as_ref(),
            &[0., 1., 2., 3., 4., 5., 6., 7., 8., 9.]
        );
        app.undo_edit();
        assert_eq!(app.audio_info.as_ref().unwrap().samples.len(), 12);
        app.selection = Some(0..12);
        app.delete_selection();
        assert!(app.audio_info.as_ref().unwrap().samples.is_empty());
        app.paste_audio();
        assert_eq!(app.audio_info.as_ref().unwrap().samples.as_ref(), &[2., 3.]);
    }

    #[test]
    fn removing_all_markers_preserves_audio_and_selection() {
        let mut app = app();
        app.attack_markers = vec![1, 3];
        app.selected_attack_markers = vec![1];
        let audio = app.audio_info.as_ref().unwrap().samples.clone();
        app.remove_all_markers();
        assert!(app.attack_markers.is_empty());
        assert!(app.selected_attack_markers.is_empty());
        assert_eq!(app.selection, Some(2..4));
        assert_eq!(app.audio_info.as_ref().unwrap().samples, audio);
    }
    #[test]
    fn timing_uses_rendered_grid_preserves_markers_loop_and_bypass() {
        let mut app = app();
        app.attack_markers = vec![1, 6];
        app.delete_selection();
        app.selected_attack_markers = vec![6];
        app.grid_enabled = true;
        app.preview_timing = true;
        app.looping = true;
        app.selection = Some(0..7);
        let before = app.audio_info.as_ref().unwrap().samples.clone();
        assert_eq!(app.timing_targets(), vec![(1, 1), (4, 5)]);
        assert_eq!(app.audio_info.as_ref().unwrap().samples, before);
        app.apply_timing();
        assert_eq!(app.attack_markers, vec![1, 6]);
        assert_eq!(app.visible_attacks(), vec![(1, 1), (6, 5)]);
        assert_eq!(app.audio_info.as_ref().unwrap().samples[5], 6.0);
        assert_eq!(app.selection, Some(0..7));
        assert!(app.looping);
        app.undo_edit();
        assert_eq!(app.audio_info.as_ref().unwrap().samples, before);
        app.redo_edit();
        assert_eq!(app.visible_attacks(), vec![(1, 1), (6, 5)]);
        app.bypass_edits = true;
        app.refresh_edits();
        assert_eq!(
            app.audio_info.as_ref().unwrap().samples,
            app.original_audio.as_ref().unwrap().samples
        );
    }
    #[test]
    fn attack_markers_follow_cuts_and_return_on_undo() {
        let mut app = app();
        app.attack_markers = vec![1, 3, 7];
        app.delete_selection();
        assert_eq!(app.visible_attacks(), vec![(1, 1), (7, 5)]);
        assert_eq!(app.source_frame(5), Some(7));
        app.edits.undo();
        app.refresh_edits();
        assert_eq!(app.visible_attacks(), vec![(1, 1), (3, 3), (7, 7)]);
    }

    fn app() -> SampleWorkbench {
        let clip = AudioClip {
            channels: 1,
            sample_rate: 10,
            duration_seconds: 1.0,
            samples: (0..10).map(|n| n as f32).collect::<Vec<_>>().into(),
        };
        SampleWorkbench {
            original_audio: Some(clip.clone()),
            audio_info: Some(clip),
            selection: Some(2..4),
            ..Default::default()
        }
    }
    fn key(app: &mut SampleWorkbench, key: egui::Key, repeat: bool, focused: bool) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            // egui derives repeats from prior frames; inject the already-classified
            // event here to exercise the app's repeat guard independently.
            ctx.input_mut(|i| {
                for event in &mut i.events {
                    if let egui::Event::Key { repeat: actual, .. } = event {
                        *actual = repeat;
                    }
                }
            });
            if focused {
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("text")));
            }
            app.shortcuts(ctx);
        });
    }
    #[test]
    fn backspace_closes_gap_undo_restores_and_escape_only_clears_selection() {
        let mut app = app();
        key(&mut app, egui::Key::Backspace, false, false);
        assert_eq!(
            app.audio_info.as_ref().unwrap().samples.as_ref(),
            &[0., 1., 4., 5., 6., 7., 8., 9.]
        );
        assert!(app.selection.is_none());
        assert_eq!(app.position_seconds, 0.2);
        app.edits.undo();
        app.refresh_edits();
        assert_eq!(app.audio_info.as_ref().unwrap().samples.len(), 10);
        app.edits.redo();
        app.refresh_edits();
        assert_eq!(app.audio_info.as_ref().unwrap().samples.len(), 8);
        app.selection = Some(1..3);
        key(&mut app, egui::Key::Escape, false, false);
        assert!(app.selection.is_none());
        assert_eq!(app.audio_info.as_ref().unwrap().samples.len(), 8);
    }
    #[test]
    fn repeated_or_text_editing_backspace_does_not_delete() {
        let mut app = app();
        key(&mut app, egui::Key::Backspace, true, false);
        key(&mut app, egui::Key::Backspace, false, true);
        assert_eq!(app.audio_info.as_ref().unwrap().samples.len(), 10);
    }
}
