fn main() {
    let mut args = std::env::args_os().collect::<Vec<_>>();

    // Cargo invokes subcommands as `cargo-mobench mobench <args>`. The injected
    // `mobench` token is Cargo's subcommand name, not a mobench subcommand.
    // Keep accepting a direct `cargo-mobench <args>` invocation as well.
    if args
        .get(1)
        .is_some_and(|arg| arg == std::ffi::OsStr::new("mobench"))
    {
        args.remove(1);
    }

    if let Err(err) = mobench::run_from(args) {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
