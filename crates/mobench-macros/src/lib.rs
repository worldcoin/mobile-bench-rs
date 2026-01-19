//! # mobench-macros
//!
//! Procedural macros for the mobench mobile benchmarking SDK.
//!
//! This crate provides the [`#[benchmark]`](macro@benchmark) attribute macro
//! that marks functions for mobile benchmarking. Functions annotated with this
//! macro are automatically registered in a global registry and can be discovered
//! and executed at runtime.
//!
//! ## Usage
//!
//! Most users should import the macro via [`mobench-sdk`](https://crates.io/crates/mobench-sdk)
//! rather than using this crate directly:
//!
//! ```ignore
//! use mobench_sdk::benchmark;
//!
//! #[benchmark]
//! fn my_benchmark() {
//!     // Your benchmark code here
//!     let result = expensive_computation();
//!     std::hint::black_box(result);
//! }
//! ```
//!
//! ## How It Works
//!
//! The `#[benchmark]` macro:
//!
//! 1. **Preserves the original function** - The function remains callable as normal
//! 2. **Registers with inventory** - Creates a static registration that the SDK discovers at runtime
//! 3. **Captures the fully-qualified name** - Uses `module_path!()` to generate unique names like `my_crate::my_module::my_benchmark`
//!
//! ## Requirements
//!
//! - The [`inventory`](https://crates.io/crates/inventory) crate must be in your dependency tree
//! - Functions must have no parameters and return `()`
//! - The function should not panic during normal execution
//!
//! ## Example: Multiple Benchmarks
//!
//! ```ignore
//! use mobench_sdk::benchmark;
//!
//! #[benchmark]
//! fn benchmark_sorting() {
//!     let mut data: Vec<i32> = (0..1000).rev().collect();
//!     data.sort();
//!     std::hint::black_box(data);
//! }
//!
//! #[benchmark]
//! fn benchmark_hashing() {
//!     use std::collections::hash_map::DefaultHasher;
//!     use std::hash::{Hash, Hasher};
//!
//!     let mut hasher = DefaultHasher::new();
//!     "hello world".hash(&mut hasher);
//!     std::hint::black_box(hasher.finish());
//! }
//! ```
//!
//! Both functions will be registered with names like:
//! - `my_crate::benchmark_sorting`
//! - `my_crate::benchmark_hashing`
//!
//! ## Crate Ecosystem
//!
//! This crate is part of the mobench ecosystem:
//!
//! - **[`mobench-sdk`](https://crates.io/crates/mobench-sdk)** - Core SDK (re-exports this macro)
//! - **[`mobench`](https://crates.io/crates/mobench)** - CLI tool
//! - **`mobench-macros`** (this crate) - Proc macros
//! - **[`mobench-runner`](https://crates.io/crates/mobench-runner)** - Timing harness

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// Marks a function as a benchmark for mobile execution.
///
/// This attribute macro registers the function in the global benchmark registry,
/// making it discoverable and executable by the mobench runtime.
///
/// # Usage
///
/// ```ignore
/// use mobench_sdk::benchmark;
///
/// #[benchmark]
/// fn fibonacci_bench() {
///     let result = fibonacci(30);
///     std::hint::black_box(result);
/// }
/// ```
///
/// # Function Requirements
///
/// The annotated function must:
/// - Take no parameters
/// - Return `()` (unit type)
/// - Not panic during normal execution
///
/// # Best Practices
///
/// ## Use `black_box` to Prevent Optimization
///
/// Always wrap results with [`std::hint::black_box`] to prevent the compiler
/// from optimizing away the computation:
///
/// ```ignore
/// #[benchmark]
/// fn good_benchmark() {
///     let result = compute_something();
///     std::hint::black_box(result);  // Prevents optimization
/// }
/// ```
///
/// ## Avoid Side Effects
///
/// Benchmarks should be deterministic. Avoid:
/// - File I/O
/// - Network calls
/// - Random number generation (unless seeded)
/// - Global mutable state
///
/// ## Keep Benchmarks Focused
///
/// Each benchmark should measure one specific operation:
///
/// ```ignore
/// // Good: Focused benchmark
/// #[benchmark]
/// fn benchmark_json_parse() {
///     let json = r#"{"key": "value"}"#;
///     let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
///     std::hint::black_box(parsed);
/// }
///
/// // Avoid: Multiple operations in one benchmark
/// #[benchmark]
/// fn benchmark_everything() {
///     let json = create_json();  // Measured
///     let parsed = parse_json(&json);  // Measured
///     let serialized = serialize(parsed);  // Measured
///     std::hint::black_box(serialized);
/// }
/// ```
///
/// # Generated Code
///
/// The macro generates code equivalent to:
///
/// ```ignore
/// fn my_benchmark() {
///     // Original function body
/// }
///
/// inventory::submit! {
///     mobench_sdk::registry::BenchFunction {
///         name: "my_crate::my_module::my_benchmark",
///         invoke: |_args| {
///             my_benchmark();
///             Ok(())
///         },
///     }
/// }
/// ```
///
/// # Discovering Benchmarks
///
/// Registered benchmarks can be discovered at runtime:
///
/// ```ignore
/// use mobench_sdk::{discover_benchmarks, list_benchmark_names};
///
/// // Get all benchmark names
/// for name in list_benchmark_names() {
///     println!("Found: {}", name);
/// }
///
/// // Get full benchmark info
/// for bench in discover_benchmarks() {
///     println!("Benchmark: {}", bench.name);
/// }
/// ```
#[proc_macro_attribute]
pub fn benchmark(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();
    let vis = &input_fn.vis;
    let sig = &input_fn.sig;
    let block = &input_fn.block;
    let attrs = &input_fn.attrs;

    // Get the module path for fully-qualified name
    // Note: This will generate the fully-qualified name at compile time
    let module_path = quote! { module_path!() };

    let expanded = quote! {
        // Preserve the original function
        #(#attrs)*
        #vis #sig {
            #block
        }

        // Register the function with inventory
        ::inventory::submit! {
            ::mobench_sdk::registry::BenchFunction {
                name: ::std::concat!(#module_path, "::", #fn_name_str),
                invoke: |_args| {
                    #fn_name();
                    Ok(())
                },
            }
        }
    };

    TokenStream::from(expanded)
}
