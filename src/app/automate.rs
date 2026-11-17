//! Scripted input for testing the UI (debug builds only).
//!
//! `PAPERVIEW_AUTOMATE` holds `;`-separated steps, fed in as real egui input
//! events; `PAPERVIEW_SHOTS` is where screenshots go. Steps:
//!
//! * `wait MS` — let the app run
//! * `key [shift+|ctrl+]NAME down|up|tap` — e.g. `key Q down`, `key shift+Q tap`
//! * `move X Y` · `click X Y` · `rclick X Y` · `press X Y` · `release X Y`
//! * `drop PATH` — drop a file or folder onto the window
//! * `type TEXT` — text input into the focused field
//! * `scroll DY [MS]` — mouse wheel, logical points (negative scrolls down), then pause
//! * `shot NAME` — save `NAME.ppm` (waits for the frame to be captured)
//! * `dump` — print view and peek state to stderr
//! * `quit`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2};

use super::ViewerApp;

pub(super) struct Script {
    steps: Vec<String>,
    next: usize,
    wait_until: Option<Instant>,
    awaiting_shot: Option<String>,
    shots: PathBuf,
    pointer: Pos2,
    modifiers: Modifiers,
    /// Frame times being recorded between `perf start` and `perf stop`.
    perf: Option<Vec<f32>>,
    /// Time spent building each frame (our code, not vsync), same window.
    work: Option<Vec<f32>>,
    cpu: Option<Vec<f32>>,
}

impl Script {
    pub fn record_cpu(&mut self, secs: f32) {
        if let Some(c) = &mut self.cpu {
            c.push(secs);
        }
    }

    pub fn record_work(&mut self, secs: f32) {
        if let Some(w) = &mut self.work {
            w.push(secs);
        }
    }
}

impl Script {
    pub fn from_env() -> Option<Script> {
        let raw = std::env::var("PAPERVIEW_AUTOMATE").ok()?;
        Some(Script {
            steps: raw
                .split(';')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            next: 0,
            wait_until: Some(Instant::now() + Duration::from_millis(800)),
            awaiting_shot: None,
            shots: std::env::var_os("PAPERVIEW_SHOTS")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            pointer: Pos2::ZERO,
            modifiers: Modifiers::NONE,
            perf: None,
            work: None,
            cpu: None,
        })
    }
}

fn parse_key(spec: &str) -> Option<(Modifiers, Key)> {
    let mut mods = Modifiers::NONE;
    let mut name = spec;
    while let Some((m, rest)) = name.split_once('+') {
        match m.to_ascii_lowercase().as_str() {
            "shift" => mods.shift = true,
            "ctrl" => {
                mods.ctrl = true;
                mods.command = true;
            }
            "alt" => mods.alt = true,
            _ => return None,
        }
        name = rest;
    }
    Some((mods, Key::from_name(name)?))
}

fn save_ppm(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.reserve(w * h * 3);
    for px in &image.pixels {
        out.extend_from_slice(&[px.r(), px.g(), px.b()]);
    }
    std::fs::write(path, out)
}

impl ViewerApp {
    pub(super) fn automate(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(mut s) = self.automation.take() else {
            return;
        };
        ctx.request_repaint();
        if let Some(times) = &mut s.perf {
            times.push(ctx.input(|i| i.unstable_dt));
        }

        if let Some(name) = s.awaiting_shot.clone() {
            for ev in &raw.events {
                if let Event::Screenshot { image, .. } = ev {
                    let path = s.shots.join(format!("{name}.ppm"));
                    match save_ppm(&path, image) {
                        Ok(()) => eprintln!("automate: shot {}", path.display()),
                        Err(e) => eprintln!("automate: shot failed: {e}"),
                    }
                    s.awaiting_shot = None;
                }
            }
        }

        while s.awaiting_shot.is_none()
            && s.wait_until.is_none_or(|t| Instant::now() >= t)
            && s.next < s.steps.len()
        {
            s.wait_until = None;
            let step = s.steps[s.next].clone();
            s.next += 1;
            let parts: Vec<&str> = step.split_whitespace().collect();
            let num = |i: usize| {
                parts
                    .get(i)
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.0)
            };
            match parts.first().copied() {
                Some("wait") => {
                    s.wait_until = Some(Instant::now() + Duration::from_millis(num(1) as u64));
                }
                Some("key") => {
                    let Some((mods, key)) = parts.get(1).and_then(|k| parse_key(k)) else {
                        eprintln!("automate: bad key step `{step}`");
                        continue;
                    };
                    let action = parts.get(2).copied().unwrap_or("tap");
                    // Like egui-winit: Ctrl+C / Ctrl+X arrive as Copy / Cut
                    // events, not as key presses.
                    if mods.command && action != "up" {
                        let clip = match key {
                            Key::C => Some(Event::Copy),
                            Key::X => Some(Event::Cut),
                            _ => None,
                        };
                        if let Some(ev) = clip {
                            raw.events.push(ev);
                            continue;
                        }
                    }
                    let mut send = |pressed: bool| {
                        raw.events.push(Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed,
                            repeat: false,
                            modifiers: mods,
                        });
                    };
                    match action {
                        "down" => {
                            s.modifiers = mods;
                            send(true);
                        }
                        "up" => {
                            send(false);
                            s.modifiers = Modifiers::NONE;
                        }
                        _ => {
                            // A tap holds its modifiers for this frame only.
                            s.modifiers = Modifiers::NONE;
                            send(true);
                            send(false);
                            // Release on a later frame, so the press is seen on its own.
                            s.wait_until = Some(Instant::now() + Duration::from_millis(60));
                        }
                    }
                    raw.modifiers = mods;
                    break;
                }
                Some(op @ ("move" | "click" | "rclick" | "press" | "release")) => {
                    let pos = Pos2::new(num(1), num(2));
                    s.pointer = pos;
                    raw.events.push(Event::PointerMoved(pos));
                    let button = if op == "rclick" {
                        PointerButton::Secondary
                    } else {
                        PointerButton::Primary
                    };
                    let mut push = |pressed| {
                        raw.events.push(Event::PointerButton {
                            pos,
                            button,
                            pressed,
                            modifiers: s.modifiers,
                        })
                    };
                    match op {
                        "click" | "rclick" => {
                            push(true);
                            push(false);
                        }
                        "press" => push(true),
                        "release" => push(false),
                        _ => {}
                    }
                    if op != "move" {
                        s.wait_until = Some(Instant::now() + Duration::from_millis(120));
                    }
                    break;
                }
                Some("drop") => {
                    let path = step.split_once(' ').map(|(_, t)| t).unwrap_or("");
                    raw.dropped_files.push(egui::DroppedFile {
                        path: Some(PathBuf::from(path)),
                        ..Default::default()
                    });
                    s.wait_until = Some(Instant::now() + Duration::from_millis(100));
                    break;
                }
                Some("type") => {
                    let text = step
                        .split_once(' ')
                        .map(|(_, t)| t)
                        .unwrap_or("")
                        .to_string();
                    raw.events.push(Event::Text(text));
                    s.wait_until = Some(Instant::now() + Duration::from_millis(80));
                    break;
                }
                Some("scroll") => {
                    raw.events.push(Event::PointerMoved(s.pointer));
                    raw.events.push(Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, num(1)),
                        modifiers: s.modifiers,
                        phase: egui::TouchPhase::Move,
                    });
                    let pause = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(200);
                    s.wait_until = Some(Instant::now() + Duration::from_millis(pause));
                    break;
                }
                Some("shot") => {
                    let name = parts.get(1).copied().unwrap_or("shot").to_string();
                    s.awaiting_shot = Some(name);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
                Some("dump") => {
                    eprintln!(
                        "automate: dump page={} zoom={:?} offset=({:.1},{:.1}) peeking={} away={} spot={:?} recents={} toast={:?}",
                        self.page_index + 1,
                        self.zoom,
                        self.last_offset.x,
                        self.last_offset.y,
                        self.peek.peeking(),
                        self.peek.away(),
                        self.peek_spot().map(|p| (p.page + 1, (p.y_frac * 1000.0).round() / 1000.0)),
                        self.recents.len(),
                        self.toast.as_ref().map(|t| t.0.clone()),
                    );
                }
                Some("cloud") => {
                    eprintln!(
                        "automate: cloud accounts={} current={:?} dir={:?} enabled={} busy={} listed={} files={} status={:?}",
                        self.cloud_accounts.len(),
                        self.current_account().map(|a| a.name.clone()),
                        self.cloud_dir,
                        self.cloud_enabled,
                        self.cloud_busy,
                        self.cloud_listed,
                        self.cloud_files.len(),
                        self.cloud_status,
                    );
                }
                Some("win") => {
                    let (max, min, rect) = ctx.input(|i| {
                        let v = i.viewport();
                        (v.maximized, v.minimized, v.inner_rect)
                    });
                    eprintln!(
                        "automate: window maximized={max:?} minimized={min:?} inner={rect:?}"
                    );
                }
                Some("zoom") => {
                    eprintln!(
                        "automate: zoom applied={:.2} setting={:.2} slider={:.2}",
                        ctx.zoom_factor(),
                        self.ui_scale,
                        self.ui_scale_draft
                    );
                }
                Some("perf") => {
                    if parts.get(1) == Some(&"start") {
                        s.perf = Some(Vec::new());
                        s.work = Some(Vec::new());
                        s.cpu = Some(Vec::new());
                    } else if let Some(mut t) = s.perf.take() {
                        if let Some(mut c) = s.cpu.take() {
                            c.sort_by(|a, b| a.total_cmp(b));
                            let n = c.len().max(1);
                            eprintln!(
                                "automate: frame-cpu avg={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms over7ms={}",
                                c.iter().sum::<f32>() / n as f32 * 1000.0,
                                c[(n * 95 / 100).min(n - 1)] * 1000.0,
                                c[(n * 99 / 100).min(n - 1)] * 1000.0,
                                c.last().copied().unwrap_or(0.0) * 1000.0,
                                c.iter().filter(|d| **d > 0.007).count(),
                            );
                        }
                        if let Some(mut w) = s.work.take() {
                            w.sort_by(|a, b| a.total_cmp(b));
                            let n = w.len().max(1);
                            eprintln!(
                                "automate: work avg={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms over7ms={}",
                                w.iter().sum::<f32>() / n as f32 * 1000.0,
                                w[(n * 95 / 100).min(n - 1)] * 1000.0,
                                w[(n * 99 / 100).min(n - 1)] * 1000.0,
                                w.last().copied().unwrap_or(0.0) * 1000.0,
                                w.iter().filter(|d| **d > 0.007).count(),
                            );
                        }
                        t.sort_by(|a, b| a.total_cmp(b));
                        let n = t.len().max(1);
                        let avg = t.iter().sum::<f32>() / n as f32;
                        let p95 = t[(n * 95 / 100).min(n - 1)];
                        let max = t.last().copied().unwrap_or(0.0);
                        let slow = t.iter().filter(|d| **d > 1.0 / 30.0).count();
                        eprintln!(
                            "automate: perf frames={n} avg={:.1}ms p95={:.1}ms max={:.1}ms over33ms={slow}",
                            avg * 1000.0,
                            p95 * 1000.0,
                            max * 1000.0
                        );
                    }
                }
                Some("dt") => {
                    eprintln!(
                        "automate: frame dt={:.3}s stable_dt={:.3}s",
                        ctx.input(|i| i.unstable_dt),
                        ctx.input(|i| i.stable_dt)
                    );
                }
                Some("layer") => {
                    eprintln!(
                        "automate: layer at {:?} = {:?}",
                        s.pointer,
                        ctx.layer_id_at(s.pointer)
                    );
                }
                Some("quit") => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                _ => eprintln!("automate: unknown step `{step}`"),
            }
        }
        // Keep modifiers held across frames while a key is down.
        if raw.modifiers.is_none() {
            raw.modifiers = s.modifiers;
        }
        self.automation = Some(s);
    }
}
