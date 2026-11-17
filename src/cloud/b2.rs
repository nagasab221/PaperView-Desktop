//! Backblaze B2 through its native JSON API (no request signing).
//!
//! Folders are name prefixes. An empty folder is kept alive by a zero-byte
//! `.bzEmpty` file, the same convention as Backblaze's web UI.

use sha1::{Digest, Sha1};

use super::http::{self, Response};
use super::{join, percent_encode, Account, Backend, CloudFile};

pub struct B2 {
    agent: ureq::Agent,
    api_url: String,
    download_url: String,
    token: String,
    bucket_id: String,
    bucket_name: String,
    /// Folder inside the bucket ("" for the whole bucket).
    prefix: String,
}

#[derive(serde::Deserialize)]
struct AuthResp {
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "apiUrl")]
    api_url: String,
    #[serde(rename = "downloadUrl")]
    download_url: String,
    #[serde(rename = "authorizationToken")]
    token: String,
}

#[derive(serde::Deserialize)]
struct BucketsResp {
    buckets: Vec<Bucket>,
}

#[derive(serde::Deserialize)]
struct Bucket {
    #[serde(rename = "bucketId")]
    id: String,
    #[serde(rename = "bucketName")]
    name: String,
}

#[derive(serde::Deserialize)]
struct ListResp {
    files: Vec<FileInfo>,
    #[serde(rename = "nextFileName", default)]
    next_file_name: Option<String>,
}

#[derive(serde::Deserialize)]
struct FileInfo {
    #[serde(rename = "fileName")]
    name: String,
    #[serde(rename = "fileId", default)]
    file_id: Option<String>,
    #[serde(rename = "contentLength", default)]
    size: u64,
    #[serde(rename = "uploadTimestamp", default)]
    ts: i64,
    #[serde(default)]
    action: String,
}

#[derive(serde::Deserialize)]
struct UploadUrlResp {
    #[serde(rename = "uploadUrl")]
    url: String,
    #[serde(rename = "authorizationToken")]
    token: String,
}

impl B2 {
    pub fn connect(acct: &Account, secret: &str) -> Result<B2, String> {
        let agent = http::agent(false);
        let resp = agent
            .get("https://api.backblazeb2.com/b2api/v2/b2_authorize_account")
            .header(
                "Authorization",
                http::basic(acct.user.trim(), secret.trim()),
            )
            .call()
            .map_err(|e| http::net_err("Could not reach Backblaze", e))?;
        let mut resp = http::check("Backblaze didn't accept the Key ID / Application key", resp)?;
        let auth: AuthResp = http::json("Authorize", &mut resp)?;

        let mut b2 = B2 {
            agent,
            api_url: auth.api_url,
            download_url: auth.download_url,
            token: auth.token,
            bucket_id: String::new(),
            bucket_name: acct.bucket.trim().to_string(),
            prefix: acct.folder.trim().trim_matches('/').to_string(),
        };
        let body = serde_json::json!({
            "accountId": auth.account_id,
            "bucketName": b2.bucket_name,
        });
        let mut resp = b2.api("b2_list_buckets", &body, "Could not list buckets")?;
        let buckets: BucketsResp = http::json("List buckets", &mut resp)?;
        let bucket = buckets
            .buckets
            .into_iter()
            .find(|b| b.name == b2.bucket_name)
            .ok_or_else(|| {
                format!(
                    "Bucket “{}” not found (is the application key allowed to access it?)",
                    b2.bucket_name
                )
            })?;
        b2.bucket_id = bucket.id;
        Ok(b2)
    }

    fn api(&self, call: &str, body: &serde_json::Value, what: &str) -> Result<Response, String> {
        let resp = self
            .agent
            .post(format!("{}/b2api/v2/{call}", self.api_url))
            .header("Authorization", &self.token)
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| http::net_err(what, e))?;
        http::check(what, resp)
    }

    /// Bucket file name for an account path.
    fn key(&self, path: &str) -> String {
        join(&self.prefix, path)
    }

    /// Account path for a bucket file name.
    fn unkey<'a>(&self, key: &'a str) -> &'a str {
        if self.prefix.is_empty() {
            key
        } else {
            key.strip_prefix(&self.prefix)
                .map(|k| k.trim_start_matches('/'))
                .unwrap_or(key)
        }
    }

    fn upload_bytes(
        &self,
        key: &str,
        data: &[u8],
        content_type: &str,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        let sha = super::hex(&Sha1::digest(data));
        let body = serde_json::json!({ "bucketId": self.bucket_id });
        let mut resp = self.api("b2_get_upload_url", &body, "Could not get an upload URL")?;
        let up: UploadUrlResp = http::json("Get upload URL", &mut resp)?;
        let mut reader = http::Counting::new(data, progress);
        let resp = self
            .agent
            .post(&up.url)
            .header("Authorization", &up.token)
            .header("X-Bz-File-Name", percent_encode(key, true))
            .header("Content-Type", content_type)
            .header("Content-Length", data.len().to_string())
            .header("X-Bz-Content-Sha1", &sha)
            .send(ureq::SendBody::from_reader(&mut reader))
            .map_err(|e| http::net_err("Upload failed", e))?;
        http::check("Upload failed", resp).map(|_| ())
    }

    /// The id of the newest version of a file.
    fn file_id(&self, key: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "bucketId": self.bucket_id,
            "startFileName": key,
            "maxFileCount": 1,
        });
        let mut resp = self.api("b2_list_file_names", &body, "Could not look up the file")?;
        let listing: ListResp = http::json("List files", &mut resp)?;
        listing
            .files
            .into_iter()
            .find(|f| f.name == key)
            .and_then(|f| f.file_id)
            .ok_or_else(|| format!("{key} is no longer in the bucket"))
    }
}

impl Backend for B2 {
    fn list(&mut self, dir: &str) -> Result<Vec<CloudFile>, String> {
        let prefix = match self.key(dir) {
            k if k.is_empty() => k,
            k => format!("{k}/"),
        };
        let mut out = Vec::new();
        let mut start: Option<String> = None;
        loop {
            let mut body = serde_json::json!({
                "bucketId": self.bucket_id,
                "maxFileCount": 1000,
                "prefix": prefix,
                "delimiter": "/",
            });
            if let Some(sfn) = &start {
                body["startFileName"] = serde_json::json!(sfn);
            }
            let mut resp = self.api("b2_list_file_names", &body, "Could not list files")?;
            let listing: ListResp = http::json("List files", &mut resp)?;
            for f in listing.files {
                let path = self.unkey(&f.name).to_string();
                match f.action.as_str() {
                    "folder" => out.push(CloudFile::dir(path)),
                    "" | "upload" => out.push(CloudFile::file(
                        path,
                        f.size,
                        f.ts,
                        f.file_id.unwrap_or_default(),
                    )),
                    _ => {}
                }
            }
            match listing.next_file_name {
                Some(next) if out.len() < 10_000 => start = Some(next),
                _ => break,
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
        let url = format!(
            "{}/file/{}/{}",
            self.download_url,
            self.bucket_name,
            percent_encode(&self.key(path), true)
        );
        let resp = self
            .agent
            .get(&url)
            .header("Authorization", &self.token)
            .call()
            .map_err(|e| http::net_err("Download failed", e))?;
        let mut resp = http::check("Download failed", resp)?;
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
            "b2/x-auto"
        };
        self.upload_bytes(&self.key(path), data, ct, progress)
    }

    fn delete(&mut self, path: &str, id: &str) -> Result<(), String> {
        let key = self.key(path);
        let id = match id {
            "" => self.file_id(&key)?,
            id => id.to_string(),
        };
        let body = serde_json::json!({ "fileName": key, "fileId": id });
        self.api(
            "b2_delete_file_version",
            &body,
            &format!("Could not delete {path}"),
        )
        .map(|_| ())
    }

    /// B2 has no rename: copy server-side under the new name, then delete
    /// the old version.
    fn rename(&mut self, from: &str, id: &str, to: &str) -> Result<(), String> {
        let key = self.key(from);
        let id = match id {
            "" => self.file_id(&key)?,
            id => id.to_string(),
        };
        let body = serde_json::json!({
            "sourceFileId": id,
            "fileName": self.key(to),
            "metadataDirective": "COPY",
        });
        self.api(
            "b2_copy_file",
            &body,
            &format!("Could not copy {from} to {to}"),
        )?;
        self.delete(from, &id)
    }

    fn needs_folders(&self) -> bool {
        false
    }

    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        let key = join(&self.key(path), ".bzEmpty");
        self.upload_bytes(&key, &[], "application/x-bzEmpty", &mut |_| {})
    }
}
