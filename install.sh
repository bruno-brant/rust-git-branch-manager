#!/bin/sh
# Install git-branch-manager on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/bruno-brant/rust-git-branch-manager/main/install.sh | sh
#
# Options (when running the script directly, not through a pipe):
#   --version vX.Y.Z   install a specific release instead of the newest
#   --dir PATH         install somewhere other than ~/.local/bin
#
# Windows users want install.ps1 instead.

set -eu

REPO="bruno-brant/rust-git-branch-manager"
# Overridable so the script can be tested, or pointed at a fork or mirror.
BASE="${GBM_BASE_URL:-https://github.com/$REPO}"
BIN="git-branch-manager"
INSTALL_DIR="${GBM_INSTALL_DIR:-$HOME/.local/bin}"
VERSION=""

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; [ -n "$VERSION" ] || die "--version needs a value"; shift 2 ;;
    --dir) INSTALL_DIR="${2:-}"; [ -n "$INSTALL_DIR" ] || die "--dir needs a value"; shift 2 ;;
    -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"

# --- Which build? ------------------------------------------------------------
# Only x86-64 binaries are published. macOS ships a universal binary, so Apple
# Silicon is covered by it; Linux on arm64 has nothing to download and must say
# so rather than installing an x86-64 binary that dies with "exec format error".
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)
    case "$arch" in
      x86_64 | amd64) platform="linux-x86_64-static" ;;
      *) die "no prebuilt binary for Linux $arch — build from source with:
  cargo install --git https://github.com/$REPO" ;;
    esac
    ;;
  Darwin)
    case "$arch" in
      x86_64 | arm64) platform="macos-universal" ;;
      *) die "no prebuilt binary for macOS $arch" ;;
    esac
    ;;
  *) die "unsupported system: $os (on Windows use install.ps1)" ;;
esac

# --- Which version? ----------------------------------------------------------
# /releases/latest redirects to the tag page, so the last segment of the
# resolved URL is the version. No API token and no hourly rate limit.
if [ -z "$VERSION" ]; then
  latest_url="$(curl -fsSLo /dev/null -w '%{url_effective}' "$BASE/releases/latest")" \
    || die "could not reach GitHub to find the latest release"
  VERSION="${latest_url##*/}"
  case "$VERSION" in v*) ;; *) die "could not parse a version from $latest_url" ;; esac
fi

asset="$BIN-$VERSION-$platform.tar.gz"
say "Installing $BIN $VERSION ($platform) into $INSTALL_DIR"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

curl -fsSL -o "$tmp/$asset" "$BASE/releases/download/$VERSION/$asset" \
  || die "download failed: $BASE/releases/download/$VERSION/$asset"

# --- Verify ------------------------------------------------------------------
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else return 1
  fi
}

# Releases before v0.2.1 have no SHA256SUMS. Verify whenever it is published,
# and say plainly when it is not, rather than implying a check that did not run.
if curl -fsSL -o "$tmp/SHA256SUMS" "$BASE/releases/download/$VERSION/SHA256SUMS" 2>/dev/null; then
  expected="$(awk -v f="$asset" '$2 == f || $2 == "*" f {print $1}' "$tmp/SHA256SUMS")"
  [ -n "$expected" ] || die "SHA256SUMS has no entry for $asset"
  actual="$(sha256_of "$tmp/$asset")" || die "no sha256sum or shasum available to verify the download"
  [ "$expected" = "$actual" ] || die "checksum mismatch for $asset
  expected $expected
  actual   $actual"
  say "Checksum verified."
else
  warn "this release publishes no SHA256SUMS; the download was not verified"
fi

# --- Install -----------------------------------------------------------------
tar -xzf "$tmp/$asset" -C "$tmp"
[ -f "$tmp/$BIN-$VERSION-$platform/$BIN" ] || die "archive did not contain $BIN where expected"

mkdir -p "$INSTALL_DIR"
mv "$tmp/$BIN-$VERSION-$platform/$BIN" "$INSTALL_DIR/$BIN"
chmod +x "$INSTALL_DIR/$BIN"

# Unsigned binaries downloaded through a browser are quarantined by Gatekeeper.
# curl does not set the flag, so this is belt and braces and may fail harmlessly.
if [ "$os" = "Darwin" ]; then
  xattr -d com.apple.quarantine "$INSTALL_DIR/$BIN" 2>/dev/null || true
fi

say "Installed $INSTALL_DIR/$BIN"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) say "Run it from inside any git repository: $BIN" ;;
  *) say "
$INSTALL_DIR is not on your PATH. Add it:
  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
