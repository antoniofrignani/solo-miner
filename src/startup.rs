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
    let main =
        MainThreadMarker::new().expect("Il controllo Metal deve essere sul thread principale");
    let app = NSApplication::sharedApplication(main);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let alert = NSAlert::new(main);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str("Impossibile inizializzare Metal"));
    alert.setInformativeText(&NSString::from_str(&format!(
        "Solo Miner richiede una GPU compatibile con Metal.\n\n{error:#}"
    )));
    alert.addButtonWithTitle(&NSString::from_str("Esci"));
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
            || anyhow::bail!("Nessuna GPU Metal disponibile"),
            |error| {
                assert_eq!(error.to_string(), "Nessuna GPU Metal disponibile");
                shown.set(true);
            },
            || panic!("L'app non deve avviarsi senza Metal"),
        );
        assert!(result.is_err());
        assert!(shown.get());
    }

    #[test]
    fn supported_gpu_starts_app_without_alert() -> Result<()> {
        let started = Cell::new(false);
        run(
            || Ok(()),
            |_| panic!("Metal disponibile"),
            || {
                started.set(true);
                Ok(())
            },
        )?;
        assert!(started.get());
        Ok(())
    }
}
