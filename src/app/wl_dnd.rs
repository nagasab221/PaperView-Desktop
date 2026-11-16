//! Wayland drag and drop, spoken on winit's own connection.
//!
//! winit has no drag and drop on Wayland, so PaperView does it here:
//!
//! * A tab pulled out of the bar becomes a real drag: the compositor carries
//!   a small picture of the page under the pointer, even outside the window.
//!   Dropped on another PaperView window, that window takes the tab; dropped
//!   anywhere else, it gets a window of its own.
//! * Files dragged in from a file manager open, as they do on other systems.
//!
//! Our objects live on their own event queue, dispatched by a thread of ours;
//! libwayland lets it read the socket alongside winit.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_data_source, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{
    delegate_noop, event_created_child, Connection, Dispatch, Proxy, QueueHandle, WEnum,
};

/// What a dragged tab carries: `page\npath`.
const TAB_MIME: &str = "application/x-paperview-tab";
const URI_MIME: &str = "text/uri-list";

pub(super) enum DndEvent {
    /// Another window's tab was dropped on us.
    TabDropped { path: PathBuf, page: usize },
    /// Files from another app were dropped on us.
    Files(Vec<PathBuf>),
    /// Our own tab was dropped back on our own window, here (surface pixels).
    OwnDrop { y: f32 },
    /// Our drag is over: `taken` when a window accepted the tab.
    DragEnded { taken: bool },
}

/// A picture for the drag: premultiplied BGRA (wl_shm ARGB8888), row-major.
pub(super) struct DragIcon {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl DragIcon {
    /// A page picture (straight RGBA) in a hairline frame with an accent
    /// line along the top, like the active tab's card.
    pub fn framed(
        rgba: &[u8],
        width: u32,
        height: u32,
        edge: egui::Color32,
        accent: egui::Color32,
    ) -> DragIcon {
        let (w, h) = (width + 2, height + 2);
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
                let c = if y <= 3 && x > 0 && x < w - 1 {
                    [accent.r(), accent.g(), accent.b(), 255]
                } else if border {
                    [edge.r(), edge.g(), edge.b(), 255]
                } else {
                    let i = (((y - 1) * width + (x - 1)) * 4) as usize;
                    let a = rgba[i + 3] as u32;
                    // Over white, so transparent pages look like paper.
                    let over = |v: u8| ((v as u32 * a + 255 * (255 - a)) / 255) as u8;
                    [over(rgba[i]), over(rgba[i + 1]), over(rgba[i + 2]), 255]
                };
                let o = ((y * w + x) * 4) as usize;
                bgra[o..o + 4].copy_from_slice(&[c[2], c[1], c[0], c[3]]);
            }
        }
        DragIcon {
            width: w,
            height: h,
            bgra,
        }
    }

    /// A plain card, for tabs without a page picture (books).
    pub fn card(paper: egui::Color32, edge: egui::Color32, accent: egui::Color32) -> DragIcon {
        let (w, h) = (110u32, 150u32);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|_| [paper.r(), paper.g(), paper.b(), 255])
            .collect();
        DragIcon::framed(&rgba, w, h, edge, accent)
    }
}

struct Shared {
    /// The last pointer press on our window: a drag must start from it.
    press_serial: AtomicU32,
    /// One of our tabs is being dragged right now.
    own_drag: AtomicBool,
    /// A window asked for the dragged tab's data: it's taking it.
    sent: AtomicBool,
    /// This drag's end has been reported.
    ended: AtomicBool,
}

/// Reports the end of our drag, once.
fn end_drag(shared: &Shared, tx: &Sender<DndEvent>, ctx: &egui::Context, taken: bool) {
    if !shared.ended.swap(true, Ordering::SeqCst) {
        shared.own_drag.store(false, Ordering::SeqCst);
        let _ = tx.send(DndEvent::DragEnded { taken });
        ctx.request_repaint();
    }
}

struct Drag {
    source: wl_data_source::WlDataSource,
    icon: wl_surface::WlSurface,
    buffer: wl_buffer::WlBuffer,
    pool: wl_shm_pool::WlShmPool,
    _memory: File,
}

pub(super) struct Dnd {
    conn: Connection,
    qh: QueueHandle<State>,
    shared: Arc<Shared>,
    events: Receiver<DndEvent>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    manager: wl_data_device_manager::WlDataDeviceManager,
    device: wl_data_device::WlDataDevice,
    origin: wl_surface::WlSurface,
    drag: Option<Drag>,
}

impl Dnd {
    /// Joins winit's Wayland connection; `None` elsewhere (X11, Windows).
    pub fn connect(frame: &eframe::Frame, ctx: &egui::Context) -> Option<Dnd> {
        let RawDisplayHandle::Wayland(display) = frame.display_handle().ok()?.as_raw() else {
            return None;
        };
        let RawWindowHandle::Wayland(window) = frame.window_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: winit's display outlives the app, and so this connection.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=4, ()).ok()?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).ok()?;
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ()).ok()?;
        let manager: wl_data_device_manager::WlDataDeviceManager =
            globals.bind(&qh, 3..=3, ()).ok()?;
        let device = manager.get_data_device(&seat, &qh, ());
        // SAFETY: the window's surface lives as long as the window.
        let origin_id = unsafe {
            ObjectId::from_ptr(
                wl_surface::WlSurface::interface(),
                window.surface.as_ptr().cast(),
            )
        }
        .ok()?;
        let origin = wl_surface::WlSurface::from_id(&conn, origin_id.clone()).ok()?;

        let (tx, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            press_serial: AtomicU32::new(0),
            own_drag: AtomicBool::new(false),
            sent: AtomicBool::new(false),
            ended: AtomicBool::new(false),
        });
        let mut state = State {
            conn: conn.clone(),
            ctx: ctx.clone(),
            tx,
            shared: shared.clone(),
            origin: origin_id,
            pointer: None,
            offer: None,
            offer_mime: None,
            on_origin: false,
            y: 0.0,
        };
        queue.roundtrip(&mut state).ok()?;
        std::thread::Builder::new()
            .name("wayland-dnd".into())
            .spawn(move || while queue.blocking_dispatch(&mut state).is_ok() {})
            .ok()?;
        Some(Dnd {
            conn,
            qh,
            shared,
            events,
            compositor,
            shm,
            manager,
            device,
            origin,
            drag: None,
        })
    }

    pub fn next_event(&mut self) -> Option<DndEvent> {
        let ev = self.events.try_recv().ok()?;
        if let DndEvent::DragEnded { .. } = ev {
            if let Some(d) = self.drag.take() {
                d.source.destroy();
                d.icon.destroy();
                d.buffer.destroy();
                d.pool.destroy();
                let _ = self.conn.flush();
            }
        }
        Some(ev)
    }

    /// Drops a drag the compositor never took up (egui saw the button come
    /// up, which can't happen during a real drag).
    pub fn abandon(&mut self) {
        self.shared.own_drag.store(false, Ordering::SeqCst);
        self.shared.ended.store(true, Ordering::SeqCst);
        if let Some(d) = self.drag.take() {
            d.source.destroy();
            d.icon.destroy();
            d.buffer.destroy();
            d.pool.destroy();
            let _ = self.conn.flush();
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Starts dragging a tab with `icon` under the pointer. False when the
    /// compositor can't be asked (no press to start from).
    pub fn start_tab_drag(&mut self, path: &std::path::Path, page: usize, icon: &DragIcon) -> bool {
        let serial = self.shared.press_serial.load(Ordering::SeqCst);
        if serial == 0 || self.drag.is_some() {
            return false;
        }
        let Some((memory, pool, buffer)) = self.icon_buffer(icon) else {
            return false;
        };
        let source = self
            .manager
            .create_data_source(&self.qh, SourceData(format!("{page}\n{}", path.display())));
        source.offer(TAB_MIME.into());
        source.set_actions(DndAction::Move);
        let surface = self.compositor.create_surface(&self.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, icon.width as i32, icon.height as i32);
        surface.commit();
        self.shared.own_drag.store(true, Ordering::SeqCst);
        self.shared.sent.store(false, Ordering::SeqCst);
        self.shared.ended.store(false, Ordering::SeqCst);
        self.device
            .start_drag(Some(&source), &self.origin, Some(&surface), serial);
        // Centre the picture under the pointer, a little below it.
        surface.attach(Some(&buffer), -(icon.width as i32) / 2, 10);
        surface.damage(0, 0, icon.width as i32, icon.height as i32);
        surface.commit();
        let _ = self.conn.flush();
        self.drag = Some(Drag {
            source,
            icon: surface,
            buffer,
            pool,
            _memory: memory,
        });
        true
    }

    fn icon_buffer(
        &self,
        icon: &DragIcon,
    ) -> Option<(File, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer)> {
        let size = icon.bgra.len();
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join(format!("paperview-drag-{}", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .ok()?;
        let _ = std::fs::remove_file(&path);
        file.write_all(&icon.bgra).ok()?;
        file.flush().ok()?;
        let pool = self
            .shm
            .create_pool(file.as_fd(), size as i32, &self.qh, ());
        let buffer = pool.create_buffer(
            0,
            icon.width as i32,
            icon.height as i32,
            icon.width as i32 * 4,
            wl_shm::Format::Argb8888,
            &self.qh,
            (),
        );
        Some((file, pool, buffer))
    }
}

struct SourceData(String);

#[derive(Default)]
struct OfferData {
    mimes: Mutex<Vec<String>>,
    /// The action the compositor settled on; `finish` is only allowed
    /// (without a protocol error, which would end the app) when it isn't none.
    action: AtomicU32,
}

/// Ends a dropped offer, finishing it only when that's allowed.
fn finish_offer(offer: &wl_data_offer::WlDataOffer) {
    let agreed = offer
        .data::<OfferData>()
        .is_some_and(|d| d.action.load(Ordering::SeqCst) != 0);
    if agreed {
        offer.finish();
    }
    offer.destroy();
}

struct State {
    conn: Connection,
    ctx: egui::Context,
    tx: Sender<DndEvent>,
    shared: Arc<Shared>,
    origin: ObjectId,
    pointer: Option<wl_pointer::WlPointer>,
    /// The offer over our window now, and the type we accepted from it.
    offer: Option<wl_data_offer::WlDataOffer>,
    offer_mime: Option<&'static str>,
    on_origin: bool,
    y: f64,
}

impl State {
    fn send(&self, ev: DndEvent) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_data_device_manager::WlDataDeviceManager);

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Button {
            serial,
            state: WEnum::Value(wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            state.shared.press_serial.store(serial, Ordering::SeqCst);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, SourceData> for State {
    fn event(
        state: &mut Self,
        _: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        data: &SourceData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { fd, .. } => {
                let mut out = File::from(fd);
                let _ = out.write_all(data.0.as_bytes());
                state.shared.sent.store(true, Ordering::SeqCst);
            }
            wl_data_source::Event::DndFinished => {
                end_drag(&state.shared, &state.tx, &state.ctx, true);
            }
            wl_data_source::Event::Cancelled => {
                if state.shared.sent.load(Ordering::SeqCst) {
                    end_drag(&state.shared, &state.tx, &state.ctx, true);
                } else {
                    // Every listener in the receiving window hears the drop,
                    // and one that ignores it can cancel before the one that
                    // takes the tab has asked for it. Give that one a moment.
                    let (shared, tx, ctx) =
                        (state.shared.clone(), state.tx.clone(), state.ctx.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(400));
                        let taken = shared.sent.load(Ordering::SeqCst);
                        end_drag(&shared, &tx, &ctx, taken);
                    });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_offer::WlDataOffer, OfferData> for State {
    fn event(
        _: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        data: &OfferData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                data.mimes.lock().unwrap().push(mime_type);
            }
            wl_data_offer::Event::Action { dnd_action } => {
                let bits = match dnd_action {
                    WEnum::Value(a) => a.bits(),
                    WEnum::Unknown(_) => 0,
                };
                data.action.store(bits, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                surface,
                y,
                id,
                ..
            } => {
                if let Some(old) = state.offer.take() {
                    old.destroy();
                }
                state.on_origin = surface.id() == state.origin;
                state.y = y;
                let Some(offer) = id else { return };
                let mimes = offer
                    .data::<OfferData>()
                    .map(|d| d.mimes.lock().unwrap().clone())
                    .unwrap_or_default();
                let has = |m: &str| mimes.iter().any(|x| x == m);
                let (mime, actions, preferred) = if has(TAB_MIME) {
                    (Some(TAB_MIME), DndAction::Move, DndAction::Move)
                } else if has(URI_MIME) {
                    (
                        Some(URI_MIME),
                        DndAction::Copy | DndAction::Move,
                        DndAction::Copy,
                    )
                } else {
                    (None, DndAction::None, DndAction::None)
                };
                offer.accept(serial, mime.map(String::from));
                if offer.version() >= 3 {
                    offer.set_actions(actions, preferred);
                }
                state.offer_mime = mime;
                state.offer = Some(offer);
            }
            wl_data_device::Event::Motion { y, .. } => state.y = y,
            wl_data_device::Event::Leave => {
                if let Some(old) = state.offer.take() {
                    old.destroy();
                }
            }
            wl_data_device::Event::Drop => {
                let Some(offer) = state.offer.take() else {
                    return;
                };
                let Some(mime) = state.offer_mime.take() else {
                    offer.destroy();
                    return;
                };
                let own = state.on_origin && state.shared.own_drag.load(Ordering::SeqCst);
                if own && mime == TAB_MIME {
                    // Our own tab, back on our own window.
                    state.send(DndEvent::OwnDrop { y: state.y as f32 });
                    finish_offer(&offer);
                    return;
                }
                let Ok((mut read, write)) = UnixStream::pair() else {
                    offer.destroy();
                    return;
                };
                offer.receive(mime.into(), write.as_fd());
                let _ = state.conn.flush();
                drop(write);
                let (conn, tx, ctx) = (state.conn.clone(), state.tx.clone(), state.ctx.clone());
                // The sender may be slow (or be us): read off this thread.
                std::thread::spawn(move || {
                    let mut text = String::new();
                    let _ = read.read_to_string(&mut text);
                    if let Some(ev) = parse_drop(mime, &text) {
                        let _ = tx.send(ev);
                        ctx.request_repaint();
                    }
                    finish_offer(&offer);
                    let _ = conn.flush();
                });
            }
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }

    event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, OfferData::default()),
    ]);
}

fn parse_drop(mime: &str, text: &str) -> Option<DndEvent> {
    if mime == TAB_MIME {
        let (page, path) = text.split_once('\n')?;
        return Some(DndEvent::TabDropped {
            path: PathBuf::from(path),
            page: page.trim().parse().ok()?,
        });
    }
    let files: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        .map(|p| PathBuf::from(percent_decode(p.trim_start_matches("localhost"))))
        .collect();
    (!files.is_empty()).then_some(DndEvent::Files(files))
}

/// `%20` and friends back to bytes (URIs in a uri-list are escaped).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_payloads_parse() {
        match parse_drop(TAB_MIME, "7\n/media/x/My notes.pdf") {
            Some(DndEvent::TabDropped { path, page }) => {
                assert_eq!(page, 7);
                assert_eq!(path, PathBuf::from("/media/x/My notes.pdf"));
            }
            _ => panic!("not a tab"),
        }
    }

    #[test]
    fn uri_lists_decode() {
        let text = "file:///home/a/My%20notes.pdf\r\n# comment\r\nfile:///b/%D8%B9.epub\r\n";
        match parse_drop(URI_MIME, text) {
            Some(DndEvent::Files(f)) => {
                assert_eq!(f[0], PathBuf::from("/home/a/My notes.pdf"));
                assert_eq!(f[1], PathBuf::from("/b/ع.epub"));
            }
            _ => panic!("no files"),
        }
        assert_eq!(percent_decode("a%2"), "a%2");
    }
}
