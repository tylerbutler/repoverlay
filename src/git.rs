//! Git subprocess helpers with progress indication and Ctrl+C handling.

use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};

/// Global flag set by the Ctrl+C handler.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Install a global Ctrl+C handler that sets the interrupted flag.
///
/// Must be called once at startup. Subsequent Ctrl+C presses will set the flag
/// and, if a child process is registered, kill it.
pub(crate) fn install_ctrlc_handler() {
    // Shared child PID that the handler can kill.
    let child_pid: Arc<std::sync::Mutex<Option<u32>>> = Arc::new(std::sync::Mutex::new(None));
    // Store globally so `register_child` / `unregister_child` can access it.
    CHILD_PID.set(child_pid.clone()).ok();

    ctrlc::set_handler(move || {
        INTERRUPTED.store(true, Ordering::SeqCst);

        // Kill the active child process if there is one.
        if let Ok(guard) = child_pid.lock()
            && let Some(pid) = *guard
        {
            #[cfg(unix)]
            {
                // Send SIGTERM to the child process.
                #[allow(unsafe_code, clippy::cast_possible_wrap)]
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
            }
        }
    })
    .expect("Failed to set Ctrl+C handler");
}

/// Check whether the user has pressed Ctrl+C.
pub(crate) fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

// Global storage for the child PID so the ctrlc handler can kill it.
static CHILD_PID: std::sync::OnceLock<Arc<std::sync::Mutex<Option<u32>>>> =
    std::sync::OnceLock::new();

/// Register a child process so Ctrl+C will kill it.
fn register_child(child: &Child) {
    if let Some(pid_lock) = CHILD_PID.get()
        && let Ok(mut guard) = pid_lock.lock()
    {
        *guard = Some(child.id());
    }
}

/// Unregister the child process after it exits.
fn unregister_child() {
    if let Some(pid_lock) = CHILD_PID.get()
        && let Ok(mut guard) = pid_lock.lock()
    {
        *guard = None;
    }
}

/// Run a git command with a spinner and Ctrl+C propagation.
///
/// The child process inherits stderr so git progress output (e.g. clone
/// percentages) is visible. Stdout is captured for commands that return data,
/// or inherited for commands where we only care about the exit status.
///
/// Returns `(ExitStatus, captured_stdout)`. If `capture_stdout` is false the
/// returned Vec is empty.
pub(crate) fn run_git_with_spinner(
    args: &[&str],
    working_dir: Option<&std::path::Path>,
    message: &str,
    capture_stdout: bool,
) -> Result<(ExitStatus, Vec<u8>)> {
    let mut cmd = Command::new("git");
    cmd.args(args);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    // Always let git write progress to stderr (clone/fetch percentages).
    cmd.stderr(Stdio::inherit());

    if capture_stdout {
        cmd.stdout(Stdio::piped());
    } else {
        cmd.stdout(Stdio::inherit());
    }

    let mut child = cmd.spawn().context("Failed to execute git")?;
    register_child(&child);

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.blue} {msg}")
            .expect("valid template")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(80));

    // Poll the child until it exits or Ctrl+C is received.
    let status = loop {
        if let Some(status) = child.try_wait().context("Failed to wait on git process")? {
            break status;
        }
        if is_interrupted() {
            let _ = child.kill();
            let _ = child.wait();
            unregister_child();
            spinner.finish_and_clear();
            bail!("Interrupted");
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    unregister_child();
    spinner.finish_and_clear();

    let stdout_bytes = if capture_stdout {
        // Child stdout was piped — read it now that the process has exited.
        use std::io::Read;
        let mut buf = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            out.read_to_end(&mut buf)?;
        }
        buf
    } else {
        Vec::new()
    };

    Ok((status, stdout_bytes))
}
