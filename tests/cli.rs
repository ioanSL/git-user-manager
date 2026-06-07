//! End-to-end CLI tests. Each runs the real `gum` binary against an isolated
//! HOME + XDG_CONFIG_HOME tempdir, so the real git/ssh config is never touched.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use std::path::Path;
use tempfile::TempDir;

struct Env {
    home: TempDir,
}

impl Env {
    fn new() -> Self {
        Env {
            home: TempDir::new().unwrap(),
        }
    }

    fn path(&self) -> &Path {
        self.home.path()
    }

    /// A `gum` invocation pinned to this env's isolated HOME.
    fn gum(&self) -> Command {
        let mut c = Command::cargo_bin("gum").unwrap();
        c.env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.path().join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        c
    }

    fn git(&self, dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.path().join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn git_out(&self, dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("HOME", self.path())
            .env("XDG_CONFIG_HOME", self.path().join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn gitconfig(&self) -> String {
        std::fs::read_to_string(self.path().join(".gitconfig")).unwrap_or_default()
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path().join(rel)).unwrap_or_default()
    }

    /// Write a public key file and return its path as a string.
    fn write_pubkey(&self, name: &str, b64: &str) -> String {
        let p = self.path().join(format!("{name}.pub"));
        std::fs::write(&p, format!("ssh-ed25519 {b64} test@host\n")).unwrap();
        p.to_string_lossy().to_string()
    }
}

#[test]
fn add_list_show_and_duplicate() {
    let e = Env::new();
    e.gum()
        .args([
            "add",
            "work",
            "--user-name",
            "Ada",
            "--email",
            "ada@work.dev",
        ])
        .assert()
        .success();
    e.gum()
        .arg("list")
        .assert()
        .success()
        .stdout(contains("work"));
    e.gum()
        .args(["show", "work"])
        .assert()
        .success()
        .stdout(contains("ada@work.dev"));
    // Duplicate name must fail.
    e.gum()
        .args(["add", "work", "--user-name", "X", "--email", "x@y.dev"])
        .assert()
        .failure();
}

#[test]
fn default_sets_global_identity_only() {
    let e = Env::new();
    e.gum()
        .args([
            "add",
            "work",
            "--user-name",
            "Ada",
            "--email",
            "ada@work.dev",
        ])
        .assert()
        .success();
    e.gum().args(["default", "work"]).assert().success();
    let cfg = e.gitconfig();
    assert!(cfg.contains("ada@work.dev"));
}

#[test]
fn use_global_writes_credentials() {
    let e = Env::new();
    e.gum()
        .args([
            "add",
            "w",
            "--user-name",
            "A",
            "--email",
            "a@x.dev",
            "--host",
            "https://github.com",
            "--username",
            "auser",
        ])
        .assert()
        .success();
    e.gum()
        .args(["use", "w", "--global", "-y"])
        .assert()
        .success();
    let cfg = e.gitconfig().to_lowercase();
    assert!(cfg.contains("a@x.dev"));
    assert!(cfg.contains("usehttppath"));
}

#[test]
fn auto_switch_resolves_identity_by_remote() {
    let e = Env::new();
    e.gum()
        .args([
            "add",
            "work",
            "--user-name",
            "Ada",
            "--email",
            "ada@work.dev",
            "--remote",
            "git@github.com:work-org/**",
        ])
        .assert()
        .success();
    e.gum().args(["auto", "enable", "work"]).assert().success();

    let repo = e.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    e.git(&repo, &["init", "-q"]);
    e.git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:work-org/proj.git",
        ],
    );
    assert_eq!(e.git_out(&repo, &["config", "user.email"]), "ada@work.dev");
}

#[test]
fn ssh_alias_block_written_and_listed() {
    let e = Env::new();
    let key = e.path().join("id_work");
    std::fs::write(&key, "PRIV").unwrap();
    e.gum()
        .args([
            "add",
            "work",
            "--user-name",
            "A",
            "--email",
            "a@x.dev",
            "--ssh-key",
            key.to_str().unwrap(),
            "--ssh-host-alias",
            "github.com-work",
        ])
        .assert()
        .success();
    let cfg = e.read(".ssh/config");
    assert!(cfg.contains("Host github.com-work"));
    assert!(cfg.contains("IdentityFile"));
    e.gum()
        .args(["ssh", "status"])
        .assert()
        .success()
        .stdout(contains("github.com-work"));
}

#[test]
fn signers_file_generated_and_wired() {
    let e = Env::new();
    let pubkey = e.write_pubkey("id_s", "AAAATESTKEYsigner");
    e.gum()
        .args([
            "add",
            "s",
            "--user-name",
            "S",
            "--email",
            "s@x.dev",
            "--sign-format",
            "ssh",
            "--sign-key",
            &pubkey,
            "--auto-sign",
        ])
        .assert()
        .success();
    let allowed = e.read(".config/git-user-manager/allowed_signers");
    assert!(allowed.contains("s@x.dev"));
    assert!(allowed.contains("AAAATESTKEYsigner"));
    assert!(e.gitconfig().to_lowercase().contains("allowedsignersfile"));
}

#[test]
fn doctor_fix_removes_token_and_enables_useconfigonly() {
    let e = Env::new();
    e.git(
        e.path(),
        &[
            "config",
            "--global",
            "url.https://ghp_FAKEtoken12345@github.com/org/.insteadOf",
            "https://github.com/org/",
        ],
    );
    // Report (no --fix) flags it without leaking the token in full.
    e.gum()
        .arg("doctor")
        .assert()
        .success()
        .stdout(contains("Plaintext credential"))
        .stdout(contains("ghp_FAKEtoken12345").not());
    // Fix removes it and turns on the safety knob.
    e.gum().args(["doctor", "--fix", "-y"]).assert().success();
    let cfg = e.gitconfig().to_lowercase();
    assert!(!cfg.contains("ghp_faketoken12345"));
    assert!(cfg.contains("useconfigonly"));
}

#[test]
fn remove_tears_down_profile_and_ssh_block() {
    let e = Env::new();
    let key = e.path().join("id_r");
    std::fs::write(&key, "PRIV").unwrap();
    e.gum()
        .args([
            "add",
            "r",
            "--user-name",
            "R",
            "--email",
            "r@x.dev",
            "--ssh-key",
            key.to_str().unwrap(),
            "--ssh-host-alias",
            "github.com-r",
        ])
        .assert()
        .success();
    e.gum().args(["remove", "r"]).assert().success();
    e.gum()
        .arg("list")
        .assert()
        .success()
        .stdout(contains("No profiles"));
    assert!(!e.read(".ssh/config").contains("github.com-r"));
}
