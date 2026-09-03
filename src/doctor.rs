//! `gum doctor` — audit global git config for security/consistency problems.
//!
//! The headline check is plaintext credentials embedded in config (tokens in
//! `url.*.insteadOf` rules, or `user:password@` in URLs), which leak into
//! backups, screen-shares, and `git config --list` dumps.

use anyhow::Result;

use crate::git::{self, Scope};
use crate::profile::Registry;
use crate::signers;

/// Token prefixes we recognise as secrets that must never sit in plaintext.
const TOKEN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
];

/// Printed after a plaintext credential is removed from config.
const ROTATE_NOTE: &str = "Rotate this token now — it may already be compromised — then \
     authenticate via a credential helper (e.g. `git config --global \
     credential.helper libsecret`) or switch the remote to SSH.";

#[derive(Clone, Copy, PartialEq)]
pub enum Severity {
    Critical,
    Warn,
    Info,
}

impl Severity {
    pub fn tag(self) -> &'static str {
        match self {
            Severity::Critical => "CRIT",
            Severity::Warn => "WARN",
            Severity::Info => "INFO",
        }
    }
}

/// An automated remediation a finding can carry.
enum FixAction {
    /// Remove an entire `[section "subsection"]` from global config.
    RemoveSection(String),
    /// Set a key to a value in global config.
    SetGlobal(String, String),
}

pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    fix: Option<FixAction>,
}

impl Finding {
    pub fn is_fixable(&self) -> bool {
        self.fix.is_some()
    }

    /// A short, redacted description of what the fix will do.
    pub fn fix_prompt(&self) -> Option<String> {
        match &self.fix {
            Some(FixAction::RemoveSection(s)) => {
                Some(format!("Remove '{}' from ~/.gitconfig", redact(s)))
            }
            Some(FixAction::SetGlobal(k, v)) => Some(format!("Set {k} = {v} in ~/.gitconfig")),
            None => None,
        }
    }

    /// Apply this finding's fix to global config.
    pub fn apply(&self) -> Result<()> {
        match &self.fix {
            Some(FixAction::RemoveSection(s)) => git::remove_section(Scope::Global, s),
            Some(FixAction::SetGlobal(k, v)) => git::set(Scope::Global, k, v),
            None => Ok(()),
        }
    }
}

/// Return a human description if `s` embeds a secret, else `None`.
fn secret_kind(s: &str) -> Option<String> {
    for p in TOKEN_PREFIXES {
        if s.contains(p) {
            let label = p.trim_end_matches(['_', '-']);
            return Some(format!("contains a {label} token"));
        }
    }
    // user:password@host style credentials in a URL.
    if let Some((_, rest)) = s.split_once("://") {
        let authority = rest.split('/').next().unwrap_or("");
        if let Some((userinfo, _)) = authority.split_once('@') {
            if userinfo.contains(':') {
                return Some("embeds credentials (user:password@) in the URL".to_string());
            }
        }
    }
    None
}

/// Mask secrets in `s` for safe display.
fn redact(s: &str) -> String {
    let mut s = s.to_string();
    for p in TOKEN_PREFIXES {
        // Mask every occurrence of the prefix, not just the first.
        let mut pos = 0;
        while let Some(rel) = s[pos..].find(p) {
            let after = pos + rel + p.len();
            let end = s[after..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .map(|o| after + o)
                .unwrap_or(s.len());
            s.replace_range(after..end, "…");
            pos = after + "…".len(); // advance past the prefix + the mask
        }
    }
    // Mask user:password@ → user:***@
    if let Some(scheme_end) = s.find("://") {
        let rest_start = scheme_end + 3;
        if let Some(at_rel) = s[rest_start..].find('@') {
            let at = rest_start + at_rel;
            if let Some(colon_rel) = s[rest_start..at].find(':') {
                let colon = rest_start + colon_rel;
                s.replace_range(colon + 1..at, "***");
            }
        }
    }
    s
}

/// Run all checks against global config.
pub fn audit() -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let entries = git::list(Scope::Global)?;

    // 1. Plaintext secrets anywhere in global config.
    for (key, value) in &entries {
        let kind = secret_kind(key).or_else(|| secret_kind(value));
        let Some(kind) = kind else { continue };

        // The secret lives in the key for url.* rules, otherwise in the value.
        let location = if secret_kind(key).is_some() {
            redact(key)
        } else {
            format!("{key} = {}", redact(value))
        };

        let fix = if key.starts_with("url.") {
            // Drop the trailing `.insteadof`/`.pushinsteadof` to get the section.
            key.rsplit_once('.')
                .map(|(section, _)| FixAction::RemoveSection(section.to_string()))
        } else {
            None
        };

        findings.push(Finding {
            severity: Severity::Critical,
            title: "Plaintext credential in git config".to_string(),
            detail: format!("{location}\n       ({kind})"),
            fix,
        });
    }

    // 2. Identity set in global config?
    let name = git::get(Scope::Global, "user.name")?;
    let email = git::get(Scope::Global, "user.email")?;
    if name.is_none() || email.is_none() {
        findings.push(Finding {
            severity: Severity::Warn,
            title: "Global git identity is incomplete".to_string(),
            detail: format!(
                "user.name = {}, user.email = {}",
                name.as_deref().unwrap_or("(unset)"),
                email.as_deref().unwrap_or("(unset)")
            ),
            fix: None,
        });
    }

    // 3. Safety: refuse to auto-guess identity from hostname/env.
    let use_config_only = git::get(Scope::Global, "user.useConfigOnly")?;
    if use_config_only.as_deref() != Some("true") {
        findings.push(Finding {
            severity: Severity::Info,
            title: "user.useConfigOnly is not enabled".to_string(),
            detail: "Without it, git may silently commit under a guessed identity \
                     if none is configured."
                .to_string(),
            fix: Some(FixAction::SetGlobal(
                "user.useConfigOnly".to_string(),
                "true".to_string(),
            )),
        });
    }

    // 4/5. Profile-aware checks.
    if let Ok(reg) = Registry::load() {
        // 5. SSH commit-signature verification wiring.
        let ssh_signers = signers::ssh_signing_count(&reg.profiles);
        if ssh_signers > 0 {
            let resolved = signers::entries(&reg.profiles).len();
            if resolved < ssh_signers {
                findings.push(Finding {
                    severity: Severity::Warn,
                    title: "Some SSH signing keys could not be resolved".to_string(),
                    detail: format!(
                        "{}/{ssh_signers} signer(s) have a usable public key; \
                         the rest are missing a .pub",
                        resolved
                    ),
                    fix: None,
                });
            }
            if resolved > 0 {
                let want = signers::path()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
                let cur = git::get(Scope::Global, "gpg.ssh.allowedSignersFile")?;
                if cur.is_none() || (want.is_some() && cur != want) {
                    findings.push(Finding {
                        severity: Severity::Info,
                        title: "SSH signers are not wired for verification".to_string(),
                        detail: "gpg.ssh.allowedSignersFile is unset or points elsewhere; \
                                 SSH-signed commits won't verify against your profiles. \
                                 Run `gum signers sync`."
                            .to_string(),
                        fix: None,
                    });
                }
            }
        }

        // Profiles referencing key files that don't exist on disk.
        for p in &reg.profiles {
            let mut keys = Vec::new();
            if let Some(ssh) = &p.ssh {
                keys.push(("ssh", ssh.key.clone()));
            }
            if let Some(s) = &p.signing {
                if s.format == "ssh" {
                    keys.push(("signing", s.key.clone()));
                }
            }
            for (kind, key) in keys {
                if !key_file_exists(&key) {
                    findings.push(Finding {
                        severity: Severity::Warn,
                        title: format!("Profile '{}' references a missing {kind} key", p.name),
                        detail: format!("{key} does not exist"),
                        fix: None,
                    });
                }
            }
        }
    }

    Ok(findings)
}

/// Whether a key path exists, expanding a leading `~/`.
fn key_file_exists(path: &str) -> bool {
    let expanded = if path == "~" {
        match std::env::home_dir() {
            Some(home) => home,
            None => return true,
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        match std::env::home_dir() {
            Some(home) => home.join(rest),
            None => return true,
        }
    } else if path.starts_with('~') {
        return true;
    } else {
        std::path::PathBuf::from(path)
    };
    expanded.exists()
}

/// Entry point for `gum doctor [--fix] [--yes]`.
pub fn run(fix: bool, yes: bool, confirm: impl Fn(&str) -> Result<bool>) -> Result<()> {
    let findings = audit()?;
    if findings.is_empty() {
        println!("✓ No issues found.");
        return Ok(());
    }

    for (i, f) in findings.iter().enumerate() {
        println!("[{}] {}: {}", f.severity.tag(), i + 1, f.title);
        for line in f.detail.lines() {
            println!("       {line}");
        }
        if f.fix.is_some() && !fix {
            println!("       fix available — run `gum doctor --fix`");
        }
        println!();
    }

    if !fix {
        let fixable = findings.iter().filter(|f| f.fix.is_some()).count();
        println!(
            "{} issue(s) found, {fixable} auto-fixable. Re-run with --fix to apply.",
            findings.len()
        );
        return Ok(());
    }

    for f in &findings {
        let Some(prompt) = f.fix_prompt() else {
            continue;
        };
        if !yes && !confirm(&format!("{prompt}?"))? {
            println!("  skipped.");
            continue;
        }
        f.apply()?;
        println!("  fixed: {}", f.title);
        if matches!(f.fix, Some(FixAction::RemoveSection(_))) {
            println!("  → {ROTATE_NOTE}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_kind_detects_tokens_and_userinfo() {
        assert!(secret_kind("ghp_abcdEFGH").is_some());
        assert!(secret_kind("glpat-xyz").is_some());
        assert!(secret_kind("https://alice:pw@gitlab.com/").is_some());
        assert!(secret_kind("https://github.com/foo").is_none());
        assert!(secret_kind("just some text").is_none());
    }

    #[test]
    fn redact_masks_all_same_prefix_tokens() {
        let r = redact("token ghp_FIRSTaaaa; backup ghp_SECONDbbbb");
        assert!(!r.contains("FIRSTaaaa"), "{r}");
        assert!(!r.contains("SECONDbbbb"), "{r}");
        assert!(r.contains("ghp_…"));
    }

    #[test]
    fn redact_masks_userinfo_password() {
        let r = redact("https://alice:s3cret@gitlab.com/");
        assert!(!r.contains("s3cret"));
        assert!(r.contains("alice:***@"));
    }

    #[test]
    fn key_file_exists_tilde_user_not_flagged() {
        // ~user/ can't be resolved without libc → must not false-warn.
        assert!(key_file_exists("~nonexistentuser/.ssh/id_ed25519"));
    }

    #[test]
    fn key_file_exists_absolute_missing_is_false() {
        assert!(!key_file_exists("/definitely/not/here/xyz_0001"));
    }
}
