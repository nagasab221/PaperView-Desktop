//! Moving tabs between PaperView windows, as browsers do.
//!
//! Every window listens on a Unix socket in `$XDG_RUNTIME_DIR/paperview/`.
//! A tab dropped outside its tab bar is offered to the other windows; the one
//! the pointer is over takes it and opens the document at the same page. When
//! nobody takes it, the tab gets a window of its own.
//!
//! The pointer test is the trick: on Wayland a window can't learn where the
//! others are, but when the drag ends the compositor sends the pointer to the
//! window underneath, so that window — and only that one — sees it hovering.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;

/// How long a window looks for the pointer before turning an offer down.
const LOOK_FOR: Duration = Duration::from_millis(350);
/// How long the offering window waits for answers.
const ANSWER_IN: Duration = Duration::from_millis(900);

/// A tab another window is offering us.
pub(super) struct Offer {
    pub path: PathBuf,
    pub page: usize,
    reply: Sender<bool>,
    since: Instant,
}

impl Offer {
    pub fn answer(self, take: bool) {
        let _ = self.reply.send(take);
    }

    pub fn expired(&self) -> bool {
        self.since.elapsed() > LOOK_FOR
    }
}

/// What became of a tab we offered.
pub(super) struct Outcome {
    pub path: PathBuf,
    pub page: usize,
    pub taken: bool,
}

pub(super) struct Handoff {
    #[cfg(unix)]
    socket: Option<PathBuf>,
    offers: Receiver<Offer>,
    outcomes_tx: Sender<Outcome>,
    outcomes: Receiver<Outcome>,
    ctx: egui::Context,
}

#[cfg(unix)]
fn socket_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("paperview")
}

impl Handoff {
    pub fn start(ctx: &egui::Context) -> Handoff {
        let (offer_tx, offers) = mpsc::channel();
        let (outcomes_tx, outcomes) = mpsc::channel();
        #[cfg(unix)]
        let socket = listen(ctx.clone(), offer_tx);
        #[cfg(not(unix))]
        drop(offer_tx);
        Handoff {
            #[cfg(unix)]
            socket,
            offers,
            outcomes_tx,
            outcomes,
            ctx: ctx.clone(),
        }
    }

    /// The next offer from another window, if one came in.
    pub fn next_offer(&self) -> Option<Offer> {
        self.offers.try_recv().ok()
    }

    /// The result of an offer we made, once it's known.
    pub fn next_outcome(&self) -> Option<Outcome> {
        self.outcomes.try_recv().ok()
    }

    /// Offers `path` to the other windows, in the background.
    pub fn offer(&self, path: &Path, page: usize) {
        let tx = self.outcomes_tx.clone();
        let ctx = self.ctx.clone();
        let path = path.to_path_buf();
        #[cfg(unix)]
        let own = self.socket.clone();
        std::thread::spawn(move || {
            #[cfg(unix)]
            let taken = ask_peers(own.as_deref(), &path, page);
            #[cfg(not(unix))]
            let taken = false;
            let _ = tx.send(Outcome { path, page, taken });
            ctx.request_repaint();
        });
    }
}

#[cfg(unix)]
impl Drop for Handoff {
    fn drop(&mut self) {
        if let Some(s) = &self.socket {
            let _ = std::fs::remove_file(s);
        }
    }
}

/// `OFFER <page> <path>` in, `YES` or `NO` out.
#[cfg(unix)]
fn listen(ctx: egui::Context, offers: Sender<Offer>) -> Option<PathBuf> {
    use std::os::unix::net::UnixListener;
    let dir = socket_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).ok()?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let offers = offers.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let Some((page, path)) = parse_offer(line.trim_end_matches('\n')) else {
                    return;
                };
                let (reply, answer) = mpsc::channel();
                let offer = Offer {
                    path,
                    page,
                    reply,
                    since: Instant::now(),
                };
                if offers.send(offer).is_err() {
                    return;
                }
                ctx.request_repaint();
                let take = answer
                    .recv_timeout(LOOK_FOR + Duration::from_millis(300))
                    .unwrap_or(false);
                let mut stream = &stream;
                let _ = stream.write_all(if take { b"YES\n" } else { b"NO\n" });
            });
        }
    });
    Some(path)
}

fn parse_offer(line: &str) -> Option<(usize, PathBuf)> {
    let rest = line.strip_prefix("OFFER ")?;
    let (page, path) = rest.split_once(' ')?;
    Some((page.parse().ok()?, PathBuf::from(path)))
}

/// Asks every other window at once; true when one of them took the tab.
#[cfg(unix)]
fn ask_peers(own: Option<&Path>, path: &Path, page: usize) -> bool {
    use std::os::unix::net::UnixStream;
    let Ok(entries) = std::fs::read_dir(socket_dir()) else {
        return false;
    };
    let msg = format!("OFFER {page} {}\n", path.display());
    let (tx, rx) = mpsc::channel();
    let mut asked = 0;
    for entry in entries.flatten() {
        let sock = entry.path();
        if sock.extension().is_none_or(|e| e != "sock") || Some(sock.as_path()) == own {
            continue;
        }
        let Ok(mut stream) = UnixStream::connect(&sock) else {
            // A window that went away without tidying up.
            let _ = std::fs::remove_file(&sock);
            continue;
        };
        asked += 1;
        let tx = tx.clone();
        let msg = msg.clone();
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(ANSWER_IN));
            let yes = stream.write_all(msg.as_bytes()).is_ok() && {
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).is_ok() && line.trim() == "YES"
            };
            let _ = tx.send(yes);
        });
    }
    drop(tx);
    let deadline = Instant::now() + ANSWER_IN + Duration::from_millis(100);
    for _ in 0..asked {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(_) => break,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_parse_with_spaces_in_paths() {
        let (page, path) = parse_offer("OFFER 12 /media/x/Study/My notes.pdf").unwrap();
        assert_eq!(page, 12);
        assert_eq!(path, PathBuf::from("/media/x/Study/My notes.pdf"));
        assert!(parse_offer("OFFER x /a.pdf").is_none());
        assert!(parse_offer("HELLO").is_none());
    }
}
