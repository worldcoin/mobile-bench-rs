#[cfg(feature = "bindgen")]
mod bindgen {
    use camino::Utf8PathBuf;
    use std::env;
    use std::fs;
    use uniffi_bindgen::bindings::{KotlinBindingGenerator, SwiftBindingGenerator};
    use uniffi_bindgen::library_mode::generate_bindings;

    pub fn run() {
        let manifest_dir = Utf8PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let root_dir = manifest_dir.parent().unwrap().parent().unwrap();

        let lib_file = if let Ok(path) = env::var("UNIFFI_LIBRARY_PATH") {
            println!("Using UniFFI library from UNIFFI_LIBRARY_PATH");
            Utf8PathBuf::from(path)
        } else {
            let profile = env::var("UNIFFI_PROFILE").unwrap_or_else(|_| "release".to_string());
            println!(
                "Building library to generate UniFFI metadata (profile={})...",
                profile
            );
            let target_dir = root_dir.join("target").join(&profile);
            let lib_name = if cfg!(target_os = "macos") {
                "libsample_fns.dylib"
            } else if cfg!(target_os = "linux") {
                "libsample_fns.so"
            } else {
                "sample_fns.dll"
            };
            target_dir.join(lib_name)
        };

        println!("Using library: {:?}", lib_file);
        if !lib_file.exists() {
            eprintln!(
                "UniFFI library not found at {:?}. Build it first or set UNIFFI_LIBRARY_PATH.",
                lib_file
            );
            std::process::exit(1);
        }

        let kotlin_out = root_dir.join("android/app/src/main/java");
        fs::create_dir_all(&kotlin_out).unwrap();
        println!("Generating Kotlin bindings to {:?}", kotlin_out);

        generate_bindings(
            &lib_file,
            None,
            &KotlinBindingGenerator,
            &uniffi_bindgen::cargo_metadata::CrateConfigSupplier::default(),
            None,
            &kotlin_out,
            false,
        )
        .unwrap();

        println!("✓ Kotlin bindings generated");

        let swift_out = root_dir.join("ios/BenchRunner/BenchRunner/Generated");
        fs::create_dir_all(&swift_out).unwrap();
        println!("Generating Swift bindings to {:?}", swift_out);

        generate_bindings(
            &lib_file,
            None,
            &SwiftBindingGenerator,
            &uniffi_bindgen::cargo_metadata::CrateConfigSupplier::default(),
            None,
            &swift_out,
            false,
        )
        .unwrap();

        println!("✓ Swift bindings generated");
        println!("\n✓ All bindings generated successfully");

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
}

#[cfg(feature = "bindgen")]
fn main() {
    bindgen::run();
}

#[cfg(not(feature = "bindgen"))]
fn main() {
    eprintln!("generate-bindings requires --features bindgen");
    std::process::exit(1);
}
