use crate::{config::Config, miner::Stats};
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
            "{} · GPU {}% · diff {}",
            config.pool.name(),
            config.gpu_load,
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

pub fn logo_rgba(size: usize) -> Vec<u8> {
    let mut rgba = vec![0; size * size * 4];
    let curves = [
        [[0.71, 0.30], [0.62, 0.16], [0.29, 0.18], [0.29, 0.36]],
        [[0.29, 0.36], [0.29, 0.47], [0.43, 0.48], [0.52, 0.50]],
        [[0.52, 0.50], [0.65, 0.53], [0.73, 0.56], [0.70, 0.67]],
        [[0.70, 0.67], [0.67, 0.85], [0.36, 0.83], [0.27, 0.70]],
    ];
    let mut points: Vec<[f32; 2]> = Vec::new();
    for curve in curves {
        for i in 0..=64 {
            let t = i as f32 / 64.0;
            let u = 1.0 - t;
            points.push(std::array::from_fn(|axis| {
                u * u * u * curve[0][axis]
                    + 3.0 * u * u * t * curve[1][axis]
                    + 3.0 * u * t * t * curve[2][axis]
                    + t * t * t * curve[3][axis]
            }));
        }
    }
    for y in 0..size {
        for x in 0..size {
            let fx = (x as f32 + 0.5) / size as f32;
            let fy = (y as f32 + 0.5) / size as f32;
            let dx = (fx - 0.5).abs() - 0.28;
            let dy = (fy - 0.5).abs() - 0.28;
            if dx.max(0.0).hypot(dy.max(0.0)) > 0.18 {
                continue;
            }
            let distance = points
                .iter()
                .map(|p| (fx - p[0]).hypot(fy - p[1]))
                .fold(f32::INFINITY, f32::min);
            let ink = ((0.042 - distance) * size as f32 + 0.5).clamp(0.0, 1.0);
            let foreground = [17.0, 22.0, 29.0];
            let background = [245.0, 158.0, 66.0];
            let color = [
                (background[0] * (1.0 - ink) + foreground[0] * ink) as u8,
                (background[1] * (1.0 - ink) + foreground[1] * ink) as u8,
                (background[2] * (1.0 - ink) + foreground[2] * ink) as u8,
                255,
            ];
            rgba[(y * size + x) * 4..(y * size + x + 1) * 4].copy_from_slice(&color);
        }
    }
    rgba
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
