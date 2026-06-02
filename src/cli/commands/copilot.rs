use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::{Child, ExitStatus};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt;
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

/// Decide whether terminal foreground ownership should be handed to the child.
///
/// Handoff is only safe when stdin is a controlling terminal. When stdin is a
/// pipe or file (tests, CI, non-interactive use) there is no foreground process
/// group to manage, so handoff is disabled.
#[cfg(unix)]
const fn should_hand_off_terminal(stdin_is_tty: bool) -> bool {
    stdin_is_tty
}

/// RAII guard that transfers terminal foreground ownership to a child process
/// group and restores the previous foreground group on drop.
///
/// Interactive harnesses that read from the terminal must be in the foreground
/// process group, otherwise they receive `SIGTTIN`/`SIGTTOU` and hang. Because
/// the child is spawned in its own process group, we explicitly hand the
/// controlling terminal over with `tcsetpgrp` while it runs.
#[cfg(unix)]
struct TerminalForeground {
    fd: libc::c_int,
    previous_pgid: libc::pid_t,
}

#[cfg(unix)]
impl TerminalForeground {
    /// Hand terminal foreground ownership to `child_pgid` when stdin is a
    /// controlling terminal. Returns `None` when no TTY is present or the
    /// handoff could not be performed, in which case behavior is unchanged.
    fn acquire(child_pgid: libc::pid_t) -> Option<Self> {
        let fd = libc::STDIN_FILENO;
        #[allow(unsafe_code)]
        let stdin_is_tty = unsafe { libc::isatty(fd) == 1 };
        if !should_hand_off_terminal(stdin_is_tty) {
            return None;
        }

        #[allow(unsafe_code)]
        unsafe {
            let previous_pgid = libc::tcgetpgrp(fd);
            if previous_pgid < 0 {
                return None;
            }
            // Calling tcsetpgrp from a background group raises SIGTTOU; ignore it
            // around the call so we are not stopped while reassigning ownership.
            let previous_handler = libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            let result = libc::tcsetpgrp(fd, child_pgid);
            libc::signal(libc::SIGTTOU, previous_handler);
            if result != 0 {
                return None;
            }
            Some(Self { fd, previous_pgid })
        }
    }
}

#[cfg(unix)]
impl Drop for TerminalForeground {
    fn drop(&mut self) {
        #[allow(unsafe_code)]
        unsafe {
            let previous_handler = libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            libc::tcsetpgrp(self.fd, self.previous_pgid);
            libc::signal(libc::SIGTTOU, previous_handler);
        }
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
    #[cfg(unix)]
    command.process_group(0);

    let mut child = match command.spawn() {
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
    crate::git::register_child(&child);
    // The child runs in its own process group; give it the controlling terminal
    // while it runs so interactive harnesses stay in the foreground.
    #[cfg(unix)]
    let terminal_foreground = {
        #[allow(clippy::cast_possible_wrap)]
        let child_pgid = child.id() as libc::pid_t;
        TerminalForeground::acquire(child_pgid)
    };
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

fn wait_for_copilot_harness(child: &mut Child) -> Result<ExitStatus> {
    let mut forwarded_interrupt = false;
    loop {
        // If interrupted, terminate the whole child process group before we
        // accept any child exit as final, so Copilot descendants are signaled
        // before profile cleanup runs.
        if crate::git::is_interrupted() && !forwarded_interrupt {
            terminate_copilot_harness(child);
            forwarded_interrupt = true;
        }

        if let Some(status) = child
            .try_wait()
            .context("Failed to wait for Copilot harness")?
        {
            if crate::git::is_interrupted() && !forwarded_interrupt {
                terminate_copilot_harness(child);
            }
            return Ok(status);
        }

        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
fn terminate_copilot_harness(child: &mut Child) {
    #[allow(unsafe_code, clippy::cast_possible_wrap)]
    unsafe {
        if libc::kill(-(child.id() as libc::pid_t), libc::SIGTERM) == 0 {
            return;
        }
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn terminate_copilot_harness(child: &mut Child) {
    let _ = child.kill();
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

#[cfg(all(test, unix))]
mod tests {
    use super::should_hand_off_terminal;

    #[test]
    fn terminal_handoff_disabled_without_tty() {
        assert!(!should_hand_off_terminal(false));
    }

    #[test]
    fn terminal_handoff_enabled_with_tty() {
        assert!(should_hand_off_terminal(true));
    }
}
