# mobench-sdk Integration Guide

This guide shows how to integrate `mobench-sdk` into an existing Rust project, run local
mobile benchmarks, and then run them on BrowserStack.

> **Important**: This guide is for integrators importing `mobench-sdk` as a library.
> All build functionality is available via `cargo mobench` commands.

## Quick Setup Checklist

Before diving into the full guide, ensure your project meets these requirements:

### Required Cargo.toml entries

```toml
[dependencies]
mobench-sdk = "0.1"
inventory = "0.3"  # Required for benchmark registration

[lib]
# Required for mobile FFI - produces .so (Android) and .a (iOS)
crate-type = ["cdylib", "staticlib", "lib"]
```

### Benchmark Function Requirements

**Simple benchmarks** (no setup attribute) must:
- Take **no parameters**
- Return **()** (unit type) - use `std::hint::black_box()` for results
- Be **public** (`pub fn`)

```rust
use mobench_sdk::benchmark;

// CORRECT - no params, returns ()
#[benchmark]
pub fn my_benchmark() {
    let input = create_input();  // Setup inside (gets measured)
    let result = compute(input);
    std::hint::black_box(result);  // Consume result
}

// WRONG - has parameters without setup (compile error)
#[benchmark]
pub fn bad_benchmark(data: &[u8]) { ... }

// WRONG - returns a value (compile error)
#[benchmark]
pub fn bad_benchmark() -> u64 { 42 }
```

**Benchmarks with setup** must:
- Take **one parameter** matching the setup function's return type
- Return **()** (unit type)
- Be **public** (`pub fn`)

```rust
// Setup function returns the input type
fn create_test_data() -> Vec<u8> {
    vec![0u8; 1_000_000]
}

// CORRECT - parameter type matches setup return type
#[benchmark(setup = create_test_data)]
pub fn process_data(data: &Vec<u8>) {
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

### Verify Your Setup

After adding benchmarks, verify everything is working:

```bash
# Check prerequisites are installed
cargo mobench check --target android

# List discovered benchmarks
cargo mobench list

# Verify registry, spec, and artifacts
cargo mobench verify --smoke-test --function my_crate::my_benchmark

# (Optional) Validate BrowserStack device specs before running
cargo mobench devices --validate "Google Pixel 7-13.0"
```

## 1) Prerequisites

Install the following tools (per platform):

- Rust toolchain (stable) + `rustup`:
  - https://www.rust-lang.org/tools/install
- Android:
  - Android Studio (SDK + NDK manager): https://developer.android.com/studio
  - Android NDK (API 24+): https://developer.android.com/ndk/downloads
  - `cargo-ndk` (`cargo install cargo-ndk`): https://github.com/bbqsrc/cargo-ndk
  - JDK 17+ (for Gradle; any distribution): https://openjdk.org/install/
    - Note: Android Gradle Plugin (AGP) officially supports Java 17.
- iOS (macOS only):
  - Xcode + Command Line Tools: https://developer.apple.com/xcode/
  - Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
    - https://doc.rust-lang.org/rustup/targets.html
  - `xcodegen` (optional): https://github.com/yonaskolb/XcodeGen

## 2) Add mobench-sdk to your crate

In your project's `Cargo.toml`:

```toml
[dependencies]
mobench-sdk = "0.1"
```

## 3) Annotate benchmark functions

Add `#[mobench_sdk::benchmark]` to any function you want to run on devices.

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn checksum_bench() {
    let data = [1u8; 1024];
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

### Macro Validation

The `#[benchmark]` macro validates function signatures at compile time:

**No parameters allowed:**
```rust
// ERROR: #[benchmark] functions must take no parameters.
// Found 1 parameter(s): data: &[u8]
#[benchmark]
pub fn bad_benchmark(data: &[u8]) { ... }
```

**Must return unit type:**
```rust
// ERROR: #[benchmark] functions must return () (unit type).
// Found return type: u64
#[benchmark]
pub fn bad_benchmark() -> u64 { 42 }
```

The compiler provides helpful suggestions for fixing these issues.

### Debugging Registration Issues

If benchmarks aren't being discovered, use the `debug_benchmarks!()` macro:

```rust
use mobench_sdk::{benchmark, debug_benchmarks};

#[benchmark]
pub fn my_benchmark() {
    std::hint::black_box(42);
}

// Generate the debug function
debug_benchmarks!();

fn main() {
    // Print all registered benchmarks
    _debug_print_benchmarks();
    // Output:
    // Discovered benchmarks:
    //   - my_crate::my_benchmark
}
```

If no benchmarks are printed, the macro provides troubleshooting tips:
1. Ensure functions are annotated with `#[benchmark]`
2. Ensure functions are `pub` (public visibility)
3. Ensure the crate with benchmarks is linked into the binary
4. Check that `inventory` crate is in your dependencies

Benchmarks are identified by name at runtime. You can call them by:

- Fully-qualified path (e.g., `my_crate::checksum_bench`)
- Or suffix match (e.g., `checksum_bench`)

## Setup and Teardown

When benchmarking functions that require expensive initialization, you want to exclude the setup time from your measurements. The `#[benchmark]` macro supports `setup`, `teardown`, and `per_iteration` attributes for this purpose.

### The Problem: Expensive Setup Getting Measured

Without setup/teardown support, initialization is included in timing:

```rust
#[benchmark]
pub fn verify_proof() {
    // This 5-second proof generation is measured (bad!)
    let proof = generate_complex_proof();

    // This 10ms verification is what we actually want to measure
    verify(&proof);
}
```

### Solution 1: One-Time Setup (Default)

Use the `setup` attribute to run initialization once before all iterations:

```rust
// Setup runs once, returns data passed to benchmark
fn setup_proof() -> ProofInput {
    generate_complex_proof()  // Takes 5 seconds, NOT measured
}

#[benchmark(setup = setup_proof)]
pub fn verify_proof(input: &ProofInput) {
    // Only this is measured - same input reused for all iterations
    verify(&input.proof);
}
```

**How it works:**
1. `setup_proof()` is called once before timing starts
2. The returned `ProofInput` is passed by reference to each iteration
3. All iterations share the same setup data
4. Setup time is excluded from measurements

### Solution 2: Per-Iteration Setup

For benchmarks that mutate their input, use `per_iteration` to get fresh data each iteration:

```rust
fn generate_random_vec() -> Vec<i32> {
    (0..1000).map(|_| rand::random()).collect()
}

#[benchmark(setup = generate_random_vec, per_iteration)]
pub fn sort_benchmark(data: Vec<i32>) {
    let mut data = data;  // Takes ownership
    data.sort();          // Mutates data - needs fresh input each time
}
```

**How it works:**
1. `generate_random_vec()` is called before EACH iteration
2. Data is passed by value (ownership transfer)
3. Each iteration gets fresh, unmutated data
4. Setup time for each iteration is excluded from measurements

### Solution 3: Setup with Teardown

For resources that need cleanup (database connections, temporary files, etc.):

```rust
fn setup_db() -> Database {
    Database::connect("test.db")
}

fn cleanup_db(db: Database) {
    db.close();
    std::fs::remove_file("test.db").ok();
}

#[benchmark(setup = setup_db, teardown = cleanup_db)]
pub fn db_query(db: &Database) {
    db.query("SELECT * FROM users");
}
```

**How it works:**
1. `setup_db()` is called once before timing starts
2. Database reference is passed to each iteration
3. After all iterations complete, `cleanup_db()` receives ownership
4. Both setup and teardown are excluded from measurements

### Combining Per-Iteration with Teardown

```rust
fn create_temp_file() -> TempFile {
    TempFile::new("test_data.bin")
}

fn delete_temp_file(file: TempFile) {
    file.delete();
}

#[benchmark(setup = create_temp_file, teardown = delete_temp_file)]
pub fn write_benchmark(file: &TempFile) {
    file.write_all(&[0u8; 1024]);
}
```

### Pattern Selection Guide

| Pattern | When to Use | Setup Timing | Data Sharing |
|---------|-------------|--------------|--------------|
| `#[benchmark]` | Simple benchmarks, fast inline setup | N/A | N/A |
| `#[benchmark(setup = fn)]` | Expensive setup, read-only benchmark | Once | Shared reference |
| `#[benchmark(setup = fn, per_iteration)]` | Benchmarks that mutate input | Per iteration | Owned value |
| `#[benchmark(setup = fn, teardown = fn)]` | Resources needing cleanup | Once | Shared reference |

Note: `per_iteration` and `teardown` cannot be combined, as `per_iteration` mode takes ownership of
the setup value, making cleanup via teardown semantically problematic.

### Complete Example

```rust
use mobench_sdk::benchmark;

// Simple benchmark - setup is fast enough to include
#[benchmark]
pub fn fibonacci() {
    let n = 30;
    let result = fib(n);
    std::hint::black_box(result);
}

// Expensive one-time setup
fn load_model() -> Model {
    Model::load_from_disk("large_model.bin")  // 10 seconds
}

#[benchmark(setup = load_model)]
pub fn inference(model: &Model) {
    let output = model.predict(&[1.0, 2.0, 3.0]);
    std::hint::black_box(output);
}

// Per-iteration setup for mutable operations
fn create_shuffled_vec() -> Vec<i32> {
    let mut v: Vec<i32> = (0..10000).collect();
    v.shuffle(&mut rand::thread_rng());
    v
}

#[benchmark(setup = create_shuffled_vec, per_iteration)]
pub fn quicksort(mut data: Vec<i32>) {
    data.sort_unstable();
    std::hint::black_box(data);
}

// Setup + teardown for resource management
fn open_connection() -> DbConnection {
    DbConnection::connect("postgres://localhost/bench")
}

fn close_connection(conn: DbConnection) {
    conn.execute("DROP TABLE IF EXISTS bench_temp");
    conn.close();
}

#[benchmark(setup = open_connection, teardown = close_connection)]
pub fn db_insert(conn: &DbConnection) {
    conn.execute("INSERT INTO bench_temp VALUES (1, 'test')");
}
```

## 4) Scaffold mobile projects

From your repo root, create a mobile harness with the CLI:

```bash
cargo mobench init --target android --output bench-config.toml
```

This generates:

- `bench-mobile/` (FFI bridge that links your crate)
- `android/` and `ios/` app templates
- `bench-config.toml` configuration

## 5) Local Android testing

Build the Android app using the mobench:

```bash
cargo mobench build --target android
```

This automatically:

- Builds Rust libraries for all Android ABIs (arm64-v8a, armeabi-v7a, x86_64)
- Generates UniFFI Kotlin bindings
- Copies .so files to jniLibs
- Runs Gradle to create the APK

Install and run on emulator or device:

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.world.bench/.MainActivity
```

To override benchmark parameters:

```bash
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function my_crate::checksum_bench \
  --ei bench_iterations 30 \
  --ei bench_warmup 5
```

## 6) Local iOS testing

Build the iOS xcframework using the mobench:

```bash
cargo mobench build --target ios
```

This automatically:

- Builds Rust libraries for iOS device + simulator
- Generates UniFFI Swift bindings and C headers
- Creates properly structured xcframework
- Code-signs the framework
- Generates Xcode project (if xcodegen is installed)

Open and run in Xcode:

```bash
open ios/BenchRunner/BenchRunner.xcodeproj
```

The app will read `bench_spec.json` from the bundle or use defaults.

## 7) BrowserStack (Android Espresso)

Build APK + test APK:

```bash
cargo mobench build --target android
```

The CLI automatically builds both the app APK and the test APK (androidTest) required for BrowserStack Espresso testing.

Run on BrowserStack:

```bash
cargo mobench run \
  --target android \
  --function my_crate::checksum_bench \
  --iterations 100 \
  --warmup 10 \
  --devices "Google Pixel 7-13.0" \
  --release
```

**Important**: Always use the `--release` flag for BrowserStack runs. Debug builds are significantly larger (~544MB vs ~133MB for release) and may cause upload timeouts.

The CLI will automatically:

- Build in release mode (with `--release` flag)
- Upload APK and test APK to BrowserStack
- Schedule the test run
- Wait for completion
- Download results and logs

## 8) BrowserStack (iOS XCUITest)

Build iOS artifacts and package for BrowserStack:

```bash
# Build xcframework
cargo mobench build --target ios

# Package as IPA (ad-hoc signing, no Apple ID needed)
cargo mobench package-ipa --method adhoc

# Package the XCUITest runner for BrowserStack
cargo mobench package-xcuitest

# Or for development signing (requires Apple Developer account)
cargo mobench package-ipa --method development
```

Run on BrowserStack:

```bash
cargo mobench run \
  --target ios \
  --function my_crate::checksum_bench \
  --iterations 100 \
  --warmup 10 \
  --devices "iPhone 14-16" \
  --release \
  --ios-app target/mobench/ios/BenchRunner.ipa \
  --ios-test-suite target/mobench/ios/BenchRunnerUITests.zip
```

**Important**: Always use the `--release` flag for BrowserStack runs to reduce artifact sizes and prevent upload timeouts.

**iOS Packaging Commands:**

- `package-ipa`: Creates the app IPA bundle for device deployment
  - `--method adhoc`: No Apple ID required, works for BrowserStack
  - `--method development`: Requires Apple Developer account
- `package-xcuitest`: Creates the XCUITest runner zip that BrowserStack uses to drive test automation. Outputs to `target/mobench/ios/BenchRunnerUITests.zip`

## 9) Device Selection Guide

When benchmarking on BrowserStack, choosing appropriate devices helps ensure your code performs well across the spectrum of real-world hardware. Below are recommended devices for each performance tier.

### Android Device Tiers

| Tier | Example Device | BrowserStack Spec | Use Case |
|------|----------------|-------------------|----------|
| **Flagship** | Samsung Galaxy S24 Ultra | `"Samsung Galaxy S24 Ultra-14.0"` | Best-case performance, latest hardware |
| **High** | Google Pixel 8 | `"Google Pixel 8-14.0"` | Modern high-end, clean Android |
| **Medium-High** | Samsung Galaxy A54 | `"Samsung Galaxy A54-13.0"` | Popular mid-range, good baseline |
| **Medium** | Samsung Galaxy A33 | `"Samsung Galaxy A33 5G-13.0"` | Budget-conscious users |
| **Low** | Samsung Galaxy A13 | `"Samsung Galaxy A13-12.0"` | Entry-level smartphones |
| **Lowest** | Samsung Galaxy A03s | `"Samsung Galaxy A03s-12.0"` | Worst-case performance testing |

### iOS Device Tiers

| Tier | Example Device | BrowserStack Spec | Use Case |
|------|----------------|-------------------|----------|
| **Flagship** | iPhone 15 Pro Max | `"iPhone 15 Pro Max-17"` | Best-case performance, A17 Pro chip |
| **High** | iPhone 14 Pro | `"iPhone 14 Pro-17"` | Previous flagship, still powerful |
| **Medium-High** | iPhone 13 | `"iPhone 13-17"` | Mainstream device, A15 chip |
| **Medium** | iPhone 12 | `"iPhone 12-17"` | Older but still common |
| **Low** | iPhone SE 2022 | `"iPhone SE 2022-17"` | Budget iPhone, A15 chip |
| **Lowest** | iPhone 11 | `"iPhone 11-17"` | Oldest commonly supported |

### Multi-Device Benchmarking

Run benchmarks across multiple tiers to understand performance distribution:

```bash
# Android: Test across performance spectrum
cargo mobench run \
  --target android \
  --function my_crate::my_benchmark \
  --iterations 50 \
  --warmup 5 \
  --devices "Samsung Galaxy S24 Ultra-14.0" "Samsung Galaxy A54-13.0" "Samsung Galaxy A13-12.0" \
  --release

# iOS: Test across performance spectrum
cargo mobench run \
  --target ios \
  --function my_crate::my_benchmark \
  --iterations 50 \
  --warmup 5 \
  --devices "iPhone 15 Pro Max-17" "iPhone 13-17" "iPhone 11-17" \
  --release \
  --ios-app target/mobench/ios/BenchRunner.ipa \
  --ios-test-suite target/mobench/ios/BenchRunnerUITests.zip
```

### Validate Device Availability

Before running, verify your device specs are valid:

```bash
# Validate specific devices
cargo mobench devices --validate "Samsung Galaxy S24 Ultra-14.0" "iPhone 15 Pro Max-17"

# List all available Android devices
cargo mobench devices --platform android

# List all available iOS devices
cargo mobench devices --platform ios
```

**Tip**: Device availability on BrowserStack changes over time. Use `cargo mobench devices` to see currently available devices.

## 10) Verification and Troubleshooting

### Check Prerequisites

Before building, verify all required tools are installed:

```bash
# Check Android prerequisites
cargo mobench check --target android

# Check iOS prerequisites
cargo mobench check --target ios

# Output as JSON for CI
cargo mobench check --target android --format json
```

### Verify Benchmark Setup

Use the `verify` command to validate your setup:

```bash
# Full verification with smoke test
cargo mobench verify --target android --check-artifacts --smoke-test --function my_crate::my_benchmark

# Check specific spec file
cargo mobench verify --spec-path target/mobench/android/app/src/main/assets/bench_spec.json
```

The verify command checks:
1. Benchmark registry has functions registered
2. Spec file exists and is valid
3. Build artifacts are present
4. Optional smoke test passes

### View Result Summaries

After running benchmarks, get statistics with the `summary` command:

```bash
# Text summary (default)
cargo mobench summary results.json

# JSON format
cargo mobench summary results.json --format json

# CSV format
cargo mobench summary results.json --format csv
```

### Common Errors and Solutions

**"unknown benchmark function":**
```
Error: unknown benchmark function: 'my_func'. Available benchmarks: ["other_func"]

Ensure the function is:
  1. Annotated with #[benchmark]
  2. Public (pub fn)
  3. Takes no parameters and returns ()
```

**"iterations must be greater than zero":**
```
Error: iterations must be greater than zero (got 0). Minimum recommended: 10
```

**Benchmark not discovered:**
- Use `debug_benchmarks!()` macro to debug
- Verify function is `pub` and annotated with `#[benchmark]`
- Ensure `inventory` crate is a dependency

## Notes

- **No scripts needed**: All functionality is available via `cargo mobench` commands
- **Use `--release` for BrowserStack**: Debug builds are ~544MB, release builds are ~133MB. Large artifacts can cause upload timeouts.
- **Validate before running**: Use `cargo mobench verify` to catch issues early
- If you change FFI types, the build process automatically regenerates bindings
- Android emulator ABI is typically `x86_64` in Android Studio
- BrowserStack credentials must be set via `BROWSERSTACK_USERNAME` and `BROWSERSTACK_ACCESS_KEY`
- For repository development, use the same `cargo mobench` workflow
