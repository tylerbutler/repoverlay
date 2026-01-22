use std::process::Command;
use vergen::{BuildBuilder, Emitter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Emit build timestamp
    let build = BuildBuilder::default().build_timestamp(true).build()?;
    Emitter::default().add_instructions(&build)?.emit()?;

    // Capture git info directly to avoid vergen-gitcl version conflicts
    emit_git_info();

    Ok(())
}

fn emit_git_info() {
    // Get git describe (includes tag, commits since tag, and dirty status)
    if let Some(describe) = git_command(&["describe", "--tags", "--always", "--dirty"]) {
        println!("cargo:rustc-env=VERGEN_GIT_DESCRIBE={describe}");
    }

    // Get git SHA
    if let Some(sha) = git_command(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=VERGEN_GIT_SHA={sha}");
    }

    // Get branch name
    if let Some(branch) = git_command(&["rev-parse", "--abbrev-ref", "HEAD"]) {
        println!("cargo:rustc-env=VERGEN_GIT_BRANCH={branch}");
    }

    // Check if dirty
    let dirty = git_command(&["status", "--porcelain"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    println!("cargo:rustc-env=VERGEN_GIT_DIRTY={dirty}");

    // Rerun if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}

fn git_command(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
