//! Profile applicators provide profile-specific integration behavior.
#![allow(dead_code)]

pub(crate) mod copilot;

use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

use crate::profile::{ProfileConfig, ProfileMode};
use crate::profile_plan::ProfilePlan;

// DESIGN NOTE (intentional, known): harness identity is currently threaded
// around as `&str` "copilot" literals (see `copilot_applicator` and the
// removable-root matches in `profile_plan.rs`) rather than dispatched through
// this enum and the `harness()` trait method, and `ProfilePlan` carries
// `profile_name`/`harness` copies that duplicate `ProfileContext`. This is
// scaffolding for multi-harness support that only has one harness today.
// Collapsing it into first-class typed dispatch (enum + trait methods owning
// home-dir / removable-roots / JSON-merge key semantics) is a deliberate
// follow-up, not an oversight — please don't re-flag the `"copilot"` literals or
// the unused enum in isolation.
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
