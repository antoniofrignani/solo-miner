# Changelog

## 0.8.0

- GitHub update checks, verified downloads and installation from the Updates page.
- Optional startup checks, restart with mining restored and rollback after failed startup.
- Separate update preferences preserve compatibility with earlier app settings.
- Fixed an intermittent CI test that treated the first received TCP byte as a complete difficulty message.

## 0.7.0

- Two GPU batches in flight with independent output buffers.
- Two nonces per Metal thread with interleaved SHA-256 rounds.
- Job-aware pending results, clean-job invalidation and nonce reservation before dispatch.
- Duty-cycle pauses after draining 50 ms work bursts; benchmark uses the same pipeline.
- Tests for concurrent overflow, job transitions, difficulty changes, nonce exhaustion and cancellation.

## 0.6.0

- Cached SHA-256 midstates and compact GPU candidate output.
- Responsive, Balanced and Throughput workloads, saved independently of duty cycle.
- Workload-aware benchmark and automatic restart when the profile changes.
- GPU tests for compact output overflow, header changes and nonce boundaries.

## 0.5.3

- Added the app icon to the macOS bundle for notifications and system dialogs.

## 0.5.2

- English interface, notifications, error messages and documentation.

## 0.5.1

- Distribution limited to macOS Apple Silicon.
- Metal startup check; the app displays an error and exits if the check fails.

## 0.5.0

- Compact window and GPU slider with 5% increments.
- Shared icon and draggable title bar.
- Persistent activity log with configurable retention.

## 0.4.1

- Shortened interface text.

## 0.4.0

- egui interface with start, stop and automatic restart when settings change.
- Hashrate graph and light, dark and system themes.

## 0.3.2

- Integer difficulties serialized as JSON integers: SoloPool ignores values such as `10.0` but accepts `10`.

## 0.3.1

- SoloPool difficulty request sent after authorization; current job updated when the pool does not send a replacement.
- Notification summary updated when settings are saved.

## 0.3.0

- Wallet, pool, difficulty and duty cycle settings.
- Notifications for shares, block candidates and errors.

## 0.2.1

- Initial difficulty request of 1 for Public Pool. Workers appear on the dashboard after the first valid share.

## 0.2.0

- SHA-256d mining on Metal, CPU verification and a Stratum V1 client.

## 0.1.0

- Menu bar controls and pool settings.
