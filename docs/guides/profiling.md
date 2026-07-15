# Profiling Guide

Current release: **0.1.43**.

`cargo mobench profile ...` is separate from normal benchmark execution. Use
`cargo mobench run` or `cargo mobench ci run` for timing-focused benchmark
results. Use `profile run` for local native stack capture and flamegraph
artifacts.

## Capability Matrix

| Provider | Backend | Behavior |
| --- | --- | --- |
| `local` | `android-native` | Attempts real `simpleperf` capture, symbolization, folded stacks, native report, SVGs, and `flamegraph.html`. |
| `local` | `ios-instruments` | Attempts simulator-host `sample` capture, folded stacks, native report, SVGs, and `flamegraph.html`. |
| `local` | `rust-tracing` | Planned manifest/trace contract only. |
| `browserstack` | `android-native` | Unsupported native capture. |
| `browserstack` | `ios-instruments` | Unsupported native capture. |
| `browserstack` | `rust-tracing` | Unsupported. |

BrowserStack benchmark runs can still provide timing and resource metrics. They
do not provide retrievable native stack artifacts in this release.

## Quick Start

Android:

```bash
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --function sample_fns::fibonacci \
  --trace-events-output target/mobench/profile/trace-events.json
```

iOS simulator-host sampling:

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

Generate a diff:

```bash
cargo mobench profile diff \
  --baseline target/mobench/profile/baseline/profile.json \
  --candidate target/mobench/profile/candidate/profile.json \
  --normalize
```

## Profile Run Options

Common options:

- `--target <android|ios>`
- `--function <FUNCTION>`
- `--crate-path <PATH>`
- `--config <FILE>`
- `--output-dir <DIR>` defaulting to `target/mobench/profile`
- `--trace-events-output <FILE>`
- `--device <DEVICE>`
- `--os-version <VERSION>`
- `--profile <PROFILE>`
- `--device-matrix <FILE>`
- `--provider <local|browserstack>`
- `--backend <auto|android-native|ios-instruments|rust-tracing>`
- `--format <native|processed|both>`
- `--warmup-mode <cold|warm>`

`--device`, `--os-version`, `--profile`, and `--device-matrix` reuse the same
deterministic device resolution model as `mobench devices resolve`.

## Artifact Layout

Each session writes a run directory below `target/mobench/profile/<run-id>/` and
refreshes latest copies below `target/mobench/profile/`.

Common outputs:

- `profile.json`
- `summary.md`
- `artifacts/raw/...`
- `artifacts/processed/stacks.folded`
- `artifacts/processed/native-report.txt`
- `artifacts/processed/frame-locations.json` on Android when file/line metadata
  is available
- `artifacts/processed/flamegraph.full.svg`
- `artifacts/processed/flamegraph.focused.svg`
- `artifacts/processed/flamegraph.html`
- `artifacts/semantic/phases.json` when semantic phase data exists

Manifest sections:

- `native_capture`: native stack artifacts, symbolization state, and viewer
  hints.
- `semantic_profile`: optional benchmark phase data.
- `capture_metadata`: device resolution, capture settings, and warnings.

## Semantic Phases

Benchmarks can emit named semantic phases:

```rust
use mobench_sdk::{benchmark, profile_phase};

#[benchmark]
pub fn prove_and_verify() {
    let proof = profile_phase("prove", || prove());
    profile_phase("verify", || verify(&proof));
}
```

mobench stores phases in `artifacts/semantic/phases.json` and renders them
separately from native flamegraphs. Phase timing is benchmark metadata, not
sampled native stack data.

## Flamegraph Viewer

`flamegraph.html` includes:

- Full process view.
- Benchmark-only focused view when available.
- Timeline view when chronological harness spans are available.
- Source links for Android frames when `llvm-addr2line` recovers file/line data.

Aggregate flamegraphs are not relabeled as wall-clock timelines. When only
aggregate folded stacks are available, the viewer keeps aggregate stack views.

## Android Requirements

- `adb`
- Android NDK
- `simpleperf`
- NDK `llvm-addr2line`
- Local Android device or emulator

mobench discovers `llvm-addr2line` from the NDK when possible. Override with
`MOBENCH_ANDROID_LLVM_ADDR2LINE` or `LLVM_ADDR2LINE` when needed.

## iOS Requirements

- macOS
- Xcode command-line tools
- `xcrun`
- `simctl`
- Available iOS simulator runtime
- macOS `sample`

The current iOS backend is simulator-host oriented. Use Xcode Instruments
directly for deeper real-device investigations.

## Diff Outputs

`profile diff` writes a diff bundle containing:

- `profile-diff.json`
- `summary.md`
- `artifacts/processed/diff.full.folded`
- `artifacts/processed/diff.focused.folded`
- `artifacts/processed/flamegraph.full.svg`
- `artifacts/processed/flamegraph.focused.svg`
- `artifacts/processed/flamegraph.html`

Inferno differential color semantics apply: red is hotter in the candidate,
blue is hotter in the baseline.
