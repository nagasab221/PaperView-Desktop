# PaperView — Changes Log

## Bug Fixes

### `try_open` title dedup (`src/app.rs`)
The `path.file_name()` -> `String` conversion was computed twice in `try_open`:
once for the viewport title and again for the `Tab.title` field. Now the first
result (`name`) is reused for the tab title.

### `page_count()` underflow (`src/app.rs`)
`resolve_zoom` did `self.page_index.clamp(0, self.page_count() - 1)` without
checking for an empty document — if `page_count()` returned 0, the subtraction
underflowed to `i32::MAX`. Added an early-return guard (`if count == 0 { return
1.0; }`). Also changed `single_view`'s clamp to use `saturating_sub` for
defense-in-depth.

### `page_field` stale after navigation (`src/app.rs`)
The page-number text field only synced on `lost_focus()` or tab activation. Navigating
via keyboard arrows, links, search hits, or the TOC left the field showing the old
page number. Fixed by updating `self.page_field` inside `go_to_page()`.

### `open_file_location` platform guard (`src/app.rs`)
The function hardcoded `explorer.exe` with no `#[cfg]` guard — it would silently
do nothing (or panic) on non-Windows. Added `#[cfg(target_os = "windows")]` for
the real implementation and a no-op fallback for other platforms.

### TOC clone every frame (`src/app.rs` — `toc_panel`)
`let entries = self.toc.clone()` cloned the entire table of contents every frame
just to iterate it. Removed the clone; now iterates `&self.toc` directly. Click
actions are collected into a local `jump_to: Option<i32>` and applied after the
loop to avoid `&self` / `&mut self` borrow conflicts.

### Cloud files clone every frame (`src/app.rs` — `landing_saved`)
`let files = self.cloud_files.clone()` duplicated the whole cloud file list every
frame. Replaced with a lightweight `Vec<(name, size, local_path)>` collect that
only clones the three fields needed for rendering, letting `&mut self` methods
(open, download) be called freely inside the loop.

## New Features

### Reopen closed tab — `Ctrl+Shift+T` (`src/app.rs`)
Added a `closed_tabs: Vec<(PathBuf, Option<String>)>` field (most-recent-last,
capped at 32 entries). `close_tab` now pushes the closed tab's path and password
before removing it. A new `reopen_closed_tab` method pops the last entry and calls
`try_open`. Wired to `Ctrl+Shift+T` in `handle_input`.

### System theme option (`src/app.rs` — `tb_menu`)
The overflow menu previously toggled between Light and Dark. Now it offers three
enabled/disabled buttons: Light, Dark, and System (with a hover tooltip
"Follow the OS light/dark setting"). The currently active theme's button is
disabled to indicate selection.

### `Ctrl+Shift+S` for Save As (`src/app.rs` — `handle_input`)
Added `Ctrl+Shift+S` as an alternative shortcut alongside the existing `Ctrl+S`,
matching the standard convention for "Save As".

### Delete confirmation countdown (`src/app.rs` — `landing_saved`)
The "Confirm delete" button now shows the remaining seconds:
`"Confirm delete 3? (4s)"` instead of just `"Confirm delete 3?"`.

## Polish

### `cloud_stats_card` scrollable (`src/app.rs`)
Wrapped the entire stats card in `egui::ScrollArea::vertical()` so it doesn't
overflow on smaller windows. The `Frame::group` and `ScrollArea` closing braces
were adjusted and indentation corrected to match.

### `FREE_TIER_BYTES` documented (`src/app.rs`)
Changed the one-line comment to a multi-line doc comment explaining it is
Backblaze-B2-specific and how to change it for different bucket quotas.

### `.clippy.toml` (new file)
Created `.clippy.toml` with `too-many-arguments-threshold = 12` to match the
project's style (the `Keys` struct in `handle_input` has ~30 fields).

### README updates (`README.md`)
- Added `Ctrl+Shift+S` and `Ctrl+Shift+T` to the keyboard shortcuts table.
- Updated the dark mode description to mention Light / Dark / System options.

## Intentionally Skipped

### Module split of `app.rs`
The 3300+ line monolith would benefit from splitting into `toolbar.rs`,
`landing.rs`, `cloud_ui.rs`, etc., but extracting `impl ViewerApp` methods across
files while preserving borrow-checker invariants is high-risk for subtle
regressions. The targeted fixes above give ~90% of the value with near-zero risk.

## Review (Claude) — corrections to the above

Verified every change; most were sound and kept. Four issues fixed:

1. **`go_to_page` regression**: the rewrite dropped the `clamped !=
   page_index` guard, so navigating past the first/last page re-scrolled the
   current page to its top instead of doing nothing. Guard restored (the new
   `page_field` sync is kept, unconditionally).
2. **`closed_tabs` cap inverted**: `truncate(32)` keeps the *first* 32
   entries — i.e. the oldest closes — silently dropping new ones once full.
   Now drops from the front (`remove(0)`), keeping the most recent 32.
3. **Delete countdown didn't tick**: egui repaints only on input; the
   `(Ns)` label froze between events. While armed, a 200 ms
   `request_repaint_after` keeps it counting.
4. **Saved-tab rows**: the de-clone introduced a per-row
   `cloud_files.iter().find(...)` for the upload date — O(n²) per frame.
   The timestamp now rides in the row tuple. Also gave the new nested stats
   `ScrollArea` an `id_salt` to avoid nested-scroll id collisions.

All 18 tests pass; debug/release smokes clean after the corrections.
