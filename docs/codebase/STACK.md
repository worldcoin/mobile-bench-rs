# Technology Stack

Updated: 2026-07-17. Release candidate: `0.1.44`.

## Languages And Formats

- Rust 2024: workspace crates.
- Rust 2021: some example/fixture crates retained for compatibility.
- Kotlin: generated Android runner/binding code.
- Swift: generated iOS runner/binding code.
- YAML: GitHub workflows and device matrices.
- TOML: Cargo manifests and mobench config.
- JSON: benchmark specs, reports, CI summaries, schemas, and trace events.
- Markdown/Mermaid: user docs, codebase docs, release notes, and diagrams.

## Core Rust Crates

- `clap`: CLI surface.
- `serde`, `serde_json`, `serde_yaml`, `toml`: config and report
  serialization.
- `anyhow`, `thiserror`: layered error handling.
- `inventory`: benchmark registration.
- `uniffi`: Kotlin/Swift binding generation for the compatibility backend.
- `include_dir`: embedded template assets.
- `reqwest` with `rustls`: BrowserStack REST calls.
- `dotenvy`: `.env.local` credential loading.
- `time`: RFC3339 timestamps and report metadata.
- `inferno`: flamegraph SVG generation.
- `tracing`, `tracing-subscriber`: CLI diagnostics.
- `comfy-table`: terminal summary tables.
- `jsonschema`, `proptest`, `criterion`: tests and benchmark support.

## Native Toolchain Dependencies

### Android

- Android SDK and build tools.
- Android NDK.
- Rust targets:
  - `aarch64-linux-android`
  - `armv7-linux-androideabi`
  - `x86_64-linux-android`
- `cargo-ndk`.
- Gradle.
- `adb`.
- `simpleperf` for local native profiling.
- NDK `llvm-addr2line` for Android frame symbolization.

### iOS

- macOS.
- Xcode and command-line tools.
- XcodeGen for generated project flows.
- Rust targets:
  - `aarch64-apple-ios`
  - `aarch64-apple-ios-sim`
  - `x86_64-apple-ios`
- `xcrun simctl`.
- macOS `sample` for current local simulator-host profiling.
- `codesign` for xcframework signing.

## External Services

- BrowserStack App Automate:
  - Android Espresso benchmark runs.
  - iOS XCUITest benchmark runs.
  - Device inventory and validation.
  - Session artifact fetching and metric enrichment.
- GitHub Actions:
  - Rust quality checks.
  - BrowserStack benchmark workflows.
  - PR benchmark dispatch.
  - Plot/profile self-tests.
  - Sticky PR comments and Check Run summaries.

## Runtime Artifacts

Benchmark outputs:

- Result JSON from `run`.
- `summary.json`.
- `summary.md`.
- `results.csv`.
- Optional plot SVGs.

Profile outputs:

- `profile.json`.
- `summary.md`.
- Raw capture artifacts.
- `stacks.folded`.
- `native-report.txt`.
- `frame-locations.json` when Android file/line metadata is available.
- `flamegraph.full.svg`.
- `flamegraph.focused.svg`.
- `flamegraph.html`.
- `artifacts/semantic/phases.json` when semantic phase data exists.
- Profile diff bundles under `target/mobench/profile/diff/`.

## Supported Execution Modes

- Host-only benchmark execution: `cargo mobench run --local-only`.
- Android/iOS local build/package flows: `cargo mobench build`.
- BrowserStack benchmark execution: `cargo mobench run` and
  `cargo mobench ci run`.
- BrowserStack artifact fetching: `cargo mobench fetch` and `--fetch`.
- Local native profiling: `cargo mobench profile run --provider local`.

Unsupported in this release:

- BrowserStack native stack/flamegraph profiling.
- Retrievable BrowserStack native profile artifacts.

## Documentation And Schema Tooling

- Markdown files in `docs/guides/` and `docs/codebase/`.
- Mermaid sources in `docs/diagrams/`.
- JSON schemas in `docs/schemas/`.
- Release notes in `RELEASE_NOTES.md`.
