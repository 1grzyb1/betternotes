#!/bin/sh
# Installs the latest BetterNotes release on macOS or Linux.
#   curl -fsSL https://raw.githubusercontent.com/1grzyb1/betternotes/main/install.sh | sh
set -eu

REPO="1grzyb1/betternotes"
BASE="https://github.com/$REPO/releases/latest/download"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

download() {
    echo "Downloading $1..."
    curl -fL --progress-bar "$BASE/$1" -o "$tmp/$1"
}

case "$(uname -s)" in
Darwin)
    download BetterNotes-macos-universal.zip
    ditto -x -k "$tmp/BetterNotes-macos-universal.zip" "$tmp"
    dest=/Applications
    [ -w "$dest" ] || dest="$HOME/Applications"
    mkdir -p "$dest"
    rm -rf "$dest/BetterNotes.app"
    mv "$tmp/BetterNotes.app" "$dest/"
    # The app is not notarized; clear the quarantine flag so it opens normally.
    xattr -dr com.apple.quarantine "$dest/BetterNotes.app" 2>/dev/null || true
    echo "Installed to $dest/BetterNotes.app"
    ;;
Linux)
    [ "$(uname -m)" = "x86_64" ] || { echo "Only x86_64 Linux builds are available." >&2; exit 1; }
    download BetterNotes-linux-x86_64.tar.gz
    tar xzf "$tmp/BetterNotes-linux-x86_64.tar.gz" -C "$tmp"
    bin="$HOME/.local/bin"
    share="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$bin" "$share/applications" "$share/icons/hicolor/1024x1024/apps"
    install -m 755 "$tmp/BetterNotes/betternotes" "$bin/betternotes"
    install -m 644 "$tmp/BetterNotes/betternotes.png" "$share/icons/hicolor/1024x1024/apps/betternotes.png"
    cat > "$share/applications/betternotes.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=BetterNotes
Comment=Handwritten notebook
Exec=$bin/betternotes
Icon=betternotes
Categories=Office;Graphics;
EOF
    command -v update-desktop-database >/dev/null && update-desktop-database "$share/applications" 2>/dev/null || true
    echo "Installed to $bin/betternotes"
    case ":$PATH:" in *":$bin:"*) ;; *) echo "Note: $bin is not on your PATH." ;; esac
    ;;
*)
    echo "Unsupported system: $(uname -s). On Windows use install.ps1." >&2
    exit 1
    ;;
esac
