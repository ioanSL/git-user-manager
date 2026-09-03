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
    let home = std::env::home_dir().context("could not determine home directory")?;
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
        let hit_start = if exact {
            t == start
        } else {
            t.starts_with(start)
        };
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

/// Append `block`, separated from any existing content by one blank line.
fn append_block(content: &mut String, block: &str) {
    if !content.is_empty() {
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
    }
    content.push_str(block);
}

/// Upsert or remove a single profile's alias block to match its current state.
pub fn apply_profile(p: &Profile) -> Result<()> {
    let mut content = strip_block(&read_config()?, &p.name);
    if let Some(block) = render_block(p) {
        append_block(&mut content, &block);
    }
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
            append_block(&mut content, &block);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Ssh;

    fn profile(name: &str, alias: Option<&str>) -> Profile {
        Profile {
            name: name.into(),
            user_name: "N".into(),
            user_email: "e".into(),
            ssh: alias.map(|a| Ssh {
                key: "/k".into(),
                hostname: None,
                host_alias: Some(a.into()),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn strip_block_removes_only_named_block() {
        let c = "before\n# >>> gum:work >>>\nHost x\n# <<< gum:work <<<\nafter\n";
        let out = strip_block(c, "work");
        assert!(out.contains("before") && out.contains("after"));
        assert!(!out.contains("Host x"));
    }

    #[test]
    fn strip_block_does_not_collide_on_name_prefix() {
        // Stripping "work" must leave the "work2" block intact (exact marker match).
        let c = "# >>> gum:work2 >>>\nHost y\n# <<< gum:work2 <<<\n";
        let out = strip_block(c, "work");
        assert!(out.contains("Host y"));
    }

    #[test]
    fn render_block_contains_expected_fields() {
        let b = render_block(&profile("work", Some("github.com-work"))).unwrap();
        assert!(b.contains("Host github.com-work"));
        assert!(b.contains("HostName github.com"));
        assert!(b.contains("IdentityFile /k"));
        assert!(b.contains("IdentitiesOnly yes"));
    }

    #[test]
    fn render_block_none_without_alias() {
        assert!(render_block(&profile("work", None)).is_none());
    }

    #[test]
    fn apply_then_strip_round_trips() {
        // Simulate: hand-written content + a managed block, then strip it.
        let block = render_block(&profile("work", Some("gh-w"))).unwrap();
        let combined = format!("Host manual\n  HostName 10.0.0.1\n\n{block}");
        let stripped = strip_block(&combined, "work");
        assert!(stripped.contains("Host manual"));
        assert!(!stripped.contains("gh-w"));
    }
}
