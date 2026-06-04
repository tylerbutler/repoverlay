use anyhow::Result;
use std::path::PathBuf;

use super::copilot::run_ephemeral_profiles;

/// Run Claude with one or more profiles applied for the process lifetime
/// (ephemeral).
///
/// Bundle plugins are loaded natively via `--plugin-dir` (nothing placed on
/// disk); overlays and delegate-settings enablement still flow through the
/// regular apply/remove machinery so they are cleaned up when Claude exits.
pub(crate) fn handle_claude_command(
    profiles: &[String],
    target: Option<PathBuf>,
    extra_args: Vec<String>,
) -> Result<()> {
    run_ephemeral_profiles(
        "claude",
        "Claude",
        "REPOVERLAY_CLAUDE_COMMAND",
        "claude",
        profiles,
        target,
        extra_args,
    )
}
