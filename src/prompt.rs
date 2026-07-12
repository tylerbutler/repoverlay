//! Minimal interactive stdin prompts.

use anyhow::Result;
use std::io::{self, Write};

/// Ask a yes/no question, reading the answer from stdin.
///
/// Empty input selects `default`; only `y`/`yes` (case-insensitive) answers yes.
pub(crate) fn confirm(prompt: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    print!("{prompt} [{hint}] ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(match line.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "y" | "yes" => true,
        _ => false,
    })
}

/// Ask for a line of text, reading the answer from stdin.
///
/// Empty input returns `default`.
pub(crate) fn input(prompt: &str, default: &str) -> Result<String> {
    print!("{prompt} [{default}]: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    })
}
