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
}
