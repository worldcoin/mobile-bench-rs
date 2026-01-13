# Mobile Bench RS – Plan

## Goals

- Package arbitrary Rust functions into Android (Kotlin) and iOS (Swift) binaries.
- Drive builds and benchmark runs via a Rust CLI that works locally and in GitHub Actions.
- Execute binaries on real devices through BrowserStack AppAutomate, collecting timing/telemetry and artifacts.
- Produce repeatable, configurable runs (device matrix, iterations, warmups) with exportable reports.

## Non-Goals (for now)

- Desktop or web benchmarks.
- Perf profiling beyond timing/throughput (e.g., flamegraphs, memory sampling).
- Real-time dashboards; focus on generated reports and CI annotations first.

## Architecture Outline

- `mobench`: Orchestrates builds, packaging, upload, AppAutomate sessions, and result collation.
- `bench-runner`: Minimal Rust harness compiled into mobile libs; exposes FFI entrypoints for target functions and collects timings.
- Mobile bindings:
  - Android: Kotlin wrapper + APK test harness embedding Rust lib (cargo-ndk); uses Espresso/Appium-style entrypoints for AppAutomate.
  - iOS: Swift wrapper + test host app/xcframework; invokes Rust via C-ABI bindings.
- CI: GitHub Actions workflows for build (per target), upload to BrowserStack, run matrix, fetch reports, and publish summary.

## MVP Scope

- Benchmark a single exported Rust function with configurable iterations.
- Build Android APK + iOS app/xcframework locally and in CI.
- Trigger one Android device run on BrowserStack and capture timing JSON.
- CLI command: `mobench run --target android --function path::to::fn --devices "pixel_7"` producing a report.

## Task Backlog (initial)

- [ ] Repo bootstrap: Cargo workspace, `mobench` binary crate, `bench-runner` library crate, example `sample-fns` crate.
- [ ] Define FFI boundary: macro/attribute to mark benchmarkable Rust functions; export through C ABI; basic timing harness.
- [ ] Android packaging: cargo-ndk config, Kotlin wrapper module, minimal test/activity to trigger Rust bench entrypoint.
- [ ] iOS packaging: xcframework build script (cargo lipo or cargo-apple), C header generation (cbindgen), Swift wrapper, test host.
- [ ] CLI scaffolding: parse config (function path, iterations, warmups, device matrix), invoke build scripts, prepare artifacts.
- [ ] BrowserStack integration: AppAutomate REST client (upload builds, start sessions, poll status, download logs/artifacts).
- [ ] Result handling: normalize timing output to JSON, aggregate across iterations/devices, emit markdown/CSV summary.
- [ ] CI: GitHub Actions workflow covering build, artifact upload, BrowserStack-triggered run (behind secrets), and report upload.
- [ ] Developer UX: local smoke test runners, sample bench functions, docs with step-by-step usage.
- [ ] Stretch: parallel device runs, retries, percentile stats, optional energy/thermal readings where available.

## In-Repo Placeholders (current)
- Scripts: `scripts/build-android.sh`, `scripts/build-ios.sh` for manual/CI builds (require Android NDK / cargo-apple).
- Android demo app: `android/` Gradle project that loads the Rust demo cdylib (`sample-fns`) and displays results.
- Workflow: `.github/workflows/mobile-bench.yml` manual build for Android; extend with BrowserStack upload/run and iOS job.
