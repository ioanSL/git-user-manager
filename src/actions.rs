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
                out.push((
                    format!("url.git@{alias}:.insteadOf"),
                    format!("git@{host}:"),
                ));
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
    let glob = p
        .remote_match
        .as_deref()
        .context("profile has no remote glob")?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Signing, Ssh};

    fn base() -> Profile {
        Profile {
            name: "w".into(),
            user_name: "N".into(),
            user_email: "e@x".into(),
            ..Default::default()
        }
    }

    fn has(s: &[(String, String)], k: &str) -> bool {
        s.iter().any(|(key, _)| key == k)
    }

    #[test]
    fn identity_only_is_name_and_email() {
        assert_eq!(
            identity_settings(&base()),
            vec![
                ("user.name".to_string(), "N".to_string()),
                ("user.email".to_string(), "e@x".to_string()),
            ]
        );
    }

    #[test]
    fn signing_autosign_adds_gpgsign() {
        let mut p = base();
        p.signing = Some(Signing {
            format: "ssh".into(),
            key: "/k.pub".into(),
            auto_sign: true,
        });
        let s = identity_settings(&p);
        assert!(has(&s, "gpg.format"));
        assert!(has(&s, "user.signingkey"));
        assert!(s.iter().any(|(k, v)| k == "commit.gpgsign" && v == "true"));
    }

    #[test]
    fn ssh_alias_rewrites_both_url_forms_and_no_sshcommand() {
        let mut p = base();
        p.ssh = Some(Ssh {
            key: "/k".into(),
            hostname: None,
            host_alias: Some("github.com-w".into()),
        });
        let s = profile_settings(&p);
        assert!(s.contains(&(
            "url.git@github.com-w:.insteadOf".to_string(),
            "git@github.com:".to_string()
        )));
        assert!(has(&s, "url.ssh://git@github.com-w/.insteadOf"));
        assert!(!has(&s, "core.sshCommand"));
    }

    #[test]
    fn ssh_key_only_uses_sshcommand() {
        let mut p = base();
        p.ssh = Some(Ssh {
            key: "/k".into(),
            hostname: None,
            host_alias: None,
        });
        let s = profile_settings(&p);
        assert!(s
            .iter()
            .any(|(k, v)| k == "core.sshCommand" && v.contains("/k")));
        assert!(!has(&s, "url.git@github.com-w:.insteadOf"));
    }

    #[test]
    fn https_credentials_set_usehttppath() {
        let mut p = base();
        p.host = Some("https://github.com".into());
        p.username = Some("ada".into());
        let s = profile_settings(&p);
        assert!(s.contains(&(
            "credential.https://github.com.username".to_string(),
            "ada".to_string()
        )));
        assert!(has(&s, "credential.useHttpPath"));
    }
}
