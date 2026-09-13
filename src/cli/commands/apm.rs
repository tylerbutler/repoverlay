use anyhow::{Context, Result, bail};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::ApmCommand;
use crate::cli::commands::resolve_target;
use crate::plugin::PluginRef;
use crate::profile::{ProfileConfig, ResolvedJsonMerge, ResolvedPlacement};
use crate::profile_applicators::AgentHarness;

const SOURCE_MARKER: &str = ".repoverlay-apm-source";
/// Directory inside the durable artifact that stores target-native APM output.
const NATIVE_DIR: &str = ".repoverlay-native";
/// Subdirectory of [`NATIVE_DIR`] that mirrors the captured repository paths.
const NATIVE_FILES_DIR: &str = "files";
/// Claude `settings.json` hooks object captured from the temporary project.
const CLAUDE_HOOKS_FILE: &str = "claude-hooks.json";

#[derive(serde::Deserialize)]
struct TargetApmManifest {
    policy: Option<serde_json::Value>,
}

#[derive(serde::Serialize)]
struct TemporaryApmManifest<'a> {
    name: &'static str,
    version: &'static str,
    targets: [&'a str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    policy: Option<serde_json::Value>,
}

pub(crate) fn handle_apm_command(command: ApmCommand) -> Result<()> {
    match command {
        ApmCommand::Install {
            package,
            harness,
            target,
            allow_hooks,
        } => install(&package, harness, target, allow_hooks),
    }
}

fn install(
    package: &str,
    harness: AgentHarness,
    target: Option<PathBuf>,
    allow_hooks: bool,
) -> Result<()> {
    let target = resolve_target(target)?;
    install_with_apm(package, harness, &target, OsStr::new("apm"), allow_hooks)
}

fn install_with_apm(
    package: &str,
    harness: AgentHarness,
    target: &Path,
    apm: &OsStr,
    allow_hooks: bool,
) -> Result<()> {
    validate_package(package)?;
    let identity = package_identity(package);
    let key = package_key(identity)?;
    let profile_name = format!("apm-{key}");
    if let Some(state) = crate::profile_plan::list_profile_states(target)?
        .into_iter()
        .find(|state| state.name == profile_name)
    {
        bail!(
            "APM package '{package}' is already installed for {}; remove profile \
             '{profile_name}' before installing it for {harness}",
            state.harness
        )
    }
    if let Some(snapshot) = crate::profile_plan::list_restorable_profiles(target)?
        .into_iter()
        .next()
    {
        bail!(
            "Profile '{}' ({}) can be restored; run `repoverlay restore` before installing \
             another APM package",
            snapshot.name,
            snapshot.harness
        )
    }
    let harness_name = harness.to_string();
    let package_root = crate::state::external_state_dir_for_target(target)?
        .join("apm")
        .join(&key);
    ensure_package_identity(&package_root, identity)?;
    let destination = package_root.join(&harness_name).join(&key);
    let native_root = destination.join(NATIVE_DIR);

    let temp = tempfile::tempdir().context("Failed to create temporary APM project")?;
    let manifest = TemporaryApmManifest {
        name: "repoverlay-apm-install",
        version: "1.0.0",
        targets: [&harness_name],
        policy: target_policy(target)?,
    };
    let manifest =
        serde_saphyr::to_string(&manifest).context("Failed to serialize temporary APM manifest")?;
    fs::write(temp.path().join("apm.yml"), manifest)
        .context("Failed to create temporary APM manifest")?;
    mirror_git_identity(target, temp.path())?;
    run_apm(
        apm,
        temp.path(),
        &["install", "--target", &harness_name, "--", package],
        "install",
    )?;
    let resolved_mcp_servers = resolved_mcp_servers(temp.path(), harness)?;
    let capture = tempfile::tempdir().context("Failed to create temporary APM capture")?;
    let captured = capture_native_output(temp.path(), harness, capture.path(), &native_root)?;
    if captured.capabilities.iter().any(|c| c == "hooks") && !allow_hooks {
        bail!(
            "APM package '{package}' installs hooks, which run commands in this repository; \
             inspect the package and rerun with --allow-hooks"
        )
    }
    let build_dir = temp.path().join("build");
    let output = build_dir.to_string_lossy().into_owned();
    run_apm(
        apm,
        temp.path(),
        &["pack", "--format", "claude-plugin", "--output", &output],
        "pack",
    )?;

    let bundle = single_bundle(&build_dir)?;
    fs::create_dir_all(&package_root)
        .with_context(|| format!("Failed to create {}", package_root.display()))?;
    fs::write(package_root.join(SOURCE_MARKER), format!("{identity}\n"))?;

    let harness_root = package_root.join(&harness_name);
    fs::create_dir_all(&harness_root)
        .with_context(|| format!("Failed to create {}", harness_root.display()))?;
    ensure_managed_directory(&destination)?;
    let suffix = std::process::id();
    let staging = harness_root.join(format!(".{key}.new-{suffix}"));
    let backup = harness_root.join(format!(".{key}.old-{suffix}"));
    remove_if_exists(&staging)?;
    fs::create_dir(&staging)?;
    if let Err(err) = crate::overlay_repo::copy_dir_recursive(&bundle, &staging) {
        let _ = remove_if_exists(&staging);
        return Err(err).context("Failed to stage the APM plugin bundle");
    }
    if captured.is_empty() {
        // Nothing target-native to keep.
    } else if let Err(err) =
        crate::profile_plan::copy_tree_no_symlinks(capture.path(), &staging.join(NATIVE_DIR))
    {
        let _ = remove_if_exists(&staging);
        return Err(err).context("Failed to stage the target-native APM output");
    }

    remove_if_exists(&backup)?;
    if destination.exists()
        && let Err(err) = fs::rename(&destination, &backup)
    {
        let _ = remove_if_exists(&staging);
        return Err(err).with_context(|| {
            format!(
                "Failed to preserve existing bundle {}",
                destination.display()
            )
        });
    }
    if let Err(err) = fs::rename(&staging, &destination) {
        let _ = restore_bundle(&backup, &destination);
        return Err(err).context("Failed to activate the new APM bundle");
    }

    let resolved_placements = native_placements(&native_root, harness)?;
    let resolved_json_merges = claude_hooks_merge(&native_root, harness)?
        .into_iter()
        .collect();
    let profile = ProfileConfig {
        description: Some(format!("APM package {package}")),
        plugins: vec![PluginRef::Local {
            source: destination.clone(),
        }],
        resolved_mcp_servers,
        resolved_placements,
        resolved_json_merges,
        handled_plugin_capabilities: captured.capabilities,
        ..ProfileConfig::default()
    };
    if let Err(err) =
        crate::profile_plan::apply_profile_config(&profile_name, harness, target, &profile)
    {
        remove_if_exists(&destination)?;
        restore_bundle(&backup, &destination)?;
        return Err(err).context("Failed to apply the APM package");
    }

    if let Err(err) = remove_if_exists(&backup) {
        eprintln!(
            "Warning: could not remove old APM bundle {}: {err}",
            backup.display()
        );
    }
    println!("Installed APM package '{package}' as profile '{profile_name}'");
    Ok(())
}

fn run_apm(apm: &OsStr, directory: &Path, args: &[&str], operation: &str) -> Result<()> {
    let status = Command::new(apm)
        .args(args)
        .current_dir(directory)
        .status()
        .with_context(|| format!("Failed to run 'apm {operation}'; is APM installed?"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("APM {operation} failed with {status}")
    }
}

fn target_policy(target: &Path) -> Result<Option<serde_json::Value>> {
    let path = target.join("apm.yml");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("Failed to read {}", path.display())),
    };
    let manifest: TargetApmManifest = serde_saphyr::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(manifest.policy)
}

fn resolved_mcp_servers(
    project: &Path,
    harness: AgentHarness,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mcp_path = project.join(".mcp.json");
    if harness == AgentHarness::Copilot {
        let copilot_path = project.join(".github").join("mcp.json");
        if copilot_path.is_file() {
            fs::copy(&copilot_path, &mcp_path).with_context(|| {
                format!(
                    "Failed to normalize Copilot MCP configuration {}",
                    copilot_path.display()
                )
            })?;
        }
    }

    let content = match fs::read_to_string(&mcp_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::Map::new());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "Failed to read APM MCP configuration {}",
                    mcp_path.display()
                )
            });
        }
    };
    let value: serde_json::Value = serde_json::from_str(&content).with_context(|| {
        format!(
            "Failed to parse APM MCP configuration {}",
            mcp_path.display()
        )
    })?;
    match value.get("mcpServers") {
        None => Ok(serde_json::Map::new()),
        Some(serde_json::Value::Object(servers)) => Ok(servers.clone()),
        Some(_) => bail!(
            "APM MCP configuration {} has a non-object 'mcpServers' value",
            mcp_path.display()
        ),
    }
}

/// Target-native APM output captured from the temporary project.
struct NativeCapture {
    /// Plugin capabilities the capture handled (`commands`, `instructions`,
    /// `hooks`), so the applicator does not report them as skipped.
    capabilities: Vec<String>,
}

impl NativeCapture {
    const fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }
}

/// A repository-relative directory repoverlay copies out of the temporary APM
/// project.
struct NativeRoot {
    /// Repository-relative directory in the temporary project.
    relative: &'static str,
    /// Plugin capability this directory satisfies.
    capability: &'static str,
    /// Whether each direct child becomes a managed placement in the repository.
    /// Claude hook scripts are kept in the durable artifact only, because Claude
    /// runs them through the absolute command path in `settings.json`.
    placed: bool,
}

const fn root(relative: &'static str, capability: &'static str, placed: bool) -> NativeRoot {
    NativeRoot {
        relative,
        capability,
        placed,
    }
}

/// The allowlisted target-native directories for a harness.
fn native_roots(harness: AgentHarness) -> Vec<NativeRoot> {
    match harness {
        AgentHarness::Claude => vec![
            root(".claude/commands", "commands", true),
            root(".claude/rules", "instructions", true),
            root(".claude/hooks", "hooks", false),
        ],
        AgentHarness::Copilot => vec![
            root(".github/prompts", "commands", true),
            root(".github/instructions", "instructions", true),
            root(".github/hooks", "hooks", true),
        ],
    }
}

/// Copy the allowlisted target-native APM output into `capture_dir`.
///
/// Only the directories in [`native_roots`] and Claude's `hooks` settings object
/// are captured. Symlinked sources are rejected, and hook commands that point
/// into the temporary project are rewritten to `native_root`, the durable
/// artifact location the capture will occupy.
fn capture_native_output(
    project: &Path,
    harness: AgentHarness,
    capture_dir: &Path,
    native_root: &Path,
) -> Result<NativeCapture> {
    let mut capabilities: Vec<String> = Vec::new();
    let files_root = capture_dir.join(NATIVE_FILES_DIR);
    for root in native_roots(harness) {
        let source = project.join(root.relative);
        if !source.is_dir() || fs::read_dir(&source)?.next().is_none() {
            continue;
        }
        let destination = files_root.join(root.relative);
        crate::profile_plan::copy_tree_no_symlinks(&source, &destination).with_context(|| {
            format!(
                "Failed to capture target-native APM output {}",
                root.relative
            )
        })?;
        if root.capability == "hooks" {
            rewrite_hook_files(&destination, project, native_root)?;
        }
        if !capabilities.iter().any(|known| known == root.capability) {
            capabilities.push(root.capability.to_string());
        }
    }

    if harness == AgentHarness::Claude
        && let Some(hooks) = claude_settings_hooks(project)?
    {
        let hooks = rewrite_project_paths(&hooks, project, native_root)?;
        let content = serde_json::to_string_pretty(&hooks)
            .context("Failed to serialize captured Claude hooks")?;
        crate::state::atomic_write(&capture_dir.join(CLAUDE_HOOKS_FILE), &content)?;
        if !capabilities.iter().any(|known| known == "hooks") {
            capabilities.push("hooks".to_string());
        }
    }

    Ok(NativeCapture { capabilities })
}

/// Read the top-level `hooks` object from the temporary project's Claude
/// settings, ignoring every other setting.
fn claude_settings_hooks(project: &Path) -> Result<Option<serde_json::Value>> {
    let path = project.join(".claude").join("settings.json");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("Failed to read {}", path.display())),
    };
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    match value.get("hooks") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Object(hooks)) if hooks.is_empty() => Ok(None),
        Some(serde_json::Value::Object(hooks)) => {
            Ok(Some(serde_json::Value::Object(hooks.clone())))
        }
        Some(_) => bail!(
            "APM wrote a non-object 'hooks' value into {}",
            path.display()
        ),
    }
}

/// Rewrite temporary-project paths inside every captured hook definition.
fn rewrite_hook_files(directory: &Path, project: &Path, native_root: &Path) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("Failed to read {}", directory.display()))?
    {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(OsStr::to_str) != Some("json") {
            continue;
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse hook definition {}", path.display()))?;
        let value = rewrite_project_paths(&value, project, native_root)?;
        let rewritten =
            serde_json::to_string_pretty(&value).context("Failed to serialize hook definition")?;
        crate::state::atomic_write(&path, &rewritten)?;
    }
    Ok(())
}

/// Replace temporary-project prefixes with the durable artifact location.
///
/// Relative commands stay relative. An absolute path that points into a
/// temporary directory outside this install's project is rejected, because it
/// cannot survive the install.
fn rewrite_project_paths(
    value: &serde_json::Value,
    project: &Path,
    native_root: &Path,
) -> Result<serde_json::Value> {
    let project = project.to_string_lossy().into_owned();
    let native = native_root
        .join(NATIVE_FILES_DIR)
        .to_string_lossy()
        .into_owned();
    let temp_dir = std::env::temp_dir().to_string_lossy().into_owned();
    rewrite_value(value, &project, &native, &temp_dir)
}

fn rewrite_value(
    value: &serde_json::Value,
    project: &str,
    native: &str,
    temp_dir: &str,
) -> Result<serde_json::Value> {
    Ok(match value {
        serde_json::Value::String(text) => {
            reject_foreign_temp_path(text, project, temp_dir)?;
            serde_json::Value::String(text.replace(project, native))
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| rewrite_value(item, project, native, temp_dir))
                .collect::<Result<Vec<_>>>()?,
        ),
        serde_json::Value::Object(entries) => {
            let mut rewritten = serde_json::Map::new();
            for (key, entry) in entries {
                rewritten.insert(
                    key.clone(),
                    rewrite_value(entry, project, native, temp_dir)?,
                );
            }
            serde_json::Value::Object(rewritten)
        }
        other => other.clone(),
    })
}

fn reject_foreign_temp_path(text: &str, project: &str, temp_dir: &str) -> Result<()> {
    for token in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '=' | ',')) {
        if token.starts_with(temp_dir) && !token.starts_with(project) {
            bail!(
                "APM hook references temporary path '{token}', which will not exist after install"
            )
        }
    }
    Ok(())
}

/// Build one managed placement per captured command, prompt, instruction, or
/// hook file, so each target file is independently owned and restorable.
fn native_placements(native_root: &Path, harness: AgentHarness) -> Result<Vec<ResolvedPlacement>> {
    let files_root = native_root.join(NATIVE_FILES_DIR);
    let mut placements = Vec::new();
    for root in native_roots(harness).iter().filter(|root| root.placed) {
        let relative = root.relative;
        let source_root = files_root.join(relative);
        if !source_root.is_dir() {
            continue;
        }
        let mut entries = fs::read_dir(&source_root)
            .with_context(|| format!("Failed to read {}", source_root.display()))?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort();
        for name in entries {
            placements.push(ResolvedPlacement {
                source: source_root.join(&name),
                target: Path::new(relative).join(&name),
            });
        }
    }
    Ok(placements)
}

/// Build the `.claude/settings.json` merge for captured Claude hooks, owning one
/// JSON pointer per hook event.
fn claude_hooks_merge(
    native_root: &Path,
    harness: AgentHarness,
) -> Result<Option<ResolvedJsonMerge>> {
    if harness != AgentHarness::Claude {
        return Ok(None);
    }
    let path = native_root.join(CLAUDE_HOOKS_FILE);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("Failed to read {}", path.display())),
    };
    let hooks: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse captured Claude hooks {}", path.display()))?;
    if hooks.is_empty() {
        return Ok(None);
    }
    let owned_paths = hooks
        .keys()
        .map(|event| crate::profile_plan::json_pointer(&["hooks", event]))
        .collect();
    Ok(Some(ResolvedJsonMerge {
        target: PathBuf::from(".claude").join("settings.json"),
        value: serde_json::json!({ "hooks": hooks }),
        owned_paths,
    }))
}

fn mirror_git_identity(target: &Path, project: &Path) -> Result<()> {
    let remote = Command::new("git")
        .args([
            "-C",
            &target.to_string_lossy(),
            "config",
            "--get",
            "remote.origin.url",
        ])
        .output()
        .context("Failed to inspect the target repository's Git remote")?;
    if !remote.status.success() {
        return Ok(());
    }
    let remote = String::from_utf8(remote.stdout)
        .context("Target repository remote URL is not valid UTF-8")?;
    let remote = remote.trim();
    if remote.is_empty() {
        return Ok(());
    }
    run_git(project, &["init", "--quiet"])?;
    run_git(project, &["config", "--local", "remote.origin.url", remote])
}

fn run_git(directory: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(directory)
        .status()
        .context("Failed to preserve the target repository's Git identity for APM policy checks")?;
    if status.success() {
        Ok(())
    } else {
        bail!("Failed to preserve the target repository's Git identity for APM policy checks")
    }
}

fn single_bundle(build_dir: &Path) -> Result<PathBuf> {
    let mut bundles = fs::read_dir(build_dir)
        .with_context(|| format!("APM did not create {}", build_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path());
    let bundle = bundles
        .next()
        .ok_or_else(|| anyhow::anyhow!("APM pack produced no plugin bundle"))?;
    if bundles.next().is_some() {
        bail!("APM pack produced more than one plugin bundle")
    }
    Ok(bundle)
}

fn ensure_package_identity(path: &Path, identity: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    ensure_managed_directory(path)?;
    let recorded = fs::read_to_string(path.join(SOURCE_MARKER)).with_context(|| {
        format!(
            "Refusing to replace unrecognized APM artifact {}",
            path.display()
        )
    })?;
    if recorded.trim() != identity {
        bail!(
            "APM package '{identity}' conflicts with installed package '{}'",
            recorded.trim()
        )
    }
    Ok(())
}

fn ensure_managed_directory(path: &Path) -> Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Refusing to replace non-directory path {}", path.display())
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)?;
        }
        Ok(_) => fs::remove_file(path)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    Ok(())
}

fn restore_bundle(backup: &Path, destination: &Path) -> Result<()> {
    if backup.exists() {
        fs::rename(backup, destination).with_context(|| {
            format!(
                "Failed to restore previous bundle {}",
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn validate_package(package: &str) -> Result<()> {
    if package.trim().is_empty()
        || package.trim() != package
        || package.chars().any(char::is_control)
    {
        bail!("APM package reference must be a non-empty, single-line value")
    }
    Ok(())
}

fn package_identity(package: &str) -> &str {
    package
        .split('#')
        .next()
        .unwrap_or(package)
        .trim_end_matches('/')
}

fn package_key(identity: &str) -> Result<String> {
    let key: String = identity
        .trim_end_matches(".git")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let key = key.trim_matches('-').to_string();
    if key.is_empty() || key.len() > 120 {
        bail!("APM package reference does not produce a usable installation name")
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_key_uses_full_package_identity() {
        assert_eq!(
            package_key(package_identity("tylerbutler/apm-base#v0.2.2")).unwrap(),
            "tylerbutler-apm-base"
        );
        assert_eq!(
            package_key(package_identity("https://example.com/org/tools.git")).unwrap(),
            "https---example.com-org-tools"
        );
        assert_ne!(
            package_key(package_identity("alice/tool")).unwrap(),
            package_key(package_identity("bob/tool")).unwrap()
        );
    }

    #[test]
    fn mirror_git_identity_preserves_origin() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("target");
        let project = temp.path().join("project");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&project).unwrap();
        run_git(&target, &["init", "--quiet"]).unwrap();
        run_git(
            &target,
            &[
                "config",
                "--local",
                "remote.origin.url",
                "https://github.com/acme/example.git",
            ],
        )
        .unwrap();

        mirror_git_identity(&target, &project).unwrap();

        let remote = Command::new("git")
            .args([
                "-C",
                &project.to_string_lossy(),
                "remote",
                "get-url",
                "origin",
            ])
            .output()
            .unwrap();
        assert!(remote.status.success());
        assert_eq!(
            String::from_utf8(remote.stdout).unwrap().trim(),
            "https://github.com/acme/example.git"
        );
    }

    #[test]
    fn package_rejects_empty_and_control_characters() {
        assert!(validate_package(" ").is_err());
        assert!(validate_package("owner/package\nother/package").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn install_applies_bundle_without_rewriting_repo_config() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("repo");
        fs::create_dir_all(target.join(".git/info")).unwrap();
        fs::create_dir_all(target.join(".repoverlay")).unwrap();
        let config_path = target.join(".repoverlay/config.ccl");
        let original_config = "# Preserve this comment.\n";
        fs::write(&config_path, original_config).unwrap();
        fs::write(
            target.join("apm.yml"),
            "name: target\n\"policy\" :\n  fetch_failure_default: block\ndependencies:\n  apm: []\n",
        )
        .unwrap();

        let apm = temp.path().join("apm");
        fs::write(
            &apm,
            r#"#!/bin/sh
set -eu
if [ "$1" = install ]; then
    grep -q 'fetch_failure_default: block' apm.yml
    if [ "$3" = copilot ]; then
        mkdir -p .github
        config=.github/mcp.json
        skills_root=.agents
    else
        config=.mcp.json
        skills_root=.claude
    fi
    printf '%s\n' "{\"mcpServers\":{\"demo\":{\"command\":\"$skills_root/skills/demo/bin/server\",\"env\":{\"TOKEN\":\"\${TOKEN}\"},\"headers\":{\"Authorization\":\"Bearer \${TOKEN}\"}}}}" > "$config"
    exit 0
fi
while [ "$1" != "--output" ]; do shift; done
output=$2
test -f .mcp.json
bundle="$output/test-package-1.0.0"
mkdir -p "$bundle/skills/demo/bin" "$bundle/hooks" "$bundle/instructions" "$bundle/bin"
printf '%s\n' '# Demo' > "$bundle/skills/demo/SKILL.md"
printf '%s\n' '#!/bin/sh' > "$bundle/skills/demo/bin/server"
printf '%s\n' '#!/bin/sh' > "$bundle/bin/server"
printf '%s\n' '{"mcpServers":{"demo":{"command":"${CLAUDE_PLUGIN_ROOT}/bin/server"}}}' > "$bundle/.mcp.json"
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&apm).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&apm, permissions).unwrap();

        install_with_apm(
            "owner/test-package#v1.0.0",
            AgentHarness::Claude,
            &target,
            apm.as_os_str(),
            false,
        )
        .unwrap();
        let repeat = install_with_apm(
            "owner/test-package#v1.0.0",
            AgentHarness::Claude,
            &target,
            apm.as_os_str(),
            false,
        )
        .unwrap_err();
        assert!(repeat.to_string().contains("already installed"));
        let other_harness = install_with_apm(
            "owner/test-package#v1.0.0",
            AgentHarness::Copilot,
            &target,
            apm.as_os_str(),
            false,
        )
        .unwrap_err();
        assert!(other_harness.to_string().contains("installed for claude"));
        let mcp_conflict = install_with_apm(
            "other/test-package",
            AgentHarness::Copilot,
            &target,
            apm.as_os_str(),
            false,
        )
        .unwrap_err();
        assert!(format!("{mcp_conflict:#}").contains("already managed"));

        assert!(target.join(".claude/skills/demo/SKILL.md").is_file());
        let artifact_root = crate::state::external_state_dir_for_target(&target)
            .unwrap()
            .join("apm/owner-test-package");
        let artifact = artifact_root.join("claude/owner-test-package");
        assert!(artifact.join("skills/demo/SKILL.md").is_file());
        assert_eq!(fs::read_to_string(config_path).unwrap(), original_config);
        let state = crate::profile::load_profile_state(
            &target,
            "apm-owner-test-package",
            AgentHarness::Claude,
        )
        .unwrap();
        assert!(
            state
                .skipped
                .iter()
                .any(|skip| skip.capability.ends_with(":hooks"))
        );
        assert!(
            state
                .skipped
                .iter()
                .any(|skip| skip.capability.ends_with(":instructions"))
        );

        fs::remove_dir_all(target.join(".repoverlay")).unwrap();
        fs::remove_dir_all(target.join(".claude")).unwrap();
        let restorable = install_with_apm(
            "owner/test-package#v1.0.0",
            AgentHarness::Copilot,
            &target,
            apm.as_os_str(),
            false,
        )
        .unwrap_err();
        assert!(restorable.to_string().contains("can be restored"));
        assert_eq!(crate::profile_plan::restore_profiles(&target).unwrap(), 1);
        assert!(target.join(".claude/skills/demo/SKILL.md").is_file());
        assert!(artifact.join("skills/demo/SKILL.md").is_file());
        let mcp: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target.join(".mcp.json")).unwrap()).unwrap();
        let command = mcp["mcpServers"]["demo"]["command"].as_str().unwrap();
        assert!(target.join(command).is_file());
        assert_eq!(mcp["mcpServers"]["demo"]["env"]["TOKEN"], "${TOKEN}");
        assert_eq!(
            mcp["mcpServers"]["demo"]["headers"]["Authorization"],
            "Bearer ${TOKEN}"
        );
        crate::profile_plan::remove_profile(
            "apm-owner-test-package",
            AgentHarness::Claude,
            &target,
        )
        .unwrap();

        install_with_apm(
            "owner/test-package#v1.0.0",
            AgentHarness::Copilot,
            &target,
            apm.as_os_str(),
            false,
        )
        .unwrap();
        assert!(target.join(".agents/skills/demo/SKILL.md").is_file());
        assert!(
            artifact_root
                .join("copilot/owner-test-package/skills/demo/SKILL.md")
                .is_file()
        );
        let mcp: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["demo"]["env"]["TOKEN"], "${TOKEN}");
        assert_eq!(
            mcp["mcpServers"]["demo"]["headers"]["Authorization"],
            "Bearer ${TOKEN}"
        );
        crate::profile_plan::remove_profile(
            "apm-owner-test-package",
            AgentHarness::Copilot,
            &target,
        )
        .unwrap();
        remove_if_exists(artifact_root.parent().unwrap()).unwrap();
    }

    /// An APM stub that writes target-native commands, instructions, and hooks
    /// for both harnesses, plus a bundle that carries the same capabilities.
    #[cfg(unix)]
    fn native_apm_stub(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let apm = dir.join("apm-native");
        fs::write(
            &apm,
            r#"#!/bin/sh
set -eu
if [ "$1" = install ]; then
    if [ "$3" = copilot ]; then
        mkdir -p .github/prompts .github/instructions .github/hooks
        printf '%s\n' '# prompt' > .github/prompts/demo.prompt.md
        printf '%s\n' '---' 'applyTo: "**"' '---' 'rule body' > .github/instructions/demo.instructions.md
        printf '%s\n' '#!/bin/sh' > .github/hooks/demo.sh
        chmod +x .github/hooks/demo.sh
        printf '{"hooks":[{"command":"%s/.github/hooks/demo.sh"}]}' "$PWD" > .github/hooks/demo.json
    else
        mkdir -p .claude/commands .claude/rules .claude/hooks
        printf '%s\n' '# command' > .claude/commands/demo.md
        printf '%s\n' 'rule body' > .claude/rules/demo.md
        printf '%s\n' '#!/bin/sh' > .claude/hooks/run.sh
        chmod +x .claude/hooks/run.sh
        printf '{"model":"temporary","hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"%s/.claude/hooks/run.sh"}]}]}}' "$PWD" > .claude/settings.json
    fi
    exit 0
fi
while [ "$1" != "--output" ]; do shift; done
output=$2
bundle="$output/native-package-1.0.0"
mkdir -p "$bundle/hooks" "$bundle/commands" "$bundle/instructions"
printf '%s\n' 'x' > "$bundle/hooks/a.sh"
printf '%s\n' 'x' > "$bundle/commands/a.md"
printf '%s\n' 'x' > "$bundle/instructions/a.md"
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&apm).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&apm, permissions).unwrap();
        apm
    }

    #[cfg(unix)]
    fn native_target(dir: &Path) -> PathBuf {
        let target = dir.join("repo");
        fs::create_dir_all(target.join(".git/info")).unwrap();
        fs::create_dir_all(target.join(".repoverlay")).unwrap();
        target
    }

    #[cfg(unix)]
    #[test]
    fn install_applies_claude_commands_rules_and_hooks() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = native_target(temp.path());
        let apm = native_apm_stub(temp.path());
        fs::create_dir_all(target.join(".claude/commands")).unwrap();
        fs::write(target.join(".claude/commands/demo.md"), "user command\n").unwrap();
        fs::write(
            target.join(".claude/settings.json"),
            "{\"model\":\"user\"}\n",
        )
        .unwrap();

        let refused = install_with_apm(
            "owner/native-package",
            AgentHarness::Claude,
            &target,
            apm.as_os_str(),
            false,
        )
        .unwrap_err();
        assert!(refused.to_string().contains("--allow-hooks"));
        assert_eq!(
            fs::read_to_string(target.join(".claude/commands/demo.md")).unwrap(),
            "user command\n"
        );

        install_with_apm(
            "owner/native-package",
            AgentHarness::Claude,
            &target,
            apm.as_os_str(),
            true,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(target.join(".claude/commands/demo.md")).unwrap(),
            "# command\n"
        );
        assert_eq!(
            fs::read_to_string(target.join(".claude/rules/demo.md")).unwrap(),
            "rule body\n"
        );
        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["model"], "user");
        let command = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(
            Path::new(command).is_file(),
            "hook script {command} is missing"
        );

        let state = crate::profile::load_profile_state(
            &target,
            "apm-owner-native-package",
            AgentHarness::Claude,
        )
        .unwrap();
        assert!(
            state.skipped.is_empty(),
            "captured capabilities must not be reported as skipped: {:?}",
            state.skipped
        );

        crate::profile_plan::remove_profile(
            "apm-owner-native-package",
            AgentHarness::Claude,
            &target,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(target.join(".claude/commands/demo.md")).unwrap(),
            "user command\n"
        );
        assert!(!target.join(".claude/rules/demo.md").exists());
        let settings: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["model"], "user");
        assert!(settings.get("hooks").is_none_or(serde_json::Value::is_null));
        remove_if_exists(
            &crate::state::external_state_dir_for_target(&target)
                .unwrap()
                .join("apm"),
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn install_applies_copilot_prompts_instructions_and_hooks() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::TempDir::new().unwrap();
        let target = native_target(temp.path());
        let apm = native_apm_stub(temp.path());

        install_with_apm(
            "owner/native-package",
            AgentHarness::Copilot,
            &target,
            apm.as_os_str(),
            true,
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(target.join(".github/prompts/demo.prompt.md")).unwrap(),
            "# prompt\n"
        );
        assert_eq!(
            fs::read_to_string(target.join(".github/instructions/demo.instructions.md")).unwrap(),
            "---\napplyTo: \"**\"\n---\nrule body\n"
        );
        let hook: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target.join(".github/hooks/demo.json")).unwrap(),
        )
        .unwrap();
        let command = hook["hooks"][0]["command"].as_str().unwrap();
        assert!(
            Path::new(command).is_file(),
            "hook script {command} is missing"
        );
        assert_eq!(
            fs::metadata(command).unwrap().permissions().mode() & 0o111,
            0o111
        );

        // Restore rebuilds the placements after `git clean` removes them.
        fs::remove_dir_all(target.join(".repoverlay")).unwrap();
        fs::remove_dir_all(target.join(".github")).unwrap();
        assert_eq!(crate::profile_plan::restore_profiles(&target).unwrap(), 1);
        assert_eq!(
            fs::read_to_string(target.join(".github/prompts/demo.prompt.md")).unwrap(),
            "# prompt\n"
        );
        assert!(target.join(".github/hooks/demo.json").is_file());

        crate::profile_plan::remove_profile(
            "apm-owner-native-package",
            AgentHarness::Copilot,
            &target,
        )
        .unwrap();
        assert!(!target.join(".github/prompts/demo.prompt.md").exists());
        assert!(!target.join(".github/hooks/demo.json").exists());
        remove_if_exists(
            &crate::state::external_state_dir_for_target(&target)
                .unwrap()
                .join("apm"),
        )
        .unwrap();
    }

    #[test]
    fn hook_paths_outside_the_temporary_project_are_rejected() {
        let temp_dir = std::env::temp_dir();
        let project = temp_dir.join("repoverlay-apm-project");
        let native = temp_dir.join("durable");
        let value = serde_json::json!({
            "command": format!("{}/other/run.sh", temp_dir.display()),
        });
        assert!(rewrite_project_paths(&value, &project, &native).is_err());

        let value = serde_json::json!({ "command": "./relative.sh" });
        assert_eq!(
            rewrite_project_paths(&value, &project, &native).unwrap(),
            value
        );
    }
}
