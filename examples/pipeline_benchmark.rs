//! Compare the app's integrated pipeline/duty-cycle benchmark with v0.6.0.
use solo_miner::{config, pool, pow};
#[allow(dead_code)]
#[path = "gpu_experiments/baseline.rs"]
mod baseline;
#[allow(dead_code)]
#[path = "../src/gpu.rs"]
mod gpu;
#[allow(dead_code)]
#[path = "../src/miner.rs"]
mod miner;

use anyhow::{Result, ensure};
use bitcoin::hashes::{Hash, sha256};
use objc2_foundation::NSProcessInfo;
use serde_json::json;
use std::{
    cell::Cell,
    fs::File,
    io::Write,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn thermal() -> isize {
    NSProcessInfo::processInfo().thermalState().0
}
fn legacy(gpu: &baseline::Gpu, load: u8, seconds: f64) -> Result<f64> {
    let started = Instant::now();
    let mut total = 0u64;
    let mut nonce = 0u32;
    while started.elapsed().as_secs_f64() < seconds {
        let batch_start = Instant::now();
        gpu.scan(&pow::genesis_header(), &[0; 32], nonce, gpu.batch_size())?;
        nonce = nonce.wrapping_add(gpu.batch_size());
        total += u64::from(gpu.batch_size());
        let duration = batch_start
            .elapsed()
            .mul_f64((100.0 - f64::from(load)) / f64::from(load));
        // Same pause granularity as the v0.6.0 mining worker.
        let pause = Instant::now();
        while pause.elapsed() < duration {
            std::thread::sleep(
                duration
                    .saturating_sub(pause.elapsed())
                    .min(Duration::from_millis(20)),
            );
        }
    }
    Ok(total as f64 / started.elapsed().as_secs_f64() / 1e6)
}
fn record(file: &mut File, value: serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *file, &value)?;
    writeln!(file)?;
    file.flush()?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let option = |key: &str| args.windows(2).find(|a| a[0] == key).map(|a| a[1].as_str());
    let rounds: usize = option("--rounds").unwrap_or("3").parse()?;
    ensure!((1..=10).contains(&rounds), "Invalid rounds");
    let loads: Vec<u8> = option("--loads")
        .unwrap_or("100,85")
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        !loads.is_empty() && loads.iter().all(|l| (1..=100).contains(l)),
        "Invalid loads"
    );
    let mut file = File::create(option("--out").unwrap_or("target/pipeline-benchmark.jsonl"))?;
    record(
        &mut file,
        json!({"kind":"metadata","rounds":rounds,"seconds":3,"loads":loads,"legacy_warmup_seconds":0.5,"burst_ms":miner::DUTY_BURST_MS,
        "gpu_source_sha256":sha256::Hash::hash(include_bytes!("../src/gpu.rs")).to_string(),
        "miner_source_sha256":sha256::Hash::hash(include_bytes!("../src/miner.rs")).to_string(),
        "shader_sha256":sha256::Hash::hash(include_bytes!("../src/sha256.metal")).to_string(),
        "os":String::from_utf8_lossy(&std::process::Command::new("sw_vers").arg("-productVersion").output()?.stdout).trim()}),
    )?;
    for profile in config::GpuProfile::ALL {
        let old = baseline::Gpu::with_profile(profile)?;
        let new = gpu::Gpu::with_profile(profile)?;
        for &load in &loads {
            for round in 0..rounds {
                for variant in ["before", "pipeline", "after"] {
                    if variant != "pipeline" {
                        legacy(&old, load, 0.5)?;
                    }
                    let thermal_before = thermal();
                    let start = Instant::now();
                    let mh_s = if variant == "pipeline" {
                        let speed = Cell::new(0.0);
                        miner::run_benchmark(&new, load, &AtomicBool::new(false), &|s| {
                            speed.set(s.hashes_per_second)
                        })?;
                        speed.get() / 1e6
                    } else {
                        legacy(&old, load, 3.0)?
                    };
                    let value = json!({"kind":"sample","gpu":new.name,"profile":profile.label(),"batch":profile.batch_size(),
                        "load":load,"round":round,"variant":variant,"mh_s":mh_s,"wall_seconds":start.elapsed().as_secs_f64(),
                        "thermal_before":thermal_before,"thermal_after":thermal()});
                    println!("{value}");
                    record(&mut file, value)?;
                }
            }
        }
    }
    Ok(())
}
