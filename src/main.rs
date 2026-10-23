#[cfg(target_os = "macos")]
mod activity;
use solo_miner::config;
#[cfg(target_os = "macos")]
use solo_miner::{pool, pow};
#[cfg(target_os = "macos")]
mod desktop;
#[cfg(target_os = "macos")]
mod gpu;
#[cfg(target_os = "macos")]
mod lifecycle;
#[cfg(target_os = "macos")]
mod miner;
#[cfg(target_os = "macos")]
mod notifications;
#[cfg(target_os = "macos")]
mod startup;
#[cfg(target_os = "macos")]
mod tray;

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if std::env::args().any(|arg| arg == "--gpu-check") {
        let gpu = gpu::Gpu::new()?;
        println!(
            "GPU: {} — SHA-256d and target comparison verified",
            gpu.name
        );
        miner::run_benchmark(
            &gpu,
            25,
            &std::sync::atomic::AtomicBool::new(false),
            &|stats| println!("{:.0} H/s (duty cycle 25%)", stats.hashes_per_second),
        )?;
        return Ok(());
    }
    let path = config::config_path()?;
    let config = config::Config::load(&path)?;
    if std::env::args().any(|arg| arg == "--check") {
        println!("Settings: {}", path.display());
        println!("Pool: {} ({})", config.pool.name(), config.pool.endpoint());
        println!("GPU duty cycle: {}%", config.gpu_load);
        println!(
            "Reward address: {}",
            if config.bitcoin_address.is_empty() {
                "not configured"
            } else {
                &config.bitcoin_address
            }
        );
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        startup::run(
            || objc2::rc::autoreleasepool(|_| gpu::Gpu::new().map(drop)),
            startup::show_error,
            || desktop::run(config, path),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!(
            "Mining requires macOS and Metal. Only --check is available on this platform."
        )
    }
}
