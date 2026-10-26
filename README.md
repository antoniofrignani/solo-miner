# Solo Miner

[![Build](https://github.com/antoniofrignani/solo-miner/actions/workflows/ci.yml/badge.svg)](https://github.com/antoniofrignani/solo-miner/actions/workflows/ci.yml)

Bitcoin miner for macOS, written in Rust. SHA-256d hashing on Metal GPUs, an egui interface and a menu bar icon. MIT license.

## Platform

Only the **macOS Apple Silicon (arm64)** build is distributed. The app is ad hoc signed and is not notarized by Apple.

At startup, the app checks the GPU, shaders and Metal calculations. If the check fails, it displays the error and exits before opening the interface or connecting to a pool.

## Usage

Open **Solo Miner.app**, choose Solo or Pool, configure the provider and GPU duty cycle, then save and start mining. The saved Bitcoin mainnet reward address identifies the worker at every available provider.

- **Overview**: hashrate, shares, session time, graph and pool information.
- **Mining & wallet**: settings, notifications and a 3-second benchmark.
- **Appearance**: light/dark/system mode, theme and accent color.
- **Activity**: disk log with 7-day, 30-day or unlimited retention.
- **Updates**: check GitHub releases, download, then install and restart. Optional checks at startup.

Solo providers: CKPool Solo, Braiins Solo, Public Pool, SoloPool and OnPool. Pool providers: SoloPool, Public Pool, OCEAN and OnPool. SoloPool supports a configurable solo/pool split. Provider endpoints and verification: [docs/pools.md](docs/pools.md).

Saving changes to the wallet, mode, provider, SoloPool split, difficulty or duty cycle restarts mining. Appearance and notification changes apply without a restart. Closing the window leaves mining active; quit the app from the menu bar or with `⌘Q`.

The duty cycle controls GPU work and pauses, in 5% increments in the interface. Below 100%, the worker drains the pipeline after approximately 50 ms of work, then pauses. Stop and pool messages are checked between batches. It does not set a power or temperature limit. The pool assigns the actual share difficulty. Block candidates are checked on the CPU before submission; acceptance and rewards depend on the pool and network.

## GPU workload

**Mining & wallet → GPU workload** selects the batch size:

| Profile | Nonces per batch | Use |
| --- | ---: | --- |
| Responsive | 65,536 | Shorter stop and job-update latency |
| Balanced (default) | 262,144 | Moderate batch size |
| Throughput | 1,048,576 | Fewer CPU/GPU synchronizations |

Save the profile to apply it; active mining restarts automatically. The benchmark uses the selected profile and duty cycle without saving them. Workload changes do not set GPU clocks or a watt limit. Power and performance depend on the Mac, other workloads and thermal conditions.

The engine caches the SHA-256 midstate of each header, processes two nonces per thread with interleaved rounds and keeps two batches in flight. Each batch retains its own job and output buffer; only matching nonces return to the CPU for verification. [Measurements and test coverage](docs/performance.md).

## Build

Requires an Apple Silicon Mac, Rust stable and Xcode Command Line Tools.

```sh
cargo build --locked --release
sh scripts/bundle.sh
open 'dist/Solo Miner.app'
```

```sh
cargo run -- --check                # Settings path and values
cargo run -- --options              # Open settings
cargo run -- --resume-mining        # Start mining
cargo run --release -- --gpu-check  # GPU self-test and benchmark
cargo run --release -- --gpu-profile  # Compare workloads at 100% duty, without pool connections
```

Settings are stored in `config.toml` and `pool-options.json` in the app's user directory; `--check` prints the path. The `activity.jsonl` log is in the same directory. Hashrate history and session counters are kept in memory.

Regenerate the bundled icon with `sh scripts/app-icon.sh`.

## Tests

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Regular tests do not contact external pools. Hardware and network tests are available on macOS:

```sh
cargo test --locked metal_profiles_match_cpu_and_preserve_overflow -- --ignored --nocapture
cargo test --locked gpu_to_stratum_share -- --ignored --nocapture
cargo test --locked pipeline_benchmark_cancellation_releases_slots -- --ignored --nocapture
cargo test --locked solopool_live_difficulty_handshake -- --ignored --nocapture
cargo test --locked wallet_pools_live_handshake -- --ignored --nocapture
cargo test --locked public_pool_live_shares -- --ignored --nocapture
```

The first checks GPU hashes, concurrent output, overflow and nonce boundaries against CPU results for every workload profile. The second uses a Metal GPU and a local Stratum server. The third checks GPU cancellation at 5%, 85% and 100% duty. The next two check live wallet-pool handshakes using saved settings, without submitting shares. The final test sends real GPU shares to Public Pool in Solo and PPLNS modes using the saved wallet; run it explicitly. Ordinary tests cover clean/non-clean job changes, queued difficulty updates, provider authentication parameters and nonce exhaustion.

## GitHub Actions

A `vX.Y.Z` tag matching the version in `Cargo.toml` runs formatting, Clippy, tests and a build on macOS Apple Silicon. After a successful build, GitHub Actions publishes the release archive and SHA-256 checksum. The workflow does not run on branch pushes or pull requests.

[Changelog](docs/HISTORY.md).

## Updates

The app checks stable releases from this repository. Downloads must match the release SHA-256 checksum, app identity, version and Apple Silicon architecture. The installer stops mining, keeps the previous bundle and waits for the new app to confirm startup. Failed startup restores the previous version. Mining resumes if it was active before installation.

The app folder must be writable. Updates do not request administrator access; move the app to a writable folder if needed. Wallet and mining settings remain in the user directory. Update preferences use a separate `updates.json` file for compatibility with older versions.

Versions before 0.8.0 require a manual installation to gain the updater. [Update implementation and tests](docs/updates.md).
