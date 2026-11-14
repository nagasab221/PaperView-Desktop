#!/bin/sh
# One-line install of PaperView for Linux (x86_64), no root needed:
#
#   curl -fsSL https://raw.githubusercontent.com/nagasab221/PaperView-Desktop/main/install.sh | sh
#
# Downloads the latest release, installs it into ~/.local/opt/paperview, and
# adds PaperView to your app menu. Run it again any time to update.
set -eu

URL="https://github.com/nagasab221/PaperView-Desktop/releases/latest/download/PaperView-linux-x86_64.tar.gz"

if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
    echo "PaperView's download is for 64-bit Intel/AMD Linux (x86_64)." >&2
    echo "On other systems, build it from source: https://github.com/nagasab221/PaperView-Desktop" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

echo "Downloading PaperView…"
if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" | tar xz -C "$TMP"
elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$URL" | tar xz -C "$TMP"
else
    echo "Please install curl or wget first." >&2
    exit 1
fi

"$TMP/PaperView-linux-x86_64/install.sh"
