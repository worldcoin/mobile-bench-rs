# Technology Stack

Updated: 2026-03-27

## Languages

- Rust 2024: workspace root and primary implementation language
- Rust 2021: some example and fixture crates retained for compatibility
- Kotlin: generated Android bindings and runner code
- Swift: generated iOS bindings and runner code
- YAML/TOML/JSON: workflow, config, matrix, and report contracts

## Core Rust crates

- `clap`: CLI surface
- `serde`, `serde_json`, `serde_yaml`, `toml`: config and report serialization
- `anyhow`, `thiserror`: layered error handling
- `inventory`: benchmark registration
- `uniffi`: Kotlin/Swift binding generation
- `include_dir`: embedded template assets
- `reqwest` with `rustls`: BrowserStack REST calls
- `time`: RFC3339 timestamps and report metadata
- `inferno`: flamegraph SVG generation

## Native toolchain dependencies

### Android

- Rust Android targets
- Android SDK + NDK
- `cargo-ndk`
- Gradle / Android build tools
- `adb`
- `simpleperf`
- `llvm-addr2line` for symbolization

### iOS

- Rust iOS targets
- Xcode / xcodebuild
- XcodeGen for generated project flows
- `xcrun simctl`
- macOS `sample` for current local native profiling

## External services

- BrowserStack App Automate
  - benchmark execution on real devices
  - device inventory resolution
  - session artifact fetching and metric enrichment
- GitHub Actions
  - fixture benchmark workflows
  - plot fixture verification
  - PR auto-dispatch and sticky comments

## Runtime artifacts

Benchmark outputs:
- `summary.json`
- `summary.md`
- `results.csv`
- plot SVGs when enabled

Profile outputs:
- `profile.json`
- `summary.md`
- raw capture artifacts (`sample.perf`, `sample.txt`, etc.)
- processed stacks (`stacks.folded`, `native-report.txt`)
- viewer artifacts (`flamegraph.full.svg`, `flamegraph.focused.svg`, `flamegraph.html`)
- semantic sidecar data (`artifacts/semantic/phases.json`)

## Supported execution modes

- Local benchmark execution
- BrowserStack benchmark execution
- Local Android native profiling
- Local iOS native profiling

Explicitly not supported:
- BrowserStack native profiling with retrievable flamegraph-capable artifacts
