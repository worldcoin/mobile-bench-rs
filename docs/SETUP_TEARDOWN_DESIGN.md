# Setup/Teardown Implementation Design

## Overview

This document describes the implementation of setup/teardown support for `#[benchmark]` functions, allowing expensive initialization to be excluded from timing measurements.

## API Design

### Basic Usage (unchanged)
```rust
#[benchmark]
pub fn simple_benchmark() {
    let result = compute_something();
    std::hint::black_box(result);
}
```

### With Setup
```rust
fn setup_proof() -> ProofInput {
    ProofInput {
        proof: generate_complex_proof(),  // Expensive, not measured
        data: load_test_data(),
    }
}

#[benchmark(setup = setup_proof)]
pub fn verify_proof(input: &ProofInput) {
    verify(&input.proof);  // Only this is measured
}
```

### With Setup and Teardown
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

### Setup Modes

Two modes for when setup runs:

```rust
// Mode 1: Setup once before all iterations (default)
#[benchmark(setup = setup_proof)]
pub fn verify_proof(input: &ProofInput) { ... }

// Mode 2: Setup before each iteration (for mutations)
#[benchmark(setup = setup_data, per_iteration)]
pub fn sort_data(data: Vec<i32>) {  // Takes ownership
    data.sort();
}
```

---

## Implementation Changes

### 1. `timing.rs` - Add Setup-Aware Run Functions

```rust
/// Runs a benchmark with setup that executes once before all iterations.
///
/// The setup function is called once, then the benchmark runs multiple
/// times using a reference to the setup result.
pub fn run_closure_with_setup<S, T, F>(
    spec: BenchSpec,
    setup: S,
    mut f: F,
) -> Result<BenchReport, TimingError>
where
    S: FnOnce() -> T,
    F: FnMut(&T) -> Result<(), TimingError>,
{
    // Validate iterations
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations { count: spec.iterations });
    }

    // Setup phase - not timed
    let input = setup();

    // Warmup phase - not recorded
    for _ in 0..spec.warmup {
        f(&input)?;
    }

    // Measurement phase
    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for _ in 0..spec.iterations {
        let start = Instant::now();
        f(&input)?;
        samples.push(BenchSample::from_duration(start.elapsed()));
    }

    Ok(BenchReport { spec, samples })
}

/// Runs a benchmark with per-iteration setup.
///
/// Setup runs before each iteration and is not timed.
/// The benchmark takes ownership of the setup result.
pub fn run_closure_with_setup_per_iter<S, T, F>(
    spec: BenchSpec,
    mut setup: S,
    mut f: F,
) -> Result<BenchReport, TimingError>
where
    S: FnMut() -> T,
    F: FnMut(T) -> Result<(), TimingError>,
{
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations { count: spec.iterations });
    }

    // Warmup phase
    for _ in 0..spec.warmup {
        let input = setup();
        f(input)?;
    }

    // Measurement phase
    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for _ in 0..spec.iterations {
        let input = setup();  // Not timed

        let start = Instant::now();
        f(input)?;  // Only this is timed
        samples.push(BenchSample::from_duration(start.elapsed()));
    }

    Ok(BenchReport { spec, samples })
}

/// Runs a benchmark with setup and teardown.
pub fn run_closure_with_setup_teardown<S, T, F, D>(
    spec: BenchSpec,
    setup: S,
    mut f: F,
    teardown: D,
) -> Result<BenchReport, TimingError>
where
    S: FnOnce() -> T,
    F: FnMut(&T) -> Result<(), TimingError>,
    D: FnOnce(T),
{
    if spec.iterations == 0 {
        return Err(TimingError::NoIterations { count: spec.iterations });
    }

    let input = setup();

    // Warmup
    for _ in 0..spec.warmup {
        f(&input)?;
    }

    // Measurement
    let mut samples = Vec::with_capacity(spec.iterations as usize);
    for _ in 0..spec.iterations {
        let start = Instant::now();
        f(&input)?;
        samples.push(BenchSample::from_duration(start.elapsed()));
    }

    // Teardown - not timed
    teardown(input);

    Ok(BenchReport { spec, samples })
}
```

### 2. `registry.rs` - Change to Store Runner Functions

```rust
use crate::timing::{BenchReport, BenchSpec, TimingError};

/// A registered benchmark function
pub struct BenchFunction {
    /// Fully-qualified name of the benchmark function
    pub name: &'static str,

    /// Runner function that executes the benchmark with timing
    ///
    /// Takes a BenchSpec and returns a BenchReport directly.
    /// The runner handles setup/teardown internally.
    pub runner: fn(BenchSpec) -> Result<BenchReport, TimingError>,
}

inventory::collect!(BenchFunction);

// find_benchmark, discover_benchmarks, list_benchmark_names remain the same
```

### 3. `runner.rs` - Simplify to Delegate to Registry

```rust
pub fn run_benchmark(spec: BenchSpec) -> Result<RunnerReport, BenchError> {
    let bench_fn = find_benchmark(&spec.name).ok_or_else(|| {
        let available = list_benchmark_names()
            .into_iter()
            .map(String::from)
            .collect();
        BenchError::UnknownFunction(spec.name.clone(), available)
    })?;

    // Simply call the stored runner - it handles setup/teardown
    let report = (bench_fn.runner)(spec)?;
    Ok(report)
}
```

### 4. `mobench-macros/src/lib.rs` - Parse Setup/Teardown Attributes

```rust
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn, Meta, Expr, Ident};

struct BenchmarkArgs {
    setup: Option<Ident>,
    teardown: Option<Ident>,
    per_iteration: bool,
}

impl BenchmarkArgs {
    fn parse(attr: TokenStream) -> syn::Result<Self> {
        // Parse: #[benchmark] or #[benchmark(setup = foo, teardown = bar, per_iteration)]
        // ...
    }
}

#[proc_macro_attribute]
pub fn benchmark(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match BenchmarkArgs::parse(attr) {
        Ok(args) => args,
        Err(e) => return e.to_compile_error().into(),
    };

    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();
    let vis = &input_fn.vis;
    let sig = &input_fn.sig;
    let block = &input_fn.block;
    let attrs = &input_fn.attrs;

    // Validate based on whether setup is provided
    if args.setup.is_some() {
        // With setup: must have exactly one parameter (reference or owned)
        validate_setup_benchmark(&input_fn)?;
    } else {
        // No setup: must have no parameters (current behavior)
        validate_simple_benchmark(&input_fn)?;
    }

    // Generate the runner based on configuration
    let runner = generate_runner(&fn_name, &args);

    let expanded = quote! {
        #(#attrs)*
        #vis #sig {
            #block
        }

        ::inventory::submit! {
            ::mobench_sdk::registry::BenchFunction {
                name: ::std::concat!(::std::module_path!(), "::", #fn_name_str),
                runner: #runner,
            }
        }
    };

    TokenStream::from(expanded)
}

fn generate_runner(fn_name: &Ident, args: &BenchmarkArgs) -> proc_macro2::TokenStream {
    match (&args.setup, &args.teardown, args.per_iteration) {
        // No setup - current behavior
        (None, None, _) => quote! {
            |spec| ::mobench_sdk::timing::run_closure(spec, || {
                #fn_name();
                Ok(())
            })
        },

        // Setup only, runs once
        (Some(setup), None, false) => quote! {
            |spec| ::mobench_sdk::timing::run_closure_with_setup(
                spec,
                || #setup(),
                |input| {
                    #fn_name(input);
                    Ok(())
                },
            )
        },

        // Setup only, per iteration
        (Some(setup), None, true) => quote! {
            |spec| ::mobench_sdk::timing::run_closure_with_setup_per_iter(
                spec,
                || #setup(),
                |input| {
                    #fn_name(input);
                    Ok(())
                },
            )
        },

        // Setup + teardown
        (Some(setup), Some(teardown), false) => quote! {
            |spec| ::mobench_sdk::timing::run_closure_with_setup_teardown(
                spec,
                || #setup(),
                |input| {
                    #fn_name(input);
                    Ok(())
                },
                |input| #teardown(input),
            )
        },

        // Teardown without setup is invalid
        (None, Some(_), _) => {
            // This would be caught earlier during validation
            quote! { compile_error!("teardown requires setup") }
        }

        // Per-iteration with teardown not supported yet
        (Some(_), Some(_), true) => {
            quote! { compile_error!("per_iteration with teardown is not supported") }
        }
    }
}
```

---

## Migration & Compatibility

### Breaking Change

The `BenchFunction.invoke` field changes to `BenchFunction.runner`. This affects:
1. Any code directly accessing `BenchFunction.invoke`
2. The FFI examples that use the registry directly

### Migration Path

1. Update `BenchFunction` in registry.rs
2. Update macro to generate runner instead of invoke
3. Update `run_benchmark` in runner.rs
4. Update any FFI code that accesses registry directly

### FFI Considerations

The mobile apps call `run_benchmark(spec)` which is unchanged. The setup/teardown
all happens within Rust before results cross the FFI boundary. No changes needed
to Kotlin/Swift code.

---

## Example Expansions

### Simple Benchmark (unchanged behavior)

```rust
#[benchmark]
pub fn fibonacci() {
    std::hint::black_box(fib(20));
}

// Expands to:
pub fn fibonacci() {
    std::hint::black_box(fib(20));
}

inventory::submit! {
    mobench_sdk::registry::BenchFunction {
        name: "my_crate::fibonacci",
        runner: |spec| mobench_sdk::timing::run_closure(spec, || {
            fibonacci();
            Ok(())
        }),
    }
}
```

### With Setup

```rust
fn setup_proof() -> ProofInput { ... }

#[benchmark(setup = setup_proof)]
pub fn verify_proof(input: &ProofInput) {
    verify(&input.proof);
}

// Expands to:
pub fn verify_proof(input: &ProofInput) {
    verify(&input.proof);
}

inventory::submit! {
    mobench_sdk::registry::BenchFunction {
        name: "my_crate::verify_proof",
        runner: |spec| mobench_sdk::timing::run_closure_with_setup(
            spec,
            || setup_proof(),
            |input| {
                verify_proof(input);
                Ok(())
            },
        ),
    }
}
```

### Per-Iteration Setup

```rust
fn generate_random_vec() -> Vec<i32> {
    (0..1000).map(|_| rand::random()).collect()
}

#[benchmark(setup = generate_random_vec, per_iteration)]
pub fn sort_benchmark(mut data: Vec<i32>) {
    data.sort();
    std::hint::black_box(data);
}

// Expands to:
pub fn sort_benchmark(mut data: Vec<i32>) {
    data.sort();
    std::hint::black_box(data);
}

inventory::submit! {
    mobench_sdk::registry::BenchFunction {
        name: "my_crate::sort_benchmark",
        runner: |spec| mobench_sdk::timing::run_closure_with_setup_per_iter(
            spec,
            || generate_random_vec(),
            |data| {
                sort_benchmark(data);
                Ok(())
            },
        ),
    }
}
```

---

## Testing Strategy

1. **Unit tests for timing functions**
   - Test `run_closure_with_setup` verifies setup runs once
   - Test `run_closure_with_setup_per_iter` verifies setup runs each iteration
   - Test teardown is called after all iterations

2. **Macro expansion tests**
   - Test simple benchmark still works
   - Test setup-only benchmark
   - Test setup + teardown benchmark
   - Test per_iteration mode

3. **Integration tests**
   - End-to-end test with actual setup function
   - Verify timing excludes setup

4. **Compile-fail tests**
   - `#[benchmark(teardown = foo)]` without setup should fail
   - `#[benchmark(setup = foo)]` with wrong parameter count should fail

---

## Implementation Order

1. Add new timing functions to `timing.rs` (no breaking changes)
2. Add `BenchFunction.runner` alongside `BenchFunction.invoke` temporarily
3. Update macro to use new runner field
4. Update `run_benchmark` to use runner
5. Remove deprecated `invoke` field
6. Add tests
7. Update documentation
