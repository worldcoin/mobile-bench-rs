//! Procedural macros for bench-sdk
//!
//! This crate provides the `#[benchmark]` attribute macro for marking functions
//! as benchmarkable. Functions marked with this attribute are automatically
//! registered and can be discovered at runtime.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

/// Marks a function as a benchmark.
///
/// This macro registers the function in the global benchmark registry and
/// makes it available for execution via the bench-sdk runtime.
///
/// # Example
///
/// ```ignore
/// use bench_sdk::benchmark;
///
/// #[benchmark]
/// fn fibonacci_bench() {
///     let result = fibonacci(30);
///     std::hint::black_box(result);
/// }
/// ```
///
/// The macro preserves the original function and creates a registration entry
/// that allows the benchmark to be discovered and invoked by name.
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
            ::bench_sdk::registry::BenchFunction {
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
