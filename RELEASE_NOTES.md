# Release Notes

`mobench`, `mobench-sdk`, and `mobench-macros` were published rapidly during
bring-up. Treat only the current release line as supported for new integrations
unless an older release is explicitly called out. For a concise
release-by-release change list, see `CHANGELOG.md`.

Crates.io release pages:

- [mobench](https://crates.io/crates/mobench)
- [mobench-sdk](https://crates.io/crates/mobench-sdk)
- [mobench-macros](https://crates.io/crates/mobench-macros)

## Unreleased

No user-facing unreleased changes yet.

## v0.1.49

Status: current supported release.

Publication date: 2026-07-31.

### Native Android ABI and diagnostics

The native C ABI Android runner now maps Rust `usize` arguments and
`MobenchBuf` lengths/capacities with JNA `NativeLong`. This preserves the
correct LP32/LP64 layout on supported Android ABIs, including `armeabi-v7a`,
and prevents the pre-execution `SIGBUS`/`BUS_ADRALN` failure caused by fixed
width Kotlin `Long` fields.

Native reports can now include application-defined scalar metrics through
`mobench_sdk::record_sample_u64` and `record_run_u64`. Sample values and
run-wide values are serialized under `custom_metrics`; timing remains owned by
the harness. Native ABI panic errors retain string panic payloads, and Android
runners report a dead worker promptly instead of waiting for the full timeout.

BrowserStack artifact uploads use HTTP/1.1 with no implicit total request
deadline, while retaining a bounded connection timeout, for reliable large
IPA/AAB transfers.

### Public ProveKit browser SDK release coverage

The downstream WASM release gate now builds a lockfile-pinned
`@worldcoin/provekit@0.1.0` browser fixture and executes it through Mobench's
`run-web` BrowserStack Automate path. The fixture initializes the public SDK
with `threads: false` for cross-browser compatibility, loads pinned PKP/PKV
artifacts, proves and verifies deterministic inputs, and rejects a tampered
proof. It runs alongside, rather than replacing, the Rust ProveKit WASM lane.

### Migration From v0.1.48

Update `mobench`, `mobench-sdk`, and `mobench-macros` to `0.1.49` as
applicable. Existing benchmark configurations remain compatible. Native C ABI
users should regenerate Android runners to receive the 32-bit ABI correction.

## v0.1.48

Status: superseded by `v0.1.49`.

Publication date: 2026-07-23.

### Browser-hosted WASM benchmarks

`mobench-sdk` now uses browser `performance.now()` timing and exports the
benchmark registry through a `wasm-bindgen` JSON bridge. `cargo mobench build
--target web` produces a static bundle, while `cargo mobench run-web` drives
that bundle through BrowserStack Automate's W3C WebDriver endpoint on desktop
and real-mobile browsers.

### Real downstream release gate

The manual release self-test now pins real ProveKit and world-id-protocol
fixtures. A release candidate must pass ProveKit passport age-check proving and
World ID nullifier proving on two Android devices, two iOS devices, and browser
WASM on macOS Safari, Windows Chrome, iOS Safari, and Android Chrome. The
secretless build jobs patch the pinned consumers to the candidate SDK; native
credentialed jobs still consume only prebuilt packages.

The gate pins each downstream Rust toolchain exactly, permits up to three
explicitly requested attempts for transient failures in secretless fixture
preparation, and leaves long WebDriver HTTP requests bounded by the configured
script/page and outer workflow timeouts instead of reqwest's 30-second default.
Browser proofs run in a dedicated module worker and return through page-owned
polling, keeping the WebDriver page responsive and avoiding the shorter
server-side atom limit imposed by some real-mobile sessions.

### Migration From v0.1.47

Update `mobench`, `mobench-sdk`, and `mobench-macros` to `0.1.48` as
applicable. Pin reusable workflow callers to immutable commit
`9495b599d8250a5e992874aca61a6a0653b381ff`:

```yaml
uses: worldcoin/mobile-bench-rs/.github/workflows/reusable-bench.yml@9495b599d8250a5e992874aca61a6a0653b381ff
with:
  mobench_version: "0.1.48"
```

Existing native configurations remain compatible; browser benchmarks can opt
into `build --target web` and `run-web`.

## v0.1.47

Status: superseded by `v0.1.48`.

Publication date: 2026-07-20.

### Reliable Long-Running BrowserStack Sessions

The reusable workflow now accepts `max_completion_timeout_secs`, defaulting to
1,800 seconds and capped by the trusted control plane at 21,600 seconds. The
caller-provided manifest may request a shorter completion timeout, but it cannot
extend execution beyond this trusted ceiling. iOS uses the resolved bounded
timeout consistently for the application launch configuration, XCUITest wait,
and BrowserStack polling.

Generated SwiftUI runners expose a low-impact heartbeat control that XCUITest
interacts with every five minutes during long benchmarks. The XCUITest suite
predicate-waits in 30-second intervals, emits heartbeat diagnostics, and reads
the finished report from the first accessibility channel containing valid JSON.
The BrowserStack XCUITest request also uses a 15-minute idle timeout. Completion
is based on the marker's completed state, not merely the marker's existence.

Generated native-C-ABI Android runners now identify the isolated worker by PID
and process name. The instrumentation watchdog detects a dead worker after a
bounded grace period and emits structured `worker_exit` diagnostics instead of
waiting for the full benchmark timeout. Native linkage failures, including JNA
`UnsatisfiedLinkError`, are returned as benchmark failures when the process can
still report them.

Credentialed iOS and Android jobs run serially so repositories using both
platforms do not exceed constrained BrowserStack account concurrency. Android
can still proceed when the iOS run fails or is skipped, preserving the second
platform's diagnostics instead of suppressing it. Successful result summaries
still require every requested function, device, measured iteration, and warmup
for each included platform.

### Exact-Head Validation And Credential Boundary

All five PR-head API checks now retry transient GitHub API failures up to five
times with bounded backoff. The checks remain fail-closed: exhausted requests or
any SHA mismatch stop the workflow, and the credentialed jobs revalidate the
exact current PR head immediately before BrowserStack use.

The security architecture is otherwise unchanged. Pull-request code, hooks,
dependencies, Cargo, Gradle, and Xcode execute only in secretless read-only
prepare jobs. Credentialed jobs do not check out caller code or execute caller
commands; they verify the enumerated manifest and upload opaque mobile packages
for execution on BrowserStack devices. Reporting remains isolated, secrets are
passed explicitly, and normal benchmark paths do not receive `contents: write`.

### Migration From v0.1.46

Update `mobench`, `mobench-sdk`, and `mobench-macros` to `0.1.47` as applicable.
Pin reusable workflow callers to immutable commit
`f36ea5420bdb6633a6b8e91b00522ca9d5a2a84f`. Remove temporary branch-based
`mobench_ref` values once the crates.io packages are available:

```yaml
uses: worldcoin/mobile-bench-rs/.github/workflows/reusable-bench.yml@f36ea5420bdb6633a6b8e91b00522ca9d5a2a84f
with:
  mobench_version: "0.1.47"
```

Existing inputs remain compatible. Set `max_completion_timeout_secs` only when
a benchmark needs more than the 30-minute trusted default. Continue passing
BrowserStack secrets explicitly and never use `secrets: inherit`.

## v0.1.46

Status: superseded by `v0.1.47`.

Publication date: 2026-07-19.

### Pinned Toolchains And Native Prebuilt Preparation

Reusable BrowserStack callers can now pass `rust_toolchain` with an exact
channel such as `nightly-2026-03-04`. The input defaults to `stable`. Every
requested Android or iOS target is installed for that exact caller build
toolchain in the secretless prepare job. The trusted `cargo-mobench` control
plane remains pinned independently and builds with its own stable toolchain.

`cargo mobench ci prepare` now accepts typed `--ffi-backend` values. Selection
precedence is the explicit CLI or reusable-workflow value, then
`[project].ffi_backend` in `mobench.toml`, then the compatibility default
`uniffi`. Android and iOS builders receive the resolved backend in every build
and packaging path. `native-c-abi` preparation therefore emits native runners
and never invokes `uniffi-bindgen`. The Android native runner now also matches
the generated Espresso timeout/failure interface, allowing both APKs to compile
and package as one verified prebuilt handoff.

For actual UniFFI projects, the reusable workflow resolves the lockfile through
the Cargo workspace containing `crate_path`. It no longer selects the first
nested `Cargo.lock` found in a monorepo. Native C ABI and BoltFFI projects skip
UniFFI generator installation entirely.

### Migration From v0.1.45

Pin the reusable workflow to immutable commit
`1ac54adaf2bd97c6ca303705e1e0471257716f48` and install `mobench 0.1.46`.
Pinned-toolchain callers should pass their exact toolchain explicitly:

```yaml
jobs:
  mobench:
    uses: worldcoin/mobile-bench-rs/.github/workflows/reusable-bench.yml@1ac54adaf2bd97c6ca303705e1e0471257716f48
    with:
      rust_toolchain: nightly-2026-03-04
      ffi_backend: native-c-abi
```

Omit `ffi_backend` to use `mobench.toml`, or omit both sources to retain the
UniFFI default. Continue passing BrowserStack secrets explicitly; do not use
`secrets: inherit`.

The two-stage authorization boundary is unchanged. Caller checkout, toolchain
selection, generator discovery, hooks, dependencies, and mobile builds remain
secretless. Credentialed jobs verify and execute only the fixed trusted
control-plane command against enumerated prebuilt artifacts; they never check
out the caller or run Cargo, Gradle, Xcode, or caller scripts.

## v0.1.45

Status: superseded by `v0.1.46`.

Publication date: 2026-07-17. The prebuilt BrowserStack path passed on Android
and iOS, including exact-head validation, secretless packaging, credentialed
prebuilt execution, complete-matrix sanitization, and isolated reporting.

### Secure Workflow Compatibility

The reusable workflow now accepts an optional `prepare_script` for generic
caller-owned fixture generation and toolchain setup. The value must be a
normalized repository-relative path resolving to a regular file inside the
exact PR checkout. It runs with `MOBENCH_CI_PREPARE=1` only in the secretless,
read-only platform preparation jobs. A hook failure prevents packaging and
manifest upload.

Callers may select different benchmarks with `functions_ios` and
`functions_android`; an empty platform input falls back to `functions`.
Structured `ios_devices` and `android_devices` inputs accept arrays of
`{"device":"...","os_version":"..."}` objects. They override the legacy
single-device fields and device profile while preserving those fields as the
compatibility fallback.

Each function is packaged once per platform, then the trusted prebuilt runner
submits all requested devices without rebuilding caller code. The run is
complete only when every requested function/device pair has exactly one result.
Missing, unexpected, and duplicate shards fail closed, while BrowserStack
diagnostic artifacts remain available for investigating partial failures.

The authorization invariant is unchanged: an authorized `/mobench` command
authorizes a run but does not make the requested PR revision trusted. PR code
still never executes in a job containing BrowserStack credentials or a
write-capable repository token.

### Migration From v0.1.44

Existing callers remain compatible without new inputs. Update the reusable
workflow reference and the explicitly installed mobench version to the final
immutable `v0.1.45` release revision. Add `prepare_script` only when the project
needs caller-specific preparation, and use the platform function/device inputs
only when the shared selections are insufficient. Continue passing
BrowserStack secrets explicitly; do not use `secrets: inherit`.

The CLI package again declares `mobench` as its default binary, and SDK rustdoc
examples consistently reference `0.1.45`.

## v0.1.44

Status: superseded by `v0.1.45`.

Publication date: 2026-07-17. The prebuilt BrowserStack path passed on Android
and iOS, including exact-head validation, secretless packaging, credentialed
prebuilt execution, sanitization, and isolated reporting.

### BrowserStack Pull-Request Trust Boundary

The reusable workflow now assumes that an authorized `/mobench` request can
target a malicious fork revision. Authorization to request a benchmark is not
treated as trust in the requested commit.

- `validate-request` accepts only a full commit SHA and verifies that it is the
  current head of the requested pull request.
- `prepare-android` and `prepare-ios` check out that exact revision, run fixture
  hooks and builds without BrowserStack secrets or protected environments, and
  have read-only repository permissions. Their only handoff is an explicitly
  enumerated mobile artifact set plus a machine-readable manifest.
- `run-android` and `run-ios` do not check out the caller and
  do not invoke caller scripts, Cargo, Gradle, Xcode, dependencies, fixture
  generators, or mobile binaries on the GitHub runner. A trusted, immutable
  mobench revision verifies the manifest and uploads only the prebuilt
  APK/Android test APK or IPA/XCUITest suite. Mobile code executes only on
  BrowserStack devices.
- BrowserStack credentials are scoped to the upload/run/fetch operations in the
  credentialed jobs. An environment approval is defense in depth, not the
  security boundary.
- Result summarization remains read-only. Sticky comments/check updates happen
  in a separate reporting job with only the required PR/check write permission
  and no caller checkout. Plot-branch writes are disabled unless a caller
  explicitly opts into the protected publishing job.

The artifact manifest binds the artifact set to one platform and benchmark ABI,
and binds each normalized relative path to its artifact role, byte size, and
SHA-256 digest. Verification rejects absolute or traversing paths, missing or
unexpected files, invalid metadata, size mismatches, and digest mismatches
before credentials are used.
Downloaded report fields and filenames remain untrusted and are escaped or
rejected before shell, workflow-command, path, Markdown, or HTML use.
BrowserStack API bodies, device logs, and downloaded diagnostics are bounded;
provider error bodies are not echoed into the Actions command stream, and
manifest-provided completion timeouts cannot exceed the trusted runner cap.

### New Two-Stage CLI Interface

`cargo mobench ci prepare --target <android|ios> --source-sha <full-sha>
--manifest <path>` builds and packages the untrusted revision and writes the
artifact manifest.

`cargo mobench ci run-prebuilt --manifest <path> --expected-source-sha
<full-sha> --expected-platform <android|ios> --expected-functions <functions>
--expected-iterations <count> --expected-warmup <count> --devices <selection>
--output-dir <path>` verifies that handoff and performs only trusted BrowserStack
upload, run, fetch, and report normalization.
`run-prebuilt` never invokes build tools, caller hooks, or files from a caller
checkout.

### Android Result Handoff And Collection Fixes

Android runner result codes now use a qualified mobench namespace instead of an
unqualified `RESULT_OK` that can collide with `Activity.RESULT_OK`. BrowserStack
Espresso collection now reads per-test-case logs, reconstructs bounded chunked
benchmark JSON, and rejects oversized text/report payloads. The fix covers the
UniFFI, native C ABI, and BoltFFI runner paths.

### Migration For Reusable Workflow Callers

Existing callers should update their reusable workflow reference to the
immutable `v0.1.44` release commit, keep passing the exact PR number/head SHA,
grant `actions: read`, `contents: read`, `pull-requests: write`, and
`checks: write` at the caller level for reporting, and pass
`BROWSERSTACK_USERNAME` and `BROWSERSTACK_ACCESS_KEY` explicitly.
Do not use `secrets: inherit`. Platform, function, iteration, warmup, device,
artifact, summary, and sticky-comment inputs remain available; callers do not
need to copy the split-job YAML into their own repositories.

## v0.1.43

Status: superseded by `v0.1.44`.

Publication date: 2026-07-05.

### CI Split-Run Merging

- Added `cargo mobench ci merge-split-runs` for CI workflows that run each
  measured sample as a separate `cargo mobench ci run` invocation.
- Merges `sample-*/summary.json` inputs back into standard `summary.json`,
  `summary.md`, and `results.csv` outputs.
- Validates the requested benchmark function, requested device, one benchmark
  per input summary, one device per input summary, and exact measured sample
  count before writing merged outputs.
- Recomputes `samples_ns`, `min_ns`, `max_ns`, `mean_ns`, `median_ns`, `p95_ns`,
  and resource columns so existing report, plot, PR comment, and comparison
  tooling can consume merged results unchanged.
- Documents the split-sample workflow for long or fragile BrowserStack lanes
  that need to run each measured sample as its own provider invocation.

## v0.1.42

Status: superseded by `v0.1.43`.

Publication date: 2026-06-29.

### Native C ABI Release Hardening

- Fixed `cargo mobench ci run` and `mobench run` build helpers so
  config-selected `native-c-abi` backends propagate through Android builds, iOS
  builds, CI runs, and iOS BrowserStack packaging.
- Prevented CI run flows from rebuilding native C ABI projects with the default
  UniFFI backend.
- Kept BrowserStack log/result extraction compatible across `uniffi` and
  `native-c-abi` generated runners.
- Documented the merged `boltffi` runner backend alongside `uniffi` and
  `native-c-abi`.
- Refreshed the root README, crate READMEs, release docs, and Mermaid diagrams
  for the current backend matrix, profiling artifact layout, and CI output
  contract.

## v0.1.41

Status: superseded by `v0.1.42`.

- Added `[project].ffi_backend` with `uniffi` as the compatibility default and
  `native-c-abi` as the direct mobench JSON C ABI benchmark runner backend.
- Added `mobench_sdk::export_native_c_abi!()` and `MobenchBuf` so
  registry-based benchmark crates export:
  - `mobench_run_benchmark_json`
  - `mobench_free_buf`
  - `mobench_last_error_message`
- Updated Android and iOS builders to branch on the selected FFI backend, skip
  UniFFI binding generation for `native-c-abi`, and generate native JSON C ABI
  runner templates.
- Added native C ABI headers to generated iOS frameworks when that backend is
  selected.

## v0.1.37

Status: superseded by `v0.1.41`.

- Added `cargo mobench profile run --trace-events-output <path>` for downstream
  consumers that need machine-readable harness trace/event JSON.
- Added the `mobench-sdk` `registry` feature for benchmark macro registration,
  inventory discovery, and runtime execution without builder/template
  dependencies.
- Moved generated FFI wrapper example benchmark crates to the narrower
  `registry` feature instead of the full SDK build-tooling feature set.
- Added property-test coverage for run config device matrix parsing.

## v0.1.36

Status: superseded by `v0.1.37`.

- Added production-readiness documentation for public API boundaries, semver
  expectations, feature flags, MSRV, release checks, examples, and launch
  diagrams.
- Added Rust quality CI covering rustfmt, clippy, rustdoc, tests, and
  manually-triggered publish dry-runs.
- Added opt-in structured CLI tracing through `--verbose` or `MOBENCH_LOG`, plus
  explicit `doctor` MSRV checks.
- Added host-only fixture contract coverage for stable Markdown and CSV
  rendering.
- Hardened clean first-run spec embedding for generated Android and iOS
  projects.
- Restricted authenticated BrowserStack artifact downloads to BrowserStack HTTPS
  hosts.
- Restored config-file runs without duplicate `--target` / `--function` flags
  while preserving CLI-over-config precedence.
- Tightened generated mobile template compatibility around minimal UniFFI report
  types.
- Added compile-fail coverage for async benchmark functions and setup/teardown
  error behavior.

## v0.1.35

Status: superseded by `v0.1.36`.

- Added iOS benchmark app process peak memory reporting using Mach `task_info`.
- Marked iOS process peak resources as `memory_process = "benchmark_app"` to
  match the Android summary contract while reflecting iOS app-process execution.
- Added Android foreground service type metadata required by newer Android
  platform rules.

## v0.1.34

Status: superseded by `v0.1.35`.

- Rendered one SVG plot per benchmark function in the
  `Device Comparison Plots` summary section.
- Standardized benchmark-scoped resource columns in `results.csv`.
- Added BrowserStack metric normalization documentation.

## v0.1.33

Status: superseded by `v0.1.34`.

- Measured benchmark CPU time as process user-plus-kernel time across all
  threads.
- Reworked rendered CI summaries into one table covering wall mean, wall total,
  CPU median, CPU total, CPU-to-wall ratio, and peak memory columns.
- Exposed `mobench_ref` and `mobench_version` on manual Mobile Bench workflow
  branch-pinned validation.

## v0.1.32 and earlier

Status: historical test builds unless explicitly noted in package metadata. Do
not use for new integrations.

Earlier releases covered the initial CI contract, BrowserStack orchestration,
setup/teardown macro support, generated mobile templates, device matrices, and
consolidation of old `mobench-runner` functionality into `mobench-sdk`.

## Published Version History

| Version | Published | Published crates | Status |
| --- | --- | --- | --- |
| `v0.1.49` | 2026-07-31 | `mobench 0.1.49`, `mobench-sdk 0.1.49`, `mobench-macros 0.1.49` | Current supported release |
| `v0.1.48` | 2026-07-23 | `mobench 0.1.48`, `mobench-sdk 0.1.48`, `mobench-macros 0.1.48` | Superseded by `v0.1.49` |
| `v0.1.47` | 2026-07-20 | `mobench 0.1.47`, `mobench-sdk 0.1.47`, `mobench-macros 0.1.47` | Superseded by `v0.1.48` |
| `v0.1.46` | 2026-07-19 | `mobench 0.1.46`, `mobench-sdk 0.1.46`, `mobench-macros 0.1.46` | Superseded by `v0.1.47` |
| `v0.1.45` | 2026-07-17 | `mobench 0.1.45`, `mobench-sdk 0.1.45`, `mobench-macros 0.1.45` | Superseded by `v0.1.46` |
| `v0.1.44` | 2026-07-17 | `mobench 0.1.44`, `mobench-sdk 0.1.44`, `mobench-macros 0.1.44` | Superseded by `v0.1.45` |
| `v0.1.43` | 2026-07-05 | `mobench 0.1.43`, `mobench-sdk 0.1.43`, `mobench-macros 0.1.43` | Superseded by `v0.1.44` |
| `v0.1.42` | 2026-06-29 | `mobench 0.1.42`, `mobench-sdk 0.1.42`, `mobench-macros 0.1.42` | Superseded by `v0.1.43` |
| `v0.1.41` | 2026-05-14 | `mobench 0.1.41`, `mobench-sdk 0.1.41`, `mobench-macros 0.1.41` | Superseded by `v0.1.42` |
| `v0.1.37` | 2026-04-27 | `mobench 0.1.37`, `mobench-sdk 0.1.37`, `mobench-macros 0.1.37` | Superseded by `v0.1.41` |
| `v0.1.36` | 2026-04-27 | `mobench 0.1.36`, `mobench-sdk 0.1.36`, `mobench-macros 0.1.36` | Superseded by `v0.1.37` |
| `v0.1.35` | 2026-04-24 | `mobench 0.1.35`, `mobench-sdk 0.1.35`, `mobench-macros 0.1.35` | Superseded by `v0.1.36` |
| `v0.1.34` | 2026-04-23 | `mobench 0.1.34`, `mobench-sdk 0.1.34`, `mobench-macros 0.1.34` | Superseded by `v0.1.35` |
| `v0.1.33` | 2026-04-17 | `mobench 0.1.33`, `mobench-sdk 0.1.33`, `mobench-macros 0.1.33` | Superseded by `v0.1.34` |
| `v0.1.32` and earlier | 2026-01 through 2026-04 | `mobench`, `mobench-sdk`, `mobench-macros` pre-support publishes | Historical |
