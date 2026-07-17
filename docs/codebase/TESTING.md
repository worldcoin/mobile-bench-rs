# Testing

Updated: 2026-07-17. Current release candidate: `0.1.45`.

## Host-Side Rust Tests

Primary commands:

```bash
cargo test --workspace
cargo test -p mobench-sdk
cargo test -p mobench-macros
cargo test -p mobench
```

Focused suites:

```bash
cargo test -p mobench profile_ -- --nocapture
cargo test -p mobench flamegraph_viewer -- --nocapture
cargo test -p mobench devices_ -- --nocapture
cargo test -p mobench summarize -- --nocapture
```

Common coverage areas:

- Timing and registry behavior.
- Setup, teardown, and per-iteration macro behavior.
- Build/config/project resolution.
- BrowserStack device resolution and fetch parsing.
- CI summary, CSV, Markdown, plot, PR, and check-run rendering.
- Native C ABI JSON boundary.
- Profile planning/execution semantics.
- Flamegraph viewer HTML generation.
- Template regeneration and generated runner invariants.

Host-only tests should not require Android SDK, Xcode, BrowserStack
credentials, connected devices, or native profiler binaries.

## Test Taxonomy

- **Host-only**: requires only the Rust toolchain. Examples: schema validation,
  CLI parsing, summary rendering, BrowserStack JSON normalization, template text
  invariants, and timing behavior.
- **Tool-gated**: requires local platform tools but no external service.
  Examples: Android Gradle/NDK builds, iOS/Xcode packaging, `adb`,
  `simpleperf`, simulator-host `sample`, and plot rendering dependencies.
- **Service-gated**: requires credentials or remote infrastructure. Examples:
  BrowserStack benchmark execution, GitHub PR comment publishing, and workflow
  dispatch chains.

Gate host-only tests in normal Rust CI. Keep tool-gated and service-gated tests
in named workflows or explicit local commands so failures are attributable to
code, local tooling, or provider state.

## CLI Smoke Checks

Validate command surfaces before releases:

```bash
cargo run -q -p mobench --bin mobench -- --help
cargo run -q -p mobench --bin mobench -- build --help
cargo run -q -p mobench --bin mobench -- run --help
cargo run -q -p mobench --bin mobench -- ci run --help
cargo run -q -p mobench --bin mobench -- ci prepare --help
cargo run -q -p mobench --bin mobench -- ci run-prebuilt --help
cargo run -q -p mobench --bin mobench -- profile run --help
```

Validate local prerequisites:

```bash
cargo mobench check --target android
cargo mobench check --target ios
cargo mobench doctor --target both --browserstack false
```

## Fixture Validation

The repository keeps lightweight fixture CI around example crates.

Key paths:

- `examples/basic-benchmark/`
- `examples/ffi-benchmark/`
- `crates/sample-fns/`
- `.github/workflows/mobile-bench-plot-fixtures.yml`

Typical local validation:

```bash
cargo build -p mobench --bins --locked
export PATH="$PWD/target/debug:$PATH"
mobench fixture verify-plots basic
mobench fixture verify-plots ffi
```

## CI Contract Validation

Run a host-only CI contract smoke test:

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --local-only \
  --iterations 20 \
  --warmup 5 \
  --plots auto \
  --output-dir target/mobench/ci
```

Expected outputs:

- `target/mobench/ci/summary.json`
- `target/mobench/ci/summary.md`
- `target/mobench/ci/results.csv`
- `target/mobench/ci/plots/*.svg` when plots render

## Local Profiling Smoke Tests

These are separate from BrowserStack benchmark validation.

Android:

```bash
cargo run -p mobench --bin mobench -- profile run \
  --target android \
  --provider local \
  --backend android-native \
  --crate-path crates/sample-fns \
  --function sample_fns::fibonacci
```

iOS:

```bash
cargo run -p mobench --bin mobench -- profile run \
  --target ios \
  --provider local \
  --backend ios-instruments \
  --crate-path crates/sample-fns \
  --function sample_fns::fibonacci
```

Expected profile outputs:

- Run-scoped `profile.json`.
- Latest `summary.md`.
- Raw and processed native artifacts.
- `flamegraph.html`.
- `frame-locations.json` on Android when source metadata is available.
- `profile-diff.json` and `summary.md` under `target/mobench/profile/diff/`
  for differential comparisons.

BrowserStack native profiling should remain an explicit unsupported path.

## Workflow-Level Testing

Benchmark workflows:

- `mobile-bench.yml`
- `reusable-bench.yml`
- `mobile-bench-pr-auto.yml`
- `mobile-bench-pr-command.yml`
- `mobile-bench-selftest.yml`

Artifact/reporting workflows:

- `mobile-bench-plot-fixtures.yml`
- `mobile-bench-profile-selftest.yml`
- `mobile-bench-action-example.yml`

Quality workflows:

- `rust.yml`
- `compile-gate.yml`

### Reusable Workflow Security Regression Tests

The security suite must prove behavior at the job boundary, not merely inspect
the commenter's authorization:

- a hostile fixture's `build.rs`, fixture hook, dependency, and benchmark see
  no BrowserStack variables and cannot use the job token for repository writes;
- credentialed jobs have no caller checkout and invoke no caller-controlled
  process on the GitHub runner;
- caller preparation hooks are path-confined, secretless, read-only, and fail
  before manifest upload; platform function fallback and structured device
  arrays remain data rather than shell programs;
- complete-matrix tests cover multiple functions and devices and reject
  missing, unexpected, and duplicate result shards;
- manifest verification rejects traversal, absolute/duplicate/unexpected paths,
  missing/extra files, size mismatches, hash mismatches, platform mismatches,
  and incompatible benchmark ABI metadata;
- report inputs reject workflow-command/path injection and escape untrusted
  benchmark names and Markdown/HTML fields;
- `actionlint` and existing workflow/self-tests remain green.

One Android and one iOS run through `ci run-prebuilt` are service-gated release
checks. Record them separately from static and host-only checks; do not claim
live validation from fixture or workflow-shape tests.

## Testing Guidance

- Prefer exact, focused tests for CLI/help/error text changes.
- Do not fake successful profile captures; unsupported paths should fail
  explicitly.
- When template behavior changes, verify both generator output and at least one
  generated artifact path.
- When comments or docs feed command help or rustdoc output, rerun focused tests
  even if the change looks documentation-only.
