//! Native JSON C ABI for benchmark runners.
//!
//! This module provides the implementation behind the stable native backend
//! contract. Benchmark crates can export the C symbols with
//! [`crate::export_native_c_abi!`], then generated Android/iOS apps can pass a
//! serialized [`crate::BenchSpec`] JSON payload and receive a serialized
//! [`crate::RunnerReport`] JSON payload without using UniFFI-generated bindings.

use crate::BenchSpec;
use libc::c_char;
use std::cell::RefCell;
use std::ffi::CString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::slice;

/// Owned byte buffer returned by the native mobench C ABI.
///
/// The buffer layout intentionally mirrors common Rust-to-C ownership patterns:
/// Rust allocates the bytes, transfers ownership by filling this struct, and
/// the caller returns ownership exactly once with `mobench_free_buf`.
#[repr(C)]
#[derive(Debug)]
pub struct MobenchBuf {
    /// Pointer to the first byte of the allocation.
    pub ptr: *mut u8,
    /// Number of initialized bytes.
    pub len: usize,
    /// Allocation capacity needed to reconstruct and free the buffer.
    pub cap: usize,
}

impl MobenchBuf {
    fn clear(&mut self) {
        self.ptr = ptr::null_mut();
        self.len = 0;
        self.cap = 0;
    }
}

impl Default for MobenchBuf {
    fn default() -> Self {
        Self {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

/// Runs a registered benchmark from JSON and writes the JSON report to `out`.
///
/// # Safety
///
/// `spec_ptr` must either be non-null and valid for reads of `spec_len` bytes,
/// or `spec_len` must be zero. `out` must be non-null and valid for writes of
/// one [`MobenchBuf`]. When this returns `0`, the caller owns `out` and must
/// release it exactly once with [`mobench_free_buf_impl`].
pub unsafe fn mobench_run_benchmark_json_impl(
    spec_ptr: *const u8,
    spec_len: usize,
    out: *mut MobenchBuf,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if out.is_null() {
            return Err("output buffer pointer must not be null".to_string());
        }

        // Leave `out` in a known empty state even when parsing or execution
        // fails, so native callers can avoid conditional cleanup paths.
        unsafe { (*out).clear() };

        if spec_len > 0 && spec_ptr.is_null() {
            return Err("spec pointer must not be null when spec length is non-zero".to_string());
        }

        let spec_bytes = if spec_len == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(spec_ptr, spec_len) }
        };
        let spec: BenchSpec = serde_json::from_slice(spec_bytes)
            .map_err(|error| format!("failed to parse BenchSpec JSON: {error}"))?;
        let report = crate::run_benchmark(spec).map_err(|error| error.to_string())?;
        let mut bytes = serde_json::to_vec(&report)
            .map_err(|error| format!("failed to serialize BenchReport JSON: {error}"))?;

        let buf = MobenchBuf {
            ptr: bytes.as_mut_ptr(),
            len: bytes.len(),
            cap: bytes.capacity(),
        };
        std::mem::forget(bytes);
        unsafe { *out = buf };
        Ok(())
    }));

    match result {
        Ok(Ok(())) => {
            clear_last_error();
            0
        }
        Ok(Err(error)) => {
            set_last_error(error);
            1
        }
        Err(_) => {
            set_last_error("benchmark panicked across native C ABI boundary");
            2
        }
    }
}

/// Frees a buffer returned by [`mobench_run_benchmark_json_impl`].
///
/// # Safety
///
/// `buf` may be null. If non-null and `buf.ptr` is non-null, the struct must
/// contain a buffer previously returned by this module that has not already
/// been freed. The struct is zeroed before this function returns.
pub unsafe fn mobench_free_buf_impl(buf: *mut MobenchBuf) {
    if buf.is_null() {
        return;
    }

    let buf_ref = unsafe { &mut *buf };
    if !buf_ref.ptr.is_null() {
        let ptr = buf_ref.ptr;
        let len = buf_ref.len;
        let cap = buf_ref.cap;
        buf_ref.clear();
        unsafe {
            drop(Vec::from_raw_parts(ptr, len, cap));
        }
    } else {
        buf_ref.clear();
    }
}

/// Returns the most recent native ABI error message for this thread.
pub fn mobench_last_error_message_impl() -> *const c_char {
    LAST_ERROR.with(|message| message.borrow().as_ptr())
}

fn clear_last_error() {
    LAST_ERROR.with(|message| *message.borrow_mut() = CString::default());
}

fn set_last_error(message: impl AsRef<str>) {
    let sanitized = message.as_ref().replace('\0', "\\0");
    let c_string = CString::new(sanitized).unwrap_or_default();
    LAST_ERROR.with(|last_error| *last_error.borrow_mut() = c_string);
}

/// Exports the stable mobench native JSON C ABI symbols from a benchmark crate.
///
/// Add this once in the root of a benchmark cdylib/staticlib crate that uses
/// the mobench registry:
///
/// ```ignore
/// mobench_sdk::export_native_c_abi!();
/// ```
#[macro_export]
macro_rules! export_native_c_abi {
    () => {
        /// Runs a mobench benchmark from a JSON `BenchSpec` payload.
        ///
        /// # Safety
        ///
        /// `spec_ptr` must be valid for `spec_len` bytes when `spec_len` is
        /// non-zero, and `out` must be valid for one writable
        /// [`mobench_sdk::MobenchBuf`].
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn mobench_run_benchmark_json(
            spec_ptr: *const u8,
            spec_len: usize,
            out: *mut $crate::MobenchBuf,
        ) -> i32 {
            unsafe {
                $crate::native_c_abi::mobench_run_benchmark_json_impl(spec_ptr, spec_len, out)
            }
        }

        /// Frees a `MobenchBuf` returned by `mobench_run_benchmark_json`.
        ///
        /// # Safety
        ///
        /// `buf` may be null. If non-null and non-empty, it must contain a
        /// buffer returned by `mobench_run_benchmark_json` that has not already
        /// been freed.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn mobench_free_buf(buf: *mut $crate::MobenchBuf) {
            unsafe { $crate::native_c_abi::mobench_free_buf_impl(buf) }
        }

        /// Returns the last native mobench C ABI error message for this thread.
        #[unsafe(no_mangle)]
        pub extern "C" fn mobench_last_error_message() -> *const ::std::os::raw::c_char {
            $crate::native_c_abi::mobench_last_error_message_impl()
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BenchFunction, TimingError};
    use std::ffi::CStr;

    fn native_abi_test_runner(spec: crate::BenchSpec) -> Result<crate::RunnerReport, TimingError> {
        Ok(crate::RunnerReport {
            spec,
            samples: vec![crate::BenchSample {
                duration_ns: 42,
                cpu_time_ms: None,
                peak_memory_kb: None,
                process_peak_memory_kb: None,
            }],
            phases: Vec::new(),
            timeline: Vec::new(),
        })
    }

    inventory::submit! {
        BenchFunction {
            name: "native_abi_test_benchmark",
            runner: native_abi_test_runner,
        }
    }

    #[test]
    fn runs_valid_spec_and_returns_report_json() {
        let spec = br#"{"name":"native_abi_test_benchmark","iterations":1,"warmup":0}"#;
        let mut out = MobenchBuf::default();

        let status =
            unsafe { mobench_run_benchmark_json_impl(spec.as_ptr(), spec.len(), &mut out) };

        assert_eq!(status, 0);
        assert!(!out.ptr.is_null());
        assert!(out.len > 0);

        let report_bytes = unsafe { slice::from_raw_parts(out.ptr, out.len) };
        let report: crate::RunnerReport = serde_json::from_slice(report_bytes).unwrap();
        assert_eq!(report.spec.name, "native_abi_test_benchmark");
        assert_eq!(report.samples[0].duration_ns, 42);

        unsafe { mobench_free_buf_impl(&mut out) };
        assert!(out.ptr.is_null());
        assert_eq!(out.len, 0);
        assert_eq!(out.cap, 0);
    }

    #[test]
    fn invalid_json_returns_error_without_output() {
        let spec = b"not json";
        let mut out = MobenchBuf::default();

        let status =
            unsafe { mobench_run_benchmark_json_impl(spec.as_ptr(), spec.len(), &mut out) };

        assert_ne!(status, 0);
        assert!(out.ptr.is_null());
        let error = unsafe { CStr::from_ptr(mobench_last_error_message_impl()) }
            .to_string_lossy()
            .into_owned();
        assert!(error.contains("failed to parse BenchSpec JSON"));
    }

    #[test]
    fn unknown_benchmark_returns_error_without_output() {
        let spec = br#"{"name":"definitely_missing","iterations":1,"warmup":0}"#;
        let mut out = MobenchBuf::default();

        let status =
            unsafe { mobench_run_benchmark_json_impl(spec.as_ptr(), spec.len(), &mut out) };

        assert_ne!(status, 0);
        assert!(out.ptr.is_null());
        let error = unsafe { CStr::from_ptr(mobench_last_error_message_impl()) }
            .to_string_lossy()
            .into_owned();
        assert!(error.contains("unknown benchmark function"));
    }

    #[test]
    fn free_null_and_empty_buffers_are_safe() {
        unsafe { mobench_free_buf_impl(ptr::null_mut()) };

        let mut out = MobenchBuf::default();
        unsafe { mobench_free_buf_impl(&mut out) };

        assert!(out.ptr.is_null());
        assert_eq!(out.len, 0);
        assert_eq!(out.cap, 0);
    }
}
