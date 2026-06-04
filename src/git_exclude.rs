use anyhow::Result;
use log::debug;
use std::fs;
use std::path::Path;

use crate::git::resolve_git_exclude_path;
use crate::github;
use crate::state::{
    MANAGED_SECTION_NAME, STATE_DIR, exclude_marker_end, exclude_marker_start,
    list_applied_overlays, load_overlay_state,
};

pub(crate) fn update_git_exclude(
    target: &Path,
    overlay_name: &str,
    entries: &[String],
    add: bool,
) -> Result<()> {
    debug!(
        "update_git_exclude: overlay={}, add={}, entries={}",
        overlay_name,
        add,
        entries.len()
    );

    // Resolve the correct exclude path (uses git common dir for worktrees)
    let exclude_path = resolve_git_exclude_path(target)?;

    // Ensure the info directory exists
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = fs::read_to_string(&exclude_path).unwrap_or_default();

    // Remove existing section for this overlay
    content = remove_overlay_section(&content, overlay_name);

    if add {
        // Add new section for this overlay
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&exclude_marker_start(overlay_name));
        content.push('\n');
        for entry in entries {
            content.push_str(entry);
            content.push('\n');
        }
        content.push_str(&exclude_marker_end(overlay_name));
        content.push('\n');

        // Ensure managed section exists (for .repoverlay itself)
        if !content.contains(&exclude_marker_start(MANAGED_SECTION_NAME)) {
            content.push_str(&exclude_marker_start(MANAGED_SECTION_NAME));
            content.push('\n');
            content.push_str(STATE_DIR);
            content.push('\n');
            content.push_str(&exclude_marker_end(MANAGED_SECTION_NAME));
            content.push('\n');
        }
    } else {
        // Check if any overlay sections remain (excluding managed)
        if !any_overlay_sections_remain(&content) {
            // Remove the managed section too
            content = remove_overlay_section(&content, MANAGED_SECTION_NAME);
        }
    }

    // Clean up excessive newlines
    while content.ends_with("\n\n") {
        content.pop();
    }

    fs::write(&exclude_path, content)?;
    Ok(())
}

/// Ensure `.repoverlay` is in `.git/info/exclude`.
///
/// Called whenever repoverlay writes to the `.repoverlay/` directory,
/// so the state directory doesn't show up as untracked even before
/// any overlay is applied.
pub(crate) fn ensure_repoverlay_excluded(repo_root: &Path) -> Result<()> {
    let exclude_path = resolve_git_exclude_path(repo_root)?;

    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut content = fs::read_to_string(&exclude_path).unwrap_or_default();

    if content.contains(&exclude_marker_start(MANAGED_SECTION_NAME)) {
        return Ok(());
    }

    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&exclude_marker_start(MANAGED_SECTION_NAME));
    content.push('\n');
    content.push_str(STATE_DIR);
    content.push('\n');
    content.push_str(&exclude_marker_end(MANAGED_SECTION_NAME));
    content.push('\n');

    fs::write(&exclude_path, content)?;
    Ok(())
}

/// Remove an overlay section from git exclude content.
pub(crate) fn remove_overlay_section(content: &str, name: &str) -> String {
    let start_marker = exclude_marker_start(name);
    let end_marker = exclude_marker_end(name);

    let mut result = String::new();
    let mut in_section = false;

    for line in content.lines() {
        if line.trim() == start_marker {
            in_section = true;
            continue;
        }
        if line.trim() == end_marker {
            in_section = false;
            continue;
        }
        if !in_section {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing newlines
    while result.ends_with("\n\n") {
        result.pop();
    }

    result
}

/// Check if any overlay sections remain in git exclude content.
pub(crate) fn any_overlay_sections_remain(content: &str) -> bool {
    let managed_start = exclude_marker_start(MANAGED_SECTION_NAME);
    // Check for any repoverlay sections except the shared "managed" section.
    for line in content.lines() {
        if line.starts_with("# repoverlay:")
            && line.ends_with(" start")
            && line.trim() != managed_start
        {
            return true;
        }
    }
    false
}

/// Rebuild `.git/info/exclude` from overlay state files.
///
/// Reads all applied overlays from the state directory and reconstructs the
/// git exclude entries. This repairs cases where the exclude file gets out of
/// sync (e.g. after a `git clean`, manual edits, or interrupted operations).
///
/// Returns `true` if the exclude file was modified.
pub(crate) fn repair_git_exclude(target: &Path) -> Result<bool> {
    use crate::state::EntryType;

    let applied = list_applied_overlays(target)?;
    let profile_states = crate::profile_plan::list_profile_states(target).unwrap_or_default();
    if applied.is_empty() && profile_states.is_empty() {
        return Ok(false);
    }

    let exclude_path = resolve_git_exclude_path(target)?;
    let before = fs::read_to_string(&exclude_path).unwrap_or_default();

    for name in &applied {
        let name_str = name.as_str();
        let Ok(overlay_state) = load_overlay_state(target, name_str) else {
            continue;
        };

        let exclude_entries: Vec<String> = overlay_state
            .files
            .iter()
            .map(|f| {
                let path = f.target.to_string_lossy().replace('\\', "/");
                if f.entry_type == EntryType::Directory {
                    format!("{path}/")
                } else {
                    path
                }
            })
            .collect();

        update_git_exclude(target, name_str, &exclude_entries, true)?;
    }

    // Profiles write their own repo-local files (instructions, mcp.json, skills,
    // agents); rebuild their sections too so repair fully restores exclusions.
    for state in &profile_states {
        let entries = crate::profile_plan::profile_exclude_entries(target, state);
        if entries.is_empty() {
            continue;
        }
        let section = crate::profile_plan::profile_exclude_section(&state.name, &state.harness);
        update_git_exclude(target, &section, &entries, true)?;
    }

    let after = fs::read_to_string(&exclude_path).unwrap_or_default();
    Ok(before != after)
}

/// Parse owner/repo from a GitHub URL (HTTPS or SSH format).
pub(crate) fn parse_github_owner_repo(url: &str) -> Result<(String, String)> {
    github::parse_remote_url(url).ok_or_else(|| {
        if url.contains("github.com") {
            anyhow::anyhow!("Could not parse git remote URL: {url}")
        } else {
            anyhow::anyhow!(
                "Could not detect target repository from git remote.\n\
                 Non-GitHub remotes are not supported for auto-detection.\n\
                 Please specify --target org/repo"
            )
        }
    })
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

    // Tests for parse_github_owner_repo
    mod parse_github_owner_repo_tests {
        use super::*;

        #[test]
        fn parses_https_url() {
            let result = parse_github_owner_repo("https://github.com/owner/repo").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_https_url_with_git_suffix() {
            let result = parse_github_owner_repo("https://github.com/owner/repo.git").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_ssh_url() {
            let result = parse_github_owner_repo("git@github.com:owner/repo").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_ssh_url_with_git_suffix() {
            let result = parse_github_owner_repo("git@github.com:owner/repo.git").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn parses_http_url() {
            let result = parse_github_owner_repo("http://github.com/owner/repo").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn handles_url_with_extra_path() {
            let result =
                parse_github_owner_repo("https://github.com/owner/repo/tree/main/path").unwrap();
            assert_eq!(result, ("owner".to_string(), "repo".to_string()));
        }

        #[test]
        fn fails_on_non_github_url() {
            let result = parse_github_owner_repo("https://gitlab.com/owner/repo");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Non-GitHub"));
        }

        #[test]
        fn fails_on_empty_owner() {
            let result = parse_github_owner_repo("https://github.com//repo");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_empty_repo() {
            let result = parse_github_owner_repo("https://github.com/owner/");
            assert!(result.is_err());
        }

        #[test]
        fn fails_on_malformed_url() {
            let result = parse_github_owner_repo("https://github.com/onlyowner");
            assert!(result.is_err());
        }
    }

    // Tests for any_overlay_sections_remain
    mod any_overlay_sections_remain_tests {
        use super::*;

        #[test]
        fn returns_false_for_empty_content() {
            assert!(!any_overlay_sections_remain(""));
        }

        #[test]
        fn returns_false_for_no_sections() {
            let content = "*.log\n.DS_Store\n";
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_false_for_only_managed_section() {
            let content = "# repoverlay:managed start\n.repoverlay\n# repoverlay:managed end\n";
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_true_for_overlay_section() {
            let content = "# repoverlay:my-overlay start\n.envrc\n# repoverlay:my-overlay end\n";
            assert!(any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_true_for_overlay_plus_managed_section() {
            let content = "# repoverlay:my-overlay start\n.envrc\n# repoverlay:my-overlay end\n\
                           # repoverlay:managed start\n.repoverlay\n# repoverlay:managed end\n";
            assert!(any_overlay_sections_remain(content));
        }

        #[test]
        fn returns_true_for_multiple_overlay_sections() {
            let content = "# repoverlay:overlay-a start\n.envrc\n# repoverlay:overlay-a end\n\
                           # repoverlay:overlay-b start\n.env\n# repoverlay:overlay-b end\n";
            assert!(any_overlay_sections_remain(content));
        }

        #[test]
        fn ignores_partial_markers() {
            // Line that starts with "# repoverlay:" but doesn't end with " start"
            let content = "# repoverlay:something else\n";
            assert!(!any_overlay_sections_remain(content));
        }
    }

    // Tests for update_git_exclude
    mod update_git_exclude_tests {
        use super::*;

        #[test]
        fn creates_exclude_file_if_missing() {
            let repo = create_test_repo();
            let entries = vec![".envrc".to_string()];

            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            assert!(exclude_path.exists());

            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains("# repoverlay:test-overlay start"));
            assert!(content.contains(".envrc"));
            assert!(content.contains("# repoverlay:test-overlay end"));
        }

        #[test]
        fn appends_to_existing_exclude_file() {
            let repo = create_test_repo();

            // Create existing exclude content
            let exclude_path = repo.path().join(".git/info/exclude");
            fs::create_dir_all(exclude_path.parent().unwrap()).unwrap();
            fs::write(&exclude_path, "*.log\n").unwrap();

            let entries = vec![".envrc".to_string()];
            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains("*.log"));
            assert!(content.contains("# repoverlay:test-overlay start"));
        }

        #[test]
        fn removes_section_when_add_is_false() {
            let repo = create_test_repo();

            // First add a section
            let entries = vec![".envrc".to_string()];
            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            // Then remove it
            update_git_exclude(repo.path(), "test-overlay", &entries, false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(!content.contains("# repoverlay:test-overlay"));
        }

        #[test]
        fn adds_managed_section_with_first_overlay() {
            let repo = create_test_repo();
            let entries = vec![".envrc".to_string()];

            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains("# repoverlay:managed start"));
            assert!(content.contains(".repoverlay"));
        }

        #[test]
        fn removes_managed_section_when_last_overlay_removed() {
            let repo = create_test_repo();

            // Add an overlay
            let entries = vec![".envrc".to_string()];
            update_git_exclude(repo.path(), "test-overlay", &entries, true).unwrap();

            // Remove it
            update_git_exclude(repo.path(), "test-overlay", &entries, false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(!content.contains("# repoverlay:managed"));
        }

        #[test]
        fn writes_to_correct_location_in_worktree() {
            use std::process::Command;

            // Create a real git repo with a worktree
            let temp = TempDir::new().unwrap();
            let main_path = temp.path().join("main");
            fs::create_dir_all(&main_path).unwrap();
            Command::new("git")
                .args(["init"])
                .current_dir(&main_path)
                .output()
                .unwrap();
            // Need at least one commit for worktrees
            Command::new("git")
                .args(["commit", "--allow-empty", "-m", "init"])
                .current_dir(&main_path)
                .output()
                .unwrap();

            let worktree_path = temp.path().join("worktree");
            Command::new("git")
                .args([
                    "worktree",
                    "add",
                    worktree_path.to_str().unwrap(),
                    "-b",
                    "wt-branch",
                ])
                .current_dir(&main_path)
                .output()
                .unwrap();

            let entries = vec![".envrc".to_string()];
            update_git_exclude(&worktree_path, "test-overlay", &entries, true).unwrap();

            // Exclude file should be in the common git dir (main repo's .git/)
            let common_exclude = main_path.join(".git").join("info").join("exclude");
            assert!(
                common_exclude.exists(),
                "exclude file should exist in common git dir"
            );

            let content = fs::read_to_string(&common_exclude).unwrap();
            assert!(content.contains("# repoverlay:test-overlay start"));
            assert!(content.contains(".envrc"));
        }
    }

    // Tests for remove_overlay_section (additional edge cases)
    mod remove_overlay_section_additional_tests {
        use super::*;

        #[test]
        fn handles_windows_line_endings() {
            let content = "*.log\r\n# repoverlay:test start\r\n.envrc\r\n# repoverlay:test end\r\n.DS_Store\r\n";
            let result = remove_overlay_section(content, "test");
            // Should still work even though line endings differ
            assert!(!result.contains("repoverlay:test"));
        }

        #[test]
        fn handles_whitespace_around_markers() {
            let content = "  # repoverlay:test start  \n.envrc\n  # repoverlay:test end  \n";
            let result = remove_overlay_section(content, "test");
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn preserves_content_before_and_after() {
            let content = "before\n# repoverlay:test start\n.envrc\n# repoverlay:test end\nafter\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.contains("before"));
            assert!(result.contains("after"));
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn handles_empty_section() {
            let content = "# repoverlay:empty start\n# repoverlay:empty end\n";
            let result = remove_overlay_section(content, "empty");
            assert!(!result.contains("repoverlay:empty"));
        }

        #[test]
        fn removes_only_specified_overlay() {
            let content = "# repoverlay:a start\n.a\n# repoverlay:a end\n\
                          # repoverlay:b start\n.b\n# repoverlay:b end\n";
            let result = remove_overlay_section(content, "a");
            assert!(!result.contains(".a"));
            assert!(result.contains(".b"));
            assert!(result.contains("# repoverlay:b"));
        }

        #[test]
        fn handles_similar_named_overlays() {
            let content = "# repoverlay:test start\n.test\n# repoverlay:test end\n\
                          # repoverlay:test-extended start\n.extended\n# repoverlay:test-extended end\n";
            let result = remove_overlay_section(content, "test");
            assert!(!result.contains(".test\n"));
            assert!(result.contains(".extended"));
        }
    }

    // Tests for update_git_exclude with multiple overlays
    mod update_git_exclude_multiple_tests {
        use super::*;

        #[test]
        fn handles_multiple_overlays() {
            let repo = create_test_repo();

            // Add first overlay
            update_git_exclude(repo.path(), "overlay-a", &[".envrc".to_string()], true).unwrap();

            // Add second overlay
            update_git_exclude(repo.path(), "overlay-b", &[".env.local".to_string()], true)
                .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(content.contains("# repoverlay:overlay-a start"));
            assert!(content.contains(".envrc"));
            assert!(content.contains("# repoverlay:overlay-b start"));
            assert!(content.contains(".env.local"));
        }

        #[test]
        fn keeps_managed_section_when_one_overlay_remains() {
            let repo = create_test_repo();

            // Add two overlays
            update_git_exclude(repo.path(), "overlay-a", &[".envrc".to_string()], true).unwrap();
            update_git_exclude(repo.path(), "overlay-b", &[".env".to_string()], true).unwrap();

            // Remove one overlay
            update_git_exclude(repo.path(), "overlay-a", &[".envrc".to_string()], false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            // Managed section should remain because overlay-b is still there
            assert!(content.contains("# repoverlay:managed start"));
            assert!(content.contains("# repoverlay:overlay-b start"));
            assert!(!content.contains("# repoverlay:overlay-a"));
        }

        #[test]
        fn updates_existing_overlay_section() {
            let repo = create_test_repo();

            // Add overlay with one file
            update_git_exclude(repo.path(), "test", &[".envrc".to_string()], true).unwrap();

            // "Update" same overlay with different files (add=true replaces)
            update_git_exclude(
                repo.path(),
                "test",
                &[".env".to_string(), ".env.local".to_string()],
                true,
            )
            .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            // Should have new entries, old should be gone
            assert!(content.contains(".env"));
            assert!(content.contains(".env.local"));
            // Should only have one test section
            assert_eq!(content.matches("# repoverlay:test start").count(), 1);
        }

        #[test]
        fn handles_multiple_entries_per_overlay() {
            let repo = create_test_repo();

            update_git_exclude(
                repo.path(),
                "test",
                &[
                    ".envrc".to_string(),
                    ".env.local".to_string(),
                    ".vscode/settings.json".to_string(),
                ],
                true,
            )
            .unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(content.contains(".envrc"));
            assert!(content.contains(".env.local"));
            assert!(content.contains(".vscode/settings.json"));
        }
    }

    // Additional edge case tests for line ending handling
    mod line_ending_edge_cases {
        use super::*;

        #[test]
        fn remove_overlay_section_with_mixed_line_endings() {
            // Mix of LF and CRLF within the same file
            let content =
                "before\n# repoverlay:test start\r\n.envrc\n# repoverlay:test end\r\nafter\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.contains("before"));
            assert!(result.contains("after"));
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn remove_overlay_section_with_only_crlf() {
            let content = "*.log\r\n# repoverlay:test start\r\n.envrc\r\n# repoverlay:test end\r\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.contains("*.log"));
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn remove_overlay_section_preserves_trailing_newline() {
            let content = "before\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            assert!(result.ends_with('\n'));
        }

        #[test]
        fn remove_overlay_section_with_no_trailing_newline() {
            let content = "# repoverlay:test start\n.envrc\n# repoverlay:test end";
            let result = remove_overlay_section(content, "test");
            // Should handle content without trailing newline
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn update_git_exclude_with_existing_crlf_content() {
            let repo = create_test_repo();
            let exclude_path = repo.path().join(".git/info/exclude");

            // Create exclude file with CRLF line endings
            fs::create_dir_all(exclude_path.parent().unwrap()).unwrap();
            fs::write(&exclude_path, "*.log\r\n.DS_Store\r\n").unwrap();

            update_git_exclude(repo.path(), "test", &[".envrc".to_string()], true).unwrap();

            let content = fs::read_to_string(&exclude_path).unwrap();
            assert!(content.contains(".envrc"));
            assert!(content.contains("# repoverlay:test start"));
        }
    }

    // Tests for duplicate/malformed section markers
    mod malformed_section_tests {
        use super::*;

        #[test]
        fn remove_overlay_section_with_duplicate_start_markers() {
            // Two start markers, only one end marker
            let content =
                "# repoverlay:test start\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            // Should remove everything between first start and end
            assert!(!result.contains(".envrc"));
        }

        #[test]
        fn remove_overlay_section_with_unclosed_section() {
            // Start marker but no end marker
            let content = "before\n# repoverlay:test start\n.envrc\nafter\n";
            let result = remove_overlay_section(content, "test");
            // Content after start should be removed (no end marker means section continues)
            assert!(result.contains("before"));
            assert!(!result.contains(".envrc"));
            assert!(!result.contains("after"));
        }

        #[test]
        fn remove_overlay_section_with_nested_markers() {
            // Nested markers (shouldn't happen, but test robustness)
            let content = "# repoverlay:outer start\n# repoverlay:inner start\n.envrc\n# repoverlay:inner end\n# repoverlay:outer end\n";
            let result = remove_overlay_section(content, "outer");
            assert!(!result.contains(".envrc"));
            assert!(!result.contains("repoverlay:inner"));
        }

        #[test]
        fn any_overlay_sections_remain_with_malformed_marker() {
            // Marker with only "start" but not in correct format
            let content = "# repoverlay start\n.envrc\n";
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn any_overlay_sections_remain_with_extra_spaces() {
            // Extra spaces in marker
            let content = "#  repoverlay:test  start\n.envrc\n# repoverlay:test end\n";
            // Should not match due to different spacing
            assert!(!any_overlay_sections_remain(content));
        }

        #[test]
        fn remove_overlay_section_cleans_multiple_trailing_newlines() {
            // Content with empty line before section creates multiple trailing newlines after removal
            let content = "line1\n\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            // Should clean up the double newline at the end
            assert!(result.contains("line1"));
            assert!(!result.contains(".envrc"));
            assert!(
                !result.ends_with("\n\n"),
                "Should not end with double newline"
            );
            assert!(result.ends_with('\n'), "Should end with single newline");
        }

        #[test]
        fn remove_overlay_section_cleans_many_trailing_newlines() {
            // Multiple empty lines before section
            let content = "line1\n\n\n# repoverlay:test start\n.envrc\n# repoverlay:test end\n";
            let result = remove_overlay_section(content, "test");
            // Should clean up all excess trailing newlines
            assert!(
                !result.ends_with("\n\n"),
                "Should not end with double newline"
            );
        }
    }
}
