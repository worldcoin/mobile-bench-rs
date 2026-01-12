use camino::Utf8PathBuf;
use std::env;
use std::fs;
use std::process;
use uniffi_bindgen::bindings::{KotlinBindingGenerator, SwiftBindingGenerator};

fn main() {
    let manifest_dir = Utf8PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();
    let udl_file = manifest_dir.join("src/sample_fns.udl");

    if !udl_file.exists() {
        eprintln!("Error: UDL file not found at {:?}", udl_file);
        process::exit(1);
    }

    println!("Using UDL file: {:?}", udl_file);

    // Generate Kotlin bindings
    let kotlin_out = root_dir.join("android/app/src/main/java");
    fs::create_dir_all(&kotlin_out).unwrap();
    println!("Generating Kotlin bindings to {:?}", kotlin_out);

    uniffi_bindgen::generate_bindings(
        &udl_file,
        None, // config file
        KotlinBindingGenerator,
        Some(kotlin_out.as_ref()),
        None, // lib file
        None, // crate name
        false, // try_format_code
    ).unwrap();

    println!("✓ Kotlin bindings generated");

    // Generate Swift bindings
    let swift_out = root_dir.join("ios/BenchRunner/BenchRunner/Generated");
    fs::create_dir_all(&swift_out).unwrap();
    println!("Generating Swift bindings to {:?}", swift_out);

    uniffi_bindgen::generate_bindings(
        &udl_file,
        None, // config file
        SwiftBindingGenerator,
        Some(swift_out.as_ref()),
        None, // lib file
        None, // crate name
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
