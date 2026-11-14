#!/bin/sh
# Installs PaperView for you alone (no root needed):
#   ~/.local/opt/paperview/   the program and its PDF engine (PDFium)
#   ~/.local/bin/paperview    so `paperview file.pdf` works in a terminal
# and adds it to your app menu and to "Open with" for PDFs and ebooks.
# Run it again from a newer download to update.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
DEST="$HOME/.local/opt/paperview"
mkdir -p "$DEST" "$HOME/.local/bin"
install -m 755 "$HERE/paperview" "$DEST/paperview"
install -m 644 "$HERE/libpdfium.so" "$DEST/libpdfium.so"
install -m 755 "$HERE/uninstall.sh" "$DEST/uninstall.sh"
ln -sf "$DEST/paperview" "$HOME/.local/bin/paperview"
"$DEST/paperview" --register
echo "PaperView is installed. Open it from your app menu, or run: paperview"
