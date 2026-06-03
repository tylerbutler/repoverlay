use anyhow::{Context, Result, bail};
use colored::Colorize;
use std::path::PathBuf;

use super::super::PluginCommand;

/// Handle plugin subcommands.
pub(crate) fn handle_plugin_command(command: PluginCommand) -> Result<()> {
    match command {
        PluginCommand::New { name } => new_plugin(&name),
    }
}

/// Validate that a plugin name is safe to use as a directory component.
fn validate_plugin_name(name: &str) -> Result<()> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains('/') || name.contains('\\') {
        bail!(
            "Invalid plugin name {name:?}: must not be empty, '.', '..', or contain path separators"
        );
    }
    Ok(())
}

/// Scaffold a new plugin directory with a manifest and an MCP stub.
fn new_plugin(name: &str) -> Result<()> {
    validate_plugin_name(name)?;

    let plugin_dir = PathBuf::from(name);
    if plugin_dir.exists() {
        bail!("Refusing to scaffold plugin: directory '{name}' already exists");
    }

    let manifest_dir = plugin_dir.join(".claude-plugin");
    std::fs::create_dir_all(&manifest_dir)
        .with_context(|| format!("Failed to create {}", manifest_dir.display()))?;

    let manifest = serde_json::json!({
        "name": name,
        "version": "0.1.0",
        "description": "",
    });
    let manifest_path = manifest_dir.join("plugin.json");
    std::fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )
    .with_context(|| format!("Failed to write {}", manifest_path.display()))?;

    let mcp = serde_json::json!({ "mcpServers": {} });
    let mcp_path = plugin_dir.join(".mcp.json");
    std::fs::write(
        &mcp_path,
        format!("{}\n", serde_json::to_string_pretty(&mcp)?),
    )
    .with_context(|| format!("Failed to write {}", mcp_path.display()))?;

    println!(
        "{} Scaffolded plugin '{}' at {}",
        "✓".green().bold(),
        name.bold(),
        plugin_dir.display()
    );
    println!("  - {}", manifest_path.display());
    println!("  - {}", mcp_path.display());

    Ok(())
}
