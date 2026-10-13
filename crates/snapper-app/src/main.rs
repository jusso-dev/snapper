fn main() {
    let verbose = std::env::args().any(|arg| arg == "--verbose" || arg == "-v");
    let _log = snapper_core::logging::init("snapper-app", verbose);
    println!(
        "snapper {} (skeleton, GUI lands in M1)",
        env!("CARGO_PKG_VERSION")
    );
}
