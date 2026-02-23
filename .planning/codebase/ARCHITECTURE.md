# Architecture

**Analysis Date:** 2026-01-21

## Pattern Overview

**Overall:** Layered command-line SDK orchestrator with compile-time registration and FFI abstraction.

The mobench ecosystem follows a **three-crate architecture** with clear separation of concerns:
1. **CLI orchestration layer** (`mobench`) - Entry point for all operations
2. **SDK core** (`mobench-sdk`) - Core timing, building, and registry infrastructure
3. **Compile-time macro registration** (`mobench-macros`) - `#[benchmark]` attribute implementation

**Key Characteristics:**
- **Macro-based registration** - Functions marked with `#[benchmark]` auto-register via `inventory` crate at compile time
- **Feature-gated modularity** - `runner-only` feature for minimal mobile binaries vs full SDK with build automation
- **Embedded templates** - Android/iOS app templates compiled into the SDK using `include_dir!` macro
- **UniFFI FFI boundary** - Type-safe bindings generated automatically from Rust proc macros (not UDL)
- **Cross-platform building** - Automated native library builds for Android (NDK) and iOS (Xcode)

## Layers

**CLI Orchestrator (mobench):**
- Purpose: Entry point driving the full mobile benchmarking workflow
- Location: `crates/mobench/src/`
- Contains: Command handlers (init, build, run, list, fetch, package-ipa, package-xcuitest, compare)
- Depends on: `mobench-sdk` (builders, codegen, registry), BrowserStack REST API client
- Used by: End users via `cargo mobench` or `mobench` binary

**SDK Core (mobench-sdk):**
- Purpose: Timing harness, function registry, build automation, template generation
- Location: `crates/mobench-sdk/src/`
- Contains: Timing module, registry, runner, builders (Android/iOS), codegen, types
- Depends on: Standard library, serde, uniffi, include_dir, inventory
- Used by: CLI (`mobench`), user benchmarking projects, mobile apps

**Macro Registry (mobench-macros):**
- Purpose: Compile-time attribute macro for benchmark function registration
- Location: `crates/mobench-macros/src/`
- Contains: `#[benchmark]` attribute implementation
- Depends on: syn, quote, proc-macro2
- Used by: User projects and example code

**Timing Harness (mobench-sdk::timing):**
- Purpose: Minimal, portable benchmarking infrastructure for mobile targets
- Location: `crates/mobench-sdk/src/timing.rs`
- Contains: `run_closure()`, `BenchSpec`, `BenchSample`, `BenchReport`, nanosecond-precision timing
- Depends on: Standard library only (no platform-specific dependencies)
- Used by: All benchmark execution, available even with `runner-only` feature

**Build Automation (mobench-sdk::builders):**
- Purpose: Cross-compile Rust to mobile targets and package into native apps
- Location: `crates/mobench-sdk/src/builders/`
- Contains: `AndroidBuilder`, `IosBuilder`, common utilities
- Depends on: Rust toolchain, Android NDK, Xcode, `cargo-ndk`, `uniffi-bindgen`
- Used by: CLI `build` and `run` commands

**Template System (mobench-sdk::codegen):**
- Purpose: Generate mobile app projects from embedded templates
- Location: `crates/mobench-sdk/src/codegen.rs`, `crates/mobench-sdk/templates/`
- Contains: Template files (Android Gradle, iOS Xcode), parameterization
- Depends on: `include_dir!` macro for compile-time embedding
- Used by: `init` command to scaffold new projects

**Registry (mobench-sdk::registry):**
- Purpose: Runtime discovery of benchmark functions
- Location: `crates/mobench-sdk/src/registry.rs`
- Contains: `discover_benchmarks()`, `find_benchmark()`, `list_benchmark_names()`
- Depends on: `inventory` crate for global collection
- Used by: Runner, CLI list command

**Runner (mobench-sdk::runner):**
- Purpose: Benchmark execution engine linking registry to timing
- Location: `crates/mobench-sdk/src/runner.rs`
- Contains: `run_benchmark()`, `BenchmarkBuilder`
- Depends on: Registry, timing module
- Used by: CLI run command, mobile apps

## Data Flow

**User Project Setup:**
1. User adds `mobench-sdk` to `Cargo.toml` with optional build feature
2. User marks functions with `#[benchmark]` attribute
3. At compile time, macro registers functions via `inventory` crate
4. At runtime, registry discovers benchmarks when library loads

**Mobile Build Pipeline:**
1. User runs `cargo mobench build --target android` (CLI)
2. CLI instantiates `AndroidBuilder` with project path
3. Builder validates workspace (auto-detects crate location)
4. Builder compiles Rust to `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` targets via `cargo-ndk`
5. Builder generates UniFFI Kotlin bindings from Rust proc macro types
6. Builder syncs `.so` files into `android/app/src/main/jniLibs/{abi}/`
7. Builder runs `gradle assemble` to build APK and test APK
8. Builder outputs APK to `target/mobench/android/app/build/outputs/apk/`

**Benchmark Execution (Local):**
1. User runs `cargo mobench run --function my_benchmark --local-only`
2. CLI loads user project, builds benchmark binary
3. Registry discovers all `#[benchmark]` functions via `inventory`
4. Runner finds function by name, invokes it via registered closure
5. Timing harness measures iterations + warmup, records nanosecond samples
6. Report generated with statistics (mean, std dev, quantiles)

**Benchmark Execution (BrowserStack):**
1. User runs `cargo mobench run --target android --function my_benchmark --devices "Pixel 7-13.0"`
2. CLI builds APK with `release` profile (smaller upload)
3. CLI uploads APK + test APK to BrowserStack App Automate
4. Mobile app reads `bench_spec.json` from assets, calls `run_benchmark()` via FFI
5. App measures times via SDK's timing harness, returns JSON results
6. BrowserStack returns session artifacts to CLI
7. Results parsed and report generated

**State Management:**
- **Compile-time:** Benchmark function metadata embedded via `inventory` collect
- **Runtime:** Registry maintains in-memory collection of function pointers
- **Build artifacts:** Generated templates written to `target/mobench/`, source commits optional
- **Results:** JSON reports with samples, metadata (device, SDK version, timestamp)

## Key Abstractions

**BenchSpec:**
- Purpose: Declarative benchmark configuration (name, iterations, warmup)
- Examples: `crates/mobench-sdk/src/timing.rs`, `crates/sample-fns/src/lib.rs:8-13`
- Pattern: Serializable struct passed through entire pipeline (user → CLI → mobile app → timing harness)

**BenchFunction:**
- Purpose: Runtime-discoverable benchmark function pointer
- Examples: `crates/mobench-sdk/src/registry.rs:12-21`
- Pattern: Generated by `#[benchmark]` macro, collected via `inventory`, invoked by runner

**Builder Pattern:**
- Purpose: Fluent configuration for cross-compilation workflows
- Examples: `crates/mobench-sdk/src/builders/android.rs:66-110`, `crates/mobench-sdk/src/runner.rs:68-100`
- Pattern: Stateful builder accumulating configuration, `.build()` or `.run()` executes full pipeline

**FFI Boundary (UniFFI Proc Macros):**
- Purpose: Type-safe mobile binding generation from Rust annotations
- Examples: `crates/sample-fns/src/lib.rs:8, 16, 22, 29, 43, 93`
- Pattern: `#[derive(uniffi::Record)]` on types, `#[uniffi::export]` on functions, `uniffi::setup_scaffolding!()` generates bindings

**Error Propagation:**
- Purpose: Layered error handling from timing harness through CLI
- Examples: `crates/mobench-sdk/src/types.rs:50-100`, wraps `TimingError` as `BenchError`
- Pattern: Result types at all boundaries, detailed error context preserved

## Entry Points

**CLI Binary:**
- Location: `crates/mobench/src/main.rs`
- Triggers: User invokes `cargo mobench` or `mobench` binary
- Responsibilities: Parse CLI args, delegate to subcommands (init, build, run, list, fetch, etc.)

**SDK Init Command:**
- Location: `crates/mobench/src/lib.rs` (Command::InitSdk handler)
- Triggers: `cargo mobench init --target android --project-name my-project`
- Responsibilities: Generate Android/iOS projects, create config files, scaffold example benchmarks

**SDK Build Command:**
- Location: `crates/mobench/src/lib.rs` (Command::Build handler)
- Triggers: `cargo mobench build --target android`
- Responsibilities: Instantiate builder, validate workspace, cross-compile, package APK/xcframework

**SDK Run Command:**
- Location: `crates/mobench/src/lib.rs` (Command::Run handler)
- Triggers: `cargo mobench run --target android --function my_benchmark`
- Responsibilities: Build artifacts (if needed), upload to BrowserStack (if --devices), collect results

**Registry Discovery:**
- Location: `crates/mobench-sdk/src/registry.rs:41-43`
- Triggers: Binary loads, user calls `discover_benchmarks()`
- Responsibilities: Iterate `inventory` collection, return all registered functions

**Timing Harness (Mobile):**
- Location: `crates/mobench-sdk/src/timing.rs:run_closure()`
- Triggers: Mobile app calls `run_benchmark()` via UniFFI FFI
- Responsibilities: Execute closure with warmup, record nanosecond samples, return report

## Error Handling

**Strategy:** Layered result types with context preservation.

**Patterns:**
- `mobench-sdk` defines `BenchError` enum covering all operation categories (Runner, UnknownFunction, Execution, Io, Serialization, Config, Build)
- CLI uses `anyhow::Result` with `.context()` for actionable error messages
- Builders return `Result<BuildResult, BenchError>` with detailed build failure diagnostics
- Timing harness returns `TimingError` (minimal: NoIterations, Execution)
- Mobile apps receive `BenchError` via UniFFI and propagate to caller

**Handling patterns:**
```rust
// SDK error wrapping
match run_benchmark(spec) {
    Ok(report) => { ... },
    Err(BenchError::UnknownFunction(name)) => eprintln!("not found: {}", name),
    Err(BenchError::Runner(e)) => eprintln!("timing error: {}", e),
    Err(e) => eprintln!("error: {}", e),
}

// CLI error context
builder.build(&config)
    .context("failed to build Android APK")?;
```

## Cross-Cutting Concerns

**Logging:** Controlled via `--verbose` / `-v` flag in CLI. Builders and commands print progress only when enabled. No structured logging framework; simple stderr output.

**Validation:**
- Project workspace validation (Cargo.toml, crate location) in builders
- `BenchSpec` validation (iterations > 0) in timing harness
- Config file validation (required fields) in config module

**Authentication:** BrowserStack credentials resolved from:
1. Config file (supports `${ENV_VAR}` expansion)
2. Environment variables (`BROWSERSTACK_USERNAME`, `BROWSERSTACK_ACCESS_KEY`)
3. `.env.local` file (loaded via `dotenvy`)

**Feature Flags:**
- `full` (default): Builders, codegen, registry, runner enabled
- `runner-only`: Timing module only, no build automation or registry

---

*Architecture analysis: 2026-01-21*
