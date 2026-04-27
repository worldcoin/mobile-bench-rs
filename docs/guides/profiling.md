# Profiling Guide

This guide covers `cargo mobench profile ...`, the local native profiling
workflow, symbol requirements, and how profiling fits alongside normal mobench
benchmark runs.

## Scope

Profiling is local-first in the current release.

Supported today:

| Provider | Backend | What you get |
|----------|---------|--------------|
| `local` | `android-native` | `simpleperf` capture, symbolized folded stacks, `native-report.txt`, optional `frame-locations.json`, full/focused flamegraph SVGs, `flamegraph.html`, optional semantic phase summaries, and optional machine-readable trace/event JSON |
| `local` | `ios-instruments` | simulator-host `sample` capture, collapsed folded stacks, `native-report.txt`, full/focused flamegraph SVGs, `flamegraph.html`, optional semantic phase summaries, and optional machine-readable trace/event JSON |

Not supported today:

| Provider | Backend | Why |
|----------|---------|-----|
| `browserstack` | `android-native` / `ios-instruments` | BrowserStack benchmark runs can return timing and resource data, but not retrievable native-stack artifacts in the mobench session contract |
| `local` | `rust-tracing` | native rust-tracing capture is planned; use `--trace-events-output` on supported profile runs when a downstream consumer needs mobench's harness event contract |

## Quick start

### Android

```bash
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --function sample_fns::fibonacci \
  --trace-events-output target/mobench/profile/trace-events.json
```

### iOS

```bash
cargo mobench profile run \
  --target ios \
  --provider local \
  --backend ios-instruments \
  --function sample_fns::fibonacci
```

Render the latest summary:

```bash
cargo mobench profile summarize \
  --profile target/mobench/profile/profile.json
```

Generate a differential flamegraph from two mobench sessions:

```bash
cargo mobench profile diff \
  --baseline target/mobench/profile/android-sample_fns--fibonacci/profile.json \
  --candidate target/mobench/profile/profile.json \
  --normalize
```

Current flamegraph viewer:

![Mobench flamegraph viewer](../../assets/flamegraph-viewer.png)

## How profiling integrates with normal mobench runs

Profiling is a separate command surface from `cargo mobench run`.

- `cargo mobench run` is still the right tool for timing-focused benchmark runs
  locally or on BrowserStack
- `cargo mobench profile run` is for local native capture and flamegraph output
- a benchmark can optionally emit semantic phase data with
  `mobench_sdk::timing::profile_phase(...)`

When `profile_phase(...)` is present, mobench stores benchmark phases in
`artifacts/semantic/phases.json` and renders them separately from the native
flamegraph. That separation matters: phase timing is benchmark metadata, not a
sampled native stack.

## Prerequisites

### Android

Required:

- `adb`
- Android NDK
- `simpleperf` from the NDK toolchain
- `llvm-addr2line` from the NDK toolchain
- a locally reachable Android device or emulator

Notes:

- the generated Android benchmark app is marked `profileable`, which allows
  local native profilers to attach
- mobench can discover `llvm-addr2line` in the NDK automatically; if needed,
  override it with `MOBENCH_ANDROID_LLVM_ADDR2LINE` or `LLVM_ADDR2LINE`

### iOS

Required:

- macOS
- Xcode command-line tools
- `xcrun`
- `simctl`
- an available iOS simulator runtime
- the macOS `sample` command

Notes:

- the current iOS profiling backend is simulator-host only
- for real-device deep analysis, continue to use Instruments/Xcode directly

## Symbol requirements

### Android

Android flamegraphs are only as good as the symbols available to the
post-processor.

mobench expects:

- the raw `sample.perf`
- the matching unstripped Cargo-produced `.so` files
- NDK `llvm-addr2line`

The important detail is that symbolization is done against the unstripped native
libraries produced during the build, not only the packaged copies under
`jniLibs/`. That means release-like app packaging is still compatible with
profiling as long as the unstripped build outputs remain available for the same
build.

If symbols are missing, mobench still writes the session and marks
symbolization as partial or failed in `profile.json`.

When `llvm-addr2line` returns file/line metadata, mobench also writes
`artifacts/processed/frame-locations.json` and the viewer exposes source links
for selected frames and hot-path entries.

### iOS

The current backend works from the textual call graph emitted by `sample`
against a simulator-host process. That is sufficient for the local flamegraph
path, but it is not a replacement for Instruments-based symbol workflows on
real devices.

## Differential flamegraphs

`cargo mobench profile diff` compares two normalized mobench profile sessions.

Input contract:

- baseline session: `profile.json` plus its processed folded stacks
- candidate session: `profile.json` plus its processed folded stacks
- optional source locations from `frame-locations.json`

Output contract:

- `profile-diff.json`
- `summary.md`
- `artifacts/processed/diff.full.folded`
- `artifacts/processed/diff.focused.folded`
- `artifacts/processed/flamegraph.full.svg`
- `artifacts/processed/flamegraph.focused.svg`
- `artifacts/processed/flamegraph.html`

Color semantics follow the standard inferno differential model:

- red = hotter in the candidate session
- blue = hotter in the baseline session
- frame widths follow candidate sample counts

If you need the reverse width perspective for disappearing stacks, swap the
baseline and candidate inputs.

The viewer intentionally keeps aggregate hotspot analysis and exact harness
timing separate:

- `Benchmark Only` and `Full Process` remain aggregate flamegraphs over the
  whole capture
- `Timeline` shows exact harness intervals and recorded chronological samples
  when they exist
- if the capture only has harness timing, Timeline says so explicitly instead of
  pretending to crop the aggregate flamegraph by wall-clock ratio

## Artifact layout

Each session writes to `target/mobench/profile/<run-id>/` and refreshes the
top-level latest copies under `target/mobench/profile/`.

Common outputs:

- `profile.json`
- `summary.md`
- `artifacts/raw/...`
- `artifacts/processed/stacks.folded`
- `artifacts/processed/native-report.txt`
- `artifacts/processed/frame-locations.json` on Android when file/line metadata is available
- `artifacts/processed/flamegraph.full.svg`
- `artifacts/processed/flamegraph.focused.svg`
- `artifacts/processed/flamegraph.html`

Optional semantic output:

- `artifacts/semantic/phases.json`

Platform-specific raw artifacts:

- Android: `artifacts/raw/sample.perf`
- iOS: `artifacts/raw/sample.txt`

Differential outputs live under `target/mobench/profile/diff/<run-id>/` and
refresh top-level `target/mobench/profile/diff/profile-diff.json` /
`summary.md`.

## Warmup and capture behavior

Local Android and iOS native backends default to `--warmup-mode warm`.

Why warm mode exists:

- very short mobile benchmarks can disappear into profiler sampling noise
- first-run bridge and startup work can dominate otherwise useful captures

Warm mode improves the signal, but it does not remove all initialization costs.
If you need a true first-run profile, use:

```bash
cargo mobench profile run ... --warmup-mode cold
```

## Overhead and tradeoffs

Choose the profiling surface based on the question you are asking.

### Native profiling

Best for:

- identifying hot functions in Rust, FFI, bridge, allocator, or platform code
- inspecting where sampled wall-clock time actually accumulates
- comparing full-process and benchmark-focused stack shapes

Tradeoffs:

- sampling adds overhead
- very short functions may need repeated execution to appear in stacks
- Android and iOS require different host tooling even though the processed
  output is normalized

### Semantic phase timing

Best for:

- seeing benchmark-domain phases such as `prove`, `serialize`, or `verify`
- understanding logical work boundaries with low instrumentation cost

Tradeoffs:

- phase timing does not include arbitrary native stack context
- it complements flamegraphs; it does not replace them

## Local vs. CI

Use local profiling for native-stack work.

- developer machines are the primary target
- controlled self-hosted or specialized CI runners can validate the local tool
  path
- BrowserStack remains a benchmark execution surface, not a native-profile
  artifact source

If you need CI automation today:

- use BrowserStack for timing and resource metrics
- archive local profile sessions as CI artifacts when you need flamegraph
  regression triage
- compare archived sessions with `cargo mobench profile diff`

Recommended CI artifact set for profile-aware regression work:

- `profile.json`
- `artifacts/processed/stacks.folded`
- `artifacts/processed/benchmark.focused.folded`
- `artifacts/processed/frame-locations.json` when present

That deliberately defines a separate baseline model from `ci run` summary
comparison: benchmark timing regressions continue to use summary baselines,
while flamegraph regressions compare profile sessions directly.

## iOS boundary and recommendation

The current iOS backend stays on simulator-host `sample` because it is the
lowest-friction local capture path and produces folded stacks directly from the
launched benchmark process.

There is a viable higher-fidelity future path:

- Apple’s `xctrace export` can export `.trace` data to XML for post-processing
- inferno already documents an `inferno-collapse-xctrace` path from exported
  Time Profiler data into folded stacks

Recommendation:

- keep `sample` as the default mobench iOS profiling backend for local smoke and
  regression workflows
- treat Instruments import/export as a future opt-in path, not a replacement
  for the current default
- do not promise uniform simulator/device source-link fidelity until an
  explicit xctrace import surface exists in mobench

## Recommended workflow

1. Run a normal mobench benchmark to confirm the regression or hotspot.
2. Re-run the same benchmark with `cargo mobench profile run`.
3. Open `artifacts/processed/flamegraph.html`.
4. Compare the full-process and benchmark-focused views.
5. If Android source links are present, drill into selected frames from the
   viewer sidebar.
6. If phase data exists, correlate the flamegraph with
   `artifacts/semantic/phases.json`.

## Related docs

- [build.md](build.md)
- [testing.md](testing.md)
- [browserstack-metrics.md](browserstack-metrics.md)
