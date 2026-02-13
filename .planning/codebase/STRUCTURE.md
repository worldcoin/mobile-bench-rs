# Codebase Structure

**Analysis Date:** 2026-01-21

## Directory Layout

```
mobile-bench-rs/
├── crates/                          # Cargo workspace members
│   ├── mobench/                     # CLI tool (published to crates.io)
│   │   ├── src/
│   │   │   ├── main.rs              # CLI entry point
│   │   │   ├── lib.rs               # Command handlers and orchestration
│   │   │   ├── config.rs            # TOML config parsing
│   │   │   ├── browserstack.rs      # BrowserStack REST API client
│   │   │   └── bin/cargo-mobench.rs # cargo-mobench subcommand wrapper
│   │   └── Cargo.toml
│   │
│   ├── mobench-sdk/                 # Core SDK library (published)
│   │   ├── src/
│   │   │   ├── lib.rs               # Public API surface (timing + full feature)
│   │   │   ├── timing.rs            # Lightweight timing harness (always available)
│   │   │   ├── types.rs             # Error types, BuildConfig, InitConfig
│   │   │   ├── registry.rs          # Benchmark function discovery (inventory-based)
│   │   │   ├── runner.rs            # Benchmark execution engine
│   │   │   ├── codegen.rs           # Template generation from embedded files
│   │   │   └── builders/
│   │   │       ├── mod.rs           # Builders module exports
│   │   │       ├── android.rs       # AndroidBuilder (cargo-ndk, Gradle)
│   │   │       ├── ios.rs           # IosBuilder (xcodebuild, xcframework)
│   │   │       └── common.rs        # Shared builder utilities
│   │   ├── templates/               # Embedded mobile app templates
│   │   │   ├── android/             # Android Gradle project scaffold
│   │   │   └── ios/                 # iOS Xcode project scaffold
│   │   └── Cargo.toml
│   │
│   ├── mobench-macros/              # Proc macro crate (published)
│   │   ├── src/lib.rs               # #[benchmark] attribute macro
│   │   └── Cargo.toml
│   │
│   ├── sample-fns/                  # Sample functions for testing (not published)
│   │   ├── src/lib.rs               # UniFFI FFI types and run_benchmark()
│   │   └── Cargo.toml
│   │
├── examples/
│   ├── basic-benchmark/             # Minimal #[benchmark] usage example
│   │   ├── src/lib.rs               # Two bench_* functions, tests
│   │   └── Cargo.toml
│   │
│   └── ffi-benchmark/               # Full UniFFI example (see sample-fns)
│       ├── src/lib.rs
│       └── Cargo.toml
│
├── android/                         # Development Android app (not auto-generated)
│   ├── app/                         # App module
│   │   ├── src/main/
│   │   │   ├── java/dev/world/bench/MainActivity.kt   # Entry point
│   │   │   ├── java/uniffi/sample_fns/sample_fns.kt   # Generated Kotlin bindings
│   │   │   ├── assets/bench_spec.json                 # Benchmark parameters
│   │   │   └── jniLibs/{abi}/                         # Native .so libraries
│   │   ├── build.gradle                # Gradle configuration
│   │   └── src/androidTest/           # Espresso tests for BrowserStack
│   ├── build.gradle
│   ├── settings.gradle
│   └── gradle/wrapper/
│
├── ios/                             # Development iOS app (not auto-generated)
│   └── BenchRunner/
│       ├── BenchRunner/              # Xcode project source
│       │   ├── BenchRunnerFFI.swift  # FFI wrapper calling UniFFI bindings
│       │   ├── BenchRunner-Bridging-Header.h    # Objective-C bridging header
│       │   ├── Generated/            # Auto-generated UniFFI code
│       │   │   ├── sample_fns.swift  # Swift bindings from UniFFI
│       │   │   └── sample_fnsFFI.h   # C header from UniFFI
│       │   └── ...
│       ├── BenchRunnerUITests/       # XCUITest runner for BrowserStack
│       ├── BenchRunner.xcodeproj/    # Xcode project
│       └── project.yml               # XcodeGen specification
│
├── templates/                       # Source templates (symlinked to SDK)
│   ├── android/                     # Android project template source
│   └── ios/                         # iOS project template source
│
├── .github/workflows/
│   └── mobile-bench.yml             # CI/CD workflow
│
├── Cargo.toml                       # Workspace root
├── Cargo.lock
├── BUILD.md                         # Build reference
├── TESTING.md                       # Testing guide
├── CLAUDE.md                        # This project's guidelines for Claude
└── README.md
```

## Directory Purposes

**`crates/mobench/`** - CLI orchestrator
- Purpose: Entry point for all user operations (build, run, list, fetch, init)
- Contains: Command handlers, BrowserStack API integration, config parsing
- Key files: `src/lib.rs` (commands), `src/main.rs` (entry), `src/browserstack.rs` (API)

**`crates/mobench-sdk/`** - Core SDK library
- Purpose: Reusable SDK with timing harness, builders, registry, codegen
- Contains: Timing infrastructure, cross-platform builders, function discovery
- Key files: `src/lib.rs` (API), `src/timing.rs` (core), `src/registry.rs` (discovery)
- Sections:
  - `src/timing.rs` - Always available, used by mobile binaries with `runner-only` feature
  - `src/builders/` - Android/iOS build automation, requires full feature
  - `src/registry.rs` - Runtime function discovery via `inventory` crate
  - `src/codegen.rs` - Template parameterization and file generation
  - `templates/` - Embedded Android and iOS project templates

**`crates/mobench-macros/`** - Proc macro crate
- Purpose: Compile-time registration of benchmark functions
- Contains: `#[benchmark]` attribute implementation
- Key files: `src/lib.rs` (single file)

**`crates/sample-fns/`** - Sample benchmark functions
- Purpose: Reference implementation for UniFFI FFI usage
- Contains: `BenchSpec`, `BenchReport`, `run_benchmark()` with FFI types
- Used by: Repository's Android/iOS test apps
- Key distinction: Shows how to define FFI-compatible types with `#[derive(uniffi::Record)]`

**`examples/`** - Public examples
- Purpose: Demonstrate SDK usage patterns
- `basic-benchmark/` - Minimal SDK usage with `#[benchmark]`
- `ffi-benchmark/` - (Link to sample-fns implementation)

**`android/`, `ios/`** - Development test apps
- Purpose: Test mobile app integration (not auto-generated)
- Contains: Full Gradle/Xcode projects with BrowserStack integration
- Android: Espresso tests in `src/androidTest/`
- iOS: XCUITest runner in `BenchRunnerUITests/`
- These apps are what `cargo mobench build` scaffolds for users

**`templates/`** - Template sources
- Purpose: Source files compiled into SDK via `include_dir!`
- Structure mirrors `android/`, `ios/` directory layout

## Key File Locations

**Entry Points:**
- `crates/mobench/src/main.rs` - CLI binary entry point
- `crates/mobench/src/lib.rs` - Command orchestration and handlers
- `crates/mobench-sdk/src/lib.rs` - SDK public API surface
- `examples/basic-benchmark/src/lib.rs` - User project example

**Configuration:**
- `crates/mobench/src/config.rs` - TOML parsing for `bench-config.toml`
- `crates/mobench-sdk/src/types.rs` - `BuildConfig`, `InitConfig` definitions
- `android/app/build.gradle` - Android build configuration
- `ios/BenchRunner/project.yml` - XcodeGen specification

**Core Logic:**
- `crates/mobench-sdk/src/timing.rs` - Timing harness (nanosecond measurement)
- `crates/mobench-sdk/src/registry.rs` - Benchmark discovery via `inventory`
- `crates/mobench-sdk/src/runner.rs` - Execution engine linking registry to timing
- `crates/mobench-sdk/src/builders/android.rs` - NDK compilation, Gradle build
- `crates/mobench-sdk/src/builders/ios.rs` - Xcode compilation, xcframework creation
- `crates/mobench/src/browserstack.rs` - BrowserStack REST API client

**Testing:**
- `crates/mobench-sdk/src/lib.rs` - SDK unit tests
- `examples/basic-benchmark/src/lib.rs:66-99` - Integration tests
- `android/app/src/androidTest/` - Espresso tests (BrowserStack)
- `ios/BenchRunner/BenchRunnerUITests/` - XCUITest runner (BrowserStack)

**Mobile Integration:**
- `android/app/src/main/java/dev/world/bench/MainActivity.kt` - Android entry point
- `android/app/src/main/java/uniffi/sample_fns/sample_fns.kt` - Generated Kotlin bindings
- `ios/BenchRunner/BenchRunnerFFI.swift` - iOS entry point
- `ios/BenchRunner/Generated/sample_fns.swift` - Generated Swift bindings
- `crates/sample-fns/src/lib.rs` - UniFFI types and `run_benchmark()` export

## Naming Conventions

**Files:**
- Rust source files: `snake_case.rs` (e.g., `timing.rs`, `android.rs`, `main.rs`)
- Module files: `mod.rs` for re-exports, file per implementation
- Macro crates: Single file `src/lib.rs` (e.g., `mobench-macros/src/lib.rs`)
- Test files: Co-located with source as `#[cfg(test)]` mod at bottom

**Directories:**
- Crate directories: kebab-case (e.g., `mobench-sdk`, `mobench-macros`)
- Module directories: snake_case (e.g., `builders/`, `templates/`)
- Platform packages: lowercase (e.g., `android/`, `ios/`)

**Types:**
- Public types: PascalCase (e.g., `BenchSpec`, `BenchFunction`, `AndroidBuilder`)
- Error types: PascalCase variant names (e.g., `UnknownFunction`, `BuildError`)
- Enum variants: PascalCase (e.g., `Target::Android`, `BuildProfile::Release`)

**Functions:**
- Public functions: snake_case (e.g., `run_benchmark()`, `discover_benchmarks()`)
- Attribute macros: snake_case (e.g., `#[benchmark]`)
- Builder methods: snake_case (e.g., `.verbose()`, `.output_dir()`)

**Variables:**
- Local variables: snake_case (e.g., `builder`, `spec`, `output_dir`)
- Constants: SCREAMING_SNAKE_CASE (e.g., `CHECKSUM_INPUT`, `ANDROID_TEMPLATES`)
- Mutable state: Clear naming with `mut` keyword visible

## Where to Add New Code

**New Feature (e.g., new benchmarking strategy):**
- Primary code: Add to `crates/mobench-sdk/src/` as a new module
- Example: `crates/mobench-sdk/src/memory_profile.rs` for memory benchmarking
- Tests: `#[cfg(test)] mod tests { }` at bottom of module
- Public API: Re-export in `crates/mobench-sdk/src/lib.rs`

**New Builder or Platform Support:**
- Implementation: `crates/mobench-sdk/src/builders/{platform}.rs`
- Example: Adding WASM support → `crates/mobench-sdk/src/builders/wasm.rs`
- Module registration: Add to `crates/mobench-sdk/src/builders/mod.rs` with `pub mod wasm;`
- Common utilities: Extend `crates/mobench-sdk/src/builders/common.rs`

**New CLI Command:**
- Handler: Add variant to `Command` enum in `crates/mobench/src/lib.rs:150+`
- Implementation: Add to match statement in `run()` function
- Example: `Command::Analyze { ... }` → handler function for comparison logic

**Utilities and Helpers:**
- Shared across SDK modules: `crates/mobench-sdk/src/builders/common.rs`
- Shared across CLI: Add to `crates/mobench/src/lib.rs` or new module
- Shared across crates: Consider extracting to new shared crate

**Test Examples:**
- User-facing examples: `examples/` directory with `Cargo.toml` and `src/lib.rs`
- Internal integration tests: `crates/*/tests/` (create if needed) or co-located in source
- Unit tests: `#[cfg(test)] mod tests { }` at bottom of source files

## Special Directories

**`crates/mobench-sdk/templates/`:**
- Purpose: Source files embedded via `include_dir!` at compile time
- Generated: No, committed to git
- Committed: Yes, both `android/` and `ios/` templates
- Structure: Mirrors actual Android/iOS projects with placeholder variables
- Variables replaced during generation: `${PROJECT_NAME}`, `${PROJECT_SLUG}`, `${BUNDLE_PREFIX}`

**`target/mobench/`:**
- Purpose: Output directory for all build artifacts
- Generated: Yes, created by builders during `build` and `run` commands
- Committed: No, in `.gitignore`
- Contents:
  - `android/` - Generated Android project + APK
  - `ios/` - Generated iOS project + xcframework + IPA

**`.github/workflows/`:**
- Purpose: CI/CD pipeline definition
- File: `mobile-bench.yml` - Build and test automation

**`android/app/src/androidTest/`:**
- Purpose: Espresso test suite for BrowserStack execution
- Contains: JUnit tests using Espresso framework
- Execution: Runs as test APK on BrowserStack Espresso

**`ios/BenchRunner/BenchRunnerUITests/`:**
- Purpose: XCUITest runner for BrowserStack iOS
- Contains: Swift XCUITest classes
- Execution: Runs as XCUITest bundle on BrowserStack

## Import and Module Organization

**Workspace structure:**
- Root `Cargo.toml` defines members: `crates/mobench`, `crates/mobench-sdk`, `crates/mobench-macros`, `crates/sample-fns`, `examples/basic-benchmark`, `examples/ffi-benchmark`
- Workspace dependencies defined in `[workspace.dependencies]`

**Public API exports:**
- SDK: `crates/mobench-sdk/src/lib.rs` re-exports key types at crate root
- CLI: `crates/mobench/src/lib.rs` private, main CLI logic in `main.rs`
- Macros: `crates/mobench-macros/src/lib.rs` exports `#[benchmark]` macro

**Feature gating:**
- `full` (default): Builders, codegen, registry, runner
- `runner-only`: Timing module only, minimal mobile binary

---

*Structure analysis: 2026-01-21*
