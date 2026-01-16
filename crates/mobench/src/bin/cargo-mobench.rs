fn main() {
    if let Err(err) = mobench::run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
