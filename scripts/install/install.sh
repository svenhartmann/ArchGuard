#!/bin/sh
# Installs the latest published ArchGuard release on macOS, Linux or Git Bash (Windows).
#
#   curl -fsSL https://raw.githubusercontent.com/svenhartmann/ArchGuard/main/scripts/install/install.sh | sh
#
# Environment:
#   ARCHGUARD_VERSION       version to install, e.g. 0.1.3 (default: latest release)
#   ARCHGUARD_INSTALL_DIR   target directory (default: $HOME/.local/bin)
#   ARCHGUARD_DOWNLOAD_URL  base URL with the release assets (default: GitHub release of that version)
#   ARCHGUARD_DESKTOP       0 skips the desktop app (default: install it where a package exists)
#   ARCHGUARD_APP_DIR       macOS only: where ArchGuard.app goes (default: $HOME/Applications)
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

# verify <file> <sums file>: the file's SHA-256 must match its line in the sums file.
verify() {
    name="$(basename "$1")"
    expected="$(awk -v name="$name" '$2 == name { print $1 }' "$2")"
    [ -n "$expected" ] || fail "no checksum for $name in $(basename "$2")"
    [ "$(sha256 "$1")" = "$expected" ] || fail "checksum mismatch for $name"
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

verify "$tmp/$archive" "$tmp/SHA256SUMS"

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

# Desktop app: unsigned packages from the same release, checked against SHA256SUMS-desktop.
install_desktop() {
    case "$target" in
        *apple-darwin) package="ArchGuard-$version-$target.dmg" ;;
        x86_64-unknown-linux-musl) package="ArchGuard-$version-x86_64-unknown-linux-gnu.AppImage" ;;
        *windows*) package="ArchGuard-$version-$target.zip" ;;
        *) echo "No desktop app for $target; skipped."; return 0 ;;
    esac
    if ! fetch "$base/$package" "$tmp/$package" 2>/dev/null || ! fetch "$base/SHA256SUMS-desktop" "$tmp/SHA256SUMS-desktop" 2>/dev/null; then
        echo "No desktop app in release $version; skipped."
        return 0
    fi
    verify "$tmp/$package" "$tmp/SHA256SUMS-desktop"
    case "$package" in
        *.dmg)
            app_dir="${ARCHGUARD_APP_DIR:-$HOME/Applications}"
            mnt="$tmp/mnt"
            mkdir "$mnt"
            hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mnt" "$tmp/$package" || fail "could not mount $package"
            [ -d "$mnt/ArchGuard.app" ] || { hdiutil detach -quiet "$mnt"; fail "$package does not contain ArchGuard.app"; }
            mkdir -p "$app_dir"
            rm -rf "$app_dir/.ArchGuard.app.new"
            ditto "$mnt/ArchGuard.app" "$app_dir/.ArchGuard.app.new"
            hdiutil detach -quiet "$mnt"
            rm -rf "$app_dir/ArchGuard.app"
            mv "$app_dir/.ArchGuard.app.new" "$app_dir/ArchGuard.app"
            echo "Installed $app_dir/ArchGuard.app"
            ;;
        *.AppImage)
            cp "$tmp/$package" "$INSTALL_DIR/.archguard-desktop.new"
            chmod 755 "$INSTALL_DIR/.archguard-desktop.new"
            mv -f "$INSTALL_DIR/.archguard-desktop.new" "$INSTALL_DIR/archguard-desktop"
            echo "Installed $INSTALL_DIR/archguard-desktop (AppImage, needs glibc and FUSE)"
            ;;
        *.zip)
            mkdir "$tmp/d"
            unzip -q "$tmp/$package" -d "$tmp/d"
            exe="$tmp/d/${package%.zip}/archguard-desktop.exe"
            [ -f "$exe" ] || fail "$package does not contain archguard-desktop.exe"
            cp "$exe" "$INSTALL_DIR/archguard-desktop.exe"
            echo "Installed $INSTALL_DIR/archguard-desktop.exe"
            ;;
    esac
}
[ "${ARCHGUARD_DESKTOP:-1}" = 0 ] || install_desktop
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Note: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
       echo "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
echo "Update later with: archguard update"
