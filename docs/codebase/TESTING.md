# Testing

Updated: 2026-04-01

## Host-side Rust tests

Primary commands:

```bash
cargo test --workspace
cargo test -p mobench profile_ -- --nocapture
cargo test -p mobench flamegraph_viewer -- --nocapture
cargo test -p mobench devices_ -- --nocapture
cargo test -p mobench-sdk -- --nocapture
```

Common coverage areas:
- timing/registry behavior
- build/config resolution
- BrowserStack device resolution and fetch parsing
- profile planning/execution semantics
- flamegraph viewer HTML generation
- template regeneration and binding refresh behavior

## Fixture validation

The repository keeps a lightweight fixture CI loop around `examples/ffi-benchmark`.

Key paths:
- `scripts/ci/verify-example-plot-fixture.sh`
- `.github/workflows/mobile-bench.yml`
- `.github/workflows/mobile-bench-plot-fixtures.yml`

Typical local validation:

```bash
cargo build -p mobench --bins --locked
export PATH="$PWD/target/debug:$PATH"
scripts/ci/verify-example-plot-fixture.sh basic
scripts/ci/verify-example-plot-fixture.sh ffi
```

## Local profiling smoke tests

These are intentionally separate from BrowserStack benchmark validation.

Typical smoke commands:

```bash
cargo run -p mobench --bin mobench -- profile run \
  --target android \
  --provider local \
  --backend android-native \
  --crate-path crates/sample-fns \
  --function sample_fns::fibonacci

cargo run -p mobench --bin mobench -- profile run \
  --target ios \
  --provider local \
  --backend ios-instruments \
  --crate-path crates/sample-fns \
  --function sample_fns::fibonacci
```

Expected outputs:
- run-scoped `profile.json`
- `summary.md`
- raw and processed native artifacts
- `flamegraph.html`
- `frame-locations.json` on Android when source metadata is available
- `profile-diff.json` / `summary.md` under `target/mobench/profile/diff/` for differential comparisons

## Workflow-level testing

Benchmark workflows:
- BrowserStack fixture benchmark workflow
- plot fixture workflow
- self-test / PR-dispatch workflow chain

Profiling workflow:
- local native profiling self-test workflow, used to verify the flamegraph path without implying BrowserStack-native profiling support

## Testing guidance

- prefer exact, focused tests for CLI/help/error text changes
- do not fake successful profile captures; unsupported paths should fail explicitly
- when template behavior changes, verify both the generator and at least one real generated artifact path
- before merging, rerun the focused profile and flamegraph viewer suites even if the change was “docs only” when comments or doc strings touched command/help output
