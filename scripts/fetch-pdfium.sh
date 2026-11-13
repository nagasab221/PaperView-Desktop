#!/bin/sh
# Downloads the prebuilt PDFium shared library (bblanchon/pdfium-binaries)
# into vendor/pdfium and links it next to the debug and release binaries,
# where PaperView looks for it first.
set -eu
cd "$(dirname "$0")/.."
TAG="${PDFIUM_TAG:-chromium/8152}"
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) ASSET=pdfium-linux-x64.tgz; LIB=lib/libpdfium.so ;;
    Linux-aarch64) ASSET=pdfium-linux-arm64.tgz; LIB=lib/libpdfium.so ;;
    *) echo "No prebuilt PDFium configured for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT
curl -fsSL -o "$TMP/pdfium.tgz" \
    "https://github.com/bblanchon/pdfium-binaries/releases/download/$(echo "$TAG" | sed 's|/|%2F|')/$ASSET"
tar xzf "$TMP/pdfium.tgz" -C "$TMP"
mkdir -p vendor/pdfium
cp "$TMP/$LIB" vendor/pdfium/
cp "$TMP/LICENSE" vendor/pdfium/LICENSE
for profile in debug release; do
    mkdir -p "target/$profile"
    ln -sf ../../vendor/pdfium/"$(basename "$LIB")" "target/$profile/$(basename "$LIB")"
done
echo "PDFium $TAG installed in vendor/pdfium"
