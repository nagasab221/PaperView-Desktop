//! HTTP plumbing shared by the WebDAV, S3 and B2 accounts.

use std::io::Read;

pub type Response = ureq::http::Response<ureq::Body>;

/// HTTP agent: Windows' own TLS stack (Schannel) there; rustls with built-in
/// root certificates elsewhere, so no particular OpenSSL is needed.
/// `insecure` accepts any certificate — for a NAS with a self-signed one.
/// Error statuses come back as responses so their bodies can explain them.
pub fn agent(insecure: bool) -> ureq::Agent {
    #[cfg(windows)]
    let provider = ureq::tls::TlsProvider::NativeTls;
    #[cfg(not(windows))]
    let provider = ureq::tls::TlsProvider::Rustls;
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(provider)
                .disable_verification(insecure)
                .build(),
        )
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        // A dead or unreachable server fails within seconds instead of
        // leaving "Connecting…" up forever.
        .timeout_resolve(Some(std::time::Duration::from_secs(10)))
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
        .build()
        .into()
}

/// Turns transport failures into a sentence.
pub fn net_err(what: &str, e: ureq::Error) -> String {
    let text = e.to_string();
    if text.to_ascii_lowercase().contains("certificate") {
        format!(
            "{what}: the server's certificate isn't trusted ({text}). A NAS with a \
             self-signed certificate needs “Trust a self-signed certificate” in the \
             account's settings."
        )
    } else if let Some(hint) = super::network_hint(&text) {
        format!("{what}: {text}. {hint}")
    } else {
        format!("{what}: {text}")
    }
}

pub fn text(resp: &mut Response) -> String {
    let mut s = String::new();
    let _ = resp
        .body_mut()
        .as_reader()
        .take(1 << 20)
        .read_to_string(&mut s);
    s
}

/// Passes a successful response through; otherwise reads the body for the
/// server's own explanation (B2 JSON `message`, S3 XML `<Message>`).
pub fn check(what: &str, mut resp: Response) -> Result<Response, String> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = text(&mut resp);
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .or_else(|| xml_tag(&body, "Message"))
        .or_else(|| xml_tag(&body, "s:message"))
        .filter(|m| !m.trim().is_empty());
    let reason = status.canonical_reason().unwrap_or("");
    let hint = match status.as_u16() {
        401 | 403 => " — check the user name / key and password",
        404 => " — check the address, bucket and folder",
        _ => "",
    };
    Err(match detail {
        Some(d) => format!("{what}: {} {reason}: {d}", status.as_u16()),
        None => format!("{what}: {} {reason}{hint}", status.as_u16()),
    })
}

/// The text of the first `<tag>…</tag>` (good enough for error bodies).
fn xml_tag(body: &str, tag: &str) -> Option<String> {
    let start = body.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = body[start..].find(&format!("</{tag}>"))? + start;
    Some(body[start..end].trim().to_string())
}

pub fn json<T: serde::de::DeserializeOwned>(what: &str, resp: &mut Response) -> Result<T, String> {
    let body = text(resp);
    serde_json::from_str(&body).map_err(|e| format!("{what}: unexpected response: {e}"))
}

/// Copies a response body into `out`, reporting bytes so far.
pub fn stream(
    resp: &mut Response,
    out: &mut dyn std::io::Write,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, String> {
    let mut reader = resp.body_mut().as_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| format!("Could not save download: {e}"))?;
        done += n as u64;
        progress(done);
    }
    Ok(done)
}

/// A request body that reports how much of it has been read (sent).
pub struct Counting<'a> {
    data: &'a [u8],
    pos: usize,
    progress: &'a mut dyn FnMut(u64),
}

impl<'a> Counting<'a> {
    pub fn new(data: &'a [u8], progress: &'a mut dyn FnMut(u64)) -> Self {
        Counting {
            data,
            pos: 0,
            progress,
        }
    }
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.data.len() - self.pos).min(64 * 1024);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        if n > 0 {
            (self.progress)(self.pos as u64);
        }
        Ok(n)
    }
}

/// HTTP basic credentials.
pub fn basic(user: &str, pass: &str) -> String {
    use base64::Engine as _;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
    )
}
