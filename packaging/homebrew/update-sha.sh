#!/usr/bin/env bash
# Fill the Homebrew formula's url version + sha256 from a tagged GitHub release.
#
# Usage: packaging/homebrew/update-sha.sh <owner> <version>
#   e.g. packaging/homebrew/update-sha.sh ioanSL 0.1.0
#
# Downloads the auto-generated source tarball for tag vX.Y.Z, computes its
# sha256, and rewrites OWNER, the url version, and the sha256 in the formula.
set -euo pipefail

cd "$(dirname "$0")"
FORMULA="git-user-manager.rb"

OWNER="${1:?usage: update-sha.sh <owner> <version>}"
VERSION="${2:?usage: update-sha.sh <owner> <version>}"
URL="https://github.com/${OWNER}/git-user-manager/archive/refs/tags/v${VERSION}.tar.gz"

echo "fetching $URL"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT
curl -fsSL "$URL" -o "$TMP"
SHA="$(sha256sum "$TMP" | awk '{print $1}')"
echo "sha256: $SHA"

# Substitute OWNER, the tag in the url, and the sha256 line.
sed -i \
  -e "s#github.com/OWNER/#github.com/${OWNER}/#g" \
  -e "s#archive/refs/tags/v[0-9.]*\.tar\.gz#archive/refs/tags/v${VERSION}.tar.gz#" \
  -e "s#^\(\s*sha256\) \".*\"#\1 \"${SHA}\"#" \
  "$FORMULA"

echo "updated $FORMULA (owner=$OWNER version=$VERSION)"
