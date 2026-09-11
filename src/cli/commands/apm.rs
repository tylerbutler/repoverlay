use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::ApmCommand;
use crate::cli::commands::resolve_target;
use crate::config;
use crate::plugin::PluginRef;
use crate::profile::ProfileConfig;
use crate::profile_applicators::AgentHarness;

pub(crate) fn handle_apm_command(command: ApmCommand) -> Result<()> {
    match command {
        ApmCommand::Install {
            package,
            harness,
            target,
        } => install(&package, harness, target),
    }
}

fn install(package: &str, harness: AgentHarness, target: Option<PathBuf>) -> Result<()> {
    validate_package(package)?;
    let target = resolve_target(target)?;
    let slug = package_slug(package);
    let profile_name = format!("apm-{slug}");
    let source = format!(".repoverlay/apm/{slug}");

    let temp = tempfile::tempdir().context("Failed to create temporary APM project")?;
    fs::write(
        temp.path().join("apm.yml"),
        format!(
            "name: repoverlay-{slug}\nversion: 1.0.0\ntargets:\n  - claude\ndependencies:\n  apm:\n    - {package}\n"
        ),
    )?;

    run_apm(temp.path(), &["install"], "install")?;
    let build_dir = temp.path().join("build");
    let output = build_dir.to_string_lossy().into_owned();
    run_apm(
        temp.path(),
        &["pack", "--format", "claude-plugin", "--output", &output],
        "pack",
    )?;

    let bundle = single_bundle(&build_dir)?;
    let destination = target.join(&source);
    crate::path_safety::check_no_symlink_ancestors(&target, Path::new(".repoverlay/apm"))?;

    let mut repo_config = config::load_repo_config(&target)?.unwrap_or_default();
    if repo_config.profiles.contains_key(&profile_name) {
        let applied = crate::profile_plan::list_profile_states(&target)?
            .into_iter()
            .any(|state| state.name == profile_name && state.harness == harness);
        if applied {
            crate::profile_plan::remove_profile(&profile_name, harness, &target)?;
        }
        repo_config.profiles.remove(&profile_name);
    }

    if destination.exists() {
        fs::remove_dir_all(&destination)
            .with_context(|| format!("Failed to replace {}", destination.display()))?;
    }
    crate::overlay_repo::copy_dir_recursive(&bundle, &destination)?;

    repo_config.profiles.insert(
        profile_name.clone(),
        ProfileConfig {
            description: Some(format!("APM package {package}")),
            plugins: vec![PluginRef::Local {
                source: PathBuf::from(&source),
            }],
            ..ProfileConfig::default()
        },
    );
    config::save_repo_config(&target, &repo_config)?;
    crate::profile_plan::apply_profile(
        &profile_name,
        harness,
        &target,
        crate::profile::ProfileMode::Persistent,
        None,
    )?;
    println!("Installed APM package '{package}' as profile '{profile_name}'");
    Ok(())
}

fn run_apm(directory: &Path, args: &[&str], operation: &str) -> Result<()> {
    let output = Command::new("apm")
        .args(args)
        .current_dir(directory)
        .output()
        .with_context(|| format!("Failed to run 'apm {operation}'; is APM installed?"))?;
    if output.status.success() {
        return Ok(());
    }
    let details = String::from_utf8_lossy(&output.stderr);
    bail!("APM {operation} failed: {}", details.trim())
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

fn validate_package(package: &str) -> Result<()> {
    if package.is_empty() || package.chars().any(char::is_control) || package.contains('\n') {
        bail!("APM package reference must be a non-empty, single-line value")
    }
    Ok(())
}

fn package_slug(package: &str) -> String {
    let package = package.rsplit('/').next().unwrap_or(package);
    let package = package.split('#').next().unwrap_or(package);
    package
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_slug_uses_package_name() {
        assert_eq!(package_slug("tylerbutler/apm-base#v0.2.2"), "apm-base");
    }
}
