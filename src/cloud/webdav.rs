//! WebDAV: Nextcloud / ownCloud, Synology, QNAP, TrueNAS, OpenMediaVault,
//! Koofr, Box, rclone serve webdav… Basic authentication over HTTP(S).

use super::http::{self, Response};
use super::{percent_decode, percent_encode, xml, Account, Backend, CloudFile};

pub struct WebDav {
    agent: ureq::Agent,
    /// `scheme://host[:port]`
    origin: String,
    /// The account's folder on the server, encoded, no trailing slash.
    base_path: String,
    auth: Option<String>,
}

/// A request body and who to tell how much of it has gone.
type Upload<'a, 'b> = (&'a [u8], &'b mut dyn FnMut(u64));

const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

impl WebDav {
    pub fn connect(acct: &Account, secret: &str) -> Result<WebDav, String> {
        let url = acct.address.trim();
        let url = if url.contains("://") {
            url.to_string()
        } else {
            format!("https://{url}")
        };
        let (scheme, rest) = url.split_once("://").unwrap_or(("https", &url));
        let (host, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if host.is_empty() {
            return Err(
                "The server address needs a host, e.g. https://nas.local:5006/PaperView".into(),
            );
        }
        let user = acct.user.trim();
        Ok(WebDav {
            agent: http::agent(acct.insecure_tls),
            origin: format!("{}://{host}", scheme.to_ascii_lowercase()),
            // Keep what the reader typed, but make sure it's encoded once.
            base_path: percent_encode(&percent_decode(path.trim_end_matches('/')), true),
            auth: (!user.is_empty()).then(|| http::basic(user, secret)),
        })
    }

    /// Server path of an account path, encoded.
    fn server_path(&self, rel: &str, dir: bool) -> String {
        let rel = rel.trim_matches('/');
        let mut p = self.base_path.clone();
        if !rel.is_empty() {
            p.push('/');
            p.push_str(&percent_encode(rel, true));
        }
        if dir || p.is_empty() {
            p.push('/');
        }
        p
    }

    fn url(&self, rel: &str, dir: bool) -> String {
        format!("{}{}", self.origin, self.server_path(rel, dir))
    }

    fn send(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<Upload>,
        what: &str,
    ) -> Result<Response, String> {
        let mut req = ureq::http::Request::builder().method(method).uri(url);
        if let Some(a) = &self.auth {
            req = req.header("Authorization", a.as_str());
        }
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        let resp = match body {
            Some((data, progress)) => {
                let mut reader = http::Counting::new(data, progress);
                let req = req
                    .header("Content-Length", data.len().to_string())
                    .body(ureq::SendBody::from_reader(&mut reader))
                    .map_err(|e| format!("{what}: {e}"))?;
                self.agent.run(req)
            }
            None => {
                let req = req.body(()).map_err(|e| format!("{what}: {e}"))?;
                self.agent.run(req)
            }
        }
        .map_err(|e| http::net_err(what, e))?;
        http::check(what, resp)
    }
}

impl Backend for WebDav {
    fn list(&mut self, dir: &str) -> Result<Vec<CloudFile>, String> {
        let url = self.url(dir, true);
        let mut req = ureq::http::Request::builder()
            .method("PROPFIND")
            .uri(&url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8");
        if let Some(a) = &self.auth {
            req = req.header("Authorization", a.as_str());
        }
        let req = req
            .body(PROPFIND)
            .map_err(|e| format!("Could not list files: {e}"))?;
        let resp = self
            .agent
            .run(req)
            .map_err(|e| http::net_err("Could not reach the server", e))?;
        let mut resp = http::check("Could not list files", resp)?;
        let body = http::text(&mut resp);
        parse_listing(&body, &self.server_path(dir, true), dir)
    }

    fn get(
        &mut self,
        path: &str,
        _size: u64,
        out: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String> {
        let mut resp = self.send("GET", &self.url(path, false), &[], None, "Download failed")?;
        http::stream(&mut resp, out, progress)
    }

    fn put(
        &mut self,
        path: &str,
        data: &[u8],
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let ct = if path.to_ascii_lowercase().ends_with(".pdf") {
            "application/pdf"
        } else {
            "application/octet-stream"
        };
        self.send(
            "PUT",
            &self.url(path, false),
            &[("Content-Type", ct.to_string())],
            Some((data, progress)),
            "Upload failed",
        )
        .map(|_| ())
    }

    fn delete(&mut self, path: &str, _id: &str) -> Result<(), String> {
        self.send(
            "DELETE",
            &self.url(path, false),
            &[],
            None,
            &format!("Could not delete {path}"),
        )
        .map(|_| ())
    }

    fn rename(&mut self, from: &str, _id: &str, to: &str) -> Result<(), String> {
        self.send(
            "MOVE",
            &self.url(from, false),
            &[
                ("Destination", self.url(to, false)),
                ("Overwrite", "F".to_string()),
            ],
            None,
            "Could not rename",
        )
        .map(|_| ())
    }

    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        self.send(
            "MKCOL",
            &self.url(path, true),
            &[],
            None,
            "Could not make the folder",
        )
        .map(|_| ())
    }
}

/// Reads a PROPFIND multistatus. `listed` is the (encoded) server path that
/// was asked for — its own entry is skipped; `dir` is the account path.
fn parse_listing(body: &str, listed: &str, dir: &str) -> Result<Vec<CloudFile>, String> {
    let doc = xml::parse(body).map_err(|e| format!("Could not list files: {e}"))?;
    let norm = |href: &str| -> String {
        // Hrefs may be full URLs or paths; compare decoded paths.
        let path = match href.split_once("://") {
            Some((_, rest)) => rest.find('/').map(|i| &rest[i..]).unwrap_or("/"),
            None => href,
        };
        percent_decode(path).trim_end_matches('/').to_string()
    };
    let me = norm(listed);
    let mut out = Vec::new();
    for r in doc.all("response") {
        let href = norm(r.text_of("href"));
        if href == me || href.is_empty() {
            continue;
        }
        let name = href.rsplit('/').next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let path = super::join(dir, &name);
        let is_dir = r
            .find("resourcetype")
            .is_some_and(|t| t.child("collection").is_some());
        if is_dir {
            out.push(CloudFile::dir(path));
        } else {
            let size = r
                .find("getcontentlength")
                .and_then(|n| n.text.trim().parse().ok())
                .unwrap_or(0);
            let ms = r
                .find("getlastmodified")
                .and_then(|n| chrono::DateTime::parse_from_rfc2822(n.text.trim()).ok())
                .map(|t| t.timestamp_millis())
                .unwrap_or(0);
            out.push(CloudFile::file(path, size, ms, String::new()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nextcloud_style_listing() {
        let body = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
 <d:response><d:href>/remote.php/dav/files/me/Paper%20View/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
  <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
 <d:response><d:href>/remote.php/dav/files/me/Paper%20View/Math/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
  <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
 <d:response><d:href>https://cloud.example.com/remote.php/dav/files/me/Paper%20View/K%C3%B6nig.pdf</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>2048</d:getcontentlength>
  <d:getlastmodified>Mon, 16 Nov 2026 10:00:00 GMT</d:getlastmodified></d:prop>
  <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
</d:multistatus>"#;
        let files = parse_listing(body, "/remote.php/dav/files/me/Paper%20View/", "").unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].is_dir && files[0].path == "Math");
        assert_eq!(files[1].path, "König.pdf");
        assert_eq!(files[1].size, 2048);
        assert!(files[1].modified_ms > 0);
    }

    #[test]
    fn server_paths() {
        let acct = Account {
            address: "https://nas.local:5006/home/Paper View/".into(),
            user: "me".into(),
            ..Default::default()
        };
        let dav = WebDav::connect(&acct, "pw").unwrap();
        assert_eq!(dav.origin, "https://nas.local:5006");
        assert_eq!(dav.server_path("", true), "/home/Paper%20View/");
        assert_eq!(
            dav.server_path("Math/a b.pdf", false),
            "/home/Paper%20View/Math/a%20b.pdf"
        );
        let root = Account {
            address: "http://10.0.0.2:8080".into(),
            ..Default::default()
        };
        let dav = WebDav::connect(&root, "").unwrap();
        assert_eq!(dav.server_path("", true), "/");
        assert!(dav.auth.is_none());
    }
}
