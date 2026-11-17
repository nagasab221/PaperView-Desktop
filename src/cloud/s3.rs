//! S3-compatible storage: Cloudflare R2, Wasabi, AWS S3, MinIO, iDrive e2,
//! Backblaze's S3 endpoint… Requests are signed with AWS Signature V4.
//!
//! Folders are key prefixes; "New folder" writes an empty `name/` object,
//! as the AWS console does.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use super::http::{self, Response};
use super::{hex, join, percent_encode, xml, Account, Backend, CloudFile};

pub struct S3 {
    agent: ureq::Agent,
    scheme: String,
    /// Host (and port) requests go to — the bucket's own host when
    /// virtual-hosted.
    host: String,
    bucket: String,
    path_style: bool,
    region: String,
    access: String,
    secret: String,
    /// Folder inside the bucket ("" for the whole bucket).
    prefix: String,
}

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

impl S3 {
    pub fn connect(acct: &Account, secret: &str) -> Result<S3, String> {
        let endpoint = acct.address.trim();
        let (scheme, rest) = match endpoint.split_once("://") {
            Some((s, r)) => (s.to_ascii_lowercase(), r),
            None => ("https".to_string(), endpoint),
        };
        let host = rest.split('/').next().unwrap_or("").to_string();
        if host.is_empty() {
            return Err(
                "The endpoint needs a host, e.g. https://s3.eu-central-1.wasabisys.com".into(),
            );
        }
        let bucket = acct.bucket.trim().to_string();
        // AWS itself wants bucket.host; everything else is happiest with
        // host/bucket (MinIO and most NAS servers only do that).
        let path_style = !host.ends_with("amazonaws.com");
        let host = if path_style {
            host
        } else {
            format!("{bucket}.{host}")
        };
        let region = match acct.region.trim() {
            "" => "us-east-1".to_string(),
            r => r.to_string(),
        };
        let s3 = S3 {
            agent: http::agent(acct.insecure_tls),
            scheme,
            host,
            bucket,
            path_style,
            region,
            access: acct.user.trim().to_string(),
            secret: secret.trim().to_string(),
            prefix: acct.folder.trim().trim_matches('/').to_string(),
        };
        Ok(s3)
    }

    fn key(&self, path: &str) -> String {
        join(&self.prefix, path)
    }

    fn unkey<'a>(&self, key: &'a str) -> &'a str {
        if self.prefix.is_empty() {
            key
        } else {
            key.strip_prefix(&self.prefix)
                .map(|k| k.trim_start_matches('/'))
                .unwrap_or(key)
        }
    }

    /// The request path for a key ("" addresses the bucket itself).
    fn uri_path(&self, key: &str) -> String {
        match (self.path_style, key.is_empty()) {
            (true, true) => format!("/{}", percent_encode(&self.bucket, false)),
            (true, false) => format!(
                "/{}/{}",
                percent_encode(&self.bucket, false),
                percent_encode(key, true)
            ),
            (false, _) => format!("/{}", percent_encode(key, true)),
        }
    }

    fn request(
        &self,
        method: &str,
        key: &str,
        query: &[(&str, &str)],
        extra: &[(&str, String)],
        body: &[u8],
        progress: Option<&mut dyn FnMut(u64)>,
        what: &str,
    ) -> Result<Response, String> {
        let path = self.uri_path(key);
        let qs = canonical_query(query);
        let payload = if body.is_empty() {
            EMPTY_SHA256.to_string()
        } else {
            hex(&Sha256::digest(body))
        };
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let mut headers: Vec<(String, String)> = vec![
            ("host".into(), self.host.clone()),
            ("x-amz-content-sha256".into(), payload.clone()),
            ("x-amz-date".into(), amz_date.clone()),
        ];
        for (k, v) in extra {
            headers.push((k.to_ascii_lowercase(), v.clone()));
        }
        let auth = sign(&Signing {
            method,
            path: &path,
            query: &qs,
            headers: &headers,
            payload: &payload,
            access: &self.access,
            secret: &self.secret,
            region: &self.region,
            amz_date: &amz_date,
        });
        let url = if qs.is_empty() {
            format!("{}://{}{path}", self.scheme, self.host)
        } else {
            format!("{}://{}{path}?{qs}", self.scheme, self.host)
        };
        let mut req = ureq::http::Request::builder().method(method).uri(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("Authorization", auth);
        if method == "PUT" {
            req = req.header("Content-Length", body.len().to_string());
        }
        let resp = match progress {
            None if method != "PUT" => {
                let req = req.body(()).map_err(|e| format!("{what}: {e}"))?;
                self.agent.run(req)
            }
            Some(p) => {
                let mut reader = http::Counting::new(body, p);
                let req = req
                    .body(ureq::SendBody::from_reader(&mut reader))
                    .map_err(|e| format!("{what}: {e}"))?;
                self.agent.run(req)
            }
            None => {
                let req = req
                    .body(body.to_vec())
                    .map_err(|e| format!("{what}: {e}"))?;
                self.agent.run(req)
            }
        }
        .map_err(|e| http::net_err(what, e))?;
        http::check(what, resp)
    }
}

impl Backend for S3 {
    fn list(&mut self, dir: &str) -> Result<Vec<CloudFile>, String> {
        let prefix = match self.key(dir) {
            k if k.is_empty() => k,
            k => format!("{k}/"),
        };
        let mut out = Vec::new();
        let mut token = String::new();
        loop {
            let mut q = vec![
                ("list-type", "2"),
                ("delimiter", "/"),
                ("prefix", prefix.as_str()),
            ];
            if !token.is_empty() {
                q.push(("continuation-token", token.as_str()));
            }
            let mut resp = self.request("GET", "", &q, &[], &[], None, "Could not list files")?;
            let doc = xml::parse(&http::text(&mut resp)).map_err(|e| format!("List files: {e}"))?;
            for c in doc.all("Contents") {
                let key = c.text_of("Key");
                // The folder's own marker object.
                if key == prefix || key.ends_with('/') {
                    continue;
                }
                let ms = chrono::DateTime::parse_from_rfc3339(c.text_of("LastModified"))
                    .map(|t| t.timestamp_millis())
                    .unwrap_or(0);
                out.push(CloudFile::file(
                    self.unkey(key).to_string(),
                    c.text_of("Size").parse().unwrap_or(0),
                    ms,
                    String::new(),
                ));
            }
            for p in doc.all("CommonPrefixes") {
                out.push(CloudFile::dir(self.unkey(p.text_of("Prefix")).to_string()));
            }
            let more = doc.text_of("IsTruncated") == "true";
            token = doc.text_of("NextContinuationToken").to_string();
            if !more || token.is_empty() || out.len() >= 10_000 {
                break;
            }
        }
        Ok(out)
    }

    fn get(
        &mut self,
        path: &str,
        _size: u64,
        out: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64),
    ) -> Result<u64, String> {
        let mut resp = self.request(
            "GET",
            &self.key(path),
            &[],
            &[],
            &[],
            None,
            "Download failed",
        )?;
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
        self.request(
            "PUT",
            &self.key(path),
            &[],
            &[("content-type", ct.to_string())],
            data,
            Some(progress),
            "Upload failed",
        )
        .map(|_| ())
    }

    fn delete(&mut self, path: &str, _id: &str) -> Result<(), String> {
        self.request(
            "DELETE",
            &self.key(path),
            &[],
            &[],
            &[],
            None,
            &format!("Could not delete {path}"),
        )
        .map(|_| ())
    }

    /// S3 has no rename: copy server-side, then delete the original.
    fn rename(&mut self, from: &str, _id: &str, to: &str) -> Result<(), String> {
        let source = format!(
            "/{}/{}",
            percent_encode(&self.bucket, false),
            percent_encode(&self.key(from), true)
        );
        self.request(
            "PUT",
            &self.key(to),
            &[],
            &[("x-amz-copy-source", source)],
            &[],
            None,
            &format!("Could not copy {from} to {to}"),
        )?;
        self.delete(from, "")
    }

    fn needs_folders(&self) -> bool {
        false
    }

    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        let key = format!("{}/", self.key(path));
        self.request(
            "PUT",
            &key,
            &[],
            &[],
            &[],
            None,
            "Could not make the folder",
        )
        .map(|_| ())
    }
}

fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (percent_encode(k, false), percent_encode(v, false)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

struct Signing<'a> {
    method: &'a str,
    /// Already URI-encoded.
    path: &'a str,
    /// Already canonical (sorted, encoded).
    query: &'a str,
    /// Lower-case names.
    headers: &'a [(String, String)],
    payload: &'a str,
    access: &'a str,
    secret: &'a str,
    region: &'a str,
    /// `YYYYMMDDTHHMMSSZ`
    amz_date: &'a str,
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// The `Authorization` header for a request (AWS Signature Version 4).
fn sign(s: &Signing) -> String {
    let mut headers: Vec<(String, String)> = s
        .headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed: String = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let request = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed}\n{}",
        s.method, s.path, s.query, s.payload
    );
    let date = &s.amz_date[..8];
    let scope = format!("{date}/{}/s3/aws4_request", s.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        s.amz_date,
        hex(&Sha256::digest(request.as_bytes()))
    );
    let k = hmac(format!("AWS4{}", s.secret).as_bytes(), date.as_bytes());
    let k = hmac(&k, s.region.as_bytes());
    let k = hmac(&k, b"s3");
    let k = hmac(&k, b"aws4_request");
    let signature = hex(&hmac(&k, to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}",
        s.access
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCESS: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    fn h(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    /// AWS's documented example: GET /test.txt with a Range header.
    #[test]
    fn signs_get_object_like_aws() {
        let headers = [
            h("host", "examplebucket.s3.amazonaws.com"),
            h("range", "bytes=0-9"),
            h("x-amz-content-sha256", EMPTY_SHA256),
            h("x-amz-date", "20130524T000000Z"),
        ];
        let auth = sign(&Signing {
            method: "GET",
            path: "/test.txt",
            query: "",
            headers: &headers,
            payload: EMPTY_SHA256,
            access: ACCESS,
            secret: SECRET,
            region: "us-east-1",
            amz_date: "20130524T000000Z",
        });
        assert!(auth.ends_with(
            "Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        ));
        assert!(auth.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    /// AWS's documented example: GET /?max-keys=2&prefix=J (list objects).
    #[test]
    fn signs_list_like_aws() {
        let headers = [
            h("host", "examplebucket.s3.amazonaws.com"),
            h("x-amz-content-sha256", EMPTY_SHA256),
            h("x-amz-date", "20130524T000000Z"),
        ];
        let query = canonical_query(&[("prefix", "J"), ("max-keys", "2")]);
        assert_eq!(query, "max-keys=2&prefix=J");
        let auth = sign(&Signing {
            method: "GET",
            path: "/",
            query: &query,
            headers: &headers,
            payload: EMPTY_SHA256,
            access: ACCESS,
            secret: SECRET,
            region: "us-east-1",
            amz_date: "20130524T000000Z",
        });
        assert!(auth.ends_with(
            "Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        ));
    }
}
