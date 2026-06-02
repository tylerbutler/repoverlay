use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::ExitStatus;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use crate::profile::ProfileMode;
use crate::profile_applicators::ProfileApplicator;

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

    let status_result = command.status().context("Failed to run Copilot harness");
    let cleanup_result =
        crate::profile_plan::remove_profile(&context.profile_name, "copilot", &target);
    if let Err(error) = cleanup_result {
        bail!("Copilot exited, but profile cleanup failed: {error}");
    }

    let status = status_result?;
    std::process::exit(exit_code_from_status(status));
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
