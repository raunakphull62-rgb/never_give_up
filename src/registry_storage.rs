//! Registry storage backends: local disk + Backblaze B2 (S3-compatible).
//!
//! The registry keeps its data across restarts without a paid Render disk
//! by storing package archives and index metadata in a Backblaze B2 bucket
//! through its S3-compatible API. Local disk stays the default so dev and
//! tests behave exactly as before.
//!
//! [`Storage`] is the object-store trait both backends implement:
//! - [`LocalDiskStorage`]: the historical behavior (files under the data
//!   dir), refactored behind the trait.
//! - [`S3Storage`]: any S3-compatible endpoint (B2), signed with AWS SigV4
//!   over [`ureq`] (already a dependency) plus `sha2` and `hmac`. No AWS
//!   SDK (too heavy to compile).
//!
//! Object layout in the bucket:
//! ```text
//! packages/<name>/<version>.klangpkg   the archive
//! packages/<name>/<version>.sha256     hex checksum of the archive
//! index/<name>.json                    version list and metadata
//! ```
//! A missing or corrupt `index/<name>.json` is rebuilt from the
//! `packages/` listing (the archives + sidecars are the source of truth).
//!
//! Security: errors and logs never contain secret values (key id, app key,
//! tokens, or `Authorization` headers). Only the backend name (`local` /
//! `b2`) is logged at startup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Outcome of an atomic-ish create: `Created` when the key is new,
/// `AlreadyExistsSame` when the stored bytes are identical (idempotent
/// republish), `AlreadyExistsDifferent` on a content conflict (HTTP 409).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutOutcome {
    Created,
    AlreadyExistsSame,
    AlreadyExistsDifferent,
}

/// Minimal object-store surface the registry server needs.
///
/// Keys use `/` separators (e.g. `packages/foo/1.0.0.klangpkg`). All
/// methods reject traversal keys (`..`, absolute paths, backslashes).
/// Implementations must be `Send + Sync` (the server shares one instance
/// across connection threads).
pub trait Storage: Send + Sync {
    /// Fetch an object; `Ok(None)` when the key does not exist.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    /// Unconditional write (overwrite allowed).
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), String>;
    /// Create-if-absent with content comparison (see [`PutOutcome`]).
    ///
    /// # Race window
    /// The check-then-write is not a single atomic operation: two
    /// concurrent writers may both observe "absent" and both write.
    /// Versions are immutable, so a last-write race between *identical*
    /// bytes is harmless. A differing-bytes race is detected cheaply by
    /// re-reading after the write (the loser reports
    /// [`PutOutcome::AlreadyExistsDifferent`]); the window is tiny and
    /// only reachable by publishing different bytes under one version,
    /// which is already a 409 error.
    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<PutOutcome, String>;
    /// List all keys starting with `prefix`, sorted. An empty prefix
    /// lists everything.
    fn list(&self, prefix: &str) -> Result<Vec<String>, String>;
    /// True when the key exists.
    fn exists(&self, key: &str) -> Result<bool, String>;
}

/// Validate a storage key (shared by both backends + the server).
pub fn valid_storage_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 1024 || key.contains('\0') || key.contains('\\') {
        return false;
    }
    if key.starts_with('/') {
        return false;
    }
    for seg in key.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return false;
        }
        if seg.len() > 256 {
            return false;
        }
    }
    true
}

fn check_key(key: &str) -> Result<(), String> {
    if valid_storage_key(key) {
        Ok(())
    } else {
        Err(format!("bad storage key `{key}`"))
    }
}

// ---------------------------------------------------------------------------
// Backend selection + key layout
// ---------------------------------------------------------------------------

/// B2 connection parameters (all non-secret names; values never logged).
#[derive(Debug, Clone)]
pub struct B2Config {
    pub key_id: String,
    pub app_key: String,
    pub bucket: String,
    pub endpoint: String,
    pub region: String,
}

/// Which storage the server uses.
#[derive(Debug, Clone)]
pub enum StorageBackend {
    Local,
    B2(B2Config),
}

/// Short backend name for startup logs (`local` / `b2`). Never a secret.
pub fn backend_name(backend: &StorageBackend) -> &'static str {
    match backend {
        StorageBackend::Local => "local",
        StorageBackend::B2(_) => "b2",
    }
}

/// Backend selection from the environment (called once at startup):
/// - `KLANG_STORAGE=b2` selects B2 (case-insensitive, surrounding
///   whitespace ignored).
/// - Anything else or unset selects local disk.
///
/// B2 requires `B2_KEY_ID`, `B2_APP_KEY`, `B2_BUCKET`, `B2_ENDPOINT`
/// (e.g. `s3.us-west-004.backblazeb2.com`) and optionally `B2_REGION`
/// (default: derived from the endpoint host). A missing variable is a
/// startup error naming the variable but never printing any secret value.
pub fn backend_from_env() -> Result<StorageBackend, String> {
    let want_b2 = std::env::var("KLANG_STORAGE")
        .map(|v| v.trim().eq_ignore_ascii_case("b2"))
        .unwrap_or(false);
    if !want_b2 {
        return Ok(StorageBackend::Local);
    }
    let get = |name: &str| match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    };
    let key_id = get("B2_KEY_ID")
        .ok_or_else(|| "B2_KEY_ID is not set (required when KLANG_STORAGE=b2)".to_string())?;
    let app_key = get("B2_APP_KEY")
        .ok_or_else(|| "B2_APP_KEY is not set (required when KLANG_STORAGE=b2)".to_string())?;
    let bucket = get("B2_BUCKET")
        .ok_or_else(|| "B2_BUCKET is not set (required when KLANG_STORAGE=b2)".to_string())?;
    let endpoint = get("B2_ENDPOINT")
        .ok_or_else(|| "B2_ENDPOINT is not set (required when KLANG_STORAGE=b2)".to_string())?;
    let region = get("B2_REGION").unwrap_or_else(|| derive_region(&endpoint));
    Ok(StorageBackend::B2(B2Config {
        key_id,
        app_key,
        bucket,
        endpoint,
        region,
    }))
}

/// Derive the SigV4 region from an S3 endpoint host. B2 endpoints look
/// like `s3.<region>.backblazeb2.com`, so the region is the second label.
/// Anything unrecognized falls back to `us-east-1`.
pub fn derive_region(endpoint: &str) -> String {
    let host = endpoint
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 4 && parts[0] == "s3" && !parts[1].is_empty() {
        return parts[1].to_string();
    }
    "us-east-1".to_string()
}

// --- Key layout ------------------------------------------------------------

/// Local-disk keys preserve the historical layout (`index.json` plus
/// `<name>/<version>.kpkg`).
pub fn local_index_key() -> String {
    "index.json".to_string()
}

/// Historical archive key under local disk.
pub fn local_archive_key(name: &str, version: &str) -> String {
    format!("{name}/{version}.kpkg")
}

/// Bucket key for a package archive.
pub fn s3_archive_key(name: &str, version: &str) -> String {
    format!("packages/{name}/{version}.klangpkg")
}

/// Bucket key for an archive checksum sidecar (hex SHA-256).
pub fn s3_checksum_key(name: &str, version: &str) -> String {
    format!("packages/{name}/{version}.sha256")
}

/// Bucket key for a per-package index document.
pub fn s3_index_key(name: &str) -> String {
    format!("index/{name}.json")
}

// ---------------------------------------------------------------------------
// Local disk backend (historical behavior behind the trait)
// ---------------------------------------------------------------------------

/// File-backed storage: key `a/b` maps to `<root>/a/b`. Writes are
/// atomic (unique tmp file + rename); `put_if_absent` uses
/// create-new so concurrent creators serialize on the filesystem.
#[derive(Debug, Clone)]
pub struct LocalDiskStorage {
    root: PathBuf,
}

impl LocalDiskStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, key: &str) -> PathBuf {
        let mut p = self.root.clone();
        for seg in key.split('/') {
            p.push(seg);
        }
        p
    }

    fn tmp_path(&self, key: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut p = self.path(key);
        let fname = p
            .file_name()
            .map(|n| format!("{}.tmp-{}-{}", n.to_string_lossy(), std::process::id(), nanos))
            .unwrap_or_else(|| format!("obj.tmp-{}-{nanos}", std::process::id()));
        p.set_file_name(fname);
        p
    }

    fn walk(&self, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| format!("cannot list storage: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot list storage: {e}"))?;
            let path = entry.path();
            let ft = entry
                .file_type()
                .map_err(|e| format!("cannot list storage: {e}"))?;
            if ft.is_dir() {
                self.walk(&path, out)?;
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(&self.root)
                    .map_err(|_| "storage path escapes root".to_string())?;
                let key = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if key.ends_with(".tmp") || key.contains(".tmp-") {
                    continue; // ignore leftover write tmps
                }
                out.push(key);
            }
        }
        Ok(())
    }
}

impl Storage for LocalDiskStorage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        check_key(key)?;
        match std::fs::read(self.path(key)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read `{key}`: {e}")),
        }
    }

    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        check_key(key)?;
        let dest = self.path(key);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot store `{key}`: {e}"))?;
        }
        let tmp = self.tmp_path(key);
        std::fs::write(&tmp, bytes).map_err(|e| format!("cannot store `{key}`: {e}"))?;
        std::fs::rename(&tmp, &dest).map_err(|e| format!("cannot store `{key}`: {e}"))?;
        Ok(())
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<PutOutcome, String> {
        check_key(key)?;
        let dest = self.path(key);
        if dest.is_file() {
            let old = std::fs::read(&dest).map_err(|e| format!("cannot read `{key}`: {e}"))?;
            return Ok(if old == bytes {
                PutOutcome::AlreadyExistsSame
            } else {
                PutOutcome::AlreadyExistsDifferent
            });
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot store `{key}`: {e}"))?;
        }
        // Atomic create: exactly one concurrent writer wins.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
        {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(bytes)
                    .map_err(|e| format!("cannot store `{key}`: {e}"))?;
                f.sync_all()
                    .map_err(|e| format!("cannot store `{key}`: {e}"))?;
                Ok(PutOutcome::Created)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let old =
                    std::fs::read(&dest).map_err(|e| format!("cannot read `{key}`: {e}"))?;
                Ok(if old == bytes {
                    PutOutcome::AlreadyExistsSame
                } else {
                    PutOutcome::AlreadyExistsDifferent
                })
            }
            Err(e) => Err(format!("cannot store `{key}`: {e}")),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        if !prefix.is_empty() && !valid_storage_key(prefix.trim_end_matches('/')) {
            return Err(format!("bad storage prefix `{prefix}`"));
        }
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        self.walk(&self.root, &mut out)?;
        out.retain(|k| k.starts_with(prefix));
        out.sort();
        Ok(out)
    }

    fn exists(&self, key: &str) -> Result<bool, String> {
        check_key(key)?;
        Ok(self.path(key).is_file())
    }
}

// ---------------------------------------------------------------------------
// SigV4 (AWS Signature Version 4, service `s3`)
// ---------------------------------------------------------------------------

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: usize = 4; // 1 initial + up to 3 retries
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac takes any key size");
    mac.update(data);
    let out = mac.finalize().into_bytes();
    let mut r = [0u8; 32];
    r.copy_from_slice(&out);
    r
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}

/// RFC 3986 percent-encoding. Unreserved chars (`A-Z a-z 0-9 - _ . ~`)
/// pass through; `/` passes through only when `encode_slash` is false
/// (canonical URIs); everything else becomes uppercase `%XX`.
pub fn uri_encode(s: &str, encode_slash: bool) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~';
        if unreserved || (!encode_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Convert unix seconds to (year, month, day, hour, min, sec) in UTC
/// with no `chrono` dependency (days-to-civil, Howard Hinnant's algo).
fn ymd_hms_from_unix(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (if m <= 2 { y + 1 } else { y }) as i32;
    (
        year,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
    )
}

/// Current UTC time as `(x-amz-date, date-stamp)`, e.g.
/// `("20130524T000000Z", "20130524")`.
pub fn amz_dates_now() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    amz_dates_from_unix(secs)
}

fn amz_dates_from_unix(secs: u64) -> (String, String) {
    let (y, mo, d, h, mi, s) = ymd_hms_from_unix(secs);
    (
        format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z"),
        format!("{y:04}{mo:02}{d:02}"),
    )
}

/// A signed request's auth material (testable without the network).
#[derive(Debug, Clone)]
pub struct SigV4 {
    pub authorization: String,
    pub signed_headers: String,
    pub canonical_request: String,
    pub string_to_sign: String,
    pub signature: String,
}

/// Sign with AWS SigV4 (service `s3`).
///
/// - `method`: e.g. `GET`.
/// - `canonical_uri`: already-encoded absolute path (e.g. `/bucket/key`).
/// - `query`: raw (unencoded) query pairs; sorted + encoded here.
/// - `headers`: raw header pairs to sign (names case-insensitive);
///   sorted lowercased here.
/// - `payload_hash`: hex SHA-256 of the body, or `UNSIGNED-PAYLOAD`.
/// - `amz_date` / `date_stamp`: from [`amz_dates_now`] (explicit args so
///   the AWS published vectors verify offline).
pub fn sign_v4(
    method: &str,
    canonical_uri: &str,
    query: &[(String, String)],
    headers: &[(String, String)],
    payload_hash: &str,
    key_id: &str,
    secret: &str,
    region: &str,
    service: &str,
    amz_date: &str,
    date_stamp: &str,
) -> SigV4 {
    let mut q: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, true), uri_encode(v, true)))
        .collect();
    q.sort();
    let canonical_qs: String = q
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let mut hdrs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    hdrs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut canonical_hdrs = String::new();
    let mut signed = Vec::new();
    for (k, v) in &hdrs {
        canonical_hdrs.push_str(&format!("{k}:{v}\n"));
        signed.push(k.clone());
    }
    let signed_headers = signed.join(";");

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_qs}\n{canonical_hdrs}\n{signed_headers}\n{payload_hash}"
    );
    let scope = format!("{date_stamp}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex_bytes(canonical_request.as_bytes())
    );
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={key_id}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    SigV4 {
        authorization,
        signed_headers,
        canonical_request,
        string_to_sign,
        signature,
    }
}

// ---------------------------------------------------------------------------
// S3-compatible backend (Backblaze B2)
// ---------------------------------------------------------------------------

/// S3-compatible object store (Backblaze B2 via its S3 API).
///
/// - Path-style addressing: `https://<endpoint>/<bucket>/<key>`.
/// - `PUT` sends a computed payload SHA-256; `GET`/`HEAD`/`DELETE`/list
///   use `UNSIGNED-PAYLOAD`.
/// - Timeouts: 10 s connect, 30 s total. Transient failures (5xx,
///   transport errors) retry up to 3 times with backoff; 4xx never
///   retries.
#[derive(Debug, Clone)]
pub struct S3Storage {
    key_id: String,
    app_key: String,
    bucket: String,
    base_url: String,
    host: String,
    region: String,
}

impl S3Storage {
    /// Build from explicit parts. `endpoint` may be bare
    /// (`s3.us-west-004.backblazeb2.com`) or include a scheme
    /// (`http://127.0.0.1:9000` in tests). An empty `region` is derived
    /// from the endpoint.
    pub fn new(
        key_id: String,
        app_key: String,
        bucket: String,
        endpoint: String,
        region: String,
    ) -> Result<Self, String> {
        if bucket.is_empty() {
            return Err("S3 bucket must not be empty".to_string());
        }
        let endpoint = endpoint.trim().trim_end_matches('/').to_string();
        if endpoint.is_empty() {
            return Err("S3 endpoint must not be empty".to_string());
        }
        let base_url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            endpoint.clone()
        } else {
            format!("https://{endpoint}")
        };
        // Host header value: authority minus default ports.
        let authority = base_url
            .split("://")
            .nth(1)
            .unwrap_or(&base_url)
            .split('/')
            .next()
            .unwrap_or("");
        let default_port = if base_url.starts_with("https://") {
            "443"
        } else {
            "80"
        };
        let host = match authority.split_once(':') {
            Some((h, p)) if p == default_port => h.to_string(),
            _ => authority.to_string(),
        };
        let region = if region.trim().is_empty() {
            derive_region(&endpoint)
        } else {
            region.trim().to_string()
        };
        Ok(Self {
            key_id,
            app_key,
            bucket: bucket.clone(),
            base_url: format!("{base_url}/{bucket}"),
            host,
            region,
        })
    }

    /// Build from [`B2Config`].
    pub fn from_b2(cfg: &B2Config) -> Result<Self, String> {
        Self::new(
            cfg.key_id.clone(),
            cfg.app_key.clone(),
            cfg.bucket.clone(),
            cfg.endpoint.clone(),
            cfg.region.clone(),
        )
    }

    /// Signed `DELETE` (used only by the opt-in live test to clean up;
    /// the registry itself never deletes — no yanking in v1).
    pub fn delete_object(&self, key: &str) -> Result<(), String> {
        check_key(key)?;
        let path = self.object_path(key);
        let (amz_date, date_stamp) = amz_dates_now();
        let sig = self.sign("DELETE", &path, &[], UNSIGNED_PAYLOAD, &amz_date, &date_stamp);
        let url = format!("{}{}", self.base_url, path_or_slash(&path));
        self.request_once("delete", key, "DELETE", &url, &sig, &amz_date, None)
            .map(|_| ())
    }

    fn object_path(&self, key: &str) -> String {
        format!("/{}/{}", self.bucket, uri_encode(key, false))
    }

    fn sign(
        &self,
        method: &str,
        canonical_uri: &str,
        query: &[(String, String)],
        payload_hash: &str,
        amz_date: &str,
        date_stamp: &str,
    ) -> SigV4 {
        sign_v4(
            method,
            canonical_uri,
            query,
            &[
                ("host".to_string(), self.host.clone()),
                ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
                ("x-amz-date".to_string(), amz_date.to_string()),
            ],
            payload_hash,
            &self.key_id,
            &self.app_key,
            &self.region,
            "s3",
            amz_date,
            date_stamp,
        )
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .timeout_connect(Some(CONNECT_TIMEOUT))
                .timeout_global(Some(TOTAL_TIMEOUT))
                .build(),
        )
    }

    /// One signed HTTP round trip. Returns `(status, body)`.
    /// Transport errors are `Err` (retryable); HTTP statuses — including
    /// 4xx/5xx — are `Ok` (classified by the caller; 4xx never retried).
    fn round_trip(
        method: &str,
        url: &str,
        sig: &SigV4,
        amz_date: &str,
        payload_hash: &str,
        body: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>), String> {
        let agent = Self::agent();
        // NOTE: ureq types `PUT` (with body) differently from the
        // bodyless verbs, so the two shapes are built separately. The
        // error text below intentionally carries only the transport
        // failure, never credentials or auth headers.
        if method == "PUT" {
            let res = agent
                .put(url)
                .header("x-amz-date", amz_date)
                .header("x-amz-content-sha256", payload_hash)
                .header("Authorization", &sig.authorization)
                .header("Content-Type", "application/octet-stream")
                .send(body.unwrap_or(&[]))
                .map_err(|e| format!("s3 transport error: {e}"))?;
            let status = res.status().as_u16();
            let bytes = res
                .into_body()
                .read_to_vec()
                .map_err(|e| format!("s3 body read failed: {e}"))?;
            return Ok((status, bytes));
        }
        let req = match method {
            "GET" => agent.get(url),
            "HEAD" => agent.head(url),
            "DELETE" => agent.delete(url),
            _ => return Err(format!("unsupported S3 method {method}")),
        };
        let res = req
            .header("x-amz-date", amz_date)
            .header("x-amz-content-sha256", payload_hash)
            .header("Authorization", &sig.authorization)
            .call()
            .map_err(|e| format!("s3 transport error: {e}"))?;
        let status = res.status().as_u16();
        if method == "HEAD" {
            return Ok((status, Vec::new()));
        }
        let bytes = res
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("s3 body read failed: {e}"))?;
        Ok((status, bytes))
    }

    /// Retry wrapper: transport errors and 5xx retry with backoff
    /// (200 ms, 400 ms, 800 ms); 4xx returns immediately.
    fn request_with_retry(
        &self,
        op: &str,
        key: &str,
        method: &str,
        url: &str,
        sig: &SigV4,
        amz_date: &str,
        payload_hash: &str,
        body: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>), String> {
        let mut last = String::new();
        for attempt in 0..MAX_ATTEMPTS {
            match Self::round_trip(method, url, sig, amz_date, payload_hash, body) {
                Err(e) => {
                    last = e;
                    // Transport error: retry with backoff (not after
                    // the final attempt).
                    if attempt + 1 < MAX_ATTEMPTS {
                        std::thread::sleep(Duration::from_millis(200 << attempt));
                        continue;
                    }
                    return Err(format!("s3 {op} `{key}` failed: {last}"));
                }
                Ok((status, bytes)) => {
                    if (500..600).contains(&status) {
                        last = format!("HTTP {status}");
                        if attempt + 1 < MAX_ATTEMPTS {
                            std::thread::sleep(Duration::from_millis(200 << attempt));
                            continue;
                        }
                        return Err(format!("s3 {op} `{key}` failed: {last}"));
                    }
                    return Ok((status, bytes));
                }
            }
        }
        Err(format!("s3 {op} `{key}` failed: {last}"))
    }

    /// Non-retried single request (used by `delete_object`).
    fn request_once(
        &self,
        op: &str,
        key: &str,
        method: &str,
        url: &str,
        sig: &SigV4,
        amz_date: &str,
        body: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>), String> {
        let payload = if method == "PUT" {
            sha256_hex_bytes(body.unwrap_or(&[]))
        } else {
            UNSIGNED_PAYLOAD.to_string()
        };
        match Self::round_trip(method, url, sig, amz_date, &payload, body) {
            Err(e) => Err(format!("s3 {op} `{key}` failed: {e}")),
            Ok(v) => Ok(v),
        }
    }

    fn do_get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.object_path(key);
        let (amz_date, date_stamp) = amz_dates_now();
        let sig = self.sign("GET", &path, &[], UNSIGNED_PAYLOAD, &amz_date, &date_stamp);
        let url = format!("{}{}", self.base_url, path_or_slash(&path));
        let (status, bytes) = self.request_with_retry(
            "get",
            key,
            "GET",
            &url,
            &sig,
            &amz_date,
            UNSIGNED_PAYLOAD,
            None,
        )?;
        match status {
            200 => Ok(Some(bytes)),
            404 => Ok(None),
            s => Err(format!("s3 get `{key}` failed: HTTP {s}")),
        }
    }

    fn do_head(&self, key: &str) -> Result<bool, String> {
        let path = self.object_path(key);
        let (amz_date, date_stamp) = amz_dates_now();
        let sig = self.sign("HEAD", &path, &[], UNSIGNED_PAYLOAD, &amz_date, &date_stamp);
        let url = format!("{}{}", self.base_url, path_or_slash(&path));
        let (status, _) = self.request_with_retry(
            "head",
            key,
            "HEAD",
            &url,
            &sig,
            &amz_date,
            UNSIGNED_PAYLOAD,
            None,
        )?;
        match status {
            200 => Ok(true),
            404 => Ok(false),
            s => Err(format!("s3 head `{key}` failed: HTTP {s}")),
        }
    }

    fn do_put(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.object_path(key);
        let payload_hash = sha256_hex_bytes(bytes);
        let (amz_date, date_stamp) = amz_dates_now();
        let sig = self.sign("PUT", &path, &[], &payload_hash, &amz_date, &date_stamp);
        let url = format!("{}{}", self.base_url, path_or_slash(&path));
        let (status, _) = self.request_with_retry(
            "put",
            key,
            "PUT",
            &url,
            &sig,
            &amz_date,
            &payload_hash,
            Some(bytes),
        )?;
        match status {
            200 | 201 | 204 => Ok(()),
            s => Err(format!("s3 put `{key}` failed: HTTP {s}")),
        }
    }

    /// One ListObjectsV2 page. Returns `(keys, is_truncated, next_token)`.
    fn list_page(
        &self,
        prefix: &str,
        continuation: Option<&str>,
    ) -> Result<(Vec<String>, bool, Option<String>), String> {
        let mut query = vec![
            ("list-type".to_string(), "2".to_string()),
            ("max-keys".to_string(), "1000".to_string()),
            ("prefix".to_string(), prefix.to_string()),
        ];
        if let Some(t) = continuation {
            query.push(("continuation-token".to_string(), t.to_string()));
        }
        let path = format!("/{}/", self.bucket);
        let (amz_date, date_stamp) = amz_dates_now();
        let sig = self.sign("GET", &path, &query, UNSIGNED_PAYLOAD, &amz_date, &date_stamp);
        let qs: Vec<(String, String)> = {
            let mut q: Vec<(String, String)> = query
                .iter()
                .map(|(k, v)| (uri_encode(k, true), uri_encode(v, true)))
                .collect();
            q.sort();
            q
        };
        let qs_str = qs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("{}{}?{}", self.base_url, "/", qs_str);
        let (status, bytes) = self.request_with_retry(
            "list",
            prefix,
            "GET",
            &url,
            &sig,
            &amz_date,
            UNSIGNED_PAYLOAD,
            None,
        )?;
        if status != 200 {
            return Err(format!("s3 list `{prefix}` failed: HTTP {status}"));
        }
        let text =
            String::from_utf8(bytes).map_err(|_| "s3 list returned non-UTF8 XML".to_string())?;
        Ok(parse_list_xml(&text))
    }
}

fn path_or_slash(path: &str) -> String {
    // `object_path` always yields `/<bucket>/<key>`; strip the bucket so
    // the request path is `/<key>` under `base_url` (which already ends
    // with `/<bucket>`).
    let mut parts = path.splitn(3, '/');
    let _ = parts.next();
    let _ = parts.next();
    match parts.next() {
        Some(rest) => format!("/{rest}"),
        None => "/".to_string(),
    }
}

impl Storage for S3Storage {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        check_key(key)?;
        self.do_get(key)
    }

    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        check_key(key)?;
        self.do_put(key, bytes)
    }

    fn put_if_absent(&self, key: &str, bytes: &[u8]) -> Result<PutOutcome, String> {
        check_key(key)?;
        // HEAD first: absent -> PUT; present -> compare bytes.
        if self.do_head(key)? {
            let old = self
                .do_get(key)?
                .ok_or_else(|| format!("s3 put_if_absent `{key}` vanished after HEAD"))?;
            return Ok(if old == bytes {
                PutOutcome::AlreadyExistsSame
            } else {
                PutOutcome::AlreadyExistsDifferent
            });
        }
        self.do_put(key, bytes)?;
        // Cheap post-PUT re-check for a differing-bytes race: when a
        // concurrent writer stored different bytes after our HEAD, the
        // read-back differs and we report the conflict. (Identical-byte
        // races stay `Created`/harmless — versions are immutable.)
        match self.do_get(key)? {
            Some(back) if back == bytes => Ok(PutOutcome::Created),
            Some(_) => Ok(PutOutcome::AlreadyExistsDifferent),
            None => Err(format!("s3 put_if_absent `{key}` vanished after PUT")),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        if !prefix.is_empty() && !valid_storage_key(prefix.trim_end_matches('/')) {
            return Err(format!("bad storage prefix `{prefix}`"));
        }
        let mut out = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let (keys, truncated, next) = self.list_page(prefix, continuation.as_deref())?;
            out.extend(keys);
            if !truncated {
                break;
            }
            match next {
                Some(t) => continuation = Some(t),
                None => break,
            }
        }
        out.sort();
        Ok(out)
    }

    fn exists(&self, key: &str) -> Result<bool, String> {
        check_key(key)?;
        self.do_head(key)
    }
}

// ---------------------------------------------------------------------------
// Minimal ListObjectsV2 XML parsing (no new dependencies)
// ---------------------------------------------------------------------------

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn tag_contents<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(s) = rest.find(open.as_str()) {
        let after = &rest[s + open.len()..];
        if let Some(e) = after.find(close.as_str()) {
            out.push(&after[..e]);
            rest = &after[e + close.len()..];
        } else {
            break;
        }
    }
    out
}

/// Parse a ListBucketResult page into `(keys, is_truncated, next_token)`.
/// Unknown elements are ignored; a missing `IsTruncated` means false.
pub fn parse_list_xml(xml: &str) -> (Vec<String>, bool, Option<String>) {
    let keys: Vec<String> = tag_contents(xml, "Key").iter().map(|k| xml_unescape(k)).collect();
    let truncated = tag_contents(xml, "IsTruncated")
        .first()
        .map(|v| v.trim() == "true")
        .unwrap_or(false);
    let next = tag_contents(xml, "NextContinuationToken")
        .first()
        .map(|v| xml_unescape(v));
    (keys, truncated, next)
}

// ---------------------------------------------------------------------------
// Convenience: shared in-memory backend handle
// ---------------------------------------------------------------------------

/// Build the runtime [`Storage`] for a backend: local disk rooted at
/// `data_dir`, or B2 over HTTPS.
pub fn open_storage(
    backend: &StorageBackend,
    data_dir: &Path,
) -> Result<std::sync::Arc<dyn Storage>, String> {
    match backend {
        StorageBackend::Local => Ok(std::sync::Arc::new(LocalDiskStorage::new(
            data_dir.to_path_buf(),
        )) as std::sync::Arc<dyn Storage>),
        StorageBackend::B2(cfg) => Ok(std::sync::Arc::new(S3Storage::from_b2(cfg)?)
            as std::sync::Arc<dyn Storage>),
    }
}

/// Snapshot of the expected B2 object layout (for docs/tests/health).
pub fn layout_keys_for(name: &str, version: &str) -> HashMap<&'static str, String> {
    let mut m = HashMap::new();
    m.insert("archive", s3_archive_key(name, version));
    m.insert("checksum", s3_checksum_key(name, version));
    m.insert("index", s3_index_key(name));
    m
}
