//! Maintain the SSH allowed-signers file used to verify SSH-signed commits.
//! For each SSH-signing profile, resolve its public key and write a
//! `<email> <keytype> <base64>` line, then point `gpg.ssh.allowedSignersFile`
//! at it — unless the user already set that to a file of their own.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::git::{self, Scope};
use crate::profile::{Profile, Registry};

const HEADER: &str =
    "# Managed by git-user-manager. Maps committer emails to SSH signing keys.\n";

const CONFIG_KEY: &str = "gpg.ssh.allowedSignersFile";

pub fn path() -> Result<PathBuf> {
    Ok(Registry::config_dir()?.join("allowed_signers"))
}

fn is_keytype(t: &str) -> bool {
    t.starts_with("ssh-") || t.starts_with("ecdsa-") || t.starts_with("sk-")
}

fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn parse_pubkey_line(line: &str) -> Option<(String, String)> {
    let mut it = line.split_whitespace();
    let keytype = it.next()?;
    let b64 = it.next()?;
    is_keytype(keytype).then(|| (keytype.to_string(), b64.to_string()))
}

/// Resolve a signing key to `(keytype, base64)`: accepts a literal key, a
/// `.pub` path, or a private-key path whose `.pub` sibling exists.
fn resolve_pubkey(key: &str) -> Option<(String, String)> {
    if is_keytype(key.split_whitespace().next().unwrap_or("")) {
        return parse_pubkey_line(key);
    }
    let expanded = expand(key);
    let mut candidates = vec![expanded.clone()];
    if expanded.extension().map(|e| e != "pub").unwrap_or(true) {
        let mut p = expanded.into_os_string();
        p.push(".pub");
        candidates.push(PathBuf::from(p));
    }
    for c in candidates {
        if let Ok(content) = std::fs::read_to_string(&c) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(parsed) = parse_pubkey_line(line) {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

pub struct Entry {
    pub email: String,
    pub keytype: String,
    pub b64: String,
}

pub fn ssh_signing_count(profiles: &[Profile]) -> usize {
    profiles
        .iter()
        .filter(|p| p.signing.as_ref().map(|s| s.format == "ssh").unwrap_or(false))
        .count()
}

/// Entries for SSH-signing profiles whose public key was resolved.
pub fn entries(profiles: &[Profile]) -> Vec<Entry> {
    let mut out = Vec::new();
    for p in profiles {
        let Some(s) = &p.signing else { continue };
        if s.format != "ssh" {
            continue;
        }
        if let Some((keytype, b64)) = resolve_pubkey(&s.key) {
            out.push(Entry {
                email: p.user_email.clone(),
                keytype,
                b64,
            });
        }
    }
    out
}

pub struct SyncReport {
    pub written: usize,
    /// SSH-signing profiles whose public key could not be resolved.
    pub skipped: usize,
    pub wired: bool,
    /// A user-set allowed-signers file we declined to override.
    pub foreign: Option<String>,
    pub path: PathBuf,
}

pub fn sync(profiles: &[Profile]) -> Result<SyncReport> {
    let es = entries(profiles);
    let skipped = ssh_signing_count(profiles).saturating_sub(es.len());
    let path = path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut content = String::from(HEADER);
    for e in &es {
        content.push_str(&format!("{} {} {}\n", e.email, e.keytype, e.b64));
    }
    std::fs::write(&path, &content).with_context(|| format!("writing {}", path.display()))?;

    let path_str = path.to_string_lossy().to_string();
    let (mut wired, mut foreign) = (false, None);
    match git::get(Scope::Global, CONFIG_KEY)? {
        None => {
            if !es.is_empty() {
                git::set(Scope::Global, CONFIG_KEY, &path_str)?;
                wired = true;
            }
        }
        Some(cur) if cur == path_str => wired = true,
        Some(cur) => foreign = Some(cur),
    }
    Ok(SyncReport {
        written: es.len(),
        skipped,
        wired,
        foreign,
        path,
    })
}
