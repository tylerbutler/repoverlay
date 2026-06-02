//! Profile applicators provide profile-specific integration behavior.
#![allow(dead_code)]

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
