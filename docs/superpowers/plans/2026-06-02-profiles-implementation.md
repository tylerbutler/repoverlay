# Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build first-class profiles that merge global/repo config, apply profile overlays and GitHub Copilot harness assets, track profile state, and support ephemeral `repoverlay copilot --profile <name>` execution.

**Architecture:** Add focused profile modules for config/state, planning, and harness applicators. Keep overlay application in existing overlay code and let the Copilot applicator handle harness-specific MCP, instruction, and command behavior. CLI wiring stays in `src/cli/mod.rs` with command handlers in `src/cli/commands/profile.rs` and `src/cli/commands/copilot.rs`.

**Tech Stack:** Rust 2024, clap derive, serde + sickle CCL, anyhow, directories, assert_cmd integration tests, existing `just test` / `just check` commands.

---

## File structure

- Create `src/profile.rs`: profile config structs, merge rules, state structs, state path helpers, save/load/remove functions.
- Create `src/profile_plan.rs`: profile planning context, plan/action types, profile apply/remove/run orchestration.
- Create `src/profile_applicators/mod.rs`: `ProfileApplicator` trait, `AgentHarness`, capability types, applicator registry.
- Create `src/profile_applicators/copilot.rs`: GitHub Copilot applicator, harness-specific paths, MCP merge planning, instruction planning, command construction.
- Create `src/cli/commands/profile.rs`: `profile list/show/apply/status/remove` handlers.
- Create `src/cli/commands/copilot.rs`: `repoverlay copilot --profile <name> -- <extra args>` handler.
- Modify `src/lib.rs`: register new modules and re-export internal profile functions used by CLI.
- Modify `src/config.rs`: add `profiles` to `RepoverlayConfig`, parse/save it, merge global and repo-local same-name profiles by type.
- Modify `src/cli/mod.rs`: add `ProfileCommand`, `Copilot` command, and dispatch.
- Modify `src/cli/commands/mod.rs`: expose new command modules.
- Modify `tests/common/mod.rs`: add helpers for isolated profile config and profile state assertions.
- Modify `tests/cli.rs`: add profile CLI integration tests.

V1 Copilot paths for tests and deterministic behavior:

- `REPOVERLAY_COPILOT_HOME` overrides the Copilot harness home.
- Without override, use `~/.config/github-copilot`.
- MCP target: `<copilot-home>/mcp.json`.
- Instruction target: `<copilot-home>/instructions/<profile-name>/<source-file-name>`.
- Copilot executable: `REPOVERLAY_COPILOT_COMMAND` when set, otherwise `copilot`.

## Task 1: Profile config model and merge rules

**Files:**
- Create: `src/profile.rs`
- Modify: `src/lib.rs:5-29`
- Modify: `src/config.rs:14-24, 467-483, 485-512`
- Test: `src/profile.rs`
- Test: `src/config.rs`

- [ ] **Step 1: Write failing profile parsing and merge tests**

Add this test module to the new `src/profile.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parses_profile_config_from_ccl() {
        let ccl = r"
profiles =
  rust-dev =
    description = Rust profile
    overlays =
      = rust-base
    instructions =
      =
        source = copilot-instructions.md
    mcps =
      servers =
        rust =
          command = uvx
          args =
            = mcp-rust
    skills =
      = market:rust-reviewer@playground
    plugins =
      = market:rust-dev@playground
";

        let config: crate::config::RepoverlayConfig = sickle::from_str(ccl).unwrap();
        let profile = config.profiles.get("rust-dev").unwrap();
        assert_eq!(profile.description.as_deref(), Some("Rust profile"));
        assert_eq!(profile.overlays, vec!["rust-base"]);
        assert_eq!(profile.instructions[0].source, "copilot-instructions.md");
        assert_eq!(profile.mcps.servers["rust"].command, "uvx");
        assert_eq!(profile.mcps.servers["rust"].args, vec!["mcp-rust"]);
        assert_eq!(profile.skills, vec!["market:rust-reviewer@playground"]);
        assert_eq!(profile.plugins, vec!["market:rust-dev@playground"]);
    }

    #[test]
    fn merge_profile_uses_type_based_rules() {
        let mut base_servers = BTreeMap::new();
        base_servers.insert(
            "shared".to_string(),
            McpServerConfig {
                command: "base-command".to_string(),
                args: vec!["base".to_string()],
                env: BTreeMap::new(),
            },
        );
        base_servers.insert(
            "base-only".to_string(),
            McpServerConfig {
                command: "base-only-command".to_string(),
                args: Vec::new(),
                env: BTreeMap::new(),
            },
        );

        let mut override_servers = BTreeMap::new();
        override_servers.insert(
            "shared".to_string(),
            McpServerConfig {
                command: "override-command".to_string(),
                args: vec!["override".to_string()],
                env: BTreeMap::new(),
            },
        );

        let base = ProfileConfig {
            description: Some("Base".to_string()),
            overlays: vec!["base-overlay".to_string()],
            instructions: vec![InstructionConfig {
                source: "base.md".to_string(),
            }],
            mcps: McpConfig {
                servers: base_servers,
            },
            skills: vec!["base-skill".to_string()],
            plugins: vec!["base-plugin".to_string()],
        };
        let overlay = ProfileConfig {
            description: None,
            overlays: vec!["repo-overlay".to_string()],
            instructions: Vec::new(),
            mcps: McpConfig {
                servers: override_servers,
            },
            skills: Vec::new(),
            plugins: vec!["repo-plugin".to_string()],
        };

        let merged = merge_profile_config(&base, &overlay);
        assert_eq!(merged.description.as_deref(), Some("Base"));
        assert_eq!(merged.overlays, vec!["repo-overlay"]);
        assert_eq!(merged.instructions[0].source, "base.md");
        assert_eq!(merged.mcps.servers["shared"].command, "override-command");
        assert_eq!(merged.mcps.servers["base-only"].command, "base-only-command");
        assert_eq!(merged.skills, vec!["base-skill"]);
        assert_eq!(merged.plugins, vec!["repo-plugin"]);
    }
}
```

Add this config-level test to `src/config.rs` inside the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_load_config_merges_same_name_profiles() {
    let temp = TempDir::new().unwrap();
    let repo_config_dir = temp.path().join(".repoverlay");
    fs::create_dir_all(&repo_config_dir).unwrap();

    let repo_ccl = r"
profiles =
  rust-dev =
    overlays =
      = repo-rust
    mcps =
      servers =
        repo =
          command = repo-mcp
";
    fs::write(repo_config_dir.join("config.ccl"), repo_ccl).unwrap();

    let global = RepoverlayConfig {
        profiles: std::collections::BTreeMap::from([(
            "rust-dev".to_string(),
            crate::profile::ProfileConfig {
                description: Some("Global Rust".to_string()),
                overlays: vec!["global-rust".to_string()],
                instructions: vec![crate::profile::InstructionConfig {
                    source: "global.md".to_string(),
                }],
                mcps: crate::profile::McpConfig {
                    servers: std::collections::BTreeMap::from([(
                        "global".to_string(),
                        crate::profile::McpServerConfig {
                            command: "global-mcp".to_string(),
                            args: Vec::new(),
                            env: std::collections::BTreeMap::new(),
                        },
                    )]),
                },
                skills: vec!["global-skill".to_string()],
                plugins: vec!["global-plugin".to_string()],
            },
        )]),
        ..RepoverlayConfig::default()
    };
    let repo = load_repo_config(temp.path()).unwrap().unwrap();
    let merged = merge_repo_config(global, repo);
    let profile = merged.profiles.get("rust-dev").unwrap();

    assert_eq!(profile.description.as_deref(), Some("Global Rust"));
    assert_eq!(profile.overlays, vec!["repo-rust"]);
    assert!(profile.mcps.servers.contains_key("global"));
    assert!(profile.mcps.servers.contains_key("repo"));
    assert_eq!(profile.instructions[0].source, "global.md");
    assert_eq!(profile.skills, vec!["global-skill"]);
    assert_eq!(profile.plugins, vec!["global-plugin"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test profile::tests::parses_profile_config_from_ccl profile::tests::merge_profile_uses_type_based_rules config::tests::test_load_config_merges_same_name_profiles
```

Expected: compile failure because `crate::profile`, `RepoverlayConfig::profiles`, and `merge_repo_config` do not exist.

- [ ] **Step 3: Add profile structs and merge logic**

Create `src/profile.rs`:

```rust
//! Profile configuration, merge rules, and state metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct ProfileConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) overlays: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) instructions: Vec<InstructionConfig>,
    #[serde(skip_serializing_if = "McpConfig::is_empty")]
    pub(crate) mcps: McpConfig,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) skills: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) plugins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct InstructionConfig {
    pub(crate) source: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct McpConfig {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) servers: BTreeMap<String, McpServerConfig>,
}

impl McpConfig {
    pub(crate) fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct McpServerConfig {
    pub(crate) command: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
}

pub(crate) fn merge_profile_config(
    base: &ProfileConfig,
    override_profile: &ProfileConfig,
) -> ProfileConfig {
    let mut mcps = base.mcps.clone();
    for (name, server) in &override_profile.mcps.servers {
        mcps.servers.insert(name.clone(), server.clone());
    }

    ProfileConfig {
        description: override_profile
            .description
            .clone()
            .or_else(|| base.description.clone()),
        overlays: merge_list(&base.overlays, &override_profile.overlays),
        instructions: merge_list(&base.instructions, &override_profile.instructions),
        mcps,
        skills: merge_list(&base.skills, &override_profile.skills),
        plugins: merge_list(&base.plugins, &override_profile.plugins),
    }
}

fn merge_list<T: Clone>(base: &[T], override_list: &[T]) -> Vec<T> {
    if override_list.is_empty() {
        base.to_vec()
    } else {
        override_list.to_vec()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProfileMode {
    Persistent,
    Ephemeral,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ProfileState {
    pub(crate) name: String,
    pub(crate) harness: String,
    pub(crate) mode: ProfileMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    pub(crate) applied_at: DateTime<Utc>,
    pub(crate) profile_fingerprint: String,
    #[serde(default)]
    pub(crate) overlays: Vec<String>,
    #[serde(default)]
    pub(crate) files: Vec<ProfileFileEntry>,
    #[serde(default)]
    pub(crate) skipped: Vec<SkippedCapability>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ProfileFileEntry {
    pub(crate) source: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) scope: ProfileScope,
    pub(crate) action: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProfileScope {
    Repo,
    User,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct SkippedCapability {
    pub(crate) capability: String,
    pub(crate) reason: String,
}
```

Modify `src/lib.rs` module list:

```rust
mod profile;
mod profile_applicators;
mod profile_plan;
```

Add to `RepoverlayConfig` in `src/config.rs`:

```rust
    /// Named profile definitions.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) profiles: std::collections::BTreeMap<String, crate::profile::ProfileConfig>,
```

Add a merge helper in `src/config.rs`:

```rust
pub(crate) fn merge_repo_config(
    mut global: RepoverlayConfig,
    repo_config: RepoverlayConfig,
) -> RepoverlayConfig {
    let mut merged_sources = repo_config.sources;
    merged_sources.extend(global.sources);
    global.sources = merged_sources;

    if repo_config.library_path.is_some() {
        global.library_path = repo_config.library_path;
    }

    for (name, repo_profile) in repo_config.profiles {
        let merged_profile = global.profiles.get(&name).map_or(repo_profile.clone(), |base| {
            crate::profile::merge_profile_config(base, &repo_profile)
        });
        global.profiles.insert(name, merged_profile);
    }

    global
}
```

Update `load_config` in `src/config.rs`:

```rust
pub(crate) fn load_config(repo_path: Option<&Path>) -> Result<RepoverlayConfig> {
    let config = load_global_config()?;

    if let Some(repo_root) = repo_path
        && let Some(repo_config) = load_repo_config(repo_root)?
    {
        return Ok(merge_repo_config(config, repo_config));
    }

    Ok(config)
}
```

Update `generate_sources_config_ccl` to serialize the full config so profile definitions are not dropped:

```rust
pub(crate) fn generate_sources_config_ccl(config: &RepoverlayConfig) -> String {
    sickle::to_string(config).expect("RepoverlayConfig serialization should not fail")
}
```

Update existing `RepoverlayConfig { ... }` literals in `src/config.rs` tests to include
`profiles: std::collections::BTreeMap::new()` or use `..RepoverlayConfig::default()` so the new
field is always initialized.

- [ ] **Step 4: Run profile config tests**

Run:

```bash
cargo test profile::tests::parses_profile_config_from_ccl profile::tests::merge_profile_uses_type_based_rules config::tests::test_load_config_merges_same_name_profiles
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/profile.rs src/config.rs
git commit -m "feat: add profile config model"
```

## Task 2: Profile list and show commands

**Files:**
- Create: `src/cli/commands/profile.rs`
- Modify: `src/cli/commands/mod.rs:1-8`
- Modify: `src/cli/mod.rs:20-30, 46-540, 705-990`
- Test: `tests/cli.rs`
- Test: `tests/common/mod.rs`

- [ ] **Step 1: Write failing CLI tests**

Add helpers to `tests/common/mod.rs`:

```rust
impl TestContext {
    pub fn write_repo_config(&self, content: &str) {
        let config_dir = self.repo.path().join(".repoverlay");
        fs::create_dir_all(&config_dir).expect("Failed to create repo config dir");
        fs::write(config_dir.join("config.ccl"), content).expect("Failed to write repo config");
    }
}
```

Add tests to `tests/cli.rs`:

```rust
#[test]
fn profile_list_shows_repo_profiles() {
    let ctx = TestContext::new();
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    description = Rust development
",
    );

    cargo_bin_cmd!("repoverlay")
        .args(["profile", "list", "--target", ctx.repo_path().to_str().unwrap()])
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("rust-dev"))
        .stdout(predicate::str::contains("Rust development"));
}

#[test]
fn profile_show_prints_profile_details() {
    let ctx = TestContext::new();
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    description = Rust development
    overlays =
      = rust-base
    skills =
      = market:rust-reviewer@playground
",
    );

    cargo_bin_cmd!("repoverlay")
        .args(["profile", "show", "rust-dev", "--target", ctx.repo_path().to_str().unwrap()])
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("rust-dev"))
        .stdout(predicate::str::contains("rust-base"))
        .stdout(predicate::str::contains("market:rust-reviewer@playground"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test profile_list_shows_repo_profiles profile_show_prints_profile_details
```

Expected: FAIL with clap error containing `unrecognized subcommand 'profile'`.

- [ ] **Step 3: Add profile command types and handlers**

Create `src/cli/commands/profile.rs`:

```rust
use anyhow::{Result, bail};
use colored::Colorize;
use std::path::PathBuf;

use crate::cli::ProfileCommand;
use crate::config;

pub(crate) fn handle_profile_command(command: ProfileCommand) -> Result<()> {
    match command {
        ProfileCommand::List { target } => {
            let config = config::load_config(target.as_deref())?;
            if config.profiles.is_empty() {
                println!("No profiles configured.");
                return Ok(());
            }
            for (name, profile) in &config.profiles {
                match &profile.description {
                    Some(description) => println!("{} - {description}", name.bold()),
                    None => println!("{}", name.bold()),
                }
            }
            Ok(())
        }
        ProfileCommand::Show { name, target } => {
            let config = config::load_config(target.as_deref())?;
            let profile = config
                .profiles
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("Profile '{name}' not found"))?;

            println!("{}", name.bold());
            if let Some(description) = &profile.description {
                println!("  Description: {description}");
            }
            print_list("Overlays", &profile.overlays);
            print_list(
                "Instructions",
                &profile
                    .instructions
                    .iter()
                    .map(|entry| entry.source.clone())
                    .collect::<Vec<_>>(),
            );
            if !profile.mcps.servers.is_empty() {
                println!("  MCP servers:");
                for (server, config) in &profile.mcps.servers {
                    println!("    - {} ({})", server, config.command);
                }
            }
            print_list("Skills", &profile.skills);
            print_list("Plugins", &profile.plugins);
            Ok(())
        }
        ProfileCommand::Apply { .. }
        | ProfileCommand::Status { .. }
        | ProfileCommand::Remove { .. } => {
            bail!("profile apply/status/remove are added in the profile lifecycle task")
        }
    }
}

fn print_list(label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    println!("  {label}:");
    for value in values {
        println!("    - {value}");
    }
}
```

Modify `src/cli/commands/mod.rs`:

```rust
pub(crate) mod profile;
```

Modify `src/cli/mod.rs` imports:

```rust
pub(crate) use commands::profile::handle_profile_command;
```

Add command variants in `src/cli/mod.rs`:

```rust
    /// Manage profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
```

Add the subcommand enum near `SourceCommand`:

```rust
#[derive(Subcommand)]
pub(crate) enum ProfileCommand {
    /// List configured profiles
    List {
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
    /// Show a configured profile
    Show {
        /// Profile name
        name: String,
        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
    /// Apply a profile persistently
    Apply {
        name: String,
        #[arg(long)]
        harness: String,
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
    /// Show applied profile state
    Status {
        #[arg(short, long)]
        target: Option<PathBuf>,
        #[arg(long)]
        harness: Option<String>,
    },
    /// Remove an applied profile
    Remove {
        name: String,
        #[arg(long)]
        harness: String,
        #[arg(short, long)]
        target: Option<PathBuf>,
    },
}
```

Add dispatch:

```rust
        Commands::Profile { command } => {
            handle_profile_command(command)?;
        }
```

- [ ] **Step 4: Run list/show tests**

Run:

```bash
cargo test profile_list_shows_repo_profiles profile_show_prints_profile_details
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cli/mod.rs src/cli/commands/mod.rs src/cli/commands/profile.rs tests/common/mod.rs tests/cli.rs
git commit -m "feat: add profile list and show"
```

## Task 3: Profile plan model and Copilot applicator

**Files:**
- Create: `src/profile_plan.rs`
- Create: `src/profile_applicators/mod.rs`
- Create: `src/profile_applicators/copilot.rs`
- Modify: `src/lib.rs:5-43`
- Test: `src/profile_plan.rs`
- Test: `src/profile_applicators/copilot.rs`

- [ ] **Step 1: Write failing unit tests**

Add this test module to `src/profile_applicators/copilot.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{InstructionConfig, McpConfig, McpServerConfig, ProfileConfig};
    use crate::profile_applicators::{ProfileApplicator, ProfileContext};
    use std::collections::BTreeMap;

    #[test]
    fn copilot_plans_mcp_merge_and_instruction_write() {
        let temp = tempfile::TempDir::new().unwrap();
        let source_dir = temp.path().join("profile-assets");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("copilot-instructions.md"), "Be concise.").unwrap();

        let profile = ProfileConfig {
            instructions: vec![InstructionConfig {
                source: "copilot-instructions.md".to_string(),
            }],
            mcps: McpConfig {
                servers: BTreeMap::from([(
                    "rust".to_string(),
                    McpServerConfig {
                        command: "uvx".to_string(),
                        args: vec!["mcp-rust".to_string()],
                        env: BTreeMap::new(),
                    },
                )]),
            },
            ..ProfileConfig::default()
        };
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: temp.path().to_path_buf(),
            profile_asset_dir: source_dir,
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Persistent,
            session_id: None,
        };

        let plan = CopilotApplicator.plan(&profile, &context).unwrap();
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            crate::profile_plan::ProfileAction::MergeJson { target, .. }
                if target.ends_with("mcp.json")
        )));
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            crate::profile_plan::ProfileAction::WriteFile { target, .. }
                if target.ends_with("instructions/rust-dev/copilot-instructions.md")
        )));
    }

    #[test]
    fn copilot_command_uses_env_override_and_extra_args() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = ProfileContext {
            profile_name: "rust-dev".to_string(),
            target: temp.path().to_path_buf(),
            profile_asset_dir: temp.path().to_path_buf(),
            harness_home: temp.path().join("copilot-home"),
            mode: crate::profile::ProfileMode::Ephemeral,
            session_id: Some("session-1".to_string()),
        };

        let command = CopilotApplicator
            .command_with_program(&context, "echo", &["hello".to_string()])
            .unwrap();
        assert_eq!(command.get_program(), "echo");
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect::<Vec<_>>(),
            vec!["hello"]
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test profile_applicators::copilot::tests::copilot_plans_mcp_merge_and_instruction_write profile_applicators::copilot::tests::copilot_command_uses_env_override_and_extra_args
```

Expected: compile failure because plan/applicator modules do not exist.

- [ ] **Step 3: Add plan and applicator modules**

Create `src/profile_applicators/mod.rs`:

```rust
pub(crate) mod copilot;

use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

use crate::profile::{ProfileConfig, ProfileMode};
use crate::profile_plan::ProfilePlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentHarness {
    Copilot,
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileContext {
    pub(crate) profile_name: String,
    pub(crate) target: PathBuf,
    pub(crate) profile_asset_dir: PathBuf,
    pub(crate) harness_home: PathBuf,
    pub(crate) mode: ProfileMode,
    pub(crate) session_id: Option<String>,
}

pub(crate) trait ProfileApplicator {
    fn harness(&self) -> AgentHarness;
    fn plan(&self, profile: &ProfileConfig, context: &ProfileContext) -> Result<ProfilePlan>;
    fn command(&self, context: &ProfileContext, extra_args: &[String]) -> Result<Command>;
}
```

Create `src/profile_plan.rs`:

```rust
use serde_json::Value;
use std::path::PathBuf;

use crate::profile::ProfileScope;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProfilePlan {
    pub(crate) profile_name: String,
    pub(crate) harness: String,
    pub(crate) actions: Vec<ProfileAction>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProfileAction {
    ApplyOverlay { reference: String },
    WriteFile {
        source: PathBuf,
        target: PathBuf,
        scope: ProfileScope,
    },
    MergeJson {
        target: PathBuf,
        value: Value,
        scope: ProfileScope,
    },
    SkipCapability {
        capability: String,
        reason: String,
    },
}
```

Create `src/profile_applicators/copilot.rs`:

```rust
use anyhow::{Context, Result};
use serde_json::json;
use std::process::Command;

use crate::profile::{ProfileConfig, ProfileScope};
use crate::profile_applicators::{AgentHarness, ProfileApplicator, ProfileContext};
use crate::profile_plan::{ProfileAction, ProfilePlan};

pub(crate) struct CopilotApplicator;

impl CopilotApplicator {
    pub(crate) fn harness_home_from_env() -> Result<std::path::PathBuf> {
        if let Some(home) = std::env::var_os("REPOVERLAY_COPILOT_HOME") {
            return Ok(home.into());
        }
        let config_dir = crate::config::config_dir()?;
        Ok(config_dir.join("github-copilot"))
    }

    pub(crate) fn command_with_program(
        &self,
        _context: &ProfileContext,
        program: &str,
        extra_args: &[String],
    ) -> Result<Command> {
        let mut command = Command::new(program);
        command.args(extra_args);
        Ok(command)
    }
}

impl ProfileApplicator for CopilotApplicator {
    fn harness(&self) -> AgentHarness {
        AgentHarness::Copilot
    }

    fn plan(&self, profile: &ProfileConfig, context: &ProfileContext) -> Result<ProfilePlan> {
        let mut actions = Vec::new();

        for overlay in &profile.overlays {
            actions.push(ProfileAction::ApplyOverlay {
                reference: overlay.clone(),
            });
        }

        if !profile.mcps.servers.is_empty() {
            let mut servers = serde_json::Map::new();
            for (name, server) in &profile.mcps.servers {
                servers.insert(
                    name.clone(),
                    json!({
                        "command": server.command,
                        "args": server.args,
                        "env": server.env,
                    }),
                );
            }
            actions.push(ProfileAction::MergeJson {
                target: context.harness_home.join("mcp.json"),
                value: json!({ "servers": servers }),
                scope: ProfileScope::User,
            });
        }

        for instruction in &profile.instructions {
            let source = context.profile_asset_dir.join(&instruction.source);
            let file_name = source
                .file_name()
                .context("Instruction source has no file name")?;
            actions.push(ProfileAction::WriteFile {
                source,
                target: context
                    .harness_home
                    .join("instructions")
                    .join(&context.profile_name)
                    .join(file_name),
                scope: ProfileScope::User,
            });
        }

        if !profile.skills.is_empty() {
            actions.push(ProfileAction::SkipCapability {
                capability: "skills".to_string(),
                reason: "GitHub Copilot skill placement is not defined in v1".to_string(),
            });
        }
        if !profile.plugins.is_empty() {
            actions.push(ProfileAction::SkipCapability {
                capability: "plugins".to_string(),
                reason: "GitHub Copilot plugin placement is not defined in v1".to_string(),
            });
        }

        Ok(ProfilePlan {
            profile_name: context.profile_name.clone(),
            harness: "copilot".to_string(),
            actions,
        })
    }

    fn command(&self, context: &ProfileContext, extra_args: &[String]) -> Result<Command> {
        let program =
            std::env::var("REPOVERLAY_COPILOT_COMMAND").unwrap_or_else(|_| "copilot".to_string());
        self.command_with_program(context, &program, extra_args)
    }
}
```

Ensure `src/lib.rs` includes:

```rust
mod profile_applicators;
mod profile_plan;
```

- [ ] **Step 4: Run applicator tests**

Run:

```bash
cargo test profile_applicators::copilot::tests::copilot_plans_mcp_merge_and_instruction_write profile_applicators::copilot::tests::copilot_command_uses_env_override_and_extra_args
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/profile_plan.rs src/profile_applicators/mod.rs src/profile_applicators/copilot.rs
git commit -m "feat: add profile planning and copilot applicator"
```

## Task 4: Profile state persistence

**Files:**
- Modify: `src/profile.rs`
- Test: `src/profile.rs`

- [ ] **Step 1: Write failing state tests**

Add tests to `src/profile.rs`:

```rust
#[test]
fn saves_and_loads_profile_state() {
    let temp = tempfile::TempDir::new().unwrap();
    let state = ProfileState {
        name: "rust-dev".to_string(),
        harness: "copilot".to_string(),
        mode: ProfileMode::Persistent,
        session_id: None,
        applied_at: Utc::now(),
        profile_fingerprint: "sha256:test".to_string(),
        overlays: vec!["rust-base".to_string()],
        files: Vec::new(),
        skipped: Vec::new(),
    };

    save_profile_state(temp.path(), &state).unwrap();
    let loaded = load_profile_state(temp.path(), "rust-dev", "copilot").unwrap();
    assert_eq!(loaded.name, "rust-dev");
    assert_eq!(loaded.harness, "copilot");
    assert_eq!(loaded.overlays, vec!["rust-base"]);
}

#[test]
fn removes_profile_state_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let state = ProfileState {
        name: "rust-dev".to_string(),
        harness: "copilot".to_string(),
        mode: ProfileMode::Persistent,
        session_id: None,
        applied_at: Utc::now(),
        profile_fingerprint: "sha256:test".to_string(),
        overlays: Vec::new(),
        files: Vec::new(),
        skipped: Vec::new(),
    };

    save_profile_state(temp.path(), &state).unwrap();
    remove_profile_state(temp.path(), "rust-dev", "copilot").unwrap();
    assert!(load_profile_state(temp.path(), "rust-dev", "copilot").is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test profile::tests::saves_and_loads_profile_state profile::tests::removes_profile_state_file
```

Expected: compile failure because state persistence functions do not exist.

- [ ] **Step 3: Add state helpers**

Add to `src/profile.rs`:

```rust
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const PROFILES_DIR: &str = "profiles";

pub(crate) fn profile_state_path(target: &Path, name: &str, harness: &str) -> PathBuf {
    target
        .join(crate::state::STATE_DIR)
        .join(PROFILES_DIR)
        .join(format!("{name}.{harness}.ccl"))
}

pub(crate) fn save_profile_state(target: &Path, state: &ProfileState) -> Result<()> {
    let path = profile_state_path(target, &state.name, &state.harness);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = sickle::to_string(state).context("Failed to serialize profile state")?;
    fs::write(&path, content)
        .with_context(|| format!("Failed to write profile state: {}", path.display()))?;
    Ok(())
}

pub(crate) fn load_profile_state(target: &Path, name: &str, harness: &str) -> Result<ProfileState> {
    let path = profile_state_path(target, name, harness);
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read profile state: {}", path.display()))?;
    sickle::from_str(&content)
        .with_context(|| format!("Failed to parse profile state: {}", path.display()))
}

pub(crate) fn remove_profile_state(target: &Path, name: &str, harness: &str) -> Result<()> {
    let path = profile_state_path(target, name, harness);
    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove profile state: {}", path.display()))?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run state tests**

Run:

```bash
cargo test profile::tests::saves_and_loads_profile_state profile::tests::removes_profile_state_file
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/profile.rs
git commit -m "feat: persist profile state"
```

## Task 5: Persistent profile apply for overlays, instructions, and MCPs

**Files:**
- Modify: `src/profile_plan.rs`
- Modify: `src/cli/commands/profile.rs`
- Modify: `src/lib.rs`
- Test: `tests/cli.rs`

- [ ] **Step 1: Write failing integration test**

Add to `tests/cli.rs`:

```rust
#[test]
fn profile_apply_writes_copilot_assets_and_state() {
    let ctx = TestContext::new();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
    mcps =
      servers =
        rust =
          command = uvx
          args =
            = mcp-rust
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("Applied profile rust-dev"));

    assert!(copilot_home.path().join("mcp.json").exists());
    assert!(
        copilot_home
            .path()
            .join("instructions/rust-dev/copilot-instructions.md")
            .exists()
    );
    assert!(
        ctx.repo_path()
            .join(".repoverlay/profiles/rust-dev.copilot.ccl")
            .exists()
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test profile_apply_writes_copilot_assets_and_state
```

Expected: FAIL with message `profile apply/status/remove are added in the profile lifecycle task`.

- [ ] **Step 3: Implement profile apply orchestration**

Add to `src/profile_plan.rs`:

```rust
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use crate::profile::{
    ProfileFileEntry, ProfileMode, ProfileScope, ProfileState, SkippedCapability,
    save_profile_state,
};
use crate::profile_applicators::copilot::CopilotApplicator;
use crate::profile_applicators::{ProfileApplicator, ProfileContext};

pub(crate) fn apply_profile(
    name: &str,
    harness: &str,
    target: &Path,
    mode: ProfileMode,
    session_id: Option<String>,
) -> Result<ProfileState> {
    let config = crate::config::load_config(Some(target))?;
    let profile = config
        .profiles
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("Profile '{name}' not found"))?;
    let applicator = copilot_applicator(harness)?;
    let context = ProfileContext {
        profile_name: name.to_string(),
        target: target.to_path_buf(),
        profile_asset_dir: target.to_path_buf(),
        harness_home: CopilotApplicator::harness_home_from_env()?,
        mode,
        session_id: session_id.clone(),
    };
    let plan = applicator.plan(profile, &context)?;
    let mut state = ProfileState {
        name: name.to_string(),
        harness: harness.to_string(),
        mode,
        session_id,
        applied_at: Utc::now(),
        profile_fingerprint: format!("sha256:{}", simple_profile_fingerprint(profile)),
        overlays: Vec::new(),
        files: Vec::new(),
        skipped: Vec::new(),
    };

    for action in plan.actions {
        match action {
            ProfileAction::ApplyOverlay { reference } => {
                crate::apply_overlay(
                    &reference,
                    target,
                    false,
                    None,
                    None,
                    true,
                    crate::ConflictStrategy::Fail,
                    false,
                    None,
                    false,
                )?;
                state.overlays.push(reference);
            }
            ProfileAction::WriteFile {
                source,
                target,
                scope,
            } => {
                copy_profile_file(&source, &target)?;
                state.files.push(ProfileFileEntry {
                    source,
                    target,
                    scope,
                    action: "write-file".to_string(),
                });
            }
            ProfileAction::MergeJson {
                target,
                value,
                scope,
            } => {
                merge_json_value(&target, &value)?;
                state.files.push(ProfileFileEntry {
                    source: PathBuf::from("<generated>"),
                    target,
                    scope,
                    action: "merge-json".to_string(),
                });
            }
            ProfileAction::SkipCapability { capability, reason } => {
                eprintln!("Warning: skipped {capability}: {reason}");
                state.skipped.push(SkippedCapability { capability, reason });
            }
        }
    }

    save_profile_state(target, &state)?;
    println!("Applied profile {name} for {harness}");
    Ok(state)
}

fn copilot_applicator(harness: &str) -> Result<CopilotApplicator> {
    if harness == "copilot" {
        Ok(CopilotApplicator)
    } else {
        bail!("Unsupported harness '{harness}'")
    }
}

fn copy_profile_file(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target)
        .with_context(|| format!("Failed to copy {} to {}", source.display(), target.display()))?;
    Ok(())
}

fn merge_json_value(target: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut merged = if target.exists() {
        let content = fs::read_to_string(target)?;
        serde_json::from_str::<Value>(&content)?
    } else {
        Value::Object(serde_json::Map::new())
    };
    merge_json_objects(&mut merged, value);
    fs::write(target, serde_json::to_string_pretty(&merged)?)?;
    Ok(())
}

fn merge_json_objects(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                merge_json_objects(base_map.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base_value, overlay_value) => *base_value = overlay_value.clone(),
    }
}

fn simple_profile_fingerprint(profile: &crate::profile::ProfileConfig) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let serialized = sickle::to_string(profile).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    hasher.finish()
}
```

Modify `src/cli/commands/profile.rs` `Apply` arm:

```rust
        ProfileCommand::Apply {
            name,
            harness,
            target,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let target = crate::canonicalize_path(&target, "Target")?;
            crate::validate_git_repo(&target)?;
            crate::profile_plan::apply_profile(
                &name,
                &harness,
                &target,
                crate::profile::ProfileMode::Persistent,
                None,
            )?;
            Ok(())
        }
```

Make modules accessible in `src/lib.rs`:

```rust
pub(crate) mod profile;
pub(crate) mod profile_plan;
```

- [ ] **Step 4: Run apply test**

Run:

```bash
cargo test profile_apply_writes_copilot_assets_and_state
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/profile_plan.rs src/cli/commands/profile.rs tests/cli.rs
git commit -m "feat: apply copilot profiles"
```

## Task 6: Profile status and remove

**Files:**
- Modify: `src/profile_plan.rs`
- Modify: `src/cli/commands/profile.rs`
- Test: `tests/cli.rs`

- [ ] **Step 1: Write failing integration test**

Add to `tests/cli.rs`:

```rust
#[test]
fn profile_status_and_remove_manage_profile_state_and_files() {
    let ctx = TestContext::new();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "apply",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    cargo_bin_cmd!("repoverlay")
        .args(["profile", "status", "--target", ctx.repo_path().to_str().unwrap()])
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("rust-dev"))
        .stdout(predicate::str::contains("copilot"));

    cargo_bin_cmd!("repoverlay")
        .args([
            "profile",
            "remove",
            "rust-dev",
            "--harness",
            "copilot",
            "--target",
            ctx.repo_path().to_str().unwrap(),
        ])
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed profile rust-dev"));

    assert!(
        !copilot_home
            .path()
            .join("instructions/rust-dev/copilot-instructions.md")
            .exists()
    );
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/profiles/rust-dev.copilot.ccl")
            .exists()
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test profile_status_and_remove_manage_profile_state_and_files
```

Expected: FAIL because status/remove handlers still return the lifecycle placeholder error.

- [ ] **Step 3: Add status and remove behavior**

Add to `src/profile_plan.rs`:

```rust
pub(crate) fn remove_profile(name: &str, harness: &str, target: &Path) -> Result<()> {
    let state = crate::profile::load_profile_state(target, name, harness)?;
    for file in &state.files {
        if file.action == "write-file" && file.target.exists() {
            fs::remove_file(&file.target)
                .with_context(|| format!("Failed to remove {}", file.target.display()))?;
        }
    }
    for overlay in &state.overlays {
        if crate::state::load_overlay_state(target, overlay).is_ok() {
            crate::remove_overlay(target, Some(overlay.clone()), false, false)?;
        }
    }
    crate::profile::remove_profile_state(target, name, harness)?;
    println!("Removed profile {name} for {harness}");
    Ok(())
}

pub(crate) fn list_profile_states(target: &Path) -> Result<Vec<crate::profile::ProfileState>> {
    let dir = target.join(crate::state::STATE_DIR).join("profiles");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut states = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(std::ffi::OsStr::to_str) != Some("ccl") {
            continue;
        }
        let content = fs::read_to_string(entry.path())?;
        states.push(sickle::from_str(&content)?);
    }
    Ok(states)
}
```

Modify `src/cli/commands/profile.rs`:

```rust
        ProfileCommand::Status { target, harness } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let target = crate::canonicalize_path(&target, "Target")?;
            let states = crate::profile_plan::list_profile_states(&target)?;
            if states.is_empty() {
                println!("No profiles applied.");
                return Ok(());
            }
            for state in states {
                if harness.as_ref().is_some_and(|h| h != &state.harness) {
                    continue;
                }
                println!("{} ({})", state.name.bold(), state.harness);
            }
            Ok(())
        }
        ProfileCommand::Remove {
            name,
            harness,
            target,
        } => {
            let target = target.unwrap_or_else(|| PathBuf::from("."));
            let target = crate::canonicalize_path(&target, "Target")?;
            crate::profile_plan::remove_profile(&name, &harness, &target)?;
            Ok(())
        }
```

- [ ] **Step 4: Run status/remove test**

Run:

```bash
cargo test profile_status_and_remove_manage_profile_state_and_files
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/profile_plan.rs src/cli/commands/profile.rs tests/cli.rs
git commit -m "feat: add profile status and remove"
```

## Task 7: Ephemeral `repoverlay copilot --profile`

**Files:**
- Create: `src/cli/commands/copilot.rs`
- Modify: `src/cli/commands/mod.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/profile_plan.rs`
- Test: `tests/cli.rs`

- [ ] **Step 1: Write failing integration test**

Add to `tests/cli.rs`:

```rust
#[test]
fn copilot_profile_runs_command_and_cleans_up() {
    let ctx = TestContext::new();
    let copilot_home = tempfile::TempDir::new().unwrap();
    let marker = ctx.repo_path().join("copilot-ran.txt");
    ctx.create_repo_file("copilot-instructions.md", "Use Rust 2024.");
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    instructions =
      =
        source = copilot-instructions.md
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "copilot",
            "--profile",
            "rust-dev",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--",
            "-c",
            &format!("echo ran > {}", marker.display()),
        ])
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_COPILOT_COMMAND", "sh")
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .success();

    assert!(marker.exists());
    assert!(
        !copilot_home
            .path()
            .join("instructions/rust-dev/copilot-instructions.md")
            .exists()
    );
    assert!(
        !ctx.repo_path()
            .join(".repoverlay/profiles/rust-dev.copilot.ccl")
            .exists()
    );
}

#[test]
fn copilot_profile_preserves_harness_exit_code_after_cleanup() {
    let ctx = TestContext::new();
    let copilot_home = tempfile::TempDir::new().unwrap();
    ctx.write_repo_config(
        r"
profiles =
  rust-dev =
    description = Rust
",
    );

    cargo_bin_cmd!("repoverlay")
        .args([
            "copilot",
            "--profile",
            "rust-dev",
            "--target",
            ctx.repo_path().to_str().unwrap(),
            "--",
            "-c",
            "exit 7",
        ])
        .env("REPOVERLAY_COPILOT_HOME", copilot_home.path())
        .env("REPOVERLAY_COPILOT_COMMAND", "sh")
        .env("REPOVERLAY_NO_UPDATE_CHECK", "1")
        .assert()
        .code(7);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test copilot_profile_runs_command_and_cleans_up copilot_profile_preserves_harness_exit_code_after_cleanup
```

Expected: FAIL with clap error containing `unrecognized subcommand 'copilot'`.

- [ ] **Step 3: Add Copilot CLI command**

Create `src/cli/commands/copilot.rs`:

```rust
use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::profile::ProfileMode;

pub(crate) fn handle_copilot_command(
    profile: String,
    target: Option<PathBuf>,
    extra_args: Vec<String>,
) -> Result<()> {
    let target = target.unwrap_or_else(|| PathBuf::from("."));
    let target = crate::canonicalize_path(&target, "Target")?;
    crate::validate_git_repo(&target)?;
    let session_id = format!(
        "{}-copilot-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        profile
    );
    let state = crate::profile_plan::apply_profile(
        &profile,
        "copilot",
        &target,
        ProfileMode::Ephemeral,
        Some(session_id),
    )?;

    let config = crate::config::load_config(Some(&target))?;
    let profile_config = config
        .profiles
        .get(&profile)
        .context("Profile disappeared after apply")?;
    let _ = profile_config;
    let context = crate::profile_applicators::ProfileContext {
        profile_name: profile.clone(),
        target: target.clone(),
        profile_asset_dir: target.clone(),
        harness_home: crate::profile_applicators::copilot::CopilotApplicator::harness_home_from_env()?,
        mode: ProfileMode::Ephemeral,
        session_id: state.session_id.clone(),
    };
    let applicator = crate::profile_applicators::copilot::CopilotApplicator;
    let mut command = crate::profile_applicators::ProfileApplicator::command(
        &applicator,
        &context,
        &extra_args,
    )?;
    command.current_dir(&target);
    let status = command.status().context("Failed to run Copilot harness")?;

    let cleanup_result = crate::profile_plan::remove_profile(&profile, "copilot", &target);
    if let Err(error) = cleanup_result {
        anyhow::bail!("Copilot exited, but profile cleanup failed: {error}");
    }

    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    Ok(())
}
```

Modify `src/cli/commands/mod.rs`:

```rust
pub(crate) mod copilot;
```

Modify `src/cli/mod.rs` imports:

```rust
pub(crate) use commands::copilot::handle_copilot_command;
```

Add command variant:

```rust
    /// Run GitHub Copilot with a profile applied for the process lifetime
    Copilot {
        /// Profile name to apply while Copilot runs
        #[arg(long)]
        profile: String,

        /// Target repository directory (defaults to current directory)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Extra arguments forwarded to the Copilot harness
        #[arg(last = true)]
        extra_args: Vec<String>,
    },
```

Add dispatch:

```rust
        Commands::Copilot {
            profile,
            target,
            extra_args,
        } => {
            handle_copilot_command(profile, target, extra_args)?;
        }
```

- [ ] **Step 4: Run ephemeral Copilot tests**

Run:

```bash
cargo test copilot_profile_runs_command_and_cleans_up copilot_profile_preserves_harness_exit_code_after_cleanup
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/cli/mod.rs src/cli/commands/mod.rs src/cli/commands/copilot.rs src/profile_plan.rs tests/cli.rs
git commit -m "feat: run copilot with ephemeral profiles"
```

## Task 8: Full profile test pass and documentation update

**Files:**
- Modify: `README.md:32-40, 68-95`
- Modify: `website/src/content/docs/cli-reference.md` only if generated docs are checked in by the existing project workflow.

- [ ] **Step 1: Add README profile summary**

Modify `README.md` concepts section to include:

```markdown
- **Profile** — a named AI harness configuration that composes overlays, MCP servers, skills, plugins, and harness/user-level instruction files. Profiles can be applied persistently with `profile apply` or ephemerally with harness commands such as `repoverlay copilot --profile rust-dev`.
```

Modify usage section to include:

```markdown
Profiles compose overlays and AI harness configuration:

```bash
repoverlay profile list
repoverlay profile show rust-dev
repoverlay profile apply rust-dev --harness copilot
repoverlay copilot --profile rust-dev -- --help
```
```

- [ ] **Step 2: Run targeted profile tests**

Run:

```bash
cargo test profile_
```

Expected: all profile-related unit and integration tests PASS.

- [ ] **Step 3: Run full checks**

Run:

```bash
just check
```

Expected: format, lint, and tests PASS.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document profiles"
```

## Self-review against spec

- Spec requirement: named profiles in main CCL config. Covered by Task 1.
- Spec requirement: repo-local/global profile merge semantics. Covered by Task 1.
- Spec requirement: profile list/show/apply/status/remove commands. Covered by Tasks 2, 5, and 6.
- Spec requirement: GitHub Copilot applicator first. Covered by Tasks 3, 5, and 7.
- Spec requirement: MCP servers and harness/user-level instructions. Covered by Tasks 3 and 5.
- Spec requirement: repo-level instruction files use overlays. Covered by documentation in Task 8 and by only supporting user-level instruction actions in the Copilot applicator.
- Spec requirement: unsupported skills/plugins warn and skip. Covered by Task 3 and applied in Task 5.
- Spec requirement: profile state is system-managed metadata. Covered by Task 4.
- Spec requirement: ephemeral `repoverlay copilot --profile` execution. Covered by Task 7.
- Spec requirement: cleanup on harness exit and exit code preservation. Covered by Task 7.

No unresolved placeholders are present. Type names introduced in early tasks are reused consistently in later tasks.
