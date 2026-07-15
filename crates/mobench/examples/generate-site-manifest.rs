use std::path::PathBuf;

fn main() {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("mobench-site-manifest-v1.json"));

    if let Err(error) = mobench::site_manifest::write_to_path(&output) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
    println!("wrote {}", output.display());
}
