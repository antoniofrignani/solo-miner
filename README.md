# Solo Miner

Bitcoin solo miner for macOS, written in Rust. SHA-256d hashing on Metal GPUs, an egui interface and a menu bar icon. MIT license.

## Platform

Only the **macOS Apple Silicon (arm64)** build is distributed. The app is ad hoc signed and is not notarized by Apple.

At startup, the app checks the GPU, shaders and Metal calculations. If the check fails, it displays the error and exits before opening the interface or connecting to a pool.

## Usage

Open **Solo Miner.app**, configure a Bitcoin mainnet reward address, pool, share difficulty and GPU duty cycle, then save and start mining.

- **Overview**: hashrate, shares, session time, graph and pool information.
- **Mining & wallet**: settings, notifications and a 3-second benchmark.
- **Appearance**: light/dark/system mode, theme and accent color.
- **Activity**: disk log with 7-day, 30-day or unlimited retention.

Available pools: CKPool Solo, Braiins Solo, SoloPool and Public Pool. **Pool account** opens the statistics page for the configured wallet. Endpoints and Stratum notes: [docs/pools.md](docs/pools.md).

Saving changes to the wallet, pool, difficulty or duty cycle restarts mining. Appearance and notification changes apply without a restart. Closing the window leaves mining active; quit the app from the menu bar or with `⌘Q`.

The duty cycle controls GPU batches and pauses, in 5% increments in the interface. It does not set a power or temperature limit. The pool assigns the actual share difficulty. Block candidates are checked on the CPU before submission; acceptance and rewards depend on the pool and network.

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
```

Settings are stored in `config.toml` in the app's user directory; `--check` prints the path. The `activity.jsonl` log is in the same directory. Hashrate history and session counters are kept in memory.

## Tests

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Regular tests do not contact external pools. Two additional tests are available on macOS:

```sh
cargo test --locked gpu_to_stratum_share -- --ignored --nocapture
cargo test --locked solopool_live_difficulty_handshake -- --ignored --nocapture
```

The first requires a Metal GPU and uses a local Stratum server. The second checks the SoloPool handshake using saved settings, without submitting shares.

## GitHub Actions

Every push to `main` and every pull request runs formatting, Clippy, tests and a build on macOS Apple Silicon. Archives and SHA-256 checksums are available as [build artifacts](https://github.com/antoniofrignani/solo-miner/actions/workflows/ci.yml).

A `vX.Y.Z` tag matching the version in `Cargo.toml` publishes a release with the same archives after the build succeeds.

[Changelog](docs/HISTORY.md).
