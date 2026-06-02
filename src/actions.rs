//! Profile operations shared by the CLI and TUI: they perform side effects
//! (writing git config, generating include files) with no prompting/printing.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::git::{self, Scope};
use crate::profile::{Profile, Registry, Ssh};

pub const DEFAULT_SSH_HOST: &str = "github.com";

pub fn ssh_hostname(ssh: &Ssh) -> &str {
    ssh.hostname.as_deref().unwrap_or(DEFAULT_SSH_HOST)
}

pub fn effective_alias(p: &Profile) -> Option<String> {
    p.ssh.as_ref().and_then(|s| s.host_alias.clone())
}

/// Identity only: name/email + signing, no transport. What a global default
/// should set; transport belongs per-repo via auto-switch.
pub fn identity_settings(p: &Profile) -> Vec<(String, String)> {
    let mut out = vec![
        ("user.name".to_string(), p.user_name.clone()),
        ("user.email".to_string(), p.user_email.clone()),
    ];
    if let Some(s) = &p.signing {
        out.push(("gpg.format".to_string(), s.format.clone()));
        out.push(("user.signingkey".to_string(), s.key.clone()));
        if s.auto_sign {
            out.push(("commit.gpgsign".to_string(), "true".to_string()));
            out.push(("tag.gpgsign".to_string(), "true".to_string()));
        }
    }
    out
}

/// Full settings: identity + HTTPS credentials + SSH transport.
pub fn profile_settings(p: &Profile) -> Vec<(String, String)> {
    let mut out = identity_settings(p);
    if let (Some(host), Some(user)) = (&p.host, &p.username) {
        out.push((format!("credential.{host}.username"), user.clone()));
        // So multiple accounts on the same host don't collide.
        out.push(("credential.useHttpPath".to_string(), "true".to_string()));
    }
    if let Some(ssh) = &p.ssh {
        let host = ssh_hostname(ssh);
        match &ssh.host_alias {
            // Rewrite SSH remotes to the alias (distinct keys, so neither URL
            // form overwrites the other).
            Some(alias) => {
                out.push((format!("url.git@{alias}:.insteadOf"), format!("git@{host}:")));
                out.push((
                    format!("url.ssh://git@{alias}/.insteadOf"),
                    format!("ssh://git@{host}/"),
                ));
            }
            None => {
                out.push((
                    "core.sshCommand".to_string(),
                    format!("ssh -i {} -o IdentitiesOnly=yes", ssh.key),
                ));
            }
        }
    }
    out
}

pub fn apply_profile(p: &Profile, scope: Scope) -> Result<()> {
    for (key, value) in profile_settings(p) {
        git::set(scope, &key, &value)?;
    }
    Ok(())
}

/// Remove gum-originated SSH transport from GLOBAL scope. A per-account rewrite
/// in global conflicts with other accounts' per-repo rewrites (same
/// `git@github.com:` prefix), so the global default must carry none.
pub fn clear_global_transport(profiles: &[Profile]) -> Result<()> {
    for p in profiles {
        let Some(ssh) = &p.ssh else { continue };
        match &ssh.host_alias {
            Some(alias) => {
                git::unset(Scope::Global, &format!("url.git@{alias}:.insteadOf"))?;
                git::unset(Scope::Global, &format!("url.ssh://git@{alias}/.insteadOf"))?;
            }
            None => {
                let cmd = format!("ssh -i {} -o IdentitiesOnly=yes", ssh.key);
                if git::get(Scope::Global, "core.sshCommand")?.as_deref() == Some(cmd.as_str()) {
                    git::unset(Scope::Global, "core.sshCommand")?;
                }
            }
        }
    }
    Ok(())
}

/// Set a profile as the global default: identity + signing only, with any
/// gum-originated global transport cleared so it can't conflict per-repo.
pub fn set_default(p: &Profile, all: &[Profile]) -> Result<()> {
    clear_global_transport(all)?;
    for (key, value) in identity_settings(p) {
        git::set(Scope::Global, &key, &value)?;
    }
    Ok(())
}

/// Generate (or regenerate) the per-profile include file.
pub fn write_include_file(p: &Profile) -> Result<PathBuf> {
    let dir = Registry::includes_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = Registry::include_path(&p.name)?;
    // Rewrite from scratch so a regenerate leaves no stale keys.
    if path.exists() {
        std::fs::remove_file(&path).ok();
    }
    for (key, value) in profile_settings(p) {
        git::set_file(&path, &key, &value)?;
    }
    Ok(path)
}

pub fn enable_auto(p: &Profile) -> Result<PathBuf> {
    let glob = p
        .remote_match
        .as_deref()
        .context("profile has no remote glob; set one before enabling auto-switch")?;
    let include = write_include_file(p)?;
    let key = git::includeif_path_key(&git::hasconfig_condition(glob));
    git::set(Scope::Global, &key, &include.to_string_lossy())?;
    Ok(include)
}

pub fn disable_auto(p: &Profile) -> Result<()> {
    let glob = p.remote_match.as_deref().context("profile has no remote glob")?;
    let key = git::includeif_path_key(&git::hasconfig_condition(glob));
    git::unset(Scope::Global, &key)
}

pub fn auto_enabled(p: &Profile) -> Result<bool> {
    let Some(glob) = p.remote_match.as_deref() else {
        return Ok(false);
    };
    let key = git::includeif_path_key(&git::hasconfig_condition(glob));
    Ok(git::get(Scope::Global, &key)?.is_some())
}

pub fn global_active(p: &Profile) -> Result<bool> {
    Ok(git::get(Scope::Global, "user.email")?.as_deref() == Some(p.user_email.as_str()))
}
