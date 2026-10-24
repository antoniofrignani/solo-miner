# GPU performance

For the subsequent double-buffering, two-nonce and exact-dispatch comparison, see [GPU experiments](gpu-experiments.md). Those variants are experimental and are not enabled in the app.

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
