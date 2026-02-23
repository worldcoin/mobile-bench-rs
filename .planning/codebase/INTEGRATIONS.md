# External Integrations

**Analysis Date:** 2026-01-21

## APIs & External Services

**BrowserStack App Automate:**
- Service: Cloud-based mobile device testing and automation platform
- What it's used for: Running benchmarks on real Android and iOS devices; uploading APKs, xcframeworks, and test suites; scheduling test runs; collecting results
- SDK/Client: Custom REST client in `crates/mobench/src/browserstack.rs`
- Auth: HTTP Basic Auth with username and access key
- Base URL: `https://api-cloud.browserstack.com`

**BrowserStack Espresso (Android):**
- Service: Test automation framework for Android apps
- API: `app-automate/espresso/v2/` endpoints
- Operations:
  - Upload app APK: `POST /app-automate/espresso/v2/app` (multipart form)
  - Upload test suite APK: `POST /app-automate/espresso/v2/test-suite` (multipart form)
  - Schedule run: `POST /app-automate/espresso/v2/build` (JSON request body)
  - Get build status: `GET /app-automate/espresso/v2/builds/{build_id}`
  - Get device logs: `GET /app-automate/espresso/v2/builds/{build_id}/sessions/{session_id}/devicelogs`
- Implementation: `crates/mobench/src/browserstack.rs` - methods: `upload_espresso_app()`, `upload_espresso_test_suite()`, `schedule_espresso_run()`, `get_espresso_build_status()`

**BrowserStack XCUITest (iOS):**
- Service: Test automation framework for iOS apps
- API: `app-automate/xcuitest/v2/` endpoints
- Operations:
  - Upload app IPA: `POST /app-automate/xcuitest/v2/app` (multipart form)
  - Upload test suite: `POST /app-automate/xcuitest/v2/test-suite` (multipart form, zip file)
  - Schedule run: `POST /app-automate/xcuitest/v2/build` (JSON request body with `only_testing` field)
  - Get build status: `GET /app-automate/xcuitest/v2/builds/{build_id}`
  - Get device logs: `GET /app-automate/xcuitest/v2/builds/{build_id}/sessions/{session_id}/devicelogs`
- Implementation: `crates/mobench/src/browserstack.rs` - methods: `upload_xcuitest_app()`, `upload_xcuitest_test_suite()`, `schedule_xcuitest_run()`, `get_xcuitest_build_status()`
- Test specification: Hardcoded XCUITest selector `"BenchRunnerUITests/BenchRunnerUITests/testLaunchAndCaptureBenchmarkReport"` passed in `only_testing` field

## Data Storage

**Databases:**
- Not used. All data is ephemeral (benchmark specs and results are files on disk or in memory).

**File Storage:**
- Local filesystem only (no cloud storage integration)
- Artifact locations:
  - Build output: `target/mobench/android/` and `target/mobench/ios/` (customizable with `--output-dir`)
  - Benchmark specs: `target/mobile-spec/{android,ios}/bench_spec.json` (written at build time, read by mobile apps)
  - Results: `run-summary.json`, `run-summary.csv`, `run-summary.md` (written after benchmark execution)
  - BrowserStack artifacts downloaded to: `target/mobench/` (device logs, benchmark results)

**Caching:**
- None. No persistent caching layer.

## Authentication & Identity

**Auth Provider:**
- Custom - No OAuth/OIDC provider. Uses static credentials (username + access key).

**Implementation:**
- Environment variables: `BROWSERSTACK_USERNAME`, `BROWSERSTACK_ACCESS_KEY`, `BROWSERSTACK_PROJECT` (optional)
- Config file: `bench-config.toml` or `mobench.toml` with `${ENV_VAR}` expansion
- `.env.local` file: Loaded automatically via `dotenvy` crate
- HTTP Basic Auth: Credentials passed to all BrowserStack API requests as base64-encoded Authorization header
- Credential resolution order (in `crates/mobench/src/lib.rs` around line 1444-1478):
  1. Attempt to read from config file (with env var expansion)
  2. Fall back to environment variables
  3. Fall back to `.env.local` file
  4. Error if credentials not found

## Monitoring & Observability

**Error Tracking:**
- None. No external error tracking service integrated.

**Logs:**
- Console output (stdout/stderr)
- Verbose flag (`--verbose` / `-v` in CLI) enables detailed output showing all executed commands
- Dry-run flag (`--dry-run`) previews what would be done without making changes
- BrowserStack device logs: Downloaded and saved locally after benchmark execution
  - Location: `target/mobench/` (retrieved via `get_device_logs()` in `browserstack.rs`)
  - Format: Raw device logs from BrowserStack, optionally filtered by session ID

## CI/CD & Deployment

**Hosting:**
- BrowserStack App Automate - No self-hosted deployment. All mobile execution happens on BrowserStack managed devices.
- GitHub Artifacts - Results and summaries uploaded to GitHub Actions workflow artifacts
  - Artifacts: `host-run-summary`, `android-apk-artifact`, `ios-xcframework-artifact`

**CI Pipeline:**
- GitHub Actions (`.github/workflows/mobile-bench.yml`)
- Trigger: Manual dispatch (`workflow_dispatch`) with platform selection (android, ios, or both)
- Steps:
  1. Host tests: `cargo test --all`
  2. Host benchmark summary: `cargo run -p mobench -- run --local-only` (5 iterations, 1 warmup)
  3. Android build: Compiles Rust, runs `cargo mobench build --target android`, uploads APK artifacts
  4. iOS build: Compiles Rust, runs `cargo mobench build --target ios`, creates xcframework and IPA artifacts
- Upload artifacts to GitHub Actions for download

## Environment Configuration

**Required env vars:**
- `BROWSERSTACK_USERNAME` - BrowserStack App Automate username
- `BROWSERSTACK_ACCESS_KEY` - BrowserStack App Automate access key

**Optional env vars:**
- `BROWSERSTACK_PROJECT` - Project name for builds (defaults to config or empty)
- `ANDROID_NDK_HOME` - Path to Android NDK (required for Android builds on non-standard setups)

**Secrets location:**
- GitHub Actions secrets (for CI): Not configured in this repo (would be added to `.github/` secrets)
- Local development: `.env.local` file (NOT committed to git)
- Config files: `bench-config.toml` or `mobench.toml` with `${ENV_VAR}` expansion

## Webhooks & Callbacks

**Incoming:**
- None. No webhook endpoints exposed.

**Outgoing:**
- None. BrowserStack results are polled via REST API (`get_build_status()` methods), not pushed via webhook.

## Result Collection & Aggregation

**Data Flow:**
1. Benchmark parameters written to `bench_spec.json` during build
2. Mobile app reads `bench_spec.json` at runtime
3. Mobile app calls `run_benchmark()` via UniFFI bindings
4. Results serialized to JSON and returned to app
5. App uploads results or writes to local storage
6. CLI polls BrowserStack API for build status and device logs
7. Results parsed and formatted to JSON/CSV/Markdown files

**Device Communication:**
- Android: Benchmark spec passed via Intent extras or read from `bench_spec.json` asset
- iOS: Benchmark spec read from bundle resource or environment variables
- Both: Results collected through UniFFI FFI boundary in mobile test automation code

---

*Integration audit: 2026-01-21*
