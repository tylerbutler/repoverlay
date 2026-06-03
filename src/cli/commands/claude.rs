use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

use super::copilot::{exit_code_from_status, wait_for_harness};
use crate::profile::ProfileMode;

struct ProfileRunLock {
    path: PathBuf,
}

impl Drop for ProfileRunLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Run Claude with a profile applied for the process lifetime (ephemeral).
///
/// Bundle plugins are loaded natively via `--plugin-dir` (nothing placed on
/// disk); overlays and delegate-settings enablement still flow through the
/// regular apply/remove machinery so they are cleaned up when Claude exits.
pub(crate) fn handle_claude_command(
    profile: &str,
    target: Option<PathBuf>,
    extra_args: Vec<String>,
) -> Result<()> {
    let target = target.unwrap_or_else(|| PathBuf::from("."));
    let target = crate::canonicalize_path(&target, "Target")?;
    crate::validate_git_repo(&target)?;

    let state_path = crate::profile::profile_state_path(&target, profile, "claude")?;
    if state_path
        .try_exists()
        .with_context(|| format!("Failed to inspect profile state: {}", state_path.display()))?
    {
        bail!(
            "Profile '{profile}' is already applied for claude; remove it before running an \
             ephemeral session"
        );
    }

    let lock_path = crate::profile::profile_lock_path(&target, profile, "claude")?;
    let lock_parent = lock_path
        .parent()
        .context("Profile lock file has no parent directory")?;
    std::fs::create_dir_all(lock_parent).with_context(|| {
        format!(
            "Failed to create profile lock directory: {}",
            lock_parent.display()
        )
    })?;
    // Recover a lock orphaned by a dead session before trying to claim it.
    if crate::profile::inspect_lock(&lock_path)? == crate::profile::LockState::Stale {
        std::fs::remove_file(&lock_path).with_context(|| {
            format!(
                "Failed to remove stale profile lock: {}",
                lock_path.display()
            )
        })?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(mut file) => {
            use std::io::Write;
            writeln!(file, "{}", std::process::id()).with_context(|| {
                format!("Failed to write profile lock: {}", lock_path.display())
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!(
                "Profile '{profile}' is already applied or running for claude; remove it before \
                 running an ephemeral session"
            );
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Failed to create profile lock: {}", lock_path.display())
            });
        }
    }
    let profile_run_lock = ProfileRunLock { path: lock_path };

    let session_id = format!(
        "{}-claude-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        profile
    );
    let state = crate::profile_plan::apply_profile(
        profile,
        "claude",
        &target,
        ProfileMode::Ephemeral,
        Some(session_id),
    )?;

    let program =
        std::env::var("REPOVERLAY_CLAUDE_COMMAND").unwrap_or_else(|_| "claude".to_string());
    let mut command = Command::new(program);
    for dir in &state.plugin_dirs {
        command.arg("--plugin-dir").arg(dir);
    }
    command.args(&extra_args);
    drop(extra_args);
    command.current_dir(&target);

    let mut child = match crate::harness_process::HarnessProcess::spawn(command) {
        Ok(child) => child,
        Err(spawn_error) => {
            if let Err(cleanup_error) =
                crate::profile_plan::remove_profile_for_session(profile, "claude", &target)
            {
                bail!(
                    "Failed to run Claude harness: {spawn_error}; profile cleanup also failed: \
                     {cleanup_error}"
                );
            }
            return Err(spawn_error).context("Failed to run Claude harness");
        }
    };
    crate::git::register_child_pid(child.id());
    #[cfg(unix)]
    let terminal_foreground =
        crate::harness_process::TerminalForeground::acquire(child.process_group_id());
    let status_result = wait_for_harness(&mut child, "Claude");
    #[cfg(unix)]
    drop(terminal_foreground);
    crate::git::unregister_child();

    let cleanup_result =
        crate::profile_plan::remove_profile_for_session(profile, "claude", &target);
    if let Err(error) = cleanup_result {
        bail!("Claude exited, but profile cleanup failed: {error}");
    }

    let status = status_result?;
    let exit_code = exit_code_from_status(status);
    drop(profile_run_lock);
    std::process::exit(exit_code);
}
