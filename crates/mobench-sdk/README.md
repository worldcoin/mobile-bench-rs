# mobench-sdk

Mobile benchmarking SDK for Rust.

Transform your Rust project into a mobile benchmarking suite. The SDK provides the timing/runtime layer, registry, builders, and generated mobile runners used by the `mobench` CLI for local execution, BrowserStack benchmark runs, and local native profiling.

## Features

- **`#[benchmark]` macro**: Mark functions for mobile benchmarking
- **Automatic registry**: Compile-time function discovery
- **Built-in timing harness**: Lightweight timing infrastructure with warmup and iteration support
- **Mobile app generation**: Create Android/iOS apps from templates
- **Build automation**: Cross-compile and package for mobile platforms
- **Statistical analysis**: Mean, median, stddev, percentiles
- **Semantic profiling phases**: Annotate benchmark sub-steps with `profile_phase(...)`
- **BrowserStack benchmark integration**: Run timing benchmarks on real devices in the cloud
- **UniFFI bindings**: Automatic FFI generation for mobile platforms
- **Configuration file support**: `mobench.toml` for project settings
- **Config-first CLI integration**: the CLI resolves project root, crate name, and library name from flags, `mobench.toml`, workspace metadata, or git root

## Quick Start

Add mobench-sdk to your project:

```toml
[dependencies]
mobench-sdk = "0.1.42"
```

For benchmark crates that only need `#[benchmark]`, registry discovery, and
runtime execution, use the narrower registry feature:

```toml
[dependencies]
mobench-sdk = { version = "0.1.42", default-features = false, features = ["registry"] }
```

Mark functions to benchmark:

```rust
use mobench_sdk::benchmark;

#[benchmark]
fn fibonacci() {
    let result = fib(30);
    std::hint::black_box(result);
}

#[benchmark]
fn json_parsing() {
    let data = serde_json::from_str::<MyStruct>(JSON_DATA).unwrap();
    std::hint::black_box(data);
}

fn fib(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fib(n - 1) + fib(n - 2),
    }
}
```

Run programmatically:

```rust
use mobench_sdk::{run_benchmark, BenchSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = BenchSpec::new("fibonacci", 100, 10)?;
    let report = run_benchmark(spec)?;

    println!("Mean: {} ns", report.mean_ns());
    println!("Median: {} ns", report.median_ns());
    println!("Std dev: {} ns", report.std_dev_ns());

    Ok(())
}
```

## Project Setup

### 1. Initialize Mobile Benchmarking

Use the [mobench CLI](https://crates.io/crates/mobench) to scaffold your project:

```bash
cargo install mobench
cargo mobench init --target android  # or ios, or both
```

This creates:

- `bench-mobile/` - FFI wrapper crate
- `android/` or `ios/` - Mobile app projects
- `bench-config.toml` - Configuration file

The generated `bench-mobile/` crate is still the default scaffold, but the CLI can also target existing custom crate layouts through `mobench.toml`, `--project-root`, and `--crate-path`.

### 2. Add Benchmarks

```rust
use mobench_sdk::benchmark;

#[benchmark]
fn my_benchmark() {
    // Your code here
}
```

### 3. Build for Mobile

```bash
# Build to default output directory (target/mobench/)
cargo mobench build --target android

# Build a custom crate from repo root
cargo mobench build --target ios --project-root . --crate-path ./crates/zk-mobile-bench

# Or with verbose output
cargo mobench build --target android --verbose

# Or preview what would be built
cargo mobench build --target android --dry-run
```

### 4. Run on Devices

Local device workflow (builds artifacts and writes the run spec; launch the app manually):

```bash
cargo mobench run --target android --function my_benchmark
```

BrowserStack:

```bash
export BROWSERSTACK_USERNAME=your_username
export BROWSERSTACK_ACCESS_KEY=your_key

# Use --release for BrowserStack (smaller APK: ~133MB vs ~544MB debug)
cargo mobench run --target android --function my_benchmark \
  --devices "Google Pixel 7-13.0" --release
```

Local profiling:

```bash
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --crate-path ./crates/my-benchmarks \
  --function my_benchmark
```

When the benchmark emits semantic phases with `mobench_sdk::timing::profile_phase(...)`,
the CLI merges those phase timings into the profile manifest and summary next to the
native stack artifacts.

## Examples (Repository)

- `examples/basic-benchmark`: minimal SDK usage with `#[benchmark]`
- `examples/ffi-benchmark`: full UniFFI surface with `run_benchmark` and FFI types
- `crates/sample-fns`: repository demo library used by Android/iOS test apps

## API Documentation

### Core Functions

#### `run_benchmark`

Run a registered benchmark by name:

```rust
use mobench_sdk::{run_benchmark, BenchSpec};

let spec = BenchSpec::new("my_function", 50, 5)?;
let report = run_benchmark(spec)?;
```

#### `BenchmarkBuilder`

Fluent API for building and running benchmarks:

```rust
use mobench_sdk::BenchmarkBuilder;

let report = BenchmarkBuilder::new("my_function")
    .iterations(100)
    .warmup(10)
    .run()?;
```

### Types

#### `BenchSpec`

Benchmark specification:

```rust
pub struct BenchSpec {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
}
```

#### `RunnerReport`

Benchmark results with statistical analysis:

```rust
pub struct RunnerReport {
    pub spec: BenchSpec,
    pub samples: Vec<BenchSample>,
}

impl RunnerReport {
    pub fn mean_ns(&self) -> f64;
    pub fn median_ns(&self) -> u64;
    pub fn min_ns(&self) -> u64;
    pub fn max_ns(&self) -> u64;
    pub fn std_dev_ns(&self) -> f64;
    pub fn percentile(&self, p: f64) -> u64;
}
```

### Build API

#### Generate Mobile Projects

```rust
use mobench_sdk::{InitConfig, Target, generate_project};

let config = InitConfig {
    project_name: "my-benchmarks".to_string(),
    output_dir: PathBuf::from("./bench-output"),
    target: Target::Both,  // Android + iOS
    generate_examples: true,
};

let project_path = generate_project(&config)?;
```

#### Build for Android

```rust
use mobench_sdk::AndroidBuilder;

let builder = AndroidBuilder::new(PathBuf::from("."), "debug")?;
let apk = builder.build_apk()?;
println!("Built APK: {:?}", apk);
```

#### Build for iOS

```rust
use mobench_sdk::IosBuilder;

let builder = IosBuilder::new(PathBuf::from("."), "release")?;
let xcframework = builder.build_xcframework()?;
println!("Built xcframework: {:?}", xcframework);
```

## Examples

### Crypto Benchmarks

```rust
use mobench_sdk::benchmark;
use sha2::{Sha256, Digest};
use aes::Aes256;

#[benchmark]
fn sha256_1kb() {
    let data = vec![0u8; 1024];
    let hash = Sha256::digest(&data);
    std::hint::black_box(hash);
}

#[benchmark]
fn aes256_encrypt() {
    let key = [0u8; 32];
    let cipher = Aes256::new(&key.into());
    // ... encryption code
    std::hint::black_box(cipher);
}
```

### JSON Parsing Benchmarks

```rust
use mobench_sdk::benchmark;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct User {
    name: String,
    email: String,
    age: u32,
}

const JSON_DATA: &str = r#"{"name":"Alice","email":"alice@example.com","age":30}"#;

#[benchmark]
fn parse_json_small() {
    let user: User = serde_json::from_str(JSON_DATA).unwrap();
    std::hint::black_box(user);
}

#[benchmark]
fn serialize_json_small() {
    let user = User {
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        age: 30,
    };
    let json = serde_json::to_string(&user).unwrap();
    std::hint::black_box(json);
}
```

### Data Structure Benchmarks

```rust
use mobench_sdk::benchmark;
use std::collections::{HashMap, BTreeMap};

#[benchmark]
fn hashmap_insert_1000() {
    let mut map = HashMap::new();
    for i in 0..1000 {
        map.insert(i, i * 2);
    }
    std::hint::black_box(map);
}

#[benchmark]
fn btreemap_insert_1000() {
    let mut map = BTreeMap::new();
    for i in 0..1000 {
        map.insert(i, i * 2);
    }
    std::hint::black_box(map);
}
```

## Architecture

### Workflow

1. **Development**: Write benchmarks with `#[benchmark]`
2. **Compilation**: Benchmarks registered at compile time via `inventory`
3. **FFI Generation**: UniFFI creates type-safe Kotlin/Swift bindings
4. **Mobile Build**: Cross-compile to mobile platforms
5. **Execution**: Run on real devices or emulators
6. **Analysis**: Collect and analyze timing data

### Components

```
┌─────────────────────────────────────────┐
│ Your Rust Code + #[benchmark]           │
└──────────────┬──────────────────────────┘
               │
               ↓
┌─────────────────────────────────────────┐
│ mobench-sdk (Registry + Build Tools)    │
└──────────────┬──────────────────────────┘
               │
               ↓
┌─────────────────────────────────────────┐
│ UniFFI (FFI Bindings Generation)        │
└──────────────┬──────────────────────────┘
               │
       ┌───────┴───────┐
       ↓               ↓
┌─────────────┐ ┌───────────────────────┐
│ Android APK │ │ iOS xcframework / IPA │
└──────┬──────┘ └──────┬────────────────┘
       │               │
       └───────┬───────┘
               ↓
    ┌──────────────────────┐
    │ Real Mobile Devices  │
    │ (BrowserStack/Local) │
    └──────────────────────┘
```

## Configuration

### `mobench.toml` (Project Configuration)

mobench automatically loads `mobench.toml` from the current directory or parent directories:

```toml
[project]
crate = "zk-mobile-bench"
library_name = "zk_mobile_bench"
# output_dir = "target/mobench"  # default
# ffi_backend = "uniffi"          # or "native-c-abi" / "boltffi"

[android]
package = "com.example.bench"
min_sdk = 24
target_sdk = 34

[ios]
bundle_id = "com.example.bench"
deployment_target = "15.0"

[benchmarks]
default_function = "my_crate::my_benchmark"
default_iterations = 100
default_warmup = 10
```

`ffi_backend = "native-c-abi"` selects the direct mobench JSON C ABI path for
benchmarks where UniFFI overhead should not be included in timing or memory
measurements. Export the ABI from the benchmark crate with
`mobench_sdk::export_native_c_abi!()`.

`ffi_backend = "boltffi"` selects BoltFFI-generated Kotlin/Swift bindings for
the generated mobile runners. Export a `run_benchmark_json(spec_json: &str) ->
Result<String, String>` function from the benchmark crate with
`#[boltffi::export]`.

Resolution precedence is: `--project-root` / `--crate-path` → explicit `--config` → discovered `mobench.toml` → Cargo workspace root → git root → legacy `bench-mobile` fallback.

### `bench-config.toml` (Run Configuration)

```toml
target = "android"
function = "sample_fns::fibonacci"
iterations = 100
warmup = 10
device_matrix = "device-matrix.yaml"
device_tags = ["default"] # optional; filter devices by tag

[browserstack]
app_automate_username = "${BROWSERSTACK_USERNAME}"
app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "my-project-benchmarks"

[ios_xcuitest]
app = "target/mobench/ios/BenchRunner.ipa"
test_suite = "target/mobench/ios/BenchRunnerUITests.zip"
```

### `device-matrix.yaml`

```yaml
devices:
  - name: "Google Pixel 7-13.0"
    os: "android"
    os_version: "13.0"
    tags: ["default", "pixel"]
  - name: "iPhone 14-16"
    os: "ios"
    os_version: "16"
    tags: ["default", "iphone"]
```

## Requirements

### For Android

- Android NDK
- `cargo-ndk`: `cargo install cargo-ndk`
- Android SDK (API level 24+)

### For iOS

- macOS with Xcode
- Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`
- `xcodegen`: `brew install xcodegen`

## Part of mobench

This is the core SDK of the mobench ecosystem:

- **[mobench](https://crates.io/crates/mobench)** - CLI tool (recommended for most users)
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - This crate (SDK library with timing harness, build automation, and codegen)
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - `#[benchmark]` proc macro

## See Also

- [CLI Documentation](https://crates.io/crates/mobench) for command-line usage
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/) for FFI details
- [BrowserStack App Automate](https://www.browserstack.com/app-automate) for device testing

## License

Licensed under the MIT License. See [LICENSE.md](../../LICENSE.md) for details.

Copyright (c) 2026 World Foundation
