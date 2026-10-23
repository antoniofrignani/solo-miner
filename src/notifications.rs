use crate::{config::NotificationSettings, miner::MinerEvent};
use block2::RcBlock;
use objc2::{
    AnyThread, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, ProtocolObject},
};
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

define_class!(
    #[unsafe(super = NSObject)]
    struct NotificationDelegate;
    unsafe impl NSObjectProtocol for NotificationDelegate {}
    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

pub struct Notifications {
    center: Option<Retained<UNUserNotificationCenter>>,
    _delegate: Retained<NotificationDelegate>,
    report: Arc<dyn Fn(String) + Send + Sync>,
    last_error: Option<(String, Instant)>,
}
impl Notifications {
    pub fn new(report: impl Fn(String) + Send + Sync + 'static) -> Self {
        let delegate: Retained<NotificationDelegate> =
            unsafe { msg_send![NotificationDelegate::alloc(), init] };
        // UNUserNotificationCenter requires a real application bundle (cargo run has none).
        let bundled = NSBundle::mainBundle().bundleIdentifier().is_some();
        let center = bundled.then(UNUserNotificationCenter::currentNotificationCenter);
        if let Some(center) = &center {
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        Self {
            center,
            _delegate: delegate,
            report: Arc::new(report),
            last_error: None,
        }
    }
    pub fn authorize(&self) {
        let Some(center) = &self.center else {
            (self.report)("Open Solo Miner.app to use notifications.".into());
            return;
        };
        let report = self.report.clone();
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let text = if !error.is_null() {
                format!("Notification authorization error: {}", unsafe { &*error })
            } else if granted.as_bool() {
                "Notifications authorized".into()
            } else {
                "Notifications denied: enable Solo Miner in System Settings → Notifications.".into()
            };
            report(text);
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &completion,
        );
    }
    pub fn event(&mut self, event: &MinerEvent, settings: &NotificationSettings) {
        let Some((title, body)) = notification(event, settings) else {
            return;
        };
        if let MinerEvent::Error(error) = event {
            if self.last_error.as_ref().is_some_and(|(last, time)| {
                last == error && time.elapsed() < Duration::from_secs(60)
            }) {
                return;
            }
            self.last_error = Some((error.clone(), Instant::now()));
        }
        let Some(center) = &self.center else {
            return;
        };
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(&body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let id = format!(
            "solo-miner-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&id),
            &content,
            None,
        );
        let report = self.report.clone();
        let completion = RcBlock::new(move |error: *mut NSError| {
            if !error.is_null() {
                report(format!("Notification delivery failed: {}", unsafe {
                    &*error
                }));
            }
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));
    }
}
fn notification(
    event: &MinerEvent,
    settings: &NotificationSettings,
) -> Option<(&'static str, String)> {
    match event {
        MinerEvent::AcceptedShare if settings.accepted_share => {
            Some(("Share accepted", "Confirmed by pool.".into()))
        }
        MinerEvent::BlockCandidate if settings.block_candidate => Some((
            "Block candidate found",
            "The block has not yet been confirmed by the pool.".into(),
        )),
        MinerEvent::Error(error) if settings.error => Some(("Solo Miner error", error.clone())),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_categories_are_independent() {
        let settings = NotificationSettings {
            accepted_share: true,
            ..Default::default()
        };
        assert!(notification(&MinerEvent::AcceptedShare, &settings).is_some());
        assert!(notification(&MinerEvent::BlockCandidate, &settings).is_none());
        assert!(notification(&MinerEvent::Error("offline".into()), &settings).is_none());
        let settings = NotificationSettings {
            block_candidate: true,
            error: true,
            ..Default::default()
        };
        assert!(notification(&MinerEvent::AcceptedShare, &settings).is_none());
        assert!(
            notification(&MinerEvent::BlockCandidate, &settings)
                .unwrap()
                .1
                .contains("not yet been confirmed")
        );
        assert_eq!(
            notification(&MinerEvent::Error("offline".into()), &settings)
                .unwrap()
                .1,
            "offline"
        );
    }
}
