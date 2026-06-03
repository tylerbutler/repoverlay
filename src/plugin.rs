//! Plugin reference model for profiles.
//!
//! A profile lists `plugins`, each of which is either a reference to a plugin in
//! a named marketplace (`marketplace/plugin` shorthand or an expanded table) or a
//! local plugin directory (a path starting with `.` or `/`). Plugins are the only
//! mechanism profiles use to deliver MCP servers, skills, agents, and hooks.

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::path::PathBuf;

use crate::profile::ProfileScope;

/// How repoverlay installs a marketplace plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum InstallMode {
    /// repoverlay caches the bundle and self-manages placement (default).
    #[default]
    Managed,
    /// repoverlay delegates enablement to the harness (e.g. Claude `enabledPlugins`).
    Delegate,
}

/// A reference to a plugin from a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum PluginRef {
    /// A plugin provided by a named marketplace.
    Marketplace {
        /// Name of the marketplace in the `marketplaces` registry.
        marketplace: String,
        /// Plugin name within that marketplace.
        name: String,
        /// Optional git ref (branch/tag/commit) to pin.
        r#ref: Option<String>,
        /// How the plugin is installed.
        install: InstallMode,
        /// Optional scope override for delegate enablement.
        scope: Option<ProfileScope>,
    },
    /// A plugin shipped as a local directory (path starting with `.` or `/`).
    Local {
        /// Repo-relative or absolute path to the plugin bundle.
        source: PathBuf,
    },
}

impl PluginRef {
    /// Returns `true` when a bare string should be parsed as a local path.
    fn is_local_str(s: &str) -> bool {
        s.starts_with('.') || s.starts_with('/')
    }

    /// Parse the `marketplace/plugin` shorthand into its two parts.
    ///
    /// Rejects strings without exactly one `/` separator or with an empty side.
    fn parse_shorthand<E: de::Error>(s: &str) -> Result<(String, String), E> {
        let (marketplace, name) = s.split_once('/').ok_or_else(|| {
            E::custom(format!(
                "plugin reference '{s}' must be 'marketplace/plugin' or a local path \
                 starting with '.' or '/'"
            ))
        })?;
        if marketplace.is_empty() || name.is_empty() || name.contains('/') {
            return Err(E::custom(format!(
                "invalid plugin reference '{s}': expected exactly one '/' separating a \
                 non-empty marketplace and plugin name"
            )));
        }
        Ok((marketplace.to_string(), name.to_string()))
    }
}

impl fmt::Display for PluginRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local { source } => write!(f, "{}", source.display()),
            Self::Marketplace {
                marketplace,
                name,
                r#ref,
                install,
                ..
            } => {
                write!(f, "{marketplace}/{name}")?;
                if let Some(r) = r#ref {
                    write!(f, "@{r}")?;
                }
                if *install == InstallMode::Delegate {
                    write!(f, " (delegate)")?;
                }
                Ok(())
            }
        }
    }
}

impl<'de> Deserialize<'de> for PluginRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PluginRefVisitor;

        impl<'de> Visitor<'de> for PluginRefVisitor {
            type Value = PluginRef;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a 'marketplace/plugin' string, a local path, or a plugin table")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<PluginRef, E> {
                if PluginRef::is_local_str(value) {
                    return Ok(PluginRef::Local {
                        source: PathBuf::from(value),
                    });
                }
                let (marketplace, name) = PluginRef::parse_shorthand::<E>(value)?;
                Ok(PluginRef::Marketplace {
                    marketplace,
                    name,
                    r#ref: None,
                    install: InstallMode::default(),
                    scope: None,
                })
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<PluginRef, M::Error> {
                let mut marketplace: Option<String> = None;
                let mut name: Option<String> = None;
                let mut r#ref: Option<String> = None;
                let mut install: Option<InstallMode> = None;
                let mut scope: Option<ProfileScope> = None;
                let mut source: Option<PathBuf> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "marketplace" => marketplace = Some(map.next_value()?),
                        "name" => name = Some(map.next_value()?),
                        "ref" => r#ref = Some(map.next_value()?),
                        "install" => install = Some(map.next_value()?),
                        "scope" => scope = Some(map.next_value()?),
                        "source" => source = Some(map.next_value()?),
                        other => {
                            return Err(de::Error::unknown_field(
                                other,
                                &["marketplace", "name", "ref", "install", "scope", "source"],
                            ));
                        }
                    }
                }

                if let Some(source) = source {
                    if marketplace.is_some() || name.is_some() {
                        return Err(de::Error::custom(
                            "plugin entry cannot set both 'source' and 'marketplace'/'name'",
                        ));
                    }
                    return Ok(PluginRef::Local { source });
                }

                Ok(PluginRef::Marketplace {
                    marketplace: marketplace
                        .ok_or_else(|| de::Error::missing_field("marketplace"))?,
                    name: name.ok_or_else(|| de::Error::missing_field("name"))?,
                    r#ref,
                    install: install.unwrap_or_default(),
                    scope,
                })
            }
        }

        deserializer.deserialize_any(PluginRefVisitor)
    }
}

impl Serialize for PluginRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Local { source } => serializer.serialize_str(&source.to_string_lossy()),
            Self::Marketplace {
                marketplace,
                name,
                r#ref,
                install,
                scope,
            } => {
                // Use the compact `marketplace/plugin` shorthand when no other
                // fields are customized; otherwise emit a full table.
                if r#ref.is_none() && *install == InstallMode::Managed && scope.is_none() {
                    return serializer.serialize_str(&format!("{marketplace}/{name}"));
                }
                let mut len = 2;
                if r#ref.is_some() {
                    len += 1;
                }
                if *install != InstallMode::Managed {
                    len += 1;
                }
                if scope.is_some() {
                    len += 1;
                }
                let mut map = serializer.serialize_map(Some(len))?;
                map.serialize_entry("marketplace", marketplace)?;
                map.serialize_entry("name", name)?;
                if let Some(r) = r#ref {
                    map.serialize_entry("ref", r)?;
                }
                if *install != InstallMode::Managed {
                    map.serialize_entry("install", install)?;
                }
                if let Some(scope) = scope {
                    map.serialize_entry("scope", scope)?;
                }
                map.end()
            }
        }
    }
}

/// Origin of a resolved plugin bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum PluginOrigin {
    /// Bundle came from a cached git clone (commit-pinned).
    CachedGit,
    /// Bundle is a local directory on disk (a local plugin or a local marketplace checkout).
    LocalPath,
}

/// Why a plugin must be delegated to the harness rather than managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum DelegateReason {
    /// The plugin source is not a cloneable/introspectable git repo (e.g. `npm:`).
    NonGitSource,
}

/// A fully resolved plugin reference.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ResolvedPlugin {
    /// repoverlay located (and, for git sources, cached) an introspectable bundle.
    Bundle {
        /// Plugin name.
        name: String,
        /// Directory containing the plugin bundle.
        bundle_dir: PathBuf,
        /// Resolved commit SHA, when the bundle is backed by a git repo.
        resolved_commit: Option<String>,
        /// Where the bundle came from.
        origin: PluginOrigin,
    },
    /// repoverlay cannot cache/introspect this source; enablement is delegated.
    Delegate {
        /// Plugin name.
        name: String,
        /// The raw source string from the marketplace entry.
        source: String,
        /// Why delegation is required.
        reason: DelegateReason,
    },
}

/// Introspected contents of a plugin bundle directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct PluginBundle {
    /// Parsed `.claude-plugin/plugin.json`, if present.
    pub(crate) manifest: Option<serde_json::Value>,
    /// MCP servers from `.mcp.json` (`mcpServers` object), keyed by server name.
    pub(crate) mcp_servers: serde_json::Map<String, serde_json::Value>,
    /// Skill directory names found under `skills/`.
    pub(crate) skills: Vec<String>,
    /// Bundle capabilities repoverlay cannot decompose into native harness
    /// locations (e.g. `hooks`, `agents`, `commands`). Surfaced so applicators
    /// can emit `SkipCapability` rather than silently dropping them.
    pub(crate) unsupported_capabilities: Vec<String>,
}

impl PluginBundle {
    /// Read and introspect a plugin bundle directory.
    ///
    /// Missing `.claude-plugin/plugin.json`, `.mcp.json`, or `skills/` are not
    /// errors — they yield empty/absent fields. Malformed JSON is an error.
    #[allow(dead_code)]
    pub(crate) fn read(dir: &std::path::Path) -> anyhow::Result<Self> {
        use anyhow::Context;

        let manifest_path = dir.join(".claude-plugin").join("plugin.json");
        let manifest = if manifest_path.is_file() {
            let raw = std::fs::read_to_string(&manifest_path)
                .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
            Some(
                serde_json::from_str::<serde_json::Value>(&raw)
                    .with_context(|| format!("Invalid JSON in {}", manifest_path.display()))?,
            )
        } else {
            None
        };

        let mcp_path = dir.join(".mcp.json");
        let mcp_servers = if mcp_path.is_file() {
            let raw = std::fs::read_to_string(&mcp_path)
                .with_context(|| format!("Failed to read {}", mcp_path.display()))?;
            let value: serde_json::Value = serde_json::from_str(&raw)
                .with_context(|| format!("Invalid JSON in {}", mcp_path.display()))?;
            value
                .get("mcpServers")
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default()
        } else {
            serde_json::Map::new()
        };

        let mut skills = Vec::new();
        let skills_dir = dir.join("skills");
        if skills_dir.is_dir() {
            for entry in std::fs::read_dir(&skills_dir)
                .with_context(|| format!("Failed to read {}", skills_dir.display()))?
            {
                let entry = entry?;
                if entry.path().join("SKILL.md").is_file()
                    && let Some(name) = entry.file_name().to_str()
                {
                    skills.push(name.to_string());
                }
            }
            skills.sort();
        }

        // Capabilities repoverlay does not decompose: surface them so the
        // applicator can record a `SkipCapability` instead of dropping silently.
        let mut unsupported_capabilities = Vec::new();
        for capability in ["hooks", "agents", "commands"] {
            if dir.join(capability).is_dir() {
                unsupported_capabilities.push(capability.to_string());
            }
        }

        Ok(Self {
            manifest,
            mcp_servers,
            skills,
            unsupported_capabilities,
        })
    }
}

/// The literal placeholder Claude plugins use to reference their own bundle root.
const CLAUDE_PLUGIN_ROOT: &str = "${CLAUDE_PLUGIN_ROOT}";

/// Recursively substitute `${CLAUDE_PLUGIN_ROOT}` with `bundle_dir` in every
/// string value of `value`.
///
/// Object keys are left untouched. Fails if `bundle_dir` is not valid UTF-8,
/// since the substituted path is embedded into JSON the harness must read back.
pub(crate) fn substitute_plugin_root(
    value: &serde_json::Value,
    bundle_dir: &std::path::Path,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::Context;
    let root = bundle_dir.to_str().with_context(|| {
        format!(
            "Plugin bundle path is not valid UTF-8 and cannot be embedded in JSON: {}",
            bundle_dir.display()
        )
    })?;
    Ok(substitute_plugin_root_str(value, root))
}

fn substitute_plugin_root_str(value: &serde_json::Value, root: &str) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            serde_json::Value::String(s.replace(CLAUDE_PLUGIN_ROOT, root))
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| substitute_plugin_root_str(item, root))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), substitute_plugin_root_str(v, root)))
                .collect(),
        ),
        other => other.clone(),
    }
}

enum SourceKind {
    /// A relative subdirectory within the marketplace repo.
    Subdir(String),
    /// An external git URL (only github.com is cacheable).
    GitUrl(String),
    /// A non-git scheme (e.g. `npm:`) that cannot be cached.
    NonGit,
}

/// Classify a marketplace entry `source` string.
fn classify_source(source: &str) -> SourceKind {
    if source.contains("://") || source.starts_with("git@") {
        return SourceKind::GitUrl(source.to_string());
    }
    // A leading `scheme:` (e.g. `npm:pkg`) that is not a path means non-git.
    if let Some((scheme, _)) = source.split_once(':')
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        && !source.starts_with("./")
        && !source.starts_with('/')
    {
        return SourceKind::NonGit;
    }
    SourceKind::Subdir(source.to_string())
}

/// Validate that a relative subdir stays within `base`, returning the joined path.
///
/// Rejects absolute paths, `..`, and Windows root/prefix components, then verifies
/// the canonicalized result is still contained in the canonicalized `base`.
fn confined_subdir(base: &std::path::Path, sub: &str) -> anyhow::Result<PathBuf> {
    use std::path::Component;

    let sub_path = std::path::Path::new(sub);
    for component in sub_path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "plugin source '{sub}' must be a relative path within the marketplace"
                );
            }
        }
    }

    let joined = base.join(sub_path);
    let canonical_base = base.canonicalize().with_context_dir(base)?;
    let canonical_joined = joined.canonicalize().with_context_dir(&joined)?;
    if !canonical_joined.starts_with(&canonical_base) {
        anyhow::bail!("plugin source '{sub}' escapes the marketplace directory");
    }
    Ok(canonical_joined)
}

/// Small helper trait to attach a path context to canonicalize errors.
trait CanonicalizeContext {
    fn with_context_dir(self, path: &std::path::Path) -> anyhow::Result<PathBuf>;
}

impl CanonicalizeContext for std::io::Result<PathBuf> {
    fn with_context_dir(self, path: &std::path::Path) -> anyhow::Result<PathBuf> {
        use anyhow::Context;
        self.with_context(|| format!("Path does not exist: {}", path.display()))
    }
}

/// Best-effort current commit of a local git checkout.
fn local_git_commit(dir: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

/// Resolve a plugin reference into a concrete bundle (or a delegate marker).
///
/// `base_dir` is the directory that local relative plugin paths are resolved
/// against (the target repository root).
#[allow(dead_code)]
pub(crate) fn resolve_plugin(
    reference: &PluginRef,
    marketplaces: &[crate::config::Marketplace],
    cache: &crate::cache::CacheManager,
    base_dir: &std::path::Path,
    update: bool,
) -> anyhow::Result<ResolvedPlugin> {
    use anyhow::Context;

    match reference {
        PluginRef::Local { source } => {
            let bundle_dir = resolve_local_plugin_dir(base_dir, source)?;
            let resolved_commit = local_git_commit(&bundle_dir);
            Ok(ResolvedPlugin::Bundle {
                name: plugin_name_from_path(source),
                bundle_dir,
                resolved_commit,
                origin: PluginOrigin::LocalPath,
            })
        }
        PluginRef::Marketplace {
            marketplace,
            name,
            r#ref,
            ..
        } => {
            let entry = marketplaces
                .iter()
                .find(|m| &m.name == marketplace)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Marketplace '{marketplace}' is not registered. \
                         Add it with: repoverlay marketplace add {marketplace} <url>"
                    )
                })?;
            let url = entry.url.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Marketplace '{marketplace}' has no url configured")
            })?;

            let (repo_dir, market_commit, origin) =
                fetch_marketplace_repo(url, r#ref.as_deref(), cache, update)
                    .with_context(|| format!("resolving marketplace '{marketplace}'"))?;

            let manifest_path = repo_dir.join(".claude-plugin").join("marketplace.json");
            let raw = std::fs::read_to_string(&manifest_path).with_context(|| {
                format!(
                    "Marketplace '{marketplace}' is missing {}",
                    manifest_path.display()
                )
            })?;
            let manifest: MarketplaceManifest = serde_json::from_str(&raw)
                .with_context(|| format!("Invalid JSON in {}", manifest_path.display()))?;

            let plugin_entry = manifest
                .plugins
                .iter()
                .find(|p| &p.name == name)
                .ok_or_else(|| {
                    anyhow::anyhow!("Plugin '{name}' not found in marketplace '{marketplace}'")
                })?;

            let source_str = plugin_entry.source.as_str().ok_or_else(|| {
                anyhow::anyhow!(
                    "Plugin '{name}' in marketplace '{marketplace}' has a non-string source"
                )
            })?;

            match classify_source(source_str) {
                SourceKind::Subdir(sub) => {
                    let bundle_dir = confined_subdir(&repo_dir, &sub)?;
                    Ok(ResolvedPlugin::Bundle {
                        name: name.clone(),
                        bundle_dir,
                        resolved_commit: market_commit,
                        origin,
                    })
                }
                SourceKind::GitUrl(git_url) => {
                    if crate::github::GitHubSource::is_github_url(&git_url) {
                        let source = crate::github::GitHubSource::parse(&git_url)?;
                        let cached = cache.ensure_cached(&source, update)?;
                        Ok(ResolvedPlugin::Bundle {
                            name: name.clone(),
                            bundle_dir: cached.path,
                            resolved_commit: Some(cached.commit),
                            origin: PluginOrigin::CachedGit,
                        })
                    } else {
                        Ok(ResolvedPlugin::Delegate {
                            name: name.clone(),
                            source: git_url,
                            reason: DelegateReason::NonGitSource,
                        })
                    }
                }
                SourceKind::NonGit => Ok(ResolvedPlugin::Delegate {
                    name: name.clone(),
                    source: source_str.to_string(),
                    reason: DelegateReason::NonGitSource,
                }),
            }
        }
    }
}

/// Resolve a local plugin `source` path against `base_dir`.
fn resolve_local_plugin_dir(
    base_dir: &std::path::Path,
    source: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    use anyhow::Context;

    let candidate = if source.is_absolute() {
        source.to_path_buf()
    } else {
        // Relative local plugins must stay within the repo (consistent with
        // instruction-source validation).
        return confined_subdir(base_dir, &source.to_string_lossy());
    };

    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("Plugin path does not exist: {}", candidate.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("Plugin path is not a directory: {}", canonical.display());
    }
    Ok(canonical)
}

/// Derive a plugin name from a local path (the final component).
fn plugin_name_from_path(source: &std::path::Path) -> String {
    source.file_name().map_or_else(
        || source.to_string_lossy().to_string(),
        |n| n.to_string_lossy().to_string(),
    )
}

/// Fetch the marketplace repo into a local directory, returning
/// `(repo_dir, commit, origin)`.
fn fetch_marketplace_repo(
    url: &str,
    ref_override: Option<&str>,
    cache: &crate::cache::CacheManager,
    update: bool,
) -> anyhow::Result<(PathBuf, Option<String>, PluginOrigin)> {
    if crate::github::GitHubSource::is_github_url(url) {
        let source = crate::github::GitHubSource::parse(url)?.with_ref_override(ref_override)?;
        let cached = cache.ensure_cached(&source, update)?;
        return Ok((cached.path, Some(cached.commit), PluginOrigin::CachedGit));
    }

    // Local-path or file:// marketplace (used for fixtures and local development).
    let local = url.strip_prefix("file://").unwrap_or(url);
    let dir = std::path::Path::new(local);
    if !dir.is_dir() {
        anyhow::bail!("Marketplace path is not a directory: {}", dir.display());
    }
    let commit = local_git_commit(dir);
    Ok((dir.to_path_buf(), commit, PluginOrigin::LocalPath))
}

/// Minimal `.claude-plugin/marketplace.json` schema.
#[derive(Debug, serde::Deserialize)]
struct MarketplaceManifest {
    #[serde(default)]
    plugins: Vec<MarketplaceEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct MarketplaceEntry {
    name: String,
    source: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Wrap {
        #[serde(default)]
        plugins: Vec<PluginRef>,
    }

    fn parse(ccl: &str) -> Vec<PluginRef> {
        sickle::from_str::<Wrap>(ccl).unwrap().plugins
    }

    #[test]
    fn parses_marketplace_shorthand() {
        let plugins = parse("plugins =\n  = playground/rust-dev\n");
        assert_eq!(
            plugins,
            vec![PluginRef::Marketplace {
                marketplace: "playground".to_string(),
                name: "rust-dev".to_string(),
                r#ref: None,
                install: InstallMode::Managed,
                scope: None,
            }]
        );
    }

    #[test]
    fn parses_local_path() {
        let plugins = parse("plugins =\n  = ./plugins/local-mcp\n");
        assert_eq!(
            plugins,
            vec![PluginRef::Local {
                source: PathBuf::from("./plugins/local-mcp"),
            }]
        );
    }

    #[test]
    fn parses_expanded_table() {
        let ccl = "plugins =\n  =\n    marketplace = vendor\n    name = cool\n    ref = v1.2.0\n    install = delegate\n    scope = user\n";
        let plugins = parse(ccl);
        assert_eq!(
            plugins,
            vec![PluginRef::Marketplace {
                marketplace: "vendor".to_string(),
                name: "cool".to_string(),
                r#ref: Some("v1.2.0".to_string()),
                install: InstallMode::Delegate,
                scope: Some(ProfileScope::User),
            }]
        );
    }

    #[test]
    fn rejects_shorthand_with_extra_segment() {
        let err = sickle::from_str::<Wrap>("plugins =\n  = a/b/c\n").unwrap_err();
        assert!(format!("{err}").contains("invalid plugin reference"));
    }

    #[test]
    fn rejects_shorthand_with_empty_side() {
        let err = sickle::from_str::<Wrap>("plugins =\n  = playground/\n").unwrap_err();
        assert!(format!("{err}").contains("invalid plugin reference"));
    }

    #[test]
    fn shorthand_round_trips_through_serialize() {
        let original = parse("plugins =\n  = playground/rust-dev\n");
        let ccl = sickle::to_string(&Wrap2 {
            plugins: original.clone(),
        })
        .unwrap();
        let reparsed = sickle::from_str::<Wrap>(&ccl).unwrap().plugins;
        assert_eq!(original, reparsed);
    }

    #[test]
    fn expanded_table_round_trips_through_serialize() {
        let original = parse(
            "plugins =\n  =\n    marketplace = vendor\n    name = cool\n    install = delegate\n",
        );
        let ccl = sickle::to_string(&Wrap2 {
            plugins: original.clone(),
        })
        .unwrap();
        let reparsed = sickle::from_str::<Wrap>(&ccl).unwrap().plugins;
        assert_eq!(original, reparsed);
    }

    #[derive(Debug, serde::Serialize)]
    struct Wrap2 {
        plugins: Vec<PluginRef>,
    }

    // --- resolution + introspection ---

    use crate::cache::CacheManager;
    use crate::config::Marketplace;
    use std::fs;
    use tempfile::TempDir;

    /// Build a local marketplace fixture containing a single plugin entry whose
    /// `source` is `source_value`. If `with_bundle` is true, also create the
    /// `plugins/rust-dev` bundle dir with a `.mcp.json` and one skill.
    fn fixture_marketplace(source_value: &str, with_bundle: bool) -> TempDir {
        let dir = TempDir::new().unwrap();
        let market = dir.path().join(".claude-plugin");
        fs::create_dir_all(&market).unwrap();
        let manifest = serde_json::json!({
            "name": "playground",
            "plugins": [ { "name": "rust-dev", "source": source_value } ]
        });
        fs::write(
            market.join("marketplace.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        if with_bundle {
            let bundle = dir.path().join("plugins").join("rust-dev");
            fs::create_dir_all(&bundle).unwrap();
            fs::write(
                bundle.join(".mcp.json"),
                r#"{"mcpServers":{"rust":{"command":"uvx"}}}"#,
            )
            .unwrap();
            let skill = bundle.join("skills").join("formatting");
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), "# formatting").unwrap();
        }
        dir
    }

    fn registry(name: &str, url: &std::path::Path) -> Vec<Marketplace> {
        vec![Marketplace {
            name: name.to_string(),
            url: Some(url.to_string_lossy().to_string()),
        }]
    }

    fn market_ref() -> PluginRef {
        PluginRef::Marketplace {
            marketplace: "playground".to_string(),
            name: "rust-dev".to_string(),
            r#ref: None,
            install: InstallMode::Managed,
            scope: None,
        }
    }

    #[test]
    fn introspects_bundle_with_mcp_and_skills() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"rust":{"command":"uvx"},"py":{"command":"python"}}}"#,
        )
        .unwrap();
        let skill = dir.path().join("skills").join("fmt");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# fmt").unwrap();

        let bundle = PluginBundle::read(dir.path()).unwrap();
        assert_eq!(bundle.mcp_servers.len(), 2);
        assert!(bundle.mcp_servers.contains_key("rust"));
        assert_eq!(bundle.skills, vec!["fmt".to_string()]);
    }

    #[test]
    fn introspects_empty_bundle() {
        let dir = TempDir::new().unwrap();
        let bundle = PluginBundle::read(dir.path()).unwrap();
        assert!(bundle.mcp_servers.is_empty());
        assert!(bundle.skills.is_empty());
        assert!(bundle.manifest.is_none());
    }

    #[test]
    fn resolves_marketplace_plugin_subdir() {
        let fixture = fixture_marketplace("./plugins/rust-dev", true);
        let cache = CacheManager::new().unwrap();
        let base = TempDir::new().unwrap();
        let resolved = resolve_plugin(
            &market_ref(),
            &registry("playground", fixture.path()),
            &cache,
            base.path(),
            false,
        )
        .unwrap();
        match resolved {
            ResolvedPlugin::Bundle {
                name,
                bundle_dir,
                origin,
                ..
            } => {
                assert_eq!(name, "rust-dev");
                assert_eq!(origin, PluginOrigin::LocalPath);
                let bundle = PluginBundle::read(&bundle_dir).unwrap();
                assert!(bundle.mcp_servers.contains_key("rust"));
                assert_eq!(bundle.skills, vec!["formatting".to_string()]);
            }
            other @ ResolvedPlugin::Delegate { .. } => panic!("expected Bundle, got {other:?}"),
        }
    }

    #[test]
    fn unregistered_marketplace_errors() {
        let cache = CacheManager::new().unwrap();
        let base = TempDir::new().unwrap();
        let err = resolve_plugin(&market_ref(), &[], &cache, base.path(), false).unwrap_err();
        assert!(format!("{err}").contains("not registered"));
    }

    #[test]
    fn non_git_source_requires_delegate() {
        let fixture = fixture_marketplace("npm:some-package", false);
        let cache = CacheManager::new().unwrap();
        let base = TempDir::new().unwrap();
        let resolved = resolve_plugin(
            &market_ref(),
            &registry("playground", fixture.path()),
            &cache,
            base.path(),
            false,
        )
        .unwrap();
        match resolved {
            ResolvedPlugin::Delegate { reason, source, .. } => {
                assert_eq!(reason, DelegateReason::NonGitSource);
                assert_eq!(source, "npm:some-package");
            }
            other @ ResolvedPlugin::Bundle { .. } => panic!("expected Delegate, got {other:?}"),
        }
    }

    #[test]
    fn rejects_traversal_in_marketplace_source() {
        let fixture = fixture_marketplace("../../etc", false);
        let cache = CacheManager::new().unwrap();
        let base = TempDir::new().unwrap();
        let err = resolve_plugin(
            &market_ref(),
            &registry("playground", fixture.path()),
            &cache,
            base.path(),
            false,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("must be a relative path"));
    }
}
