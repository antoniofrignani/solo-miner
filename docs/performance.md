# GPU performance

## v0.7.0 pipeline

Measured October 24, 2026, on Apple M4 Pro, macOS 27.0.1, with mining stopped. The app's three-second benchmark now uses two pending batches and the interleaved two-nonce kernel. It counts completed work and includes queue drain and duty-cycle pauses. No pool connections were used for these measurements.

| Workload | v0.6.0 reference, 100% duty | Pipeline, 100% duty | Pipeline, 85% duty |
| --- | ---: | ---: | ---: |
| Responsive | 261 MH/s | 564 MH/s | 441 MH/s |
| Balanced | 395 MH/s | 566 MH/s | 440 MH/s |
| Throughput | 525 MH/s | 565 MH/s | 438 MH/s |

Rates are rounded medians. Full-duty comparisons use three paired rounds, each with the original engine before and after the pipeline. The first Responsive round was excluded because its controls drifted more than 5%; two valid rounds remain. Mean paired improvements were +116%, +43% and +7.8%. Changing the burst duration only affects duty cycles below 100%.

At 85%, the v0.6.0 reference varied too much for a reliable percentage comparison, especially on Balanced and Throughput. The final pipeline's three absolute samples per profile were stable at approximately 437–443 MH/s. OS thermal state was nominal throughout. No power, temperature or clock counters were measured.

Below full duty, work runs in approximately 50 ms bursts before the queue drains and the worker pauses. A 10 ms burst trial measured roughly 413–420 MH/s at 85%; 50 ms bursts measured 437–443 MH/s. Stop and network processing remain between batches, and pauses check cancellation every 20 ms. A duty-cycle percentage does not guarantee the same fraction of full-duty hashrate.

Each pending batch owns its header, target, nonce range and job metadata. Clean jobs discard old pending shares; non-clean jobs preserve their original submission fields. Nonces are reserved on submission, and completed work is counted once. Errors and stop drain commands before output storage can be reused. Overflow recovery can use one slot while another job remains pending.

Ordinary tests cover job transitions, immediate SoloPool difficulty changes, failure/stop draining and complete nonce-space rollover with queued work. Hardware tests cover CPU/GPU digest equality, odd ranges, concurrent output and overflow, an accepted share through a local Stratum server, and cancellation at 5%, 85% and 100% duty for every workload profile. Existing lifecycle tests cover settings restart and stop ordering.

The [initial experiments](gpu-experiments.md) and [raw integration data](benchmarks/2026-10-24/) are retained. `integrated-burst10.jsonl` contains the full-duty comparison and first pacing trial; `integrated-burst50.jsonl` contains the final 85% pacing trial. The [integration summary](benchmarks/2026-10-24/integrated-summary.json) records excluded paired rounds. Source hashes identify the compiled revisions at measurement time, before later test-only additions.

```sh
cargo run --locked --release --example pipeline_benchmark -- \
  --rounds 3 --out target/integrated.jsonl
python3 scripts/analyze-gpu-experiments.py target/integrated.jsonl
```

## v0.6.0 midstate and compact output

Measured on October 23, 2026, using the system's default Metal device, reported as **Apple M4 Pro**. The app's mining worker was stopped during the comparison. No pool connections or share submissions were used.

The reference is the v0.5.3 kernel with 65,536 nonces per batch. A local harness repeatedly called `Gpu::scan` at 100% duty for five seconds per sample. Measurements include command submission, GPU execution, synchronization and CPU result processing. Warmed samples:

| Engine / workload | Hashrate |
| --- | ---: |
| v0.5.3 | 188.4–191.3 MH/s |
| v0.6.0 Responsive | 256.4–257.3 MH/s |
| v0.6.0 Balanced | 394.4–394.5 MH/s |
| v0.6.0 Throughput | 512.1–515.2 MH/s |

The three-second `--gpu-profile` command subsequently measured 270.5, 399.7 and 514.8 MH/s respectively. Short runs vary with GPU frequency and scheduling. These are measurements from one Mac, not guaranteed rates for other devices. Power consumption was not measured.

## Changes

- Cache the SHA-256 state after the first 64 bytes of the header. The GPU performs the two remaining compression blocks for each nonce instead of repeating all three.
- Return a counter and matching nonces instead of writing, copying and scanning one flag for every nonce. The result buffer is about 4 KiB.
- Retry overflowing candidate ranges in smaller batches. An easy target cannot silently discard valid nonces.
- Increase the batch size for Balanced and Throughput to reduce CPU/GPU synchronization overhead. Threadgroups stay aligned to the pipeline's SIMD execution width.

A fully unrolled 16-word schedule and a looped 16-word schedule were also measured. Both were slower on this device and were discarded. The retained kernel uses the original 64-word schedule.

## Verification

Each GPU initialization checks full GPU digests against CPU SHA-256d, the genesis nonce and arbitrary targets. The explicit `metal_profiles_match_cpu_and_preserve_overflow` test also checks all workload profiles, header-cache changes, compact result overflow and the end of the 32-bit nonce range. `gpu_to_stratum_share` checks the real GPU through a local Stratum server.

Run comparisons with mining stopped and without other GPU-heavy applications:

```sh
cargo run --locked --release -- --gpu-profile
cargo test --locked metal_profiles_match_cpu_and_preserve_overflow -- --ignored --nocapture
cargo test --locked gpu_to_stratum_share -- --ignored --nocapture
```

GPU utilization alone does not establish which execution units are limiting the kernel. Apple describes how occupancy, ALU, memory and other counters need to be considered together in [Optimize Metal apps and games with GPU counters](https://developer.apple.com/videos/play/wwdc2020/10603/). Threadgroup sizing follows [Apple's Metal guidance](https://developer.apple.com/documentation/metal/calculating-threadgroup-and-grid-sizes).
