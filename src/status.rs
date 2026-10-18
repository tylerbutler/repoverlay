use anyhow::{Result, bail};
use colored::Colorize;
use serde::Serialize;
use std::path::Path;

use crate::OverlayName;
use crate::canonicalize_path;
use crate::profile::{ProfileMode, ProfileState};
use crate::profile_applicators::AgentHarness;
use crate::state::{
    EntryType, LinkType, OverlaySource, ResolvedVia, list_applied_overlays, load_overlay_state,
    normalize_overlay_name,
};

const STATUS_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct JsonStatusOutput {
    schema_version: u32,
    overlays: Vec<JsonStatusOverlay>,
    profiles: Vec<JsonStatusProfile>,
}

#[derive(Serialize)]
struct JsonStatusOverlay {
    name: String,
    applied_at: String,
    source: JsonStatusSource,
    files: Vec<JsonStatusFile>,
}

#[derive(Serialize)]
struct JsonStatusSource {
    #[serde(rename = "type")]
    source_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subpath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_via: Option<&'static str>,
}

#[derive(Serialize)]
struct JsonStatusFile {
    source: String,
    target: String,
    link_type: &'static str,
    entry_type: &'static str,
    status: &'static str,
}

#[derive(Serialize)]
struct JsonStatusProfile {
    name: String,
    harness: AgentHarness,
    mode: ProfileMode,
    applied_at: String,
    overlays: Vec<String>,
    files: Vec<JsonStatusProfileFile>,
}

#[derive(Serialize)]
struct JsonStatusProfileFile {
    source: String,
    target: String,
    action: String,
    status: &'static str,
}

/// Check whether a matching overlay or profile is applied (for `--quiet` mode).
pub(crate) fn status_has_entries(target: &Path, filter_name: Option<&str>) -> Result<bool> {
    let target = canonicalize_path(target, "Target directory")?;
    let applied = list_applied_overlays(&target)?;
    let profiles = crate::profile_plan::list_profile_states(&target)?;
    filter_name.map_or_else(
        || Ok(!applied.is_empty() || !profiles.is_empty()),
        |name| {
            Ok(applied.iter().any(|o| o == name)
                || profiles.iter().any(|profile| profile.name == name))
        },
    )
}

fn list_status_entries(
    target: &Path,
    filter_name: Option<&str>,
) -> Result<(Vec<OverlayName>, Vec<ProfileState>)> {
    let mut overlays = list_applied_overlays(target)?;
    let mut profiles = crate::profile_plan::list_profile_states(target)?;
    if let Some(filter) = filter_name
        && (!overlays.is_empty() || !profiles.is_empty())
    {
        let normalized = normalize_overlay_name(filter);
        let matches_overlay = |name: &OverlayName| {
            normalized
                .as_ref()
                .is_ok_and(|filter| name.as_str() == filter)
        };
        if !overlays.iter().any(matches_overlay)
            && !profiles.iter().any(|profile| profile.name == filter)
        {
            normalized?;
            let names: Vec<&str> = overlays
                .iter()
                .map(OverlayName::as_str)
                .chain(profiles.iter().map(|profile| profile.name.as_str()))
                .collect();
            bail!(
                "Overlay or profile '{}' is not applied. Available: {}",
                filter,
                names.join(", ")
            );
        }
        overlays.retain(matches_overlay);
        profiles.retain(|profile| profile.name == filter);
    }
    Ok((overlays, profiles))
}

/// Output overlay and profile status as JSON for scripting and CI integration.
pub(crate) fn show_status_json(target: &Path, filter_name: Option<&str>) -> Result<()> {
    let target = canonicalize_path(target, "Target directory")?;
    let (names, profile_states) = list_status_entries(&target, filter_name)?;

    let mut overlays = Vec::new();
    for overlay_name in &names {
        let state = load_overlay_state(&target, overlay_name.as_str())?;
        let files: Vec<JsonStatusFile> = state
            .file_entries()
            .iter()
            .map(|entry| {
                let target_path = target.join(&entry.target);
                let status = if target_path.exists() || target_path.is_symlink() {
                    "ok"
                } else {
                    "missing"
                };
                JsonStatusFile {
                    source: path_to_json_string(&entry.source),
                    target: path_to_json_string(&entry.target),
                    link_type: link_type_to_json(entry.link_type),
                    entry_type: entry_type_to_json(entry.entry_type),
                    status,
                }
            })
            .collect();

        overlays.push(JsonStatusOverlay {
            name: state.name.clone(),
            applied_at: state.applied_at.to_rfc3339(),
            source: source_to_json(&state.source),
            files,
        });
    }

    let profiles = profile_states
        .into_iter()
        .map(|state| JsonStatusProfile {
            name: state.name,
            harness: state.harness,
            mode: state.mode,
            applied_at: state.applied_at.to_rfc3339(),
            overlays: state.overlays,
            files: state
                .files
                .iter()
                .map(|entry| JsonStatusProfileFile {
                    source: path_to_json_string(&entry.source),
                    target: path_to_json_string(
                        entry.target.strip_prefix(&target).unwrap_or(&entry.target),
                    ),
                    action: entry.action.clone(),
                    status: if entry.target.exists() || entry.target.is_symlink() {
                        "ok"
                    } else {
                        "missing"
                    },
                })
                .collect(),
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&JsonStatusOutput {
            schema_version: STATUS_JSON_SCHEMA_VERSION,
            overlays,
            profiles,
        })?
    );
    Ok(())
}

fn source_to_json(source: &OverlaySource) -> JsonStatusSource {
    match source {
        OverlaySource::Local { path, source_name } => JsonStatusSource {
            source_type: "local",
            path: Some(path_to_json_string(path)),
            source_name: source_name.clone(),
            url: None,
            owner: None,
            org: None,
            repo: None,
            name: None,
            git_ref: None,
            commit: None,
            subpath: None,
            resolved_via: None,
        },
        OverlaySource::GitHub {
            url,
            owner,
            repo,
            git_ref,
            commit,
            subpath,
            ..
        } => JsonStatusSource {
            source_type: "github",
            path: None,
            source_name: None,
            url: Some(url.clone()),
            owner: Some(owner.clone()),
            org: None,
            repo: Some(repo.clone()),
            name: None,
            git_ref: Some(git_ref.clone()),
            commit: Some(commit.clone()),
            subpath: subpath.clone(),
            resolved_via: None,
        },
        OverlaySource::Library { name } => JsonStatusSource {
            source_type: "library",
            path: None,
            source_name: None,
            url: None,
            owner: None,
            org: None,
            repo: None,
            name: Some(name.clone()),
            git_ref: None,
            commit: None,
            subpath: None,
            resolved_via: None,
        },
        OverlaySource::OverlayRepo {
            org,
            repo,
            name,
            commit,
            resolved_via,
            source_name,
        } => JsonStatusSource {
            source_type: "overlay_repo",
            path: None,
            source_name: source_name.clone(),
            url: None,
            owner: None,
            org: Some(org.clone()),
            repo: Some(repo.clone()),
            name: Some(name.clone()),
            git_ref: None,
            commit: Some(commit.clone()),
            subpath: None,
            resolved_via: resolved_via.map(resolved_via_to_json),
        },
    }
}

const fn resolved_via_to_json(resolved_via: ResolvedVia) -> &'static str {
    match resolved_via {
        ResolvedVia::Direct => "direct",
        ResolvedVia::Upstream => "upstream",
    }
}

const fn link_type_to_json(link_type: LinkType) -> &'static str {
    match link_type {
        LinkType::Symlink => "symlink",
        LinkType::Copy => "copy",
        LinkType::Merged => "merged",
    }
}

const fn entry_type_to_json(entry_type: EntryType) -> &'static str {
    match entry_type {
        EntryType::File => "file",
        EntryType::Directory => "directory",
    }
}

fn path_to_json_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Show the status of applied overlays and profiles.
pub(crate) fn show_status(target: &Path, filter_name: Option<&str>) -> Result<()> {
    let target = canonicalize_path(target, "Target directory")?;
    let (applied_overlays, profiles) = list_status_entries(&target, filter_name)?;

    if applied_overlays.is_empty() && profiles.is_empty() {
        println!(
            "{} No overlays or profiles are currently applied.",
            "Status:".bold()
        );
        return Ok(());
    }

    if !applied_overlays.is_empty() && filter_name.is_none() {
        println!(
            "{} ({} overlay(s) applied)",
            "Overlay Status".bold(),
            applied_overlays.len()
        );
        println!();
    }

    for overlay_name in &applied_overlays {
        show_single_overlay_status(&target, overlay_name.as_str())?;
        println!();
    }

    if !profiles.is_empty() {
        println!(
            "{} ({} profile(s) applied)",
            "Profile Status".bold(),
            profiles.len()
        );
        println!();
        for profile in profiles {
            println!(
                "  {} {} ({})",
                "Profile:".bold(),
                profile.name.cyan(),
                profile.harness
            );
            println!(
                "    Updated: {}",
                profile.applied_at.format("%Y-%m-%d %H:%M:%S UTC")
            );
            println!("    Files:   {}", profile.files.len());
            for entry in profile.files {
                let status = if entry.target.exists() || entry.target.is_symlink() {
                    "✓".green()
                } else {
                    "✗".red()
                };
                let path = entry.target.strip_prefix(&target).unwrap_or(&entry.target);
                println!(
                    "      {} {} ({})",
                    status,
                    path.display(),
                    entry.action.dimmed()
                );
            }
            println!();
        }
    }

    Ok(())
}

/// Show status for a single overlay.
pub(crate) fn show_single_overlay_status(target: &Path, name: &str) -> Result<()> {
    let state = load_overlay_state(target, name)?;

    println!("  {} {}", "Overlay:".bold(), state.name.cyan());

    // Display source based on type
    match &state.source {
        OverlaySource::Local { path, .. } => {
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
        use crate::{ApplyOptions, apply_overlay};

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
                &ApplyOptions {
                    force_copy: true,
                    name_override: Some("status-test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    force_copy: true,
                    name_override: Some("filtered-test".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            show_status(&canonical, Some("filtered-test")).unwrap();
        }

        #[test]
        fn status_with_filter_nonexistent_fails() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    force_copy: true,
                    name_override: Some("real-overlay".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let result = show_status(&canonical, Some("nonexistent"));
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("not applied"));
            assert!(err.contains("real-overlay"));
        }
    }

    mod status_has_entries_tests {
        use super::*;
        use crate::{ApplyOptions, apply_overlay};

        #[test]
        fn no_overlays_returns_false() {
            let repo = create_test_repo();
            let canonical = repo.path().canonicalize().unwrap();
            assert!(!status_has_entries(&canonical, None).unwrap());
        }

        #[test]
        fn with_overlay_returns_true() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();
            fs::write(overlay.path().join("file.txt"), "content").unwrap();

            apply_overlay(
                overlay.path().to_str().unwrap(),
                repo.path(),
                &ApplyOptions {
                    force_copy: true,
                    name_override: Some("check-test".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            assert!(status_has_entries(&canonical, None).unwrap());
            // Filter by name should also match
            assert!(status_has_entries(&canonical, Some("check-test")).unwrap());
            // Non-existent name should return false
            assert!(!status_has_entries(&canonical, Some("nonexistent")).unwrap());
        }
    }

    mod show_status_json_tests {
        use super::*;
        use crate::{ApplyOptions, apply_overlay};

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
                &ApplyOptions {
                    force_copy: true,
                    name_override: Some("json-test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    force_copy: true,
                    name_override: Some("json-filter-test".to_string()),
                    ..ApplyOptions::default()
                },
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
                &ApplyOptions {
                    force_copy: true,
                    name_override: Some("real-json".to_string()),
                    ..ApplyOptions::default()
                },
            )
            .unwrap();

            let canonical = repo.path().canonicalize().unwrap();
            let result = show_status_json(&canonical, Some("nonexistent"));
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not applied"));
        }
    }
}
