//! Build automation for mobile platforms
//!
//! This module provides builders for Android and iOS that automate the process
//! of compiling Rust code to mobile libraries and packaging them into mobile apps.

pub mod android;
pub mod ios;

// Re-export builders
pub use android::AndroidBuilder;
pub use ios::IosBuilder;
