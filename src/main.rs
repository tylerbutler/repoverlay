//! CLI entry point for repoverlay.

fn main() {
    // Reset SIGPIPE to default so piped commands exit cleanly.
    // Rust's runtime masks SIGPIPE, causing "Broken pipe" errors
    // when output is piped to commands like `head`.
    #[cfg(unix)]
    // SAFETY: libc::signal is safe to call before any I/O. We reset SIGPIPE
    // to the OS default (terminate) which Rust's runtime masks. This is the
    // standard fix for CLI tools that pipe output.
    #[allow(unsafe_code)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    env_logger::init();

    if let Err(e) = repoverlay::run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
