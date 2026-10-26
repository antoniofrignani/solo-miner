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
mod icon;
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
#[cfg(target_os = "macos")]
mod updater;

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(plan) = std::env::args_os()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|args| args[0] == "--apply-update")
    {
        return updater::apply_update(std::path::Path::new(&plan[1]));
    }
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
    #[cfg(target_os = "macos")]
    if std::env::args().any(|arg| arg == "--gpu-profile") {
        for profile in config::GpuProfile::ALL {
            let gpu = gpu::Gpu::with_profile(profile)?;
            let speed = std::cell::Cell::new(0.0);
            miner::run_benchmark(
                &gpu,
                100,
                &std::sync::atomic::AtomicBool::new(false),
                &|stats| speed.set(stats.hashes_per_second),
            )?;
            println!(
                "{} · {} · {} nonces/batch · {:.2} MH/s (100% duty)",
                gpu.name,
                profile.label(),
                profile.batch_size(),
                speed.get() / 1e6
            );
        }
        return Ok(());
    }
    let path = config::config_path()?;
    let config = config::Config::load(&path)?;
    if std::env::args().any(|arg| arg == "--check") {
        println!("Settings: {}", path.display());
        println!("Pool: {} ({})", config.pool.name(), config.endpoint());
        println!("Mode: {:?}", config.mining_mode);
        println!("GPU duty cycle: {}%", config.gpu_load);
        println!("GPU workload: {}", config.gpu_profile.label());
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
