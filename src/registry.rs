//! Package registry: hosted service + CLI client (FOUNDATION-3 Part 2B).
//!
//! A real but scoped v1: a Rust HTTP service (same toolchain, std-only
//! networking — no tokio) serving a package index plus content-addressed
//! archives, with `klang publish` / `klang add` / `klang fetch` on the
//! CLI. Integrity is SHA-256 end to end: the server stores the hash it
//! computed at publish time, the client re-verifies every download, and
//! the lockfile pins the hash so a changed registry entry fails loudly
//! instead of silently swapping code.
//!
//! Explicit v1 scope (see `docs/foundation3-part2-design.md` §B):
//! - Storage is file-backed (`index.json` + `<name>/<version>.kpkg`
//!   under the data dir, served from a Render persistent disk). No
//!   Postgres yet — documented migration path, not silent omission.
//! - No TLS in the service (localhost or TLS-terminating proxy only).
//! - Auth is a single admin bearer token (`REGISTRY_ADMIN_TOKEN`).
//!   No GitHub OAuth, no per-user accounts, no teams.
//! - No yanking/deletion, no private packages, no web UI, no version
//!   ranges (exact `name@version` pins only), no transitive registry
//!   dependencies (packages must be self-contained).
//! - Archive format is a minimal owned container (`KLANGPKG1`, NOT
//!   tar.gz — std-only, no compression deps); tar.gz is the documented
//!   migration path.
//!
//! Name-squatting policy: first-come-first-served (known limitation).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::mcp::{parse_json, Json};

// ---------------------------------------------------------------------------
// Names, versions, hashing
// ---------------------------------------------------------------------------

/// Default registry base URL (localhost v1). Overridden by
/// `--registry` / `KLANG_REGISTRY`.
pub const DEFAULT_REGISTRY: &str = "http://127.0.0.1:8765";

/// Default port for `klang-registry` when none is given.
pub const DEFAULT_PORT: u16 = 8765;

/// Package names: `[A-Za-z0-9_-]+`. The charset restriction is load-
/// bearing: names become directory/file names server-side, so `..`,
/// `/`, absolute paths, and whitespace can never smuggle traversal.
pub fn valid_pkg_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Versions: `X.Y.Z` numeric triples. Exact pins only in v1 (no ranges,
/// no `latest` in manifests — the server still reports `latest` as
/// metadata for humans).
pub fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && !version.is_empty()
        && version.len() <= 32
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// SHA-256 of bytes as lowercase hex (integrity anchor everywhere).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ---------------------------------------------------------------------------
// Archive format (v1, owned — NOT tar.gz)
// ---------------------------------------------------------------------------
//
// `KLANGPKG1` magic, u32 BE file count, then entries sorted by name:
// u16 BE name length, name bytes (UTF-8), u64 BE content length,
// content bytes. Deterministic (sorted) so re-packing an extracted
// vendor directory reproduces the identical hash for lock verification.
//
// Caps (zip-bomb defense, enforced on pack AND unpack):
const ARCHIVE_MAGIC: &[u8; 9] = b"KLANGPKG1";
const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

/// Archive-safe relative path: `*.klang` or exactly `klang.toml`, no
/// `..`, no absolute paths, no backslashes, no control characters.
fn safe_archive_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 256 || name.contains('\0') || name.contains('\\') {
        return false;
    }
    let p = Path::new(name);
    if p.is_absolute() || name.starts_with("~/") {
        return false;
    }
    if name.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return false;
    }
    name == "klang.toml" || (name.ends_with(".klang") && !name.starts_with('.'))
}

/// Pack sorted, validated entries deterministically.
pub fn pack_archive(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if files.len() > MAX_FILES {
        return Err(format!(
            "package has {} files (limit {MAX_FILES})",
            files.len()
        ));
    }
    let mut sorted = files.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen: Vec<&str> = Vec::new();
    let mut total: u64 = 0;
    for (name, content) in &sorted {
        if !safe_archive_name(name) {
            return Err(format!("unsafe file name in package: `{name}`"));
        }
        if seen.contains(&name.as_str()) {
            return Err(format!("duplicate file in package: `{name}`"));
        }
        seen.push(name);
        if content.len() as u64 > MAX_FILE_BYTES {
            return Err(format!(
                "`{name}` is {} bytes (limit {MAX_FILE_BYTES})",
                content.len()
            ));
        }
        total += content.len() as u64;
    }
    if total > MAX_TOTAL_BYTES {
        return Err(format!(
            "package is {total} bytes unpacked (limit {MAX_TOTAL_BYTES})"
        ));
    }
    let mut out = Vec::new();
    out.extend_from_slice(ARCHIVE_MAGIC);
    out.extend_from_slice(&(sorted.len() as u32).to_be_bytes());
    for (name, content) in &sorted {
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(content.len() as u64).to_be_bytes());
        out.extend_from_slice(content);
    }
    Ok(out)
}

/// Unpack with full validation (magic, truncation, caps, names).
pub fn unpack_archive(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut pos = 0;
    let take = |pos: &mut usize, n: usize| -> Result<&[u8], String> {
        if bytes.len() - *pos < n {
            return Err("archive is truncated or corrupt".to_string());
        }
        let s = &bytes[*pos..*pos + n];
        *pos += n;
        Ok(s)
    };
    if take(&mut pos, ARCHIVE_MAGIC.len())? != ARCHIVE_MAGIC {
        return Err("not a Klang package archive (bad magic)".to_string());
    }
    let count = u32::from_be_bytes(take(&mut pos, 4)?.try_into().expect("4 bytes")) as usize;
    if count > MAX_FILES {
        return Err(format!("archive claims {count} files (limit {MAX_FILES})"));
    }
    let mut out = Vec::new();
    let mut total: u64 = 0;
    let mut prev: Option<String> = None;
    for _ in 0..count {
        let nl = u16::from_be_bytes(take(&mut pos, 2)?.try_into().expect("2 bytes")) as usize;
        if nl == 0 || nl > 256 {
            return Err("archive has a bad file-name length".to_string());
        }
        let name = std::str::from_utf8(take(&mut pos, nl)?)
            .map_err(|_| "archive file name is not UTF-8".to_string())?
            .to_string();
        if !safe_archive_name(&name) {
            return Err(format!("archive contains unsafe name: `{name}`"));
        }
        if let Some(p) = &prev {
            if *p >= name {
                return Err("archive entries are not sorted (repack required)".to_string());
            }
        }
        prev = Some(name.clone());
        let cl = u64::from_be_bytes(take(&mut pos, 8)?.try_into().expect("8 bytes"));
        if cl > MAX_FILE_BYTES {
            return Err(format!("archive file `{name}` exceeds size limit"));
        }
        total += cl;
        if total > MAX_TOTAL_BYTES {
            return Err("archive exceeds total size limit".to_string());
        }
        let content = take(&mut pos, cl as usize)?.to_vec();
        out.push((name, content));
    }
    if pos != bytes.len() {
        return Err("archive has trailing garbage".to_string());
    }
    Ok(out)
}

/// Collect a publishable package directory: every `*.klang` (recursive,
/// relative paths preserved) plus the root `klang.toml` as provenance
/// metadata. Skips `.klang_pkgs/`, `klang.lock`, and `target/`. Symlinks
/// are rejected (never followed — escape risk).
pub fn collect_package_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    collect_dir(dir, dir, 0, &mut out)?;
    if out.is_empty() {
        return Err("package has no .klang files".to_string());
    }
    Ok(out)
}

fn collect_dir(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    if depth > 32 {
        return Err("package directory is nested too deeply".to_string());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot read dir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read entry: {e}"))?;
        let path = entry.path();
        let ft = entry.file_type().map_err(|e| format!("cannot stat: {e}"))?;
        if ft.is_symlink() {
            return Err(format!(
                "symlinks are not packaged: `{}`",
                path.display()
            ));
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "non-UTF8 file name in package".to_string())?;
        if ft.is_dir() {
            if name == ".klang_pkgs" || name == "target" || name.starts_with('.') {
                continue;
            }
            collect_dir(root, &path, depth + 1, out)?;
            continue;
        }
        let is_manifest = depth == 0 && name == "klang.toml";
        if !(name.ends_with(".klang") || is_manifest) || name == "klang.lock" {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| "path escapes package dir".to_string())?
            .to_str()
            .ok_or_else(|| "non-UTF8 path in package".to_string())?
            .replace('\\', "/")
            .to_string();
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read `{rel}`: {e}"))?;
        out.push((rel, bytes));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Every registry failure as data (CLI renders it; the loader maps it to
/// `E-IMPORT` diagnostics).
#[derive(Debug, Clone)]
pub struct RegistryError {
    pub kind: &'static str,
    pub message: String,
}

impl RegistryError {
    pub fn new(kind: &'static str, message: String) -> Self {
        Self { kind, message }
    }

    fn network(message: String) -> Self {
        Self::new("network", message)
    }

    fn protocol(message: String) -> Self {
        Self::new("protocol", message)
    }
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "registry {}: {}", self.kind, self.message)
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct VersionEntry {
    version: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Default)]
struct Index {
    packages: HashMap<String, Vec<VersionEntry>>,
}

impl Index {
    fn latest<'a>(versions: &'a [VersionEntry]) -> Option<&'a VersionEntry> {
        versions.iter().max_by(|a, b| version_cmp(&a.version, &b.version))
    }
}

fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<u64> = a.split('.').filter_map(|p| p.parse().ok()).collect();
    let pb: Vec<u64> = b.split('.').filter_map(|p| p.parse().ok()).collect();
    pa.cmp(&pb)
}

fn index_to_json(index: &Index) -> Json {
    let mut pkgs: Vec<(&String, &Vec<VersionEntry>)> = index.packages.iter().collect();
    pkgs.sort_by(|a, b| a.0.cmp(b.0));
    Json::Obj(vec![(
        "packages".to_string(),
        Json::Arr(
            pkgs.iter()
                .map(|(name, versions)| {
                    let mut vs = (*versions).clone();
                    vs.sort_by(|a, b| version_cmp(&a.version, &b.version));
                    Json::Obj(vec![
                        ("name".to_string(), Json::Str((*name).clone())),
                        (
                            "latest".to_string(),
                            Json::Str(
                                Index::latest(&vs)
                                    .map(|v| v.version.clone())
                                    .unwrap_or_default(),
                            ),
                        ),
                        (
                            "versions".to_string(),
                            Json::Arr(
                                vs.iter()
                                    .map(|v| {
                                        Json::Obj(vec![
                                            ("version".to_string(), Json::Str(v.version.clone())),
                                            ("sha256".to_string(), Json::Str(v.sha256.clone())),
                                            ("size".to_string(), Json::Int(v.size as i64)),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}

fn index_from_json(json: &Json) -> Result<Index, String> {
    let mut index = Index::default();
    let arr = json
        .get("packages")
        .and_then(|j| j.as_arr())
        .ok_or_else(|| "index.json: missing packages".to_string())?;
    for p in arr {
        let name = p
            .get("name")
            .and_then(|j| j.as_str())
            .ok_or_else(|| "index.json: package without name".to_string())?;
        if !valid_pkg_name(name) {
            return Err(format!("index.json: bad package name `{name}`"));
        }
        let mut versions = Vec::new();
        if let Some(vs) = p.get("versions").and_then(|j| j.as_arr()) {
            for v in vs {
                let version = v
                    .get("version")
                    .and_then(|j| j.as_str())
                    .ok_or_else(|| "index.json: version without number".to_string())?;
                let sha256 = v
                    .get("sha256")
                    .and_then(|j| j.as_str())
                    .ok_or_else(|| "index.json: version without sha256".to_string())?;
                if !valid_version(version) || sha256.len() != 64 {
                    return Err("index.json: bad version entry".to_string());
                }
                versions.push(VersionEntry {
                    version: version.to_string(),
                    sha256: sha256.to_string(),
                    size: 0,
                });
            }
        }
        index.packages.insert(name.to_string(), versions);
    }
    Ok(index)
}

/// Server configuration. The data dir holds `index.json` plus
/// `<name>/<version>.kpkg` archives.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub data_dir: PathBuf,
    pub admin_token: String,
}

struct Server {
    cfg: RegistryConfig,
    index: Mutex<Index>,
}

impl Server {
    fn load(cfg: &RegistryConfig) -> Result<Self, String> {
        std::fs::create_dir_all(&cfg.data_dir)
            .map_err(|e| format!("cannot create data dir: {e}"))?;
        let index_path = cfg.data_dir.join("index.json");
        let index = match std::fs::read_to_string(&index_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(e) => return Err(format!("cannot read index: {e}")),
            Ok(text) => {
                let json = parse_json(&text).map_err(|e| format!("bad index.json: {e}"))?;
                index_from_json(&json)?
            }
        };
        // Reconcile: entries whose archive is missing are dropped loudly
        // at startup (fail closed, never serve phantom metadata).
        let mut index = index;
        let mut dropped = Vec::new();
        for (name, versions) in index.packages.iter_mut() {
            versions.retain(|v| {
                let keep = archive_path(&cfg.data_dir, name, &v.version).exists();
                if !keep {
                    dropped.push(format!("{name}@{}", v.version));
                }
                keep
            });
        }
        if !dropped.is_empty() {
            eprintln!(
                "registry: dropping {} packages with missing archives: {}",
                dropped.len(),
                dropped.join(", ")
            );
        }
        index.packages.retain(|_, vs| !vs.is_empty());
        Ok(Self {
            cfg: cfg.clone(),
            index: Mutex::new(index),
        })
    }

    fn save(&self) -> Result<(), String> {
        let json = {
            let index = self.index.lock().expect("index lock");
            index_to_json(&index).render()
        };
        let tmp = self.cfg.data_dir.join("index.json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("cannot write index: {e}"))?;
        std::fs::rename(&tmp, self.cfg.data_dir.join("index.json"))
            .map_err(|e| format!("cannot commit index: {e}"))?;
        Ok(())
    }
}

fn archive_path(data_dir: &Path, name: &str, version: &str) -> PathBuf {
    data_dir.join(name).join(format!("{version}.kpkg"))
}

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn percent_decode(s: &str) -> Result<String, String> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                if i + 2 >= b.len() {
                    return Err("bad percent-encoding".to_string());
                }
                let hex = std::str::from_utf8(&b[i + 1..i + 3])
                    .map_err(|_| "bad percent-encoding".to_string())?;
                let v = u8::from_str_radix(hex, 16)
                    .map_err(|_| "bad percent-encoding".to_string())?;
                out.push(v);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "bad percent-encoding".to_string())
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("socket: {e}"))?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream
            .read(&mut tmp)
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 16 * 1024 * 1024 + 8192 {
            return Err("request too large".to_string());
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() >= 8192 && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Err("malformed request headers".to_string());
        }
    }
    let head_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "malformed request".to_string())?;
    let head = std::str::from_utf8(&buf[..head_end]).map_err(|_| "headers are not UTF-8".to_string())?;
    let mut lines = head.lines();
    let request_line = lines.next().ok_or_else(|| "empty request".to_string())?;
    let mut rl = request_line.split_whitespace();
    let method = rl.next().ok_or_else(|| "bad request line".to_string())?.to_string();
    let target = rl.next().ok_or_else(|| "bad request line".to_string())?.to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    if headers
        .get("transfer-encoding")
        .map(|v| v.to_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        return Err("chunked transfer-encoding is not supported".to_string());
    }
    let content_len: usize = headers
        .get("content-length")
        .map(|v| v.parse().unwrap_or(0))
        .unwrap_or(0);
    if content_len > (MAX_TOTAL_BYTES as usize + 1024) {
        return Err("request body too large".to_string());
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < content_len {
        let n = stream
            .read(&mut tmp)
            .map_err(|e| format!("read body: {e}"))?;
        if n == 0 {
            return Err("truncated request body".to_string());
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_len);
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => {
            let mut map = HashMap::new();
            for pair in q.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    map.insert(
                        percent_decode(k).map_err(|e| format!("bad query: {e}"))?,
                        percent_decode(v).map_err(|e| format!("bad query: {e}"))?,
                    );
                }
            }
            (p.to_string(), map)
        }
        None => (target, HashMap::new()),
    };
    Ok(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}

fn respond(stream: &mut TcpStream, status: u16, reason: &str, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn json_body(json: &Json) -> Vec<u8> {
    json.render().into_bytes()
}

fn err_json(message: &str) -> Vec<u8> {
    Json::Obj(vec![("error".to_string(), Json::Str(message.to_string()))]).render().into_bytes()
}

fn handle(server: &Server, req: Request, stream: &mut TcpStream) {
    let segs: Vec<&str> = req.path.split('/').filter(|s| !s.is_empty()).collect();
    // GET / (health) and GET /api/packages (index).
    if req.method == "GET" && segs.is_empty() {
        let body = Json::Obj(vec![
            ("ok".to_string(), Json::Bool(true)),
            ("service".to_string(), Json::Str("klang-registry".to_string())),
        ]);
        respond(stream, 200, "OK", "application/json", &json_body(&body));
        return;
    }
    if req.method == "GET" && segs == ["api", "packages"] {
        let body = {
            let index = server.index.lock().expect("index lock");
            index_to_json(&index)
        };
        respond(stream, 200, "OK", "application/json", &json_body(&body));
        return;
    }
    // GET /api/packages/:name
    if req.method == "GET" && segs.len() == 3 && segs[0] == "api" && segs[1] == "packages" {
        let name = match percent_decode(segs[2]) {
            Ok(n) => n,
            Err(e) => {
                respond(stream, 400, "Bad Request", "application/json", &err_json(&e));
                return;
            }
        };
        if !valid_pkg_name(&name) {
            respond(stream, 400, "Bad Request", "application/json", &err_json("bad package name"));
            return;
        }
        let body = {
            let index = server.index.lock().expect("index lock");
            match index.packages.get(&name) {
                None => {
                    respond(
                        stream,
                        404,
                        "Not Found",
                        "application/json",
                        &err_json(&format!("unknown package `{name}`")),
                    );
                    return;
                }
                Some(versions) => {
                    let mut vs = versions.clone();
                    vs.sort_by(|a, b| version_cmp(&a.version, &b.version));
                    Json::Obj(vec![
                        ("name".to_string(), Json::Str(name.clone())),
                        (
                            "latest".to_string(),
                            Json::Str(
                                Index::latest(&vs).map(|v| v.version.clone()).unwrap_or_default(),
                            ),
                        ),
                        (
                            "versions".to_string(),
                            Json::Arr(
                                vs.iter()
                                    .map(|v| {
                                        Json::Obj(vec![
                                            ("version".to_string(), Json::Str(v.version.clone())),
                                            ("sha256".to_string(), Json::Str(v.sha256.clone())),
                                            ("size".to_string(), Json::Int(v.size as i64)),
                                        ])
                                    })
                                    .collect(),
                            ),
                        ),
                    ])
                }
            }
        };
        respond(stream, 200, "OK", "application/json", &json_body(&body));
        return;
    }
    // GET /api/packages/:name/:version/download
    if req.method == "GET"
        && segs.len() == 5
        && segs[0] == "api"
        && segs[1] == "packages"
        && segs[4] == "download"
    {
        let (name, version) = match (percent_decode(segs[2]), percent_decode(segs[3])) {
            (Ok(n), Ok(v)) => (n, v),
            _ => {
                respond(stream, 400, "Bad Request", "application/json", &err_json("bad path"));
                return;
            }
        };
        if !valid_pkg_name(&name) || !valid_version(&version) {
            respond(stream, 400, "Bad Request", "application/json", &err_json("bad name or version"));
            return;
        }
        let path = archive_path(&server.cfg.data_dir, &name, &version);
        match std::fs::read(&path) {
            Err(_) => respond(
                stream,
                404,
                "Not Found",
                "application/json",
                &err_json(&format!("unknown package `{name}@{version}`")),
            ),
            Ok(bytes) => respond(stream, 200, "OK", "application/octet-stream", &bytes),
        }
        return;
    }
    // POST /api/publish?name=&version= (raw archive body, bearer auth).
    if req.method == "POST" && segs == ["api", "publish"] {
        let authed = req
            .headers
            .get("authorization")
            .map(|v| v == &format!("Bearer {}", server.cfg.admin_token))
            .unwrap_or(false);
        if !authed {
            respond(stream, 401, "Unauthorized", "application/json", &err_json("bad or missing token"));
            return;
        }
        let (Some(name), Some(version)) = (req.query.get("name"), req.query.get("version")) else {
            respond(
                stream,
                400,
                "Bad Request",
                "application/json",
                &err_json("publish needs ?name= &version="),
            );
            return;
        };
        let (name, version) = (name.clone(), version.clone());
        if !valid_pkg_name(&name) || !valid_version(&version) {
            respond(stream, 400, "Bad Request", "application/json", &err_json("bad name or version"));
            return;
        }
        if unpack_archive(&req.body).is_err() {
            respond(
                stream,
                400,
                "Bad Request",
                "application/json",
                &err_json("body is not a valid package archive"),
            );
            return;
        }
        {
            let index = server.index.lock().expect("index lock");
            if index
                .packages
                .get(&name)
                .map(|vs| vs.iter().any(|v| v.version == version))
                .unwrap_or(false)
            {
                respond(
                    stream,
                    409,
                    "Conflict",
                    "application/json",
                    &err_json(&format!("`{name}@{version}` already published (no overwrite)")),
                );
                return;
            }
        }
        let dir = server.cfg.data_dir.join(&name);
        if std::fs::create_dir_all(&dir).is_err() {
            respond(stream, 500, "Server Error", "application/json", &err_json("cannot store"));
            return;
        }
        let sha256 = sha256_hex(&req.body);
        let size = req.body.len() as u64;
        if std::fs::write(archive_path(&server.cfg.data_dir, &name, &version), &req.body).is_err() {
            respond(stream, 500, "Server Error", "application/json", &err_json("cannot store"));
            return;
        }
        {
            let mut index = server.index.lock().expect("index lock");
            index
                .packages
                .entry(name.clone())
                .or_default()
                .push(VersionEntry {
                    version: version.clone(),
                    sha256: sha256.clone(),
                    size,
                });
        }
        if server.save().is_err() {
            respond(stream, 500, "Server Error", "application/json", &err_json("cannot save index"));
            return;
        }
        let body = Json::Obj(vec![
            ("published".to_string(), Json::Bool(true)),
            ("name".to_string(), Json::Str(name)),
            ("version".to_string(), Json::Str(version)),
            ("sha256".to_string(), Json::Str(sha256)),
            ("size".to_string(), Json::Int(size as i64)),
        ]);
        respond(stream, 200, "OK", "application/json", &json_body(&body));
        return;
    }
    respond(stream, 404, "Not Found", "application/json", &err_json("unknown route"));
}

fn handle_conn(server: Arc<Server>, mut stream: TcpStream) {
    match read_request(&mut stream) {
        Err(e) => respond(&mut stream, 400, "Bad Request", "application/json", &err_json(&e)),
        Ok(req) => handle(&server, req, &mut stream),
    }
}

/// Serve forever on `listener` (thread-per-connection).
pub fn serve(listener: TcpListener, cfg: RegistryConfig) -> Result<(), String> {
    let server = Arc::new(Server::load(&cfg)?);
    for stream in listener.incoming() {
        match stream {
            Err(e) => eprintln!("registry: accept: {e}"),
            Ok(stream) => {
                let server = server.clone();
                std::thread::spawn(move || handle_conn(server, stream));
            }
        }
    }
    Ok(())
}

/// Spawn a test/ephemeral server on 127.0.0.1:0; returns its base URL.
/// The accept thread is detached (process exit reaps it).
pub fn spawn_ephemeral(data_dir: &Path, admin_token: &str) -> Result<String, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("cannot bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("cannot read port: {e}"))?
        .port();
    let cfg = RegistryConfig {
        data_dir: data_dir.to_path_buf(),
        admin_token: admin_token.to_string(),
    };
    std::thread::spawn(move || {
        if let Err(e) = serve(listener, cfg) {
            eprintln!("registry: {e}");
        }
    });
    Ok(format!("http://127.0.0.1:{port}"))
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .build(),
    )
}

fn read_bytes(
    res: ureq::http::Response<ureq::Body>,
) -> Result<(u16, Vec<u8>), RegistryError> {
    let status = res.status().as_u16();
    let bytes = res
        .into_body()
        .read_to_vec()
        .map_err(|e| RegistryError::network(format!("body read failed: {e}")))?;
    Ok((status, bytes))
}

/// Package metadata from `GET /api/packages/:name`.
#[derive(Debug, Clone)]
pub struct PackageMeta {
    pub name: String,
    pub latest: String,
    pub versions: Vec<(String, String)>,
}

fn parse_meta(name: &str, text: &str) -> Result<PackageMeta, RegistryError> {
    let json = parse_json(text).map_err(|e| RegistryError::protocol(format!("bad metadata JSON: {e}")))?;
    let latest = json
        .get("latest")
        .and_then(|j| j.as_str())
        .unwrap_or_default()
        .to_string();
    let mut versions = Vec::new();
    if let Some(vs) = json.get("versions").and_then(|j| j.as_arr()) {
        for v in vs {
            let (Some(version), Some(sha)) = (
                v.get("version").and_then(|j| j.as_str()),
                v.get("sha256").and_then(|j| j.as_str()),
            ) else {
                return Err(RegistryError::protocol("bad version entry".to_string()));
            };
            versions.push((version.to_string(), sha.to_string()));
        }
    }
    Ok(PackageMeta {
        name: name.to_string(),
        latest,
        versions,
    })
}

pub fn fetch_metadata(base: &str, name: &str) -> Result<PackageMeta, RegistryError> {
    if !valid_pkg_name(name) {
        return Err(RegistryError::protocol(format!("bad package name `{name}`")));
    }
    let url = format!("{base}/api/packages/{name}");
    let res = agent()
        .get(&url)
        .call()
        .map_err(|e| RegistryError::network(format!("GET {url}: {e}")))?;
    let (status, bytes) = read_bytes(res)?;
    let text = String::from_utf8_lossy(&bytes);
    if status == 404 {
        return Err(RegistryError::new("not-found", format!("unknown package `{name}`")));
    }
    if status != 200 {
        return Err(RegistryError::protocol(format!("metadata: HTTP {status}: {text}")));
    }
    parse_meta(name, &text)
}

pub fn download(base: &str, name: &str, version: &str) -> Result<Vec<u8>, RegistryError> {
    if !valid_pkg_name(name) || !valid_version(version) {
        return Err(RegistryError::protocol("bad name or version".to_string()));
    }
    let url = format!("{base}/api/packages/{name}/{version}/download");
    let res = agent()
        .get(&url)
        .call()
        .map_err(|e| RegistryError::network(format!("GET {url}: {e}")))?;
    let (status, bytes) = read_bytes(res)?;
    if status == 404 {
        return Err(RegistryError::new(
            "not-found",
            format!("unknown package `{name}@{version}`"),
        ));
    }
    if status != 200 {
        return Err(RegistryError::protocol(format!(
            "download: HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        )));
    }
    Ok(bytes)
}

pub fn publish_pkg(
    base: &str,
    token: &str,
    name: &str,
    version: &str,
    archive: &[u8],
) -> Result<String, RegistryError> {
    if !valid_pkg_name(name) || !valid_version(version) {
        return Err(RegistryError::protocol("bad name or version".to_string()));
    }
    let url = format!("{base}/api/publish?name={name}&version={version}");
    let res = agent()
        .post(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/octet-stream")
        .send(archive)
        .map_err(|e| RegistryError::network(format!("POST {url}: {e}")))?;
    let (status, bytes) = read_bytes(res)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    match status {
        200 => Ok(text),
        401 => Err(RegistryError::new("auth", "publish rejected: bad or missing token".to_string())),
        409 => Err(RegistryError::new(
            "conflict",
            format!("`{name}@{version}` is already published (no overwrite)"),
        )),
        _ => Err(RegistryError::protocol(format!("publish: HTTP {status}: {text}"))),
    }
}

// ---------------------------------------------------------------------------
// Project integration: deps, vendor dir, lockfile
// ---------------------------------------------------------------------------

/// Parse a manifest dep value: `registry:name@version` vs a local path.
/// Returns `Some((name, version))` for registry deps.
pub fn parse_registry_dep(value: &str) -> Option<(String, String)> {
    let rest = value.strip_prefix("registry:")?;
    let (name, version) = rest.split_once('@')?;
    if !valid_pkg_name(name) || !valid_version(version) {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

/// Walk up from `start` (≤8 levels) for `klang.toml`.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    for _ in 0..8 {
        if dir.join("klang.toml").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
    None
}

pub fn vendor_dir(root: &Path) -> PathBuf {
    root.join(".klang_pkgs")
}

pub fn pkg_dir(root: &Path, name: &str, version: &str) -> PathBuf {
    vendor_dir(root).join(name).join(version)
}

fn lock_path(root: &Path) -> PathBuf {
    root.join("klang.lock")
}

/// One pinned registry package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLock {
    pub name: String,
    pub version: String,
    pub sha256: String,
}

/// Parse `package <name> <version> <sha256>` lock lines (old file-hash
/// lines pass through untouched elsewhere).
pub fn parse_package_locks(text: &str) -> Vec<PackageLock> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if parts.next() != Some("package") {
            continue;
        }
        if let (Some(name), Some(version), Some(sha)) = (parts.next(), parts.next(), parts.next()) {
            if valid_pkg_name(name) && valid_version(version) && sha.len() == 64 {
                out.push(PackageLock {
                    name: name.to_string(),
                    version: version.to_string(),
                    sha256: sha.to_string(),
                });
            }
        }
    }
    out
}

fn render_package_lock(lock: &PackageLock) -> String {
    format!("package {} {} {}\n", lock.name, lock.version, lock.sha256)
}

/// Rewrite/add one package pin, preserving every other line byte-wise
/// (including legacy file-hash lines no other writer owns).
fn upsert_package_lock(root: &Path, lock: &PackageLock) -> Result<(), RegistryError> {
    let path = lock_path(root);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in existing.lines() {
        let t = line.trim();
        let mut parts = t.split_whitespace();
        if parts.next() == Some("package") {
            let (n, v) = (parts.next(), parts.next());
            if n == Some(lock.name.as_str()) && v == Some(lock.version.as_str()) {
                lines.push(render_package_lock(lock).trim_end().to_string());
                replaced = true;
                continue;
            }
        }
        lines.push(line.to_string());
    }
    if !replaced {
        lines.push(render_package_lock(lock).trim_end().to_string());
    }
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| RegistryError::new("io", format!("cannot write lock: {e}")))?;
    Ok(())
}

/// Recompute the archive hash of an extracted vendor directory
/// (deterministic pack ⇒ identical bytes ⇒ identical hash).
fn vendor_hash(dir: &Path) -> Result<String, RegistryError> {
    let files = collect_package_files(dir)
        .map_err(|e| RegistryError::new("io", format!("vendor dir unreadable: {e}")))?;
    let bytes =
        pack_archive(&files).map_err(|e| RegistryError::new("io", format!("cannot repack: {e}")))?;
    Ok(sha256_hex(&bytes))
}

fn extract_to(dir: &Path, files: &[(String, Vec<u8>)]) -> Result<(), RegistryError> {
    // Wipe first so a version switch never mixes stale files in.
    if dir.exists() {
        std::fs::remove_dir_all(dir)
            .map_err(|e| RegistryError::new("io", format!("cannot clear vendor dir: {e}")))?;
    }
    for (name, content) in files {
        let dest = dir.join(name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RegistryError::new("io", format!("cannot create dir: {e}")))?;
        }
        std::fs::write(&dest, content)
            .map_err(|e| RegistryError::new("io", format!("cannot write `{name}`: {e}")))?;
    }
    Ok(())
}

/// Ensure one pinned dependency is vendored + verified. Returns a log
/// line for the CLI. Offline mode skips the network (vendor dir or loud
/// error).
pub fn ensure_fetched(
    root: &Path,
    base: &str,
    name: &str,
    version: &str,
    locks: &[PackageLock],
    offline: bool,
) -> Result<String, RegistryError> {
    let dir = pkg_dir(root, name, version);
    let locked = locks
        .iter()
        .find(|l| l.name == name && l.version == version)
        .map(|l| l.sha256.clone());
    if offline {
        if !dir.is_dir() {
            return Err(RegistryError::new(
                "offline",
                format!("package `{name}@{version}` is not vendored (run `klang fetch` online first)"),
            ));
        }
        if let Some(sha) = locked {
            let now = vendor_hash(&dir)?;
            if now != sha {
                return Err(RegistryError::new(
                    "integrity",
                    format!("vendored `{name}@{version}` fails lock verification (lock {sha} vs dir {now})"),
                ));
            }
        }
        return Ok(format!("{name}@{version}: vendored (offline)"));
    }
    let meta = fetch_metadata(base, name).map_err(|e| {
        if e.kind == "not-found" {
            e
        } else {
            RegistryError::network(format!("cannot reach registry for `{name}`: {e}"))
        }
    })?;
    let advertised = meta
        .versions
        .iter()
        .find(|(v, _)| v == version)
        .map(|(_, s)| s.clone())
        .ok_or_else(|| {
            RegistryError::new("not-found", format!("package `{name}` has no version `{version}`"))
        })?;
    // The lockfile pins: a registry entry that moved under a pin is a
    // hard error, never a silent re-pin.
    if let Some(sha) = &locked {
        if *sha != advertised {
            return Err(RegistryError::new(
                "integrity",
                format!("lockfile pins `{name}@{version}` to {sha} but the registry now serves {advertised}"),
            ));
        }
    }
    // Fresh vendor dir with a matching hash: nothing to do.
    if dir.is_dir() {
        if let Ok(now) = vendor_hash(&dir) {
            if now == advertised {
                if locked.is_none() {
                    upsert_package_lock(
                        root,
                        &PackageLock {
                            name: name.to_string(),
                            version: version.to_string(),
                            sha256: advertised.clone(),
                        },
                    )?;
                }
                return Ok(format!("{name}@{version}: already fetched"));
            }
        }
    }
    let bytes = download(base, name, version)?;
    let actual = sha256_hex(&bytes);
    if actual != advertised {
        return Err(RegistryError::new(
            "integrity",
            format!("download of `{name}@{version}` failed integrity check (wanted {advertised}, got {actual})"),
        ));
    }
    let files =
        unpack_archive(&bytes).map_err(|e| RegistryError::protocol(format!("bad archive: {e}")))?;
    extract_to(&dir, &files)?;
    upsert_package_lock(
        root,
        &PackageLock {
            name: name.to_string(),
            version: version.to_string(),
            sha256: advertised.clone(),
        },
    )?;
    Ok(format!("{name}@{version}: fetched ({actual})"))
}

/// Fetch every registry dep in the project manifest. Returns log lines.
pub fn fetch_project(root: &Path, base: &str, offline: bool) -> Result<Vec<String>, RegistryError> {
    let text = std::fs::read_to_string(root.join("klang.toml"))
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
    let locks = parse_package_locks(&lock_text);
    let mut logs = Vec::new();
    let mut any_registry = false;
    for (_, value) in &manifest.deps {
        if let Some((name, version)) = parse_registry_dep(value) {
            any_registry = true;
            logs.push(ensure_fetched(root, base, &name, &version, &locks, offline)?);
        }
    }
    if !any_registry {
        logs.push("no registry dependencies".to_string());
    }
    Ok(logs)
}

/// Append `name = "registry:name@version"` to klang.toml and fetch it.
pub fn add_dependency(
    root: &Path,
    base: &str,
    name: &str,
    version: &str,
    offline: bool,
) -> Result<String, RegistryError> {
    if !valid_pkg_name(name) || !valid_version(version) {
        return Err(RegistryError::protocol("bad name or version".to_string()));
    }
    let path = root.join("klang.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    if manifest.deps.iter().any(|(n, _)| n == name) {
        return Err(RegistryError::new(
            "conflict",
            format!("`{name}` is already a dependency"),
        ));
    }
    let line = format!("{name} = \"registry:{name}@{version}\"");
    let mut new_text = text;
    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    if new_text.lines().any(|l| l.trim() == "[dependencies]") {
        new_text.push_str(&line);
        new_text.push('\n');
    } else {
        new_text.push_str("[dependencies]\n");
        new_text.push_str(&line);
        new_text.push('\n');
    }
    std::fs::write(&path, new_text)
        .map_err(|e| RegistryError::new("io", format!("cannot write klang.toml: {e}")))?;
    let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
    let locks = parse_package_locks(&lock_text);
    ensure_fetched(root, base, name, version, &locks, offline)?;
    Ok(format!("added {name}@{version}"))
}

/// Vendor fallback for the module loader: when an import's first path
/// segment names a registry dependency, resolve it into the pinned
/// vendor directory. Local files always win (the loader only calls this
/// after the relative read fails).
pub fn resolve_vendor_import(root: &Path, deps: &[(String, String)], imp: &str) -> Option<PathBuf> {
    let mut parts = imp.split('/');
    let head = parts.next()?;
    for (dep_name, value) in deps {
        if dep_name == head {
            if let Some((_, version)) = parse_registry_dep(value) {
                let rest: Vec<&str> = parts.collect();
                if rest.is_empty() {
                    return None;
                }
                if rest.iter().any(|s| *s == ".." || s.is_empty()) {
                    return None;
                }
                return Some(pkg_dir(root, dep_name, &version).join(rest.join("/")));
            }
        }
    }
    None
}
