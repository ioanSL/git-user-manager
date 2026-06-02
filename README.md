# gum — git user manager

Manage multiple git identities (GitHub/GitLab accounts) and switch between them
on demand or automatically by a repo's remote URL — so commits never land under
the wrong name or push with the wrong key.

A **profile** bundles identity (`user.name`/`email`), optional commit signing,
HTTPS credentials, and an SSH key/host-alias. Profiles live in
`~/.config/git-user-manager/profiles.toml`; `gum` writes everything through the
real `git config` and never rewrites config it didn't generate.

## Install

```sh
# Homebrew (macOS + Linux, builds from source)
brew install ioanSL/tap/git-user-manager          # or: --HEAD for latest main

# Debian/Ubuntu
sudo apt install ./git-user-manager_0.1.0_amd64.deb   # or _arm64.deb

# Prebuilt static binary (any Linux) — needs only `git` on PATH
tar xzf git-user-manager-0.1.0-amd64-linux.tar.gz
install -Dm755 git-user-manager-0.1.0-amd64-linux/gum ~/.local/bin/gum
```

## Quickstart

```sh
# Define an account and let it auto-activate on its repos
gum add work --user-name "Ada" --email ada@work.dev \
  --remote 'git@github.com:work-org/**' \
  --ssh-key ~/.ssh/id_work --ssh-host-alias github.com-work \
  --sign-format ssh --sign-key ~/.ssh/id_work.pub --auto-sign
gum auto enable work        # switch identity automatically by remote URL

gum default work            # fallback identity when no rule matches
```

Run `gum` with no arguments for the interactive TUI.

## Commands

| Command | Does |
|---|---|
| `add` / `remove` / `list` / `show` | manage profiles |
| `default <p>` | set the global default identity (identity + signing, no transport) |
| `use <p> [--local]` | apply a profile to a scope (incl. transport); previews changes |
| `current` | show the active identity and which profile it matches |
| `auto enable\|disable\|status <p>` | automatic switching by remote URL (`includeIf`) |
| `ssh sync\|status` | manage `~/.ssh/config` host-alias blocks |
| `signers sync\|status` | maintain the allowed-signers file for SSH-signed commits |
| `doctor [--fix]` | audit global config for security/consistency issues |

## How it works

- **Auto-switch** — `auto enable` writes an `includeIf "hasconfig:remote.*.url:…"`
  rule pointing at a generated per-profile include file, so git applies the right
  identity, key, and signing wherever a matching remote lives.
- **SSH** — with `--ssh-host-alias`, `gum` manages a marker-delimited block in
  `~/.ssh/config` (kept `700`/`600`) and rewrites remotes through the alias;
  without one it uses `core.sshCommand`. Transport stays per-repo so multiple
  accounts on the same host never collide.
- **Signing** — for SSH-signing profiles, `gum` maintains
  `~/.config/git-user-manager/allowed_signers` and wires
  `gpg.ssh.allowedSignersFile` (without overriding one you set yourself).
- **doctor** — flags plaintext tokens / `user:password@` in config (redacted,
  auto-fixable), incomplete identity, missing `user.useConfigOnly`, and missing
  or unwired signing keys.

## Build & release

```sh
cargo build --release                                       # local binary
packaging/release.sh        # amd64+arm64 static binaries, tarballs, .debs, SHA256SUMS → dist/
```

arm64 cross-compiles with the bundled `rust-lld` (no external toolchain; see
`.cargo/config.toml`). Pushing a `v*` tag runs `.github/workflows/release.yml`
to build and attach all artifacts to a GitHub Release.

## License

MIT.
