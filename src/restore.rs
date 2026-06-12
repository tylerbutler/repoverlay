//! Restore previously applied overlays from external state backups.

use anyhow::{Result, bail};
use colored::Colorize;
use log::debug;
use std::path::Path;

use crate::ApplyOptions;
use crate::ConflictStrategy;
use crate::apply_overlay;
use crate::canonicalize_path;
use crate::state::{OverlaySource, load_external_states};
use crate::validate_git_repo;

/// Restore overlays after git clean or other removal.
///
/// Uses external state backup (`~/.local/share/repoverlay/applied/`) to recover
/// overlays that were removed by `git clean -fdx` or similar operations.
///
/// # Workflow
///
/// 1. Load external state backup for the target repository
/// 2. For each saved overlay state, re-apply using original source
pub(crate) fn restore_overlays(
    target: &Path,
    dry_run: bool,
    conflict_strategy: ConflictStrategy,
    merge: bool,
) -> Result<()> {
    debug!(
        "restore_overlays: target={}, dry_run={}, conflict_strategy={:?}",
        target.display(),
        dry_run,
        conflict_strategy
    );
    let target = canonicalize_path(target, "Target directory")?;
    validate_git_repo(&target)?;

    // Load external state
    let external_states = load_external_states(&target)?;
    debug!("found {} external states to restore", external_states.len());

    let restorable_profiles = crate::profile_plan::list_restorable_profiles(&target)?;
    debug!("found {} profile(s) to restore", restorable_profiles.len());

    if external_states.is_empty() && restorable_profiles.is_empty() {
        println!("{} Nothing to restore.", "Status:".bold());
        println!("  No external backup found for this repository.");
        return Ok(());
    }

    if !external_states.is_empty() {
        println!(
            "{} {} overlay(s) to restore:",
            "Found".blue().bold(),
            external_states.len()
        );

        for state in &external_states {
            println!("  - {}", state.name);
            match &state.source {
                OverlaySource::Local { path, .. } => {
                    println!("    Source: {}", path.display());
                }
                OverlaySource::GitHub { url, git_ref, .. } => {
                    println!("    Source: {url} ({git_ref})");
                }
                OverlaySource::OverlayRepo {
                    org,
                    repo,
                    name: overlay_name,
                    ..
                } => {
                    println!("    Source: {org}/{repo}/{overlay_name} (overlay repo)");
                }
                OverlaySource::Library { name } => {
                    println!("    Source: {name} (library)");
                }
            }
        }
    }

    if !restorable_profiles.is_empty() {
        println!(
            "{} {} profile(s) to restore:",
            "Found".blue().bold(),
            restorable_profiles.len()
        );
        for profile in &restorable_profiles {
            println!("  - {} ({})", profile.name, profile.harness);
        }
    }

    if dry_run {
        println!("\n{} Dry run - no changes made.", "Note:".yellow());
        return Ok(());
    }

    println!();

    // Restore each overlay
    let mut restored = Vec::new();
    let mut failures = Vec::new();

    for state in external_states {
        let (source_str, ref_override) = state.source.reapply_reference();
        let ref_override = ref_override.map(str::to_string);

        // Re-apply the overlay. Always use Force since restore's purpose is to
        // re-create missing/broken symlinks from external backup state.
        match apply_overlay(
            &source_str,
            &target,
            &ApplyOptions {
                name_override: Some(state.name.clone()),
                ref_override,
                update_cache: true,
                conflict_strategy: ConflictStrategy::Force,
                merge,
                ..ApplyOptions::default()
            },
        ) {
            Ok(()) => {
                println!("  {} Restored '{}'", "✓".green(), state.name);
                restored.push(state.name.clone());
            }
            Err(e) => {
                let error = e.to_string();
                eprintln!(
                    "  {} Failed to restore '{}': {}",
                    "Error:".red(),
                    state.name,
                    error
                );
                failures.push((state.name.clone(), error));
            }
        }
    }

    // Profiles are restored after overlays so any overlay files a profile
    // depends on are already back in place.
    if !restorable_profiles.is_empty() {
        match crate::profile_plan::restore_profiles(&target) {
            Ok(count) => {
                println!(
                    "  {} Restored {count} profile(s).",
                    "Profiles:".cyan().bold()
                );
            }
            Err(e) => {
                eprintln!("  {} Failed to restore profiles: {}", "Error:".red(), e);
            }
        }
    }

    println!();
    println!("{}", "Restore summary:".bold());
    if restored.is_empty() {
        println!("  Restored: none");
    } else {
        println!("  Restored: {}", restored.join(", "));
    }

    if failures.is_empty() {
        println!("  Failed: none");
    } else {
        println!("  Failed:");
        for (name, error) in &failures {
            println!("    - {name}: {error}");
        }

        let failed_names = failures
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let restored_names = if restored.is_empty() {
            "none".to_string()
        } else {
            restored.join(", ")
        };
        bail!(
            "Failed to restore {} overlay(s): {failed_names}. Restored: {restored_names}",
            failures.len()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remove_overlay;
    use crate::state::list_applied_overlays;
    use std::fs;

    mod restore_overlays_tests {
        use super::*;
        use crate::state::{OverlayState, external_state_dir_for_target, load_external_states};
        use crate::testutil::{TestContext, create_overlay_dir};

        #[test]
        fn does_not_restore_explicitly_removed_overlay() {
            // This test verifies that `restore` does not re-apply overlays that were
            // explicitly removed via `repoverlay remove`. The issue is that if external
            // state exists but in-repo state was intentionally deleted (via remove command),
            // restore should NOT re-apply the overlay.
            //
            // The fix marks external state with `removed_at` timestamp when an overlay
            // is explicitly removed, so that `restore` knows to skip it.

            let ctx = TestContext::new().with_overlay(&[(".envrc", "export FOO=bar")]);

            // Use canonical path consistently (this is what restore_overlays does internally)
            let canonical_repo_path = ctx.repo_path().canonicalize().unwrap();

            // Step 1: Apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                &ApplyOptions::default(),
            )
            .expect("apply should succeed");

            // Verify overlay was applied
            assert!(
                ctx.file_exists(".envrc"),
                "overlay file should exist after apply"
            );
            assert!(
                ctx.overlay_state_exists("test-overlay") || ctx.state_dir_exists(),
                "in-repo state should exist"
            );

            // Verify external state was saved (before removal)
            let ext_dir = external_state_dir_for_target(&canonical_repo_path).unwrap();
            assert!(ext_dir.exists(), "external state directory should exist");

            // Step 2: Remove the overlay (this simulates explicit user removal)
            // This should mark the external state with `removed_at` instead of deleting it.
            let applied = list_applied_overlays(ctx.repo_path()).expect("list should work");
            assert!(
                !applied.is_empty(),
                "at least one overlay should be applied"
            );
            let overlay_name = &applied[0];

            remove_overlay(
                ctx.repo_path(),
                Some(overlay_name.to_string()),
                false,
                false,
            )
            .expect("remove should succeed");

            // Verify overlay was removed from in-repo state
            assert!(!ctx.file_exists(".envrc"), "overlay file should be removed");
            assert!(
                !ctx.overlay_state_exists(overlay_name.as_str()),
                "in-repo state should be removed"
            );

            // Verify external state file still exists (with removed_at marker)
            let ext_state_file = ext_dir.join(format!("{overlay_name}.ccl"));
            assert!(
                ext_state_file.exists(),
                "external state file should still exist (as tombstone)"
            );

            // Read the external state and verify it has removed_at set
            let content = fs::read_to_string(&ext_state_file).unwrap();
            let ext_state: OverlayState = sickle::from_str(&content).unwrap();
            assert!(
                ext_state.removed_at.is_some(),
                "external state should have removed_at marker"
            );

            // Verify load_external_states skips removed overlays
            let external_states =
                load_external_states(&canonical_repo_path).expect("load should work");
            assert_eq!(
                external_states.len(),
                0,
                "load_external_states should skip removed overlays"
            );

            // Step 3: Call restore - this SHOULD NOT restore the overlay
            // because it was explicitly removed (has removed_at marker).
            restore_overlays(ctx.repo_path(), false, ConflictStrategy::default(), false)
                .expect("restore should succeed");

            // Step 4: Verify the overlay was NOT restored
            assert!(
                !ctx.file_exists(".envrc"),
                "overlay file should NOT be restored after explicit removal"
            );
        }

        #[test]
        fn restores_overlay_after_git_clean() {
            // This test verifies that `restore` DOES re-apply overlays when
            // in-repo state is missing due to `git clean -fdx` (not explicit removal).
            //
            // The external state should NOT have `removed_at` set because the
            // overlay was not explicitly removed.

            let ctx = TestContext::new().with_overlay(&[(".envrc", "export FOO=bar")]);
            let canonical_repo_path = ctx.repo_path().canonicalize().unwrap();

            // Step 1: Apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                &ApplyOptions::default(),
            )
            .expect("apply should succeed");

            let applied = list_applied_overlays(ctx.repo_path()).expect("list should work");
            assert!(!applied.is_empty());

            // Verify external state exists and doesn't have removed_at
            let ext_states = load_external_states(&canonical_repo_path).unwrap();
            assert_eq!(ext_states.len(), 1);
            assert!(
                ext_states[0].removed_at.is_none(),
                "external state should NOT have removed_at"
            );

            // Step 2: Simulate `git clean -fdx` by removing only in-repo state
            // This does NOT call remove_overlay, so external state stays intact.
            fs::remove_dir_all(ctx.repo_path().join(".repoverlay")).unwrap();
            // Also remove the overlay files (as git clean would)
            fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();

            // Verify in-repo state is gone
            assert!(!ctx.state_dir_exists(), "in-repo state should be removed");
            assert!(
                !ctx.file_exists(".envrc"),
                "overlay files should be removed"
            );

            // External state should still be loadable (no removed_at marker)
            let ext_states_after = load_external_states(&canonical_repo_path).unwrap();
            assert_eq!(
                ext_states_after.len(),
                1,
                "external state should still be loadable"
            );

            // Step 3: Call restore - this SHOULD restore the overlay
            restore_overlays(ctx.repo_path(), false, ConflictStrategy::default(), false)
                .expect("restore should succeed");

            // Step 4: Verify the overlay WAS restored
            assert!(
                ctx.file_exists(".envrc"),
                "overlay file should be restored after git clean"
            );
        }

        #[test]
        fn restores_all_overlays_after_git_clean_returns_ok() {
            let ctx = TestContext::new();
            let overlay_a = create_overlay_dir(&[(".envrc", "export FOO=bar")]);
            let overlay_b = create_overlay_dir(&[(".toolrc", "tool = true")]);

            apply_overlay(
                overlay_a.path().to_str().unwrap(),
                ctx.repo_path(),
                &ApplyOptions {
                    name_override: Some("overlay-a".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .expect("first apply should succeed");
            apply_overlay(
                overlay_b.path().to_str().unwrap(),
                ctx.repo_path(),
                &ApplyOptions {
                    name_override: Some("overlay-b".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .expect("second apply should succeed");

            fs::remove_dir_all(ctx.repo_path().join(".repoverlay")).unwrap();
            fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
            fs::remove_file(ctx.repo_path().join(".toolrc")).unwrap();

            restore_overlays(ctx.repo_path(), false, ConflictStrategy::default(), false)
                .expect("restore should succeed when every overlay restores");

            assert!(
                ctx.file_exists(".envrc"),
                "first overlay should be restored"
            );
            assert!(
                ctx.file_exists(".toolrc"),
                "second overlay should be restored"
            );
        }

        #[test]
        fn returns_error_summary_after_partial_restore_failure() {
            let ctx = TestContext::new();
            let overlay_a = create_overlay_dir(&[(".envrc", "export FOO=bar")]);
            let overlay_b = create_overlay_dir(&[(".toolrc", "tool = true")]);

            apply_overlay(
                overlay_a.path().to_str().unwrap(),
                ctx.repo_path(),
                &ApplyOptions {
                    name_override: Some("restorable".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .expect("first apply should succeed");
            apply_overlay(
                overlay_b.path().to_str().unwrap(),
                ctx.repo_path(),
                &ApplyOptions {
                    name_override: Some("missing-source".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .expect("second apply should succeed");

            fs::remove_dir_all(ctx.repo_path().join(".repoverlay")).unwrap();
            fs::remove_file(ctx.repo_path().join(".envrc")).unwrap();
            fs::remove_file(ctx.repo_path().join(".toolrc")).unwrap();
            fs::remove_dir_all(overlay_b.path()).unwrap();

            let error =
                restore_overlays(ctx.repo_path(), false, ConflictStrategy::default(), false)
                    .expect_err("restore should fail when any overlay cannot be restored");
            let message = error.to_string();

            assert!(
                ctx.file_exists(".envrc"),
                "restore should continue and restore successful overlays"
            );
            assert!(
                !ctx.file_exists(".toolrc"),
                "failed overlay file should remain missing"
            );
            assert!(
                message.contains("restorable"),
                "summary should include successful overlay name: {message}"
            );
            assert!(
                message.contains("missing-source"),
                "summary should include failed overlay name: {message}"
            );
        }

        #[test]
        fn reapplying_overlay_clears_removed_marker() {
            // This test verifies that re-applying an overlay clears the removed_at marker
            // in case the user changes their mind after removal.

            let ctx = TestContext::new().with_overlay(&[(".envrc", "export FOO=bar")]);
            let canonical_repo_path = ctx.repo_path().canonicalize().unwrap();

            // Step 1: Apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                &ApplyOptions::default(),
            )
            .expect("apply should succeed");

            let applied = list_applied_overlays(ctx.repo_path()).expect("list should work");
            let overlay_name = &applied[0];

            // Step 2: Remove the overlay (marks removed_at)
            remove_overlay(
                ctx.repo_path(),
                Some(overlay_name.to_string()),
                false,
                false,
            )
            .expect("remove should succeed");

            // Verify removed_at is set
            let ext_dir = external_state_dir_for_target(&canonical_repo_path).unwrap();
            let ext_state_file = ext_dir.join(format!("{overlay_name}.ccl"));
            let content = fs::read_to_string(&ext_state_file).unwrap();
            let ext_state: OverlayState = sickle::from_str(&content).unwrap();
            assert!(ext_state.removed_at.is_some());

            // Step 3: Re-apply the overlay
            apply_overlay(
                ctx.overlay_source(),
                ctx.repo_path(),
                &ApplyOptions::default(),
            )
            .expect("re-apply should succeed");

            // Step 4: Verify removed_at is cleared
            let content_after = fs::read_to_string(&ext_state_file).unwrap();
            let ext_state_after: OverlayState = sickle::from_str(&content_after).unwrap();
            assert!(
                ext_state_after.removed_at.is_none(),
                "removed_at should be cleared after re-apply"
            );

            // Verify restore would now restore this overlay
            // (if git clean happened again)
            let ext_states = load_external_states(&canonical_repo_path).unwrap();
            assert_eq!(ext_states.len(), 1, "external state should be loadable");
        }
    }
}
