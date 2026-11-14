#!/bin/sh
# Builds the Linux download: dist/PaperView-linux-x86_64.tar.gz, a folder with
# the program, PDFium, an installer and the licences. The file name has no
# version so ".../releases/latest/download/PaperView-linux-x86_64.tar.gz"
# always points at the newest release.
set -eu
cd "$(dirname "$0")/.."
NAME=PaperView-linux-x86_64
OUT="dist/$NAME"
[ -f vendor/pdfium/libpdfium.so ] || scripts/fetch-pdfium.sh
cargo build --release --locked
rm -rf dist
mkdir -p "$OUT/licenses"
install -m 755 target/release/paperview "$OUT/paperview"
install -m 644 vendor/pdfium/libpdfium.so "$OUT/libpdfium.so"
install -m 755 packaging/linux/install.sh packaging/linux/uninstall.sh "$OUT/"
install -m 644 packaging/linux/README.txt "$OUT/README.txt"
cp LICENSE "$OUT/licenses/PaperView.txt"
cp vendor/pdfium/LICENSE "$OUT/licenses/PDFium.txt"
cp assets/fonts/OFL.txt "$OUT/licenses/Noto-fonts-OFL.txt"
for f in assets/fonts/reading/OFL-*.txt; do
    cp "$f" "$OUT/licenses/$(basename "$f" .txt | sed 's/^OFL-//')-OFL.txt"
done
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
echo "Built dist/$NAME.tar.gz ($(du -h "dist/$NAME.tar.gz" | cut -f1))"
