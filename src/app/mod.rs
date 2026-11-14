//! The egui application: toolbar, page views, thumbnails, search, input.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;

use eframe::egui::scroll_area::ScrollSource;

use crate::cloud::{self, B2Config, CloudEvent, CloudFile, CloudOp};
use crate::covers::Covers;
use crate::ui::theme::{self, Palette, Skin, ThemeChoice};
use crate::ui::widgets as w;

#[cfg(debug_assertions)]
mod automate;
mod books;
mod chrome;
mod density;
mod folders;
mod library;
mod peek;
mod reader;
mod settings;
mod typography;
use crate::pdf::{Document, LinkTarget, OpenError, PdfEngine, TocEntry};
use crate::print::{self, PrintEvent};
use crate::search::SearchState;
use crate::view::{self, Layout, TexCache, MAX_TEXTURE_DIM, PAGE_GAP, PX_PER_PT};

const MIN_ZOOM: f32 = 0.10;
const MAX_ZOOM: f32 = 6.0;
/// Thumbnail width in logical px (sidebar).
const THUMB_W: f32 = 118.0;
/// Minimum interval between wheel-driven page flips in single-page mode.
const EDGE_FLIP_COOLDOWN: Duration = Duration::from_millis(300);
/// How long after the last zoom change we keep stretching stale textures
/// before re-rendering at the final resolution.
const ZOOM_SETTLE: Duration = Duration::from_millis(140);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Zoom {
    FitWidth,
    FitPage,
    Custom(f32),
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    Single,
    Continuous,
}

/// What the home view shows when no document is active.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum HomeScreen {
    Library,
    Settings,
}

#[derive(Clone, Copy)]
enum ScrollTarget {
    PageTop(i32),
    PageBottom(i32),
    /// (page, hit index on that page)
    Hit(i32, usize),
    /// Absolute scroll offset (tab-switch restore).
    Offset(egui::Vec2),
    /// A peek spot: a point a fraction of the way down a page.
    Spot {
        page: i32,
        y_frac: f32,
        x_frac: f32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum LandingTab {
    Recent,
    /// Books the reader added (only shown once there are some).
    Books,
    /// Folders the reader added.
    Folders,
    /// The Backblaze bucket.
    Saved,
}

/// One open document's complete state, parked while its tab is inactive.
struct DocTab {
    doc: Document,
    page_sizes: Arc<Vec<(f32, f32)>>,
    page_index: i32,
    zoom: Zoom,
    effective_zoom: f32,
    rotation: u8,
    search: SearchState,
    selection: Option<((i32, i32), (i32, i32))>,
    text_counts: std::collections::HashMap<i32, i32>,
    toc: Vec<TocEntry>,
    back_stack: Vec<i32>,
    fwd_stack: Vec<i32>,
    doc_password: Option<String>,
    saved_offset: egui::Vec2,
}

/// A tab in the strip. The active tab's state lives inline in `ViewerApp`
/// (its `state` here is `None`); inactive tabs park it in `state`.
struct Tab {
    title: String,
    path: PathBuf,
    state: Option<Box<DocTab>>,
    /// A book's reader, parked while its tab is inactive.
    book: Option<Box<reader::BookView>>,
}

/// Same file? Windows paths ignore case; everywhere else two paths that
/// differ only in case are different files.
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

fn truncate_title(s: &str) -> String {
    const MAX: usize = 22;
    let mut out: String = s.chars().take(MAX).collect();
    if s.chars().count() > MAX {
        out.push('…');
    }
    out
}

fn fix_active_after_remove(active: Option<usize>, removed: usize) -> Option<usize> {
    match active {
        Some(a) if a > removed => Some(a - 1),
        Some(a) if a == removed => None,
        other => other,
    }
}

/// A print job between the system dialog and the user's confirmation.
struct PrintPreviewState {
    ticket: Option<print::PrintTicket>,
    path: PathBuf,
    doc_name: String,
    /// Currently previewed page (within ticket.from..=ticket.to).
    page: i32,
    tex: Option<egui::TextureHandle>,
    tex_page: i32,
}

#[derive(Clone, Copy, PartialEq)]
enum SidebarMode {
    Hidden,
    Thumbs,
    Toc,
}

/// One recently-opened file, with its remembered reading state.
#[derive(Clone)]
struct Recent {
    path: PathBuf,
    /// Unix seconds of the last open.
    ts: i64,
    page: i32,
    zoom: Zoom,
    rotation: u8,
}

const MAX_RECENTS: usize = 20;

fn zoom_to_str(z: Zoom) -> String {
    match z {
        Zoom::FitWidth => "fitw".to_string(),
        Zoom::FitPage => "fitp".to_string(),
        Zoom::Custom(v) => format!("custom:{v}"),
    }
}

fn zoom_from_str(s: &str) -> Zoom {
    match s {
        "fitw" => Zoom::FitWidth,
        "fitp" => Zoom::FitPage,
        other => other
            .strip_prefix("custom:")
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| Zoom::Custom(v.clamp(MIN_ZOOM, MAX_ZOOM)))
            .unwrap_or(Zoom::FitPage),
    }
}

fn encode_recents(list: &[Recent]) -> String {
    list.iter()
        .map(|r| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                r.ts,
                r.page,
                zoom_to_str(r.zoom),
                r.rotation,
                r.path.display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_recents(s: &str) -> Vec<Recent> {
    s.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.splitn(5, '\t').collect();
            match fields.as_slice() {
                // v2: ts, page, zoom, rotation, path
                [ts, page, zoom, rot, path] if !path.is_empty() => Some(Recent {
                    path: PathBuf::from(path),
                    ts: ts.parse().ok()?,
                    page: page.parse().unwrap_or(0),
                    zoom: zoom_from_str(zoom),
                    rotation: rot.parse().unwrap_or(0),
                }),
                // v1: ts, path
                [ts, path] if !path.is_empty() => Some(Recent {
                    path: PathBuf::from(path),
                    ts: ts.parse().ok()?,
                    page: 0,
                    zoom: Zoom::FitPage,
                    rotation: 0,
                }),
                _ => None,
            }
        })
        .collect()
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn fmt_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn fmt_time(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn fmt_speed(bps: f64) -> String {
    format!("{}/s", cloud::fmt_size(bps.max(0.0) as u64))
}

/// Backblaze B2's free-tier allowance (10 GB), used for the "space left"
/// stat on the Saved tab.  If you use a different bucket quota, change
/// this constant.
const FREE_TIER_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Auto-save decision: at/below the limit, not from the cloud cache, and not
/// already saved with the same name+size.
fn should_auto_save(limit_mb: u32, size: u64, from_cache: bool, already: bool) -> bool {
    !from_cache && !already && size <= (limit_mb as u64) * 1024 * 1024
}

/// Maps a rect from page space (unrotated, top-left origin, points) into view
/// space after `rot` quarter-turns clockwise. (w, h) is the unrotated page.
fn rect_to_view(hr: crate::pdf::HitRect, rot: u8, w: f32, h: f32) -> crate::pdf::HitRect {
    use crate::pdf::HitRect as R;
    match rot % 4 {
        1 => R {
            x: h - hr.y - hr.h,
            y: hr.x,
            w: hr.h,
            h: hr.w,
        },
        2 => R {
            x: w - hr.x - hr.w,
            y: h - hr.y - hr.h,
            w: hr.w,
            h: hr.h,
        },
        3 => R {
            x: hr.y,
            y: w - hr.x - hr.w,
            w: hr.h,
            h: hr.w,
        },
        _ => hr,
    }
}

/// Maps a point from view space back into page space (inverse of the above).
fn point_to_page(v: egui::Vec2, rot: u8, w: f32, h: f32) -> egui::Vec2 {
    match rot % 4 {
        1 => egui::vec2(v.y, h - v.x),
        2 => egui::vec2(w - v.x, h - v.y),
        3 => egui::vec2(w - v.y, v.x),
        _ => v,
    }
}

/// Reveals a file in Explorer (Windows only).
#[cfg(target_os = "windows")]
fn open_file_location(path: &Path) {
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

/// Linux: asks the file manager to show the file selected (the freedesktop
/// FileManager1 interface — Dolphin, Nautilus, Nemo…), falling back to just
/// opening its folder.
#[cfg(not(target_os = "windows"))]
fn open_file_location(path: &Path) {
    let uri = file_uri(path);
    let shown = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{uri}"))
        .arg("string:")
        .status()
        .is_ok_and(|s| s.success());
    if !shown {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

/// `file://` URI for a path, percent-encoding anything outside the unreserved set.
#[cfg(not(target_os = "windows"))]
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub struct ViewerApp {
    engine: Option<PdfEngine>,
    engine_error: Option<String>,
    doc: Option<Document>,
    /// Copy of the per-page sizes, so layout math never borrows `doc`.
    page_sizes: Arc<Vec<(f32, f32)>>,
    page_index: i32,
    zoom: Zoom,
    /// Zoom actually applied last frame (fit modes resolve to a number here).
    effective_zoom: f32,
    mode: ViewMode,
    cache: TexCache,
    thumb_cache: TexCache,
    sidebar: SidebarMode,
    /// Document outline, flattened.
    toc: Vec<TocEntry>,
    /// Jump history for Alt+Left / Alt+Right.
    back_stack: Vec<i32>,
    fwd_stack: Vec<i32>,
    /// Keyboard scroll to apply this frame (logical px, + = down).
    pending_scroll: f32,
    /// Whether the single-page view sat at its bottom/top edge last frame.
    last_at_bottom: bool,
    last_at_top: bool,
    /// Password dialog state: (path, entered text, previous attempt failed).
    password_prompt: Option<(PathBuf, String, bool)>,
    /// Last link probe under the pointer: (position, page, target).
    last_link_probe: Option<(egui::Pos2, Option<LinkTarget>)>,
    /// Password the open document was unlocked with (for reopen/print).
    doc_password: Option<String>,
    /// Main window handle (Win32), for parenting the print dialog.
    hwnd: isize,
    printing: bool,
    print_rx: Option<std::sync::mpsc::Receiver<PrintEvent>>,
    /// Pending print job awaiting preview confirmation.
    print_preview: Option<PrintPreviewState>,
    /// Open documents; the active one's state lives inline.
    tabs: Vec<Tab>,
    active_tab: Option<usize>,
    /// Auto-save opened PDFs to the cloud when at/below the size limit.
    auto_save: bool,
    auto_save_mb: u32,
    theme: ThemeChoice,
    search: SearchState,
    focus_search: bool,
    page_field: String,
    status: Option<String>,
    scroll_to: Option<ScrollTarget>,
    /// Scroll offset of the page view last frame (for zoom anchoring).
    last_offset: egui::Vec2,
    /// The view's size last frame and the spot at its top edge (page, how far
    /// down it, how far across the content): a resize — full screen, a
    /// maximise, a dragged edge — scrolls back to the same spot.
    resize_anchor: Option<(egui::Vec2, ScrollTarget)>,
    /// Screen rect of the page view last frame (for pointer anchoring).
    central_rect: egui::Rect,
    /// Effective zoom before a pending zoom change, for anchor math.
    zoom_prev_effective: Option<f32>,
    /// Pointer position (relative to the view) a zoom change centers on.
    zoom_anchor: Option<egui::Vec2>,
    /// While in the future, zoom is "in motion": stretch stale textures.
    zoom_gesture_until: Option<Instant>,
    /// Raw (non-ctrl) wheel delta this frame, for edge page-flips.
    wheel_y: f32,
    last_edge_flip: Option<Instant>,
    /// Text selection: ((page, char), (page, char)) — anchor and head, in
    /// gesture order (normalize with `sel_ordered`).
    selection: Option<((i32, i32), (i32, i32))>,
    /// True while a selection drag is in progress.
    selecting: bool,
    /// Cached highlight rects per page for the *current* selection.
    sel_rects: std::collections::HashMap<i32, Vec<crate::pdf::HitRect>>,
    /// Cached char count per page.
    text_counts: std::collections::HashMap<i32, i32>,
    /// Last pointer hit-test result, to avoid re-querying PDFium every frame.
    last_hit_test: Option<(egui::Pos2, (i32, i32))>,
    // Landing page
    recents: Vec<Recent>,
    landing_tab: LandingTab,
    // Cloud (Backblaze B2)
    cloud_cfg: B2Config,
    cloud_files: Vec<CloudFile>,
    cloud_status: Option<String>,
    cloud_busy: bool,
    cloud_listed: bool,
    cloud_rx: Option<std::sync::mpsc::Receiver<CloudEvent>>,
    /// Live transfer progress: (done, total, bytes/sec).
    cloud_progress: Option<(u64, u64, f64)>,
    /// Last measured speeds: (bytes/sec, unix ts).
    cloud_last_dl: Option<(f64, i64)>,
    cloud_last_ul: Option<(f64, i64)>,
    /// When the file list was last refreshed (unix ts).
    cloud_refreshed: Option<i64>,
    /// Local download-cache footprint: (files, bytes).
    cloud_cache: (usize, u64),
    /// Queued cloud ops (one runs at a time).
    cloud_queue: VecDeque<CloudOp>,
    /// Names selected in the Saved tab for batch actions.
    cloud_selected: HashSet<String>,
    /// Two-step delete confirmation deadline.
    /// An open rename / delete confirmation on the Cloud page.
    cloud_dialog: Option<library::CloudDialog>,
    /// View rotation for the whole document, quarter turns clockwise.
    rotation: u8,
    /// Recently closed tabs, most recent last: (path, password).
    closed_tabs: Vec<(PathBuf, Option<String>)>,
    /// Look: skin, and the palette last applied to egui.
    skin: Skin,
    applied: Option<Palette>,
    home: HomeScreen,
    /// The user's switch for the Cloud tab (credentials are needed as well).
    cloud_enabled: bool,
    /// Credentials being edited in Settings, applied on "Save and connect".
    b2_draft: B2Config,
    show_app_key: bool,
    /// Documents downloaded from the bucket, read from disk.
    cloud_local: Vec<library::LocalDoc>,
    covers: Covers,
    peek: peek::Peek,
    /// Q went down together with Shift (that was Shift+Q): ignore it until released.
    q_suppressed: bool,
    search_open: bool,
    /// Folders added to the Library, and what was last read from them.
    folders: Vec<PathBuf>,
    folder_listings: Vec<folders::Listing>,
    folders_scanned: Option<Instant>,
    folder_scan: Option<std::sync::mpsc::Receiver<Vec<folders::Listing>>>,
    /// Interface size (egui zoom) and the value the slider is showing.
    ui_scale: f32,
    ui_scale_draft: f32,
    library_layout: density::LibraryLayout,
    preview: Option<density::Preview>,
    /// The open book, when the active tab is one (its state lives inline,
    /// like a PDF's).
    book: Option<reader::BookView>,
    /// Books on the Library's Books tab.
    books: Vec<books::BookEntry>,
    book_spread: bool,
    /// How books are typeset: face, size, spacing, margins, paragraphs.
    reading: reader::ReadingStyle,
    /// The book bar's page box while it's being edited.
    book_page_field: String,
    /// Full screen with nothing but the pages (F11).
    immersive: bool,
    /// When the pointer last moved: the full-screen controls show for a
    /// moment after.
    immersive_stir: Option<Instant>,
    /// The first-launch "make PaperView the default?" question was answered.
    default_asked: bool,
    /// Whether PaperView is the default PDF app, checked when Settings opens.
    default_status: Option<bool>,
    /// Where the active tab's card meets the rule under the tab strip.
    active_tab_span: Option<egui::Rangef>,
    /// The sidebar's last open mode, shown while it slides shut.
    sidebar_last: SidebarMode,
    /// A transient message, bottom centre.
    toast: Option<(String, Instant)>,
    #[cfg(debug_assertions)]
    announced_first_frame: bool,
    #[cfg(debug_assertions)]
    automation: Option<automate::Script>,
    /// Tracks whether we've shown the window yet (avoids startup flash).
    shown: bool,
    /// Frame counter used to defer Visible(true) until the second frame.
    frame_nr: u8,
}

impl ViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        // Ctrl+= / Ctrl+- should zoom the *document*, not the whole UI.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let (engine, engine_error) = match PdfEngine::new() {
            Ok(engine) => (Some(engine), None),
            Err(msg) => (None, Some(msg)),
        };

        let mut app = Self {
            engine,
            engine_error,
            doc: None,
            page_sizes: Arc::new(Vec::new()),
            page_index: 0,
            zoom: Zoom::FitPage,
            effective_zoom: 1.0,
            mode: ViewMode::Continuous,
            cache: TexCache::default(),
            thumb_cache: TexCache::default(),
            sidebar: SidebarMode::Hidden,
            toc: Vec::new(),
            back_stack: Vec::new(),
            fwd_stack: Vec::new(),
            pending_scroll: 0.0,
            last_at_bottom: false,
            last_at_top: false,
            password_prompt: None,
            last_link_probe: None,
            doc_password: None,
            hwnd: 0,
            printing: false,
            print_rx: None,
            print_preview: None,
            tabs: Vec::new(),
            active_tab: None,
            auto_save: false,
            auto_save_mb: 20,
            theme: ThemeChoice::System,
            search: SearchState::default(),
            focus_search: false,
            page_field: String::new(),
            status: None,
            scroll_to: None,
            last_offset: egui::Vec2::ZERO,
            resize_anchor: None,
            central_rect: egui::Rect::NOTHING,
            zoom_prev_effective: None,
            zoom_anchor: None,
            zoom_gesture_until: None,
            wheel_y: 0.0,
            last_edge_flip: None,
            selection: None,
            selecting: false,
            sel_rects: std::collections::HashMap::new(),
            text_counts: std::collections::HashMap::new(),
            last_hit_test: None,
            recents: Vec::new(),
            landing_tab: LandingTab::Recent,
            cloud_cfg: B2Config::default(),
            cloud_files: Vec::new(),
            cloud_status: None,
            cloud_busy: false,
            cloud_listed: false,
            cloud_rx: None,
            cloud_progress: None,
            cloud_last_dl: None,
            cloud_last_ul: None,
            cloud_refreshed: None,
            cloud_cache: (0, 0),
            cloud_queue: VecDeque::new(),
            cloud_selected: HashSet::new(),
            cloud_dialog: None,
            rotation: 0,
            closed_tabs: Vec::new(),
            skin: Skin::Paper,
            applied: None,
            home: HomeScreen::Library,
            cloud_enabled: true,
            b2_draft: B2Config::default(),
            show_app_key: false,
            cloud_local: Vec::new(),
            covers: Covers::new(engine),
            peek: peek::Peek::default(),
            q_suppressed: false,
            search_open: false,
            folders: Vec::new(),
            folder_listings: Vec::new(),
            folders_scanned: None,
            folder_scan: None,
            sidebar_last: SidebarMode::Thumbs,
            active_tab_span: None,
            book: None,
            books: Vec::new(),
            book_spread: true,
            reading: reader::ReadingStyle::default(),
            book_page_field: String::new(),
            immersive: false,
            immersive_stir: None,
            default_asked: false,
            default_status: None,
            ui_scale: 1.0,
            ui_scale_draft: 1.0,
            library_layout: density::LibraryLayout::Covers,
            preview: None,
            toast: None,
            #[cfg(debug_assertions)]
            announced_first_frame: false,
            #[cfg(debug_assertions)]
            automation: automate::Script::from_env(),
            shown: false,
            frame_nr: 0,
        };

        // Restore session preferences.
        let mut last_file = None;
        if let Some(storage) = cc.storage {
            if let Some(v) = storage.get_string("mode") {
                app.mode = if v == "single" {
                    ViewMode::Single
                } else {
                    ViewMode::Continuous
                };
            }
            app.sidebar = match storage.get_string("sidebar").as_deref() {
                Some("thumbs") => SidebarMode::Thumbs,
                Some("toc") => SidebarMode::Toc,
                Some("hidden") => SidebarMode::Hidden,
                // migrate the old boolean key
                _ => {
                    if storage.get_string("thumbs").as_deref() == Some("1") {
                        SidebarMode::Thumbs
                    } else {
                        SidebarMode::Hidden
                    }
                }
            };
            app.theme = ThemeChoice::from_key(&storage.get_string("theme").unwrap_or_default());
            app.skin = Skin::from_key(&storage.get_string("skin").unwrap_or_default());
            app.cloud_enabled = storage.get_string("cloud_enabled").as_deref() != Some("0");
            if let Some(raw) = storage.get_string("peek_spots") {
                app.peek.spots = peek::decode_spots(&raw);
            }
            app.ui_scale = storage
                .get_string("ui_scale")
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(1.0)
                .clamp(density::SCALE_MIN, density::SCALE_MAX);
            app.ui_scale_draft = app.ui_scale;
            app.library_layout = density::LibraryLayout::from_key(
                &storage.get_string("library_layout").unwrap_or_default(),
            );
            if let Some(raw) = storage.get_string("books") {
                app.books = books::decode(&raw);
            }
            app.book_spread = storage.get_string("book_spread").as_deref() != Some("0");
            app.default_asked = storage.get_string("default_asked").as_deref() == Some("1");
            app.reading = match storage.get_string("reading") {
                Some(raw) => reader::ReadingStyle::decode(&raw),
                // Before typefaces could be chosen only the size was kept.
                None => reader::ReadingStyle {
                    size: storage
                        .get_string("book_font")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(18.0f32)
                        .clamp(reader::FONT_MIN, reader::FONT_MAX),
                    ..Default::default()
                },
            };
            if let Some(raw) = storage.get_string("folders") {
                app.folders = folders::decode(&raw);
            }
            app.peek.tips_shown = storage
                .get_string("peek_tips")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if let Some(z) = storage.get_string("zoom") {
                app.zoom = zoom_from_str(&z);
            }
            app.auto_save = storage.get_string("auto_save").as_deref() == Some("1");
            app.auto_save_mb = storage
                .get_string("auto_save_mb")
                .and_then(|v| v.parse().ok())
                .unwrap_or(20)
                .clamp(1, 500);
            last_file = storage.get_string("last_file").map(PathBuf::from);
            if let Some(r) = storage.get_string("recents") {
                app.recents = decode_recents(&r);
            }
            app.cloud_cfg = B2Config {
                key_id: storage.get_string("b2_key_id").unwrap_or_default(),
                app_key: storage.get_string("b2_app_key").unwrap_or_default(),
                bucket: storage.get_string("b2_bucket").unwrap_or_default(),
            };
        }
        // Scripted UI tests skip the first-launch question unless testing it.
        #[cfg(debug_assertions)]
        if std::env::var_os("PAPERVIEW_AUTOMATE").is_some()
            && std::env::var_os("PAPERVIEW_FIRST_RUN").is_none()
        {
            app.default_asked = true;
        }
        app.b2_draft = app.cloud_cfg.clone();
        theme::install_fonts(&cc.egui_ctx);
        app.refresh_local_docs();

        // Migrate the pre-landing-page "last file" into the recents list.
        if app.recents.is_empty() {
            if let Some(p) = last_file.filter(|p| p.exists()) {
                app.recents.push(Recent {
                    path: p,
                    ts: now_ts(),
                    page: 0,
                    zoom: Zoom::FitWidth,
                    rotation: 0,
                });
            }
        }

        // Know the cloud listing early: auto-save needs it to skip files
        // that are already up there.
        if app.cloud_cfg.ready() && app.cloud_enabled {
            app.start_cloud_op(&cc.egui_ctx, CloudOp::List);
        }

        if let Some(path) = initial_file {
            // A directly-launched PDF skips the landing page (and session
            // restore — it gets exactly the file that was asked for).
            app.open_path(&cc.egui_ctx, &path);
        } else {
            // Restore last session's tabs, parked; the landing page shows.
            let restore: Vec<PathBuf> = cc
                .storage
                .and_then(|s| s.get_string("open_tabs"))
                .map(|s| {
                    s.lines()
                        .filter(|l| !l.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default();
            for path in restore.iter().take(8).filter(|p| p.exists()) {
                app.try_open(&cc.egui_ctx, path, None, true);
            }
            app.go_home(&cc.egui_ctx);
        }
        app
    }

    fn open_path(&mut self, ctx: &egui::Context, path: &Path) {
        self.try_open(ctx, path, None, false);
    }

    fn try_open(&mut self, ctx: &egui::Context, path: &Path, password: Option<&str>, quiet: bool) {
        if crate::ebook::is_ebook(path) {
            self.open_book(ctx, path);
            return;
        }
        // Already open in a tab? Just switch to it.
        if password.is_none() {
            if let Some(i) = self.tabs.iter().position(|t| same_path(&t.path, path)) {
                self.activate_tab(ctx, i);
                return;
            }
        }
        let Some(engine) = &self.engine else { return };
        match engine.open(path, password) {
            Ok(doc) => {
                #[cfg(debug_assertions)]
                eprintln!(
                    "paperview: loaded {} ({} pages)",
                    doc.path.display(),
                    doc.page_count()
                );
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} — PaperView")));
                // Park the outgoing tab, then install this document as a
                // new tab whose state lives inline.
                self.stash_active_tab();
                self.tabs.push(Tab {
                    title: name.clone(),
                    path: path.to_path_buf(),
                    state: None,
                    book: None,
                });
                self.active_tab = Some(self.tabs.len() - 1);
                self.page_sizes = Arc::new(doc.page_sizes_pt.clone());
                self.toc = doc.toc();
                self.doc = Some(doc);
                self.cache.clear();
                self.thumb_cache.clear();
                self.search.deactivate();
                self.set_selection(None);
                self.text_counts.clear();
                self.back_stack.clear();
                self.fwd_stack.clear();
                self.password_prompt = None;
                self.status = None;
                // Restore this file's remembered state (Sumatra-style).
                let remembered = self
                    .recents
                    .iter()
                    .find(|r| same_path(&r.path, path))
                    .map(|r| (r.page, r.zoom, r.rotation));
                // Unknown files start at the default zoom (fit page).
                let (page, zoom, rotation) = remembered.unwrap_or((0, Zoom::FitPage, 0));
                self.page_index = page.clamp(0, self.page_count().saturating_sub(1).max(0));
                self.zoom = zoom;
                self.rotation = rotation % 4;
                self.scroll_to = Some(ScrollTarget::PageTop(self.page_index));
                self.doc_password = password.map(|s| s.to_string());
                if !quiet {
                    // Bump the recents list, carrying the state forward.
                    self.recents.retain(|r| !same_path(&r.path, path));
                    self.recents.insert(
                        0,
                        Recent {
                            path: path.to_path_buf(),
                            ts: now_ts(),
                            page: self.page_index,
                            zoom: self.zoom,
                            rotation: self.rotation,
                        },
                    );
                    self.recents.truncate(MAX_RECENTS);
                    self.maybe_auto_save(ctx, path);
                }
            }
            Err(OpenError::PasswordRequired) => {
                if !quiet {
                    self.password_prompt =
                        Some((path.to_path_buf(), String::new(), password.is_some()));
                }
            }
            Err(OpenError::Message(msg)) => {
                if !quiet {
                    self.status = Some(msg);
                }
            }
        }
    }

    /// Queues a cloud upload of a just-opened file when auto-save applies.
    fn maybe_auto_save(&mut self, ctx: &egui::Context, path: &Path) {
        if !self.auto_save || !self.cloud_active() {
            return;
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(u64::MAX);
        let from_cache = path.starts_with(cloud::cache_dir());
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let already = self
            .cloud_files
            .iter()
            .any(|f| f.name == name && f.size == size);
        if should_auto_save(self.auto_save_mb, size, from_cache, already) {
            self.toast(format!("Saving {name} to your bucket…"));
            self.start_cloud_op(ctx, CloudOp::upload(path.to_path_buf()));
        }
    }

    /// Writes the open document's page/zoom/rotation into its recents entry.
    fn stash_doc_state(&mut self) {
        let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) else {
            return;
        };
        let (page, zoom) = self
            .peek
            .held_view()
            .map(|(p, z, _)| (p, z))
            .unwrap_or((self.page_index, self.zoom));
        let rotation = self.rotation;
        if let Some(r) = self.recents.iter_mut().find(|r| same_path(&r.path, &path)) {
            r.page = page;
            r.zoom = zoom;
            r.rotation = rotation;
        }
    }

    /// Jump that records history (links, TOC, page box) — Alt+Left goes back.
    fn jump_with_history(&mut self, page: i32) {
        let count = self.page_count();
        if count == 0 {
            return;
        }
        let target = page.clamp(0, count - 1);
        if target != self.page_index {
            self.back_stack.push(self.page_index);
            self.back_stack.truncate(64);
            self.fwd_stack.clear();
        }
        self.go_to_page(target);
    }

    fn nav_back(&mut self) {
        if let Some(p) = self.back_stack.pop() {
            self.fwd_stack.push(self.page_index);
            self.go_to_page(p);
        }
    }

    fn nav_forward(&mut self) {
        if let Some(p) = self.fwd_stack.pop() {
            self.back_stack.push(self.page_index);
            self.go_to_page(p);
        }
    }

    /// Which page (strictly) contains this screen position, and where on it
    /// in unrotated page coordinates (points).
    fn page_at(&self, pages: &[(i32, egui::Rect)], pos: egui::Pos2) -> Option<(i32, egui::Vec2)> {
        let scale = PX_PER_PT * self.effective_zoom;
        for &(page, rect) in pages {
            if rect.contains(pos) {
                let view_local = (pos - rect.min) / scale;
                let (w_pt, h_pt) = self.page_sizes.get(page as usize).copied()?;
                return Some((page, point_to_page(view_local, self.rotation, w_pt, h_pt)));
            }
        }
        None
    }

    /// Resets every inline per-document field (no tab bookkeeping).
    fn clear_inline_doc(&mut self) {
        self.doc = None;
        self.page_sizes = Arc::new(Vec::new());
        self.page_index = 0;
        self.rotation = 0;
        self.cache.clear();
        self.thumb_cache.clear();
        self.search.deactivate();
        self.set_selection(None);
        self.text_counts.clear();
        self.toc = Vec::new();
        self.back_stack.clear();
        self.fwd_stack.clear();
        self.doc_password = None;
        self.scroll_to = None;
        self.last_hit_test = None;
        self.last_link_probe = None;
    }

    /// Parks the active tab's inline state back into its slot.
    fn stash_active_tab(&mut self) {
        self.peek_leave_doc();
        if self.book.is_some() {
            self.save_book_progress();
            let view = self.book.take().map(Box::new);
            if let Some(slot) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
                slot.book = view;
            }
            self.active_tab = None;
            return;
        }
        if self.doc.is_none() {
            self.active_tab = None;
            return;
        }
        self.stash_doc_state(); // per-file memory (recents)
        let Some(i) = self.active_tab else {
            // Inline doc without a tab slot shouldn't happen; drop it.
            self.clear_inline_doc();
            return;
        };
        let doc = self.doc.take().expect("checked above");
        let state = DocTab {
            doc,
            page_sizes: std::mem::take(&mut self.page_sizes),
            page_index: self.page_index,
            zoom: self.zoom,
            effective_zoom: self.effective_zoom,
            rotation: self.rotation,
            search: std::mem::take(&mut self.search),
            selection: self.selection.take(),
            text_counts: std::mem::take(&mut self.text_counts),
            toc: std::mem::take(&mut self.toc),
            back_stack: std::mem::take(&mut self.back_stack),
            fwd_stack: std::mem::take(&mut self.fwd_stack),
            doc_password: self.doc_password.take(),
            saved_offset: self.last_offset,
        };
        if let Some(slot) = self.tabs.get_mut(i) {
            slot.state = Some(Box::new(state));
        }
        // Reset leftovers (textures re-render on switch-back).
        self.cache.clear();
        self.thumb_cache.clear();
        self.sel_rects.clear();
        self.selecting = false;
        self.last_hit_test = None;
        self.last_link_probe = None;
        self.scroll_to = None;
        self.rotation = 0;
        self.active_tab = None;
    }

    /// Makes tab `i` the active one (parks the current tab first).
    fn activate_tab(&mut self, ctx: &egui::Context, i: usize) {
        if self.active_tab == Some(i) || i >= self.tabs.len() {
            return;
        }
        self.stash_active_tab();
        if let Some(view) = self.tabs[i].book.take() {
            self.book = Some(*view);
            self.active_tab = Some(i);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "{} — PaperView",
                self.tabs[i].title
            )));
            return;
        }
        let Some(state) = self.tabs[i].state.take() else {
            return;
        };
        let s = *state;
        self.doc = Some(s.doc);
        self.page_sizes = s.page_sizes;
        self.page_index = s.page_index;
        self.zoom = s.zoom;
        self.effective_zoom = s.effective_zoom;
        self.rotation = s.rotation;
        self.search = s.search;
        self.selection = s.selection;
        self.sel_rects.clear();
        self.text_counts = s.text_counts;
        self.toc = s.toc;
        self.back_stack = s.back_stack;
        self.fwd_stack = s.fwd_stack;
        self.doc_password = s.doc_password;
        self.scroll_to = Some(ScrollTarget::Offset(s.saved_offset));
        self.page_field = (s.page_index + 1).to_string();
        self.active_tab = Some(i);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "{} — PaperView",
            self.tabs[i].title
        )));
    }

    /// Closes tab `i` (active or background).
    fn close_tab(&mut self, ctx: &egui::Context, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        // Remember the closed tab so Ctrl+Shift+T can reopen it.
        let tab = &self.tabs[i];
        let password = if self.active_tab == Some(i) {
            self.doc_password.clone()
        } else {
            tab.state.as_ref().and_then(|s| s.doc_password.clone())
        };
        self.closed_tabs.push((tab.path.clone(), password));
        if self.closed_tabs.len() > 32 {
            self.closed_tabs.remove(0); // drop the oldest, keep recent closes
        }
        if self.active_tab == Some(i) {
            self.peek_leave_doc();
            self.save_book_progress();
            self.book = None;
            self.stash_doc_state();
            self.clear_inline_doc();
            self.tabs.remove(i);
            self.active_tab = None;
            if self.tabs.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title("PaperView".into()));
            } else {
                let next = i.min(self.tabs.len() - 1);
                self.activate_tab(ctx, next);
            }
        } else {
            self.tabs.remove(i);
            self.active_tab = fix_active_after_remove(self.active_tab, i);
        }
    }

    /// Shows the landing page; open tabs stay parked in the strip.
    fn go_home(&mut self, ctx: &egui::Context) {
        self.stash_active_tab();
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("PaperView".into()));
    }

    /// Cycles to the next/previous tab (Ctrl+Tab / Ctrl+Shift+Tab).
    fn cycle_tab(&mut self, ctx: &egui::Context, dir: i32) {
        let len = self.tabs.len();
        if len == 0 {
            return;
        }
        let next = match self.active_tab {
            Some(a) => (a as i32 + dir).rem_euclid(len as i32) as usize,
            None => {
                if dir > 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        self.activate_tab(ctx, next);
    }

    /// Close the current document (its tab) and return to the landing page.
    fn close_doc(&mut self, ctx: &egui::Context) {
        if let Some(i) = self.active_tab {
            self.close_tab(ctx, i);
        } else {
            self.clear_inline_doc();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("PaperView".into()));
        }
    }

    /// Reopens the most recently closed tab (Ctrl+Shift+T).
    fn reopen_closed_tab(&mut self, ctx: &egui::Context) {
        if let Some((path, password)) = self.closed_tabs.pop() {
            if path.exists() {
                self.try_open(ctx, &path, password.as_deref(), false);
            }
        }
    }

    fn start_cloud_op(&mut self, ctx: &egui::Context, op: CloudOp) {
        if self.cloud_busy {
            self.cloud_queue.push_back(op);
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.cloud_rx = Some(rx);
        self.cloud_busy = true;
        self.cloud_status = None;
        cloud::spawn(self.cloud_cfg.clone(), op, tx, ctx.clone());
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        let mut all = vec!["pdf"];
        all.extend_from_slice(crate::ebook::EXTENSIONS);
        let mut dialog = rfd::FileDialog::new()
            .add_filter("PDFs and books", &all)
            .add_filter("PDF documents", &["pdf"])
            .add_filter("Books", crate::ebook::EXTENSIONS);
        if let Some(dir) = self.doc.as_ref().and_then(|d| d.path.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_path(ctx, &path);
        }
    }

    fn page_count(&self) -> i32 {
        self.page_sizes.len() as i32
    }

    fn go_to_page(&mut self, page: i32) {
        let count = self.page_count();
        if count == 0 {
            return;
        }
        let clamped = page.clamp(0, count - 1);
        if clamped != self.page_index {
            self.page_index = clamped;
            self.scroll_to = Some(ScrollTarget::PageTop(clamped));
        }
        self.page_field = (clamped + 1).to_string();
    }

    fn change_zoom(&mut self, zoom: Zoom, anchor: Option<egui::Vec2>) {
        self.zoom_prev_effective = Some(self.effective_zoom);
        self.zoom_anchor = anchor;
        self.zoom_gesture_until = Some(Instant::now() + ZOOM_SETTLE);
        self.zoom = match zoom {
            Zoom::Custom(z) => Zoom::Custom(z.clamp(MIN_ZOOM, MAX_ZOOM)),
            other => other,
        };
    }

    fn zoom_gesture_active(&self) -> bool {
        self.zoom_gesture_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn pointer_anchor(&self, ctx: &egui::Context) -> Option<egui::Vec2> {
        let pos = ctx.input(|i| i.pointer.hover_pos())?;
        if self.central_rect.contains(pos) {
            Some(pos - self.central_rect.min)
        } else {
            None
        }
    }

    fn search_step(&mut self, dir: i32) {
        let sel = match &self.doc {
            Some(doc) => self.search.step(doc, dir),
            None => None,
        };
        if let Some((page, hit)) = sel {
            self.page_index = page;
            self.scroll_to = Some(ScrollTarget::Hit(page, hit));
        }
    }

    fn submit_search(&mut self) {
        let sel = match &self.doc {
            Some(doc) => self.search.submit(doc, self.page_index),
            None => None,
        };
        if let Some((page, hit)) = sel {
            self.page_index = page;
            self.scroll_to = Some(ScrollTarget::Hit(page, hit));
        }
    }

    /// Opens the system print dialog and spawns the print worker.
    fn start_print(&mut self, ctx: &egui::Context) {
        if self.printing || self.print_preview.is_some() {
            return;
        }
        let Some(doc) = &self.doc else { return };
        let (path, name) = (
            doc.path.clone(),
            doc.path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "PaperView document".into()),
        );
        let count = self.page_count();
        let Some(engine) = self.engine else { return };
        let _ = (engine, ctx); // job spawns after the preview is confirmed
        match print::show_dialog(self.hwnd, count) {
            Ok(Some(ticket)) => {
                let page = ticket.from;
                self.print_preview = Some(PrintPreviewState {
                    ticket: Some(ticket),
                    path,
                    doc_name: name,
                    page,
                    tex: None,
                    tex_page: -1,
                });
            }
            Ok(None) => {} // cancelled
            Err(e) => self.status = Some(e),
        }
    }

    /// Accurate preview of the pending print job (paper metrics come from the
    /// chosen printer's device context). Confirms or cancels the job.
    fn print_preview_modal(&mut self, ctx: &egui::Context) {
        let Some(mut pv) = self.print_preview.take() else {
            return;
        };
        let Some(ticket_ref) = pv.ticket.as_ref() else {
            return;
        };
        let (from, to) = (ticket_ref.from, ticket_ref.to);
        let (dev_w, dev_h, dpi) = (ticket_ref.dev_w, ticket_ref.dev_h, ticket_ref.dpi);

        // Ensure the preview texture for the current page exists (modest
        // resolution; the print path renders separately at printer dpi).
        let (w_pt, h_pt) = self
            .page_sizes
            .get(pv.page as usize)
            .copied()
            .unwrap_or((612.0, 792.0));
        let layout = print::layout_on_paper(w_pt, h_pt, dev_w, dev_h);
        if pv.tex_page != pv.page {
            if let Some(doc) = &self.doc {
                let ppp = ctx.pixels_per_point();
                let px = ((340.0 * layout.w * ppp).round() as i32).clamp(64, 2000);
                if let Ok(r) = doc.render_page(pv.page, px, layout.rotate) {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([r.width, r.height], &r.rgba);
                    pv.tex = Some(ctx.load_texture(
                        "print-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                    pv.tex_page = pv.page;
                }
            }
        }

        let mut do_print = false;
        let mut do_cancel = false;
        let modal = w::modal("print-preview", ctx).show(ctx, |ui| {
            ui.label(egui::RichText::new("Print").text_style(theme::headline()));
            let paper_in = (dev_w as f32 / dpi as f32, dev_h as f32 / dpi as f32);
            ui.label(
                egui::RichText::new(format!(
                    "Pages {} to {} | paper {:.1} x {:.1} in | {} dpi{}",
                    from + 1,
                    to + 1,
                    paper_in.0,
                    paper_in.1,
                    dpi,
                    if layout.rotate == 1 {
                        " | rotated to fit"
                    } else {
                        ""
                    }
                ))
                .small()
                .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(6.0);

            // Linux has no system print dialog: the choices live here.
            #[cfg(not(target_os = "windows"))]
            if let Some(t) = pv.ticket.as_mut() {
                print::options_ui(ui, t);
                ui.add_space(10.0);
            }

            // Paper mock-up in the printable area's exact aspect ratio.
            let aspect = dev_h as f32 / dev_w as f32;
            let mut paper_w = 340.0_f32;
            let mut paper_h = paper_w * aspect;
            if paper_h > 420.0 {
                paper_h = 420.0;
                paper_w = paper_h / aspect;
            }
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(paper_w, paper_h), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(1), egui::Color32::WHITE);
            if let Some(tex) = &pv.tex {
                let dest = egui::Rect::from_min_size(
                    rect.min + egui::vec2(layout.x * paper_w, layout.y * paper_h),
                    egui::vec2(layout.w * paper_w, layout.h * paper_h),
                );
                ui.painter().image(
                    tex.id(),
                    dest,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(pv.page > from, egui::Button::new("‹"))
                    .clicked()
                {
                    pv.page -= 1;
                }
                ui.label(format!("Page {} of {}", pv.page + 1, to + 1));
                if ui
                    .add_enabled(pv.page < to, egui::Button::new("›"))
                    .clicked()
                {
                    pv.page += 1;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if w::primary_button(ui, "Print", true).clicked() {
                        do_print = true;
                    }
                    if w::outlined_button(ui, "Cancel", true).clicked() {
                        do_cancel = true;
                    }
                });
            });
        });
        if modal.should_close() {
            do_cancel = true;
        }
        // The range may have just changed: keep the previewed page inside it.
        if let Some(t) = pv.ticket.as_ref() {
            pv.page = pv.page.clamp(t.from, t.to.max(t.from));
        }

        if do_print {
            if let (Some(ticket), Some(engine)) = (pv.ticket.take(), self.engine) {
                let (tx, rx) = std::sync::mpsc::channel();
                self.print_rx = Some(rx);
                self.printing = true;
                print::spawn_print(
                    engine,
                    pv.path.clone(),
                    self.doc_password.clone(),
                    ticket,
                    pv.doc_name.clone(),
                    tx,
                    ctx.clone(),
                );
            }
        } else if !do_cancel {
            self.print_preview = Some(pv);
        }
        // On cancel `pv` drops here; the ticket's Drop releases the DC.
    }

    /// Rotates the whole document view by the given quarter turns.
    fn rotate_by(&mut self, quarters: i8) {
        self.rotation = (self.rotation as i8 + quarters).rem_euclid(4) as u8;
        self.cache.clear();
        self.thumb_cache.clear();
        self.scroll_to = Some(ScrollTarget::PageTop(self.page_index));
    }

    /// "Save as": copies the open document's file to a chosen destination.
    fn save_as(&mut self) {
        let Some(src) = self.doc.as_ref().map(|d| d.path.clone()) else {
            return;
        };
        let name = src
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document.pdf".into());
        let dialog = rfd::FileDialog::new()
            .add_filter("PDF documents", &["pdf"])
            .set_file_name(&name);
        if let Some(dest) = dialog.save_file() {
            match std::fs::copy(&src, &dest) {
                Ok(_) => self.toast(format!("Saved a copy to {}", dest.display())),
                Err(e) => self.status = Some(format!("Save as failed: {e}")),
            }
        }
    }

    // ── text selection ───────────────────────────────────────────────────────

    fn set_selection(&mut self, sel: Option<((i32, i32), (i32, i32))>) {
        self.selection = sel;
        self.selecting = false;
        self.sel_rects.clear();
    }

    /// Selection normalized to document order (start ≤ end).
    fn sel_ordered(&self) -> Option<((i32, i32), (i32, i32))> {
        self.selection
            .map(|(a, b)| if b < a { (b, a) } else { (a, b) })
    }

    fn char_count_cached(&mut self, page: i32) -> i32 {
        if let Some(c) = self.text_counts.get(&page) {
            return *c;
        }
        let c = self
            .doc
            .as_ref()
            .and_then(|d| d.char_count(page).ok())
            .unwrap_or(0);
        self.text_counts.insert(page, c);
        c
    }

    /// The selected (start, count) char range on one page, if any.
    fn sel_range_on_page(&mut self, page: i32) -> Option<(i32, i32)> {
        let (a, b) = self.sel_ordered()?;
        if page < a.0 || page > b.0 {
            return None;
        }
        let start = if page == a.0 { a.1 } else { 0 };
        let end = if page == b.0 {
            b.1
        } else {
            self.char_count_cached(page) - 1
        };
        if end < start {
            return None;
        }
        Some((start, end - start + 1))
    }

    /// Cached highlight rects for the current selection on one page.
    fn sel_rects_for(&mut self, page: i32) -> Option<Vec<crate::pdf::HitRect>> {
        let range = self.sel_range_on_page(page)?;
        if let Some(rects) = self.sel_rects.get(&page) {
            return Some(rects.clone());
        }
        let rects = self
            .doc
            .as_ref()?
            .range_rects(page, range.0, range.1)
            .ok()?;
        self.sel_rects.insert(page, rects.clone());
        Some(rects)
    }

    /// Maps a screen position to (page, char index). `strict` requires the
    /// point to be on a page and near text (selection start); non-strict
    /// snaps to the nearest page/char (drag continuation).
    fn hit_test(
        &mut self,
        pages: &[(i32, egui::Rect)],
        pos: egui::Pos2,
        strict: bool,
    ) -> Option<(i32, i32)> {
        if !strict {
            if let Some((p, r)) = self.last_hit_test {
                if (p - pos).length() < 2.0 {
                    return Some(r);
                }
            }
        }
        let scale = PX_PER_PT * self.effective_zoom;
        let mut best: Option<(i32, egui::Rect, f32)> = None;
        for &(page, rect) in pages {
            let dx = (rect.min.x - pos.x).max(pos.x - rect.max.x).max(0.0);
            let dy = (rect.min.y - pos.y).max(pos.y - rect.max.y).max(0.0);
            let d = dx.max(dy);
            if d <= 0.0 {
                best = Some((page, rect, 0.0));
                break;
            }
            if !strict && best.is_none_or(|(_, _, bd)| d < bd) {
                best = Some((page, rect, d));
            }
        }
        let (page, rect, dist) = best?;
        if strict && dist > 0.0 {
            return None;
        }
        let view_local = (pos - rect.min) / scale; // view points (rotated space)
        let (w_pt, h_pt) = self.page_sizes.get(page as usize).copied()?;
        let local = point_to_page(view_local, self.rotation, w_pt, h_pt);
        let (tol_x, tol_y) = if strict { (12.0, 10.0) } else { (w_pt, h_pt) };
        let idx = {
            let doc = self.doc.as_ref()?;
            doc.char_index_at(page, local.x, local.y, tol_x, tol_y, !strict)
                .ok()??
        };
        let result = (page, idx);
        self.last_hit_test = Some((pos, result));
        Some(result)
    }

    fn copy_selection(&mut self, ctx: &egui::Context) {
        let Some((a, b)) = self.sel_ordered() else {
            return;
        };
        let mut out = String::new();
        for page in a.0..=b.0 {
            let Some((start, count)) = self.sel_range_on_page(page) else {
                continue;
            };
            let text = match &self.doc {
                Some(doc) => doc.range_text(page, start, count).unwrap_or_default(),
                None => return,
            };
            if !out.is_empty() && !out.ends_with('\n') {
                out.push_str("\r\n");
            }
            out.push_str(&text);
        }
        if !out.is_empty() {
            ctx.copy_text(out);
        }
    }

    fn select_all(&mut self) {
        let count = self.page_count();
        if count == 0 {
            return;
        }
        let last = count - 1;
        let last_chars = self.char_count_cached(last);
        self.set_selection(Some(((0, 0), (last, (last_chars - 1).max(0)))));
    }

    /// Selection pointer interaction over the page area. `pages` are the
    /// on-screen page rects; `content` is the full scrollable content rect.
    fn selection_ui(
        &mut self,
        ui: &mut egui::Ui,
        pages: &[(i32, egui::Rect)],
        content: egui::Rect,
    ) {
        let id = ui.id().with("text-select");
        let resp = ui.interact(content, id, egui::Sense::click_and_drag());
        let pos = resp.interact_pointer_pos().or_else(|| resp.hover_pos());

        // Link probe under the pointer (throttled to pointer movement).
        let mut hovered_link: Option<LinkTarget> = None;
        if let Some(p) = pos {
            if !self.selecting {
                match self.last_link_probe {
                    Some((prev, ref cached)) if (prev - p).length() < 2.0 => {
                        hovered_link = cached.clone();
                    }
                    _ => {
                        hovered_link = self.page_at(pages, p).and_then(|(page, pt)| {
                            let doc = self.doc.as_ref()?;
                            doc.link_at(page, pt.x, pt.y).ok().flatten()
                        });
                        self.last_link_probe = Some((p, hovered_link.clone()));
                    }
                }
            }
        }
        if resp.hovered() {
            let icon = if hovered_link.is_some() {
                egui::CursorIcon::PointingHand
            } else {
                egui::CursorIcon::Text
            };
            ui.output_mut(|o| o.cursor_icon = icon);
            if let Some(LinkTarget::Uri(uri)) = &hovered_link {
                resp.clone().on_hover_text(uri.clone());
            }
        }

        if resp.drag_started() {
            if let Some(p) = pos {
                match self.hit_test(pages, p, true) {
                    Some(hit) => {
                        self.set_selection(Some((hit, hit)));
                        self.selecting = true;
                    }
                    None => self.set_selection(None),
                }
            }
        } else if resp.dragged() && self.selecting {
            if let Some(p) = pos {
                if let Some(hit) = self.hit_test(pages, p, false) {
                    if let Some((anchor, head)) = self.selection {
                        if head != hit {
                            self.selection = Some((anchor, hit));
                            self.sel_rects.clear();
                        }
                    }
                }
            }
        }
        if resp.drag_stopped() {
            self.selecting = false;
        }

        if resp.double_clicked() {
            if let Some(p) = pos {
                if let Some((page, idx)) = self.hit_test(pages, p, true) {
                    let range = self
                        .doc
                        .as_ref()
                        .and_then(|d| d.word_range_at(page, idx).ok());
                    if let Some((start, count)) = range {
                        self.set_selection(Some(((page, start), (page, start + count - 1))));
                    }
                }
            }
        } else if resp.clicked() {
            match hovered_link {
                Some(LinkTarget::Page(page)) => self.jump_with_history(page),
                Some(LinkTarget::Uri(uri)) => {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(uri));
                }
                None => self.set_selection(None),
            }
        }

        let ctx = ui.ctx().clone();
        resp.context_menu(|ui| {
            let have = self.selection.is_some();
            if ui.add_enabled(have, egui::Button::new("Copy")).clicked() {
                self.copy_selection(&ctx);
                ui.close();
            }
            if ui.button("Select all").clicked() {
                self.select_all();
                ui.close();
            }
            ui.separator();
            if ui.button("Save as…").clicked() {
                self.save_as();
                ui.close();
            }
            if ui.button("Open file location").clicked() {
                if let Some(p) = self.doc.as_ref().map(|d| d.path.clone()) {
                    open_file_location(&p);
                }
                ui.close();
            }
        });
    }

    // ── input ────────────────────────────────────────────────────────────────

    fn handle_input(&mut self, ctx: &egui::Context) {
        // File drops: on the Saved tab they queue uploads; otherwise open.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        // A dropped folder joins the Library; files go on as before.
        let (dirs, dropped): (Vec<PathBuf>, Vec<PathBuf>) =
            dropped.into_iter().partition(|p| p.is_dir());
        for dir in dirs {
            self.add_folder(dir);
        }
        if !dropped.is_empty() {
            let upload_mode = self.doc.is_none()
                && self.home == HomeScreen::Library
                && self.landing_tab == LandingTab::Saved
                && self.cloud_active();
            if upload_mode {
                let pdfs: Vec<PathBuf> = dropped
                    .iter()
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
                    .cloned()
                    .collect();
                let mut queued = 0usize;
                if let [one] = pdfs.as_slice() {
                    // One file: let it be named first.
                    self.ask_upload(ctx, one.clone());
                } else {
                    for path in pdfs {
                        self.start_cloud_op(ctx, CloudOp::upload(path));
                        queued += 1;
                    }
                }
                if queued > 0 {
                    self.cloud_status = Some(format!(
                        "Queued {queued} upload{}",
                        if queued == 1 { "" } else { "s" }
                    ));
                } else if let Some(path) = dropped.first() {
                    self.open_path(ctx, path);
                }
            } else if let Some(path) = dropped.first() {
                self.open_path(ctx, path);
            }
        }

        let typing = ctx.egui_wants_keyboard_input();
        let search_active = self.search.active || self.search_open;
        let has_selection = self.selection.is_some();
        let immersive = self.immersive;

        struct Keys {
            open: bool,
            next: bool,
            prev: bool,
            first: bool,
            last: bool,
            zoom_in: bool,
            zoom_out: bool,
            zoom_100: bool,
            pinch: f32,
            find: bool,
            find_next: bool,
            find_prev: bool,
            nav_back: bool,
            nav_fwd: bool,
            scroll_lines: f32,
            space_fwd: bool,
            space_back: bool,
            escape: bool,
            copy: bool,
            select_all: bool,
            save_as: bool,
            print: bool,
            close_tab: bool,
            tab_next: bool,
            tab_prev: bool,
            reopen_closed: bool,
            wheel_y: f32,
            settings: bool,
            fullscreen: bool,
            leave_fullscreen: bool,
            goto_page: bool,
            peek_set: bool,
            peek_down: bool,
            peek_quick: bool,
        }
        let k = ctx.input_mut(|i| Keys {
            open: i.consume_key(egui::Modifiers::COMMAND, egui::Key::O),
            zoom_in: i.consume_key(egui::Modifiers::COMMAND, egui::Key::Plus)
                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Equals)
                || (!typing && i.key_pressed(egui::Key::Plus)),
            zoom_out: i.consume_key(egui::Modifiers::COMMAND, egui::Key::Minus)
                || (!typing && i.key_pressed(egui::Key::Minus)),
            zoom_100: i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0),
            next: !typing
                && (i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::ArrowRight)),
            prev: !typing
                && (i.key_pressed(egui::Key::PageUp) || i.key_pressed(egui::Key::ArrowLeft)),
            first: !typing && i.key_pressed(egui::Key::Home),
            last: !typing && i.key_pressed(egui::Key::End),
            pinch: i.zoom_delta(),
            find: i.consume_key(egui::Modifiers::COMMAND, egui::Key::F),
            find_next: i.consume_key(egui::Modifiers::NONE, egui::Key::F3),
            find_prev: i.consume_key(egui::Modifiers::SHIFT, egui::Key::F3),
            nav_back: i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft),
            nav_fwd: i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight),
            scroll_lines: {
                let mut d = 0.0f32;
                if !typing {
                    if i.key_pressed(egui::Key::ArrowDown) {
                        d += 64.0;
                    }
                    if i.key_pressed(egui::Key::ArrowUp) {
                        d -= 64.0;
                    }
                }
                d
            },
            space_fwd: !typing
                && i.modifiers.is_none()
                && i.consume_key(egui::Modifiers::NONE, egui::Key::Space),
            space_back: !typing && i.consume_key(egui::Modifiers::SHIFT, egui::Key::Space),
            escape: (search_active || has_selection)
                && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            copy: !typing && i.consume_key(egui::Modifiers::COMMAND, egui::Key::C),
            select_all: !typing && i.consume_key(egui::Modifiers::COMMAND, egui::Key::A),
            save_as: i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
                || i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::S,
                ),
            print: i.consume_key(egui::Modifiers::COMMAND, egui::Key::P),
            close_tab: i.consume_key(egui::Modifiers::COMMAND, egui::Key::W),
            tab_next: i.consume_key(egui::Modifiers::COMMAND, egui::Key::Tab),
            tab_prev: i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Tab,
            ),
            reopen_closed: i.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::T,
            ),
            wheel_y: if i.modifiers.ctrl {
                0.0
            } else {
                i.smooth_scroll_delta.y
            },
            settings: i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma),
            fullscreen: i.consume_key(egui::Modifiers::NONE, egui::Key::F11),
            leave_fullscreen: immersive
                && !typing
                && i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            goto_page: i.consume_key(egui::Modifiers::COMMAND, egui::Key::G),
            peek_set: !typing && i.consume_key(egui::Modifiers::SHIFT, egui::Key::Q),
            peek_down: !typing
                && i.key_down(egui::Key::Q)
                && !i.modifiers.shift
                && !i.modifiers.command
                && !i.modifiers.alt,
            // Pressed and released within one frame: a tap the held-state
            // check alone would never see.
            peek_quick: !typing
                && i.key_pressed(egui::Key::Q)
                && !i.key_down(egui::Key::Q)
                && !i.modifiers.shift
                && !i.modifiers.command
                && !i.modifiers.alt,
        });
        self.wheel_y = k.wheel_y;

        if k.open {
            self.open_dialog(ctx);
        }
        if k.tab_next {
            self.cycle_tab(ctx, 1);
        }
        if k.tab_prev {
            self.cycle_tab(ctx, -1);
        }
        if k.reopen_closed {
            self.reopen_closed_tab(ctx);
        }
        if k.close_tab {
            if let Some(i) = self.active_tab {
                self.close_tab(ctx, i);
            }
        }
        if k.settings {
            self.set_immersive(ctx, false);
            self.open_settings(ctx);
        }
        let reading = self.doc.is_some() || self.book.is_some();
        if k.fullscreen && reading {
            let on = !self.immersive;
            self.set_immersive(ctx, on);
        }
        if k.leave_fullscreen {
            self.set_immersive(ctx, false);
        }
        // Ctrl+G: straight into the page box, emptied, ready for a number.
        if k.goto_page && reading {
            self.immersive_stir = Some(Instant::now());
            self.book_page_field.clear();
            self.page_field.clear();
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("page-field")));
        }
        if k.find && self.doc.is_some() {
            self.search_open = true;
            self.focus_search = true;
        }
        if k.escape {
            if self.selection.is_some() {
                self.set_selection(None);
            } else {
                self.close_search();
            }
        }
        // Books: the same keys turn pages; zoom keys change the type size.
        if self.book.is_some() && !typing {
            if let Some(v) = &mut self.book {
                if k.next || k.space_fwd || k.scroll_lines > 0.0 {
                    v.next();
                }
                if k.prev || k.space_back || k.scroll_lines < 0.0 {
                    v.prev();
                }
                if k.first {
                    v.first();
                }
                if k.last {
                    v.last();
                }
            }
            if k.zoom_in {
                self.bump_book_size(1.0);
            }
            if k.zoom_out {
                self.bump_book_size(-1.0);
            }
            self.save_book_progress();
        }
        if self.doc.is_none() {
            return;
        }
        // Peek: Shift+Q saves the spot; Q held peeks, Q tapped jumps.
        if k.peek_set {
            self.save_peek_spot();
            self.q_suppressed = true;
        }
        if self.q_suppressed {
            if !ctx.input(|i| i.key_down(egui::Key::Q)) {
                self.q_suppressed = false;
            }
        } else if k.peek_quick && !self.peek.key_held() {
            self.peek_tap();
        } else {
            self.peek_key(ctx, k.peek_down);
        }
        if k.copy {
            self.copy_selection(ctx);
        }
        if k.select_all {
            self.select_all();
        }
        if k.save_as {
            self.save_as();
        }
        if k.print {
            self.start_print(ctx);
        }
        if k.next {
            self.go_to_page(self.page_index + 1);
        }
        if k.prev {
            self.go_to_page(self.page_index - 1);
        }
        if k.first {
            self.go_to_page(0);
        }
        if k.last {
            self.go_to_page(i32::MAX); // clamped to the last page
        }
        if k.find_next {
            self.search_step(1);
        }
        if k.find_prev {
            self.search_step(-1);
        }
        if k.nav_back {
            self.nav_back();
        }
        if k.nav_fwd {
            self.nav_forward();
        }
        if k.scroll_lines != 0.0 {
            self.pending_scroll += k.scroll_lines;
        }
        let screenful = (self.central_rect.height() * 0.85).max(64.0);
        if k.space_fwd {
            if self.mode == ViewMode::Single && self.last_at_bottom {
                self.go_to_page(self.page_index + 1);
            } else {
                self.pending_scroll += screenful;
            }
        }
        if k.space_back {
            if self.mode == ViewMode::Single && self.last_at_top {
                self.page_index = (self.page_index - 1).max(0);
                self.scroll_to = Some(ScrollTarget::PageBottom(self.page_index));
            } else {
                self.pending_scroll -= screenful;
            }
        }
        if k.zoom_in {
            self.change_zoom(Zoom::Custom(self.effective_zoom * 1.25), None);
        }
        if k.zoom_out {
            self.change_zoom(Zoom::Custom(self.effective_zoom / 1.25), None);
        }
        if k.zoom_100 {
            self.change_zoom(Zoom::Custom(1.0), None);
        }
        if k.pinch != 1.0 {
            let anchor = self.pointer_anchor(ctx);
            self.change_zoom(Zoom::Custom(self.effective_zoom * k.pinch), anchor);
        }
    }

    // ── chrome ───────────────────────────────────────────────────────────────

    fn thumbs_body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let left = ((ui.available_width() - THUMB_W) * 0.5).max(8.0);
        {
            {
                let ppp = ctx.pixels_per_point();
                let px_w = ((THUMB_W * ppp).round() as i32).clamp(16, MAX_TEXTURE_DIM);
                let sizes = Arc::clone(&self.page_sizes);

                // Fixed-width thumbs stacked vertically.
                let gap = 10.0;
                let rot = self.rotation % 4;
                let mut tops = Vec::with_capacity(sizes.len());
                let mut y = gap;
                for &(w_pt, h_pt) in sizes.iter() {
                    let (dw, dh) = if rot % 2 == 1 {
                        (h_pt, w_pt)
                    } else {
                        (w_pt, h_pt)
                    };
                    let h = THUMB_W * dh / dw.max(1.0);
                    tops.push((y, h));
                    y += h + gap + 16.0; // room for the page-number caption
                }
                let total = egui::vec2(left + THUMB_W + 8.0, y);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, viewport| {
                        ui.set_min_size(total);
                        let origin = ui.min_rect().min;
                        let mut keep: HashSet<i32> = HashSet::new();
                        for (i, &(top, h)) in tops.iter().enumerate() {
                            if top + h < viewport.min.y - 200.0 {
                                continue;
                            }
                            if top > viewport.max.y + 200.0 {
                                break;
                            }
                            let page = i as i32;
                            keep.insert(page);
                            let rect = egui::Rect::from_min_size(
                                origin + egui::vec2(left, top),
                                egui::vec2(THUMB_W, h),
                            );
                            // Selection ring: a filled accent rect painted under
                            // the thumbnail (avoids stroke-API churn).
                            if page == self.page_index {
                                ui.painter().rect_filled(
                                    rect.expand(3.0),
                                    egui::CornerRadius::same(3),
                                    ui.visuals().selection.bg_fill,
                                );
                            }
                            match self.thumb_cache.render_into(
                                &ctx,
                                self.doc.as_ref().expect("doc checked by caller"),
                                page,
                                px_w,
                                rot,
                                "thumb",
                            ) {
                                Ok((id, tsize)) => {
                                    // Same pixel-grid snap as the main view.
                                    let min = ((rect.min.to_vec2() * ppp).round() / ppp).to_pos2();
                                    let snapped = egui::Rect::from_min_size(min, tsize / ppp);
                                    ui.painter().image(
                                        id,
                                        snapped,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        egui::Color32::WHITE,
                                    );
                                }
                                Err(e) => self.status = Some(e),
                            }
                            ui.painter().text(
                                rect.center_bottom() + egui::vec2(0.0, 3.0),
                                egui::Align2::CENTER_TOP,
                                (page + 1).to_string(),
                                egui::FontId::proportional(11.0),
                                ui.visuals().weak_text_color(),
                            );
                            let resp = ui.interact(
                                rect,
                                egui::Id::new(("thumb", page)),
                                egui::Sense::click(),
                            );
                            if resp.clicked() {
                                self.go_to_page(page);
                            }
                        }
                        // Evict thumbs far outside the visible range.
                        self.thumb_cache.retain(|p, _, r| {
                            r == rot && (keep.contains(&p) || (p - self.page_index).abs() < 40)
                        });
                    });
            }
        }
    }

    /// Modal dialog asking for an encrypted document's password.
    fn password_modal(&mut self, ctx: &egui::Context) {
        let Some((path, mut input, failed)) = self.password_prompt.take() else {
            return;
        };
        let mut submit = false;
        let mut cancel = false;
        w::modal("pdf-password", ctx).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(egui::RichText::new("Password required").strong());
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.label(format!("“{name}” is encrypted."));
            if failed {
                ui.colored_label(ui.visuals().error_fg_color, "Wrong password — try again.");
            }
            ui.add_space(6.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut input)
                    .password(true)
                    .hint_text("Password")
                    .desired_width(f32::INFINITY),
            );
            edit.request_focus();
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if submit && !input.is_empty() {
            let pw = input.clone();
            self.try_open(ctx, &path.clone(), Some(&pw), false);
        } else if !cancel {
            self.password_prompt = Some((path, input, failed));
        }
    }

    /// Table-of-contents sidebar.
    fn toc_body(&mut self, ui: &mut egui::Ui) {
        {
            {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let weak = ui.visuals().weak_text_color();
                        let mut jump_to: Option<i32> = None;
                        for entry in &self.toc {
                            ui.horizontal(|ui| {
                                ui.add_space(6.0 + entry.depth as f32 * 12.0);
                                match entry.page {
                                    Some(page) => {
                                        let current = page == self.page_index;
                                        let text = egui::RichText::new(&entry.title).size(13.0);
                                        let text = if current { text.strong() } else { text };
                                        if ui
                                            .add(egui::Button::new(text).frame(false))
                                            .on_hover_text(format!("Page {}", page + 1))
                                            .clicked()
                                        {
                                            jump_to = Some(page);
                                        }
                                    }
                                    None => {
                                        ui.label(
                                            egui::RichText::new(&entry.title)
                                                .size(13.0)
                                                .color(weak),
                                        );
                                    }
                                }
                            });
                        }
                        if let Some(page) = jump_to {
                            self.jump_with_history(page);
                        }
                    });
            }
        }
    }

    // ── page views ───────────────────────────────────────────────────────────

    fn central(&mut self, ui: &mut egui::Ui) {
        let p = w::pal(ui);
        // Pages sit on the skin's viewport tone; the home view on its paper.
        let canvas = if self.doc.is_none() && self.book.is_none() {
            p.background
        } else {
            p.viewport
        };
        let frame = egui::Frame::NONE.fill(canvas);
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if let Some(msg) = &self.engine_error {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new(msg).size(15.0));
                });
                return;
            }
            if let Some(view) = &mut self.book {
                let p = w::pal(ui);
                let moved = view.show(ui, &p);
                // The tab shows the book's own title once it's read.
                let title = view.title.clone();
                if let Some(tab) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
                    if tab.title != title && !title.trim().is_empty() {
                        tab.title = title;
                    }
                }
                if moved {
                    self.save_book_progress();
                }
                return;
            }
            if self.doc.is_none() {
                self.home_view(ui);
                return;
            }
            if self.page_sizes.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label("This document has no pages.");
                });
                return;
            }
            match self.mode {
                ViewMode::Continuous => self.continuous_view(ui),
                ViewMode::Single => self.single_view(ui),
            }
        });
    }

    /// Page size in points as *displayed* (swapped when rotated 90/270 deg).
    fn display_size(&self, page: i32) -> (f32, f32) {
        let (w, h) = self.page_sizes[page as usize];
        if self.rotation % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }

    fn resolve_zoom(&mut self, avail: egui::Vec2) -> f32 {
        let count = self.page_count();
        if count == 0 {
            return 1.0;
        }
        let idx = self.page_index.clamp(0, count - 1);
        let (w_pt, h_pt) = self.display_size(idx);
        let zoom = match self.zoom {
            Zoom::Custom(z) => z,
            Zoom::FitWidth => (avail.x - PAGE_GAP * 2.0).max(64.0) / (w_pt * PX_PER_PT),
            Zoom::FitPage => {
                let zw = (avail.x - PAGE_GAP * 2.0).max(64.0) / (w_pt * PX_PER_PT);
                let zh = (avail.y - PAGE_GAP * 2.0).max(64.0) / (h_pt * PX_PER_PT);
                zw.min(zh)
            }
        }
        .clamp(MIN_ZOOM, MAX_ZOOM);
        self.effective_zoom = zoom;
        zoom
    }

    /// Builds the scroll area, applying any pending jump or zoom-anchor
    /// offset adjustment.
    fn scrolled(
        &mut self,
        layout: &Layout,
        avail: egui::Vec2,
        continuous: bool,
    ) -> egui::ScrollArea {
        let mut scroll = egui::ScrollArea::both().auto_shrink([false, false]);

        let rect_for = |page: i32| -> egui::Rect {
            if continuous {
                layout.rects[page.clamp(0, layout.rects.len() as i32 - 1) as usize]
            } else {
                layout.rects[0]
            }
        };

        if let Some(target) = self.scroll_to.take() {
            let (y, x) = match target {
                ScrollTarget::Offset(o) => (o.y.max(0.0), Some(o.x.max(0.0))),
                ScrollTarget::PageTop(p) => ((rect_for(p).min.y - PAGE_GAP).max(0.0), None),
                ScrollTarget::PageBottom(p) => {
                    ((rect_for(p).max.y + PAGE_GAP - avail.y).max(0.0), None)
                }
                ScrollTarget::Spot {
                    page,
                    y_frac,
                    x_frac,
                } => {
                    let r = rect_for(page);
                    (
                        (r.min.y + y_frac * r.height()).max(0.0),
                        Some((x_frac * layout.content.x).max(0.0)),
                    )
                }
                ScrollTarget::Hit(p, h) => {
                    let r = rect_for(p);
                    if let Some(hit) = self.search.hit_rects(p, h) {
                        let s = PX_PER_PT * self.effective_zoom;
                        let (upw, uph) = self.page_sizes[p as usize];
                        let hr = rect_to_view(hit.rects[0], self.rotation, upw, uph);
                        let y = (r.min.y + hr.y * s - avail.y * 0.35).max(0.0);
                        let x = if layout.content.x > avail.x {
                            Some((r.min.x + hr.x * s - avail.x * 0.5).max(0.0))
                        } else {
                            None
                        };
                        (y, x)
                    } else {
                        (r.min.y.max(0.0), None)
                    }
                }
            };
            scroll = scroll.vertical_scroll_offset(y);
            if let Some(x) = x {
                scroll = scroll.horizontal_scroll_offset(x);
            }
            self.zoom_prev_effective = None;
            self.zoom_anchor = None;
        } else if self.pending_scroll != 0.0 {
            let delta = std::mem::take(&mut self.pending_scroll);
            scroll = scroll.vertical_scroll_offset((self.last_offset.y + delta).max(0.0));
            self.zoom_prev_effective = None;
        } else if let Some(prev) = self.zoom_prev_effective.take() {
            let factor = self.effective_zoom / prev.max(1e-6);
            if (factor - 1.0).abs() > 1e-4 {
                let anchor = self.zoom_anchor.take().unwrap_or(avail * 0.5);
                let new_off = (self.last_offset + anchor) * factor - anchor;
                scroll = scroll
                    .vertical_scroll_offset(new_off.y.max(0.0))
                    .horizontal_scroll_offset(new_off.x.max(0.0));
            } else {
                self.zoom_anchor = None;
            }
        }
        scroll
    }

    /// Scrolling while text is being selected. egui's scroll area ignores
    /// the wheel during any drag, so the wheel is applied here; and dragging
    /// past the top or bottom edge scrolls on its own, faster the further
    /// out the pointer is. The selection follows, as the drag continues.
    fn selection_autoscroll(&mut self, ctx: &egui::Context) {
        if !self.selecting {
            return;
        }
        let (wheel, pointer, dt) = ctx.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.pointer.latest_pos(),
                i.stable_dt.min(0.05),
            )
        });
        let mut delta = -wheel;
        let view = self.central_rect;
        if let Some(pos) = pointer {
            const EDGE: f32 = 28.0;
            // Points per second: gentle at the edge, quick well past it.
            let speed = |past: f32| (past.max(0.0) * 14.0).min(2400.0);
            if pos.y > view.bottom() - EDGE {
                delta += speed(pos.y - (view.bottom() - EDGE)) * dt;
            } else if pos.y < view.top() + EDGE {
                delta -= speed(view.top() + EDGE - pos.y) * dt;
            }
        }
        if delta != 0.0 {
            self.pending_scroll += delta;
            ctx.request_repaint();
        }
    }

    fn continuous_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let avail = ui.available_size();
        let zoom = self.resolve_zoom(avail);
        let sizes_disp: Vec<(f32, f32)> = (0..self.page_count())
            .map(|i| self.display_size(i))
            .collect();
        let layout = view::layout_pages(&sizes_disp, zoom, avail.x);
        self.selection_autoscroll(&ctx);
        if let Some((was, spot)) = self.resize_anchor {
            if (was - avail).length() > 0.5 && self.scroll_to.is_none() {
                self.scroll_to = Some(spot);
            }
        }
        let scroll = self
            .scrolled(&layout, avail, true)
            .scroll_source(ScrollSource::MOUSE_WHEEL | ScrollSource::SCROLL_BAR);

        let out = scroll.show_viewport(ui, |ui, viewport| {
            ui.set_min_size(layout.content);
            let origin = ui.min_rect().min;
            let first = layout
                .rects
                .partition_point(|r| r.max.y < viewport.min.y - 300.0);
            let mut visible: Vec<i32> = Vec::new();
            let mut on_screen: Vec<(i32, egui::Rect)> = Vec::new();
            for (i, r) in layout.rects.iter().enumerate().skip(first) {
                if r.min.y > viewport.max.y + 300.0 {
                    break;
                }
                visible.push(i as i32);
                let screen = r.translate(origin.to_vec2());
                on_screen.push((i as i32, screen));
                self.paint_page(ui, i as i32, screen, ppp);
            }
            let content = egui::Rect::from_min_size(origin, layout.content);
            self.selection_ui(ui, &on_screen, content);
            visible
        });

        let visible = out.inner;
        self.last_offset = out.state.offset;
        self.central_rect = out.inner_rect;
        let top = out.state.offset.y;
        self.resize_anchor = layout
            .rects
            .iter()
            .position(|r| top < r.max.y + PAGE_GAP)
            .map(|p| {
                let r = layout.rects[p];
                let spot = ScrollTarget::Spot {
                    page: p as i32,
                    y_frac: (top - r.min.y) / r.height().max(1.0),
                    x_frac: out.state.offset.x / layout.content.x.max(1.0),
                };
                (avail, spot)
            });

        // Track which page the viewport center is on (indicator only — no
        // scroll side effects).
        if self.scroll_to.is_none() {
            let cy = out.state.offset.y + out.inner_rect.height() * 0.5;
            if let Some(p) = layout
                .rects
                .iter()
                .position(|r| cy >= r.min.y - PAGE_GAP && cy <= r.max.y + PAGE_GAP)
            {
                self.page_index = p as i32;
            }
        }

        // Evict textures for far-away pages; during a zoom gesture keep stale
        // resolutions around for stretching.
        let gesture = self.zoom_gesture_active();
        let keep: HashSet<i32> = visible
            .iter()
            .flat_map(|&p| [p - 1, p, p + 1])
            .filter(|&p| p >= 0)
            .collect();
        let rects = &layout.rects;
        let rot_now = self.rotation % 4;
        self.cache.retain(|p, px, r| {
            if !keep.contains(&p) || r != rot_now {
                return false;
            }
            if gesture {
                return true;
            }
            let wanted =
                ((rects[p as usize].width() * ppp).round() as i32).clamp(16, MAX_TEXTURE_DIM);
            px == wanted
        });
    }

    fn single_view(&mut self, ui: &mut egui::Ui) {
        // Single pages re-fit on their own; the continuous view's anchor
        // would be stale when it comes back.
        self.resize_anchor = None;
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let avail = ui.available_size();
        let zoom = self.resolve_zoom(avail);
        let page = self
            .page_index
            .clamp(0, self.page_count().saturating_sub(1).max(0));
        let size_pt = self.display_size(page);
        let layout = view::layout_single(size_pt, zoom, avail);
        let scroll = self
            .scrolled(&layout, avail, false)
            .scroll_source(ScrollSource::MOUSE_WHEEL | ScrollSource::SCROLL_BAR);

        let out = scroll.show_viewport(ui, |ui, _viewport| {
            ui.set_min_size(layout.content);
            let origin = ui.min_rect().min;
            let rect = layout.rects[0].translate(origin.to_vec2());
            self.paint_page(ui, page, rect, ppp);
            let content = egui::Rect::from_min_size(origin, layout.content);
            self.selection_ui(ui, &[(page, rect)], content);
        });

        self.last_offset = out.state.offset;
        self.central_rect = out.inner_rect;

        // Sumatra-style: wheeling past the bottom (or top) flips the page.
        let count = self.page_count();
        let at_bottom = out.state.offset.y + out.inner_rect.height() >= out.content_size.y - 2.0;
        let at_top = out.state.offset.y <= 2.0;
        self.last_at_bottom = at_bottom;
        self.last_at_top = at_top;
        let cooled = self
            .last_edge_flip
            .is_none_or(|t| t.elapsed() > EDGE_FLIP_COOLDOWN);
        if cooled && self.wheel_y < -1.5 && at_bottom && page + 1 < count {
            self.page_index = page + 1;
            self.scroll_to = Some(ScrollTarget::PageTop(self.page_index));
            self.last_edge_flip = Some(Instant::now());
        } else if cooled && self.wheel_y > 1.5 && at_top && page > 0 {
            self.page_index = page - 1;
            self.scroll_to = Some(ScrollTarget::PageBottom(self.page_index));
            self.last_edge_flip = Some(Instant::now());
        }

        let gesture = self.zoom_gesture_active();
        let wanted = ((layout.rects[0].width() * ppp).round() as i32).clamp(16, MAX_TEXTURE_DIM);
        let rot_now = self.rotation % 4;
        self.cache
            .retain(|p, px, r| (p - page).abs() <= 1 && r == rot_now && (gesture || px == wanted));
    }

    fn paint_page(&mut self, ui: &mut egui::Ui, page: i32, rect: egui::Rect, ppp: f32) {
        let ctx = ui.ctx().clone();
        let mut px_w = (rect.width() * ppp).round() as i32;
        px_w = px_w.clamp(16, MAX_TEXTURE_DIM);
        let est_h = (px_w as f32 * rect.height() / rect.width().max(1.0)) as i32;
        if est_h > MAX_TEXTURE_DIM {
            px_w = (MAX_TEXTURE_DIM as f32 * rect.width() / rect.height()).floor() as i32;
        }

        let rot = self.rotation % 4;
        let gesture = self.zoom_gesture_active();
        let tex = if let Some(t) = self.cache.get(page, px_w, rot) {
            Some(t)
        } else if gesture {
            // Don't re-render mid-gesture: stretch whatever we have, and make
            // sure a frame runs after the gesture settles to render sharp.
            ctx.request_repaint_after(ZOOM_SETTLE);
            self.cache
                .any_for_page(page, rot)
                .or_else(|| self.render_now(&ctx, page, px_w))
        } else {
            self.render_now(&ctx, page, px_w)
        };

        let Some((id, tex_size)) = tex else { return };
        // Paint on the physical-pixel grid, at the texture's exact size:
        // sub-pixel offsets or stretch make bilinear sampling smear glyphs at
        // fractional zoom levels (e.g. 88 %). A stale texture stretched
        // mid-gesture intentionally fills the layout rect instead.
        let exact = (tex_size.x as i32 - px_w).abs() <= 1;
        let paint_rect = if exact {
            let min = ((rect.min.to_vec2() * ppp).round() / ppp).to_pos2();
            egui::Rect::from_min_size(min, tex_size / ppp)
        } else {
            rect
        };
        ui.painter().image(
            id,
            paint_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        #[cfg(debug_assertions)]
        if !self.announced_first_frame {
            self.announced_first_frame = true;
            eprintln!(
                "paperview: displayed page {} ({}x{} px)",
                page + 1,
                px_w,
                est_h.min(MAX_TEXTURE_DIM)
            );
        }

        // Text-selection highlight.
        let (upw, uph) = self.page_sizes[page as usize];
        if let Some(sel_rects) = self.sel_rects_for(page) {
            let s = PX_PER_PT * self.effective_zoom;
            for hr in sel_rects {
                let hr = rect_to_view(hr, rot, upw, uph);
                let r = egui::Rect::from_min_size(
                    paint_rect.min + egui::vec2(hr.x, hr.y) * s,
                    egui::vec2(hr.w, hr.h) * s,
                );
                ui.painter().rect_filled(
                    r.expand(1.0),
                    egui::CornerRadius::same(1),
                    egui::Color32::from_rgba_unmultiplied(70, 140, 255, 80),
                );
            }
        }

        // Search highlights.
        if let Some(hits) = self.search.hits_for_page(page) {
            let s = PX_PER_PT * self.effective_zoom;
            for (i, hit) in hits.iter().enumerate() {
                let is_current = self.search.is_current(page, i);
                for hr in &hit.rects {
                    let hr = rect_to_view(*hr, rot, upw, uph);
                    let r = egui::Rect::from_min_size(
                        paint_rect.min + egui::vec2(hr.x, hr.y) * s,
                        egui::vec2(hr.w, hr.h) * s,
                    );
                    let fill = if is_current {
                        egui::Color32::from_rgba_unmultiplied(255, 140, 0, 110)
                    } else {
                        egui::Color32::from_rgba_unmultiplied(255, 235, 0, 80)
                    };
                    ui.painter()
                        .rect_filled(r.expand(1.5), egui::CornerRadius::same(2), fill);
                }
            }
        }
    }

    fn render_now(
        &mut self,
        ctx: &egui::Context,
        page: i32,
        px_w: i32,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let rot = self.rotation % 4;
        let doc = self.doc.as_ref()?;
        match self.cache.render_into(ctx, doc, page, px_w, rot, "page") {
            Ok(t) => Some(t),
            Err(e) => {
                self.status = Some(e);
                None
            }
        }
    }
}

impl eframe::App for ViewerApp {
    #[cfg(debug_assertions)]
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.automate(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(debug_assertions)]
        let frame_started = Instant::now();
        let ctx = ui.ctx().clone();

        // The look follows the chosen skin and light/dark (System tracks the OS).
        let palette = theme::palette(self.skin, self.theme.is_dark(&ctx));
        if self.applied != Some(palette) {
            theme::apply(&ctx, &palette);
            w::set_pal(&ctx, palette);
            self.applied = Some(palette);
            // This frame's root Ui took its style before the change; hand it
            // the new one so the first frame already speaks the new type scale.
            ui.set_style(ctx.global_style());
        }
        self.covers.pump(&ctx);
        self.poll_folder_scan();
        // Interface size: egui scales everything, and pages re-render sharp.
        if (ctx.zoom_factor() - self.ui_scale).abs() > 0.001 {
            ctx.set_zoom_factor(self.ui_scale);
        }

        // Defer showing the window until the second frame.  On frame 1 the
        // content is painted into the backbuffer but hasn't been swapped to
        // screen yet.  Sending Visible(true) during frame 1 would briefly
        // show a blank/stale buffer.  By frame 2 the first frame's content
        // is on-screen, so the reveal is seamless.
        if !self.shown {
            if self.frame_nr > 0 {
                self.shown = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
            self.frame_nr += 1;
            ctx.request_repaint(); // ensure frame 2 runs immediately
        }

        // Remember the Win32 window handle for parenting native dialogs.
        if self.hwnd == 0 {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = handle.as_raw() {
                    self.hwnd = w.hwnd.get();
                }
            }
        }

        // Drain print-worker events.
        if let Some(rx) = &self.print_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    PrintEvent::Status(s) => self.toast = Some((s, Instant::now())),
                    PrintEvent::Done(pages) => {
                        self.toast = Some((
                            format!(
                                "Sent {pages} page{} to the printer",
                                if pages == 1 { "" } else { "s" }
                            ),
                            Instant::now(),
                        ));
                        self.printing = false;
                    }
                    PrintEvent::Error(e) => {
                        self.status = Some(format!("Print failed: {e}"));
                        self.printing = false;
                    }
                }
            }
        }

        // Drain cloud-worker events.
        let mut open_after: Option<PathBuf> = None;
        let mut relist = false;
        let mut refresh_local = false;
        if let Some(rx) = &self.cloud_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    CloudEvent::Status(s) => self.cloud_status = Some(s),
                    CloudEvent::Error(e) => {
                        self.cloud_status = Some(e);
                        self.cloud_busy = false;
                        self.cloud_progress = None;
                    }
                    CloudEvent::Files(files) => {
                        self.cloud_files = files;
                        self.cloud_listed = true;
                        self.cloud_busy = false;
                        self.cloud_status = None;
                        self.cloud_refreshed = Some(now_ts());
                        refresh_local = true;
                    }
                    CloudEvent::Progress { done, total, bps } => {
                        self.cloud_progress = Some((done, total, bps));
                    }
                    CloudEvent::Downloaded {
                        path,
                        bytes,
                        secs,
                        open,
                    } => {
                        self.cloud_busy = false;
                        self.cloud_status = None;
                        self.cloud_progress = None;
                        self.cloud_last_dl = Some((bytes as f64 / secs.max(1e-3), now_ts()));
                        refresh_local = true;
                        if open {
                            open_after = Some(path);
                        }
                    }
                    CloudEvent::UploadDone { name, bytes, secs } => {
                        self.cloud_busy = false;
                        self.cloud_status = Some(format!("Uploaded {name}"));
                        self.cloud_progress = None;
                        self.cloud_last_ul = Some((bytes as f64 / secs.max(1e-3), now_ts()));
                        // Refresh once the whole queue is done.
                        relist = self.cloud_queue.is_empty();
                    }
                    CloudEvent::Deleted(n) => {
                        self.cloud_busy = false;
                        self.cloud_status = None;
                        self.toast = Some((
                            format!(
                                "Deleted {n} file{} from your bucket",
                                if n == 1 { "" } else { "s" }
                            ),
                            Instant::now(),
                        ));
                        self.cloud_selected.clear();
                        relist = self.cloud_queue.is_empty();
                        refresh_local = true;
                    }
                    CloudEvent::Renamed { from, to } => {
                        self.cloud_busy = false;
                        self.cloud_status = None;
                        // A Recents entry for the downloaded copy follows it.
                        let (old_local, new_local) =
                            (cloud::local_path_for(&from), cloud::local_path_for(&to));
                        for r in self
                            .recents
                            .iter_mut()
                            .filter(|r| same_path(&r.path, &old_local))
                        {
                            r.path = new_local.clone();
                        }
                        if let Some(spot) = self.peek.spots.remove(&old_local) {
                            self.peek.spots.insert(new_local, spot);
                        }
                        self.toast = Some((format!("Renamed to {to}"), Instant::now()));
                        relist = self.cloud_queue.is_empty();
                        refresh_local = true;
                    }
                }
            }
        }
        if refresh_local {
            self.refresh_local_docs();
        }
        // Run the next queued op, or refresh the listing after a batch.
        if !self.cloud_busy {
            if let Some(op) = self.cloud_queue.pop_front() {
                self.start_cloud_op(&ctx, op);
            } else if relist {
                self.cloud_listed = false;
                self.start_cloud_op(&ctx, CloudOp::List);
            }
        }
        if let Some(path) = open_after {
            self.open_path(&ctx, &path);
        }

        // Advance an in-progress search scan a little each frame.
        let pumped = match &self.doc {
            Some(doc) => self.search.pump(doc),
            None => false,
        };
        if pumped {
            ctx.request_repaint();
        }
        if let Some((page, hit)) = self.search.autoselect() {
            self.page_index = page;
            self.scroll_to = Some(ScrollTarget::Hit(page, hit));
        }

        self.handle_input(&ctx);
        self.password_modal(&ctx);
        self.print_preview_modal(&ctx);
        self.cloud_dialogs(&ctx);
        self.first_run_prompt(&ctx);
        // Full screen only makes sense with something open to read.
        if self.immersive && self.doc.is_none() && self.book.is_none() {
            self.set_immersive(&ctx, false);
        }
        if !self.immersive {
            self.window_edges(&ctx);
            self.title_bar(ui);
            self.top_bar(ui);
            if self.doc.is_some() {
                self.bottom_bar(ui);
                // Always called: it slides itself open and shut.
                self.sidebar_panel(ui);
            } else if self.book.is_some() {
                self.book_bar(ui);
                self.book_sidebar(ui);
            }
        }
        self.central(ui);
        if self.immersive {
            self.immersive_controls(&ctx);
            if self.book.is_some() {
                self.save_book_progress();
            }
        }
        self.overlays(&ctx);
        #[cfg(debug_assertions)]
        if let Some(a) = &mut self.automation {
            a.record_work(frame_started.elapsed().as_secs_f32());
            // The previous frame's whole CPU time: our code plus tessellation
            // and painting, texture uploads included.
            if let Some(cpu) = frame.info().cpu_usage {
                a.record_cpu(cpu);
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.stash_doc_state();
        storage.set_string(
            "mode",
            match self.mode {
                ViewMode::Continuous => "continuous",
                ViewMode::Single => "single",
            }
            .to_string(),
        );
        storage.set_string(
            "sidebar",
            match self.sidebar {
                SidebarMode::Hidden => "hidden",
                SidebarMode::Thumbs => "thumbs",
                SidebarMode::Toc => "toc",
            }
            .to_string(),
        );
        storage.set_string("theme", self.theme.key().to_string());
        storage.set_string("skin", self.skin.key().to_string());
        storage.set_string(
            "cloud_enabled",
            if self.cloud_enabled { "1" } else { "0" }.to_string(),
        );
        storage.set_string("peek_spots", peek::encode_spots(&self.peek.spots));
        storage.set_string("peek_tips", self.peek.tips_shown.to_string());
        storage.set_string("folders", folders::encode(&self.folders));
        self.save_book_progress();
        storage.set_string("books", books::encode(&self.books));
        storage.set_string(
            "book_spread",
            if self.book_spread { "1" } else { "0" }.to_string(),
        );
        storage.set_string(
            "default_asked",
            if self.default_asked { "1" } else { "0" }.to_string(),
        );
        storage.set_string("reading", self.reading.encode());
        storage.set_string("ui_scale", format!("{:.2}", self.ui_scale));
        storage.set_string("library_layout", self.library_layout.key().to_string());
        storage.set_string("zoom", zoom_to_str(self.zoom));
        storage.set_string("recents", encode_recents(&self.recents));
        storage.set_string("b2_key_id", self.cloud_cfg.key_id.clone());
        storage.set_string("b2_app_key", self.cloud_cfg.app_key.clone());
        storage.set_string("b2_bucket", self.cloud_cfg.bucket.clone());
        storage.set_string(
            "auto_save",
            if self.auto_save { "1" } else { "0" }.to_string(),
        );
        storage.set_string("auto_save_mb", self.auto_save_mb.to_string());
        storage.set_string(
            "open_tabs",
            self.tabs
                .iter()
                .map(|t| t.path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_path_case() {
        let a = Path::new("/home/me/Study/Notes.pdf");
        assert!(same_path(a, Path::new("/home/me/Study/Notes.pdf")));
        // Only Windows treats these as one file.
        assert_eq!(
            same_path(a, Path::new("/home/me/study/notes.pdf")),
            cfg!(windows)
        );
    }

    #[test]
    fn recents_roundtrip() {
        let list = vec![
            Recent {
                path: PathBuf::from(r"C:\docs\a.pdf"),
                ts: 1_700_000_000,
                page: 41,
                zoom: Zoom::Custom(1.5),
                rotation: 3,
            },
            Recent {
                path: PathBuf::from(r"C:\with space\b (final).pdf"),
                ts: 1,
                page: 0,
                zoom: Zoom::FitPage,
                rotation: 0,
            },
        ];
        let decoded = decode_recents(&encode_recents(&list));
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].path, list[0].path);
        assert_eq!(decoded[0].ts, 1_700_000_000);
        assert_eq!(decoded[0].page, 41);
        assert_eq!(decoded[0].rotation, 3);
        assert!(matches!(decoded[0].zoom, Zoom::Custom(z) if (z - 1.5).abs() < 1e-4));
        assert_eq!(decoded[1].path, list[1].path);
        assert!(matches!(decoded[1].zoom, Zoom::FitPage));

        // Legacy v1 lines ("ts\tpath") still parse with defaults.
        let legacy = decode_recents("123\tC:\\old.pdf");
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].page, 0);
        assert!(matches!(legacy[0].zoom, Zoom::FitPage));
    }

    #[test]
    fn rotation_transforms_round_trip() {
        use crate::pdf::HitRect;
        let (w, h) = (612.0, 792.0);
        let hr = HitRect {
            x: 72.0,
            y: 100.0,
            w: 200.0,
            h: 30.0,
        };
        for rot in 0..4u8 {
            let v = rect_to_view(hr, rot, w, h);
            // Dimensions swap on odd rotations.
            if rot % 2 == 1 {
                assert_eq!((v.w, v.h), (hr.h, hr.w), "rot {rot}");
            } else {
                assert_eq!((v.w, v.h), (hr.w, hr.h), "rot {rot}");
            }
            // The rect's center must round-trip through point_to_page.
            let center_view = egui::vec2(v.x + v.w * 0.5, v.y + v.h * 0.5);
            let center_page = point_to_page(center_view, rot, w, h);
            assert!(
                (center_page.x - (hr.x + hr.w * 0.5)).abs() < 0.01,
                "rot {rot} cx"
            );
            assert!(
                (center_page.y - (hr.y + hr.h * 0.5)).abs() < 0.01,
                "rot {rot} cy"
            );
            // And stay inside the rotated page bounds.
            let (vw, vh) = if rot % 2 == 1 { (h, w) } else { (w, h) };
            assert!(v.x >= 0.0 && v.x + v.w <= vw, "rot {rot} x-bounds");
            assert!(v.y >= 0.0 && v.y + v.h <= vh, "rot {rot} y-bounds");
        }
    }

    #[test]
    fn tab_index_fixup_after_remove() {
        assert_eq!(fix_active_after_remove(Some(3), 1), Some(2));
        assert_eq!(fix_active_after_remove(Some(1), 1), None);
        assert_eq!(fix_active_after_remove(Some(0), 2), Some(0));
        assert_eq!(fix_active_after_remove(None, 0), None);
    }

    #[test]
    fn tab_titles_truncate() {
        assert_eq!(truncate_title("short.pdf"), "short.pdf");
        let long = "a-very-long-document-name-indeed.pdf";
        let t = truncate_title(long);
        assert!(t.chars().count() == 23 && t.ends_with('…'));
    }

    #[test]
    fn auto_save_decision() {
        let mb = 20;
        assert!(should_auto_save(mb, 5 * 1024 * 1024, false, false));
        assert!(should_auto_save(mb, 20 * 1024 * 1024, false, false)); // inclusive
        assert!(!should_auto_save(mb, 20 * 1024 * 1024 + 1, false, false));
        assert!(!should_auto_save(mb, 1024, true, false)); // from cache
        assert!(!should_auto_save(mb, 1024, false, true)); // already saved
    }

    #[test]
    fn recents_skip_malformed() {
        assert!(decode_recents("").is_empty());
        assert!(decode_recents("no-tab-here").is_empty());
        assert!(decode_recents("notanumber\tC:\\x.pdf").is_empty());
        assert!(decode_recents("123\t").is_empty());
        assert_eq!(decode_recents("123\tC:\\ok.pdf").len(), 1);
    }
}
