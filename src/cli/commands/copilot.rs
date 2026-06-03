use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use crate::profile::ProfileMode;
use crate::profile_applicators::ProfileApplicator;

struct ProfileRunLock {
    path: PathBuf,
}

impl Drop for ProfileRunLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn handle_copilot_command(
    profile: String,
    target: Option<PathBuf>,
    extra_args: Vec<String>,
) -> Result<()> {
    let target = target.unwrap_or_else(|| PathBuf::from("."));
    let target = crate::canonicalize_path(&target, "Target")?;
    crate::validate_git_repo(&target)?;

    let state_path = crate::profile::profile_state_path(&target, &profile, "copilot")?;
    if state_path
        .try_exists()
        .with_context(|| format!("Failed to inspect profile state: {}", state_path.display()))?
    {
        bail!(
            "Profile '{profile}' is already applied for copilot; remove it before running an \
             ephemeral session"
        );
    }

    let lock_path = crate::profile::profile_lock_path(&target, &profile, "copilot")?;
    let lock_parent = lock_path
        .parent()
        .context("Profile lock file has no parent directory")?;
    std::fs::create_dir_all(lock_parent).with_context(|| {
        format!(
            "Failed to create profile lock directory: {}",
            lock_parent.display()
        )
    })?;
    // Recover a lock orphaned by a dead session (SIGKILL/power loss) before trying
    // to claim it; a live lock is left in place so `create_new` rejects us below.
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
                "Profile '{profile}' is already applied or running for copilot; remove it before \
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
        "{}-copilot-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        profile
    );
    let state = crate::profile_plan::apply_profile(
        &profile,
        "copilot",
        &target,
        ProfileMode::Ephemeral,
        Some(session_id),
    )?;

    let config = crate::config::load_config(Some(&target))?;
    config
        .profiles
        .get(&profile)
        .context("Profile disappeared after apply")?;

    let context = crate::profile_applicators::ProfileContext {
        profile_name: profile,
        target: target.clone(),
        profile_asset_dir: target.clone(),
        harness_home:
            crate::profile_applicators::copilot::CopilotApplicator::harness_home_from_env()?,
        mode: ProfileMode::Ephemeral,
        session_id: state.session_id,
    };
    let applicator = crate::profile_applicators::copilot::CopilotApplicator;
    let mut command = applicator.command(&context, &extra_args)?;
    drop(extra_args);
    command.current_dir(&target);

    let mut child = match crate::harness_process::HarnessProcess::spawn(command) {
        Ok(child) => child,
        Err(spawn_error) => {
            if let Err(cleanup_error) = crate::profile_plan::remove_profile_for_session(
                &context.profile_name,
                "copilot",
                &target,
            ) {
                bail!(
                    "Failed to run Copilot harness: {spawn_error}; profile cleanup also failed: \
                     {cleanup_error}"
                );
            }
            return Err(spawn_error).context("Failed to run Copilot harness");
        }
    };
    crate::git::register_child_pid(child.id());
    // The child runs in its own process group; give it the controlling terminal
    // while it runs so interactive harnesses stay in the foreground.
    #[cfg(unix)]
    let terminal_foreground =
        crate::harness_process::TerminalForeground::acquire(child.process_group_id());
    let status_result = wait_for_copilot_harness(&mut child);
    // Restore foreground ownership before profile cleanup runs.
    #[cfg(unix)]
    drop(terminal_foreground);
    crate::git::unregister_child();

    let cleanup_result =
        crate::profile_plan::remove_profile_for_session(&context.profile_name, "copilot", &target);
    if let Err(error) = cleanup_result {
        bail!("Copilot exited, but profile cleanup failed: {error}");
    }

    let status = status_result?;
    let exit_code = exit_code_from_status(status);
    drop(profile_run_lock);
    std::process::exit(exit_code);
}

fn wait_for_copilot_harness(
    child: &mut crate::harness_process::HarnessProcess,
) -> Result<ExitStatus> {
    let mut forwarded_interrupt = false;
    loop {
        // If interrupted, terminate the whole child process group before we
        // accept any child exit as final, so Copilot descendants are signaled
        // before profile cleanup runs.
        if crate::git::is_interrupted() && !forwarded_interrupt {
            child.terminate();
            forwarded_interrupt = true;
        }

        if let Some(status) = child
            .try_wait()
            .context("Failed to wait for Copilot harness")?
        {
            if crate::git::is_interrupted() && !forwarded_interrupt {
                child.terminate();
            }
            return Ok(status);
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

fn exit_code_from_status(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    if let Some(signal) = status.signal() {
        return 128 + signal;
    }

    1
}

