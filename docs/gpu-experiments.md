# GPU experiments

Measured on October 24, 2026, on an Apple M4 Pro, macOS 27.0.1. Reference: v0.6.0 (`b4bb339`). The initial comparison used an isolated experimental engine and kernels under `examples/`. The winning combination was subsequently integrated in v0.7.0; see [integrated measurements](performance.md). The examples retain a frozen v0.6.0 reference for reproducibility.

## Method

The app's mining process was stopped during measurements. The harness runs at 100% duty without a pool connection. Each measured interval includes command encoding, execution, waiting, result validation and final queue drain. Only completed, assigned nonces count toward hashrate. New headers are used when the nonce space is exhausted.

The main comparison uses five rounds per profile, 0.5-second warmups and 2-second samples. Each round starts and ends with the unmodified production `Gpu::scan`. Experimental variants rotate order, reversed on alternating rounds. `sync` runs the original kernel through the instrumented experimental engine to check harness overhead. Each variant has its own queue and pipeline; only one variant runs at a time.

The analysis interpolates the production rate between each round's two controls and calculates each sample's percentage change against that baseline. Tables show median MH/s and mean paired change. Entire rounds are excluded if production controls drift more than 5% or the OS reports serious/critical thermal pressure. Raw data remain available even for exclusions. Bootstrap intervals use 10,000 resamples with seed 42; five samples on one machine do not establish performance on other Macs.

No power, clock, temperature or register-spill counters were measured. Command timestamps describe command intervals, which may overlap; they do not measure shader occupancy or individual ALU execution time. Thermal state 0 means the OS reports nominal pressure, not that temperature and frequency are constant.

## Variants

| Name | Change |
| --- | --- |
| `sync` | Original shader and threadgroups; one command in flight |
| `double` | Original shader; two complete batches in flight with independent preallocated output buffers |
| `double_split` | Original shader; split each requested batch into two commands, submit both, then retire both before returning |
| `two_sequential` | Two nonces per thread; independent compression calls |
| `two_interleaved` | Two nonces per thread; explicitly interleaved compression rounds |
| `dispatch_threads` | Original shader; exact thread grid via `dispatchThreads` |
| `double_two_sequential` | Two commands in flight with sequential two-nonce kernel |
| `double_two_interleaved` | Two commands in flight with interleaved two-nonce kernel |

The pilot also tests `double_threads`, combining the original shader, two commands in flight and exact dispatch.

## Main results

All five rounds per profile passed the thermal/drift filter. Every recorded sample reported nominal thermal pressure. Approximate production baselines were 261, 402 and 525 MH/s. Results are at 100% duty, not the saved mining duty cycle.

Each cell shows **median MH/s (mean paired change)**:

| Variant | Responsive: 65,536 | Balanced: 262,144 | Throughput: 1,048,576 |
| --- | ---: | ---: | ---: |
| Original through experimental harness | 261.57 (+0.92%) | 401.66 (+0.14%) | 525.27 (+0.17%) |
| Double buffer | 549.33 (+111.05%) | 454.90 (+12.75%) | 335.23 (-36.18%) |
| Split batch, two buffers | 261.79 (+6.03%) | 320.24 (-20.30%) | 343.92 (-34.43%) |
| Two nonces, sequential | 238.14 (-8.20%) | 367.73 (-8.42%) | 492.02 (-6.31%) |
| Two nonces, interleaved | 254.27 (-2.62%) | 387.89 (-3.32%) | 510.75 (-2.80%) |
| Exact thread dispatch | 259.09 (-0.16%) | 401.14 (-0.14%) | 525.04 (-0.05%) |
| Double + sequential | 485.26 (+86.56%) | 504.49 (+25.60%) | 500.79 (-4.66%) |
| Double + interleaved | **564.65 (+117.31%)** | **565.57 (+40.80%)** | **565.34 (+7.60%)** |

The split-batch Responsive samples range from 261 to 295 MH/s; their skew explains why the mean paired gain is larger than the median rate suggests. The other profiles make this variant unattractive regardless.

The separate three-round Throughput pilot agrees: 565.48 MH/s (+7.42%) for double + interleaved, versus 333.81 MH/s (-36.52%) for double alone. Double + exact dispatch reaches 333.07 MH/s (-36.73%), so changing the dispatch API does not rescue that regression.

The ragged-grid check also shows no gain from exact dispatch: mean paired changes are -0.88% at 1,001 nonces, -3.48% at 65,535 and -0.05% at 1,048,575. The 65,535 case is noisy (239–256 MH/s for exact dispatch). Round 0 at 1,001 nonces was excluded because its production controls drifted 38%; two valid rounds remain there. These short, small-batch measurements do not justify claiming a general slowdown from the dispatch API.

The machine-readable [summary](benchmarks/2026-10-24/summary.json) includes sample counts, ranges, paired baselines, bootstrap intervals, timing telemetry and exclusions.

## Recommendation

The experiment selected **double buffering with the interleaved two-nonce kernel** for integration. It is the only tested combination that improves all three profiles and reaches approximately 565 MH/s with even the smallest batch. At that point the larger batch provides no throughput advantage on this Mac; smaller batches are worth considering for responsiveness after measuring job-switch and stop latency.

Do not enable double buffering alone for all profiles: the Throughput regression is repeatable. Do not adopt two nonces per thread alone: both kernel variants lose throughput. Exact dispatch has no demonstrated performance advantage on the standard profiles.

Double buffering substantially reduces measured gaps between command intervals, but that does not guarantee higher hashing throughput. The original shader's two-command case slows down at large batches despite smaller gaps. The measurements do not isolate the underlying hardware cause; register spilling, occupancy or cache behavior cannot be claimed from these timings.

Integration verification covers clean/new Stratum jobs with pending batches, stale-result handling, nonce ownership, overflow retries, stop/restart and duty-cycle pacing. The [performance report](performance.md) records subsequent benchmark results at 100% and 85% duty. The experiment harness does not change app settings.

The cross-batch `double` variants require the mining loop to retain job and nonce-range ownership until each pending result completes. A synchronous `Gpu::scan` cannot pipeline the caller's next batch on its own. `double_split` measures the alternative that can remain behind that API. Integration tests cover job changes, cancellation, duty-cycle pacing and accepted-share attribution.

All three production batch sizes are multiples of the pipeline's SIMD width (32). They have no partial final threadgroup to remove. The separate ragged-grid comparison tests nonmultiples. Exact dispatch still requires support for nonuniform threadgroups; see [Apple's dispatchThreads documentation](https://developer.apple.com/documentation/metal/mtlcomputecommandencoder/dispatchthreads(_:threadsperthreadgroup:)).

## Correctness

Every selected variant must pass CPU/GPU comparisons before timing:

- Full digests for odd counts, SIMD boundaries and the end of the 32-bit nonce range.
- Candidate sets for three arbitrary headers and targets.
- Output overflow, including 2,049 matching nonces ending at `u32::MAX`.
- Two different jobs in flight simultaneously with isolated outputs.
- Refusal to reuse a busy output slot; dropping a pending command drains it before reuse.
- Rejection of wrapping nonce ranges.

Candidates returned during timing are also checked on the CPU. These checks establish hashing correctness for the tested inputs; they do not replace mining-loop integration tests.

## Reproduce

Stop mining and other GPU workloads first. No wallet or pool settings are changed.

```sh
cargo run --locked --release --example gpu_experiments -- \
  --batches 65536,262144,1048576 --rounds 5 --seconds 2 --warmup 0.5 \
  --variants sync,double,double_split,two_sequential,two_interleaved,dispatch_threads,double_two_interleaved,double_two_sequential \
  --out target/comparison.jsonl

cargo run --locked --release --example gpu_experiments -- \
  --batches 1001,65535,1048575 --rounds 3 --seconds 1.5 --warmup 0.5 \
  --variants sync,dispatch_threads --out target/ragged.jsonl

python3 scripts/analyze-gpu-experiments.py \
  target/comparison.jsonl target/ragged.jsonl --json target/summary.json
```

Use `--check-only` for correctness checks without timing. The harness requires a Metal-capable Apple Silicon Mac. Recorded metadata include source hashes, OS version, GPU name and pipeline limits. Files in [benchmarks/2026-10-24](benchmarks/2026-10-24/) preserve the pilot and measured samples.
