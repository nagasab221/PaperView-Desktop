#!/bin/sh
# Installs PaperView for the current user (no root needed):
#   ~/.local/opt/paperview/   the program and its PDFium library
#   ~/.local/bin/paperview    a link, so `paperview file.pdf` works in a terminal
# and adds PaperView to the app menu and the file manager's "Open with" list.
# Run it again after building a new version to update. Undo with:
#   paperview --unregister && rm -rf ~/.local/opt/paperview ~/.local/bin/paperview
set -eu
cd "$(dirname "$0")/.."
BIN=target/release/paperview
LIB=vendor/pdfium/libpdfium.so
[ -x "$BIN" ] || { echo "Build it first: cargo build --release" >&2; exit 1; }
[ -f "$LIB" ] || { echo "Fetch PDFium first: scripts/fetch-pdfium.sh" >&2; exit 1; }
DEST="$HOME/.local/opt/paperview"
mkdir -p "$DEST" "$HOME/.local/bin"
install -m 755 "$BIN" "$DEST/paperview"
install -m 644 "$LIB" "$DEST/libpdfium.so"
ln -sf "$DEST/paperview" "$HOME/.local/bin/paperview"
"$DEST/paperview" --register
echo "PaperView installed. Find it in your app menu, or run: paperview"
