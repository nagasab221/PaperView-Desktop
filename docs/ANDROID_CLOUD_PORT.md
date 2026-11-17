# Cloud accounts: porting the desktop change to Android

The desktop app (v0.6) went from one Backblaze B2 bucket to **several cloud accounts of
five kinds**, with folders you can browse. This file lists what the Android app
(`nagasab221/Paperview`, Kotlin/Compose) needs to match: the same model and naming, so both
apps see the same files in the same places. The desktop code is the reference: `src/cloud/`
(protocols), `src/app/library.rs` (Cloud tab), `src/app/settings.rs` (accounts in Settings).

Android files that change: `cloud/B2Client.kt`, `cloud/CloudViewModel.kt`,
`store/Prefs.kt`, `settings/SettingsScreen.kt`, `landing/LandingScreen.kt`, and
`files/FileAccess.kt` (download location).

---

## 1. Account model

One account per entry, stored as a JSON list (desktop: eframe key `cloud_accounts`; Android:
a `stringPreferencesKey("cloud_accounts")`). Keep the same field names so a list could one day
be copied between devices:

```json
{
  "id": "e2bccfac0a3c0",        // stable; names the secret and the download folder
  "name": "Home NAS",           // what the reader calls it
  "kind": "Folder" | "WebDav" | "S3" | "B2" | "Ssh",
  "address": "",                // Folder: folder · WebDAV: URL incl. path · S3: endpoint URL · SSH: host
  "user": "",                   // WebDAV user (optional) · S3 access key ID · B2 key ID · SSH user
  "bucket": "",                 // S3 / B2
  "region": "",                 // S3 ("auto" for R2; empty = us-east-1)
  "folder": "",                 // folder inside the bucket (S3/B2) or on the server (SSH, default "PaperView")
  "port": "",                   // SSH
  "insecure_tls": false,        // WebDAV / S3: accept a self-signed certificate
  "plain_secret": "…"           // ONLY when no secure store was available (see §3)
}
```

Also stored: `cloud_current` (id of the account the Cloud tab shows), `cloud_dir` (folder
being shown, `""` = top) and the existing Cloud-tab on/off switch.

"Ready" (enough to try connecting): Folder → address; WebDAV → address; S3 → address + user +
bucket; B2 → user + bucket; SSH → address. A secret is required to save a new S3/B2 account
(WebDAV may have an empty password).

## 2. Migration from the single bucket

On first launch with no `cloud_accounts` key, if `b2_key_id` and `b2_bucket` are set:

1. Make a B2 account `{ name: "Backblaze B2", kind: B2, user: key id, bucket }` with a new id.
2. Put `b2_app_key` in the secure store under that id (§3).
3. Move the loose downloaded files in the cloud cache folder into `<cache>/<account id>/`, and
   update Recents / peek spots that point at them.
4. Make it current, then blank `b2_key_id`, `b2_app_key` and `b2_bucket`.

## 3. Secrets

The desktop keeps secrets in the system keyring (service `PaperView`, user = account id),
touches it only off the UI thread, and caches what it read for the session. If the keyring
refuses, it falls back to `plain_secret` and Settings says so.

Android: encrypt with an **Android Keystore** AES/GCM key, then store the ciphertext in
DataStore under `secret_<id>`. EncryptedSharedPreferences is deprecated, so don't use it.
Delete the entry when the account is removed. Editing an account shows the secret field
empty with the hint "Saved — leave empty to keep it".

## 4. Paths, folders and listings: the shared conventions

These matter for interop. Desktop and phone must agree on them.

- Paths are relative to the account's top folder, `/`-separated: `Math/Calculus.pdf`.
  S3/B2 prepend `folder` (if set) to make the key; WebDAV appends to the URL path; SSH
  appends to `folder` (relative = from the home folder, `~/` stripped).
- **Listings are one folder at a time.** Folders come first, then files, each sorted by name
  ignoring case. Keep only folders and documents (`pdf` + the ebook extensions), and hide
  names starting with `.`.
- **Folders in object stores:**
  - B2: list with `prefix = "<dir>/"` and `delimiter = "/"`; entries with `action == "folder"`
    are subfolders. *New folder* uploads an empty `<dir>/<name>/.bzEmpty` (Backblaze's own web
    UI does the same).
  - S3: ListObjectsV2 with `prefix`, `delimiter=/` and continuation tokens. `CommonPrefixes`
    are subfolders; skip keys ending in `/` (folder markers). *New folder* PUTs an empty
    `<dir>/<name>/` object.
- **Rename** stays within the folder and keeps the extension. B2 and S3 have no rename:
  copy server-side (B2 `b2_copy_file`, S3 `PUT` + `x-amz-copy-source: /bucket/<encoded key>`),
  then delete the original. WebDAV uses `MOVE` with `Destination` and `Overwrite: F`; SFTP
  uses `RENAME`.
- **Downloads** go to `<cloud cache>/<account id>/<path>`, keeping subfolders, with each part
  made safe (`/ \ :` → `_`, no `..`). Write to a hidden `.<name>.part` first, then rename it,
  so a broken transfer never looks downloaded.
- **Uploads** go into the folder being shown. Auto-save always uploads to the account's top
  folder and skips a file whose name and size are already there.

## 5. The five kinds on Android

| Kind | Desktop | Android approach |
|---|---|---|
| Folder | plain filesystem, files **open in place** | A folder picked with `ACTION_OPEN_DOCUMENT_TREE` (persist the URI permission), read with `DocumentFile`, which is already a dependency. This also covers SMB/Nextcloud/Syncthing apps that expose a documents provider. Store the tree URI in `address`. Open files in place; "Keep a copy offline" downloads. |
| WebDAV | `src/cloud/webdav.rs` | OkHttp with `Request.Builder().method("PROPFIND", body)` and `Depth: 1`. Parse `multistatus` by local name (`response/href`, `resourcetype/collection`, `getcontentlength`, `getlastmodified` RFC 1123). Hrefs can be absolute URLs or paths: decode them and skip the entry for the folder itself. Use pre-emptive Basic auth. Then `GET`/`PUT`/`DELETE`/`MKCOL`/`MOVE`. |
| S3 | `src/cloud/s3.rs` | Hand-written SigV4 (`javax.crypto.Mac` HmacSHA256). Path-style (`host/bucket/key`) unless the host ends in `amazonaws.com`, which uses virtual-hosted (`bucket.host`). Sign `host`, `x-amz-content-sha256` (the real body hash) and `x-amz-date`, plus `x-amz-copy-source` when copying. Copy the two unit tests in `s3.rs`: they are AWS's documented signatures. |
| B2 | `src/cloud/b2.rs` | Extend `B2Client.kt`: add `prefix` + `delimiter` to `b2_list_file_names`, `.bzEmpty` folders, and an optional `folder` prefix. Show the server's JSON `message` on errors. |
| SSH | `src/cloud/sftp.rs` runs the system `ssh` | Android has no `ssh` binary, so use a library, e.g. **mwiede/jsch** (`com.github.mwiede:jsch`, maintained JSch fork, small) and its `ChannelSftp`. The phone has no `~/.ssh`, so the form needs host, port, user, folder, and **either a password or a key**: generate an Ed25519 key in the app and show its public key to copy into `~/.ssh/authorized_keys`. That's the better default. Keep host keys trust-on-first-use (store the fingerprint per account; refuse a changed one). Make the folder (`mkdir -p`) on first connect, as the desktop does. |

Self-signed certificates (`insecure_tls`): give that account its own OkHttp client with a
trust-all `X509TrustManager` and hostname verifier. Never apply it globally.

Error wording the desktop uses: a 401/403 shows "<name> didn't let PaperView in", anything
else "Couldn't reach <name>", with the server's message underneath, then "Check the account
in Settings" and "Try again". A TLS certificate failure adds the hint to turn on "Trust a
self-signed certificate".

## 6. UI

**Cloud tab** (`LandingScreen.kt`):
- Header actions: *Upload…*, *New folder…*, *Refresh*.
- Under it, a breadcrumb `[kind icon] Account › Math › Topology`; every part except the
  last is tappable. On the right, when there's more than one ready account, a **Switch
  account** menu (icon + name for each).
- Groups: **Folders** (tap to enter), **On this computer** (downloaded; "Copies kept
  offline" for Folder accounts), **In <account name>** (not downloaded).
- Row subtitle: "Click to download · 2 KB · 17 Nov 2026" (Folder accounts: "Opens from
  <name> · …").
- Row menu: Open / Download / Download and open (Folder: Open, Show in folder, Keep a copy
  offline), Rename…, Remove download, Delete from <name>…, Select.
- Footer: "N files · size", plus "of 10 GB free tier" for B2 at the top folder only.
- The library subtitle is the account summary, e.g. `SSH · nagasab:PaperView`,
  `WebDAV · https://…`, `S3 · host/bucket/folder`, `Backblaze B2 · bucket`.
- Switching accounts resets to the top folder; the last account and folder are remembered.

**Dialogs:**
- *Save to cloud*: "It's saved in <account> › <folder> under this name…"; shows "Replace" if
  the name is taken.
- *Rename*.
- *New folder*: no `/`, no leading `.`, no duplicates.
- *Delete* ("removed from <account> for good… downloaded copies are removed too").
- *Remove account*: forgets the account, its secret and its downloads. Nothing is deleted on
  the server.

**Settings** (`SettingsScreen.kt`), section **Cloud** with an *Add account* action:
- Account rows (kind icon, name, summary, "· shown in the Cloud tab") with *Show*, *Edit*,
  *Remove*.
- The form: type chips (Folder · WebDAV · S3 · Backblaze B2 · SSH; fixed when editing), a
  one-line blurb per type, then the fields:
  - Name
  - Folder: Folder + Choose…
  - WebDAV: Server address, User name, Password, Trust self-signed
  - S3: Endpoint, Region, Bucket, Access key ID, Secret access key, Folder in the bucket,
    Trust self-signed
  - B2: Key ID, Application key ("starts with K" check), Bucket name, Folder in the bucket
  - SSH: Host, User, Port, Folder on the server, plus key/password on Android
- *Save and connect* makes the account current, turns the Cloud tab on and lists it.
- Then the existing Cloud tab switch, *Auto-save opened documents* ("…to <account>"), and
  a note on where secrets are kept.

## 7. Testing what you port

- The desktop's `examples/cloud_probe.rs` runs mkdir → upload → list → download
  (byte-compared) → rename → delete against any account. Write the same as an instrumented
  test, or a debug-only screen.
- **SSH:** the laptop (`nagasab`, `~/PaperView`) already holds a test file uploaded from the
  desktop. Browse to it from the phone.
- **B2:** the phone and desktop share a bucket. Make a folder on one and check it shows up on
  the other (the `.bzEmpty` convention).
