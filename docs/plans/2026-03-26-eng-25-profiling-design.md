# Mobench Profiling Feature Set Design

## Summary

Add a new profiling subsystem to Mobench that complements timing benchmarks
with native profile artifacts and flamegraph-capable visualizations. The CLI
should remain the user-facing orchestrator and should reuse the same benchmark
selection, build, packaging, device resolution, and output conventions that
existing `run` and `ci run` commands already use.

The design standardizes orchestration and artifact layout, not a fake common
raw profile format. Android and iOS should keep their native capture formats,
while Mobench provides a shared command surface, normalized metadata, and
reporting helpers. BrowserStack stays in scope as an execution environment, but
remote native sampling capture is not an MVP dependency.

## Goals

- Keep the primary workflow inside the existing `mobench` CLI.
- Add a dedicated profiling command family that feels parallel to `run` and
  `report summarize`.
- Produce native raw profile artifacts plus normalized metadata for downstream
  viewing and automation.
- Support local-first native profiling on Android and iOS with release-like
  builds and explicit symbol handling.
- Make flamegraph-capable output first-class on at least the Android path.
- Preserve compatibility with the current benchmark/report contract.

## Non-Goals

- Replacing the existing timing benchmark flow.
- Inventing a single cross-platform raw profile format.
- Depending on undocumented BrowserStack native-profiler APIs for MVP success.
- Treating app-level instrumentation as a substitute for native stack sampling.
- Changing the required v1 CI outputs in `summary.json`, `summary.md`, or
  `results.csv`.

## User Experience

The new workflow should add a dedicated profile command family:

```bash
cargo mobench profile run --target android --function sample_fns::fibonacci
cargo mobench profile summarize --profile target/mobench/profile/profile.json
cargo mobench profile summarize --profile target/mobench/profile/<run-id>/profile.json
```

`profile run` should mirror existing benchmark selectors and project-resolution
flags:

- `--target`
- `--function`
- `--iterations`
- `--warmup`
- `--config`
- `--crate-path`
- `--project-root`
- `--output-dir`
- device/provider flags already used by benchmark runs

Profiling-specific controls should be additive:

- `--backend auto|android-native|ios-instruments|rust-tracing`
- `--format native|processed|both`
- `--time-limit`
- `--symbolize auto|off|require`

The command must be local-first. Developers should be able to run local Android
and local iOS profiling through Mobench, then continue in Android Studio,
Firefox Profiler, Xcode, or Instruments using artifacts that Mobench wrote.

BrowserStack should remain a supported execution target conceptually, but the
MVP must not assume that remote native sampling capture is available. If that
path is unsupported, Mobench should fail explicitly and explain why.

## Architecture

### Orchestration Layer

Mobench should own:

- target and benchmark resolution
- build/profile mode selection
- device/provider selection
- output directory and artifact naming
- symbol discovery and policy handling
- backend selection and subprocess invocation

This keeps profiling aligned with the existing CLI instead of becoming a set of
ad hoc shell recipes.

### Platform Backend Layer

Each platform backend should expose a shared interface to the CLI:

- `prepare()`
- `capture()`
- `process()`
- `summarize()`

Android and iOS should share orchestration semantics while keeping native tool
choices:

- Android: native sampling capture, then optional import/conversion into a
  flamegraph-capable viewer format.
- iOS: `xctrace` / Instruments capture with `.trace` bundles and exported
  summaries.

### Artifact And Report Layer

Raw artifacts should stay native. Mobench should normalize only the metadata
around them:

- what benchmark ran
- on which target/device/provider
- which backend executed
- where raw and processed artifacts live
- symbol status
- viewer hints
- partial-failure state

This lets Mobench summarize captures without pretending `.trace` bundles and
Android sample captures are interchangeable.

## Command Surface

The MVP command family should be:

- `cargo mobench profile run`
- `cargo mobench profile summarize`

`profile run` creates a profile session and writes artifacts. `profile summarize`
reads the normalized metadata file and renders terminal or markdown output
without re-running the benchmark.

Future commands can remain additive:

- `cargo mobench profile open`
- `cargo mobench profile export`
- `cargo mobench profile import`

## Artifacts

Profiling should not modify the existing CI v1 contract. Instead it should add a
parallel additive artifact tree:

```text
target/mobench/profile/
  profile.json          # latest session convenience copy
  summary.md            # latest session convenience copy
  <run-id>/
    profile.json
    summary.md
    artifacts/
      raw/
      processed/
```

`profile.json` is the normalized metadata file. It should record:

- benchmark identity and run parameters
- target/platform/device/provider
- backend, capture mode, and requested output format
- raw artifact paths
- processed artifact paths
- symbolization state
- capture timestamps and durations
- warnings and partial failures
- viewer hints and recommended next actions

The raw artifacts remain platform-specific:

- Android: native sampling capture plus processed flamegraph-capable output when
  available
- iOS: `.trace` bundles and optional exported XML summaries
- optional Rust tracing output if a future fallback backend is added

## Backend Behavior

### Android

Android should be the first flamegraph-capable path in the MVP. Mobench should
handle:

- building release-like artifacts with symbol retention
- launching the benchmarked app on a local Android target
- coordinating native sampling capture
- emitting raw and processed artifacts

The preferred viewer can differ from iOS as long as the processed output is
clearly documented and actionable.

### iOS

iOS should use Instruments via `xctrace`. Mobench should:

- build release-like artifacts with dSYMs
- launch or attach to the benchmarked app on a local iOS target
- record a `Time Profiler` trace
- emit `.trace` artifacts plus exported metadata

The first-class viewer is Instruments/Xcode, not Firefox Profiler.

### BrowserStack

BrowserStack remains in scope as a run environment, but the MVP should treat
native remote profile capture as optional. The benchmark may run on a real phone
there, but the profiling backend must fail explicitly if the provider cannot
support native capture. That keeps BrowserStack integration honest instead of
relying on undocumented capabilities.

## Symbolization

Profile runs are only useful if Mobench can report symbol quality clearly.

Rules:

- Android must preserve or locate unstripped native symbols.
- iOS must preserve or locate dSYMs.
- `--symbolize require` should fail the command when symbolization prerequisites
  are missing.
- `--symbolize auto` may continue but must record warnings and degraded status
  in `profile.json`.

Unsymbolized captures are operationally failures even when raw files exist.

## Error Handling

The profiling subsystem should distinguish:

- benchmark execution failure
- capture startup failure
- symbol discovery or symbolization failure
- artifact conversion/export failure
- unsupported backend or provider capability
- viewer/tool availability failure

Partial success must be expressible in `profile.json`. For example, a benchmark
may run and produce a raw trace while processed exports fail.

## Testing Strategy

### Unit Tests

- CLI parser coverage for `profile` commands and options
- artifact-path and manifest-serialization tests
- backend selection and capability gating tests
- command-construction tests for external tools

### Fixture Tests

- `profile.json` rendering and summary formatting
- partial-failure serialization
- symbol-warning propagation

### Smoke Tests

- opt-in local Android smoke tests
- opt-in local iOS smoke tests
- no assumption that CI can run full native profilers on both platforms

## References

- [docs/CONTRACT_CI_V1.md](/Users/dcbuilder/.config/superpowers/worktrees/mobile-bench-rs/codex-eng-25-profiling/docs/CONTRACT_CI_V1.md)
- [samply README](https://github.com/mstange/samply)
- [Firefox profiling with simpleperf](https://firefox-source-docs.mozilla.org/performance/profiling_with_simpleperf.html)
- [Android Studio profiling docs](https://developer.android.com/studio/profile)
- [Xcode performance and metrics](https://developer.apple.com/documentation/xcode/performance-and-metrics)

Local verification on this machine also confirmed that `xcrun xctrace` exposes
`record`, `export`, and `import` commands, including the `Time Profiler`
template, which makes a CLI-orchestrated iOS path viable on Xcode hosts.
