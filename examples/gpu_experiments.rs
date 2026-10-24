use solo_miner::{config, pow};
#[path = "gpu_experiments/engine.rs"]
mod engine;
#[allow(dead_code)]
#[path = "gpu_experiments/baseline.rs"]
mod production;

use anyhow::{Context, Result, ensure};
use bitcoin::hashes::{Hash, sha256};
use engine::{Engine, Output, VARIANTS, Variant, Work};
use objc2_foundation::NSProcessInfo;
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs::File,
    io::Write,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
struct Sample {
    kind: &'static str,
    round: usize,
    variant: String,
    batch: u32,
    completed_hashes: u64,
    commands: u64,
    seconds: f64,
    mh_s: f64,
    gpu_command_ms: Option<f64>,
    gpu_gap_ms: Option<f64>,
    cpu_wait_ms: Option<f64>,
    latency_ms: Option<f64>,
    drain_ms: Option<f64>,
    thermal_before: isize,
    thermal_after: isize,
    started_unix: f64,
}
fn thermal() -> isize {
    NSProcessInfo::processInfo().thermalState().0
}
fn unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
struct Cursor {
    header: [u8; 80],
    target: [u8; 32],
    nonce: u64,
    generation: u32,
}
impl Cursor {
    fn new() -> Result<Self> {
        Ok(Self {
            header: pow::genesis_header(),
            target: pow::share_target(1.0)?,
            nonce: 0,
            generation: 0,
        })
    }
    fn next(&mut self, batch: u32) -> Work {
        if self.nonce > u64::from(u32::MAX) {
            self.nonce = 0;
            self.generation += 1;
            self.header[68..72].copy_from_slice(&self.generation.to_le_bytes());
        }
        let count = u64::from(batch).min(u64::from(u32::MAX) + 1 - self.nonce) as u32;
        let work = Work {
            header: self.header,
            target: self.target,
            start: self.nonce as u32,
            count,
        };
        self.nonce += u64::from(count);
        work
    }
}
fn candidates(work: Work, raw: &[u32]) -> Result<()> {
    ensure!(
        raw[0] as usize == raw.len() - 1,
        "Candidate overflow during timed sample"
    );
    let mut sorted = raw[1..].to_vec();
    sorted.sort_unstable();
    ensure!(
        sorted.windows(2).all(|p| p[0] != p[1]),
        "Duplicate candidate"
    );
    let mut header = work.header;
    for nonce in sorted {
        ensure!(
            nonce >= work.start && u64::from(nonce) < u64::from(work.start) + u64::from(work.count),
            "Candidate outside assigned range"
        );
        header[76..80].copy_from_slice(&nonce.to_le_bytes());
        ensure!(
            pow::meets(&pow::hash(&header), &work.target),
            "Invalid candidate"
        );
    }
    Ok(())
}
fn validate(gpu: &Engine) -> Result<()> {
    let header = pow::genesis_header();
    for count in [1, 2, 3, 31, 32, 33, 63, 65, 127] {
        let start = u32::MAX - count + 1;
        let work = Work {
            header,
            target: [255; 32],
            start,
            count,
        };
        let out = gpu.submit(0, work, true)?.finish()?;
        for (i, words) in out.raw.as_chunks::<8>().0.iter().enumerate() {
            let mut probe = header;
            probe[76..80].copy_from_slice(&(start + i as u32).to_le_bytes());
            ensure!(
                words
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .eq(pow::hash(&probe)),
                "Digest mismatch: {} / {count}/{i}",
                gpu.variant.name
            );
        }
        ensure!(
            out.raw.len() == count as usize * 8,
            "Missing diagnostic digests"
        );
    }
    for seed in [0u8, 73, 255] {
        let header = [seed; 80];
        let mut probe = header;
        probe[76..80].copy_from_slice(&37u32.to_le_bytes());
        let target = pow::hash(&probe);
        let expected: Vec<_> = (0..4097u32)
            .filter(|n| {
                probe[76..80].copy_from_slice(&n.to_le_bytes());
                pow::meets(&pow::hash(&probe), &target)
            })
            .collect();
        ensure!(
            gpu.scan(Work {
                header,
                target,
                start: 0,
                count: 4097
            })? == expected,
            "Candidate mismatch/overflow: {}",
            gpu.variant.name
        );
    }
    let start = u32::MAX - 2048;
    ensure!(
        gpu.scan(Work {
            header,
            target: [255; 32],
            start,
            count: 2049
        })? == (start..=u32::MAX).collect::<Vec<_>>(),
        "Lost boundary candidates"
    );
    let a = Work {
        header,
        target: [255; 32],
        start: 11,
        count: 65,
    };
    let b = Work {
        header: [17; 80],
        target: [255; 32],
        start: 71,
        count: 127,
    };
    let first = gpu.submit(0, a, false)?;
    ensure!(
        gpu.submit(0, b, false).is_err(),
        "In-flight output was reused"
    );
    let second = gpu.submit(1, b, false)?;
    let out_a = first.finish()?;
    let out_b = second.finish()?;
    for out in [out_a, out_b] {
        candidates(out.work, &out.raw)?;
        let mut found = out.raw[1..].to_vec();
        found.sort_unstable();
        ensure!(
            found == (out.work.start..out.work.start + out.work.count).collect::<Vec<_>>(),
            "Cross-job or double-buffer corruption"
        );
    }
    let stale = gpu.submit(0, a, false)?;
    drop(stale);
    candidates(b, &gpu.submit(0, b, false)?.finish()?.raw)?;
    ensure!(
        gpu.submit(
            0,
            Work {
                start: u32::MAX,
                count: 2,
                ..a
            },
            false
        )
        .is_err(),
        "Accepted wrapping range"
    );
    Ok(())
}
struct Accumulator {
    hashes: u64,
    commands: u64,
    gpu: f64,
    gap: f64,
    wait: f64,
    latency: f64,
    last_end: Option<f64>,
    timed: u64,
}
impl Accumulator {
    fn new() -> Self {
        Self {
            hashes: 0,
            commands: 0,
            gpu: 0.0,
            gap: 0.0,
            wait: 0.0,
            latency: 0.0,
            last_end: None,
            timed: 0,
        }
    }
    fn add(&mut self, out: Output) -> Result<()> {
        candidates(out.work, &out.raw)?;
        self.hashes += u64::from(out.work.count);
        self.commands += 1;
        self.wait += out.wait_seconds;
        self.latency += out.latency_seconds;
        if out.gpu_start > 0.0 && out.gpu_end >= out.gpu_start {
            self.gpu += out.gpu_end - out.gpu_start;
            self.timed += 1;
            if let Some(end) = self.last_end {
                self.gap += (out.gpu_start - end).max(0.0);
            }
            self.last_end = Some(out.gpu_end);
        }
        Ok(())
    }
}
fn sample_production(gpu: &production::Gpu, batch: u32, seconds: f64) -> Result<Sample> {
    let mut cursor = Cursor::new()?;
    let start = Instant::now();
    let thermal_before = thermal();
    let started_unix = unix();
    let mut hashes = 0;
    let mut commands = 0;
    while start.elapsed().as_secs_f64() < seconds {
        let work = cursor.next(batch);
        let found = gpu.scan(&work.header, &work.target, work.start, work.count)?;
        for nonce in found {
            let mut h = work.header;
            h[76..80].copy_from_slice(&nonce.to_le_bytes());
            ensure!(
                pow::meets(&pow::hash(&h), &work.target),
                "Production candidate mismatch"
            );
        }
        hashes += u64::from(work.count);
        commands += 1;
    }
    let seconds = start.elapsed().as_secs_f64();
    Ok(Sample {
        kind: "sample",
        round: 0,
        variant: "production".into(),
        batch,
        completed_hashes: hashes,
        commands,
        seconds,
        mh_s: hashes as f64 / seconds / 1e6,
        gpu_command_ms: None,
        gpu_gap_ms: None,
        cpu_wait_ms: None,
        latency_ms: None,
        drain_ms: None,
        thermal_before,
        thermal_after: thermal(),
        started_unix,
    })
}
fn sample(gpu: &Engine, batch: u32, seconds: f64) -> Result<Sample> {
    let mut cursor = Cursor::new()?;
    let mut sum = Accumulator::new();
    let thermal_before = thermal();
    let started_unix = unix();
    let start = Instant::now();
    let mut drain = None;
    if gpu.variant.split {
        while start.elapsed().as_secs_f64() < seconds {
            let work = cursor.next(batch);
            let half = work.count.div_ceil(2);
            let a = gpu.submit(
                0,
                Work {
                    count: half,
                    ..work
                },
                false,
            )?;
            let b = if work.count > half {
                Some(gpu.submit(
                    1,
                    Work {
                        start: work.start + half,
                        count: work.count - half,
                        ..work
                    },
                    false,
                )?)
            } else {
                None
            };
            sum.add(a.finish()?)?;
            if let Some(b) = b {
                sum.add(b.finish()?)?;
            }
        }
    } else {
        let mut pending = VecDeque::new();
        for slot in 0..gpu.variant.depth {
            pending.push_back((slot, gpu.submit(slot, cursor.next(batch), false)?));
        }
        while let Some((slot, command)) = pending.pop_front() {
            sum.add(command.finish()?)?;
            if start.elapsed().as_secs_f64() < seconds {
                pending.push_back((slot, gpu.submit(slot, cursor.next(batch), false)?));
            } else if drain.is_none() {
                drain = Some(Instant::now());
            }
        }
    }
    let seconds = start.elapsed().as_secs_f64();
    let n = sum.commands as f64;
    Ok(Sample {
        kind: "sample",
        round: 0,
        variant: gpu.variant.name.into(),
        batch,
        completed_hashes: sum.hashes,
        commands: sum.commands,
        seconds,
        mh_s: sum.hashes as f64 / seconds / 1e6,
        gpu_command_ms: (sum.timed > 0).then_some(sum.gpu / sum.timed as f64 * 1000.0),
        gpu_gap_ms: (sum.timed > 1).then_some(sum.gap / (sum.timed - 1) as f64 * 1000.0),
        cpu_wait_ms: Some(sum.wait / n * 1000.0),
        latency_ms: Some(sum.latency / n * 1000.0),
        drain_ms: drain.map(|d| d.elapsed().as_secs_f64() * 1000.0),
        thermal_before,
        thermal_after: thermal(),
        started_unix,
    })
}
fn option<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.windows(2).find(|a| a[0] == key).map(|a| a[1].as_str())
}
fn write_record(file: &mut File, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *file, value)?;
    writeln!(file)?;
    file.flush()?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let duration: f64 = option(&args, "--seconds").unwrap_or("2").parse()?;
    let warmup: f64 = option(&args, "--warmup").unwrap_or("1").parse()?;
    let rounds: usize = option(&args, "--rounds").unwrap_or("5").parse()?;
    ensure!(
        duration > 0.0
            && duration <= 30.0
            && (0.0..=10.0).contains(&warmup)
            && rounds > 0
            && rounds <= 20,
        "Invalid timing parameters"
    );
    let batches: Vec<u32> = option(&args, "--batches")
        .unwrap_or("65536,262144,1048576")
        .split(',')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(
        batches.iter().all(|n| *n > 0 && *n <= 1_048_576),
        "Invalid batch size"
    );
    let variants: Vec<Variant> = VARIANTS
        .into_iter()
        .filter(|v| {
            option(&args, "--variants")
                .is_none_or(|list| list.split(',').any(|name| name == v.name))
        })
        .collect();
    ensure!(!variants.is_empty(), "No variants selected");
    let output = option(&args, "--out").unwrap_or("target/gpu-experiments.jsonl");
    let mut file = File::create(output).context("Output file")?;
    let prod = production::Gpu::new()?;
    let mut engines = Vec::new();
    for variant in variants {
        let gpu = Engine::new(variant)?;
        validate(&gpu)?;
        println!(
            "correctness OK: {} (SIMD {}, max {})",
            variant.name, gpu.width, gpu.max_threads
        );
        engines.push(gpu);
    }
    let os = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()?;
    write_record(
        &mut file,
        &serde_json::json!({"kind":"metadata","gpu":prod.name,"os":String::from_utf8_lossy(&os.stdout).trim(),"production_default_batch":prod.batch_size(),"seconds":duration,"warmup":warmup,"rounds":rounds,"batches":batches,"variants":engines.iter().map(|g|serde_json::json!({"variant":g.variant,"gpu":g.name,"simd_width":g.width,"max_threads":g.max_threads})).collect::<Vec<_>>(),"correctness":"passed","thermal_states":{"0":"nominal","1":"fair","2":"serious","3":"critical"},"created_unix":unix(),"production_shader_sha256":sha256::Hash::hash(include_bytes!("gpu_experiments/baseline.metal")).to_string(),"experimental_shader_sha256":sha256::Hash::hash(include_bytes!("gpu_experiments/two_nonce.metal")).to_string(),"harness_sha256":sha256::Hash::hash(include_bytes!("gpu_experiments.rs")).to_string(),"engine_sha256":sha256::Hash::hash(include_bytes!("gpu_experiments/engine.rs")).to_string()}),
    )?;
    if args.iter().any(|a| a == "--check-only") {
        return Ok(());
    }
    for batch in batches {
        for round in 0..rounds {
            if warmup > 0.0 {
                sample_production(&prod, batch, warmup)?;
            }
            let mut anchor = sample_production(&prod, batch, duration)?;
            anchor.round = round;
            anchor.variant = "production_before".into();
            println!(
                "{batch} round {round} {}: {:.2} MH/s",
                anchor.variant, anchor.mh_s
            );
            write_record(&mut file, &anchor)?;
            // Rotate order; reverse every other round to reduce ordering/thermal bias.
            let mut order: Vec<_> = (0..engines.len())
                .map(|i| (i + round) % engines.len())
                .collect();
            if round % 2 == 1 {
                order.reverse();
            }
            for index in order {
                let engine = &engines[index];
                if warmup > 0.0 {
                    sample(engine, batch, warmup)?;
                }
                let mut value = sample(engine, batch, duration)?;
                value.round = round;
                println!(
                    "{batch} round {round} {}: {:.2} MH/s (thermal {}→{})",
                    value.variant, value.mh_s, value.thermal_before, value.thermal_after
                );
                write_record(&mut file, &value)?;
            }
            if warmup > 0.0 {
                sample_production(&prod, batch, warmup)?;
            }
            let mut anchor = sample_production(&prod, batch, duration)?;
            anchor.round = round;
            anchor.variant = "production_after".into();
            println!(
                "{batch} round {round} {}: {:.2} MH/s",
                anchor.variant, anchor.mh_s
            );
            write_record(&mut file, &anchor)?;
        }
    }
    println!("Results: {output}");
    Ok(())
}
