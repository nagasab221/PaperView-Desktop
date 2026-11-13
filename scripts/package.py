"""Build the portable PaperView folder + zip.

Produces:
  dist/PaperView/            paperview.exe + pdfium.dll + README.txt
  dist/PaperView-portable.zip

Run:  py scripts/package.py   (from the repo root)
"""

import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

PORTABLE_README = """PaperView — portable
====================

A lightweight, fast PDF viewer. Keep paperview.exe and pdfium.dll together;
run paperview.exe. No installation required.

Optional: use the toolbar's "..." menu -> "Register as PDF viewer" to appear
in Windows' "Open with" list for .pdf files (current user only; the same menu
can unregister). Re-run it if you move this folder.

Settings live in %APPDATA%\\PaperView; cloud downloads cache in
%LOCALAPPDATA%\\PaperView\\cloud.
"""


def main() -> None:
    cargo = shutil.which("cargo") or str(
        Path.home() / ".cargo" / "bin" / "cargo.exe"
    )
    print("building release…")
    subprocess.check_call([cargo, "build", "--release"], cwd=ROOT)

    exe = ROOT / "target" / "release" / "paperview.exe"
    dll = ROOT / "pdfium.dll"
    if not dll.exists():
        dll = ROOT / "target" / "release" / "pdfium.dll"
    if not exe.exists() or not dll.exists():
        sys.exit("missing paperview.exe or pdfium.dll")

    dist = ROOT / "dist"
    folder = dist / "PaperView"
    if folder.exists():
        shutil.rmtree(folder)
    folder.mkdir(parents=True)
    shutil.copy2(exe, folder / "paperview.exe")
    shutil.copy2(dll, folder / "pdfium.dll")
    (folder / "README.txt").write_text(PORTABLE_README, encoding="utf-8")

    zip_path = dist / "PaperView-portable.zip"
    if zip_path.exists():
        zip_path.unlink()
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
        for p in sorted(folder.iterdir()):
            z.write(p, f"PaperView/{p.name}")

    total = sum(p.stat().st_size for p in folder.iterdir())
    print(f"portable folder: {folder}  ({total / 1024 / 1024:.1f} MB)")
    print(f"zip:             {zip_path}  ({zip_path.stat().st_size / 1024 / 1024:.1f} MB)")


if __name__ == "__main__":
    main()
