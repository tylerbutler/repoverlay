//! JSON deep merge utilities for overlay application.
//!
//! When overlays contain JSON files that already exist in the target repo,
//! this module merges them recursively instead of overwriting.

use serde_json::Value;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Result of merging two JSON values, with statistics for logging.
#[derive(Debug, Default)]
pub(crate) struct MergeResult {
    pub(crate) merged: Value,
    pub(crate) keys_added: usize,
    pub(crate) keys_overridden: usize,
    pub(crate) type_mismatches: Vec<TypeMismatch>,
}

/// A type mismatch encountered during merge.
#[derive(Debug)]
pub(crate) struct TypeMismatch {
    pub(crate) key_path: String,
    pub(crate) base_type: String,
    pub(crate) overlay_type: String,
}

/// Errors that can occur while merging JSON files.
#[derive(Debug, Error)]
pub(crate) enum JsonMergeError {
    #[error("Failed to read JSON file: {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse JSON file: {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Failed to serialize merged JSON: {0}")]
    Serialize(#[source] serde_json::Error),

    #[error("Failed to write merged JSON: {path}: {source}")]
    Write {
        path: PathBuf,
        source: anyhow::Error,
    },
}

/// Deep merge two JSON values. Overlay wins for scalars, arrays, and type mismatches.
/// Objects are recursively merged.
pub(crate) fn deep_merge(base: &Value, overlay: &Value) -> MergeResult {
    let mut result = MergeResult::default();
    result.merged = merge_values(base, overlay, "", &mut result);
    result
}

fn merge_values(base: &Value, overlay: &Value, path: &str, stats: &mut MergeResult) -> Value {
    if let (Value::Object(base_map), Value::Object(overlay_map)) = (base, overlay) {
        let mut merged = base_map.clone();
        for (key, overlay_val) in overlay_map {
            let key_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            if let Some(base_val) = base_map.get(key) {
                let merged_val = merge_values(base_val, overlay_val, &key_path, stats);
                merged.insert(key.clone(), merged_val);
            } else {
                stats.keys_added += 1;
                merged.insert(key.clone(), overlay_val.clone());
            }
        }
        Value::Object(merged)
    } else {
        // Non-object cases: overlay wins
        if base != overlay {
            if std::mem::discriminant(base) == std::mem::discriminant(overlay) {
                stats.keys_overridden += 1;
            } else {
                let key_path = if path.is_empty() { "(root)" } else { path };
                stats.type_mismatches.push(TypeMismatch {
                    key_path: key_path.to_owned(),
                    base_type: json_type_name(base).to_string(),
                    overlay_type: json_type_name(overlay).to_string(),
                });
            }
        }
        overlay.clone()
    }
}

const fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Check if a file path has a .json extension.
pub(crate) fn is_json_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// Read a file, parse as JSON, and return the parsed value.
/// Returns an error if the file can't be read or isn't valid JSON.
fn read_json_file(path: &Path) -> Result<Value, JsonMergeError> {
    let content = std::fs::read_to_string(path).map_err(|source| JsonMergeError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&content).map_err(|source| JsonMergeError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Merge overlay JSON into base JSON and write the result to the target path.
/// Returns the `MergeResult` with statistics for logging.
pub(crate) fn merge_json_files(
    base_path: &Path,
    overlay_path: &Path,
    target_path: &Path,
) -> Result<MergeResult, JsonMergeError> {
    let base = read_json_file(base_path)?;
    let overlay = read_json_file(overlay_path)?;
    let result = deep_merge(&base, &overlay);

    let output = serde_json::to_string_pretty(&result.merged).map_err(JsonMergeError::Serialize)?;
    crate::fs_util::atomic_write(target_path, &output).map_err(|source| JsonMergeError::Write {
        path: target_path.to_path_buf(),
        source,
    })?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    #[cfg(unix)]
    use tempfile::TempDir;

    #[test]
    fn merge_disjoint_objects() {
        let base = json!({"a": 1});
        let overlay = json!({"b": 2});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": 2}));
        assert_eq!(result.keys_added, 1);
        assert_eq!(result.keys_overridden, 0);
        assert!(result.type_mismatches.is_empty());
    }

    #[test]
    fn merge_overlapping_scalars_overlay_wins() {
        let base = json!({"a": 1, "b": 2});
        let overlay = json!({"b": 99});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": 99}));
        assert_eq!(result.keys_overridden, 1);
    }

    #[test]
    fn merge_nested_objects_recursively() {
        let base = json!({"outer": {"a": 1, "b": 2}});
        let overlay = json!({"outer": {"b": 99, "c": 3}});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"outer": {"a": 1, "b": 99, "c": 3}}));
        assert_eq!(result.keys_overridden, 1); // outer.b
        assert_eq!(result.keys_added, 1); // outer.c
    }

    #[test]
    fn merge_arrays_overlay_replaces() {
        let base = json!({"list": [1, 2, 3]});
        let overlay = json!({"list": [4, 5]});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"list": [4, 5]}));
        assert_eq!(result.keys_overridden, 1);
    }

    #[test]
    fn merge_type_mismatch_overlay_wins_and_logs() {
        let base = json!({"key": "string_value"});
        let overlay = json!({"key": {"nested": true}});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"key": {"nested": true}}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].key_path, "key");
        assert_eq!(result.type_mismatches[0].base_type, "string");
        assert_eq!(result.type_mismatches[0].overlay_type, "object");
    }

    #[test]
    fn merge_deeply_nested_type_mismatch_has_full_path() {
        let base = json!({"a": {"b": {"c": 42}}});
        let overlay = json!({"a": {"b": {"c": "now a string"}}});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": {"b": {"c": "now a string"}}}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].key_path, "a.b.c");
    }

    #[test]
    fn merge_empty_base_takes_overlay() {
        let base = json!({});
        let overlay = json!({"a": 1, "b": [2]});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": [2]}));
        assert_eq!(result.keys_added, 2);
    }

    #[test]
    fn merge_empty_overlay_preserves_base() {
        let base = json!({"a": 1, "b": 2});
        let overlay = json!({});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1, "b": 2}));
        assert_eq!(result.keys_added, 0);
        assert_eq!(result.keys_overridden, 0);
    }

    #[test]
    fn is_json_file_detects_extension() {
        assert!(is_json_file(Path::new("settings.json")));
        assert!(is_json_file(Path::new("path/to/config.JSON")));
        assert!(!is_json_file(Path::new("file.txt")));
        assert!(!is_json_file(Path::new("file.jsonl")));
        assert!(!is_json_file(Path::new("file")));
    }

    #[test]
    fn merge_null_values() {
        let base = json!({"a": null});
        let overlay = json!({"a": 1});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"a": 1}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].base_type, "null");
    }

    #[cfg(unix)]
    #[test]
    fn merge_json_files_writes_atomically_without_following_target_symlink() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().join("base.json");
        let overlay = dir.path().join("overlay.json");
        let external = dir.path().join("external.json");
        let target = dir.path().join("target.json");

        fs::write(&base, r#"{"base": true}"#).unwrap();
        fs::write(&overlay, r#"{"overlay": true}"#).unwrap();
        fs::write(&external, r#"{"external": true}"#).unwrap();
        symlink(&external, &target).unwrap();

        merge_json_files(&base, &overlay, &target).unwrap();

        assert_eq!(
            fs::read_to_string(&external).unwrap(),
            r#"{"external": true}"#
        );
        assert!(!target.is_symlink());
        let merged: Value = serde_json::from_str(&fs::read_to_string(target).unwrap()).unwrap();
        assert_eq!(merged, json!({"base": true, "overlay": true}));
    }

    #[test]
    fn merge_bool_to_string_type_mismatch() {
        let base = json!({"flag": true});
        let overlay = json!({"flag": "yes"});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"flag": "yes"}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].base_type, "bool");
        assert_eq!(result.type_mismatches[0].overlay_type, "string");
    }

    #[test]
    fn merge_array_to_object_type_mismatch() {
        let base = json!({"data": [1, 2, 3]});
        let overlay = json!({"data": {"nested": true}});
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!({"data": {"nested": true}}));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].base_type, "array");
        assert_eq!(result.type_mismatches[0].overlay_type, "object");
    }

    #[test]
    fn merge_root_level_type_mismatch_uses_root_path() {
        let base = json!("a string");
        let overlay = json!(42);
        let result = deep_merge(&base, &overlay);
        assert_eq!(result.merged, json!(42));
        assert_eq!(result.type_mismatches.len(), 1);
        assert_eq!(result.type_mismatches[0].key_path, "(root)");
        assert_eq!(result.type_mismatches[0].base_type, "string");
        assert_eq!(result.type_mismatches[0].overlay_type, "number");
    }
}
