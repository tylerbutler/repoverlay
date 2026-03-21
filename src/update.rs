//! Version string building and update checking.

use colored::Colorize;
use std::sync::LazyLock;

/// Build version string with git info for local builds
pub(crate) static VERSION: LazyLock<String> = LazyLock::new(|| {
    let version = env!("CARGO_PKG_VERSION");
    let is_ci = option_env!("REPOVERLAY_CI_BUILD") == Some("true");

    // CI builds just show the version
    if is_ci {
        return version.to_string();
    }

    // Local builds show: {version}-{branch} ({sha}) or {version}-{branch} ({sha}) (dirty)
    let sha = option_env!("VERGEN_GIT_SHA").map(|s| &s[..7.min(s.len())]);
    let branch = option_env!("VERGEN_GIT_BRANCH");
    let dirty = option_env!("VERGEN_GIT_DIRTY") == Some("true");

    match (sha, branch, dirty) {
        (Some(sha), Some(branch), true) => format!("{version}-{branch} ({sha}) (dirty)"),
        (Some(sha), Some(branch), false) => format!("{version}-{branch} ({sha})"),
        (Some(sha), None, true) => format!("{version} ({sha}) (dirty)"),
        (Some(sha), None, false) => format!("{version} ({sha})"),
        (None, _, _) => version.to_string(),
    }
});

pub(crate) fn version_string() -> &'static str {
    &VERSION
}

/// Check for updates and print a notification if a new version is available.
///
/// Uses tiny-update-check to query crates.io with caching (24 hours).
/// Fetches an update message from the website when an update is available.
pub(crate) fn check_for_updates() {
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");

    let checker = tiny_update_check::UpdateChecker::new(name, version)
        .message_url("https://repoverlay.tylerbutler.com/update-message.txt");

    if let Ok(Some(update)) = checker.check_detailed() {
        eprintln!();
        eprintln!(
            "{} A new version of {} is available: {} → {}",
            "Update available:".yellow().bold(),
            name,
            update.current,
            update.latest.green().bold()
        );
        if let Some(msg) = &update.message {
            eprintln!();
            eprintln!("{msg}");
        } else {
            eprintln!(
                "                  {}",
                "https://github.com/tylerbutler/repoverlay/releases".cyan()
            );
        }
    }
}
