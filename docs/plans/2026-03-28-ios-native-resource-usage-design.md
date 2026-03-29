# Native iOS Resource Usage Design

## Goal

Make iOS benchmark runs emit native `elapsed_cpu_ms` and `peak_memory_kb` so the existing `mobench` summary pipeline populates `CPU total (ms)` and `Peak memory` in CI tables without a separate profiler merge path.

## Current State

- Android already writes resource data directly from the app into `resources`, including `elapsed_cpu_ms`.
- iOS benchmark JSON currently writes only `platform` and `timestamp_ms` in [ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift](/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/codex-ios-native-resource-usage-ci/ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift).
- The CI workflow in [reusable-bench.yml](/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/codex-ios-native-resource-usage-ci/.github/workflows/reusable-bench.yml) already renders summaries from the benchmark JSON path; it does not need a second profiling job.
- Rust summary parsing in [crates/mobench/src/lib.rs](/Users/dcbuilder/Code/world/mobile-bench-rs/.worktrees/codex-ios-native-resource-usage-ci/crates/mobench/src/lib.rs) can already read `elapsed_cpu_ms`, but it does not currently read a direct `peak_memory_kb` field from `resources`.

## Chosen Approach

Use native in-app process accounting in the iOS BenchRunner app.

- Capture a resource snapshot immediately before `runBenchmark(spec:)`.
- Capture another snapshot immediately after the benchmark returns.
- Compute `elapsed_cpu_ms` as the delta between the cumulative CPU totals.
- Report `peak_memory_kb` from the end snapshot's high-water resident memory reading.

This keeps the existing CI execution model intact:

- BrowserStack still runs the iOS app and UI test.
- The UI test still extracts one benchmark JSON payload.
- `mobench` still builds `summary.json` from that payload.
- `ci summarize` and `report summarize` keep consuming the same summary schema.

## Data Contract

The iOS benchmark JSON `resources` object should become:

```json
{
  "platform": "ios",
  "timestamp_ms": 1710000000000,
  "elapsed_cpu_ms": 482,
  "peak_memory_kb": 249416
}
```

Parsing changes:

- `extract_benchmark_resource_usage` should keep reading `elapsed_cpu_ms`.
- It should also read a direct `peak_memory_kb` field before falling back to BrowserStack memory metrics or Android heap fields.

## Native Metric Source

Use native Darwin/Mach task APIs in the iOS app:

- CPU total: cumulative task user + system CPU time.
- Peak memory: task high-water resident memory.

The process is launched fresh for each benchmark execution in the current CI flow, so a process-level high-water mark is a reasonable approximation for benchmark peak memory and is materially better than the current "missing value" state.

## Files Expected To Change

- `ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift`
- `crates/mobench-sdk/templates/ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift.template`
- `crates/mobench/src/lib.rs`
- `docs/plans/2026-03-28-ios-native-resource-usage-design.md`
- `docs/plans/2026-03-28-ios-native-resource-usage.md`

## Testing Strategy

- Add Rust unit tests that cover iOS-style resource payloads containing `elapsed_cpu_ms` plus direct `peak_memory_kb`.
- Extend summary/render tests so iOS resource usage survives summary generation.
- Run targeted `cargo test -p mobench ...` coverage locally.
- Validate the end-to-end CI path with a workflow dispatch after pushing the branch.

## Non-Goals

- No new BrowserStack CPU percentage column.
- No separate Instruments-only summary merge pipeline.
- No change to Android resource collection semantics.
