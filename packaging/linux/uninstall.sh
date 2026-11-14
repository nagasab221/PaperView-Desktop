#!/bin/sh
# Removes PaperView. Your library, settings and reading positions stay in
# ~/.local/share/paperview; delete that folder too to forget everything.
set -eu
DEST="$HOME/.local/opt/paperview"
[ -x "$DEST/paperview" ] && "$DEST/paperview" --unregister || true
rm -rf "$DEST" "$HOME/.local/bin/paperview"
echo "PaperView is uninstalled."
