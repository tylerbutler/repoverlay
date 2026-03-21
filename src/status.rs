use anyhow::{Result, bail};
use colored::Colorize;
use std::path::{Path, PathBuf};

use crate::OverlayName;
use crate::canonicalize_path;
use crate::state::{
    EntryType, LinkType, OVERLAYS_DIR, OverlaySource, STATE_DIR, list_applied_overlays,
    load_overlay_state, normalize_overlay_name,
};

/// Check whether overlays are currently applied (for `--quiet` mode).
///
/// If `filter_name` is `Some`, checks only whether that specific overlay is applied.
/// Otherwise, checks whether any overlay is applied.
///
/// Returns `true` if at least one matching overlay is applied, `false` otherwise.
pub(crate) fn status_has_overlays(target: &Path, filter_name: Option<&str>) -> Result<bool> {
    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    if !overlays_dir.exists() {
        return Ok(false);
    }
    let applied = list_applied_overlays(&target)?;
    filter_name.map_or_else(
        || Ok(!applied.is_empty()),
        |name| Ok(applied.iter().any(|o| o == name)),
    )
}

/// Output overlay status as JSON for scripting and CI integration.
pub(crate) fn show_status_json(target: &Path, filter_name: Option<&str>) -> Result<()> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct JsonOutput {
        overlays: Vec<JsonOverlay>,
    }

    #[derive(Serialize)]
    struct JsonOverlay {
        name: String,
        applied_at: String,
        source: OverlaySource,
        files: Vec<JsonFileEntry>,
    }

    #[derive(Serialize)]
    struct JsonFileEntry {
        source: PathBuf,
        target: PathBuf,
        link_type: LinkType,
        entry_type: EntryType,
        status: &'static str,
    }

    let target = canonicalize_path(target, "Target directory")?;
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonOutput { overlays: vec![] })?
        );
        return Ok(());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        println!(
            "{}",
            serde_json::to_string_pretty(&JsonOutput { overlays: vec![] })?
        );
        return Ok(());
    }

    // Filter if name provided
    let names: Vec<&OverlayName> = if let Some(filter) = filter_name {
        let normalized = normalize_overlay_name(filter)?;
        if !applied_overlays.iter().any(|n| n == normalized.as_str()) {
            let names: Vec<&str> = applied_overlays.iter().map(OverlayName::as_str).collect();
            bail!(
                "Overlay '{}' is not applied. Available: {}",
                filter,
                names.join(", ")
            );
        }
        applied_overlays
            .iter()
            .filter(|n| n.as_str() == normalized.as_str())
            .collect()
    } else {
        applied_overlays.iter().collect()
    };

    let mut overlays = Vec::new();
    for overlay_name in &names {
        let state = load_overlay_state(&target, overlay_name.as_str())?;
        let files: Vec<JsonFileEntry> = state
            .file_entries()
            .iter()
            .map(|entry| {
                let target_path = target.join(&entry.target);
                let status = if target_path.exists() || target_path.is_symlink() {
                    "ok"
                } else {
                    "missing"
                };
                JsonFileEntry {
                    source: entry.source.clone(),
                    target: entry.target.clone(),
                    link_type: entry.link_type,
                    entry_type: entry.entry_type,
                    status,
                }
            })
            .collect();

        overlays.push(JsonOverlay {
            name: state.name.clone(),
            applied_at: state.applied_at.to_rfc3339(),
            source: state.source.clone(),
            files,
        });
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&JsonOutput { overlays })?
    );
    Ok(())
}

/// Show the status of applied overlays.
pub(crate) fn show_status(target: &Path, filter_name: Option<String>) -> Result<()> {
    let target = canonicalize_path(target, "Target directory")?;

    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        println!("{} No overlays are currently applied.", "Status:".bold());
        return Ok(());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        println!("{} No overlays are currently applied.", "Status:".bold());
        return Ok(());
    }

    // If filtering by name, show just that overlay
    if let Some(filter) = filter_name {
        let normalized = normalize_overlay_name(&filter)?;

        if !applied_overlays.iter().any(|n| n == normalized.as_str()) {
            let names: Vec<&str> = applied_overlays.iter().map(OverlayName::as_str).collect();
            bail!(
                "Overlay '{}' is not applied. Available: {}",
                filter,
                names.join(", ")
            );
        }

        show_single_overlay_status(&target, &normalized)?;
        return Ok(());
    }

    // Show summary header
    println!(
        "{} ({} overlay(s) applied)",
        "Overlay Status".bold(),
        applied_overlays.len()
    );
    println!();

    for overlay_name in &applied_overlays {
        show_single_overlay_status(&target, overlay_name.as_str())?;
        println!();
    }

    Ok(())
}

/// Show status for a single overlay.
pub(crate) fn show_single_overlay_status(target: &Path, name: &str) -> Result<()> {
    let state = load_overlay_state(target, name)?;

    println!("  {} {}", "Overlay:".bold(), state.name.cyan());

    // Display source based on type
    match &state.source {
        OverlaySource::Local { path } => {
            println!("    Source:  {}", path.display());
        }
        OverlaySource::GitHub {
            url,
            git_ref,
            commit,
            subpath,
            ..
        } => {
            println!("    Source:  {} {}", url, "(GitHub)".dimmed());
            println!("    Ref:     {git_ref}");
            let short_commit = &commit[..12.min(commit.len())];
            println!("    Commit:  {short_commit}");
            if let Some(sp) = subpath {
                println!("    Subpath: {sp}");
            }
        }
        OverlaySource::OverlayRepo {
            org,
            repo,
            name: overlay_name,
            commit,
            resolved_via,
            source_name,
        } => {
            let via_upstream = matches!(resolved_via, Some(crate::state::ResolvedVia::Upstream));
            let via_str = if via_upstream {
                format!(" {}", "(via upstream)".yellow())
            } else {
                String::new()
            };
            println!("    Source:  {org}/{repo}/{overlay_name}{via_str}");
            let short_commit = &commit[..12.min(commit.len())];
            println!("    Commit:  {short_commit}");
            if let Some(source) = source_name {
                println!("    From:    {}", source.cyan());
            }
        }
        OverlaySource::Library { name } => {
            println!("    Source:  {} {}", name, "(library)".dimmed());
        }
    }

    println!(
        "    Updated: {}",
        state.applied_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("    Files:   {}", state.file_count());

    for entry in state.file_entries() {
        let target_path = target.join(&entry.target);
        let status = if target_path.exists() || target_path.is_symlink() {
            "✓".green()
        } else {
            "✗".red()
        };

        let type_str = match entry.link_type {
            LinkType::Symlink => "symlink",
            LinkType::Copy => "copy",
            LinkType::Merged => "merged",
        };

        // Add trailing slash and [dir] marker for directories
        let (path_display, dir_marker) = match entry.entry_type {
            EntryType::Directory => (format!("{}/", entry.target.display()), " [dir]"),
            EntryType::File => (entry.target.display().to_string(), ""),
        };

        println!(
            "      {} {}{} ({})",
            status,
            path_display,
            dir_marker.magenta(),
            type_str.dimmed()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to init git repo");
        dir
    }

    // Tests for show_status
    mod show_status_tests {
        use super::*;
        use crate::{apply_overlay, ConflictStrategy};

        #[test]
        fn status_no_overlays_succeeds() {
            let repo = create_test_repo();
            let canonical = repo.path().canonicalize().unwrap();
            // Should not error, just print message
            show_status(&canonical, None).unwrap();
        }

        #[test]
        fn status_with_overlay_succeeds() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("status-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            show_status(&canonical, None).unwrap();
        }

        #[test]
        fn status_with_filter_existing() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("filtered-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            show_status(&canonical, Some("filtered-test".to_string())).unwrap();
        }

        #[test]
        fn status_with_filter_nonexistent_fails() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("real-overlay".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let result = show_status(&canonical, Some("nonexistent".to_string()));
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("not applied"));
            assert!(err.contains("real-overlay"));
        }
    }

    mod status_has_overlays_tests {
        use super::*;
        use crate::{apply_overlay, ConflictStrategy};

        #[test]
        fn no_overlays_returns_false() {
            let repo = create_test_repo();
            let canonical = repo.path().canonicalize().unwrap();
            assert!(!status_has_overlays(&canonical, None).unwrap());
        }

        #[test]
        fn with_overlay_returns_true() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("check-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            assert!(status_has_overlays(&canonical, None).unwrap());
            // Filter by name should also match
            assert!(status_has_overlays(&canonical, Some("check-test")).unwrap());
            // Non-existent name should return false
            assert!(!status_has_overlays(&canonical, Some("nonexistent")).unwrap());
        }
    }

    mod show_status_json_tests {
        use super::*;
        use crate::{apply_overlay, ConflictStrategy};

        #[test]
        fn json_no_overlays_outputs_empty_array() {
            let repo = create_test_repo();
            let canonical = repo.path().canonicalize().unwrap();
            // Should not error
            show_status_json(&canonical, None).unwrap();
        }

        #[test]
        fn json_with_overlay_succeeds() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("json-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            show_status_json(&canonical, None).unwrap();
        }

        #[test]
        fn json_with_name_filter() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("json-filter-test".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            show_status_json(&canonical, Some("json-filter-test")).unwrap();
        }

        #[test]
        fn json_with_nonexistent_filter_fails() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                true,
                Some("real-json".to_string()),
                None,
                false,
                ConflictStrategy::default(),
                false,
                None,
                false,
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let result = show_status_json(&canonical, Some("nonexistent"));
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not applied"));
        }
    }
}
