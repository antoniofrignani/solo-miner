use crate::{
    config::{Config, validate_address},
    gpu::Gpu,
    pow::{self, Job},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub state: String,
    pub gpu: String,
    pub hashes_per_second: f64,
    pub network_difficulty: Option<f64>,
    pub share_difficulty: Option<f64>,
    pub accepted: u64,
    pub rejected: u64,
    pub block_candidates: u64,
    pub event: Option<MinerEvent>,
}

#[derive(Clone, Debug)]
pub enum MinerEvent {
    AcceptedShare,
    BlockCandidate,
    Error(String),
}

pub struct Handle {
    stop: Arc<AtomicBool>,
}
impl Handle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn(
    config: Config,
    benchmark: bool,
    notify: impl Fn(Stats) + Send + 'static,
    finished: impl Fn(Result<()>) + Send + 'static,
) -> Handle {
    let stop = Arc::new(AtomicBool::new(false));
    let cancel = stop.clone();
    thread::spawn(move || {
        let result = (|| {
            config.validate()?;
            if !benchmark {
                validate_address(&config.bitcoin_address)?;
            }
            notify(Stats {
                state: "Starting GPU…".into(),
                ..Stats::default()
            });
            let gpu = Gpu::with_profile(config.gpu_profile)?;
            if benchmark {
                return run_benchmark(&gpu, config.gpu_load, &cancel, &notify);
            }
            while !cancel.load(Ordering::Relaxed) {
                notify(Stats {
                    state: "Connecting to pool…".into(),
                    gpu: gpu.name.clone(),
                    ..Stats::default()
                });
                let result = connect(config.pool.endpoint(), &cancel)
                    .and_then(|stream| session(stream, &config, &cancel, &gpu.name, &notify, &gpu));
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(error) = result {
                    if error.downcast_ref::<GpuFailure>().is_some() {
                        return Err(error);
                    }
                    notify(Stats {
                        state: format!("Reconnecting in 5 s: {error:#}"),
                        event: Some(MinerEvent::Error(format!("{error:#}"))),
                        gpu: gpu.name.clone(),
                        ..Stats::default()
                    });
                    pause(Duration::from_secs(5), &cancel);
                }
            }
            Ok(())
        })();
        finished(result);
    });
    Handle { stop }
}

pub fn run_benchmark(
    gpu: &Gpu,
    load: u8,
    stop: &AtomicBool,
    notify: &impl Fn(Stats),
) -> Result<()> {
    ensure!((1..=100).contains(&load), "Invalid GPU duty cycle");
    let batch = gpu.batch_size();
    let start = Instant::now();
    let mut total = 0u64;
    let mut reported = Instant::now();
    let mut nonce = 0u64;
    let mut header = pow::genesis_header();
    let mut generation = 0u32;
    let mut pending = VecDeque::new();
    let mut duty = DutyCycle::new(load);
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        while pending.len() < 2 && start.elapsed() < Duration::from_secs(3) && duty.may_submit() {
            if nonce > u64::from(u32::MAX) {
                nonce = 0;
                generation += 1;
                header[68..72].copy_from_slice(&generation.to_le_bytes());
            }
            let count = u64::from(batch).min(u64::from(u32::MAX) + 1 - nonce) as u32;
            duty.submitted();
            pending.push_back((count, gpu.submit(&header, &[0; 32], nonce as u32, count)?));
            nonce += u64::from(count);
        }
        if let Some((count, command)) = pending.pop_front() {
            gpu.complete(command)?;
            total += u64::from(count);
            if pending.is_empty() {
                duty.drained(stop);
            }
        } else if start.elapsed() >= Duration::from_secs(3) {
            break;
        }
        if reported.elapsed() >= Duration::from_millis(250) {
            notify(Stats {
                state: "Local SHA-256d benchmark (3 s)".into(),
                gpu: gpu.name.clone(),
                hashes_per_second: total as f64 / start.elapsed().as_secs_f64(),
                ..Stats::default()
            });
            reported = Instant::now();
        }
    }
    drop(pending); // Stop also waits for queued work before releasing the GPU.
    notify(Stats {
        state: "Local SHA-256d benchmark (3 s)".into(),
        gpu: gpu.name.clone(),
        hashes_per_second: total as f64 / start.elapsed().as_secs_f64(),
        ..Stats::default()
    });
    Ok(())
}

fn pause(duration: Duration, stop: &AtomicBool) {
    let start = Instant::now();
    while start.elapsed() < duration && !stop.load(Ordering::Relaxed) {
        thread::sleep(
            duration
                .saturating_sub(start.elapsed())
                .min(Duration::from_millis(20)),
        );
    }
}
fn throttle(work: Duration, load: u8, stop: &AtomicBool) {
    pause(
        work.mul_f64((100.0 - f64::from(load)) / f64::from(load)),
        stop,
    );
}
fn connect(endpoint: &str, stop: &AtomicBool) -> Result<TcpStream> {
    let host = endpoint
        .strip_prefix("stratum+tcp://")
        .context("Unsupported endpoint")?;
    let addresses = host.to_socket_addrs().context("DNS lookup failed")?;
    let mut failures = Vec::new();
    for address in addresses.take(8) {
        ensure!(!stop.load(Ordering::Relaxed), "Stopped");
        match TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
            Ok(stream) => return Ok(stream),
            Err(error) => failures.push(format!("{address}: {error}")),
        }
    }
    bail!(
        "TCP connection to {host} failed: {}",
        if failures.is_empty() {
            "DNS returned no addresses".into()
        } else {
            failures.join("; ")
        }
    )
}

#[derive(Debug)]
struct GpuFailure;
impl std::fmt::Display for GpuFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GPU error: mining stopped")
    }
}
impl std::error::Error for GpuFailure {}

struct Wire {
    stream: TcpStream,
    input: Vec<u8>,
    output: VecDeque<u8>,
}
impl Wire {
    fn new(stream: TcpStream) -> Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            input: Vec::new(),
            output: VecDeque::new(),
        })
    }
    fn send(&mut self, value: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&value)?;
        ensure!(
            self.output.len() + bytes.len() < 1_048_576,
            "Stratum queue full"
        );
        self.output.extend(bytes);
        self.output.push_back(b'\n');
        Ok(())
    }
    fn poll(&mut self) -> Result<Vec<Value>> {
        while !self.output.is_empty() {
            let bytes = self.output.make_contiguous();
            match self.stream.write(bytes) {
                Ok(0) => bail!("Connection closed while writing"),
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
        }
        let mut bytes = [0u8; 16384];
        // Bound network work per loop so stop and hashing cannot be starved by a peer.
        for _ in 0..64 {
            match self.stream.read(&mut bytes) {
                Ok(0) => bail!("Pool disconnected"),
                Ok(n) => self.input.extend_from_slice(&bytes[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.into()),
            }
            ensure!(self.input.len() <= 1_048_576, "Stratum message too large");
        }
        let mut messages = Vec::new();
        while let Some(end) = self.input.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.input.drain(..=end).collect();
            if line.iter().any(|b| !b.is_ascii_whitespace()) {
                messages.push(serde_json::from_slice(&line)?);
            }
        }
        Ok(messages)
    }
}

/// SoloPool accepts 10 but silently ignores 10.0.
fn difficulty_request(difficulty: f64) -> Value {
    let value = if difficulty.fract() == 0.0 && (1.0..=1e12).contains(&difficulty) {
        json!(difficulty as u64)
    } else {
        json!(difficulty)
    };
    json!({"id":3,"method":"mining.suggest_difficulty","params":[value]})
}

trait Scanner {
    type Pending;
    fn submit(
        &mut self,
        header: &[u8; 80],
        target: &[u8; 32],
        start: u32,
        count: u32,
    ) -> Result<Self::Pending>;
    fn complete(&mut self, pending: Self::Pending) -> Result<Vec<u32>>;
}
impl Scanner for &Gpu {
    type Pending = crate::gpu::Pending;
    fn submit(&mut self, h: &[u8; 80], t: &[u8; 32], s: u32, n: u32) -> Result<Self::Pending> {
        Gpu::submit(self, h, t, s, n)
    }
    fn complete(&mut self, pending: Self::Pending) -> Result<Vec<u32>> {
        Gpu::complete(self, pending)
    }
}
#[cfg(test)]
struct SyncScan<F>(F);
#[cfg(test)]
impl<F: FnMut(&[u8; 80], &[u8; 32], u32, u32) -> Result<Vec<u32>>> Scanner for SyncScan<F> {
    type Pending = Vec<u32>;
    fn submit(&mut self, h: &[u8; 80], t: &[u8; 32], s: u32, n: u32) -> Result<Self::Pending> {
        (self.0)(h, t, s, n)
    }
    fn complete(&mut self, pending: Self::Pending) -> Result<Vec<u32>> {
        Ok(pending)
    }
}

struct InFlight<P> {
    pending: P,
    job: Job,
    target: [u8; 32],
    start: u32,
    count: u32,
    epoch: u64,
}

// Pause only after draining a short burst, so no GPU work runs during the pause.
struct DutyCycle {
    load: u8,
    started: Option<Instant>,
}
pub(crate) const DUTY_BURST_MS: u64 = 50;
impl DutyCycle {
    fn new(load: u8) -> Self {
        Self {
            load,
            started: None,
        }
    }
    fn may_submit(&self) -> bool {
        self.load == 100
            || self
                .started
                .is_none_or(|s| s.elapsed() < Duration::from_millis(DUTY_BURST_MS))
    }
    fn submitted(&mut self) {
        self.started.get_or_insert_with(Instant::now);
    }
    fn drained(&mut self, stop: &AtomicBool) {
        if let Some(start) = self.started.take() {
            throttle(start.elapsed(), self.load, stop);
        }
    }
}

fn session(
    mut_stream: TcpStream,
    config: &Config,
    stop: &AtomicBool,
    gpu_name: &str,
    notify: &impl Fn(Stats),
    mut scan: impl Scanner,
) -> Result<()> {
    session_inner(mut_stream, config, stop, gpu_name, notify, &mut scan)
}
fn session_inner<S: Scanner>(
    stream: TcpStream,
    config: &Config,
    stop: &AtomicBool,
    gpu_name: &str,
    notify: &impl Fn(Stats),
    scan: &mut S,
) -> Result<()> {
    let mut wire = Wire::new(stream)?;
    wire.send(json!({"id":1,"method":"mining.subscribe","params":[format!("solo-miner/{}",env!("CARGO_PKG_VERSION"))]}))?;
    // SoloPool resets difficulty during authorize. Other pools (including Public
    // Pool) need the suggestion before authorization starts job initialization.
    if config.pool != crate::pool::Pool::Solopool {
        wire.send(difficulty_request(config.share_difficulty))?;
    }
    let username = format!("{}.mac", config.bitcoin_address);
    let mut subscribed = false;
    let mut authorized = false;
    let mut extra1 = Vec::new();
    let mut extra_size = 0usize;
    let mut counter = 0u128;
    let mut next_diff = 1.0;
    let mut params = Value::Null;
    let mut job: Option<Job> = None;
    let mut nonce = 0u64;
    let mut epoch = 0u64;
    let mut in_flight: VecDeque<InFlight<S::Pending>> = VecDeque::new();
    let mut duty = DutyCycle::new(config.gpu_load);
    let mut next_id = 10u64;
    let mut pending = HashMap::new();
    let started = Instant::now();
    let mut last_job = Instant::now();
    let mut measured = Instant::now();
    let mut hashes = 0u64;
    let mut stats = Stats {
        state: "Stratum handshake…".into(),
        gpu: gpu_name.into(),
        ..Stats::default()
    };
    while !stop.load(Ordering::Relaxed) {
        for message in wire.poll()? {
            match message["method"].as_str() {
                Some("mining.set_difficulty") => {
                    next_diff = message["params"][0]
                        .as_f64()
                        .context("Missing share difficulty")?;
                    let target = pow::share_target(next_diff)?;
                    // SoloPool can change difficulty without sending a new job.
                    if config.pool == crate::pool::Pool::Solopool
                        && let Some(current) = job.as_mut()
                    {
                        current.share_target = target;
                        current.share_difficulty = next_diff;
                        stats.share_difficulty = Some(next_diff);
                        notify(stats.clone());
                    }
                }
                Some("mining.notify") => {
                    ensure!(subscribed, "Job received before subscription");
                    params = message["params"].clone();
                    let extra2 = next_extra(&mut counter, extra_size)?;
                    let new_job = pow::build_job(&params, &extra1, &extra2, next_diff)?;
                    if params[8].as_bool() == Some(true) {
                        epoch = epoch.checked_add(1).context("Job generation exhausted")?;
                    }
                    stats.network_difficulty = Some(pow::difficulty(&new_job.network_target));
                    stats.share_difficulty = Some(next_diff);
                    job = Some(new_job);
                    nonce = 0;
                    last_job = Instant::now();
                    notify(stats.clone());
                }
                Some("mining.set_extranonce") | Some("client.reconnect") => {
                    bail!("Pool requested a new session")
                }
                Some("client.get_version") => {
                    wire.send(json!({"id":message["id"],"result":format!("solo-miner/{}",env!("CARGO_PKG_VERSION")),"error":null}))?
                }
                Some(_) => {}
                None => match message["id"].as_u64() {
                    Some(1) => {
                        ensure!(
                            message["error"].is_null(),
                            "Subscription rejected: {}",
                            message["error"]
                        );
                        extra1 = hex::decode(
                            message["result"][1]
                                .as_str()
                                .context("Missing extranonce1")?,
                        )?;
                        ensure!(extra1.len() <= 64, "Extranonce1 too long");
                        extra_size = message["result"][2]
                            .as_u64()
                            .context("Missing extranonce2 size")?
                            as usize;
                        ensure!(
                            (1..=16).contains(&extra_size),
                            "Unsupported extranonce2 size"
                        );
                        subscribed = true;
                        // SoloPool selects 100% solo mode with password x.
                        wire.send(
                            json!({"id":2,"method":"mining.authorize","params":[username,"x"]}),
                        )?;
                    }
                    Some(2) => {
                        ensure!(
                            message["result"].as_bool() == Some(true) && message["error"].is_null(),
                            "Authorization rejected: {}",
                            message["error"]
                        );
                        authorized = true;
                        if config.pool == crate::pool::Pool::Solopool {
                            wire.send(difficulty_request(config.share_difficulty))?;
                        }
                        stats.state = "Waiting for job…".into();
                        notify(stats.clone());
                    }
                    Some(id) if pending.remove(&id).is_some() => {
                        if message["result"].as_bool() == Some(true) && message["error"].is_null() {
                            stats.accepted += 1;
                            let mut event = stats.clone();
                            event.event = Some(MinerEvent::AcceptedShare);
                            notify(event);
                        } else {
                            stats.rejected += 1;
                            let mut event = stats.clone();
                            event.event = Some(MinerEvent::Error(format!(
                                "Share rejected by pool: {}",
                                message["error"]
                            )));
                            notify(event);
                        }
                        notify(stats.clone());
                    }
                    _ => {}
                },
            }
        }
        ensure!(
            authorized || started.elapsed() < Duration::from_secs(20),
            "Handshake timeout"
        );
        ensure!(
            last_job.elapsed() < Duration::from_secs(120),
            "No recent job: session expired"
        );
        ensure!(
            pending
                .values()
                .all(|time: &Instant| time.elapsed() < Duration::from_secs(30)),
            "Share response timeout"
        );
        if authorized && let Some(current) = job.as_mut() {
            while in_flight.len() < 2 && duty.may_submit() && !stop.load(Ordering::Relaxed) {
                if nonce > u64::from(u32::MAX) {
                    let extra2 = next_extra(&mut counter, extra_size)?;
                    *current = pow::build_job(&params, &extra1, &extra2, current.share_difficulty)?;
                    nonce = 0;
                }
                let count = u64::from(config.gpu_profile.batch_size())
                    .min(u64::from(u32::MAX) + 1 - nonce) as u32;
                // Always search network candidates, even if pool difficulty is harder.
                let target = easier_target(current.share_target, current.network_target);
                duty.submitted();
                let pending = scan
                    .submit(&current.header, &target, nonce as u32, count)
                    .context(GpuFailure)?;
                in_flight.push_back(InFlight {
                    pending,
                    job: current.clone(),
                    target,
                    start: nonce as u32,
                    count,
                    epoch,
                });
                nonce += u64::from(count);
            }
        }
        if let Some(work) = in_flight.pop_front() {
            let found = scan.complete(work.pending).context(GpuFailure)?;
            hashes += u64::from(work.count);
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // Clean jobs invalidate pending work; non-clean updates retain its original job.
            let valid = work.epoch == epoch;
            let current = &work.job;
            let target = work.target;
            for found_nonce in found {
                if found_nonce < work.start
                    || u64::from(found_nonce) >= u64::from(work.start) + u64::from(work.count)
                {
                    return Err(anyhow::anyhow!("GPU nonce outside assigned range"))
                        .context(GpuFailure);
                }
                let mut header = current.header;
                header[76..80].copy_from_slice(&found_nonce.to_le_bytes());
                let hash = pow::hash(&header);
                if !pow::meets(&hash, &target) {
                    return Err(anyhow::anyhow!("Invalid GPU result")).context(GpuFailure);
                }
                if !valid {
                    continue;
                }
                // SoloPool applies difficulty changes immediately to the existing job.
                let share_target = job
                    .as_ref()
                    .filter(|j| config.pool == crate::pool::Pool::Solopool && j.id == current.id)
                    .map_or(current.share_target, |j| j.share_target);
                if !pow::meets(&hash, &share_target) && !pow::meets(&hash, &current.network_target)
                {
                    continue;
                }
                if pow::meets(&hash, &current.network_target) {
                    stats.block_candidates += 1;
                    let mut event = stats.clone();
                    event.event = Some(MinerEvent::BlockCandidate);
                    notify(event);
                }
                ensure!(pending.len() < 1024, "Too many pending shares");
                wire.send(json!({"id":next_id,"method":"mining.submit","params":[username,current.id,current.extranonce2,current.ntime,format!("{found_nonce:08x}")]}))?;
                pending.insert(next_id, Instant::now());
                next_id += 1;
            }
            if in_flight.is_empty() {
                duty.drained(stop);
            }
            stats.state = if stats.accepted == 0 {
                "Mining · waiting for first share".into()
            } else {
                "Mining active".into()
            };
        } else {
            pause(Duration::from_millis(20), stop);
        }
        if measured.elapsed() >= Duration::from_secs(1) {
            stats.hashes_per_second = hashes as f64 / measured.elapsed().as_secs_f64();
            notify(stats.clone());
            hashes = 0;
            measured = Instant::now();
        }
    }
    Ok(())
}
fn easier_target(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a.iter().rev().cmp(b.iter().rev()).is_ge() {
        a
    } else {
        b
    }
}
fn next_extra(counter: &mut u128, size: usize) -> Result<Vec<u8>> {
    if size < 16 {
        ensure!(
            *counter < (1u128 << (size * 8)),
            "Extranonce2 exhausted: new session required"
        );
    }
    let bytes = counter.to_le_bytes()[..size].to_vec();
    *counter = counter.checked_add(1).context("Extranonce2 exhausted")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
    };
    #[test]
    fn fragmented_protocol_and_submit() -> Result<()> {
        fixture(false, crate::pool::Pool::Public)
    }
    #[test]
    #[ignore = "Requires a real Metal GPU; run explicitly on macOS"]
    fn gpu_to_stratum_share() -> Result<()> {
        fixture(true, crate::pool::Pool::Public)
    }
    #[test]
    fn solopool_suggests_after_authorization_and_updates_existing_job() -> Result<()> {
        fixture(false, crate::pool::Pool::Solopool)
    }
    fn fixture(use_gpu: bool, pool: crate::pool::Pool) -> Result<()> {
        let gpu = if use_gpu { Some(Gpu::new()?) } else { None };
        let diff = if use_gpu { 1e-6 } else { 1e-12 };
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let socket = TcpStream::connect(listener.local_addr()?)?;
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let config = Config {
            bitcoin_address: "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
            gpu_load: 100,
            pool,
            share_difficulty: if pool == crate::pool::Pool::Solopool {
                2.0
            } else {
                2.5
            },
            ..Config::default()
        };
        let server = thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut reader = BufReader::new(stream.try_clone()?);
            let mut line = String::new();
            reader.read_line(&mut line)?;
            assert_eq!(
                serde_json::from_str::<Value>(&line)?["method"],
                "mining.subscribe"
            );
            let requested = config.share_difficulty;
            let read_suggestion = |reader: &mut BufReader<TcpStream>| -> Result<()> {
                let mut line = String::new();
                reader.read_line(&mut line)?;
                let suggest: Value = serde_json::from_str(&line)?;
                assert_eq!(suggest["id"], 3);
                assert_eq!(suggest["method"], "mining.suggest_difficulty");
                if pool == crate::pool::Pool::Solopool {
                    assert_eq!(suggest["params"][0].as_u64(), Some(requested as u64));
                } else {
                    assert_eq!(suggest["params"], json!([requested]));
                }
                Ok(())
            };
            if pool != crate::pool::Pool::Solopool {
                read_suggestion(&mut reader)?;
            }
            stream.write_all(b"{\"id\":1,\"result\":[[],\"aa\",4],")?;
            thread::sleep(Duration::from_millis(30));
            stream.write_all(b"\"error\":null}\n")?;
            line.clear();
            reader.read_line(&mut line)?;
            let auth: Value = serde_json::from_str(&line)?;
            assert_eq!(auth["params"][0], "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa.mac");
            assert_eq!(auth["params"][1], "x");
            if pool == crate::pool::Pool::Solopool {
                // Real server sequence: initial work before auth reply, then only
                // a set_difficulty after the post-auth suggestion (no new job).
                writeln!(
                    stream,
                    "{}",
                    json!({"method":"mining.set_difficulty","params":[16384]})
                )?;
                writeln!(
                    stream,
                    "{}",
                    json!({"method":"mining.notify","params":["fixture","00".repeat(32),"01","02",[],"20000000","1d00ffff","495fab29",true]})
                )?;
            }
            writeln!(stream, "{}", json!({"id":2,"result":true,"error":null}))?;
            if pool == crate::pool::Pool::Solopool {
                read_suggestion(&mut reader)?;
            }
            writeln!(
                stream,
                "{}",
                json!({"method":"mining.set_difficulty","params":[diff]})
            )?;
            if pool != crate::pool::Pool::Solopool {
                writeln!(
                    stream,
                    "{}",
                    json!({"method":"mining.notify","params":["fixture","00".repeat(32),"01","02",[],"20000000","1d00ffff","495fab29",true]})
                )?;
            }

            line.clear();
            reader.read_line(&mut line)?;
            let submit: Value = serde_json::from_str(&line)?;
            assert_eq!(submit["method"], "mining.submit");
            assert_eq!(submit["params"][1], "fixture");
            assert_eq!(submit["params"][2], "00000000");
            assert_eq!(submit["params"][3], "495fab29");
            if !use_gpu && pool != crate::pool::Pool::Solopool {
                assert_eq!(submit["params"][4], "00000000");
            }
            let nonce = u32::from_str_radix(submit["params"][4].as_str().unwrap(), 16)?;
            let params = json!([
                "fixture",
                "00".repeat(32),
                "01",
                "02",
                [],
                "20000000",
                "1d00ffff",
                "495fab29",
                true
            ]);
            let mut job = pow::build_job(&params, &[0xaa], &[0, 0, 0, 0], diff)?;
            job.header[76..80].copy_from_slice(&nonce.to_le_bytes());
            assert!(pow::meets(&pow::hash(&job.header), &job.share_target));
            writeln!(
                stream,
                "{}",
                json!({"id":submit["id"],"result":true,"error":null})
            )?;
            while !done.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        });
        let notify = |stats: Stats| {
            if stats.accepted == 1 {
                stop.store(true, Ordering::Relaxed);
            }
        };
        let result = if let Some(gpu) = &gpu {
            session(socket, &config, &stop, "fixture", &notify, gpu)
        } else {
            let mut emitted = false;
            session(
                socket,
                &config,
                &stop,
                "fixture",
                &notify,
                SyncScan(|_: &[u8; 80], t: &[u8; 32], start, _| {
                    if !emitted && *t == [255; 32] {
                        emitted = true;
                        Ok(vec![start])
                    } else {
                        Ok(vec![])
                    }
                }),
            )
        };
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap()?;
        result
    }
    #[test]
    #[ignore = "Live SoloPool handshake; no GPU work or share submissions"]
    fn solopool_live_difficulty_handshake() -> Result<()> {
        let config = Config::load(&crate::config::config_path()?)?;
        ensure!(
            config.pool == crate::pool::Pool::Solopool,
            "Select SoloPool first"
        );
        let stop = Arc::new(AtomicBool::new(false));
        let deadline = stop.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(20));
            deadline.store(true, Ordering::Relaxed);
        });
        let seen = std::sync::Mutex::new(None);
        let stream = connect(config.pool.endpoint(), &stop)?;
        session(
            stream,
            &config,
            &stop,
            "handshake-only",
            &|stats| {
                if let Some(diff) = stats.share_difficulty {
                    eprintln!("Live server assigned difficulty: {diff}");
                    *seen.lock().unwrap() = Some(diff);
                    if diff == config.share_difficulty {
                        stop.store(true, Ordering::Relaxed);
                    }
                }
            },
            SyncScan(|_: &[u8; 80], _: &[u8; 32], _, _| Ok(vec![])),
        )?;
        ensure!(
            *seen.lock().unwrap() == Some(config.share_difficulty),
            "Requested difficulty not confirmed by production parser"
        );
        Ok(())
    }
    #[test]
    fn difficulty_wire_format_preserves_integer_type() {
        // Check the JSON type: as_f64 would accept both 10 and 10.0.
        let wire = serde_json::to_string(&difficulty_request(10.0)).unwrap();
        let parsed: Value = serde_json::from_str(&wire).unwrap();
        assert_eq!(parsed["params"][0].as_u64(), Some(10));
        assert!(wire.contains("[10]"));
        assert_eq!(difficulty_request(0.5)["params"][0].as_f64(), Some(0.5));
    }
    #[test]
    fn extranonce_does_not_repeat() {
        let mut value = 255;
        assert_eq!(next_extra(&mut value, 1).unwrap(), vec![255]);
        assert!(next_extra(&mut value, 1).is_err());
    }

    struct QueuedProbe {
        header: [u8; 80],
        target: [u8; 32],
        start: u32,
        live: std::rc::Rc<std::cell::Cell<usize>>,
    }
    impl Drop for QueuedProbe {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }
    type IssuedRanges = std::rc::Rc<std::cell::RefCell<Vec<([u8; 80], u32, u32)>>>;
    struct QueueProbe<'a> {
        server: TcpStream,
        client: TcpStream,
        stop: &'a AtomicBool,
        update: Value,
        completed: usize,
        fail: bool,
        live: std::rc::Rc<std::cell::Cell<usize>>,
        issued: IssuedRanges,
    }
    impl Scanner for QueueProbe<'_> {
        type Pending = QueuedProbe;
        fn submit(
            &mut self,
            header: &[u8; 80],
            target: &[u8; 32],
            start: u32,
            count: u32,
        ) -> Result<QueuedProbe> {
            self.live.set(self.live.get() + 1);
            assert!(self.live.get() <= 2);
            self.issued.borrow_mut().push((*header, start, count));
            Ok(QueuedProbe {
                header: *header,
                target: *target,
                start,
                live: self.live.clone(),
            })
        }
        fn complete(&mut self, pending: QueuedProbe) -> Result<Vec<u32>> {
            self.completed += 1;
            if self.completed == 1 {
                assert_eq!(
                    self.live.get(),
                    2,
                    "Second batch must be submitted before waiting for the first"
                );
                writeln!(self.server, "{}", self.update)?;
                // Wait for TCP delivery without consuming the update; the next
                // production Wire::poll must process it before retiring batch 2.
                let deadline = Instant::now();
                loop {
                    match self.client.peek(&mut [0u8; 1]) {
                        Ok(n) if n > 0 => break,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        other => bail!("Queue fixture delivery: {other:?}"),
                    }
                    ensure!(
                        deadline.elapsed() < Duration::from_secs(2),
                        "Queue fixture delivery timeout"
                    );
                    thread::yield_now();
                }
                return Ok(vec![]);
            }
            if self.completed == 2 && self.fail {
                bail!("Injected GPU failure");
            }
            if self.completed == 4 {
                self.stop.store(true, Ordering::Relaxed);
                return Ok(vec![]);
            }
            let mut header = pending.header;
            header[76..80].copy_from_slice(&pending.start.to_le_bytes());
            Ok(if pow::meets(&pow::hash(&header), &pending.target) {
                vec![pending.start]
            } else {
                vec![]
            })
        }
    }
    fn queued_transition(clean: Option<bool>, fail: bool) -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let client = TcpStream::connect(listener.local_addr()?)?;
        let (mut server, _) = listener.accept()?;
        server.set_nodelay(true)?;
        server.set_read_timeout(Some(Duration::from_secs(3)))?;
        let job_params = |id: &str, clean: bool| {
            json!([
                id,
                "00".repeat(32),
                "01",
                "02",
                [],
                "20000000",
                "1d00ffff",
                "495fab29",
                clean
            ])
        };
        for message in [
            json!({"id":1,"result":[[],"aa",4],"error":null}),
            json!({"id":2,"result":true,"error":null}),
            json!({"method":"mining.set_difficulty","params":[1e-12]}),
            json!({"method":"mining.notify","params":job_params("old",true)}),
        ] {
            writeln!(server, "{message}")?;
        }
        let update = clean.map_or_else(
            || json!({"method":"mining.set_difficulty","params":[1e12]}),
            |clean| json!({"method":"mining.notify","params":job_params("new",clean)}),
        );
        let config = Config {
            bitcoin_address: "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
            gpu_load: 100,
            pool: if clean.is_none() {
                crate::pool::Pool::Solopool
            } else {
                crate::pool::Pool::Public
            },
            ..Config::default()
        };
        let stop = AtomicBool::new(false);
        let live = std::rc::Rc::new(std::cell::Cell::new(0));
        let issued = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let scanner = QueueProbe {
            server: server.try_clone()?,
            client: client.try_clone()?,
            stop: &stop,
            update,
            completed: 0,
            fail,
            live: live.clone(),
            issued: issued.clone(),
        };
        let result = session(client, &config, &stop, "queue-probe", &|_| {}, scanner);
        if fail {
            assert!(result.unwrap_err().downcast_ref::<GpuFailure>().is_some());
        } else {
            result?;
        }
        assert_eq!(live.get(), 0, "Stop/error must drain all queued commands");
        let mut text = String::new();
        server.read_to_string(&mut text)?;
        let submits: Vec<Value> = text
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .filter(|v: &Value| v["method"] == "mining.submit")
            .collect();
        let ids: Vec<_> = submits
            .iter()
            .map(|v| v["params"][1].as_str().unwrap())
            .collect();
        match (fail, clean) {
            (true, _) | (_, None) => assert!(
                ids.is_empty(),
                "Invalid or harder-difficulty results must not be submitted"
            ),
            (_, Some(true)) => assert_eq!(ids, vec!["new"]),
            (_, Some(false)) => {
                assert_eq!(ids, vec!["old", "new"]);
                assert_eq!(submits[0]["params"][2], "00000000");
                assert_eq!(submits[1]["params"][2], "01000000");
            }
        }
        let issued = issued.borrow();
        assert_eq!(issued[0].1, 0);
        assert_eq!(issued[1].1, config.gpu_profile.batch_size());
        if clean.is_some() {
            assert_ne!(issued[0].0, issued[2].0);
            assert_eq!(issued[2].1, 0);
        }
        Ok(())
    }
    #[test]
    fn clean_job_discards_queued_old_shares_and_stop_drains() -> Result<()> {
        queued_transition(Some(true), false)
    }
    #[test]
    fn nonclean_job_submits_queued_results_with_original_job() -> Result<()> {
        queued_transition(Some(false), false)
    }
    #[test]
    fn harder_solopool_difficulty_filters_queued_shares() -> Result<()> {
        queued_transition(None, false)
    }
    #[test]
    fn gpu_failure_drains_other_pending_work() -> Result<()> {
        queued_transition(Some(true), true)
    }
    #[test]
    fn duty_cycle_drains_before_pause_and_stop_interrupts_pause() {
        let stop = AtomicBool::new(true);
        let mut duty = DutyCycle::new(85);
        duty.started = Some(Instant::now() - Duration::from_millis(DUTY_BURST_MS + 1));
        assert!(!duty.may_submit());
        let started = Instant::now();
        duty.drained(&stop);
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(duty.may_submit());
        let mut full = DutyCycle::new(100);
        full.started = Some(Instant::now() - Duration::from_secs(1));
        assert!(full.may_submit());
    }
    #[test]
    #[ignore = "Requires a real Metal GPU; verifies cancellation during work and duty-cycle pauses"]
    fn pipeline_benchmark_cancellation_releases_slots() -> Result<()> {
        for profile in crate::config::GpuProfile::ALL {
            let gpu = Gpu::with_profile(profile)?;
            for load in [5, 85, 100] {
                let stop = AtomicBool::new(false);
                let started = Instant::now();
                std::thread::scope(|scope| -> Result<()> {
                    scope.spawn(|| {
                        thread::sleep(Duration::from_millis(50));
                        stop.store(true, Ordering::Relaxed);
                    });
                    run_benchmark(&gpu, load, &stop, &|_| {})
                })?;
                assert!(
                    started.elapsed() < Duration::from_secs(1),
                    "Stop must interrupt pacing and drain promptly"
                );
                let a = gpu.submit(&pow::genesis_header(), &[255; 32], 0, 33)?;
                let b = gpu.submit(&[17; 80], &[255; 32], u32::MAX - 64, 65)?;
                assert_eq!(gpu.complete(a)?, (0..33).collect::<Vec<_>>());
                assert_eq!(
                    gpu.complete(b)?,
                    (u32::MAX - 64..=u32::MAX).collect::<Vec<_>>()
                );
            }
        }
        Ok(())
    }
    struct WrapProbe<'a> {
        stop: &'a AtomicBool,
        header: Option<[u8; 80]>,
        next: u64,
        generations: usize,
        new_completed: usize,
    }
    impl Scanner for WrapProbe<'_> {
        type Pending = ([u8; 80], u32);
        fn submit(
            &mut self,
            h: &[u8; 80],
            _: &[u8; 32],
            start: u32,
            count: u32,
        ) -> Result<Self::Pending> {
            if self.header.is_some_and(|old| old != *h) {
                assert_eq!(
                    self.next,
                    u64::from(u32::MAX) + 1,
                    "Entire nonce space must be reserved exactly once"
                );
                self.next = 0;
                self.generations += 1;
            }
            self.header = Some(*h);
            assert_eq!(
                u64::from(start),
                self.next,
                "Queued ranges must not overlap or skip nonces"
            );
            self.next += u64::from(count);
            assert!(self.next <= u64::from(u32::MAX) + 1);
            Ok((*h, start))
        }
        fn complete(&mut self, (header, start): Self::Pending) -> Result<Vec<u32>> {
            if self.generations > 0 && self.header == Some(header) {
                self.new_completed += 1;
                if self.new_completed == 1 {
                    return Ok(vec![start]);
                }
                self.stop.store(true, Ordering::Relaxed);
            }
            Ok(vec![])
        }
    }
    #[test]
    fn nonce_exhaustion_rolls_extranonce_with_two_batches_in_flight() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let client = TcpStream::connect(listener.local_addr()?)?;
        let (mut server, _) = listener.accept()?;
        server.set_read_timeout(Some(Duration::from_secs(3)))?;
        for message in [
            json!({"id":1,"result":[[],"aa",4],"error":null}),
            json!({"id":2,"result":true,"error":null}),
            json!({"method":"mining.set_difficulty","params":[1e-12]}),
            json!({"method":"mining.notify","params":["wrap","00".repeat(32),"01","02",[],"20000000","1d00ffff","495fab29",true]}),
        ] {
            writeln!(server, "{message}")?;
        }
        let config = Config {
            bitcoin_address: "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
            gpu_load: 100,
            gpu_profile: crate::config::GpuProfile::Throughput,
            pool: crate::pool::Pool::Public,
            ..Config::default()
        };
        let stop = AtomicBool::new(false);
        let probe = WrapProbe {
            stop: &stop,
            header: None,
            next: 0,
            generations: 0,
            new_completed: 0,
        };
        session(client, &config, &stop, "nonce-wrap", &|_| {}, probe)?;
        let mut text = String::new();
        server.read_to_string(&mut text)?;
        let submits: Vec<Value> = text
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .filter(|v: &Value| v["method"] == "mining.submit")
            .collect();
        assert_eq!(submits.len(), 1);
        assert_eq!(submits[0]["params"][1], "wrap");
        assert_eq!(submits[0]["params"][2], "01000000");
        assert_eq!(submits[0]["params"][4], "00000000");
        Ok(())
    }
}
