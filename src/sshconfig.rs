//! Manage per-profile `Host` aliases in `~/.ssh/config`, each in a delimited
//! block (`# >>> gum:<name> >>>` … `# <<< gum:<name> <<<`) so gum can update or
//! remove it without disturbing hand-written entries.

use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::actions;
use crate::profile::Profile;

fn start_marker(name: &str) -> String {
    format!("# >>> gum:{name} >>>")
}
fn end_marker(name: &str) -> String {
    format!("# <<< gum:{name} <<<")
}
const ANY_START: &str = "# >>> gum:";
const ANY_END: &str = "# <<< gum:";

pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".ssh").join("config"))
}

fn read_config() -> Result<String> {
    let path = config_path()?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Write back with `~/.ssh` at 700 and the file at 600.
fn write_config(content: &str) -> Result<()> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        set_mode(dir, 0o700);
    }
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    set_mode(&path, 0o600);
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}
#[cfg(not(unix))]
fn set_mode(_path: &std::path::Path, _mode: u32) {}

fn strip_block(content: &str, name: &str) -> String {
    drop_between(content, &start_marker(name), &end_marker(name), true)
}

fn strip_all(content: &str) -> String {
    drop_between(content, ANY_START, ANY_END, false)
}

/// Drop lines from a start marker through an end marker (inclusive). `exact`
/// requires marker equality; otherwise a prefix match.
fn drop_between(content: &str, start: &str, end: &str, exact: bool) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in content.lines() {
        let t = line.trim();
        let hit_start = if exact { t == start } else { t.starts_with(start) };
        let hit_end = if exact { t == end } else { t.starts_with(end) };
        if skipping {
            if hit_end {
                skipping = false;
            }
            continue;
        }
        if hit_start {
            skipping = true;
            continue;
        }
        out.push(line);
    }
    let mut s = out.join("\n");
    while s.ends_with("\n\n") {
        s.pop();
    }
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn render_block(p: &Profile) -> Option<String> {
    let ssh = p.ssh.as_ref()?;
    let alias = ssh.host_alias.as_deref()?;
    let host = actions::ssh_hostname(ssh);
    Some(format!(
        "{start}\n\
         Host {alias}\n    \
         HostName {host}\n    \
         User git\n    \
         IdentityFile {key}\n    \
         IdentitiesOnly yes\n\
         {end}\n",
        start = start_marker(&p.name),
        end = end_marker(&p.name),
        key = ssh.key,
    ))
}

/// Upsert or remove a single profile's alias block to match its current state.
pub fn apply_profile(p: &Profile) -> Result<()> {
    let stripped = strip_block(&read_config()?, &p.name);
    let content = match render_block(p) {
        Some(block) => {
            let mut c = stripped;
            if !c.is_empty() && !c.ends_with('\n') {
                c.push('\n');
            }
            if !c.is_empty() {
                c.push('\n');
            }
            c.push_str(&block);
            c
        }
        None => stripped,
    };
    write_config(&content)
}

pub fn remove(name: &str) -> Result<()> {
    let content = strip_block(&read_config()?, name);
    write_config(&content)
}

/// Rewrite all gum-managed blocks from the registry, pruning orphans.
pub fn sync(profiles: &[Profile]) -> Result<usize> {
    let mut content = strip_all(&read_config()?);
    let mut count = 0;
    for p in profiles {
        if let Some(block) = render_block(p) {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&block);
            count += 1;
        }
    }
    write_config(&content)?;
    Ok(count)
}

/// Profiles whose alias block is currently present, as (name, alias).
pub fn managed(profiles: &[Profile]) -> Result<Vec<(String, String)>> {
    let content = read_config()?;
    let mut out = Vec::new();
    for p in profiles {
        if content.lines().any(|l| l.trim() == start_marker(&p.name)) {
            if let Some(alias) = actions::effective_alias(p) {
                out.push((p.name.clone(), alias));
            }
        }
    }
    Ok(out)
}
