# Mobench Profiling Upgrades Design

## Summary

Upgrade Mobench profiling from "real local capture with basic artifacts" to a
two-layer profiling system that answers both of the questions users actually
have when investigating mobile benchmark performance:

- Which native and Rust functions were hot?
- Which benchmark phase was expensive?

The design keeps native profilers as the source of truth for stack-level CPU
behavior while adding an explicit semantic profiling layer for benchmark phases
such as `load`, `prove`, `serialize`, and `verify`.

## Goals

- Improve the quality and interpretability of native profiling artifacts on
  Android and iOS.
- Make Android flamegraphs show demangled Rust frames below the UniFFI/JNA
  bridge.
- Preserve the current iOS ability to show Rust frames below the FFI boundary.
- Add an opt-in semantic profiling layer in `mobench-sdk` for benchmark phases.
- Keep the normalized `profile.json` contract, but separate native capture from
  semantic instrumentation clearly.
- Keep BrowserStack native profiling explicitly unsupported unless real
  artifact retrieval exists.

## Non-Goals

- Replacing native profilers with benchmark instrumentation.
- Promising exact per-iteration call traces from sampled profiling.
- Migrating the Android bridge from JNA/UniFFI to Java FFM in this upgrade.
- Treating BrowserStack timing or memory metrics as native stack profiling.

## Current State

The branch already supports real local profiling:

- Android local profiling uses `simpleperf`, writes `sample.perf`,
  `stacks.folded`, and `flamegraph.html`.
- iOS local profiling samples the simulator-host process and writes
  `sample.txt`, `stacks.folded`, and `flamegraph.html`.
- BrowserStack native profiling is explicitly unsupported.

Observed limitations:

- iOS preserves the FFI and Rust call chain well in current outputs.
- Android captures native samples successfully, but the current folded/flamegraph
  outputs do not fully symbolize internal Rust frames and still show unresolved
  native offsets in practice.
- Current native profiling is good at identifying hot call stacks, but it does
  not answer semantic questions such as "how much time was spent proving vs
  serializing?"

## Recommended Approach

Use a two-layer profiling model:

1. Native capture layer
   - Android: improve `simpleperf` symbol fidelity and capture hygiene.
   - iOS: keep the current working local flow, but hide it behind a capture
     interface that can later support richer native tools such as `xctrace`.
2. Semantic profiling layer
   - Add an opt-in phase API in `mobench-sdk`.
   - Record flat benchmark phases such as `prove`, `serialize`, and `verify`.
   - Merge these phase timings into `profile.json` and `summary.md` without
     pretending they came from native profilers.

This approach preserves native profiling accuracy while making benchmark-level
performance reports actionable.

## Product Shape

The upgraded profiling experience should answer two questions explicitly:

### Stack-level

Which Rust or native functions were hot during the benchmark?

This is answered by:

- raw native capture artifacts
- symbolized folded stacks
- rendered flamegraphs
- optional native plain-text reports

### Phase-level

How much time was spent in major benchmark stages such as proving,
serialization, or verification?

This is answered by:

- benchmark-side semantic phase instrumentation
- merged phase timing output in `profile.json`
- rendered phase summaries in `summary.md`

## Architecture

### 1. Capture Executor

Platform-specific native profilers should remain responsible for producing raw
capture data.

- Android executor: run `simpleperf` and persist all intermediate artifacts
  needed for later symbolization.
- iOS executor: keep the current working local capture path, but return a raw
  capture bundle through a stable interface instead of assuming the raw text
  file is the final product.

### 2. Symbolizer

A platform-specific post-processing layer should resolve native frames into
stable, readable function names.

- Android:
  - consume unresolved native offsets from `simpleperf` output
  - resolve them with `llvm-addr2line -Cfpe` against unstripped Rust shared
    libraries
  - rewrite folded stacks before flamegraph generation
- iOS:
  - continue using already-symbolized frames from the working local path
  - keep the symbolizer interface generic enough to support `xctrace` later

### 3. Semantic Profiler

An opt-in semantic profiling API should be added to `mobench-sdk`.

Start with flat phases only:

- `profile_phase("load", || ...)`
- `profile_phase("prove", || ...)`
- `profile_phase("serialize", || ...)`
- `profile_phase("verify", || ...)`

This layer records benchmark meaning, not stack structure.

### 4. Renderers

Mobench should render:

- native folded stacks into `flamegraph.html`
- a native plain-text call tree report for inspection
- semantic phase summaries into Markdown and JSON outputs

## Output Contract

The manifest should distinguish native capture from semantic instrumentation.

Recommended top-level sections:

- `native_capture`
  - `status`
  - `raw_artifacts`
  - `processed_artifacts`
  - `symbolization`
- `semantic_profile`
  - `status`
  - `phases`
  - optional `spans_path`
- `capture_metadata`
  - target device/runtime
  - sample duration / sample frequency
  - warmup mode
  - capture method details

The on-disk run directory can remain compatible with the current layout:

```text
target/mobench/profile/<run-id>/
  profile.json
  summary.md
  artifacts/
    raw/
    processed/
    semantic/
```

Suggested additions:

- `artifacts/processed/native-report.txt`
- `artifacts/semantic/phases.json`

## Native Profiling Improvements

### Android

Android needs better symbol fidelity and less setup noise.

Required improvements:

- warm the app and bridge before recording so JNA/UniFFI startup does not
  dominate the flamegraph
- preserve and use unstripped Rust `.so` files for symbolization
- symbolize `lib*.so[+offset]` frames with `llvm-addr2line -Cfpe`
- record unresolved-frame counts and expose them in the manifest and summary
- prefer release builds with debuginfo for profiling runs

Desired outputs:

- symbolized `stacks.folded`
- symbolized `flamegraph.html`
- `native-report.txt` for text inspection

### iOS

iOS should keep the current working capture path while improving capture
quality and metadata.

Required improvements:

- warm the benchmark before recording to reduce launch/setup noise
- record the exact capture method in metadata
- preserve current symbol visibility in folded stacks and flamegraphs

Future-compatible improvements:

- abstract the working capture path behind an executor interface
- allow a richer `xctrace` backend later without changing the manifest model

## Semantic Profiling Layer

The semantic profiling layer exists to answer benchmark-specific questions that
sampled native stacks cannot answer reliably.

The initial semantic profiling API should be:

- opt-in
- flat rather than nested
- cheap enough to use in hot benchmarks without distorting results

Output example:

- `prove = 92%`
- `serialize = 5%`
- `verify = 3%`

The summary must label this clearly as benchmark instrumentation rather than
native profiling.

## Android Interop Direction

Do not include an Android FFM migration in this upgrade.

Rationale:

- JNA still relies on JNI internally, but avoids handwritten JNI glue.
- The Java Foreign Function and Memory API is a standard JDK feature in Java SE
  22, but it is not an Android-supported surface that this project can rely on
  today.
- The immediate profiling problems are symbolization and startup noise, not the
  existence of the bridge itself.

Short-term plan:

- keep JNA/UniFFI
- warm the bridge before native capture
- make the Rust frames below the bridge visible in Android outputs

## Testing Strategy

### Unit Tests

- manifest serialization for native plus semantic sections
- Android symbolization rewrite logic
- iOS/native renderer behavior
- semantic phase merge and summary rendering

### Smoke Tests

- local Android smoke test confirming symbolized Rust frames appear in rendered
  outputs
- local iOS smoke test confirming the FFI-to-Rust chain is preserved
- semantic phase smoke test confirming phase timings land in `profile.json` and
  `summary.md`

### Documentation Checks

- README capability matrix remains honest
- docs distinguish sampled native stacks from semantic phase profiling

## Rollout

Roll out in this order:

1. Android native symbolization and warm profiling
2. native plain-text report output
3. semantic profiling API in `mobench-sdk`
4. manifest and summary extensions for semantic phases
5. iOS capture metadata cleanup and warm profiling polish

This ordering improves the current flamegraphs first, then adds the semantic
layer users need for proving benchmarks.
