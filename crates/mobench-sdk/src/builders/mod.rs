//! Build automation for mobile platforms.
//!
//! This module provides builders for Android and iOS that automate the process
//! of compiling Rust code to mobile libraries and packaging them into mobile apps.
//!
//! ## Overview
//!
//! The builders handle the complete build pipeline:
//!
//! 1. **Rust compilation** - Cross-compile Rust to mobile targets
//! 2. **Binding generation** - Generate UniFFI Kotlin/Swift bindings
//! 3. **Native library packaging** - Copy `.so` files to Android or create xcframework for iOS
//! 4. **App building** - Run Gradle (Android) or xcodebuild (iOS)
//!
//! ## Builders
//!
//! | Builder | Platform | Output |
//! |---------|----------|--------|
//! | [`AndroidBuilder`] | Android | APK with native `.so` libraries |
//! | [`IosBuilder`] | iOS | xcframework with static libraries |
//!
//! ## Common Utilities
//!
//! The `common` module (internal) provides shared functionality:
//!
//! - Workspace-aware Cargo target directory detection
//! - Host library path resolution for UniFFI binding generation
//! - Consistent command execution with actionable error messages
//!
//! ## Builder Options
//!
//! Both builders support the following configuration:
//!
//! - **`verbose(bool)`** - Enable detailed output showing each build step
//! - **`dry_run(bool)`** - Preview build steps without making changes
//! - **`output_dir(path)`** - Customize output location (default: `target/mobench/`)
//! - **`crate_dir(path)`** - Override auto-detected crate location
//!
//! ## Example
//!
//! ```ignore
//! use mobench_sdk::builders::{AndroidBuilder, IosBuilder};
//! use mobench_sdk::{BuildConfig, BuildProfile, Target};
//!
//! // Build for Android with dry-run
//! let android = AndroidBuilder::new(".", "my-bench")
//!     .verbose(true)
//!     .dry_run(true);  // Preview only
//!
//! // Build for iOS
//! let ios = IosBuilder::new(".", "my-bench")
//!     .verbose(true);
//!
//! let config = BuildConfig {
//!     target: Target::Android,
//!     profile: BuildProfile::Release,
//!     incremental: true,
//! };
//!
//! android.build(&config)?;
//! # Ok::<(), mobench_sdk::BenchError>(())
//! ```

pub mod android;
pub mod ios;
pub mod common;

// Re-export builders
pub use android::AndroidBuilder;
pub use ios::{IosBuilder, SigningMethod};
pub use common::{embed_bench_spec, embed_bench_meta, EmbeddedBenchSpec, BenchMeta, create_bench_meta};
