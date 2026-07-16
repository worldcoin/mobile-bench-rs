fn main() {
    if let Err(err) = mobench::install_interrupt_handler() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
    if let Err(err) = mobench::run() {
        eprintln!("{err:#}");
        std::process::exit(if mobench::interruption_requested() {
            130
        } else {
            1
        });
    }
}
