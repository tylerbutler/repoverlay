//! CLI entry point for repoverlay.

fn main() {
    if let Err(e) = repoverlay::run() {
        eprintln!("Error: {e:?}");
        std::process::exit(1);
    }
}
