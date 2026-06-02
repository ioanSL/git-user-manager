#!/usr/bin/env bash
# Build all release artifacts for git-user-manager into dist/:
#   - static musl binaries for amd64 + arm64
#   - a .tar.gz per arch (binary + README + LICENSE if present)
#   - a .deb per arch
#   - SHA256SUMS over everything
#
# Usage: packaging/release.sh [amd64|arm64 ...]   (default: both)
set -euo pipefail

cd "$(dirname "$0")/.."

PKG=git-user-manager
VERSION=$(awk -F\" '/^version[[:space:]]*=/ {print $2; exit}' Cargo.toml)
ARCHES=("$@")
[[ ${#ARCHES[@]} -eq 0 ]] && ARCHES=(amd64 arm64)

declare -A TRIPLE=(
  [amd64]=x86_64-unknown-linux-musl
  [arm64]=aarch64-unknown-linux-musl
)

mkdir -p dist

for ARCH in "${ARCHES[@]}"; do
  TARGET="${TRIPLE[$ARCH]:-}"
  [[ -z "$TARGET" ]] && { echo "unknown arch: $ARCH" >&2; exit 2; }

  echo "==> $ARCH ($TARGET)"
  rustup target add "$TARGET" >/dev/null 2>&1 || true
  cargo build --release --target "$TARGET"

  BIN="target/${TARGET}/release/gum"
  file "$BIN" | sed 's/^/    /'

  # Tarball: gum + docs, staged under a versioned dir.
  STAGE="$(mktemp -d)"; chmod 755 "$STAGE"
  DIR="${PKG}-${VERSION}-${ARCH}-linux"
  mkdir -p "$STAGE/$DIR"
  install -m755 "$BIN" "$STAGE/$DIR/gum"
  install -m644 README.md "$STAGE/$DIR/README.md"
  [[ -f LICENSE ]] && install -m644 LICENSE "$STAGE/$DIR/LICENSE"
  tar -C "$STAGE" -czf "dist/${DIR}.tar.gz" "$DIR"
  rm -rf "$STAGE"
  echo "    built: dist/${DIR}.tar.gz"

  # Debian package (reuses the per-arch builder).
  packaging/build-deb.sh "$ARCH" | sed 's/^/    /'
done

# Checksums over all current artifacts.
( cd dist && sha256sum "${PKG}"*.deb "${PKG}"*.tar.gz > SHA256SUMS )
echo "==> dist/"
ls -lh dist | awk 'NR>1 {print "    "$5"  "$9}'
echo "    checksums: dist/SHA256SUMS"
