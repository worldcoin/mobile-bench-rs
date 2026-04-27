# FFI Benchmark Example

This example demonstrates the larger integration shape used by benchmark crates
that need explicit UniFFI bindings and custom FFI-facing types. Use it after the
basic example when validating mobile runner behavior or downstream projects with
their own FFI surface.

## What It Shows

- UniFFI build dependency setup
- SDK benchmark registration and registry execution
- FFI-safe benchmark request and error handling patterns
- host-side tests for invalid input, unknown benchmark names, and successful
  benchmark execution
- deterministic fixture output under `examples/fixtures/ffi/summary.json`

## Try It

From the repository root:

```bash
cargo test -p ffi-benchmark
cargo mobench list --crate-path examples/ffi-benchmark
cargo mobench run --target android --function ffi_benchmark::fibonacci --local-only
```

Use this example as the starting point for crates that need custom data types
crossing the Kotlin or Swift boundary.
