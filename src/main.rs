mod activity;
mod config;
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
mod pool;
mod pow;
#[cfg(target_os = "macos")]
mod tray;

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if std::env::args().any(|arg| arg == "--gpu-check") {
        let gpu = gpu::Gpu::new()?;
        println!("GPU: {} — SHA-256d e confronto target verificati", gpu.name);
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
        println!("Configurazione: {}", path.display());
        println!("Pool: {} ({})", config.pool.name(), config.pool.endpoint());
        println!("GPU duty cycle: {}%", config.gpu_load);
        println!(
            "Indirizzo reward: {}",
            if config.bitcoin_address.is_empty() {
                "non configurato"
            } else {
                &config.bitcoin_address
            }
        );
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    desktop::run(config, path)?;
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("L'interfaccia richiede macOS. Usa --check per controllare la configurazione.");
    Ok(())
}
