// Standalone script to generate UniFFI bindings
// Usage: cargo script scripts/bindgen.rs

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.parent().unwrap();
    let lib_path = root_dir.join("target/debug/libsample_fns.dylib");

    if !lib_path.exists() {
        let lib_path_so = root_dir.join("target/debug/libsample_fns.so");
        if lib_path_so.exists() {
            generate_bindings(&lib_path_so, root_dir)?;
        } else {
            eprintln!("Error: Library not found. Run 'cargo build -p sample-fns' first");
            std::process::exit(1);
        }
    } else {
        generate_bindings(&lib_path, root_dir)?;
    }

    Ok(())
}

fn generate_bindings(lib_path: &Path, root_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use uniffi_bindgen::{bindings, library_mode};

    // Generate Kotlin bindings
    let kotlin_out = root_dir.join("android/app/src/main/java");
    library_mode::generate_bindings(
        lib_path,
        None,
        &bindings::TargetLanguage::Kotlin,
        &kotlin_out,
        false,
    )?;

    println!("✓ Kotlin bindings generated: {:?}", kotlin_out);

    // Generate Swift bindings
    let swift_out = root_dir.join("ios/BenchRunner/BenchRunner/Generated");
    std::fs::create_dir_all(&swift_out)?;
    library_mode::generate_bindings(
        lib_path,
        None,
        &bindings::TargetLanguage::Swift,
        &swift_out,
        false,
    )?;

    println!("✓ Swift bindings generated: {:?}", swift_out);

    Ok(())
}
