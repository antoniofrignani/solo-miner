use crate::{config::Config, icon::logo_rgba, miner::Stats};
use anyhow::Result;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    },
};
#[derive(Clone, Copy)]
pub enum Action {
    Show,
    Settings,
    Start,
    Stop,
    Account,
    Quit,
}
pub struct Tray {
    _icon: TrayIcon,
    _app_menu: Menu,
    events: std::sync::mpsc::Receiver<MenuEvent>,
    show: MenuItem,
    settings: MenuItem,
    start: MenuItem,
    stop: MenuItem,
    account: MenuItem,
    quit: MenuItem,
    status: MenuItem,
    summary: MenuItem,
    rate: MenuItem,
}
impl Tray {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let (sender, events) = std::sync::mpsc::channel();
        let menu = Menu::new();
        let show = MenuItem::new(
            "Open Solo Miner",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::Digit1)),
        );
        let settings = MenuItem::new(
            "Settings…",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::Comma)),
        );
        let show_id = show.id().clone();
        let settings_id = settings.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id || event.id == settings_id {
                show_native_window();
            }
            let _ = sender.send(event);
            wake();
        }));
        let status = MenuItem::new("Idle", false, None);
        let summary = MenuItem::new("", false, None);
        let rate = MenuItem::new("Hashrate: —", false, None);
        let start = MenuItem::new("Start mining", true, None);
        let stop = MenuItem::new("Stop mining", false, None);
        let account = MenuItem::new("Open pool account…", true, None);
        let quit = MenuItem::new("Quit Solo Miner", true, None);
        menu.append_items(&[
            &show,
            &status,
            &rate,
            &summary,
            &PredefinedMenuItem::separator(),
            &start,
            &stop,
            &settings,
            &account,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon()?)
            .with_icon_as_template(false)
            .with_tooltip("Solo Miner")
            .build()?;
        let app_menu = Menu::new();
        let app = Submenu::new("Solo Miner", true);
        app.append_items(&[&show, &settings, &PredefinedMenuItem::separator(), &quit])?;
        let edit = Submenu::new("Edit", true);
        edit.append_items(&[
            &PredefinedMenuItem::cut(Some("Cut")),
            &PredefinedMenuItem::copy(Some("Copy")),
            &PredefinedMenuItem::paste(Some("Paste")),
            &PredefinedMenuItem::select_all(Some("Select All")),
        ])?;
        app_menu.append_items(&[&app, &edit])?;
        app_menu.init_for_nsapp();
        Ok(Self {
            _icon: icon,
            _app_menu: app_menu,
            events,
            show,
            settings,
            start,
            stop,
            account,
            quit,
            status,
            summary,
            rate,
        })
    }
    pub fn poll(&self) -> Option<Action> {
        let event = self.events.try_recv().ok()?;
        [
            (&self.show, Action::Show),
            (&self.settings, Action::Settings),
            (&self.start, Action::Start),
            (&self.stop, Action::Stop),
            (&self.account, Action::Account),
            (&self.quit, Action::Quit),
        ]
        .into_iter()
        .find_map(|(item, action)| (event.id == *item.id()).then_some(action))
    }
    pub fn update(&self, config: &Config, stats: &Stats, busy: bool) {
        self.status.set_text(&stats.state);
        self.rate
            .set_text(format!("Hashrate: {}", rate(stats.hashes_per_second)));
        self.summary.set_text(format!(
            "{} · GPU {}% · {} · diff {}",
            config.pool.name(),
            config.gpu_load,
            config.gpu_profile.label(),
            config.share_difficulty
        ));
        self.start
            .set_enabled(!busy && !config.bitcoin_address.is_empty());
        self.stop.set_enabled(busy);
        self.account.set_enabled(!config.bitcoin_address.is_empty());
    }
}
pub fn rate(value: f64) -> String {
    if value >= 1e9 {
        format!("{:.2} GH/s", value / 1e9)
    } else if value >= 1e6 {
        format!("{:.2} MH/s", value / 1e6)
    } else if value >= 1e3 {
        format!("{:.1} kH/s", value / 1e3)
    } else {
        format!("{value:.0} H/s")
    }
}
fn icon() -> Result<Icon> {
    Ok(Icon::from_rgba(logo_rgba(36), 36, 36)?)
}

// Hidden viewports stop rendering; reveal the native window before waking egui.
fn show_native_window() {
    if let Some(main) = objc2::MainThreadMarker::new() {
        let app = objc2_app_kit::NSApplication::sharedApplication(main);
        for window in app.windows().iter() {
            if window.title().to_string() == "Solo Miner" {
                window.makeKeyAndOrderFront(None);
            }
        }
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}
