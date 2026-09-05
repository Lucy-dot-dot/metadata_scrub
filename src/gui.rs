use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use eframe::egui;
use egui::{Color32, Vec2};

use crate::scrub::{self, Job, Outcome};

#[derive(Clone, Copy, PartialEq)]
enum LineKind {
    Scrubbed,
    Clean,
    Skipped,
    Failed,
    Info,
}

enum Event {
    Total(usize),
    Progress { line: String, kind: LineKind },
    Finished { stopped: bool, summary: String },
    Fatal(String),
}

pub fn run() -> anyhow::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_256.png"))
        .expect("failed to load window icon");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Metadata Scrub")
            .with_inner_size(Vec2::new(780.0, 560.0))
            .with_icon(icon)
            .with_app_id("metadata_scrub"),
        ..Default::default()
    };
    eframe::run_native("Metadata Scrub", options, Box::new(|_cc| Ok(Box::new(ScrubApp::default()))))?;
    Ok(())
}

#[derive(Default)]
struct ScrubApp {
    path: String,
    output_dir: String,
    recursive: bool,
    backup: bool,
    dry_run: bool,

    running: bool,
    stop: Option<std::sync::Arc<AtomicBool>>,
    rx: Option<Receiver<Event>>,
    total: usize,
    done: usize,

    log: Vec<(LineKind, String)>,
    status: String,
}

fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    let pad = ui.spacing().button_padding;
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font_id, Color32::WHITE);
    galley.size().x + 2.0 * pad.x
}

impl ScrubApp {
    fn push_log(&mut self, kind: LineKind, line: impl Into<String>) {
        self.log.push((kind, line.into()));
        if self.log.len() > 1000 {
            self.log.drain(..self.log.len() - 1000);
        }
    }

    fn start(&mut self) {
        let root = PathBuf::from(self.path.trim().to_string());
        if self.path.trim().is_empty() {
            self.status = "Choose a file or folder first".to_string();
            return;
        }
        let job = Job {
            recursive: self.recursive,
            dry_run: self.dry_run,
            backup: self.backup,
            output_dir: {
                let t = self.output_dir.trim();
                (!t.is_empty()).then(|| PathBuf::from(t.to_string()))
            },
        };

        let (tx, rx) = channel::<Event>();
        let stop = std::sync::Arc::new(AtomicBool::new(false));

        self.rx = Some(rx);
        self.stop = Some(stop.clone());
        self.running = true;
        self.done = 0;
        self.total = 0;
        self.status = "Working...".to_string();
        self.push_log(LineKind::Info, format!("Scanning {}", root.display()));

        std::thread::spawn(move || {            let files = match scrub::collect_files(&root, job.recursive) {
                Ok(files) => files,
                Err(err) => {
                    let _ = tx.send(Event::Fatal(format!("{err:#}")));
                    return;
                }
            };
            let _ = tx.send(Event::Total(files.len()));

            let result = scrub::run_job(&root, &job, Some(&stop), |path, res| {
                let name = path.display().to_string();
                let (line, kind) = match res {
                    Ok(outcome @ Outcome::Scrubbed { .. }) => {
                        (format!("scrubbed  {name}  ({})", outcome.label()), LineKind::Scrubbed)
                    }
                    Ok(outcome @ Outcome::Clean { .. }) => {
                        (format!("clean     {name}  ({})", outcome.label()), LineKind::Clean)
                    }
                    Ok(outcome @ Outcome::Unsupported { .. }) => {
                        (format!("skipped   {name}  ({})", outcome.label()), LineKind::Skipped)
                    }
                    Err(err) => (format!("failed    {name}  ({err:#})"), LineKind::Failed),
                };
                let _ = tx.send(Event::Progress { line, kind });
            });

            match result {
                Ok(stats) => {
                    let mut summary = format!(
                        "{} scrubbed, {} already clean, {} unsupported, {} failed",
                        stats.scrubbed, stats.clean, stats.unsupported, stats.failed
                    );
                    if stats.stopped {
                        summary = format!("stopped early - {summary}");
                    }
                    let _ = tx.send(Event::Finished { stopped: stats.stopped, summary });
                }
                Err(err) => {
                    let _ = tx.send(Event::Fatal(format!("{err:#}")));
                }
            }
        });
    }
}

impl eframe::App for ScrubApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(rx) = self.rx.take() {
            let mut repaint = false;
            let mut finished = false;
            while let Ok(event) = rx.try_recv() {
                repaint = true;
                match event {
                    Event::Total(n) => {
                        self.total = n;
                        self.push_log(LineKind::Info, format!("{n} supported file(s) found"));
                    }
                    Event::Progress { line, kind } => {
                        self.done += 1;
                        self.push_log(kind, line);
                    }
                    Event::Finished { stopped, summary } => {
                        finished = true;
                        self.running = false;
                        self.stop = None;
                        self.status = if stopped { "Stopped" } else { "Done" }.to_string();
                        self.push_log(LineKind::Info, summary);
                    }
                    Event::Fatal(msg) => {
                        finished = true;
                        self.running = false;
                        self.stop = None;
                        self.status = "Failed".to_string();
                        self.push_log(LineKind::Failed, msg);
                    }
                }
            }
            if !finished {
                self.rx = Some(rx);
            }
            if repaint || self.running {
                ctx.request_repaint_after(Duration::from_millis(120));
            }
        }

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if self.running {
                    ui.spinner();
                }
                ui.label(&self.status);
                if self.total > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{} / {} files", self.done, self.total));
                    });
                }
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Metadata Scrub");
            ui.add_space(6.0);
            ui.label("Strip EXIF, XMP, IPTC and QuickTime metadata from photos and videos.");

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Input:");
                let buttons_w = button_width(ui, "File...")
                    + button_width(ui, "Folder...")
                    + 2.0 * ui.spacing().item_spacing.x;
                ui.add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text("Path to a file or folder")
                        .desired_width((ui.available_width() - buttons_w).max(0.0)),
                );
                ui.add_enabled_ui(!self.running, |ui| {
                    if ui.button("File...").clicked()
                        && let Some(file) = rfd::FileDialog::new().pick_file()
                    {
                        self.path = file.display().to_string();
                    }
                    if ui.button("Folder...").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                    {
                        self.path = folder.display().to_string();
                    }
                });
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Output:");
                let buttons_w = button_width(ui, "Browse...") + ui.spacing().item_spacing.x;
                ui.add(
                    egui::TextEdit::singleline(&mut self.output_dir)
                        .hint_text("Optional: write cleaned copies here instead of scrubbing in place")
                        .desired_width((ui.available_width() - buttons_w).max(0.0)),
                );
                ui.add_enabled_ui(!self.running, |ui| {
                    if ui.button("Browse...").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                    {
                        self.output_dir = folder.display().to_string();
                    }
                });
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_enabled(!self.running, |ui: &mut egui::Ui| {
                    ui.checkbox(&mut self.recursive, "Recurse into subfolders")
                });
                ui.add_enabled(!self.running, |ui: &mut egui::Ui| {
                    ui.checkbox(&mut self.backup, "Keep .bak backups")
                });
                ui.add_enabled(!self.running, |ui: &mut egui::Ui| {
                    ui.checkbox(&mut self.dry_run, "Dry run")
                });
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let scrub_btn = ui.add_enabled(
                    !self.running && !self.path.trim().is_empty(),
                    egui::Button::new(egui::RichText::new("  Scrub  ").strong()),
                );
                if scrub_btn.clicked() {
                    self.start();
                }
                if self.running
                    && ui.button("Stop").clicked()
                    && let Some(stop) = &self.stop
                {
                    stop.store(true, Ordering::Relaxed);
                }
                if self.running {
                    ui.add(
                        egui::ProgressBar::new(if self.total > 0 {
                            self.done as f32 / self.total as f32
                        } else {
                            0.0
                        })
                        .animate(true)
                        .desired_width(240.0),
                    );
                }
            });

            ui.add_space(4.0);
            if self.output_dir.trim().is_empty() {
                ui.label(
                    egui::RichText::new("Files are modified in place. Set an output folder to keep originals untouched.")
                        .weak()
                        .small(),
                );
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                for (kind, line) in &self.log {
                    let color = match kind {
                        LineKind::Scrubbed => Color32::from_rgb(140, 220, 140),
                        LineKind::Clean => ui.visuals().text_color(),
                        LineKind::Skipped => Color32::from_rgb(230, 200, 100),
                        LineKind::Failed => Color32::from_rgb(235, 110, 110),
                        LineKind::Info => ui.visuals().weak_text_color(),
                    };
                    ui.colored_label(color, egui::RichText::new(line).monospace());
                }
            });
        });
    }
}
