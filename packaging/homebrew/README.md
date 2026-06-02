# Homebrew formula

`git-user-manager.rb` is a **build-from-source** formula: Homebrew compiles it
with Rust, so the same formula works on macOS (Intel + Apple Silicon) and Linux
without shipping per-OS binaries. It declares `git` as a runtime dependency.

## Try it now (no release needed)

```sh
brew install --HEAD ioanSL/tap/git-user-manager
# or point directly at the file in a checkout:
brew install --HEAD --build-from-source ./packaging/homebrew/git-user-manager.rb
```

## Releases are automated

Pushing a `v*` tag runs `.github/workflows/release.yml`, which refreshes this
formula (`update-sha.sh` sets the tag's `url` + `sha256`), attaches it to the
GitHub Release, and — if a tap token is configured — pushes it to the tap. To
do it by hand instead:

```sh
packaging/homebrew/update-sha.sh ioanSL 0.1.0   # after tagging v0.1.0
```

## Setting up the tap (one time)

1. Create `github.com/ioanSL/homebrew-tap` (the formula lands in its `Formula/`).
2. Create a PAT with `contents:write` on that repo and add it to **this** repo's
   Actions secrets as `HOMEBREW_TAP_TOKEN`. Without it, the release still builds
   and attaches the formula — it just won't auto-push to the tap.

Users then install with:

```sh
brew install ioanSL/tap/git-user-manager
```

> Prefer prebuilt binaries over compiling? The `v*` release workflow already
> publishes static `.tar.gz` / `.deb` artifacts for amd64 + arm64 Linux; a
> binary (bottle-style) formula could download those instead. Build-from-source
> is used here because macOS binaries aren't produced by the release flow.
