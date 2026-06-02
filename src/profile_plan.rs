#![allow(dead_code)]

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
    ApplyOverlay {
        reference: String,
    },
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
