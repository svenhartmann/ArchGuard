#!/bin/sh
# Installs the latest published ArchGuard release on macOS, Linux or Git Bash (Windows).
#
#   curl -fsSL https://raw.githubusercontent.com/svenhartmann/ArchGuard/main/scripts/install/install.sh | sh
#
# Environment:
#   ARCHGUARD_VERSION       version to install, e.g. 0.1.3 (default: latest release)
#   ARCHGUARD_INSTALL_DIR   target directory (default: $HOME/.local/bin)
#   ARCHGUARD_DOWNLOAD_URL  base URL with the release assets (default: GitHub release of that version)
set -eu

REPO="svenhartmann/ArchGuard"
INSTALL_DIR="${ARCHGUARD_INSTALL_DIR:-$HOME/.local/bin}"

fail() { echo "archguard install: $*" >&2; exit 1; }

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
    fetch_stdout() { curl -fsSL --retry 3 "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
    fetch_stdout() { wget -q -O - "$1"; }
else
    fail "curl or wget is required"
fi

detect_target() {
    case "$(uname -m)" in
        x86_64 | amd64) arch=x86_64 ;;
        arm64 | aarch64) arch=aarch64 ;;
        *) fail "unsupported CPU architecture: $(uname -m)" ;;
    esac
    case "$(uname -s)" in
        Darwin) echo "$arch-apple-darwin" ;;
        Linux) echo "$arch-unknown-linux-musl" ;;
        MINGW* | MSYS* | CYGWIN*) echo "x86_64-pc-windows-msvc" ;;
        *) fail "unsupported OS: $(uname -s); on Windows use install.ps1" ;;
    esac
}

latest_version() {
    # The API only lists published releases, never drafts or pre-releases.
    fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d ' ' -f 1
    else fail "sha256sum or shasum is required to verify the download"
    fi
}

target="$(detect_target)"
version="${ARCHGUARD_VERSION:-$(latest_version)}"
version="${version#v}"
[ -n "$version" ] || fail "could not determine the latest release"
base="${ARCHGUARD_DOWNLOAD_URL:-https://github.com/$REPO/releases/download/v$version}"

case "$target" in
    *windows*) ext=zip; binary=archguard.exe ;;
    *) ext=tar.gz; binary=archguard ;;
esac
stem="archguard-$version-$target"
archive="$stem.$ext"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Downloading ArchGuard $version for $target ..."
fetch "$base/$archive" "$tmp/$archive" || fail "download failed: $base/$archive"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "download failed: $base/SHA256SUMS"

expected="$(awk -v name="$archive" '$2 == name { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || fail "no checksum for $archive in SHA256SUMS"
[ "$(sha256 "$tmp/$archive")" = "$expected" ] || fail "checksum mismatch for $archive"

mkdir "$tmp/x"
if [ "$ext" = zip ]; then
    command -v unzip >/dev/null 2>&1 || fail "unzip is required"
    unzip -q "$tmp/$archive" -d "$tmp/x"
else
    tar -xzf "$tmp/$archive" -C "$tmp/x"
fi
[ -f "$tmp/x/$stem/$binary" ] || fail "archive does not contain $stem/$binary"

mkdir -p "$INSTALL_DIR"
# Copy next to the target first, then rename, so a running binary is replaced atomically.
cp "$tmp/x/$stem/$binary" "$INSTALL_DIR/.$binary.new"
chmod 755 "$INSTALL_DIR/.$binary.new"
mv -f "$INSTALL_DIR/.$binary.new" "$INSTALL_DIR/$binary"

echo "Installed $INSTALL_DIR/$binary"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Note: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
       echo "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
echo "Update later with: archguard update"
