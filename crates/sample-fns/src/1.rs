//! Sample functions that can be pulled into the mobile benchmarking harness.
//! These double as simple host-side demos for the CLI and JNI wrapper.

use bench_runner::{BenchSpec, run_closure};
use core::ffi::{c_char, c_void};
use jni::JNIEnv;
use jni::objects::{JObject, JString};
use jni::sys::{jint, jstring};
use serde_json::json;
use std::ffi::{CStr, CString};

const DEFAULT_FUNCTION: &str = "sample_fns::fibonacci";
const CHECKSUM_INPUT: [u8; 1024] = [1; 1024];

pub fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => {
            let mut a = 0u64;
            let mut b = 1u64;
            for _ in 2..=n {
                let next = a + b;
                a = b;
                b = next;
            }
            b
        }
    }
}

pub fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().map(|&b| b as u64).sum()
}

// Simple FFI entrypoints the mobile runners can link against.
#[unsafe(no_mangle)]
pub extern "C" fn bench_fib_24() -> u64 {
    fibonacci(24)
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_checksum_1k() -> u64 {
    checksum(&CHECKSUM_INPUT)
}

/// Generic C ABI: run a named benchmark and return JSON as a heap-allocated C string.
/// Caller must free with `bench_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn bench_run_json(
    function: *const c_char,
    iterations: u32,
    warmup: u32,
) -> *mut c_char {
    let fn_name = unsafe {
        if function.is_null() {
            DEFAULT_FUNCTION.to_string()
        } else {
            CStr::from_ptr(function).to_string_lossy().into_owned()
        }
    };

    let result = run_named_benchmark(&fn_name, iterations, warmup).unwrap_or_else(|err| err);

    // SAFETY: CString will NUL-terminate; if the benchmark returned interior NULs we
    // fall back to a simple error string.
    CString::new(result)
        .unwrap_or_else(|_| CString::new("run error: invalid utf-8").expect("valid literal"))
        .into_raw()
}

/// Free strings returned from `bench_run_json`.
#[unsafe(no_mangle)]
pub extern "C" fn bench_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

// JNI glue for the Android wrapper: legacy direct bindings.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_world_bench_MainActivity_bench_1fib_124(
    _env: *mut c_void,
    _thiz: *mut c_void,
) -> i64 {
    bench_fib_24() as i64
}

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_world_bench_MainActivity_bench_1checksum_11k(
    _env: *mut c_void,
    _thiz: *mut c_void,
) -> i64 {
    bench_checksum_1k() as i64
}

// JNI glue that exercises the bench harness and returns a JSON string.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_world_bench_MainActivity_runBench(
    mut env: JNIEnv,
    _thiz: JObject,
    function: JString,
    iterations: jint,
    warmup: jint,
) -> jstring {
    let fn_name = env
        .get_string(&function)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|_| DEFAULT_FUNCTION.to_string());

    if iterations < 0 || warmup < 0 {
        return env
            .new_string("spec error: iterations and warmup must be non-negative")
            .expect("new_string failed")
            .into_raw();
    }

    let result = run_named_benchmark(&fn_name, iterations as u32, warmup as u32);
    let json = match result {
        Ok(json) => json,
        Err(msg) => {
            return env.new_string(msg).expect("new_string failed").into_raw();
        }
    };

    env.new_string(json).expect("new_string failed").into_raw()
}

fn run_named_benchmark(name: &str, iterations: u32, warmup: u32) -> Result<String, String> {
    let (resolved_name, bench_fn): (&'static str, fn()) = match normalize_function_name(name) {
        Some(name) => name,
        None => {
            return Err(format!(
                "spec error: unknown function '{name}', try '{DEFAULT_FUNCTION}' or 'sample_fns::checksum'"
            ));
        }
    };

    let spec = BenchSpec::new(resolved_name, iterations, warmup)
        .map_err(|e| format!("spec error: {e}"))?;
    let report = run_closure(spec.clone(), || {
        bench_fn();
        Ok(())
    })
    .map_err(|e| format!("run error: {e}"))?;

    let json = json!({
        "spec": {
            "name": spec.name,
            "iterations": spec.iterations,
            "warmup": spec.warmup,
        },
        "samples": report.samples,
    });

    Ok(json.to_string())
}

fn normalize_function_name(name: &str) -> Option<(&'static str, fn())> {
    match name {
        "fibonacci" | "fib" | "sample_fns::fibonacci" | "" => {
            Some((DEFAULT_FUNCTION, bench_fib_once))
        }
        "checksum" | "checksum_1k" | "sample_fns::checksum" => {
            Some(("sample_fns::checksum", bench_checksum_once))
        }
        _ => None,
    }
}

fn bench_fib_once() {
    let _ = fibonacci(24);
}

fn bench_checksum_once() {
    let _ = checksum(&CHECKSUM_INPUT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::ptr;

    #[test]
    fn fib_sequence() {
        assert_eq!(fibonacci(0), 0);
        assert_eq!(fibonacci(1), 1);
        assert_eq!(fibonacci(10), 55);
    }

    #[test]
    fn checksum_matches() {
        assert_eq!(checksum(&CHECKSUM_INPUT), 1024);
    }

    #[test]
    fn c_api_runs_benchmark() {
        let fn_name = CString::new("sample_fns::fibonacci").unwrap();
        let raw = bench_run_json(fn_name.as_ptr(), 2, 0);
        assert!(!raw.is_null());

        let json = unsafe { CString::from_raw(raw) }
            .into_string()
            .expect("utf8");
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["spec"]["name"], "sample_fns::fibonacci");
        assert_eq!(v["spec"]["iterations"], 2);
    }

    #[test]
    fn c_api_handles_null_function() {
        let raw = bench_run_json(ptr::null(), 1, 0);
        assert!(!raw.is_null());
        let json = unsafe { CString::from_raw(raw) }
            .into_string()
            .expect("utf8");
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["spec"]["name"], DEFAULT_FUNCTION);
    }
}
