//! Scripted input for testing the UI (debug builds only).
//!
//! `PAPERVIEW_AUTOMATE` holds `;`-separated steps, fed in as real egui input
//! events; `PAPERVIEW_SHOTS` is where screenshots go. Steps:
//!
//! * `wait MS` — let the app run
//! * `key [shift+|ctrl+]NAME down|up|tap` — e.g. `key Q down`, `key shift+Q tap`
//! * `move X Y` · `click X Y` · `rclick X Y` · `press X Y` · `release X Y`
//! * `type TEXT` — text input into the focused field
//! * `scroll DY` — mouse wheel, logical points (negative scrolls down)
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
}

impl Script {
    pub fn from_env() -> Option<Script> {
        let raw = std::env::var("PAPERVIEW_AUTOMATE").ok()?;
        Some(Script {
            steps: raw.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            next: 0,
            wait_until: Some(Instant::now() + Duration::from_millis(800)),
            awaiting_shot: None,
            shots: std::env::var_os("PAPERVIEW_SHOTS")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            pointer: Pos2::ZERO,
            modifiers: Modifiers::NONE,
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
        let Some(mut s) = self.automation.take() else { return };
        ctx.request_repaint();

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
            let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
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
                    let button =
                        if op == "rclick" { PointerButton::Secondary } else { PointerButton::Primary };
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
                    s.wait_until = Some(Instant::now() + Duration::from_millis(120));
                    break;
                }
                Some("type") => {
                    let text = step.split_once(' ').map(|(_, t)| t).unwrap_or("").to_string();
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
                    s.wait_until = Some(Instant::now() + Duration::from_millis(200));
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
