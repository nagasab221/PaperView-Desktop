# PaperView

A lightweight, fast-starting PDF viewer for Windows, in the spirit of
[Sumatra PDF](https://www.sumatrapdfreader.org/): small binary, near-instant
startup, low memory. Built with Rust, [egui](https://github.com/emilk/egui)
(immediate-mode GUI, no bundled runtime) and Google's
[PDFium](https://pdfium.googlesource.com/pdfium/) for rendering.

**Viewer only** — no annotation, no form filling, no editing.

## Status

Phase 1 (MVP) + Phase 2 — done:

- Open a PDF via file dialog, drag-and-drop, or command line (`paperview.exe file.pdf`)
- **Tabs**: every file opens in its own tab (browser-style strip with a Home
  pill; click to switch, `×`/middle-click/`Ctrl+W` to close, `Ctrl+Tab` /
  `Ctrl+Shift+Tab` to cycle). Open tabs are restored — parked behind the
  landing page — on the next launch.
- **Tiered toolbar**: clusters fold onto extra rows at fixed widths (no
  overlap, no mid-word wrapping); infrequent actions (rotate, save as, file
  location, cloud save, theme, close/home) live in the `⋯` menu
- Crisp page rendering (hi-dpi aware); zoom anchors on the mouse cursor
- **Continuous scroll** (default): pages flow into each other, Sumatra-style —
  the wheel carries you straight into the next page
- **Single-page mode**: wheeling past the bottom/top edge flips the page
- Page navigation: buttons, jump-to-page box, thumbnails, **table of
  contents sidebar** (document outline, TOC button), `PgUp`/`PgDn`,
  `←`/`→`, `Home`/`End`, `↑`/`↓` line scrolling, `Space`/`Shift+Space`
  screenfuls (flips the page at the edges in single-page mode)
- **Clickable links**: internal jumps and external URLs (pointer cursor +
  URL tooltip; `Alt+←`/`Alt+→` walk the jump history)
- **Password-protected PDFs**: a prompt appears for encrypted files
- **Per-file memory**: every file reopens at the page/zoom/rotation you left
  it at (Sumatra-style)
- **Annotations**: existing highlight/note/form appearances in the PDF are
  rendered
- Zoom: `Ctrl` `+`/`-`/`0`, `Ctrl`+scroll wheel, fit-to-width and fit-to-page modes
- **Thumbnail sidebar** (☰) with click-to-jump and current-page highlight
- **Text selection & copy**: click-drag to select (across pages in continuous
  mode), double-click selects a word, `Ctrl+A` selects everything, `Ctrl+C` or
  right-click → Copy puts it on the clipboard, `Esc` clears. Coordinates
  respect each page's CropBox, so print-layout PDFs highlight correctly.
- **Rotate** (toolbar `Rotate L` / `Rotate R`): turns the whole document in
  90° steps — selection, search highlights, and thumbnails all follow
- **Save as** (`Ctrl+S` or right-click): copies the open PDF anywhere — handy
  for files fetched from the cloud cache
- **Printing** (`Print…` toolbar button or `Ctrl+P`): the system print dialog
  (printer choice, page range, copies via the driver), then an **accurate
  in-app print preview** built from the chosen printer's real paper metrics —
  page through the range on a paper mock-up before committing. Pages are
  rasterized by PDFium at printer resolution (capped at 300 dpi), auto-rotated
  to the paper orientation, and printed on a background thread with progress
  in the status bar. (Windows 11's own dialog preview pane only supports
  WinRT-pipeline apps, hence the in-app preview.)
- **Open file location** (right-click on the page or on a Recent entry):
  reveals the file in Explorer
- **Text search** (`Ctrl+F`): case-insensitive, whole document, incremental
  scan (no freeze on huge files), yellow highlights + orange active match,
  `F3`/`Shift+F3` cycling
- **Dark mode** toggle (☀/🌙) with Light / Dark / System options — follows
  the system theme until you choose
- **Session persistence**: window size/position, view mode, zoom, theme,
  sidebar state, recents list, cloud credentials
- **Landing page** on every plain launch (opening a PDF directly skips it):
  a **Recent** tab with your last 20 files (click to reopen, ✕ to remove) and
  a **Saved ☁** tab backed by Backblaze B2 — the `Home` toolbar button
  returns to it
- **Auto-save to cloud**: optionally (on by default, toggle in the Saved
  tab's stats sidebar) every PDF you open is uploaded automatically when it's
  at or under a size limit (default 20 MB) — already-saved files and files
  opened from the cloud cache are skipped
- **Save to cloud**: `☁ Save` uploads the open PDF to a private
  [Backblaze B2](https://www.backblaze.com/cloud-storage) bucket (10 GB free);
  the Saved tab lists, downloads, and opens them on any launch. A stats
  sidebar shows the connected bucket, storage used/left (against the 10 GB
  free tier), file count, largest file, local cache size, live download
  progress with speed, and last measured transfer speeds. Drop PDFs (several
  at once) onto the Saved tab to queue uploads; row checkboxes enable batch
  Download and batch Delete (two-step confirm — deletes are permanent). Setup: create
  a bucket + application key on Backblaze, paste Key ID / application key /
  bucket name into the Saved tab, Connect. Credentials are stored **locally in
  plain text** in PaperView's settings file; downloads cache under
  `%LOCALAPPDATA%\PaperView\cloud`.
- **Crisp rendering at any zoom**: pages are painted on the physical-pixel
  grid at the texture's exact size — no sub-pixel resampling blur at odd
  zoom levels like 88 %

- **PDF file association**: `⋯ → Register as PDF viewer` (or
  `paperview.exe --register`) adds PaperView to Windows' "Open with" list for
  `.pdf` — per-user (HKCU), no admin, undo via the same menu or
  `--unregister`. Pick it once with "Always" to make it the default; re-run
  registration if you move the exe.
- **Portable packaging**: `py scripts/package.py` builds
  `dist/PaperView-portable.zip` (exe + pdfium.dll + notes, ~7 MB zipped) —
  unzip anywhere and run; settings stay in `%APPDATA%\PaperView`.

All three phases of the original roadmap are complete. (A classic installer
remains optional — the portable zip plus in-app registration covers the
workflow.)

## Running it

PaperView needs `pdfium.dll` next to `paperview.exe` (it is **not** bundled):

1. Download `pdfium-win-x64.tgz` from the latest
   [bblanchon/pdfium-binaries release](https://github.com/bblanchon/pdfium-binaries/releases)
   (direct link: <https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-win-x64.tgz>).
2. Extract it (`tar -xzf pdfium-win-x64.tgz`) and copy `bin\pdfium.dll` next to
   `paperview.exe`.

For development, a copy of `pdfium.dll` in the repository root (git-ignored)
also works — the app looks next to the executable first, then in the working
directory, then on the system path. `cargo run` and `cargo test` both rely on
the repo-root copy.

This project was built and tested against PDFium `chromium/7961`
(v152.0.7961.0) with `pdfium-render`'s `pdfium_latest` bindings. If a future
DLL drops symbols the bindings expect, binding fails at startup with a clear
error — pin the DLL to a matching release in that case.

## Keyboard shortcuts

| Keys                | Action                          |
| ------------------- | ------------------------------- |
| `Ctrl+O`            | Open file                       |
| `PgDn` / `→`        | Next page                       |
| `PgUp` / `←`        | Previous page                   |
| `Home` / `End`      | First / last page               |
| `Ctrl` `+` / `-`    | Zoom in / out                   |
| `Ctrl+0`            | Zoom to 100 %                   |
| `Ctrl`+scroll wheel | Zoom (anchored on the cursor)   |
| `Ctrl+F`            | Focus search box                |
| `Enter` (in search) | Search / next match             |
| `F3` / `Shift+F3`   | Next / previous match           |
| `Ctrl+C`            | Copy selected text              |
| `Ctrl+A`            | Select all text                 |
| `Ctrl+S`            | Save a copy (Save as)           |
| `Ctrl+Shift+S`      | Save a copy (Save as)           |
| `Ctrl+P`            | Print                           |
| `Ctrl+W`            | Close tab                       |
| `Ctrl+Shift+T`      | Reopen last closed tab          |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab   |
| `↑` / `↓`           | Scroll by lines                 |
| `Space` / `Shift+Space` | Scroll a screenful (page-flips at edges in single mode) |
| `Alt+←` / `Alt+→`   | Back / forward in jump history  |
| `Esc`               | Clear selection, then search    |

Drag a `.pdf` onto the window to open it. In single-page mode, scrolling past
the bottom of a page flips to the next one (and past the top, back).

## App icon

The icon (page + fold + PDF badge in the app's blue) is generated by
`py scripts/make_icon.py` (Pillow) into `assets/`: a multi-size
`paperview.ico` embedded into the exe at link time (`build.rs` +
`windres` from `~/.cargo/gnu-binutils`, using `cat` as the preprocessor since
the .rc has no directives), and `icon_128.rgba` embedded as the runtime
window/taskbar icon.

## Building from source

```
cargo build --release
```

The binary lands in `target\release\paperview.exe`; put `pdfium.dll` next to it
(the release profile uses fat LTO + strip for a small binary).

### Notes for the GNU toolchain (no Visual Studio required)

This project builds on `stable-x86_64-pc-windows-gnu`, which needs no Visual
Studio Build Tools. Two machine-setup wrinkles were solved via
`.cargo/config.toml` (MSVC builds ignore all of it):

1. **dlltool needs an assembler.** The `windows-*` crates use `raw-dylib`
   linking, for which rustc invokes `dlltool.exe`. Rustup bundles a GNU
   dlltool but *not* the GNU assembler (`as.exe`) it silently spawns — without
   binutils installed it fails with a bare `CreateProcess` error. Fix:
   `~/.cargo/gnu-binutils/` holds `dlltool.exe` + `as.exe` (plus their four
   runtime DLLs: libintl, libiconv, zlib, zstd) extracted from the MSYS2
   `mingw-w64-x86_64-{binutils,gettext-runtime,libiconv,zlib,zstd}` packages —
   no MSYS2 installation, just the files. `-C dlltool=` points there.

   *Do not* substitute `llvm-dlltool` (tempting, since rustup's `llvm-tools`
   component ships it inside `llvm-ar`): its short-import objects mixed with
   MinGW-style import libraries make GNU ld emit a corrupt import-descriptor
   table, and depending on link layout the binaries **segfault before
   `main()`** (the loader stops fixing up IAT slots mid-table). Linking with
   LLD instead of GNU ld reshuffles but does not fix this. All-GNU import
   objects are the reliable configuration.

2. **A missing import library.** Rustup's self-contained MinGW ships a
   curated subset of Windows import libraries; `shlwapi` (linked by the
   `webbrowser` crate) is not in it. `~/.cargo/gnu-import-libs/libshlwapi.a`
   was generated from the system DLL's export table with
   [scripts/gen_implib.py](scripts/gen_implib.py) and is added via `-L`:

   ```
   py scripts/gen_implib.py shlwapi %USERPROFILE%\.cargo\gnu-import-libs %USERPROFILE%\.cargo\gnu-binutils\dlltool.exe
   ```

### Dependency footprint

- `pdfium-render` is built with `default-features = false` — its default
  `image_latest` feature would pull the `image` crate's full codec stack
  (including AVIF). PDFium renders straight into an RGBA buffer that egui
  uploads as a texture; no image crate needed.
- `eframe` is built with the `glow` (OpenGL) backend instead of the default
  `wgpu` — smaller and faster to start for a 2-D app.
- A png-only copy of `image` still rides along via eframe/arboard (window
  icons, clipboard) — that's upstream's call, and it does not include the
  heavyweight codecs.
- `accesskit` (screen-reader support) is enabled.

## Tests

```
cargo test
```

`tests/render_smoke.rs` binds PDFium headlessly, opens
`tests/fixtures/sample.pdf` (3 pages, generated by
`tests/fixtures/make_fixture.py`), renders pages, and checks pixel statistics —
including a red-vs-blue check that would catch BGRA/RGBA channel swaps.

## Licenses of bundled/linked components

- PDFium: BSD-3-Clause (Google); `pdfium-render`: MIT/Apache-2.0;
  egui/eframe: MIT/Apache-2.0; rfd: MIT.
