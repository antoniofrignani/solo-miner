use anyhow::Result;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertStyle, NSApplication, NSApplicationActivationPolicy};
use objc2_foundation::NSString;

pub fn run(
    check: impl FnOnce() -> Result<()>,
    show_error: impl FnOnce(&anyhow::Error),
    start: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if let Err(error) = check() {
        show_error(&error);
        return Err(error);
    }
    start()
}

pub fn show_error(error: &anyhow::Error) {
    let main = MainThreadMarker::new().expect("Metal startup check must run on the main thread");
    let app = NSApplication::sharedApplication(main);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let alert = NSAlert::new(main);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str("Unable to initialize Metal"));
    alert.setInformativeText(&NSString::from_str(&format!(
        "Solo Miner requires a Metal-compatible GPU.\n\n{error:#}"
    )));
    alert.addButtonWithTitle(&NSString::from_str("Quit"));
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    alert.runModal();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn metal_failure_shows_error_without_starting_app() {
        let shown = Cell::new(false);
        let result = run(
            || anyhow::bail!("No Metal GPU available"),
            |error| {
                assert_eq!(error.to_string(), "No Metal GPU available");
                shown.set(true);
            },
            || panic!("The app must not start without Metal"),
        );
        assert!(result.is_err());
        assert!(shown.get());
    }

    #[test]
    fn supported_gpu_starts_app_without_alert() -> Result<()> {
        let started = Cell::new(false);
        run(
            || Ok(()),
            |_| panic!("Metal is available"),
            || {
                started.set(true);
                Ok(())
            },
        )?;
        assert!(started.get());
        Ok(())
    }
}
