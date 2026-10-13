fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");
    let print_log_dir = args.iter().any(|arg| arg == "--print-log-dir");

    let _log = snapper_core::logging::init("snapper-cli", verbose);
    if print_log_dir {
        println!("{}", snapper_core::logging::log_dir().display());
        return;
    }
    // Validation hook for the crash report. Not a user-facing command.
    if std::env::var("SNAPPER_TEST_PANIC").ok().as_deref() == Some("1") {
        panic!("SNAPPER_TEST_PANIC");
    }

    println!("snapper-cli {} (skeleton)", env!("CARGO_PKG_VERSION"));
}
