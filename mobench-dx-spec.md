# mobench DX Improvement Spec

## Goals

Primary goals
- Reduce first-time integration to under 30 minutes for a new crate.
- Reduce iteration loop to under 2 minutes for edit -> run on device.
- Make BrowserStack runs predictable, reproducible, and traceable.

Secondary goals
- Make the system self-documenting: CLI output shows next steps.
- Provide clear error guidance for common failure modes.
- Ensure benchmark configuration always flows into the mobile app.

## Current Friction Summary

Integration
- Too much manual boilerplate (UniFFI types, bindgen, error mapping).
- Benchmark name discovery is not surfaced by the CLI.

Build and run
- Build vs run behavior can be unclear; output depends on cached scaffolding.
- Benchmark choice is not reliably passed into the iOS app; defaults can win.

BrowserStack
- Requires manual packaging and credential setup; weak feedback on device or test selection errors.
- Artifacts and results are not always correlated to the requested benchmark config.

Reporting
- Report may not reflect requested benchmark (function mismatch, missing config).
- Report source (local vs device vs fetched artifacts) is not explicit.

## Design Principles

- Single source of truth for benchmark configuration.
- Deterministic automation: same CLI args yield same device run.
- Clear orchestration: run always produces complete, consistent artifacts.
- Progressive disclosure: power-user options without overwhelming defaults.

## Proposed Improvements

### A. CLI and Configuration

A1. Unified benchmark spec pipeline
- CLI always writes a bench spec and ensures it is bundled into the app.
- CLI args override defaults and any prior spec.

A2. Run always uses the requested benchmark
- Validate function exists before running.
- Ensure spec is embedded in iOS bundle and Android assets.

A3. Standardized config discovery
- Support `mobench.toml` by default with precedence:
  1. CLI args
  2. mobench.toml
  3. .env.local (credentials only)
  4. defaults

A4. Benchmark name discovery
- `mobench list` always works without a full build.
- Provide clear errors when inventory registry is missing.

A5. Better CLI output
- Print resolved spec at the start of every run (function, iterations, warmup, devices, profile).
- Print exact locations for bench spec and artifacts.

### B. Build and Run Artifacts

B1. Single build+package+run path
- `mobench run` always:
  1. Generates scaffolding if missing.
  2. Builds and generates bindings.
  3. Packages IPA/XCUITest for iOS when BrowserStack is used.

B2. Deterministic build output paths
- All artifacts live under `target/mobench/{platform}`.
- No cross-repo path leakage; paths are relative to the bench crate.

B3. Explicit artifact stamping
- Embed `bench_spec.json` in iOS app bundle and Android assets.
- Add `bench_meta.json` with spec, commit hash (if available), build time, target, and profile.

### C. BrowserStack Workflow

C1. Credentials onboarding
- Detect missing credentials and suggest exact variables or config paths.

C2. Device UX
- `mobench devices` lists available device identifiers and OS versions.
- `--devices` supports fuzzy match and validation.

C3. Artifact correlation
- Every run outputs build ID, device/OS used, and local fetch paths.

C4. Fetch as default for BrowserStack
- `mobench run` defaults to fetching artifacts.
- Poll with clear progress and timeout.
- If video fetch fails, continue and warn once.

### D. Benchmark Authoring and Fixtures

D1. SDK-provided UniFFI types
- Provide `mobench-sdk::uniffi` exports for BenchSpec, BenchSample, BenchReport, BenchError.

D2. Fixture helper library
- Provide helpers like `deterministic_rng(seed)` and cached input generation.

D3. Benchmark APIs
- Support setup vs run separation for proof-only vs full pipeline benchmarks.

### E. Testing and Verification

E1. `mobench verify`
- Validate registry, spec, and artifacts; run local smoke tests where possible.

E2. Spec consistency check
- CLI check to verify bundle contains expected spec.

### F. Reporting and Data Model

F1. Structured result format
- Standard JSON report with spec, device info, samples, stats, and timestamps.
- Report must reflect spec actually run.

F2. Summary display
- `mobench summary` prints avg/min/max/median, sample count, device, and OS version.

### G. Documentation and Onboarding

G1. Quick start (10 steps max)
- Minimal flow: init -> edit -> run on device.

G2. Error-first docs
- Link to top 10 errors with fixes and recovery steps.

## Priority Roadmap

P0
- Benchmark spec always embedded in app bundle.
- `mobench run` produces complete artifacts.
- Requested benchmark selection is always honored.

P1
- `mobench list` always works.
- `mobench devices` and device validation.
- `mobench verify` and `mobench summary`.

P2
- Rich reporting dashboard.
- Spec snapshots and result comparisons across builds.

## Success Metrics

- Under 30 minutes to first device run for a new project.
- Under 2 minutes to iterate from change to device run.
- Zero mismatch between requested and measured benchmark function.
