use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::ExitStatus;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use crate::profile::ProfileMode;
use crate::profile_applicators::AgentHarness;

struct ProfileRunLock {
    path: PathBuf,
}

impl Drop for ProfileRunLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn handle_copilot_command(
    profiles: &[String],
    target: Option<PathBuf>,
    extra_args: Vec<String>,
) -> Result<()> {
    run_ephemeral_profiles(AgentHarness::Copilot, profiles, target, extra_args)
}

/// Claim the ephemeral session lock for `(profile, harness)`.
///
/// Recovers a lock orphaned by a dead session (SIGKILL/power loss) before trying
/// to claim it; a live lock is left in place so `create_new` rejects us.
fn claim_profile_lock(
    target: &Path,
    profile: &str,
    harness: AgentHarness,
) -> Result<ProfileRunLock> {
    let lock_path = crate::profile::profile_lock_path(target, profile, harness)?;
    let lock_parent = lock_path
        .parent()
        .context("Profile lock file has no parent directory")?;
    std::fs::create_dir_all(lock_parent).with_context(|| {
        format!(
            "Failed to create profile lock directory: {}",
            lock_parent.display()
        )
    })?;
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
            // Own the file via the guard immediately, so a failed PID write
            // doesn't orphan the lock file (Drop removes it on the `?`).
            let guard = ProfileRunLock {
                path: lock_path.clone(),
            };
            writeln!(file, "{}", std::process::id()).with_context(|| {
                format!("Failed to write profile lock: {}", lock_path.display())
            })?;
            Ok(guard)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!(
                "Profile '{profile}' is already applied or running for {harness}; remove it \
                 before running an ephemeral session"
            );
        }
        Err(error) => Err(error)
            .with_context(|| format!("Failed to create profile lock: {}", lock_path.display())),
    }
}

/// Remove every ephemerally-applied profile, in reverse application order.
///
/// All removals are attempted even if one fails; the first error is returned so
/// the caller can surface it.
fn rollback_applied(applied: &[String], harness: AgentHarness, target: &Path) -> Result<()> {
    let mut first_err: Option<anyhow::Error> = None;
    for profile in applied.iter().rev() {
        if let Err(error) =
            crate::profile_plan::remove_profile_for_session(profile, harness, target)
            && first_err.is_none()
        {
            first_err = Some(error);
        }
    }
    first_err.map_or(Ok(()), Err)
}

/// Apply one or more profiles ephemerally, run the agent harness for the
/// lifetime of the session, then tear the profiles down again.
///
/// Each profile gets its own session lock and ephemeral apply; bundle plugins
/// resolved for ephemeral use are aggregated and loaded natively via
/// `--plugin-dir` (deduplicated, nothing placed on disk). A failure while
/// applying any profile rolls back the ones already applied so the repository is
/// never left half-configured.
pub(crate) fn run_ephemeral_profiles(
    harness: AgentHarness,
    profiles: &[String],
    target: Option<PathBuf>,
    extra_args: Vec<String>,
) -> Result<()> {
    let label = harness.label();
    let target = target.unwrap_or_else(|| PathBuf::from("."));
    let target = crate::canonicalize_path(&target, "Target")?;
    crate::validate_git_repo(&target)?;

    // Reject duplicate names up front; otherwise the second apply of the same
    // profile would fail with the less obvious "already applied" error.
    let mut seen = BTreeSet::new();
    for profile in profiles {
        if !seen.insert(profile.as_str()) {
            bail!("Profile '{profile}' was specified more than once");
        }
    }

    // Refuse before mutating anything if any requested profile is already
    // persistently applied for this harness.
    for profile in profiles {
        let state_path = crate::profile::profile_state_path(&target, profile, harness)?;
        if state_path
            .try_exists()
            .with_context(|| format!("Failed to inspect profile state: {}", state_path.display()))?
        {
            bail!(
                "Profile '{profile}' is already applied for {harness}; remove it before running \
                 an ephemeral session"
            );
        }
    }

    // Locks are held (and their files removed on drop) for the whole session.
    let mut locks: Vec<ProfileRunLock> = Vec::new();
    let mut applied: Vec<String> = Vec::new();
    let mut plugin_dirs: Vec<PathBuf> = Vec::new();

    let apply_outcome = (|| -> Result<()> {
        for profile in profiles {
            // Claim the lock before applying; pushing it first means a failed
            // apply still releases the lock when `locks` is dropped.
            locks.push(claim_profile_lock(&target, profile, harness)?);
            let session_id = format!(
                "{}-{}-{}",
                chrono::Utc::now().format("%Y%m%d%H%M%S"),
                harness,
                profile
            );
            let state = crate::profile_plan::apply_profile(
                profile,
                harness,
                &target,
                ProfileMode::Ephemeral,
                Some(session_id),
            )?;
            applied.push(profile.clone());
            for dir in state.plugin_dirs {
                if !plugin_dirs.contains(&dir) {
                    plugin_dirs.push(dir);
                }
            }
        }
        Ok(())
    })();

    if let Err(apply_error) = apply_outcome {
        let cleanup = rollback_applied(&applied, harness, &target);
        drop(locks);
        if let Err(cleanup_error) = cleanup {
            bail!(
                "Failed to apply profiles: {apply_error}; rolling back already-applied profiles \
                 also failed: {cleanup_error}"
            );
        }
        return Err(apply_error);
    }

    let mut command = Command::new(harness.program());
    for dir in &plugin_dirs {
        command.arg("--plugin-dir").arg(dir);
    }
    command.args(&extra_args);
    drop(extra_args);
    command.current_dir(&target);

    let mut child = match crate::harness_process::HarnessProcess::spawn(command) {
        Ok(child) => child,
        Err(spawn_error) => {
            let cleanup = rollback_applied(&applied, harness, &target);
            drop(locks);
            if let Err(cleanup_error) = cleanup {
                bail!(
                    "Failed to run {label} harness: {spawn_error}; profile cleanup also failed: \
                     {cleanup_error}"
                );
            }
            return Err(spawn_error).context(format!("Failed to run {label} harness"));
        }
    };
    crate::git::register_child_pid(child.id());
    // The child runs in its own process group; give it the controlling terminal
    // while it runs so interactive harnesses stay in the foreground.
    #[cfg(unix)]
    let terminal_foreground =
        crate::harness_process::TerminalForeground::acquire(child.process_group_id());
    let status_result = wait_for_harness(&mut child, label);
    // Restore foreground ownership before profile cleanup runs.
    #[cfg(unix)]
    drop(terminal_foreground);
    crate::git::unregister_child();

    let cleanup_result = rollback_applied(&applied, harness, &target);
    if let Err(error) = cleanup_result {
        bail!("{label} exited, but profile cleanup failed: {error}");
    }

    let status = status_result?;
    let exit_code = exit_code_from_status(status);
    drop(locks);
    std::process::exit(exit_code);
}

pub(crate) fn wait_for_harness(
    child: &mut crate::harness_process::HarnessProcess,
    label: &str,
) -> Result<ExitStatus> {
    let mut forwarded_interrupt = false;
    loop {
        // If interrupted, terminate the whole child process group before we
        // accept any child exit as final, so harness descendants are signaled
        // before profile cleanup runs.
        if crate::git::is_interrupted() && !forwarded_interrupt {
            child.terminate();
            forwarded_interrupt = true;
        }

        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("Failed to wait for {label} harness"))?
        {
            // Not redundant with the top-of-loop check: this covers the race where
            // an interrupt arrives between that check and `try_wait` returning the
            // parent's exit, so the process group is still signaled before cleanup
            // even if descendants outlived the parent.
            if crate::git::is_interrupted() && !forwarded_interrupt {
                child.terminate();
            }
            return Ok(status);
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn exit_code_from_status(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    if let Some(signal) = status.signal() {
        return 128 + signal;
    }

    1
}
