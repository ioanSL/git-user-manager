#!/usr/bin/env bash
# Build a .deb for git-user-manager from the static musl binary.
#
# Usage: packaging/build-deb.sh [amd64|arm64]   (default: amd64)
# Output: dist/git-user-manager_<version>_<arch>.deb
set -euo pipefail

cd "$(dirname "$0")/.."

PKG=git-user-manager
ARCH="${1:-amd64}"

case "$ARCH" in
  amd64) TARGET=x86_64-unknown-linux-musl ;;
  arm64) TARGET=aarch64-unknown-linux-musl ;;
  *) echo "unknown arch: $ARCH (use amd64 or arm64)" >&2; exit 2 ;;
esac

BIN="target/${TARGET}/release/gum"

# Read version from Cargo.toml ([package] version = "x.y.z").
VERSION=$(awk -F\" '/^version[[:space:]]*=/ {print $2; exit}' Cargo.toml)

# Build the static binary if it isn't there yet.
if [[ ! -f "$BIN" ]]; then
  echo "building static binary ($TARGET)..."
  rustup target add "$TARGET" >/dev/null 2>&1 || true
  cargo build --release --target "$TARGET"
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
chmod 755 "$STAGE"  # mktemp defaults to 700; dirs in a .deb should be 755

# Filesystem layout: /usr/bin/gum + docs.
install -Dm755 "$BIN" "$STAGE/usr/bin/gum"
install -Dm644 README.md "$STAGE/usr/share/doc/${PKG}/README.md"

# Debian control metadata.
mkdir -p "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<EOF
Package: ${PKG}
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${ARCH}
Depends: git
Maintainer: Ioan <ioan@raccoons.dev>
Description: Manage multiple git identities (GitHub/GitLab users)
 gum manages multiple git identities and switches between them, either
 imperatively (gum use) or automatically by remote URL via includeIf
 (gum auto). Ships an interactive TUI, SSH host-alias and allowed-signers
 upkeep, and a security doctor. The binary is statically linked.
EOF

mkdir -p dist
DEB="dist/${PKG}_${VERSION}_${ARCH}.deb"
dpkg-deb --build --root-owner-group "$STAGE" "$DEB"
echo "built: $DEB"
