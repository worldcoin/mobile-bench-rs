# BrowserStack Metrics Guide

Current release: **0.2.1**.

BrowserStack benchmark runs can provide timing results and resource metrics
when those values are present in mobile runner output or provider logs. Native
stack capture and flamegraph artifacts are local-only in this release.

## Captured Data

The normal benchmark result contains timing samples:

- `duration_ns`
- `mean_ns`
- `median_ns`
- `p95_ns`
- `min_ns`
- `max_ns`
- sample count
- device and platform metadata when available

`ci run` normalizes those results into `summary.json`, `summary.md`, and
`results.csv`.

## Resource Columns

`results.csv` includes resource columns when data is available:

- `cpu_total_ms`: total CPU time across samples.
- `cpu_median_ms`: median per-sample CPU time.
- `effective_cpu_cores_median`: median process CPU time divided by sample wall time.
- `logical_cpu_count`: logical processors installed or reported by the operating system.
- `affinity_cpu_count`: processors available to the benchmark process.
- `rayon_num_threads_env`: explicit `RAYON_NUM_THREADS` setting, when present.
- `peak_memory_kb`: baseline-adjusted peak memory growth.
- `peak_memory_growth_kb`: explicit growth alias used by CI summaries.
- `process_peak_memory_kb`: measured process peak memory.

Blank fields mean the data was not available for that run/device.

`summary.md` includes resource data in the benchmark table when any resource
column is present.

## Metric Sources

Metrics can come from:

- The SDK timing harness.
- Generated Android or iOS runner output.
- BrowserStack session logs parsed by mobench.
- Structured performance snapshots emitted by mobile templates.

Provider-level metrics vary by platform, device, and BrowserStack API
availability. Treat missing resource values as unavailable data, not as zero.

## CI Example

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --plots auto \
  --output-dir target/mobench/ci
```

Inspect outputs:

```bash
cargo mobench report summarize \
  --summary target/mobench/ci/summary.json \
  --plots auto
```

## Profiling Boundary

Use local profiling for native stacks and flamegraphs:

```bash
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --function sample_fns::fibonacci
```

BrowserStack with `android-native`, `ios-instruments`, or `rust-tracing` is
unsupported for native capture in this release.

## Interpreting Results

- Compare like-for-like devices, OS versions, build modes, and iteration counts.
- Prefer `--release` for BrowserStack runs.
- Use medians or percentiles for noisy device data.
- Keep missing resource fields separate from measured zero values.
- Use baselines with `--regression-threshold-pct` for CI regression checks.

An effective-core value near `1.0` means the measured work consumed approximately one
CPU-second per wall-clock second, even when the process can access more processors. That
usually indicates serial application code. Values above `1.0` demonstrate concurrent
CPU execution.

To validate the runner independently, run the synthetic saturation fixture on the same
device:

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::parallel_cpu_saturation \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --output-dir target/mobench/cpu-saturation
```

If saturation reports multiple effective cores but the application fixture remains near
`1.0`, the application path is serial. If both remain near `1.0` despite a larger affinity
count, investigate provider quotas, affinity, or thermal constraints.

## Troubleshooting

- No resource columns: verify the mobile runner emitted resource snapshots or
  BrowserStack exposed the expected logs.
- No plots: install plot dependencies or use `--plots off`.
- Upload timeout: rerun with `--release`.
- Missing artifacts: rerun with `--fetch` or call `cargo mobench fetch`.
