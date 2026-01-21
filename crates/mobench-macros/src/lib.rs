//! # mobench-macros
//!
//! [![Crates.io](https://img.shields.io/crates/v/mobench-macros.svg)](https://crates.io/crates/mobench-macros)
//! [![Documentation](https://docs.rs/mobench-macros/badge.svg)](https://docs.rs/mobench-macros)
//! [![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/worldcoin/mobile-bench-rs/blob/main/LICENSE)
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
//! ## Setup and Teardown
//!
//! For benchmarks that need expensive setup that shouldn't be measured:
//!
//! ```ignore
//! use mobench_sdk::benchmark;
//!
//! fn setup_data() -> Vec<u8> {
//!     vec![0u8; 1_000_000]  // Not measured
//! }
//!
//! #[benchmark(setup = setup_data)]
//! fn hash_benchmark(data: &Vec<u8>) {
//!     std::hint::black_box(compute_hash(data));  // Only this is measured
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
//! 4. **Handles setup/teardown** - If specified, wraps the benchmark with setup/teardown that aren't timed
//!
//! ## Requirements
//!
//! - The [`inventory`](https://crates.io/crates/inventory) crate must be in your dependency tree
//! - Simple benchmarks: no parameters, returns `()`
//! - With setup: exactly one parameter (reference to setup result), returns `()`
//! - The function should not panic during normal execution
//!
//! ## Crate Ecosystem
//!
//! This crate is part of the mobench ecosystem:
//!
//! - **[`mobench-sdk`](https://crates.io/crates/mobench-sdk)** - Core SDK with timing harness (re-exports this macro)
//! - **[`mobench`](https://crates.io/crates/mobench)** - CLI tool
//! - **`mobench-macros`** (this crate) - Proc macros

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    Ident, ItemFn, ReturnType, Token,
};

/// Arguments to the benchmark attribute
struct BenchmarkArgs {
    setup: Option<Ident>,
    teardown: Option<Ident>,
    per_iteration: bool,
}

impl Parse for BenchmarkArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut setup = None;
        let mut teardown = None;
        let mut per_iteration = false;

        if input.is_empty() {
            return Ok(Self {
                setup,
                teardown,
                per_iteration,
            });
        }

        // Parse key = value pairs separated by commas
        let args = Punctuated::<BenchmarkArg, Token![,]>::parse_terminated(input)?;

        for arg in args {
            match arg {
                BenchmarkArg::Setup(ident) => {
                    if setup.is_some() {
                        return Err(syn::Error::new_spanned(ident, "duplicate setup argument"));
                    }
                    setup = Some(ident);
                }
                BenchmarkArg::Teardown(ident) => {
                    if teardown.is_some() {
                        return Err(syn::Error::new_spanned(
                            ident,
                            "duplicate teardown argument",
                        ));
                    }
                    teardown = Some(ident);
                }
                BenchmarkArg::PerIteration => {
                    per_iteration = true;
                }
            }
        }

        // Validate: teardown without setup is invalid
        if teardown.is_some() && setup.is_none() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "teardown requires setup to be specified",
            ));
        }

        // Validate: per_iteration with teardown is not supported
        if per_iteration && teardown.is_some() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "per_iteration mode is not compatible with teardown",
            ));
        }

        Ok(Self {
            setup,
            teardown,
            per_iteration,
        })
    }
}

enum BenchmarkArg {
    Setup(Ident),
    Teardown(Ident),
    PerIteration,
}

impl Parse for BenchmarkArg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;

        match name.to_string().as_str() {
            "setup" => {
                input.parse::<Token![=]>()?;
                let value: Ident = input.parse()?;
                Ok(BenchmarkArg::Setup(value))
            }
            "teardown" => {
                input.parse::<Token![=]>()?;
                let value: Ident = input.parse()?;
                Ok(BenchmarkArg::Teardown(value))
            }
            "per_iteration" => Ok(BenchmarkArg::PerIteration),
            _ => Err(syn::Error::new_spanned(
                name,
                "expected 'setup', 'teardown', or 'per_iteration'",
            )),
        }
    }
}

/// Marks a function as a benchmark for mobile execution.
///
/// This attribute macro registers the function in the global benchmark registry,
/// making it discoverable and executable by the mobench runtime.
///
/// # Basic Usage
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
/// # With Setup (setup runs once, not measured)
///
/// ```ignore
/// use mobench_sdk::benchmark;
///
/// fn setup_proof() -> ProofInput {
///     ProofInput::generate()  // Expensive, not measured
/// }
///
/// #[benchmark(setup = setup_proof)]
/// fn verify_proof(input: &ProofInput) {
///     verify(&input.proof);  // Only this is measured
/// }
/// ```
///
/// # With Per-Iteration Setup (for mutating benchmarks)
///
/// ```ignore
/// use mobench_sdk::benchmark;
///
/// fn generate_random_vec() -> Vec<i32> {
///     (0..1000).map(|_| rand::random()).collect()
/// }
///
/// #[benchmark(setup = generate_random_vec, per_iteration)]
/// fn sort_benchmark(data: Vec<i32>) {
///     let mut data = data;
///     data.sort();
///     std::hint::black_box(data);
/// }
/// ```
///
/// # With Setup and Teardown
///
/// ```ignore
/// use mobench_sdk::benchmark;
///
/// fn setup_db() -> Database { Database::connect("test.db") }
/// fn cleanup_db(db: Database) { db.close(); }
///
/// #[benchmark(setup = setup_db, teardown = cleanup_db)]
/// fn db_query(db: &Database) {
///     db.query("SELECT * FROM users");
/// }
/// ```
///
/// # Function Requirements
///
/// **Without setup:**
/// - Take no parameters
/// - Return `()` (unit type)
///
/// **With setup:**
/// - Take exactly one parameter (reference to setup result, or owned for per_iteration)
/// - Return `()` (unit type)
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
#[proc_macro_attribute]
pub fn benchmark(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as BenchmarkArgs);
    let input_fn = parse_macro_input!(item as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();
    let vis = &input_fn.vis;
    let sig = &input_fn.sig;
    let block = &input_fn.block;
    let attrs = &input_fn.attrs;

    // Validate based on whether setup is provided
    if args.setup.is_some() {
        // With setup: must have exactly one parameter
        if input_fn.sig.inputs.len() != 1 {
            let param_count = input_fn.sig.inputs.len();
            return syn::Error::new_spanned(
                &input_fn.sig,
                format!(
                    "#[benchmark(setup = ...)] functions must take exactly one parameter.\n\
                     Found {} parameter(s).\n\n\
                     Example:\n\
                     fn setup_data() -> MyData {{ ... }}\n\n\
                     #[benchmark(setup = setup_data)]\n\
                     fn {}(input: &MyData) {{\n\
                         // input is the result of setup_data()\n\
                     }}",
                    param_count, fn_name_str
                ),
            )
            .to_compile_error()
            .into();
        }
    } else {
        // No setup: must have no parameters
        if !input_fn.sig.inputs.is_empty() {
            let param_count = input_fn.sig.inputs.len();
            let param_names: Vec<String> = input_fn
                .sig
                .inputs
                .iter()
                .map(|arg| match arg {
                    syn::FnArg::Receiver(_) => "self".to_string(),
                    syn::FnArg::Typed(pat) => quote!(#pat).to_string(),
                })
                .collect();
            return syn::Error::new_spanned(
                &input_fn.sig.inputs,
                format!(
                    "#[benchmark] functions must take no parameters.\n\
                     Found {} parameter(s): {}\n\n\
                     If you need setup data, use the setup attribute:\n\n\
                     fn setup_data() -> MyData {{ ... }}\n\n\
                     #[benchmark(setup = setup_data)]\n\
                     fn {}(input: &MyData) {{\n\
                         // Your benchmark code using input\n\
                     }}",
                    param_count,
                    param_names.join(", "),
                    fn_name_str
                ),
            )
            .to_compile_error()
            .into();
        }
    }

    // Validate: function must return () (unit type)
    match &input_fn.sig.output {
        ReturnType::Default => {} // () return type is OK
        ReturnType::Type(_, return_type) => {
            let type_str = quote!(#return_type).to_string();
            if type_str.trim() != "()" {
                return syn::Error::new_spanned(
                    return_type,
                    format!(
                        "#[benchmark] functions must return () (unit type).\n\
                         Found return type: {}\n\n\
                         Benchmark results should be consumed with std::hint::black_box() \
                         rather than returned:\n\n\
                         #[benchmark]\n\
                         fn {}() {{\n\
                             let result = compute_something();\n\
                             std::hint::black_box(result);  // Prevents optimization\n\
                         }}",
                        type_str, fn_name_str
                    ),
                )
                .to_compile_error()
                .into();
            }
        }
    }

    // Generate the runner based on configuration
    let runner = generate_runner(fn_name, &args);

    let expanded = quote! {
        // Preserve the original function
        #(#attrs)*
        #vis #sig {
            #block
        }

        // Register the function with inventory
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
        // No setup - simple benchmark
        (None, None, _) => quote! {
            |spec: ::mobench_sdk::timing::BenchSpec| -> ::std::result::Result<::mobench_sdk::timing::BenchReport, ::mobench_sdk::timing::TimingError> {
                ::mobench_sdk::timing::run_closure(spec, || {
                    #fn_name();
                    Ok(())
                })
            }
        },

        // Setup only, runs once before all iterations
        (Some(setup), None, false) => quote! {
            |spec: ::mobench_sdk::timing::BenchSpec| -> ::std::result::Result<::mobench_sdk::timing::BenchReport, ::mobench_sdk::timing::TimingError> {
                ::mobench_sdk::timing::run_closure_with_setup(
                    spec,
                    || #setup(),
                    |input| {
                        #fn_name(input);
                        Ok(())
                    },
                )
            }
        },

        // Setup only, per iteration (for mutating benchmarks)
        (Some(setup), None, true) => quote! {
            |spec: ::mobench_sdk::timing::BenchSpec| -> ::std::result::Result<::mobench_sdk::timing::BenchReport, ::mobench_sdk::timing::TimingError> {
                ::mobench_sdk::timing::run_closure_with_setup_per_iter(
                    spec,
                    || #setup(),
                    |input| {
                        #fn_name(input);
                        Ok(())
                    },
                )
            }
        },

        // Setup + teardown (per_iteration with teardown is rejected during parsing)
        (Some(setup), Some(teardown), false) => quote! {
            |spec: ::mobench_sdk::timing::BenchSpec| -> ::std::result::Result<::mobench_sdk::timing::BenchReport, ::mobench_sdk::timing::TimingError> {
                ::mobench_sdk::timing::run_closure_with_setup_teardown(
                    spec,
                    || #setup(),
                    |input| {
                        #fn_name(input);
                        Ok(())
                    },
                    |input| #teardown(input),
                )
            }
        },

        // These cases are rejected during parsing, but we need to handle them
        (None, Some(_), _) | (Some(_), Some(_), true) => {
            quote! { compile_error!("invalid benchmark configuration") }
        }
    }
}
