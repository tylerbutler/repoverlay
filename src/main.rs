use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Overlay config files into git repositories without committing them
#[derive(Parser)]
#[command(name = "repoverlay")]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Apply an overlay to a git repository
    Apply {
        /// Path to the overlay source directory
        source: PathBuf,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Force copy mode instead of symlinks (default on Windows)
        #[arg(long)]
        copy: bool,

        /// Override the overlay name (defaults to config name or directory name)
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Remove applied overlay(s)
    Remove {
        /// Name of the overlay to remove (interactive if not specified)
        name: Option<String>,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Remove all applied overlays
        #[arg(long)]
        all: bool,
    },

    /// Show the status of applied overlays
    Status {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Show only a specific overlay
        #[arg(short, long)]
        name: Option<String>,
    },
}

/// Configuration file for an overlay source (repoverlay.toml)
#[derive(Debug, Deserialize, Serialize, Default)]
struct OverlayConfig {
    #[serde(default)]
    overlay: OverlayMeta,
    #[serde(default)]
    mappings: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct OverlayMeta {
    name: Option<String>,
    description: Option<String>,
}

/// Global metadata for the .repoverlay directory
#[derive(Debug, Deserialize, Serialize)]
struct GlobalMeta {
    version: u32,
}

impl Default for GlobalMeta {
    fn default() -> Self {
        Self { version: 1 }
    }
}

/// State file tracking applied overlay (.repoverlay/overlays/<name>.toml)
#[derive(Debug, Deserialize, Serialize)]
struct OverlayState {
    meta: StateMeta,
    files: Vec<FileEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StateMeta {
    applied_at: DateTime<Utc>,
    source: PathBuf,
    name: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileEntry {
    source: PathBuf,
    target: PathBuf,
    #[serde(rename = "type")]
    link_type: LinkType,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum LinkType {
    Symlink,
    Copy,
}

const STATE_DIR: &str = ".repoverlay";
const OVERLAYS_DIR: &str = "overlays";
const META_FILE: &str = "meta.toml";
const CONFIG_FILE: &str = "repoverlay.toml";
const GIT_EXCLUDE: &str = ".git/info/exclude";
const MANAGED_SECTION_NAME: &str = "managed";

fn exclude_marker_start(name: &str) -> String {
    format!("# repoverlay:{} start", name)
}

fn exclude_marker_end(name: &str) -> String {
    format!("# repoverlay:{} end", name)
}

/// Validate and normalize overlay name for use as filename
fn normalize_overlay_name(name: &str) -> Result<String> {
    let normalized: String = name
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();

    if normalized.is_empty() {
        bail!("Invalid overlay name: '{}'", name);
    }
    Ok(normalized)
}

/// Load all target paths from all applied overlays, returning a map of path -> overlay_name
fn load_all_overlay_targets(target: &Path) -> Result<HashMap<String, String>> {
    let mut targets = HashMap::new();
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        return Ok(targets);
    }

    for entry in fs::read_dir(&overlays_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "toml").unwrap_or(false) {
            let content = fs::read_to_string(&path)?;
            let state: OverlayState = toml::from_str(&content)?;
            let overlay_name = state.meta.name.clone();

            for file in state.files {
                let target_str = file.target.to_string_lossy().to_string();
                targets.insert(target_str, overlay_name.clone());
            }
        }
    }

    Ok(targets)
}

/// List all applied overlays, returning their normalized names
fn list_applied_overlays(target: &Path) -> Result<Vec<String>> {
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        return Ok(Vec::new());
    }

    let mut names: Vec<String> = fs::read_dir(&overlays_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "toml")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .collect();

    names.sort();
    Ok(names)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Apply {
            source,
            target,
            copy,
            name,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            apply_overlay(&source, &target, copy, name)?;
        }
        Commands::Remove { name, target, all } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            remove_overlay(&target, name, all)?;
        }
        Commands::Status { target, name } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            show_status(&target, name)?;
        }
    }

    Ok(())
}

fn apply_overlay(
    source: &Path,
    target: &Path,
    force_copy: bool,
    name_override: Option<String>,
) -> Result<()> {
    // Validate source exists
    let source = source
        .canonicalize()
        .with_context(|| format!("Overlay source not found: {}", source.display()))?;

    // Validate target exists and is a git repo
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    if !target.join(".git").exists() {
        bail!("Target is not a git repository: {}", target.display());
    }

    // Determine link type
    let link_type = if force_copy || cfg!(windows) {
        LinkType::Copy
    } else {
        LinkType::Symlink
    };

    // Load overlay config (optional)
    let config_path = source.join(CONFIG_FILE);
    let config: OverlayConfig = if config_path.exists() {
        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config: {}", config_path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", config_path.display()))?
    } else {
        OverlayConfig::default()
    };

    // Determine overlay name (priority: CLI override > config > directory name)
    let overlay_name = name_override
        .or(config.overlay.name.clone())
        .unwrap_or_else(|| {
            source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unnamed".to_string())
        });
    let normalized_name = normalize_overlay_name(&overlay_name)?;

    // Check if this specific overlay already exists
    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);
    let overlay_state_path = overlays_dir.join(format!("{}.toml", normalized_name));
    if overlay_state_path.exists() {
        bail!(
            "Overlay '{}' is already applied. Run 'repoverlay remove {}' first.",
            overlay_name,
            normalized_name
        );
    }

    // Load all existing overlay targets to check for conflicts
    let existing_targets = load_all_overlay_targets(&target)?;

    println!("{} overlay: {}", "Applying".green().bold(), overlay_name);

    // Collect files to overlay
    let mut files: Vec<FileEntry> = Vec::new();
    let mut exclude_entries: Vec<String> = Vec::new();

    for entry in WalkDir::new(&source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel_path = entry.path().strip_prefix(&source)?;

        // Skip the config file itself
        if rel_path == Path::new(CONFIG_FILE) {
            continue;
        }

        let rel_str = rel_path.to_string_lossy().to_string();

        // Apply path mapping if defined
        let target_rel = config
            .mappings
            .get(&rel_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| rel_path.to_path_buf());

        let target_rel_str = target_rel.to_string_lossy().to_string();
        let source_file = entry.path().to_path_buf();
        let target_file = target.join(&target_rel);

        // Check for conflicts with existing overlays
        if let Some(conflicting_overlay) = existing_targets.get(&target_rel_str) {
            bail!(
                "Conflict: file '{}' is already managed by overlay '{}'\n\
                 Remove that overlay first or use different file mappings.",
                target_rel.display(),
                conflicting_overlay
            );
        }

        // Check for conflicts with existing files in repo
        if target_file.exists() {
            bail!(
                "Conflict: target file already exists: {}\n\
                 Remove it first or add a mapping to rename the overlay file.",
                target_file.display()
            );
        }

        // Create parent directories if needed
        if let Some(parent) = target_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        // Create symlink or copy
        match link_type {
            LinkType::Symlink => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&source_file, &target_file).with_context(|| {
                    format!("Failed to create symlink: {}", target_file.display())
                })?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(&source_file, &target_file).with_context(
                    || format!("Failed to create symlink: {}", target_file.display()),
                )?;
            }
            LinkType::Copy => {
                fs::copy(&source_file, &target_file)
                    .with_context(|| format!("Failed to copy file: {}", target_file.display()))?;
            }
        }

        println!("  {} {}", "+".green(), target_rel.display());

        files.push(FileEntry {
            source: rel_path.to_path_buf(),
            target: target_rel.clone(),
            link_type,
        });

        // Add to exclude list (use forward slashes for git)
        let exclude_path = target_rel.to_string_lossy().replace('\\', "/");
        exclude_entries.push(exclude_path);
    }

    if files.is_empty() {
        bail!("No files found in overlay source: {}", source.display());
    }

    // Update .git/info/exclude with this overlay's entries
    update_git_exclude(&target, &normalized_name, &exclude_entries, true)?;

    // Ensure state directories exist
    fs::create_dir_all(&overlays_dir)?;

    // Write global meta if this is the first overlay
    let meta_path = target.join(STATE_DIR).join(META_FILE);
    if !meta_path.exists() {
        let global_meta = GlobalMeta::default();
        fs::write(&meta_path, toml::to_string_pretty(&global_meta)?)?;
    }

    // Save overlay state
    let state = OverlayState {
        meta: StateMeta {
            applied_at: Utc::now(),
            source: source.clone(),
            name: overlay_name.clone(),
        },
        files,
    };

    fs::write(&overlay_state_path, toml::to_string_pretty(&state)?)?;

    println!(
        "\n{} Applied {} file(s) from '{}'",
        "✓".green().bold(),
        state.files.len(),
        overlay_name
    );

    Ok(())
}

fn remove_overlay(target: &Path, name: Option<String>, remove_all: bool) -> Result<()> {
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

    let overlays_dir = target.join(STATE_DIR).join(OVERLAYS_DIR);

    if !overlays_dir.exists() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    let applied_overlays = list_applied_overlays(&target)?;
    if applied_overlays.is_empty() {
        bail!("No overlays are currently applied in: {}", target.display());
    }

    if remove_all {
        // Remove all overlays
        for overlay_name in &applied_overlays {
            remove_single_overlay(&target, &overlays_dir, overlay_name)?;
        }

        // Clean up .repoverlay directory entirely
        fs::remove_dir_all(target.join(STATE_DIR))?;

        println!("\n{} Removed all overlays", "✓".green().bold());
    } else if let Some(name) = name {
        let normalized_name = normalize_overlay_name(&name)?;
        remove_single_overlay(&target, &overlays_dir, &normalized_name)?;

        // Check if any overlays remain
        let remaining = list_applied_overlays(&target)?;
        if remaining.is_empty() {
            // No overlays left, clean up .repoverlay directory
            fs::remove_dir_all(target.join(STATE_DIR))?;
        }
    } else {
        // Interactive selection
        println!("{}", "Select overlay to remove:".bold());
        println!();
        for (i, name) in applied_overlays.iter().enumerate() {
            println!("  {}. {}", i + 1, name);
        }
        println!(
            "  {}. {} (remove all)",
            applied_overlays.len() + 1,
            "all".bold()
        );
        println!();

        print!("Enter selection (1-{}): ", applied_overlays.len() + 1);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if let Ok(selection) = input.parse::<usize>() {
            if selection == applied_overlays.len() + 1 {
                // Remove all
                for overlay_name in &applied_overlays {
                    remove_single_overlay(&target, &overlays_dir, overlay_name)?;
                }
                fs::remove_dir_all(target.join(STATE_DIR))?;
                println!("\n{} Removed all overlays", "✓".green().bold());
            } else if selection >= 1 && selection <= applied_overlays.len() {
                let overlay_name = &applied_overlays[selection - 1];
                remove_single_overlay(&target, &overlays_dir, overlay_name)?;

                let remaining = list_applied_overlays(&target)?;
                if remaining.is_empty() {
                    fs::remove_dir_all(target.join(STATE_DIR))?;
                }
            } else {
                bail!("Invalid selection: {}", selection);
            }
        } else if input.eq_ignore_ascii_case("all") {
            for overlay_name in &applied_overlays {
                remove_single_overlay(&target, &overlays_dir, overlay_name)?;
            }
            fs::remove_dir_all(target.join(STATE_DIR))?;
            println!("\n{} Removed all overlays", "✓".green().bold());
        } else {
            bail!("Invalid selection: {}", input);
        }
    }

    Ok(())
}

fn remove_single_overlay(target: &Path, overlays_dir: &Path, name: &str) -> Result<()> {
    let state_file = overlays_dir.join(format!("{}.toml", name));

    if !state_file.exists() {
        // List available overlays for helpful error message
        let available = list_applied_overlays(target)?;

        if available.is_empty() {
            bail!("No overlays are currently applied");
        } else {
            bail!(
                "Overlay '{}' not found. Available overlays: {}",
                name,
                available.join(", ")
            );
        }
    }

    let state_content = fs::read_to_string(&state_file)?;
    let state: OverlayState = toml::from_str(&state_content)?;

    println!("{} overlay: {}", "Removing".red().bold(), state.meta.name);

    let mut exclude_entries = Vec::new();

    // Remove files
    for entry in &state.files {
        let file_path = target.join(&entry.target);

        if file_path.exists() || file_path.is_symlink() {
            fs::remove_file(&file_path)
                .with_context(|| format!("Failed to remove: {}", file_path.display()))?;
            println!("  {} {}", "-".red(), entry.target.display());

            // Remove empty parent directories (but not the target itself)
            let mut parent = file_path.parent();
            while let Some(dir) = parent {
                if dir == target {
                    break;
                }
                if dir
                    .read_dir()
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
                {
                    fs::remove_dir(dir).ok();
                    parent = dir.parent();
                } else {
                    break;
                }
            }
        }

        let exclude_path = entry.target.to_string_lossy().replace('\\', "/");
        exclude_entries.push(exclude_path);
    }

    // Update git exclude (remove this overlay's section)
    update_git_exclude(target, name, &exclude_entries, false)?;

    // Remove state file
    fs::remove_file(&state_file)?;

    println!(
        "\n{} Removed {} file(s) from '{}'",
        "✓".green().bold(),
        state.files.len(),
        state.meta.name
    );

    Ok(())
}

fn show_status(target: &Path, filter_name: Option<String>) -> Result<()> {
    let target = target
        .canonicalize()
        .with_context(|| format!("Target directory not found: {}", target.display()))?;

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
        let state_file = overlays_dir.join(format!("{}.toml", normalized));

        if !state_file.exists() {
            bail!(
                "Overlay '{}' is not applied. Available: {}",
                filter,
                applied_overlays.join(", ")
            );
        }

        show_single_overlay_status(&target, &state_file)?;
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
        let state_file = overlays_dir.join(format!("{}.toml", overlay_name));
        show_single_overlay_status(&target, &state_file)?;
        println!();
    }

    Ok(())
}

fn show_single_overlay_status(target: &Path, state_file: &Path) -> Result<()> {
    let state_content = fs::read_to_string(state_file)?;
    let state: OverlayState = toml::from_str(&state_content)?;

    println!("  {} {}", "Overlay:".bold(), state.meta.name.cyan());
    println!("    Source:  {}", state.meta.source.display());
    println!(
        "    Applied: {}",
        state.meta.applied_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    println!("    Files:   {}", state.files.len());

    for entry in &state.files {
        let target_path = target.join(&entry.target);
        let status = if target_path.exists() || target_path.is_symlink() {
            "✓".green()
        } else {
            "✗".red()
        };

        let type_str = match entry.link_type {
            LinkType::Symlink => "symlink",
            LinkType::Copy => "copy",
        };

        println!(
            "      {} {} ({})",
            status,
            entry.target.display(),
            type_str.dimmed()
        );
    }

    Ok(())
}

fn update_git_exclude(
    target: &Path,
    overlay_name: &str,
    entries: &[String],
    add: bool,
) -> Result<()> {
    let exclude_path = target.join(GIT_EXCLUDE);

    // Ensure the .git/info directory exists
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

fn remove_overlay_section(content: &str, name: &str) -> String {
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

fn any_overlay_sections_remain(content: &str) -> bool {
    // Check for any repoverlay sections except "managed"
    for line in content.lines() {
        if line.starts_with("# repoverlay:")
            && line.ends_with(" start")
            && !line.contains(MANAGED_SECTION_NAME)
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    // Helper to create a test git repository
    fn create_test_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .expect("Failed to init git repo");
        dir
    }

    // Helper to create a test overlay directory with files
    fn create_test_overlay(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (path, content) in files {
            let file_path = dir.path().join(path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(file_path, content).unwrap();
        }
        dir
    }

    // Unit tests for remove_overlay_section
    mod remove_section {
        use super::*;

        #[test]
        fn empty_content() {
            let result = remove_overlay_section("", "test-overlay");
            assert_eq!(result, "");
        }

        #[test]
        fn no_section_present() {
            let content = "*.log\n.DS_Store\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n.DS_Store\n");
        }

        #[test]
        fn section_at_end() {
            let content = "*.log\n# repoverlay:test-overlay start\n.envrc\n.repoverlay\n# repoverlay:test-overlay end\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n");
        }

        #[test]
        fn section_at_beginning() {
            let content =
                "# repoverlay:test-overlay start\n.envrc\n# repoverlay:test-overlay end\n*.log\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n");
        }

        #[test]
        fn section_in_middle() {
            let content = "*.log\n# repoverlay:test-overlay start\n.envrc\n# repoverlay:test-overlay end\n.DS_Store\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "*.log\n.DS_Store\n");
        }

        #[test]
        fn only_section() {
            let content = "# repoverlay:test-overlay start\n.envrc\n.repoverlay\n# repoverlay:test-overlay end\n";
            let result = remove_overlay_section(content, "test-overlay");
            assert_eq!(result, "");
        }

        #[test]
        fn removes_only_specified_overlay() {
            let content = "# repoverlay:overlay-a start\n.envrc\n# repoverlay:overlay-a end\n# repoverlay:overlay-b start\n.env\n# repoverlay:overlay-b end\n";
            let result = remove_overlay_section(content, "overlay-a");
            assert!(result.contains("overlay-b"));
            assert!(!result.contains("overlay-a"));
        }
    }

    // Integration tests for apply command
    mod apply {
        use super::*;

        #[test]
        fn applies_single_file() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(overlay.path(), repo.path(), false, None);
            assert!(result.is_ok(), "apply_overlay failed: {:?}", result);

            // Check symlink was created
            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists(), ".envrc should exist");
            assert!(target_file.is_symlink(), ".envrc should be a symlink");

            // Check content is correct
            let content = fs::read_to_string(&target_file).unwrap();
            assert_eq!(content, "export FOO=bar");

            // Check state was saved in new location
            let overlays_dir = repo.path().join(".repoverlay/overlays");
            assert!(overlays_dir.exists(), "overlays dir should exist");
        }

        #[test]
        fn applies_nested_files() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (".vscode/settings.json", r#"{"editor.tabSize": 2}"#),
            ]);

            let result = apply_overlay(overlay.path(), repo.path(), false, None);
            assert!(result.is_ok());

            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".vscode/settings.json").exists());
        }

        #[test]
        fn applies_with_copy_mode() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(overlay.path(), repo.path(), true, None);
            assert!(result.is_ok());

            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists());
            assert!(
                !target_file.is_symlink(),
                ".envrc should NOT be a symlink in copy mode"
            );
        }

        #[test]
        fn updates_git_exclude_with_overlay_section() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false, None).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            // New per-overlay marker format
            assert!(content.contains("# repoverlay:"));
            assert!(content.contains(" start"));
            assert!(content.contains(".envrc"));
            assert!(content.contains(" end"));
            // Managed section for .repoverlay
            assert!(content.contains("# repoverlay:managed start"));
            assert!(content.contains(".repoverlay"));
        }

        #[test]
        fn respects_path_mappings() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (
                    "repoverlay.toml",
                    r#"
[mappings]
".envrc" = ".env"
"#,
                ),
            ]);

            apply_overlay(overlay.path(), repo.path(), false, None).unwrap();

            assert!(
                !repo.path().join(".envrc").exists(),
                ".envrc should not exist"
            );
            assert!(repo.path().join(".env").exists(), ".env should exist");
        }

        #[test]
        fn uses_overlay_name_from_config() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (
                    "repoverlay.toml",
                    r#"
[overlay]
name = "my-custom-overlay"
"#,
                ),
            ]);

            apply_overlay(overlay.path(), repo.path(), false, None).unwrap();

            // State file should be named after the normalized overlay name
            let state_file = repo
                .path()
                .join(".repoverlay/overlays/my-custom-overlay.toml");
            assert!(state_file.exists(), "state file should use overlay name");
        }

        #[test]
        fn uses_name_override() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path(),
                repo.path(),
                false,
                Some("custom-name".to_string()),
            )
            .unwrap();

            let state_file = repo.path().join(".repoverlay/overlays/custom-name.toml");
            assert!(state_file.exists(), "state file should use override name");
        }

        #[test]
        fn fails_on_non_git_directory() {
            let dir = TempDir::new().unwrap();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            let result = apply_overlay(overlay.path(), dir.path(), false, None);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not a git repository")
            );
        }

        #[test]
        fn fails_on_duplicate_overlay_name() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path(),
                repo.path(),
                false,
                Some("my-overlay".to_string()),
            );
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("already applied"));
        }

        #[test]
        fn fails_on_file_conflict_with_repo() {
            let repo = create_test_repo();
            fs::write(repo.path().join(".envrc"), "existing content").unwrap();

            let overlay = create_test_overlay(&[(".envrc", "new content")]);

            let result = apply_overlay(overlay.path(), repo.path(), false, None);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Conflict"));
        }

        #[test]
        fn fails_on_file_conflict_between_overlays() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "first")]);
            let overlay2 = create_test_overlay(&[(".envrc", "second")]);

            apply_overlay(
                overlay1.path(),
                repo.path(),
                false,
                Some("first".to_string()),
            )
            .unwrap();

            let result = apply_overlay(
                overlay2.path(),
                repo.path(),
                false,
                Some("second".to_string()),
            );
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Conflict") || err.contains("already managed"));
        }

        #[test]
        fn fails_on_empty_overlay() {
            let repo = create_test_repo();
            let overlay = TempDir::new().unwrap();

            let result = apply_overlay(overlay.path(), repo.path(), false, None);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No files found"));
        }

        #[test]
        fn fails_on_nonexistent_source() {
            let repo = create_test_repo();
            let result = apply_overlay(Path::new("/nonexistent/path"), repo.path(), false, None);
            assert!(result.is_err());
        }
    }

    // Integration tests for remove command
    mod remove {
        use super::*;

        #[test]
        fn removes_overlay_by_name() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[
                (".envrc", "export FOO=bar"),
                (".vscode/settings.json", r#"{"key": "value"}"#),
            ]);

            apply_overlay(
                overlay.path(),
                repo.path(),
                false,
                Some("test-overlay".to_string()),
            )
            .unwrap();
            remove_overlay(repo.path(), Some("test-overlay".to_string()), false).unwrap();

            assert!(!repo.path().join(".envrc").exists());
            assert!(!repo.path().join(".vscode/settings.json").exists());
            assert!(!repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_all_overlays() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
            )
            .unwrap();
            apply_overlay(
                overlay2.path(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
            )
            .unwrap();

            assert!(repo.path().join(".envrc").exists());
            assert!(repo.path().join(".env.local").exists());

            remove_overlay(repo.path(), None, true).unwrap();

            assert!(!repo.path().join(".envrc").exists());
            assert!(!repo.path().join(".env.local").exists());
            assert!(!repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_one_overlay_preserves_others() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
            )
            .unwrap();
            apply_overlay(
                overlay2.path(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
            )
            .unwrap();

            remove_overlay(repo.path(), Some("overlay-a".to_string()), false).unwrap();

            assert!(!repo.path().join(".envrc").exists());
            assert!(repo.path().join(".env.local").exists());
            assert!(repo.path().join(".repoverlay").exists());
        }

        #[test]
        fn removes_empty_parent_directories() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);

            apply_overlay(overlay.path(), repo.path(), false, Some("test".to_string())).unwrap();
            assert!(repo.path().join(".vscode").exists());

            remove_overlay(repo.path(), Some("test".to_string()), false).unwrap();
            assert!(
                !repo.path().join(".vscode").exists(),
                ".vscode should be removed"
            );
        }

        #[test]
        fn preserves_non_empty_parent_directories() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".vscode/settings.json", r#"{"key": "value"}"#)]);

            // Create another file in .vscode that isn't from the overlay
            fs::create_dir_all(repo.path().join(".vscode")).unwrap();
            fs::write(repo.path().join(".vscode/other.json"), "{}").unwrap();

            apply_overlay(overlay.path(), repo.path(), false, Some("test".to_string())).unwrap();
            remove_overlay(repo.path(), Some("test".to_string()), false).unwrap();

            assert!(
                repo.path().join(".vscode").exists(),
                ".vscode should remain"
            );
            assert!(repo.path().join(".vscode/other.json").exists());
        }

        #[test]
        fn cleans_git_exclude_for_overlay() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false, Some("test".to_string())).unwrap();
            remove_overlay(repo.path(), Some("test".to_string()), false).unwrap();

            let exclude_path = repo.path().join(".git/info/exclude");
            let content = fs::read_to_string(&exclude_path).unwrap();

            assert!(!content.contains("# repoverlay:test start"));
            assert!(!content.contains(".envrc"));
            assert!(!content.contains("# repoverlay:managed"));
        }

        #[test]
        fn fails_when_no_overlay_applied() {
            let repo = create_test_repo();

            let result = remove_overlay(repo.path(), Some("nonexistent".to_string()), false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("No overlay"));
        }

        #[test]
        fn fails_on_unknown_overlay_name() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(
                overlay.path(),
                repo.path(),
                false,
                Some("real-overlay".to_string()),
            )
            .unwrap();

            let result = remove_overlay(repo.path(), Some("fake-overlay".to_string()), false);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not found"));
        }

        #[test]
        fn handles_already_deleted_files() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false, Some("test".to_string())).unwrap();

            // Manually delete the file
            fs::remove_file(repo.path().join(".envrc")).unwrap();

            // Remove should still succeed
            let result = remove_overlay(repo.path(), Some("test".to_string()), false);
            assert!(result.is_ok());
        }
    }

    // Integration tests for status command
    mod status {
        use super::*;

        #[test]
        fn shows_no_overlay_when_none_applied() {
            let repo = create_test_repo();
            let result = show_status(repo.path(), None);
            assert!(result.is_ok());
        }

        #[test]
        fn shows_status_when_overlay_applied() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false, Some("test".to_string())).unwrap();

            let result = show_status(repo.path(), None);
            assert!(result.is_ok());
        }

        #[test]
        fn shows_status_for_multiple_overlays() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
            )
            .unwrap();
            apply_overlay(
                overlay2.path(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
            )
            .unwrap();

            let result = show_status(repo.path(), None);
            assert!(result.is_ok());
        }

        #[test]
        fn shows_status_for_specific_overlay() {
            let repo = create_test_repo();
            let overlay1 = create_test_overlay(&[(".envrc", "export FOO=bar")]);
            let overlay2 = create_test_overlay(&[(".env.local", "LOCAL=true")]);

            apply_overlay(
                overlay1.path(),
                repo.path(),
                false,
                Some("overlay-a".to_string()),
            )
            .unwrap();
            apply_overlay(
                overlay2.path(),
                repo.path(),
                false,
                Some("overlay-b".to_string()),
            )
            .unwrap();

            let result = show_status(repo.path(), Some("overlay-a".to_string()));
            assert!(result.is_ok());
        }

        #[test]
        fn fails_on_unknown_overlay_filter() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            apply_overlay(overlay.path(), repo.path(), false, Some("real".to_string())).unwrap();

            let result = show_status(repo.path(), Some("fake".to_string()));
            assert!(result.is_err());
        }
    }

    // CLI integration tests using assert_cmd
    mod cli {
        use super::*;
        use assert_cmd::assert::OutputAssertExt;
        use predicates::prelude::*;

        fn repoverlay_cmd() -> std::process::Command {
            let path = std::env::var("CARGO_BIN_EXE_repoverlay").unwrap_or_else(|_| {
                env!("CARGO_MANIFEST_DIR").to_string() + "/target/debug/repoverlay"
            });
            std::process::Command::new(path)
        }

        #[test]
        fn help_displays() {
            repoverlay_cmd()
                .arg("--help")
                .assert()
                .success()
                .stdout(predicate::str::contains("Overlay config files"));
        }

        #[test]
        fn version_displays() {
            repoverlay_cmd()
                .arg("--version")
                .assert()
                .success()
                .stdout(predicate::str::contains("repoverlay"));
        }

        #[test]
        fn apply_help_displays() {
            repoverlay_cmd()
                .args(["apply", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Apply an overlay"));
        }

        #[test]
        fn remove_help_displays() {
            repoverlay_cmd()
                .args(["remove", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Remove"));
        }

        #[test]
        fn status_help_displays() {
            repoverlay_cmd()
                .args(["status", "--help"])
                .assert()
                .success()
                .stdout(predicate::str::contains("status"));
        }

        #[test]
        fn apply_requires_source_argument() {
            repoverlay_cmd()
                .arg("apply")
                .assert()
                .failure()
                .stderr(predicate::str::contains("required"));
        }

        #[test]
        fn apply_and_remove_workflow() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            // Apply with explicit name
            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .args(["--name", "test-overlay"])
                .assert()
                .success()
                .stdout(predicate::str::contains("Applying"));

            assert!(repo.path().join(".envrc").exists());

            // Status
            repoverlay_cmd()
                .args(["status", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("Overlay Status"));

            // Remove by name
            repoverlay_cmd()
                .args([
                    "remove",
                    "test-overlay",
                    "--target",
                    repo.path().to_str().unwrap(),
                ])
                .assert()
                .success()
                .stdout(predicate::str::contains("Removing"));

            assert!(!repo.path().join(".envrc").exists());
        }

        #[test]
        fn apply_and_remove_all_workflow() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            // Apply
            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .assert()
                .success();

            assert!(repo.path().join(".envrc").exists());

            // Remove with --all
            repoverlay_cmd()
                .args(["remove", "--all", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("Removed all"));

            assert!(!repo.path().join(".envrc").exists());
        }

        #[test]
        fn apply_with_copy_flag() {
            let repo = create_test_repo();
            let overlay = create_test_overlay(&[(".envrc", "export FOO=bar")]);

            repoverlay_cmd()
                .args(["apply", overlay.path().to_str().unwrap()])
                .args(["--target", repo.path().to_str().unwrap()])
                .arg("--copy")
                .assert()
                .success();

            let target_file = repo.path().join(".envrc");
            assert!(target_file.exists());
            assert!(!target_file.is_symlink());
        }

        #[test]
        fn status_when_no_overlay() {
            let repo = create_test_repo();

            repoverlay_cmd()
                .args(["status", "--target", repo.path().to_str().unwrap()])
                .assert()
                .success()
                .stdout(predicate::str::contains("No overlay"));
        }

        #[test]
        fn remove_when_no_overlay() {
            let repo = create_test_repo();

            // Use --all to avoid interactive prompt
            repoverlay_cmd()
                .args(["remove", "--all", "--target", repo.path().to_str().unwrap()])
                .assert()
                .failure()
                .stderr(predicate::str::contains("No overlay"));
        }
    }
}
