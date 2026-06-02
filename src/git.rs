//! Thin wrapper over the `git config` CLI (avoids linking libgit2 and keeps
//! identical escaping/include semantics to the command line).

use anyhow::{bail, Context, Result};
use std::process::Command;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Local,
}

impl Scope {
    fn flag(self) -> &'static str {
        match self {
            Scope::Global => "--global",
            Scope::Local => "--local",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::Global => "global (~/.gitconfig)",
            Scope::Local => "local (.git/config)",
        }
    }
}

pub fn get(scope: Scope, key: &str) -> Result<Option<String>> {
    run_get(&["config", scope.flag(), "--get", key])
}

/// Effective value in the current directory (resolves `include`/`includeIf`).
pub fn get_effective(key: &str) -> Result<Option<String>> {
    run_get(&["config", "--get", key])
}

fn run_get(args: &[&str]) -> Result<Option<String>> {
    let out = Command::new("git")
        .args(args)
        .output()
        .context("failed to run git; is it installed and on PATH?")?;
    if out.status.success() {
        Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
    } else {
        match out.status.code() {
            Some(1) => Ok(None), // not set
            _ => {
                let err = String::from_utf8_lossy(&out.stderr);
                bail!("git {} failed: {}", args.join(" "), err.trim());
            }
        }
    }
}

pub fn set(scope: Scope, key: &str, value: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["config", scope.flag(), key, value])
        .status()
        .context("failed to run git config")?;
    if !status.success() {
        bail!("git config --{} {} failed", scope.flag(), key);
    }
    Ok(())
}

/// Set a key in an explicit file (for generating include files); git handles
/// section creation and value escaping.
pub fn set_file(path: &std::path::Path, key: &str, value: &str) -> Result<()> {
    let status = Command::new("git")
        .args(["config", "--file"])
        .arg(path)
        .args([key, value])
        .status()
        .context("failed to run git config --file")?;
    if !status.success() {
        bail!("git config --file {} {} failed", path.display(), key);
    }
    Ok(())
}

/// Idempotent: succeeds even if the key was already absent.
pub fn unset(scope: Scope, key: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["config", scope.flag(), "--unset", key])
        .output()
        .context("failed to run git config")?;
    match out.status.code() {
        Some(0) | Some(5) => Ok(()), // 5 = key not present
        _ => bail!(
            "git config --unset {} failed: {}",
            key,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// List a scope as (key, value) pairs. NUL-delimited so values with newlines
/// or `=` parse safely.
pub fn list(scope: Scope) -> Result<Vec<(String, String)>> {
    let out = Command::new("git")
        .args(["config", scope.flag(), "--list", "-z"])
        .output()
        .context("failed to run git config --list")?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut entries = Vec::new();
    for record in text.split('\0').filter(|r| !r.is_empty()) {
        // Each record is "key\nvalue".
        match record.split_once('\n') {
            Some((k, v)) => entries.push((k.to_string(), v.to_string())),
            None => entries.push((record.to_string(), String::new())),
        }
    }
    Ok(entries)
}

/// Idempotent: succeeds even if the section is absent.
pub fn remove_section(scope: Scope, name: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["config", scope.flag(), "--remove-section", name])
        .output()
        .context("failed to run git config --remove-section")?;
    match out.status.code() {
        Some(0) | Some(128) => Ok(()), // 128 = no such section
        _ => bail!(
            "git config --remove-section {} failed: {}",
            name,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

pub fn in_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Flat key for an `includeIf` path entry. git takes the section before the
/// first dot and the variable after the last, so the condition (with its own
/// dots and colons) becomes the subsection verbatim.
pub fn includeif_path_key(condition: &str) -> String {
    format!("includeIf.{condition}.path")
}

pub fn hasconfig_condition(remote_glob: &str) -> String {
    format!("hasconfig:remote.*.url:{remote_glob}")
}
