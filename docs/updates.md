# App updates

The Updates page uses the public GitHub latest-release API for `antoniofrignani/solo-miner`. Checks run in a background thread, manually or at startup. Only newer stable semantic versions are offered; drafts, prereleases and downgrades are excluded. No GitHub token or wallet data are sent.

The updater accepts the fixed `solo-miner-macos-arm64.zip` and `.sha256` asset names from the matching release URL in this repository. It checks the archive size and SHA-256 against the checksum asset, and the GitHub asset digest when available. HTTPS verification remains enabled across redirects. Metadata, download size, entry count and expanded size are bounded.

Extraction rejects paths outside the bundle, traversal, symlinks and duplicate paths. Before installation, the updater checks the bundle identifier, executable name, version, arm64 Mach-O header and code signature. These integrity checks trust this GitHub repository and HTTPS; the current ad hoc signature is not an independent publisher identity or an Ed25519 update signature.

## Installation

Staging is created beside the current app. macOS rename exchange swaps the two bundles atomically on the same filesystem, keeping the app path present throughout replacement and rollback. A non-writable folder or filesystem without rename exchange support reports an error without requesting administrator privileges. Running an unbundled development binary cannot replace an installed app.

1. Download and verify without interrupting mining.
2. On **Install and restart**, stop the worker and wait for its completion event.
3. Start a copy of the current executable as the installer helper, then exit the app.
4. The helper waits for the original process to exit and rechecks the staged bundle and current version.
5. Exchange the current and staged apps atomically, retaining the previous bundle as a backup.
6. Launch the new app with its prior mining state. After GPU startup checks, configuration loading and UI creation, the new app confirms its version to the helper.
7. Remove the backup after confirmation. If launch fails or does not confirm within 30 seconds, terminate the new child, restore the previous bundle and reopen it.

Worker shutdown is bounded by existing GPU completion and cancellable duty-cycle pauses. The helper aborts if the original app has not exited within 60 seconds. Failure reports are stored beside the configuration as `update-result.txt` and included in the next app's activity log. Failed installations retain their staging folder and installer log for diagnosis. Wallet, pool and mining preferences are never replaced.

Update preferences are stored separately in `updates.json`; adding a field to the older strict configuration format would prevent rollback to an earlier app.

## Verification

Normal tests check semantic version ordering, download origin, checksum parsing, archive path/symlink rejection, executable permissions, successful replacement and rollback after failed replacement or launch.

Explicit tests:

```sh
# Fetches and verifies the published app, without installation.
cargo test --locked published_release_download_and_bundle_verification -- --ignored --nocapture

# Runs the real helper process against isolated compiled fixture apps.
# Does not replace the installed Solo Miner app or use real mining settings.
cargo test --locked installer_subprocess_commit_and_rollback -- --ignored --nocapture
```

The helper fixture requires macOS Command Line Tools and codesign. Its subprocess entry test is invoked by the fixture and is not a standalone test.

Set `SOLO_MINER_INSTALLER_BINARY` to the absolute bundled executable path when running the helper fixture to test the packaged executable instead of the test executable.
