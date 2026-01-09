use camino::Utf8PathBuf;
use std::env;
use std::fs;
use uniffi_bindgen::bindings::{KotlinBindingGenerator, SwiftBindingGenerator};
use uniffi_bindgen::library_mode::generate_bindings;

fn main() {
    let manifest_dir = Utf8PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();

    // Build the library first to generate metadata
    println!("Building library to generate UniFFI metadata...");
    let target_dir = root_dir.join("target");
    let lib_file = if cfg!(target_os = "macos") {
        target_dir.join("debug/libsample_fns.dylib")
    } else if cfg!(target_os = "linux") {
        target_dir.join("debug/libsample_fns.so")
    } else {
        target_dir.join("debug/sample_fns.dll")
    };

    println!("Using library: {:?}", lib_file);

    // Generate Kotlin bindings
    let kotlin_out = root_dir.join("android/app/src/main/java");
    fs::create_dir_all(&kotlin_out).unwrap();
    println!("Generating Kotlin bindings to {:?}", kotlin_out);

    generate_bindings(
        &lib_file,
        None, // crate name (auto-detect)
        &KotlinBindingGenerator,
        &uniffi_bindgen::cargo_metadata::CrateConfigSupplier::default(),
        None, // config override path
        &kotlin_out,
        false, // try_format_code
    ).unwrap();

    println!("✓ Kotlin bindings generated");

    // Generate Swift bindings
    let swift_out = root_dir.join("ios/BenchRunner/BenchRunner/Generated");
    fs::create_dir_all(&swift_out).unwrap();
    println!("Generating Swift bindings to {:?}", swift_out);

    generate_bindings(
        &lib_file,
        None, // crate name (auto-detect)
        &SwiftBindingGenerator,
        &uniffi_bindgen::cargo_metadata::CrateConfigSupplier::default(),
        None, // config override path
        &swift_out,
        false, // try_format_code
    ).unwrap();

    println!("✓ Swift bindings generated");

    println!("\n✓ All bindings generated successfully");

    // List generated files
    println!("\nGenerated Kotlin files:");
    list_files_recursively(&kotlin_out);

    println!("\nGenerated Swift files:");
    list_files_recursively(&swift_out);
}

fn list_files_recursively(dir: &Utf8PathBuf) {
    if let Ok(entries) = fs::read_dir(dir.as_std_path()) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    println!("  Directory: {}", path.display());
                    list_files_recursively(&Utf8PathBuf::from_path_buf(path).unwrap());
                } else {
                    println!("  File: {}", path.display());
                }
            }
        }
    }
}
