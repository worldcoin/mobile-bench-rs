# Basic Benchmark Example

This example is the smallest complete mobench SDK integration in the workspace.
Use it when validating that a downstream crate can define benchmarks with
`#[benchmark]`, discover them through the SDK registry, and execute them through
the host runner before moving to Android or iOS devices.

## What It Shows

- `mobench-sdk` and `inventory` dependency setup
- `cdylib`, `staticlib`, and `lib` crate types for mobile FFI builds
- benchmark functions registered with `#[benchmark]`
- host-side tests that call the SDK runner
- deterministic fixture output under `examples/fixtures/basic/summary.json`

## Try It

From the repository root:

```bash
cargo test -p basic-benchmark
cargo mobench list --crate-path examples/basic-benchmark
cargo mobench run \
  --crate-path examples/basic-benchmark \
  --target android \
  --function basic_benchmark::bench_fibonacci \
  --local-only
```

Use the `cargo mobench run` command as a host/local smoke test. Building and
running on actual Android or iOS devices still requires the platform toolchains
documented in `docs/guides/build.md`.
