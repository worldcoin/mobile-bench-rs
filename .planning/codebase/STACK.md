# Technology Stack

**Analysis Date:** 2026-01-21

## Languages

**Primary:**
- Rust 2024 edition - Core SDK, CLI tool, and proc macros. Published as `mobench`, `mobench-sdk`, `mobench-macros` on crates.io.

**Secondary:**
- Rust 2021 edition - Example crates (`sample-fns`, `ffi-benchmark`) required for UniFFI-generated binding compatibility (UniFFI v0.28 targets 2021 edition)
- Kotlin - Auto-generated UniFFI FFI bindings for Android apps (via `uniffi-bindgen`)
- Swift - Auto-generated UniFFI FFI bindings for iOS apps (via `uniffi-bindgen`)

## Runtime

**Environment:**
- Rust toolchain (stable)
  - Android targets: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`
  - iOS targets: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (simulator on M1+ Macs)

**Package Manager:**
- Cargo (Rust package manager)
- Lockfile: `Cargo.lock` (present)
- Workspace resolver: v2

## Frameworks

**Core Benchmarking:**
- `mobench-sdk` - Core SDK library with timing harness (`timing` module), registry system, and build automation
- `mobench-macros` - Proc macro crate providing `#[benchmark]` attribute for function registration
- `mobench` - CLI tool for orchestrating builds, packaging, and execution

**Code Generation & FFI:**
- UniFFI v0.28 - Foreign function interface (FFI) generation using proc macros (no UDL files)
  - Feature: `build` for build script support
  - Feature: `cli` for binding generation CLI
  - Generates Kotlin and Swift bindings automatically from Rust code
- `inventory` v0.3 - Runtime function discovery via registration macros (used by `#[benchmark]`)
- `include_dir` v0.7 - Embed mobile app templates at compile time (no runtime file I/O)

**CLI & Configuration:**
- `clap` v4 - Command-line argument parsing with `derive` feature
- `serde` + `serde_json` v1 - Serialization framework
- `serde_yaml` v0.9 - YAML parsing for device matrices
- `toml` v0.8 - TOML parsing for config files
- `dotenvy` v0.15 - Environment variable loading from `.env.local` files

**Error Handling:**
- `thiserror` v1 - Derive macro for error types
- `anyhow` v1 - Error context and ergonomic error handling

**HTTP & Networking:**
- `reqwest` v0.12 (blocking client) - HTTP client for BrowserStack API
  - Features: `rustls-tls` (TLS via rustls, no OpenSSL), `blocking`, `json`, `multipart` (form-based uploads)
  - Used for: app/test suite uploads, build scheduling, result fetching

**Timing & Dates:**
- `time` v0.3 - Nanosecond-precision timing and RFC3339 timestamp formatting

**Proc Macro Dependencies:**
- `syn` v2 - Full featured Rust AST parsing (required for `#[benchmark]` macro implementation)
- `quote` v1 - Rust code generation
- `proc-macro2` v1 - Procedural macro utilities

## Key Dependencies

**Critical:**
- `uniffi` v0.28 - FFI binding generation. Without it, mobile apps cannot call Rust code. Breaking changes in UniFFI versions require code updates.
- `inventory` v0.3 - Runtime function registry. The `#[benchmark]` macro uses `inventory::collect!()` to auto-register benchmarks at compile time.
- `reqwest` v0.12 (blocking) - BrowserStack API communication. All device upload, scheduling, and result fetching depends on this.

**Infrastructure:**
- `dotenvy` v0.15 - Supports `.env.local` for credential management (BrowserStack username/access key)
- `include_dir` v0.7 - Android and iOS app templates embedded in the binary. No runtime file I/O needed.
- `time` v0.3 - Precise timing measurement (nanosecond granularity) and RFC3339 formatting for results

## Configuration

**Environment:**
- Credentials resolved in order:
  1. Config file (supports `${ENV_VAR}` expansion)
  2. Environment variables: `BROWSERSTACK_USERNAME`, `BROWSERSTACK_ACCESS_KEY`, `BROWSERSTACK_PROJECT`
  3. `.env.local` file (loaded automatically via `dotenvy`)
  4. Android NDK: `ANDROID_NDK_HOME` environment variable

**Build:**
- `Cargo.toml` - Workspace manifest at `/Users/dcbuilder/Code/world/mobile-bench-rs/Cargo.toml`
- `bench-config.toml` - User-generated project configuration (via `cargo mobench init`)
- `mobench.toml` - Optional CLI configuration in project root
- Mobile app projects created in `target/mobench/` (default, customizable with `--output-dir`)

**Features:**
- `mobench-sdk` feature flags:
  - `full` (default) - Complete SDK with build automation, templates, and registry
  - `runner-only` - Minimal timing harness for mobile binaries (low binary size footprint)

## Platform Requirements

**Development:**
- Rust stable toolchain
- `cargo-ndk` (for Android cross-compilation)
- Android SDK (API level 34)
- Android NDK (v26.1.10909125 or compatible)
- For iOS: Xcode toolchain with Swift support

**Production (BrowserStack):**
- No local platform setup required
- Artifacts uploaded to BrowserStack App Automate for execution on real devices
- Espresso framework for Android test automation
- XCUITest framework for iOS test automation

---

*Stack analysis: 2026-01-21*
