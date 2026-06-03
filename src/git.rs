//! Git subprocess helpers, repository inspection, and Ctrl+C handling.

#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
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
/// Register a child process so Ctrl+C will kill it.
pub(crate) fn register_child(child: &Child) {
    register_child_pid(child.id());
}

/// Register a child PID so Ctrl+C will kill it.
pub(crate) fn register_child_pid(pid: u32) {
    if let Some(pid_lock) = CHILD_PID.get()
        && let Ok(mut guard) = pid_lock.lock()
    {
        *guard = Some(pid);
    }
}

/// Unregister the child process after it exits.
pub(crate) fn unregister_child() {
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

/// Validate that a path is a git repository (has a `.git` directory or file).
pub(crate) fn validate_git_repo(path: &Path) -> Result<()> {
    if !path.join(".git").exists() {
        bail!("Target is not a git repository: {}", path.display());
    }
    Ok(())
}

/// Resolve the path to `.git/info/exclude` for a repository.
///
/// Uses `git rev-parse --git-path` which correctly resolves to the common git
/// directory for worktrees (git reads `info/exclude` from the shared `.git/`,
/// not from the worktree-specific `$GIT_DIR`).
pub(crate) fn resolve_git_exclude_path(repo_path: &Path) -> Result<PathBuf> {
    let output = process::Command::new("git")
        .args(["rev-parse", "--git-path", "info/exclude"])
        .current_dir(repo_path)
        .output()
        .context("Failed to run git rev-parse --git-path")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to resolve git exclude path in {}: {}",
            repo_path.display(),
            stderr.trim()
        );
    }

    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let path = PathBuf::from(&path_str);

    // Handle relative paths (relative to repo_path)
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(repo_path.join(path))
    }
}

/// Resolve the actual git directory for a repository.
///
/// In a regular git repository, `.git` is a directory containing the git database.
/// In a git worktree, `.git` is a file containing `gitdir: /path/to/git/dir`.
/// This function handles both cases and returns the path to the actual git directory.
///
/// Note: For `info/exclude` paths, use [`resolve_git_exclude_path`] instead,
/// which correctly resolves to the common git directory for worktrees.
#[cfg(test)]
pub(crate) fn resolve_git_dir(repo_path: &Path) -> Result<PathBuf> {
    let git_path = repo_path.join(".git");

    if git_path.is_dir() {
        // Regular git repository
        return Ok(git_path);
    }

    if git_path.is_file() {
        // Git worktree - .git is a file containing "gitdir: /path/to/git/dir"
        let content = fs::read_to_string(&git_path)
            .with_context(|| format!("Failed to read .git file: {}", git_path.display()))?;

        for line in content.lines() {
            let line = line.trim();
            if let Some(path_str) = line.strip_prefix("gitdir:") {
                let path_str = path_str.trim();
                let gitdir = PathBuf::from(path_str);

                // Handle relative paths (relative to repo_path)
                let gitdir = if gitdir.is_absolute() {
                    gitdir
                } else {
                    repo_path.join(gitdir)
                };

                return gitdir.canonicalize().with_context(|| {
                    format!("Failed to resolve gitdir path: {}", gitdir.display())
                });
            }
        }

        bail!(
            "Invalid .git file (no gitdir found): {}",
            git_path.display()
        );
    }

    bail!("Not a git repository: {}", repo_path.display());
}
