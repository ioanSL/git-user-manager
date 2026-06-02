//! Profile data model and the TOML registry.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signing {
    /// "ssh" or "openpgp".
    pub format: String,
    /// GPG key id, or path to an SSH public key when `format == "ssh"`.
    pub key: String,
    #[serde(default)]
    pub auto_sign: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ssh {
    /// Path to the private key, e.g. `~/.ssh/id_work`.
    pub key: String,
    /// Real host the alias points at (default `github.com`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// `~/.ssh/config` alias. Set → remotes route through it; unset →
    /// `core.sshCommand` is used instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Slug used to address the profile, e.g. "work".
    pub name: String,
    pub user_name: String,
    pub user_email: String,
    /// Credential host base, e.g. "https://github.com".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Account username, to disambiguate HTTPS credentials per host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Glob for `hasconfig` auto-switching, e.g. "https://github.com/work/**".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_match: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing: Option<Signing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<Ssh>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default, rename = "profile")]
    pub profiles: Vec<Profile>,
}

impl Registry {
    pub fn config_dir() -> Result<PathBuf> {
        let base = dirs::config_dir().context("could not determine config directory")?;
        Ok(base.join("git-user-manager"))
    }

    pub fn path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("profiles.toml"))
    }

    pub fn includes_dir() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("profiles"))
    }

    pub fn include_path(name: &str) -> Result<PathBuf> {
        Ok(Self::includes_dir()?.join(format!("{name}.gitconfig")))
    }

    /// Load the registry, returning an empty one if the file is absent.
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing registry")?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }

    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn add(&mut self, profile: Profile) -> Result<()> {
        if self.get(&profile.name).is_some() {
            bail!("a profile named '{}' already exists", profile.name);
        }
        self.profiles.push(profile);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<Profile> {
        let idx = self
            .profiles
            .iter()
            .position(|p| p.name == name)
            .with_context(|| format!("no profile named '{name}'"))?;
        Ok(self.profiles.remove(idx))
    }
}
