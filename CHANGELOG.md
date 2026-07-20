# Changelog

All notable user-facing changes to `mobench`, `mobench-sdk`, and
`mobench-macros` are tracked here by release. See `RELEASE_NOTES.md` for the
longer integration-oriented release notes and support status.

## Unreleased

No user-facing unreleased changes yet.

## v0.1.47 - 2026-07-20

### Added

- Added a reusable-workflow `max_completion_timeout_secs` input with a trusted
  30-minute default and a hard six-hour control-plane maximum for long-running
  prebuilt BrowserStack benchmarks.
- Added XCUITest heartbeat interactions and accessibility-backed report
  transport so long iOS benchmarks stay active and return validated JSON across
  supported iOS/Xcode combinations.
- Added native Android worker PID/process reporting and early worker-death
  detection, including structured failures for native linkage errors.

### Changed

- Serialized credentialed iOS and Android jobs to respect constrained
  BrowserStack account concurrency while allowing Android to proceed when the
  iOS lane fails or is skipped.
- Reused the trusted completion timeout for iOS app launch, XCUITest, and
  BrowserStack polling so every layer observes the same bounded run window.

### Fixed

- Fixed iOS completion detection so the XCUITest waits for the completed state
  instead of treating the existence of a hidden marker as completion.
- Fixed native Android failures such as JNA `UnsatisfiedLinkError` being hidden
  behind the full benchmark timeout.
- Added five bounded retries to every GitHub PR-head lookup. Transient API
  failures retry, while an exhausted lookup or mismatched SHA still fails
  closed before build or credential use.

### Security

- Preserved the secretless prepare and credentialed prebuilt execution split.
  Retry, timeout, heartbeat, and result-transport changes do not introduce a
  caller checkout or caller-controlled command in credentialed jobs.

## v0.1.46 - 2026-07-19

### Added

- Added the reusable-workflow `rust_toolchain` input, defaulting to `stable`,
  so pinned callers can install every mobile target on their exact Rust
  toolchain while the trusted control plane remains on its own stable toolchain.
- Added typed `cargo mobench ci prepare --ffi-backend` selection with explicit
  CLI values taking precedence over `mobench.toml` and the UniFFI default.

### Fixed

- Propagated the resolved FFI backend through every Android and iOS builder
  path, including secretless prebuilt preparation, so native C ABI projects
  generate native runners without invoking `uniffi-bindgen`.
- Fixed the generated Android native C ABI runner and Espresso test interface
  so the app and test APKs compile and package together.
- Resolved UniFFI generator versions from the Cargo workspace containing
  `crate_path` instead of an arbitrary nested lockfile, and skipped UniFFI
  generator installation for native C ABI and BoltFFI callers.

### Security

- Preserved the secretless prepare and credentialed prebuilt execution split.
  Toolchain and lockfile decisions influenced by caller files remain confined
  to untrusted prepare jobs; credentialed jobs still have no caller checkout or
  caller-controlled build command.

## v0.1.45 - 2026-07-17

### Added

- Added the optional `prepare_script` reusable-workflow input for caller-specific
  code generation and toolchain setup. The normalized repository-relative hook
  runs only in secretless, read-only prepare jobs with
  `MOBENCH_CI_PREPARE=1`; invalid, escaping, missing, or failing hooks stop the
  handoff before any manifest is uploaded.
- Added `functions_ios` and `functions_android`, each falling back to the shared
  `functions` input, plus structured `ios_devices` and `android_devices` JSON
  arrays for platform-specific multi-device runs.

### Changed

- Reusable BrowserStack runs now require the complete function/device matrix.
  Missing, unexpected, or duplicate result shards fail the run instead of
  producing a successful partial summary.
- Generated CI workflows use the secure two-stage reusable workflow by default;
  the local composite action remains appropriate only for trusted revisions or
  secretless execution.

### Fixed

- Restored `cargo run -p mobench -- ...` binary selection by placing
  `default-run` in the package manifest, and updated SDK rustdoc dependency
  examples to the `0.1.45` release line.

### Security

- Preserved the `0.1.44` credential boundary: caller hooks, dependencies, and
  builds remain confined to untrusted prepare jobs, while credentialed jobs use
  only verified prebuilt mobile packages and the SHA-pinned trusted mobench
  control plane.

## v0.1.44 - 2026-07-17

### Security

- Split the reusable BrowserStack workflow into untrusted prepare jobs and
  credentialed prebuilt-run jobs. Pull-request code is built without secrets,
  protected environments, or write-capable GitHub permissions; credentialed
  jobs never check out or execute the pull-request revision on the GitHub
  runner.
- Added strict manifest verification for the enumerated APK, Android test APK,
  IPA, and XCUITest artifacts, including normalized paths, platform and
  benchmark ABI metadata, file sizes, and SHA-256 hashes.
- Isolated sticky PR/check reporting from benchmark execution and treats report
  artifacts, filenames, benchmark names, Markdown, CSV, and JSON as untrusted
  input.

### Added

- Added `cargo mobench ci prepare` for unprivileged mobile builds and
  `cargo mobench ci run-prebuilt` for trusted upload, BrowserStack execution,
  result collection, and CI contract output from verified prebuilt artifacts.
- Added trust-boundary regression fixtures covering hostile build scripts,
  fixture hooks, dependencies, benchmark code, artifact paths, and report
  fields.

### Changed

- Reusable workflow callers receive the two-stage architecture by default.
- Workflow-level permissions are empty; jobs receive only the read or narrowly
  scoped write permissions they need. Plot-branch publication remains an
  explicit, protected opt-in instead of a default benchmark permission.
- `/mobench` dispatches exact fork PR heads instead of treating fork origin as
  trust, and PR callers must supply both the PR number and full head SHA.
- Third-party workflow actions are pinned to immutable commit SHAs and
  BrowserStack secrets must be passed explicitly.
- Bounded credentialed BrowserStack API, device-log, and diagnostic downloads,
  neutralized provider response bodies before runner logging, and capped
  PR-provided completion timeouts against a trusted control-plane maximum.

### Fixed

- Prevented Android `ResultReceiver` success codes from colliding with
  `Activity.RESULT_OK` across the UniFFI and native C ABI runners, and kept the
  BoltFFI runner's result markers aligned with the collector contract.
- Hardened BrowserStack Android result collection for per-test-case Espresso
  logs and chunked benchmark JSON while bounding downloaded text and report
  sizes.

## v0.1.43 - 2026-07-05

### Added

- Added `cargo mobench ci merge-split-runs` for CI workflows that run each
  measured sample as its own `cargo mobench ci run` invocation.
- Added split-run merge documentation for long or fragile BrowserStack lanes.

### Changed

- Split-run merging writes the same `summary.json`, `summary.md`, and
  `results.csv` contract used by normal `mobench ci run` output.
- Merged summaries recompute measured timing statistics and resource columns so
  existing report, plot, PR comment, and comparison tooling can consume them
  unchanged.

### Validation

- Merge inputs are rejected unless they contain exactly one device, exactly one
  benchmark result, the requested benchmark function, the requested device, and
  exactly the requested measured sample count.

## v0.1.42 - 2026-06-29

### Fixed

- Propagated config-selected `native-c-abi` backends through Android builds, iOS
  builds, CI runs, and iOS BrowserStack packaging.
- Prevented CI flows from rebuilding native C ABI projects with the default
  UniFFI backend.

### Changed

- Kept BrowserStack log/result extraction compatible across `uniffi` and
  `native-c-abi` generated runners.
- Refreshed documentation for the backend matrix, profiling artifact layout, and
  CI output contract.

## v0.1.41 - 2026-05-14

### Added

- Added `[project].ffi_backend` with the default `uniffi` backend and direct
  `native-c-abi` JSON runner support.
- Added `mobench_sdk::export_native_c_abi!()` and native C ABI exports for
  registry-based benchmark crates.
- Added generated native C ABI runner templates for Android and iOS.
- Added native C ABI headers to generated iOS frameworks when that backend is
  selected.

## v0.1.37 - 2026-04-27

### Added

- Added `cargo mobench profile run --trace-events-output <path>` for
  machine-readable harness trace/event JSON.
- Added the `mobench-sdk` `registry` feature for benchmark macro registration,
  inventory discovery, and runtime execution without builder/template
  dependencies.
- Added property-test coverage for run config device matrix parsing.

### Changed

- Narrowed generated FFI wrapper example crates to the `registry` feature
  instead of the full SDK build-tooling feature set.

## v0.1.36 - 2026-04-27

### Added

- Added production-readiness documentation for public APIs, semver boundaries,
  feature flags, MSRV, release checks, examples, and diagrams.
- Added Rust quality CI covering rustfmt, clippy, rustdoc, tests, and manual
  publish dry-runs.
- Added opt-in structured CLI tracing through `--verbose` or `MOBENCH_LOG`.
- Added explicit `doctor` MSRV checks.
- Added host-only fixture contract coverage for stable Markdown and CSV
  rendering.

### Fixed

- Hardened clean first-run spec embedding for generated Android and iOS
  projects.
- Restricted authenticated BrowserStack artifact downloads to BrowserStack HTTPS
  hosts.
- Restored config-file runs without duplicate `--target` or `--function` flags
  while preserving CLI-over-config precedence.
- Tightened generated mobile template compatibility around minimal UniFFI report
  types.
- Added compile-fail coverage for async benchmark functions and setup/teardown
  error behavior.

## v0.1.35 - 2026-04-24

### Added

- Added iOS benchmark app process peak memory reporting using Mach `task_info`.
- Added Android foreground service type metadata required by newer Android
  platform rules.

### Changed

- Marked iOS process peak resources with
  `memory_process = "benchmark_app"` to match the Android summary contract while
  reflecting iOS app-process execution.

## v0.1.34 - 2026-04-23

### Added

- Added Mobile Bench workflow branch-pinned validation.

## v0.1.32 and earlier

Historical test builds unless explicitly noted in package metadata. Earlier
releases covered the initial CI contract, BrowserStack orchestration,
setup/teardown macro support, generated mobile templates, device matrices, and
consolidation of old `mobench-runner` functionality into `mobench-sdk`.
