//! Switch to a different overlay set atomically.

use anyhow::Result;
use colored::Colorize;
use std::fs;
use std::path::Path;

use crate::ApplyOptions;
use crate::apply_overlay;
use crate::remove_overlay;
use crate::state::{OVERLAYS_DIR, STATE_DIR};
use crate::validate_git_repo;

/// Switch to a different overlay by removing all existing overlays first.
///
/// Atomic replacement of all overlays - useful for switching between different
/// configurations (e.g., different AI agent setups).
///
/// # Workflow
///
/// 1. Remove all existing overlays (if any)
/// 2. Apply the new overlay
pub(crate) fn switch_overlay(source: &str, target: &Path, options: &ApplyOptions) -> Result<()> {
    validate_git_repo(target)?;

    // Check if any overlays are currently applied
    let state_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    let has_overlays = state_dir.exists() && fs::read_dir(&state_dir)?.next().is_some();

    if has_overlays {
        println!("{} existing overlays...", "Removing".yellow().bold());
        // Remove all existing overlays
        remove_overlay(target, None, true, options.dry_run)?;
    }

    // Apply the new overlay
    println!("{} new overlay...", "Applying".blue().bold());
    apply_overlay(source, target, options)?;

    Ok(())
}
