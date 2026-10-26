use crate::{
    activity::Journal,
    config::{Appearance, Config, GpuProfile, Palette, ThemeMode},
    lifecycle::{Lifecycle, Work},
    miner::{self, MinerEvent, Stats},
    notifications::Notifications,
    pool::{MiningMode, Pool},
    tray::{self, Tray},
    updater::{self, Prepared, Release},
};
use anyhow::{Context, Result};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use std::{
    collections::VecDeque,
    path::PathBuf,
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

pub fn run(config: Config, path: PathBuf) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Solo Miner")
            .with_inner_size([960.0, 580.0])
            .with_min_inner_size([880.0, 580.0])
            .with_icon(egui::IconData {
                rgba: crate::icon::logo_rgba(64),
                width: 64,
                height: 64,
            })
            .with_decorations(false),
        centered: true,
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "Solo Miner",
        options,
        Box::new(move |cc| Ok(Box::new(Desktop::new(cc, config, path)?))),
    )
    .map_err(|e| anyhow::anyhow!("Interface: {e}"))
}
#[derive(Clone, Copy, PartialEq)]
enum Page {
    Overview,
    Mining,
    Appearance,
    Activity,
    Updates,
}
enum Event {
    Stats(Stats),
    Finished(Option<String>),
    Notification(String),
    UpdateChecked(std::result::Result<Option<Release>, String>),
    UpdateProgress(u64, u64),
    UpdatePrepared(std::result::Result<Box<Prepared>, String>),
}
#[derive(Default, PartialEq)]
enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    Downloading,
    Ready,
    Waiting,
}
#[derive(Default)]
struct UpdateUi {
    phase: UpdatePhase,
    release: Option<Release>,
    prepared: Option<Prepared>,
    status: String,
    downloaded: u64,
    total: u64,
    check_at_startup: bool,
    resume: bool,
}
struct Desktop {
    config: Config,
    draft: Config,
    difficulty: String,
    path: PathBuf,
    page: Page,
    context: egui::Context,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    tray: Tray,
    notifications: Notifications,
    notification_status: String,
    lifecycle: Lifecycle,
    worker: Option<miner::Handle>,
    stats: Stats,
    launched: Instant,
    session_start: Option<Instant>,
    history: VecDeque<(f64, f64)>,
    graph_seconds: f64,
    journal: Journal,
    logo: egui::TextureHandle,
    message: String,
    benchmark_result: Option<(u8, GpuProfile, f64)>,
    benchmark_load: u8,
    benchmark_profile: GpuProfile,
    quitting: bool,
    updates: UpdateUi,
}
impl Desktop {
    fn new(cc: &eframe::CreationContext<'_>, config: Config, path: PathBuf) -> Result<Self> {
        let context = cc.egui_ctx.clone();
        let (tx, rx) = mpsc::channel();
        let wake = context.clone();
        let tray = Tray::new(move || wake.request_repaint())?;
        let sender = tx.clone();
        let wake = context.clone();
        let notifications = Notifications::new(move |text| {
            let _ = sender.send(Event::Notification(text));
            wake.request_repaint();
        });
        if config.notifications.enabled() {
            notifications.authorize();
        }
        let page = if std::env::args().any(|a| a == "--options") {
            Page::Mining
        } else {
            Page::Overview
        };
        let journal = Journal::open(
            path.with_file_name("activity.jsonl"),
            config.log_retention_days,
        )?;
        let logo = context.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &crate::icon::logo_rgba(64)),
            Default::default(),
        );
        let mut app = Self {
            difficulty: config.share_difficulty.to_string(),
            draft: config.clone(),
            config,
            path: path.clone(),
            page,
            context,
            tx,
            rx,
            tray,
            notifications,
            notification_status: "".into(),
            lifecycle: Lifecycle::default(),
            worker: None,
            stats: Stats {
                state: "Idle".into(),
                ..Default::default()
            },
            launched: Instant::now(),
            session_start: None,
            history: VecDeque::new(),
            graph_seconds: 300.0,
            journal,
            logo,
            message: String::new(),
            benchmark_result: None,
            benchmark_load: 25,
            benchmark_profile: GpuProfile::default(),
            quitting: false,
            updates: UpdateUi {
                check_at_startup: std::fs::read(path.with_file_name("updates.json"))
                    .ok()
                    .and_then(|v| serde_json::from_slice::<bool>(&v).ok())
                    .unwrap_or(true),
                ..Default::default()
            },
        };
        app.log("App", concat!("Solo Miner ", env!("CARGO_PKG_VERSION")));
        if std::env::args().any(|a| a == "--resume-mining") {
            app.start();
        }
        updater::confirm_launch()?;
        if let Ok(result) = std::fs::read_to_string(app.path.with_file_name("update-result.txt")) {
            app.log("Update", &result);
            app.updates.status = result;
            let _ = std::fs::remove_file(app.path.with_file_name("update-result.txt"));
        }
        if app.updates.check_at_startup {
            app.check_updates();
        }
        Ok(app)
    }
    fn log(&mut self, kind: &str, text: impl Into<String>) {
        if let Err(e) = self.journal.append(kind, text.into()) {
            self.message = format!("Log: {e:#}");
        }
    }

    fn error(&mut self, text: String) {
        self.message = text.clone();
        self.log("Error", &text);
        self.notifications
            .event(&MinerEvent::Error(text), &self.config.notifications);
    }
    fn launch(&mut self, kind: Work, config: Config) {
        if self.worker.is_some() {
            return;
        }
        self.message.clear();
        self.lifecycle.launched(kind);
        let sender = self.tx.clone();
        let wake = self.context.clone();
        let finished = self.tx.clone();
        let done = self.context.clone();
        self.worker = Some(miner::spawn(
            config,
            kind == Work::Benchmark,
            move |stats| {
                let _ = sender.send(Event::Stats(stats));
                wake.request_repaint();
            },
            move |r| {
                let _ = finished.send(Event::Finished(r.err().map(|e| format!("{e:#}"))));
                done.request_repaint();
            },
        ));
        self.stats = Stats {
            state: if kind == Work::Mining {
                "Starting mining…"
            } else {
                "Running benchmark…"
            }
            .into(),
            ..Default::default()
        };
        if kind == Work::Mining {
            self.session_start = Some(Instant::now());
        }
        self.log(
            "Session",
            if kind == Work::Mining {
                "GPU mining started"
            } else {
                "Local GPU benchmark, 3 seconds"
            },
        );
    }
    fn start(&mut self) {
        if self.updates.phase == UpdatePhase::Waiting {
            return;
        }
        if let Err(e) = self
            .config
            .validate()
            .and_then(|_| self.config.mining_username().map(|_| ()))
        {
            self.error(format!("{e:#}"));
            self.page = Page::Mining;
            return;
        }
        self.lifecycle.start();
        if self.worker.is_none() {
            self.launch(Work::Mining, self.config.clone());
        } else if self.lifecycle.work == Some(Work::Benchmark) {
            self.lifecycle.stopping = true;
            if let Some(h) = &self.worker {
                h.stop();
            }
        }
    }
    fn stop(&mut self) {
        self.lifecycle.stop();
        if let Some(h) = &self.worker {
            h.stop();
            self.stats.state = "Stopping…".into();
        }
        self.message = "Stop requested".into();
    }
    fn read_draft(&self) -> Result<Config> {
        let mut next = self.draft.clone();
        next.bitcoin_address = next.bitcoin_address.trim().into();
        next.braiins_account = next.braiins_account.trim().into();
        next.f2pool_account = next.f2pool_account.trim().into();
        next.share_difficulty = self
            .difficulty
            .trim()
            .replace(',', ".")
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a numeric share difficulty"))?;
        next.validate()?;
        Ok(next)
    }
    fn save(&mut self, appearance_only: bool) {
        if self.updates.phase == UpdatePhase::Waiting {
            return;
        }
        let result = if appearance_only {
            let mut next = self.config.clone();
            next.appearance = self.draft.appearance.clone();
            Ok(next)
        } else {
            self.read_draft()
        };
        let next = match result {
            Ok(c) => c,
            Err(e) => {
                self.error(format!("{e:#}"));
                return;
            }
        };
        // Keep the current worker running if validation or saving fails.
        if let Err(e) = next.save(&self.path) {
            self.error(format!("Save failed: {e:#}"));
            return;
        }
        let changed = self.config.mining_changed(&next);
        let notices_changed = self.config.notifications != next.notifications;
        self.config = next;
        if !appearance_only {
            self.draft = self.config.clone();
            self.difficulty = self.config.share_difficulty.to_string();
        }
        if notices_changed {
            if self.config.notifications.enabled() {
                self.notifications.authorize();
            } else {
                self.notification_status = "Notifications disabled in app".into();
            }
        }
        if self.lifecycle.reconfigure(changed) {
            if let Some(h) = &self.worker {
                h.stop();
            }
            self.stats.state = "Restarting…".into();
            self.message = "Settings saved · restarting…".into();
            self.log("Settings", "Restarting with new settings");
        } else {
            self.message = if appearance_only {
                "Appearance saved"
            } else {
                "Settings saved"
            }
            .into();
            self.log("Settings", self.message.clone());
        }
    }
    fn benchmark(&mut self) {
        if self.updates.phase == UpdatePhase::Waiting {
            return;
        }
        if self.worker.is_some() {
            return;
        }
        match self.read_draft() {
            Ok(config) => {
                self.benchmark_load = config.gpu_load;
                self.benchmark_profile = config.gpu_profile;
                self.benchmark_result = None;
                self.launch(Work::Benchmark, config);
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }
    fn account(&mut self) {
        if let Err(e) = self
            .config
            .pool
            .account_url(&self.config.bitcoin_address)
            .and_then(|url| Ok(Command::new("/usr/bin/open").arg(url).spawn()?))
        {
            self.error(format!("Open pool account: {e:#}"));
        }
    }
    fn show(&self) {
        for command in [
            egui::ViewportCommand::Visible(true),
            egui::ViewportCommand::Minimized(false),
            egui::ViewportCommand::Focus,
        ] {
            self.context.send_viewport_cmd(command);
        }
        self.context.request_repaint();
    }
    fn quit(&mut self) {
        if self.updates.phase == UpdatePhase::Waiting {
            return;
        }
        self.quitting = true;
        self.lifecycle.stop();
        self.worker.take();
        self.context.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    fn receive(&mut self) {
        while let Some(action) = self.tray.poll() {
            match action {
                tray::Action::Show => {
                    self.show();
                    self.page = Page::Overview;
                }
                tray::Action::Settings => {
                    self.show();
                    self.page = Page::Mining;
                }
                tray::Action::Start => self.start(),
                tray::Action::Stop => self.stop(),
                tray::Action::Account => self.account(),
                tray::Action::Quit => self.quit(),
            }
        }
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::UpdateChecked(result) => {
                    self.updates.phase = UpdatePhase::Idle;
                    match result {
                        Ok(release) => {
                            self.updates.status = release.as_ref().map_or_else(
                                || "Up to date".into(),
                                |r| format!("Version {} available", r.version),
                            );
                            self.updates.release = release;
                        }
                        Err(error) => self.updates.status = error,
                    }
                }
                Event::UpdateProgress(downloaded, total) => {
                    self.updates.downloaded = downloaded;
                    self.updates.total = total;
                }
                Event::UpdatePrepared(result) => match result {
                    Ok(prepared) => {
                        self.updates.prepared = Some(*prepared);
                        self.updates.phase = UpdatePhase::Ready;
                        self.updates.status = "Ready to install".into();
                    }
                    Err(error) => {
                        self.updates.phase = UpdatePhase::Idle;
                        self.updates.status = error;
                    }
                },
                Event::Notification(text) => {
                    self.notification_status = text;
                }
                Event::Stats(stats) => {
                    if let Some(event) = &stats.event {
                        self.notifications.event(event, &self.config.notifications);
                        let (kind, text) = match event {
                            MinerEvent::AcceptedShare => ("Share", "Share accepted".into()),
                            MinerEvent::BlockCandidate => ("Block", "Block candidates".into()),
                            MinerEvent::Error(e) => ("Error", e.clone()),
                        };
                        self.log(kind, text);
                    }
                    if self.lifecycle.work == Some(Work::Benchmark) && stats.hashes_per_second > 0.0
                    {
                        self.benchmark_result = Some((
                            self.benchmark_load,
                            self.benchmark_profile,
                            stats.hashes_per_second,
                        ));
                    }
                    if self.lifecycle.work == Some(Work::Mining) {
                        let now = self.launched.elapsed().as_secs_f64();
                        if self.history.back().is_none_or(|(t, _)| now - t >= 0.8) {
                            self.history.push_back((now, stats.hashes_per_second));
                        }
                        while self.history.front().is_some_and(|(t, _)| now - t > 900.0) {
                            self.history.pop_front();
                        }
                    }
                    if !self.lifecycle.stopping {
                        self.stats = stats;
                    } else {
                        self.stats.gpu = stats.gpu;
                    }
                }
                Event::Finished(error) => {
                    self.worker.take();
                    let was = self.lifecycle.work;
                    let restart = self.lifecycle.finished(error.is_some());
                    if let Some(e) = error {
                        self.error(e);
                    }
                    self.stats.hashes_per_second = 0.0;
                    self.stats.network_difficulty = None;
                    self.stats.share_difficulty = None;
                    self.stats.state = if was == Some(Work::Benchmark) {
                        "Benchmark finished"
                    } else {
                        "Mining stopped"
                    }
                    .into();
                    self.log("Session", self.stats.state.clone());
                    if was == Some(Work::Mining) {
                        self.history
                            .push_back((self.launched.elapsed().as_secs_f64(), 0.0));
                    }
                    if restart {
                        self.launch(Work::Mining, self.config.clone());
                        self.message = "Mining restarted".into();
                    }
                }
            }
        }
        self.tray
            .update(&self.config, &self.stats, self.worker.is_some());
        if self.updates.phase == UpdatePhase::Waiting && self.worker.is_none() {
            self.finish_update();
        }
    }
    fn updates_page(&mut self, ui: &mut egui::Ui, c: Colors) {
        card(c).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(concat!("Version ", env!("CARGO_PKG_VERSION")))
                        .strong()
                        .size(18.0),
                );
                if ui
                    .add_enabled(
                        self.updates.phase == UpdatePhase::Idle,
                        egui::Button::new("Check for updates"),
                    )
                    .clicked()
                {
                    self.check_updates();
                }
            });
            ui.add_space(8.0);
            if ui
                .checkbox(&mut self.updates.check_at_startup, "Check at startup")
                .changed()
            {
                let saved = (|| -> Result<()> {
                    use std::io::Write;
                    let mut file = tempfile::NamedTempFile::new_in(
                        self.path.parent().context("Missing settings folder")?,
                    )?;
                    file.write_all(
                        serde_json::to_string(&self.updates.check_at_startup)?.as_bytes(),
                    )?;
                    file.as_file().sync_all()?;
                    file.persist(self.path.with_file_name("updates.json"))?;
                    Ok(())
                })();
                if let Err(error) = saved {
                    self.updates.status = format!("Save failed: {error:#}");
                }
            }
            ui.add_space(8.0);
            ui.label(&self.updates.status);
            if self.updates.phase == UpdatePhase::Checking {
                ui.spinner();
            }
            if self.updates.phase == UpdatePhase::Downloading {
                let progress = if self.updates.total > 0 {
                    self.updates.downloaded as f32 / self.updates.total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(progress)
                        .show_percentage()
                        .animate(true),
                );
            }
            if let Some(release) = &self.updates.release {
                let page = release.page.clone();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if self.updates.phase == UpdatePhase::Idle
                        && ui.button("Download update").clicked()
                    {
                        self.download_update();
                    }
                    if self.updates.phase == UpdatePhase::Ready
                        && ui.button("Install and restart").clicked()
                    {
                        self.install_update();
                    }
                    ui.hyperlink_to("Release notes", page);
                });
            }
        });
    }
    fn check_updates(&mut self) {
        if self.updates.phase != UpdatePhase::Idle {
            return;
        }
        self.updates.phase = UpdatePhase::Checking;
        self.updates.status = "Checking GitHub…".into();
        let sender = self.tx.clone();
        let wake = self.context.clone();
        std::thread::spawn(move || {
            let result = updater::check().map_err(|e| format!("Check failed: {e:#}"));
            let _ = sender.send(Event::UpdateChecked(result));
            wake.request_repaint();
        });
    }
    fn download_update(&mut self) {
        let Some(release) = self.updates.release.clone() else {
            return;
        };
        if self.updates.phase != UpdatePhase::Idle {
            return;
        }
        self.updates.phase = UpdatePhase::Downloading;
        self.updates.downloaded = 0;
        self.updates.total = 0;
        self.updates.status = "Downloading and verifying…".into();
        let sender = self.tx.clone();
        let wake = self.context.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now();
            let result = updater::prepare(release, |downloaded, total| {
                if last.elapsed() >= Duration::from_millis(100) || downloaded == total {
                    let _ = sender.send(Event::UpdateProgress(downloaded, total));
                    wake.request_repaint();
                    last = Instant::now();
                }
            })
            .map(Box::new)
            .map_err(|e| format!("Download failed: {e:#}"));
            let _ = sender.send(Event::UpdatePrepared(result));
            wake.request_repaint();
        });
    }
    fn install_update(&mut self) {
        if self.updates.phase != UpdatePhase::Ready {
            return;
        }
        self.updates.resume =
            self.lifecycle.want_mining && self.lifecycle.work == Some(Work::Mining);
        self.updates.phase = UpdatePhase::Waiting;
        self.updates.status = "Stopping mining before update…".into();
        self.stop();
        self.context.request_repaint();
    }
    fn finish_update(&mut self) {
        let Some(prepared) = self.updates.prepared.take() else {
            return;
        };
        match prepared.launch(
            self.updates.resume,
            self.path.with_file_name("update-result.txt"),
        ) {
            Ok(()) => {
                self.log("Update", "Installing update");
                self.quitting = true;
                self.context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(error) => {
                self.updates.phase = UpdatePhase::Idle;
                self.updates.status = format!("Install failed: {error:#}");
                if self.updates.resume {
                    self.start();
                }
            }
        }
    }
}
#[derive(Clone, Copy)]
struct Colors {
    bg: Color32,
    side: Color32,
    card: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    accent_text: Color32,
    dark: bool,
}
fn colors(appearance: &Appearance, system: Option<egui::Theme>) -> Colors {
    let dark = match appearance.mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => system != Some(egui::Theme::Light),
    };
    let rgb = |v: [u8; 3]| Color32::from_rgb(v[0], v[1], v[2]);
    let (bg, side, card, border) = match (dark, appearance.palette) {
        (true, Palette::Graphite) => ([18, 21, 27], [13, 16, 21], [25, 29, 37], [43, 49, 60]),
        (true, Palette::Ocean) => ([13, 23, 34], [9, 17, 26], [19, 34, 49], [34, 56, 75]),
        (true, Palette::Forest) => ([16, 25, 23], [10, 18, 17], [23, 35, 31], [40, 58, 51]),
        (false, Palette::Graphite) => (
            [243, 245, 249],
            [233, 236, 242],
            [255, 255, 255],
            [216, 222, 232],
        ),
        (false, Palette::Ocean) => (
            [239, 246, 251],
            [226, 237, 246],
            [255, 255, 255],
            [207, 222, 235],
        ),
        (false, Palette::Forest) => (
            [240, 247, 242],
            [226, 238, 230],
            [255, 255, 255],
            [209, 226, 215],
        ),
    };
    Colors {
        bg: rgb(bg),
        side: rgb(side),
        card: rgb(card),
        border: rgb(border),
        text: if dark {
            rgb([232, 237, 245])
        } else {
            rgb([24, 34, 48])
        },
        muted: if dark {
            rgb([146, 160, 180])
        } else {
            rgb([93, 108, 129])
        },
        accent: rgb(appearance.accent),
        accent_text: if dark {
            rgb(appearance.accent)
        } else {
            rgb(appearance.accent).gamma_multiply(0.55)
        },
        dark,
    }
}
fn card(c: Colors) -> egui::Frame {
    egui::Frame::new()
        .fill(c.card)
        .stroke(Stroke::new(1.0_f32, c.border))
        .corner_radius(10)
        .inner_margin(12)
}
fn note(ui: &mut egui::Ui, text: impl Into<String>, c: Colors) {
    ui.label(RichText::new(text).size(12.0).color(c.muted));
}
fn metric(ui: &mut egui::Ui, c: Colors, label: &str, value: &str, detail: &str) {
    card(c).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        note(ui, label, c);
        ui.add_space(4.0);
        ui.set_min_height(70.0);
        ui.label(RichText::new(value).size(23.0).strong());
        if !detail.is_empty() {
            ui.add_space(6.0);
            note(ui, detail, c);
        }
    });
}
impl Desktop {
    fn style(&self, ctx: &egui::Context) -> Colors {
        let c = colors(&self.draft.appearance, ctx.system_theme());
        ctx.set_theme(match self.draft.appearance.mode {
            ThemeMode::Dark => egui::ThemePreference::Dark,
            ThemeMode::Light => egui::ThemePreference::Light,
            ThemeMode::System => egui::ThemePreference::System,
        });
        let mut style = (*ctx.style()).clone();
        style.visuals = if c.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        style.visuals.panel_fill = c.bg;
        style.visuals.window_fill = c.card;
        style.visuals.extreme_bg_color = c.side;
        style.visuals.override_text_color = Some(c.text);
        style.visuals.selection.bg_fill = c.accent.gamma_multiply(0.3);
        style.visuals.selection.stroke = Stroke::new(1.0_f32, c.accent);
        style.visuals.widgets.inactive.bg_fill = c.side;
        style.visuals.widgets.inactive.weak_bg_fill = c.side;
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, c.border);
        style.visuals.widgets.hovered.bg_fill =
            c.accent.gamma_multiply(if c.dark { 0.24 } else { 0.8 });
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, c.accent);
        style.visuals.widgets.active.bg_fill = c.accent.gamma_multiply(0.35);
        style.visuals.hyperlink_color = c.accent;
        style.spacing.item_spacing = Vec2::new(8.0, 7.0);
        style.spacing.button_padding = Vec2::new(10.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        ctx.set_style(style);
        c
    }
    fn titlebar(&mut self, ctx: &egui::Context, c: Colors) {
        egui::TopBottomPanel::top("titlebar")
            .exact_height(36.0)
            .frame(
                egui::Frame::new()
                    .fill(c.side)
                    .inner_margin(egui::Margin::symmetric(12, 3)),
            )
            .show(ctx, |ui| {
                let full = ui.max_rect().expand2(Vec2::new(12.0, 3.0));
                let controls =
                    egui::Rect::from_min_max(egui::pos2(full.right() - 76.0, full.top()), full.max);
                let drag =
                    egui::Rect::from_min_max(full.min, egui::pos2(controls.left(), full.bottom()));
                let response = ui.interact(
                    drag,
                    ui.id().with("drag_titlebar"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(concat!("SOLO MINER / ", env!("CARGO_PKG_VERSION")))
                            .size(11.0)
                            .color(c.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").on_hover_text("Hide").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        }
                        if ui.button("−").on_hover_text("Minimize").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                });
            });
    }
    fn sidebar(&mut self, ctx: &egui::Context, c: Colors) {
        egui::SidePanel::left("navigation")
            .exact_width(170.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(c.side).inner_margin(12))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.image((self.logo.id(), Vec2::splat(28.0)));
                    ui.label(RichText::new("Solo Miner").size(17.0).strong());
                });
                ui.add_space(14.0);
                for (page, text) in [
                    (Page::Overview, "Overview"),
                    (Page::Mining, "Mining & wallet"),
                    (Page::Appearance, "Appearance"),
                    (Page::Activity, "Activity"),
                    (
                        Page::Updates,
                        if self.updates.release.is_some() {
                            "Updates •"
                        } else {
                            "Updates"
                        },
                    ),
                ] {
                    let active = self.page == page;
                    let button = egui::Button::new(RichText::new(text).color(if active {
                        c.accent
                    } else {
                        c.muted
                    }))
                    .fill(if active {
                        c.accent.gamma_multiply(if c.dark { 0.14 } else { 0.16 })
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(146.0, 36.0));
                    if ui.add(button).clicked() {
                        self.page = page;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            self.config.pool.name(),
                            if self.config.mining_mode == MiningMode::Solo {
                                "Solo"
                            } else {
                                "Pool"
                            }
                        ))
                        .size(13.0)
                        .strong(),
                    );
                    note(
                        ui,
                        format!(
                            "GPU {}% · diff {}",
                            self.config.gpu_load, self.config.share_difficulty
                        ),
                        c,
                    );
                });
            });
    }
    fn controls(&mut self, ui: &mut egui::Ui, c: Colors) {
        ui.horizontal(|ui| {
            let running = self.lifecycle.work == Some(Work::Mining);
            let label = if self.lifecycle.stopping {
                "Restarting / stopping…"
            } else if running {
                "Mining active"
            } else if self.lifecycle.work == Some(Work::Benchmark) {
                "Benchmark GPU"
            } else {
                "Idle"
            };
            let (dot, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), egui::Sense::hover());
            ui.painter().circle_filled(
                dot.center(),
                4.0,
                if self.worker.is_some() {
                    c.accent
                } else {
                    c.muted
                },
            );
            ui.label(RichText::new(label).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.worker.is_some(),
                        egui::Button::new("Stop mining").min_size(Vec2::new(124.0, 38.0)),
                    )
                    .clicked()
                {
                    self.stop();
                }
                if ui
                    .add_enabled(
                        !self.lifecycle.want_mining
                            && self.config.validate().is_ok()
                            && self.config.mining_username().is_ok(),
                        egui::Button::new(
                            RichText::new("Start mining")
                                .color(accent_foreground(c.accent))
                                .strong(),
                        )
                        .fill(c.accent)
                        .min_size(Vec2::new(134.0, 38.0)),
                    )
                    .clicked()
                {
                    self.start();
                }
            });
        });
        ui.add_space(8.0);
    }
    fn overview(&mut self, ui: &mut egui::Ui, c: Colors) {
        let mining = self.lifecycle.work == Some(Work::Mining);
        let speed = if mining {
            self.stats.hashes_per_second
        } else {
            0.0
        };
        let uptime = if mining {
            self.session_start
                .map(|s| duration(s.elapsed().as_secs_f64()))
                .unwrap_or_else(|| "—".into())
        } else {
            "—".into()
        };
        ui.columns(3, |cols| {
            metric(&mut cols[0], c, "CURRENT HASHRATE", &tray::rate(speed), "");
            metric(
                &mut cols[1],
                c,
                "ACCEPTED SHARES",
                &self.stats.accepted.to_string(),
                &format!("{} rejected", self.stats.rejected),
            );
            metric(&mut cols[2], c, "SESSION TIME", &uptime, "");
        });
        ui.add_space(8.0);
        ui.columns(2, |cols| {
            self.history_card(&mut cols[0], c);
            self.pool_card(&mut cols[1], c, speed);
        });
    }
    fn history_card(&mut self, ui: &mut egui::Ui, c: Colors) {
        card(c).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Hashrate").strong().size(16.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (seconds, label) in [(900.0, "15 min"), (300.0, "5 min"), (60.0, "1 min")] {
                        ui.selectable_value(&mut self.graph_seconds, seconds, label);
                    }
                });
            });
            ui.add_space(8.0);
            self.graph(ui, c);
        });
    }
    fn pool_card(&mut self, ui: &mut egui::Ui, c: Colors, speed: f64) {
        card(c).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.config.pool.name()).strong().size(18.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.config.validate().is_ok() && self.config.mining_username().is_ok(),
                            egui::Button::new("Pool account ↗"),
                        )
                        .clicked()
                    {
                        self.account();
                    }
                });
            });
            note(ui, self.config.endpoint(), c);
            if self.config.pool == Pool::Solopool {
                note(
                    ui,
                    format!(
                        "{}% solo · {}% pool",
                        if self.config.mining_mode == MiningMode::Solo {
                            100
                        } else {
                            self.config.solo_percent
                        },
                        if self.config.mining_mode == MiningMode::Solo {
                            0
                        } else {
                            100 - self.config.solo_percent
                        }
                    ),
                    c,
                );
            }
            ui.add_space(6.0);
            ui.columns(3, |cols| {
                detail(
                    &mut cols[0],
                    "Share difficulty",
                    self.stats
                        .share_difficulty
                        .map(number)
                        .unwrap_or_else(|| "—".into()),
                    c,
                );
                detail(
                    &mut cols[1],
                    "Network difficulty",
                    self.stats
                        .network_difficulty
                        .map(number)
                        .unwrap_or_else(|| "—".into()),
                    c,
                );
                detail(
                    &mut cols[2],
                    "Average share time",
                    self.stats
                        .share_difficulty
                        .filter(|_| speed > 0.0)
                        .map(|d| duration(d * 4_294_967_296.0 / speed))
                        .unwrap_or_else(|| "—".into()),
                    c,
                );
            });
            ui.add_space(8.0);
            if self.config.requires_address() {
                note(ui, "REWARD ADDRESS", c);
                ui.label(
                    RichText::new(&self.config.bitcoin_address)
                        .monospace()
                        .size(12.0),
                );
            } else {
                note(ui, "POOL ACCOUNT", c);
                ui.label(
                    RichText::new(if self.config.pool == Pool::BraiinsPool {
                        &self.config.braiins_account
                    } else {
                        &self.config.f2pool_account
                    })
                    .monospace()
                    .size(12.0),
                );
            }
            if !self.stats.gpu.is_empty() {
                note(
                    ui,
                    format!(
                        "{} · {} · {} block candidates",
                        self.stats.gpu,
                        self.config.gpu_profile.label(),
                        self.stats.block_candidates
                    ),
                    c,
                );
            }
        });
    }
    fn graph(&self, ui: &mut egui::Ui, c: Colors) {
        let (outer, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 160.0), egui::Sense::hover());
        let r = egui::Rect::from_min_max(
            outer.min + Vec2::new(58.0, 10.0),
            outer.max - Vec2::new(12.0, 22.0),
        );
        let now = self.launched.elapsed().as_secs_f64();
        let since = (now - self.graph_seconds).max(0.0);
        let span = (now - since).max(1.0);
        let points: Vec<_> = self
            .history
            .iter()
            .copied()
            .filter(|(t, _)| *t >= since)
            .collect();
        let max = points.iter().map(|(_, v)| *v).fold(0.0, f64::max).max(1e6) * 1.15;
        let painter = ui.painter();
        for step in 0..=3 {
            let y = egui::lerp(r.bottom()..=r.top(), step as f32 / 3.0);
            painter.line_segment(
                [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
                Stroke::new(1.0_f32, c.border),
            );
            painter.text(
                egui::pos2(r.left() - 8.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{:.1} M", max * step as f64 / 3.0 / 1e6),
                egui::FontId::proportional(10.0),
                c.muted,
            );
        }
        let xy = |t: f64, v: f64| {
            egui::pos2(
                r.left() + ((t - since) / span) as f32 * r.width(),
                r.bottom() - (v / max) as f32 * r.height(),
            )
        };
        for pair in points.windows(2) {
            let a = xy(pair[0].0, pair[0].1);
            let b = xy(pair[1].0, pair[1].1);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(a.x, r.bottom()),
                    a,
                    b,
                    egui::pos2(b.x, r.bottom()),
                ],
                c.accent.linear_multiply(0.08),
                Stroke::NONE,
            ));
            painter.line_segment([a, b], Stroke::new(2.2_f32, c.accent));
        }
        painter.text(
            egui::pos2(r.left(), r.bottom() + 13.0),
            egui::Align2::LEFT_CENTER,
            "Start of interval",
            egui::FontId::proportional(10.0),
            c.muted,
        );
        painter.text(
            egui::pos2(r.right(), r.bottom() + 13.0),
            egui::Align2::RIGHT_CENTER,
            "Now",
            egui::FontId::proportional(10.0),
            c.muted,
        );
        if points.is_empty() {
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                "No data",
                egui::FontId::proportional(13.0),
                c.muted,
            );
        }
        if let Some(last) = points.last() {
            painter.circle_filled(xy(last.0, last.1), 3.5, c.accent);
        }
        if let Some(pos) = response.hover_pos().filter(|pos| r.contains(*pos))
            && let Some((t, value)) = points.iter().min_by(|a, b| {
                (xy(a.0, a.1).x - pos.x)
                    .abs()
                    .total_cmp(&(xy(b.0, b.1).x - pos.x).abs())
            })
        {
            let p = xy(*t, *value);
            painter.line_segment(
                [egui::pos2(p.x, r.top()), egui::pos2(p.x, r.bottom())],
                Stroke::new(1.0_f32, c.muted),
            );
            painter.circle_filled(p, 4.0, c.accent);
            response.on_hover_ui(|ui| {
                ui.strong(tray::rate(*value));
                ui.label(format!("{} ago", duration(now - t)));
            });
        }
    }
    fn mining_settings(&mut self, ui: &mut egui::Ui, c: Colors) {
        card(c).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Mode");
                for (mode, label) in [(MiningMode::Solo, "Solo"), (MiningMode::Pool, "Pool")] {
                    if ui.selectable_value(&mut self.draft.mining_mode, mode, label).clicked() {
                        if !self.draft.pool.supports(mode) {
                            self.draft.pool = if mode == MiningMode::Solo { Pool::Ckpool } else { Pool::Solopool };
                        }
                        if mode == MiningMode::Pool && self.draft.pool == Pool::Solopool && self.draft.solo_percent == 100 {
                            self.draft.solo_percent = 0;
                        }
                    }
                }
            });
            ui.add_space(6.0);
            if !self.draft.pool.supports(self.draft.mining_mode) {
                note(ui, "This provider is disabled. Choose another pool.", c);
            }
            note(ui, "REWARD ADDRESS", c);
            ui.add(
                egui::TextEdit::singleline(&mut self.draft.bitcoin_address)
                    .hint_text("bc1…")
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(8, 5)),
            );
            ui.add_space(6.0);
            ui.columns(2, |cols| {
                note(&mut cols[0], "MINING POOL", c);
                egui::ComboBox::from_id_salt("pool")
                    .selected_text(self.draft.pool.name())
                    .width(240.0)
                    .show_ui(&mut cols[0], |ui| {
                        let choices = if self.draft.mining_mode == MiningMode::Solo { &Pool::SOLO[..] } else { &Pool::SHARED[..] };
                        for &pool in choices {
                            ui.selectable_value(&mut self.draft.pool, pool, pool.name());
                        }
                    });
                if self.draft.pool.accepts_difficulty_suggestion() {
                    note(
                        &mut cols[1],
                        if self.draft.pool == Pool::Solopool { "SHARE DIFFICULTY · INTEGER ≥ 1" } else { "SHARE DIFFICULTY" },
                        c,
                    );
                    cols[1].add(
                        egui::TextEdit::singleline(&mut self.difficulty)
                            .desired_width(240.0)
                            .margin(egui::Margin::symmetric(8, 4)),
                    );
                } else {
                    note(&mut cols[1], "SHARE DIFFICULTY", c);
                    cols[1].label("Assigned by pool");
                }
            });
            if self.draft.pool == Pool::Solopool && self.draft.mining_mode == MiningMode::Pool {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    note(ui, "SOLO / POOL SPLIT", c);
                    ui.label(format!(
                        "{}% solo · {}% pool",
                        self.draft.solo_percent,
                        100 - self.draft.solo_percent
                    ));
                });
                let width = ui.available_width();
                ui.scope(|ui| {
                    ui.spacing_mut().slider_width = width;
                    ui.add(
                        egui::Slider::new(&mut self.draft.solo_percent, 0..=99)
                            .show_value(false)
                            .step_by(1.0),
                    )
                    .on_hover_text("SoloPool may lock decreases for 48 hours after an increase. The ratio also applies to other workers on the same address.");
                });
            }
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("GPU duty cycle").strong());
                ui.label(
                    RichText::new(format!("{}%", self.draft.gpu_load))
                        .color(c.accent_text)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("gpu_profile")
                        .selected_text(self.draft.gpu_profile.label())
                        .width(132.0)
                        .show_ui(ui, |ui| {
                            for profile in GpuProfile::ALL {
                                ui.selectable_value(&mut self.draft.gpu_profile, profile, profile.label());
                            }
                        }).response.on_hover_text("Batch size: smaller for responsiveness, larger for throughput. Independent of duty cycle.");
                    ui.label("GPU workload");
                });
            });
            duty_slider(ui, &mut self.draft.gpu_load, c);
        });
        ui.add_space(10.0);
        ui.columns(2, |cols| {
            card(c).show(&mut cols[0], |ui| {
                ui.label(RichText::new("Notifications").size(17.0).strong());
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(
                        &mut self.draft.notifications.accepted_share,
                        "Accepted shares",
                    );
                    ui.checkbox(
                        &mut self.draft.notifications.block_candidate,
                        "Block candidates",
                    );
                    ui.checkbox(&mut self.draft.notifications.error, "Errors");
                });
                if !self.notification_status.is_empty() {
                    note(ui, &self.notification_status, c);
                }
            });
            card(c).show(&mut cols[1], |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Benchmark").size(17.0).strong());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(self.worker.is_none(), egui::Button::new("Run (3 s)"))
                            .clicked()
                        {
                            self.benchmark();
                        }
                    });
                });
                if let Some((load, profile, value)) = self.benchmark_result {
                    ui.label(
                        RichText::new(tray::rate(value))
                            .size(20.0)
                            .color(c.accent_text)
                            .strong(),
                    );
                    note(ui, format!("{}% duty · {}", load, profile.label()), c);
                }
                if self.worker.is_some() {
                    note(ui, "Available when mining is stopped", c);
                }
            });
        });
    }

    fn appearance(&mut self, ui: &mut egui::Ui, c: Colors) {
        card(c).show(ui, |ui| {
            ui.label(RichText::new("Mode").size(17.0).strong());
            ui.horizontal(|ui| {
                for (mode, label) in [
                    (ThemeMode::Light, "Light"),
                    (ThemeMode::Dark, "Dark"),
                    (ThemeMode::System, "System"),
                ] {
                    ui.selectable_value(&mut self.draft.appearance.mode, mode, label);
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(RichText::new("Theme").size(17.0).strong());
            ui.horizontal(|ui| {
                for (palette, label) in [
                    (Palette::Graphite, "Graphite"),
                    (Palette::Ocean, "Ocean"),
                    (Palette::Forest, "Forest"),
                ] {
                    ui.selectable_value(&mut self.draft.appearance.palette, palette, label);
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(RichText::new("Accent color").size(17.0).strong());
            ui.horizontal(|ui| {
                for (accent, name) in [
                    ([245, 158, 66], "Amber"),
                    ([60, 194, 218], "Cyan"),
                    ([167, 139, 250], "Purple"),
                    ([244, 114, 151], "Pink"),
                    ([91, 201, 147], "Mint"),
                ] {
                    let selected = self.draft.appearance.accent == accent;
                    let b = egui::Button::new(
                        RichText::new(name)
                            .color(Color32::from_rgb(accent[0], accent[1], accent[2])),
                    )
                    .stroke(Stroke::new(
                        if selected { 2.0_f32 } else { 1.0_f32 },
                        if selected { c.accent } else { c.border },
                    ));
                    if ui.add(b).clicked() {
                        self.draft.appearance.accent = accent;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Custom");
                ui.color_edit_button_srgb(&mut self.draft.appearance.accent);
            });
        });
    }
    fn settings_actions(&mut self, ui: &mut egui::Ui, c: Colors) {
        ui.horizontal(|ui| {
            if self.page == Page::Mining {
                let label = if self.lifecycle.want_mining {
                    "Save & restart mining"
                } else {
                    "Save settings"
                };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(label)
                                .strong()
                                .color(accent_foreground(c.accent)),
                        )
                        .fill(c.accent)
                        .min_size(Vec2::new(200.0, 32.0)),
                    )
                    .clicked()
                {
                    self.save(false);
                }
                if ui.button("Discard changes").clicked() {
                    self.draft = self.config.clone();
                    self.difficulty = self.config.share_difficulty.to_string();
                    self.message = "Changes discarded".into();
                }
            } else {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("Save appearance")
                                .strong()
                                .color(accent_foreground(c.accent)),
                        )
                        .fill(c.accent)
                        .min_size(Vec2::new(160.0, 32.0)),
                    )
                    .clicked()
                {
                    self.save(true);
                }
                if ui.button("Restore saved").clicked() {
                    self.draft.appearance = self.config.appearance.clone();
                }
            }
        });
    }
    fn activity(&mut self, ui: &mut egui::Ui, c: Colors) {
        ui.horizontal(|ui| {
            ui.label("Keep");
            let mut days = self.config.log_retention_days;
            egui::ComboBox::from_id_salt("retention")
                .selected_text(if days == 0 {
                    "All".into()
                } else {
                    format!("{days} days")
                })
                .show_ui(ui, |ui| {
                    for (value, label) in [(7, "7 days"), (30, "30 days"), (0, "All")] {
                        ui.selectable_value(&mut days, value, label);
                    }
                });
            if days != self.config.log_retention_days {
                let mut next = self.config.clone();
                next.log_retention_days = days;
                match next
                    .save(&self.path)
                    .and_then(|_| self.journal.set_retention(days))
                {
                    Ok(()) => {
                        self.config = next;
                        self.draft.log_retention_days = days;
                    }
                    Err(e) => self.message = format!("Log: {e:#}"),
                }
            }
            note(ui, format!("{} events", self.journal.entries.len()), c);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Clear log").clicked()
                    && let Err(e) = self.journal.clear()
                {
                    self.message = format!("Log: {e:#}");
                }
            });
        });
        ui.add_space(6.0);
        let widths = [148.0, 82.0];
        ui.horizontal(|ui| {
            ui.add_sized(
                [widths[0], 22.0],
                egui::Label::new(RichText::new("Date and time").strong()),
            );
            ui.add_sized(
                [widths[1], 22.0],
                egui::Label::new(RichText::new("Type").strong()),
            );
            ui.strong("Message");
        });
        ui.separator();
        if self.journal.entries.is_empty() {
            note(ui, "No events", c);
        }
        egui::ScrollArea::vertical()
            .id_salt("log_rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for entry in self.journal.entries.iter().rev() {
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            [widths[0], 22.0],
                            egui::Label::new(RichText::new(entry.date()).size(12.0).color(c.muted)),
                        );
                        ui.add_sized(
                            [widths[1], 22.0],
                            egui::Label::new(RichText::new(&entry.kind).size(12.0).color(
                                if entry.kind == "Error" {
                                    Color32::from_rgb(235, 105, 105)
                                } else {
                                    c.muted
                                },
                            )),
                        );
                        ui.add(egui::Label::new(&entry.text).wrap().selectable(true));
                    });
                    ui.separator();
                }
            });
    }
}
impl eframe::App for Desktop {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive();
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
            self.quit();
        }
        let c = self.style(ctx);
        self.titlebar(ctx, c);
        self.sidebar(ctx, c);
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(c.side)
                    .inner_margin(egui::Margin::symmetric(14, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if self.message.is_empty() {
                            &self.stats.state
                        } else {
                            &self.message
                        })
                        .size(11.0)
                        .color(c.muted),
                    );
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(c.bg).inner_margin(16))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.updates.phase != UpdatePhase::Waiting, |ui| {
                    self.controls(ui, c)
                });
                if matches!(self.page, Page::Mining | Page::Appearance) {
                    egui::TopBottomPanel::bottom("page_actions")
                        .exact_height(46.0)
                        .frame(
                            egui::Frame::new()
                                .fill(c.bg)
                                .inner_margin(egui::Margin::symmetric(0, 5)),
                        )
                        .show_inside(ui, |ui| self.settings_actions(ui, c));
                }
                if self.page == Page::Activity {
                    self.activity(ui, c);
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Overview => self.overview(ui, c),
                        Page::Mining => self.mining_settings(ui, c),
                        Page::Appearance => self.appearance(ui, c),
                        Page::Activity => self.activity(ui, c),
                        Page::Updates => self.updates_page(ui, c),
                    });
            });
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}
fn detail(ui: &mut egui::Ui, label: &str, value: String, c: Colors) {
    note(ui, label, c);
    ui.label(RichText::new(value).strong().size(16.0));
}
fn number(value: f64) -> String {
    if value >= 1e12 {
        format!("{:.2} T", value / 1e12)
    } else if value >= 1e9 {
        format!("{:.2} G", value / 1e9)
    } else if value >= 1e6 {
        format!("{:.2} M", value / 1e6)
    } else if value >= 1e3 {
        format!("{:.2} k", value / 1e3)
    } else {
        format!("{value:.4}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .into()
    }
}
fn duration(seconds: f64) -> String {
    if seconds >= 86400.0 {
        format!("{:.1} days", seconds / 86400.0)
    } else if seconds >= 3600.0 {
        format!("{:.1} hours", seconds / 3600.0)
    } else if seconds >= 60.0 {
        format!("{:.1} min", seconds / 60.0)
    } else {
        format!("{seconds:.0} s")
    }
}

fn accent_foreground(accent: Color32) -> Color32 {
    let luminance = 0.2126 * f64::from(accent.r())
        + 0.7152 * f64::from(accent.g())
        + 0.0722 * f64::from(accent.b());
    if luminance > 145.0 {
        Color32::from_rgb(16, 21, 27)
    } else {
        Color32::WHITE
    }
}

fn duty_slider(ui: &mut egui::Ui, value: &mut u8, c: Colors) {
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        ui.spacing_mut().interact_size.y = 22.0;
        let response = ui.add(
            egui::Slider::new(value, 5..=100)
                .step_by(5.0)
                .clamping(egui::SliderClamping::Edits)
                .show_value(false),
        );
        let radius = ui.spacing().interact_size.y / 2.5;
        let left = response.rect.left() + radius;
        let right = response.rect.right() - radius;
        let y = response.rect.bottom() + 3.0;
        for n in 1..=20 {
            let x = egui::lerp(left..=right, (n - 1) as f32 / 19.0);
            ui.painter().circle_filled(
                egui::pos2(x, y),
                1.5,
                if n * 5 <= i32::from(*value) {
                    c.accent
                } else {
                    c.muted
                },
            );
        }
        let (labels, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 12.0), egui::Sense::hover());
        for (x, align, text) in [
            (labels.left(), egui::Align2::LEFT_CENTER, "5%"),
            (labels.right(), egui::Align2::RIGHT_CENTER, "100%"),
        ] {
            ui.painter().text(
                egui::pos2(x, labels.center().y),
                align,
                text,
                egui::FontId::proportional(10.0),
                c.muted,
            );
        }
    });
}
