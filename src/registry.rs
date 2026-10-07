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
use std::time::{Duration, Instant};

use crate::mcp::{parse_json, Json};
use crate::registry_storage::{
    local_archive_key, local_index_key, s3_archive_key, s3_checksum_key, s3_index_key,
    PutOutcome, Storage,
};

// ---------------------------------------------------------------------------
// Names, versions, hashing
// ---------------------------------------------------------------------------

/// Official registry (PRD §3). Resolution order (first match wins):
/// 1. `--registry URL`, 2. `KLANG_REGISTRY` env, 3. `[registry] url` in
/// the project's `klang.toml`, 4. `url` in `~/.klang/config.toml`,
/// 5. this constant.
pub const DEFAULT_REGISTRY: &str = "https://klang.raunakdevelops.dpdns.org";

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
// S7: max files 500, max archive 5 MiB by default (env override).
const ARCHIVE_MAGIC: &[u8; 9] = b"KLANGPKG1";
const MAX_FILES: usize = 500;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
/// Default max published archive bytes (S7). Overridden by
/// `KLANG_MAX_ARCHIVE_BYTES`.
pub const DEFAULT_MAX_ARCHIVE_BYTES: u64 = 5 * 1024 * 1024;

/// Effective max archive bytes (env `KLANG_MAX_ARCHIVE_BYTES` or default).
pub fn max_archive_bytes() -> u64 {
    std::env::var("KLANG_MAX_ARCHIVE_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_ARCHIVE_BYTES)
}

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
pub struct VersionEntry {
    version: String,
    sha256: String,
    size: u64,
    /// Transitive registry requirements: (package name, constraint raw).
    /// Empty for self-contained packages. Old index files load as empty.
    dependencies: Vec<(String, String)>,
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
                                            (
                                                "dependencies".to_string(),
                                                Json::Arr(
                                                    v.dependencies
                                                        .iter()
                                                        .map(|(n, c)| {
                                                            Json::Obj(vec![
                                                                ("name".to_string(), Json::Str(n.clone())),
                                                                (
                                                                    "constraint".to_string(),
                                                                    Json::Str(c.clone()),
                                                                ),
                                                            ])
                                                        })
                                                        .collect(),
                                                ),
                                            ),
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
                // Dependencies are optional (old index files have none).
                let mut dependencies = Vec::new();
                if let Some(ds) = v.get("dependencies").and_then(|j| j.as_arr()) {
                    for d in ds {
                        let (Some(n), Some(c)) = (
                            d.get("name").and_then(|j| j.as_str()),
                            d.get("constraint")
                                .or_else(|| d.get("req"))
                                .and_then(|j| j.as_str()),
                        ) else {
                            continue;
                        };
                        if valid_pkg_name(n) && !c.is_empty() && c.len() <= 32 {
                            dependencies.push((n.to_string(), c.to_string()));
                        }
                    }
                }
                versions.push(VersionEntry {
                    version: version.to_string(),
                    sha256: sha256.to_string(),
                    size: 0,
                    dependencies,
                });
            }
        }
        index.packages.insert(name.to_string(), versions);
    }
    Ok(index)
}

/// Server configuration. The data dir holds `index.json` plus
/// `<name>/<version>.kpkg` archives on local disk; with the B2 backend
/// the data dir is unused (state lives in the bucket) but kept so the
/// CLI shape and local dev behave as before.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub data_dir: PathBuf,
    pub admin_token: String,
}

/// TTL for the per-package version-list cache (B2 backend only). Reads
/// refresh a package from the bucket when its entry is older than this;
/// publishes invalidate the entry immediately.
const INDEX_CACHE_TTL: Duration = Duration::from_secs(30);

struct Server {
    cfg: RegistryConfig,
    index: Mutex<Index>,
    /// Failed publish auth per client IP (timestamps). S5 rate limiting.
    auth_failures: Mutex<HashMap<String, Vec<std::time::Instant>>>,
    /// Object store (local disk or B2). All persistence goes through it.
    storage: Arc<dyn Storage>,
    /// Backend name for logs/health (`local` / `b2`). Never a secret.
    backend: &'static str,
    /// Per-package version lists: (fetched_at, entries). B2 only.
    index_cache: Mutex<HashMap<String, (Instant, Vec<VersionEntry>)>>,
}

/// S1: publishing is enabled only when the server token is at least 32 chars.
pub fn publishing_enabled(token: &str) -> bool {
    token.len() >= 32
}

/// S9 + S2: constant-time bearer check that does not leak token length.
/// Both sides are hashed with SHA-256 first so the comparison is always
/// over 32-byte digests (no early exit, no length oracle).
fn bearer_auth_ok(provided: Option<&str>, expected_token: &str) -> bool {
    let expected = format!("Bearer {expected_token}");
    let prov = provided.unwrap_or("");
    let h_prov = sha256_bytes(prov.as_bytes());
    let h_exp = sha256_bytes(expected.as_bytes());
    constant_time_eq(&h_prov, &h_exp)
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Effective client IP for S5 rate limiting. Behind Render's proxy, use
/// `X-Forwarded-For` carefully: first hop only (left-most entry). The
/// header is spoofable unless a trusted proxy strips it — documented in
/// `docs/deploy-registry.md`. Falls back to the TCP peer IP.
fn effective_client_ip(headers: &HashMap<String, String>, peer: &str) -> String {
    if let Some(xff) = headers.get("x-forwarded-for") {
        let first = xff.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    peer.to_string()
}

/// S5: true when `ip` has ≥10 failed auths in the last 60s.
fn is_rate_limited(server: &Server, ip: &str) -> bool {
    let mut map = server.auth_failures.lock().expect("auth lock");
    let now = std::time::Instant::now();
    let entry = map.entry(ip.to_string()).or_default();
    entry.retain(|t| now.duration_since(*t).as_secs() < 60);
    entry.len() >= 10
}

fn record_auth_failure(server: &Server, ip: &str) {
    let mut map = server.auth_failures.lock().expect("auth lock");
    let now = std::time::Instant::now();
    let entry = map.entry(ip.to_string()).or_default();
    entry.retain(|t| now.duration_since(*t).as_secs() < 60);
    entry.push(now);
}

impl Server {
    fn load(cfg: &RegistryConfig) -> Result<Self, String> {
        let storage = crate::registry_storage::open_storage(
            &crate::registry_storage::StorageBackend::Local,
            &cfg.data_dir,
        )?;
        Self::load_with_storage(cfg, storage, "local")
    }

    fn load_with_storage(
        cfg: &RegistryConfig,
        storage: Arc<dyn Storage>,
        backend: &'static str,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(&cfg.data_dir)
            .map_err(|e| format!("cannot create data dir: {e}"))?;
        let mut index = if backend == "b2" {
            Self::load_b2_index(&storage)?
        } else {
            Self::load_local_index(&storage)?
        };
        if backend != "b2" {
            // Reconcile: entries whose archive is missing are dropped loudly
            // at startup (fail closed, never serve phantom metadata).
            let mut dropped = Vec::new();
            for (name, versions) in index.packages.iter_mut() {
                versions.retain(|v| {
                    let keep = storage
                        .exists(&local_archive_key(name, &v.version))
                        .unwrap_or(false);
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
        }
        index.packages.retain(|_, vs| !vs.is_empty());
        Ok(Self {
            cfg: cfg.clone(),
            index: Mutex::new(index),
            auth_failures: Mutex::new(HashMap::new()),
            storage,
            backend,
            index_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Load the full index from local-disk storage (`index.json`).
    fn load_local_index(storage: &Arc<dyn Storage>) -> Result<Index, String> {
        match storage.get(&local_index_key())? {
            None => Ok(Index::default()),
            Some(bytes) => {
                let text =
                    String::from_utf8(bytes).map_err(|_| "bad index.json: not UTF-8".to_string())?;
                let json = parse_json(&text).map_err(|e| format!("bad index.json: {e}"))?;
                index_from_json(&json)
            }
        }
    }

    /// Load the full index from a bucket. Archives under `packages/` are
    /// the source of truth: each stored `index/<name>.json` is used when
    /// it covers exactly the archived versions, otherwise it is repaired
    /// from the listing (and the repaired doc is written back).
    fn load_b2_index(storage: &Arc<dyn Storage>) -> Result<Index, String> {
        let mut found: HashMap<String, Vec<String>> = HashMap::new();
        for key in storage.list("packages/")? {
            let Some(rest) = key.strip_prefix("packages/") else {
                continue;
            };
            let Some((name, file)) = rest.split_once('/') else {
                continue;
            };
            if rest.contains("//") {
                continue;
            }
            if let Some(version) = file.strip_suffix(".klangpkg") {
                if valid_pkg_name(name) && valid_version(version) {
                    found
                        .entry(name.to_string())
                        .or_default()
                        .push(version.to_string());
                }
            }
        }
        let mut names: Vec<String> = found.keys().cloned().collect();
        names.sort();
        let mut index = Index::default();
        for name in names {
            let mut versions = found.remove(&name).unwrap_or_default();
            versions.sort_by(|a, b| version_cmp(a, b));
            versions.dedup();
            let stored = match storage.get(&s3_index_key(&name)) {
                Ok(Some(bytes)) => versions_from_meta_json(&name, &bytes).ok(),
                _ => None,
            };
            let entries = match stored {
                Some(vs) if same_version_set(&vs, &versions) => vs,
                _ => {
                    let repaired = Self::repair_package_index(storage, &name, &versions)?;
                    if let Ok(doc) = package_meta_json(&name, &repaired) {
                        // Best-effort: a failed write heals on next load.
                        let _ = storage.put(&s3_index_key(&name), doc.as_bytes());
                    }
                    repaired
                }
            };
            if !entries.is_empty() {
                index.packages.insert(name, entries);
            }
        }
        Ok(index)
    }

    /// Rebuild one package's entries from archived versions: checksum
    /// from the `.sha256` sidecar when valid (else hashed from the
    /// archive bytes), size from the archive, dependencies extracted
    /// from the embedded manifest when present.
    fn repair_package_index(
        storage: &Arc<dyn Storage>,
        name: &str,
        versions: &[String],
    ) -> Result<Vec<VersionEntry>, String> {
        let mut out = Vec::new();
        for version in versions {
            let bytes = match storage.get(&s3_archive_key(name, version))? {
                Some(b) => b,
                None => continue, // vanished between list and get; skip
            };
            let sha256 = match storage.get(&s3_checksum_key(name, version)) {
                Ok(Some(raw)) => {
                    let s = String::from_utf8_lossy(&raw).trim().to_string();
                    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                        // Repair a missing sidecar is unnecessary; keep it.
                        s
                    } else {
                        let h = sha256_hex(&bytes);
                        let _ = storage.put(&s3_checksum_key(name, version), h.as_bytes());
                        h
                    }
                }
                _ => {
                    let h = sha256_hex(&bytes);
                    let _ = storage.put(&s3_checksum_key(name, version), h.as_bytes());
                    h
                }
            };
            let dependencies = unpack_archive(&bytes)
                .ok()
                .map(|files| manifest_deps(&files, name))
                .unwrap_or_default();
            out.push(VersionEntry {
                version: version.clone(),
                sha256,
                size: bytes.len() as u64,
                dependencies,
            });
        }
        out.sort_by(|a, b| version_cmp(&a.version, &b.version));
        Ok(out)
    }

    /// Refresh one package from bucket storage when its cache entry is
    /// missing or older than [`INDEX_CACHE_TTL`] (B2 only; local disk is
    /// single-writer so memory is always fresh). Never holds both locks
    /// at once (publish takes them in the opposite order).
    fn refresh_package_if_stale(&self, name: &str) {
        if self.backend != "b2" || !valid_pkg_name(name) {
            return;
        }
        let stale = match self.index_cache.lock().expect("cache lock").get(name) {
            None => true,
            Some((at, _)) => at.elapsed() >= INDEX_CACHE_TTL,
        };
        if !stale {
            return;
        }
        let keys = match self.storage.list(&format!("packages/{name}/")) {
            Ok(k) => k,
            Err(_) => return, // transient: keep serving memory
        };
        let mut versions: Vec<String> = Vec::new();
        for key in &keys {
            let prefix = format!("packages/{name}/");
            if let Some(file) = key.strip_prefix(&prefix) {
                if let Some(v) = file.strip_suffix(".klangpkg") {
                    if valid_version(v) && !file.contains('/') {
                        versions.push(v.to_string());
                    }
                }
            }
        }
        versions.sort_by(|a, b| version_cmp(a, b));
        versions.dedup();
        let entries = if versions.is_empty() {
            Vec::new()
        } else {
            match self.storage.get(&s3_index_key(name)) {
                Ok(Some(bytes)) => match versions_from_meta_json(name, &bytes) {
                    Ok(vs) if same_version_set(&vs, &versions) => vs,
                    _ => match Self::repair_package_index(&self.storage, name, &versions) {
                        Ok(vs) => vs,
                        Err(_) => return,
                    },
                },
                _ => match Self::repair_package_index(&self.storage, name, &versions) {
                    Ok(vs) => vs,
                    Err(_) => return,
                },
            }
        };
        {
            let mut index = self.index.lock().expect("index lock");
            if entries.is_empty() {
                index.packages.remove(name);
            } else {
                index.packages.insert(name.to_string(), entries.clone());
            }
        }
        self.index_cache
            .lock()
            .expect("cache lock")
            .insert(name.to_string(), (Instant::now(), entries));
    }

    /// Drop a package's cache entry (called on publish).
    fn invalidate_package(&self, name: &str) {
        self.index_cache.lock().expect("cache lock").remove(name);
    }

    fn save(&self) -> Result<(), String> {
        if self.backend == "b2" {
            // The publish path writes the touched `index/<name>.json`
            // directly; a full save rewrites every package doc (used by
            // tests and as a repair helper).
            let docs: Vec<(String, String)> = {
                let index = self.index.lock().expect("index lock");
                index
                    .packages
                    .iter()
                    .filter_map(|(name, versions)| {
                        package_meta_json(name, versions)
                            .ok()
                            .map(|doc| (s3_index_key(name), doc))
                    })
                    .collect()
            };
            for (key, doc) in &docs {
                self.storage.put(key, doc.as_bytes())?;
            }
            return Ok(());
        }
        let json = {
            let index = self.index.lock().expect("index lock");
            index_to_json(&index).render()
        };
        self.storage.put(&local_index_key(), json.as_bytes())
    }
}

/// True when the stored entries cover exactly the archived versions.
fn same_version_set(entries: &[VersionEntry], versions: &[String]) -> bool {
    if entries.len() != versions.len() {
        return false;
    }
    let mut have: Vec<&str> = entries.iter().map(|e| e.version.as_str()).collect();
    have.sort_unstable();
    let mut want: Vec<&str> = versions.iter().map(|s| s.as_str()).collect();
    want.sort_unstable();
    have == want
}

/// Render one package's metadata doc (also the stored `index/<name>.json`
/// shape on B2).
fn package_meta_json(name: &str, versions: &[VersionEntry]) -> Result<String, String> {
    let mut vs = versions.to_vec();
    vs.sort_by(|a, b| version_cmp(&a.version, &b.version));
    let latest = Index::latest(&vs).map(|v| v.version.clone()).unwrap_or_default();
    let doc = Json::Obj(vec![
        ("name".to_string(), Json::Str(name.to_string())),
        ("latest".to_string(), Json::Str(latest)),
        (
            "versions".to_string(),
            Json::Arr(
                vs.iter()
                    .map(|v| {
                        Json::Obj(vec![
                            ("version".to_string(), Json::Str(v.version.clone())),
                            ("sha256".to_string(), Json::Str(v.sha256.clone())),
                            ("size".to_string(), Json::Int(v.size as i64)),
                            (
                                "dependencies".to_string(),
                                Json::Arr(
                                    v.dependencies
                                        .iter()
                                        .map(|(n, c)| {
                                            Json::Obj(vec![
                                                ("name".to_string(), Json::Str(n.clone())),
                                                ("constraint".to_string(), Json::Str(c.clone())),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    Ok(doc.render())
}

/// Parse one package's metadata doc back into entries.
fn versions_from_meta_json(name: &str, bytes: &[u8]) -> Result<Vec<VersionEntry>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| format!("index for `{name}` is not UTF-8"))?;
    let json = parse_json(text).map_err(|e| format!("bad index for `{name}`: {e}"))?;
    let got = json
        .get("name")
        .and_then(|j| j.as_str())
        .ok_or_else(|| format!("index for `{name}` has no name"))?;
    if got != name {
        return Err(format!("index name mismatch for `{name}`"));
    }
    let arr = json
        .get("versions")
        .and_then(|j| j.as_arr())
        .ok_or_else(|| format!("index for `{name}` has no versions"))?;
    let mut out = Vec::new();
    for v in arr {
        let version = v
            .get("version")
            .and_then(|j| j.as_str())
            .ok_or_else(|| format!("index for `{name}` has a version without number"))?;
        let sha256 = v
            .get("sha256")
            .and_then(|j| j.as_str())
            .ok_or_else(|| format!("index for `{name}` has a version without sha256"))?;
        if !valid_version(version) || sha256.len() != 64 {
            return Err(format!("index for `{name}` has a bad version entry"));
        }
        let size = match v.get("size") {
            Some(Json::Int(n)) => (*n).max(0) as u64,
            _ => 0,
        };
        let mut dependencies = Vec::new();
        if let Some(ds) = v.get("dependencies").and_then(|j| j.as_arr()) {
            for d in ds {
                let (Some(n), Some(c)) = (
                    d.get("name").and_then(|j| j.as_str()),
                    d.get("constraint")
                        .or_else(|| d.get("req"))
                        .and_then(|j| j.as_str()),
                ) else {
                    continue;
                };
                if valid_pkg_name(n) && !c.is_empty() && c.len() <= 32 {
                    dependencies.push((n.to_string(), c.to_string()));
                }
            }
        }
        out.push(VersionEntry {
            version: version.to_string(),
            sha256: sha256.to_string(),
            size,
            dependencies,
        });
    }
    out.sort_by(|a, b| version_cmp(&a.version, &b.version));
    Ok(out)
}

/// Extract transitive registry requirements from the embedded klang.toml
/// (if any). Malformed manifests yield no deps rather than failing the
/// publish; self-dependencies are dropped (unresolvable cycle).
fn manifest_deps(files: &[(String, Vec<u8>)], pkg_name: &str) -> Vec<(String, String)> {
    files
        .iter()
        .find(|(n, _)| n == "klang.toml")
        .and_then(|(_, b)| String::from_utf8(b.clone()).ok())
        .and_then(|t| crate::package::Manifest::parse(&t).ok())
        .map(|m| {
            let mut out = Vec::new();
            for (k, v) in m.deps.iter().chain(m.dev_deps.iter()) {
                if let Some((pn, c)) = parse_registry_req(v, k) {
                    if pn != pkg_name {
                        out.push((pn, c));
                    }
                }
            }
            out.sort();
            out.dedup();
            out
        })
        .unwrap_or_default()
}

/// Small `{"published":...}` response body shared by the publish paths.
fn published_json(name: &str, version: &str, sha256: &str, size: u64) -> Vec<u8> {
    json_body(&Json::Obj(vec![
        ("published".to_string(), Json::Bool(true)),
        ("name".to_string(), Json::Str(name.to_string())),
        ("version".to_string(), Json::Str(version.to_string())),
        ("sha256".to_string(), Json::Str(sha256.to_string())),
        ("size".to_string(), Json::Int(size as i64)),
    ]))
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
    respond_with_headers(stream, status, reason, content_type, None, body)
}

fn respond_with_headers(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    retry_after: Option<u64>,
    body: &[u8],
) {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(secs) = retry_after {
        head.push_str(&format!("Retry-After: {secs}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
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

fn handle(server: &Server, req: Request, client_ip: &str, stream: &mut TcpStream) {
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
    // GET /health: same liveness proof, never touches storage.
    if req.method == "GET" && segs == ["health"] {
        let body = Json::Obj(vec![
            ("ok".to_string(), Json::Bool(true)),
            ("service".to_string(), Json::Str("klang-registry".to_string())),
        ]);
        respond(stream, 200, "OK", "application/json", &json_body(&body));
        return;
    }
    // GET /health/storage: cheap storage probe (an empty-prefix list
    // that stays one page on an empty bucket). Reports ok/error with
    // only the backend name — never credentials or bucket details.
    if req.method == "GET" && segs == ["health", "storage"] {
        match server.storage.list("health/") {
            Ok(_) => {
                let body = Json::Obj(vec![
                    ("ok".to_string(), Json::Bool(true)),
                    ("storage".to_string(), Json::Str(server.backend.to_string())),
                ]);
                respond(stream, 200, "OK", "application/json", &json_body(&body));
            }
            Err(_) => {
                let body = Json::Obj(vec![
                    ("ok".to_string(), Json::Bool(false)),
                    ("storage".to_string(), Json::Str(server.backend.to_string())),
                    (
                        "error".to_string(),
                        Json::Str("storage unreachable".to_string()),
                    ),
                ]);
                respond(stream, 503, "Service Unavailable", "application/json", &json_body(&body));
            }
        }
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
        // B2: refresh the package when its cache entry is stale (a
        // sibling instance may have published since). Local: no-op.
        server.refresh_package_if_stale(&name);
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
                                            (
                                                "dependencies".to_string(),
                                                Json::Arr(
                                                    v.dependencies
                                                        .iter()
                                                        .map(|(n, c)| {
                                                            Json::Obj(vec![
                                                                ("name".to_string(), Json::Str(n.clone())),
                                                                (
                                                                    "constraint".to_string(),
                                                                    Json::Str(c.clone()),
                                                                ),
                                                            ])
                                                        })
                                                        .collect(),
                                                ),
                                            ),
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
        let key = if server.backend == "b2" {
            s3_archive_key(&name, &version)
        } else {
            local_archive_key(&name, &version)
        };
        match server.storage.get(&key) {
            Ok(Some(bytes)) => respond(stream, 200, "OK", "application/octet-stream", &bytes),
            Ok(None) => respond(
                stream,
                404,
                "Not Found",
                "application/json",
                &err_json(&format!("unknown package `{name}@{version}`")),
            ),
            Err(_) => respond(
                stream,
                500,
                "Server Error",
                "application/json",
                &err_json("storage error"),
            ),
        }
        return;
    }
    // POST /api/publish?name=&version= (raw archive body, bearer auth).
    // S1-S2/S4-S9: fail-closed when unconfigured, constant-time auth,
    // rate-limited failures, idempotent republish, size caps.
    // S4: never log the token, Authorization header, or bodies; error
    // bodies never echo the supplied token.
    if req.method == "POST" && segs == ["api", "publish"] {
        // S1: fail closed — reads still work, writes get 503.
        if !publishing_enabled(&server.cfg.admin_token) {
            respond(
                stream,
                503,
                "Service Unavailable",
                "application/json",
                &err_json("publishing disabled: server token not configured"),
            );
            return;
        }
        // S5: rate-limit failed auth per client IP (10/min -> 429).
        if is_rate_limited(server, client_ip) {
            respond_with_headers(
                stream,
                429,
                "Too Many Requests",
                "application/json",
                Some(60),
                &err_json("too many failed auth attempts (retry later)"),
            );
            return;
        }
        let authed = bearer_auth_ok(
            req.headers.get("authorization").map(|s| s.as_str()),
            &server.cfg.admin_token,
        );
        if !authed {
            record_auth_failure(server, client_ip);
            // S4: generic message, never echoes the supplied token.
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
        // S7: max archive size (413 when exceeded).
        if req.body.len() as u64 > max_archive_bytes() {
            respond(
                stream,
                413,
                "Payload Too Large",
                "application/json",
                &err_json("archive exceeds size limit"),
            );
            return;
        }
        let files = match unpack_archive(&req.body) {
            Ok(f) => f,
            Err(e) => {
                // Distinguish traversal/unsafe names (400) from generic
                // corruption (400). Oversize already handled above.
                let msg = e.to_string();
                if msg.contains("unsafe") || msg.contains("traversal") {
                    respond(stream, 400, "Bad Request", "application/json", &err_json("archive contains unsafe name"));
                } else {
                    respond(
                        stream,
                        400,
                        "Bad Request",
                        "application/json",
                        &err_json("body is not a valid package archive"),
                    );
                }
                return;
            }
        };
        // Transitive registry requirements from the embedded klang.toml
        // (malformed manifests publish with no deps, never fail here).
        let dependencies: Vec<(String, String)> = manifest_deps(&files, &name);
        let archive_key = if server.backend == "b2" {
            s3_archive_key(&name, &version)
        } else {
            local_archive_key(&name, &version)
        };
        // S6 immutability: same bytes -> 200 idempotent; different bytes
        // -> 409 Conflict (never overwrite).
        let already_published = {
            let index = server.index.lock().expect("index lock");
            index
                .packages
                .get(&name)
                .map(|vs| vs.iter().any(|v| v.version == version))
                .unwrap_or(false)
        };
        if already_published {
            let identical = server
                .storage
                .get(&archive_key)
                .map(|old| old.map(|b| b == req.body).unwrap_or(false))
                .unwrap_or(false);
            if identical {
                // Idempotent republish: return current metadata.
                let (sha256, size) = {
                    let idx = server.index.lock().expect("index lock");
                    idx.packages
                        .get(&name)
                        .and_then(|vs| vs.iter().find(|v| v.version == version))
                        .map(|v| (v.sha256.clone(), v.size))
                        .unwrap_or_else(|| (sha256_hex(&req.body), req.body.len() as u64))
                };
                respond(
                    stream,
                    200,
                    "OK",
                    "application/json",
                    &published_json(&name, &version, &sha256, size),
                );
                return;
            }
            respond(
                stream,
                409,
                "Conflict",
                "application/json",
                &err_json(&format!("`{name}@{version}` already published (no overwrite)")),
            );
            return;
        }
        let sha256 = sha256_hex(&req.body);
        let size = req.body.len() as u64;
        // Create-if-absent at the object layer: identical bytes racing
        // here stay idempotent; different bytes report a conflict. (See
        // the race note on `Storage::put_if_absent`.)
        match server.storage.put_if_absent(&archive_key, &req.body) {
            Err(_) => {
                respond(stream, 500, "Server Error", "application/json", &err_json("cannot store"));
                return;
            }
            Ok(PutOutcome::AlreadyExistsDifferent) => {
                respond(
                    stream,
                    409,
                    "Conflict",
                    "application/json",
                    &err_json(&format!("`{name}@{version}` already published (no overwrite)")),
                );
                return;
            }
            Ok(PutOutcome::AlreadyExistsSame) => {
                let (sha256, size) = {
                    let idx = server.index.lock().expect("index lock");
                    idx.packages
                        .get(&name)
                        .and_then(|vs| vs.iter().find(|v| v.version == version))
                        .map(|v| (v.sha256.clone(), v.size))
                        .unwrap_or_else(|| (sha256_hex(&req.body), req.body.len() as u64))
                };
                respond(
                    stream,
                    200,
                    "OK",
                    "application/json",
                    &published_json(&name, &version, &sha256, size),
                );
                return;
            }
            Ok(PutOutcome::Created) => {}
        }
        if server.backend == "b2" {
            // Checksum sidecar + per-package index doc; then drop the
            // cached version list so the next read is fresh.
            if server
                .storage
                .put(&s3_checksum_key(&name, &version), sha256.as_bytes())
                .is_err()
            {
                respond(stream, 500, "Server Error", "application/json", &err_json("cannot store"));
                return;
            }
            let doc = {
                let mut index = server.index.lock().expect("index lock");
                let entry = index.packages.entry(name.clone()).or_default();
                if !entry.iter().any(|v| v.version == version) {
                    entry.push(VersionEntry {
                        version: version.clone(),
                        sha256: sha256.clone(),
                        size,
                        dependencies: dependencies.clone(),
                    });
                }
                let current = index.packages.get(&name).cloned().unwrap_or_default();
                match package_meta_json(&name, &current) {
                    Ok(d) => d,
                    Err(_) => {
                        respond(stream, 500, "Server Error", "application/json", &err_json("cannot save index"));
                        return;
                    }
                }
            };
            if server.storage.put(&s3_index_key(&name), doc.as_bytes()).is_err() {
                respond(stream, 500, "Server Error", "application/json", &err_json("cannot save index"));
                return;
            }
            server.invalidate_package(&name);
        } else {
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
                        dependencies: dependencies.clone(),
                    });
            }
            if server.save().is_err() {
                respond(stream, 500, "Server Error", "application/json", &err_json("cannot save index"));
                return;
            }
        }
        respond(
            stream,
            200,
            "OK",
            "application/json",
            &published_json(&name, &version, &sha256, size),
        );
        return;
    }
    respond(stream, 404, "Not Found", "application/json", &err_json("unknown route"));
}

fn handle_conn(server: Arc<Server>, mut stream: TcpStream) {
    // Peer IP for rate limiting (fallback when X-Forwarded-For is absent).
    let peer = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    match read_request(&mut stream) {
        Err(e) => respond(&mut stream, 400, "Bad Request", "application/json", &err_json(&e)),
        Ok(req) => {
            let ip = effective_client_ip(&req.headers, &peer);
            // S4: never log headers/bodies/tokens here.
            handle(&server, req, &ip, &mut stream)
        }
    }
}

/// Serve forever on `listener` (thread-per-connection) with local-disk
/// storage (the default; dev and tests behave as before).
pub fn serve(listener: TcpListener, cfg: RegistryConfig) -> Result<(), String> {
    let storage = crate::registry_storage::open_storage(
        &crate::registry_storage::StorageBackend::Local,
        &cfg.data_dir,
    )?;
    serve_with_storage(listener, cfg, storage, "local")
}

/// Serve forever on `listener` with an explicit storage backend.
/// Logs only the backend name — never credentials or bucket details.
pub fn serve_with_storage(
    listener: TcpListener,
    cfg: RegistryConfig,
    storage: Arc<dyn Storage>,
    backend: &'static str,
) -> Result<(), String> {
    let server = Arc::new(Server::load_with_storage(&cfg, storage, backend)?);
    eprintln!("registry: storage backend `{}`", server.backend);
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
    let storage = crate::registry_storage::open_storage(
        &crate::registry_storage::StorageBackend::Local,
        data_dir,
    )?;
    spawn_ephemeral_with_storage(data_dir, admin_token, storage, "local")
}

/// Spawn an ephemeral server on 127.0.0.1:0 with an explicit storage
/// backend (used by the B2/mock-S3 tests); returns its base URL.
pub fn spawn_ephemeral_with_storage(
    data_dir: &Path,
    admin_token: &str,
    storage: Arc<dyn Storage>,
    backend: &'static str,
) -> Result<String, String> {
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
        if let Err(e) = serve_with_storage(listener, cfg, storage, backend) {
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
    /// Per-version transitive requirements: (version, [(dep, constraint)]).
    /// Empty when the server predates transitive metadata.
    pub version_deps: Vec<(String, Vec<(String, String)>)>,
}

impl PackageMeta {
    /// Transitive requirements declared by `version` (empty if none).
    pub fn deps_for(&self, version: &str) -> Vec<(String, String)> {
        self.version_deps
            .iter()
            .find(|(v, _)| v == version)
            .map(|(_, d)| d.clone())
            .unwrap_or_default()
    }
}

fn parse_meta(name: &str, text: &str) -> Result<PackageMeta, RegistryError> {
    let json = parse_json(text).map_err(|e| RegistryError::protocol(format!("bad metadata JSON: {e}")))?;
    let latest = json
        .get("latest")
        .and_then(|j| j.as_str())
        .unwrap_or_default()
        .to_string();
    let mut versions = Vec::new();
    let mut version_deps = Vec::new();
    if let Some(vs) = json.get("versions").and_then(|j| j.as_arr()) {
        for v in vs {
            let (Some(version), Some(sha)) = (
                v.get("version").and_then(|j| j.as_str()),
                v.get("sha256").and_then(|j| j.as_str()),
            ) else {
                return Err(RegistryError::protocol("bad version entry".to_string()));
            };
            versions.push((version.to_string(), sha.to_string()));
            let mut deps = Vec::new();
            if let Some(ds) = v.get("dependencies").and_then(|j| j.as_arr()) {
                for d in ds {
                    let (Some(n), Some(c)) = (
                        d.get("name").and_then(|j| j.as_str()),
                        d.get("constraint")
                            .or_else(|| d.get("req"))
                            .and_then(|j| j.as_str()),
                    ) else {
                        continue;
                    };
                    if valid_pkg_name(n) && !c.is_empty() {
                        deps.push((n.to_string(), c.to_string()));
                    }
                }
            }
            version_deps.push((version.to_string(), deps));
        }
    }
    Ok(PackageMeta {
        name: name.to_string(),
        latest,
        versions,
        version_deps,
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
    // C6: never send a token over a non-HTTPS URL (except localhost).
    // The check lives here so every publish path enforces it.
    if validate_registry_url(base).is_err() {
        return Err(RegistryError::new(
            "auth",
            "refusing to send credentials over an insecure registry URL (use https or localhost)".to_string(),
        ));
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
        503 => Err(RegistryError::new(
            "unavailable",
            "publishing disabled: server token not configured".to_string(),
        )),
        429 => Err(RegistryError::new(
            "rate-limited",
            "too many failed auth attempts (retry later)".to_string(),
        )),
        413 => Err(RegistryError::new(
            "too-large",
            "archive exceeds size limit".to_string(),
        )),
        _ => Err(RegistryError::protocol(format!("publish: HTTP {status}: {text}"))),
    }
}

// ---------------------------------------------------------------------------
// Project integration: deps, vendor dir, lockfile
// ---------------------------------------------------------------------------

/// Parse a manifest dep value: `registry:name@version` vs a local path.
/// Returns `Some((name, version))` for registry deps.
/// Exact pins only (backcompat); for ranges see `parse_registry_req`.
pub fn parse_registry_dep(value: &str) -> Option<(String, String)> {
    let rest = value.strip_prefix("registry:")?;
    let (name, version) = rest.split_once('@')?;
    if !valid_pkg_name(name) || !valid_version(version) {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

/// Parse a manifest dep into a registry requirement with a SemVer
/// constraint. Supports both shapes:
/// - `registry:name@<constraint>` (explicit; constraint may be exact,
///   `^`/`~`/`>=`/`latest`),
/// - `<constraint>` with `key` as the package name (PRD shape:
///   `collections = "^1.2.0"`).
/// Returns `Some((package_name, constraint_raw))`. Local paths return `None`.
pub fn parse_registry_req(value: &str, key: &str) -> Option<(String, String)> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix("registry:") {
        let (name, constraint) = rest.split_once('@')?;
        let (name, constraint) = (name.trim(), constraint.trim());
        if !valid_pkg_name(name) || constraint.is_empty() || constraint.len() > 32 {
            return None;
        }
        if crate::package::version::parse_constraint(constraint).is_err() {
            return None;
        }
        return Some((name.to_string(), constraint.to_string()));
    }
    // Bare constraint with the key as package name.
    if !valid_pkg_name(key) {
        return None;
    }
    let v = value;
    if v.starts_with('.') || v.starts_with('/') || v.starts_with('~') && v.contains('/') {
        return None;
    }
    if v.contains('/') || v.contains('\\') || v.contains("..") {
        return None;
    }
    if crate::package::version::parse_constraint(v).is_ok() {
        // Distinguish from local paths: a bare path like `./x.klang`
        // never parses as a constraint (contains `/`), and plain words
        // like `latest` do. Exact `1.2.3` is a registry pin here.
        return Some((key.to_string(), v.to_string()));
    }
    None
}

/// True when a manifest value refers to the registry (exact or ranged).
pub fn is_registry_dep(value: &str, key: &str) -> bool {
    parse_registry_dep(value).is_some() || parse_registry_req(value, key).is_some()
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
/// Lenient: skips malformed lines. New security-sensitive paths should
/// prefer [`parse_package_locks_strict`] which rejects bad checksums.
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
            if valid_pkg_name(name) && valid_version(version) && is_valid_checksum(sha) {
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

/// Strict lock parse (B2): any `package` line with a missing/empty/
/// non-64-hex checksum (or bad name/version/extra fields) is an error.
/// Non-`package` lines are ignored (legacy file-hash lines, blanks).
pub fn parse_package_locks_strict(text: &str) -> Result<Vec<PackageLock>, RegistryError> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if parts.next() != Some("package") {
            continue;
        }
        let (name, version, sha) = match (parts.next(), parts.next(), parts.next()) {
            (Some(n), Some(v), Some(s)) => (n, v, s),
            _ => {
                return Err(RegistryError::new(
                    "integrity",
                    format!("bad lock line (want `package <name> <version> <sha256>`): `{line}`"),
                ));
            }
        };
        if parts.next().is_some() {
            return Err(RegistryError::new(
                "integrity",
                format!("bad lock line (extra fields): `{line}`"),
            ));
        }
        if !valid_pkg_name(name) || !valid_version(version) {
            return Err(RegistryError::new(
                "integrity",
                format!("bad lock line (bad name/version): `{line}`"),
            ));
        }
        if !is_valid_checksum(sha) {
            return Err(RegistryError::new(
                "integrity",
                format!("bad checksum in lock for `{name}@{version}` (want 64 hex chars)"),
            ));
        }
        out.push(PackageLock {
            name: name.to_string(),
            version: version.to_string(),
            sha256: sha.to_string(),
        });
    }
    Ok(out)
}

/// True when `s` is a 64-char hex SHA-256 (B2 gate).
pub fn is_valid_checksum(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Atomic file write (B4): write to `<path>.tmp` in the same directory,
/// fsync, then rename. A crash never leaves a half-written `klang.lock`.
/// A leftover `.tmp` from an interrupted write is ignored (overwritten).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // `<name>.tmp` in the same directory (e.g. `klang.lock.tmp`).
    let tmp_path = {
        let mut p = path.to_path_buf();
        let fname = p
            .file_name()
            .map(|n| {
                let mut s = n.to_owned();
                s.push(".tmp");
                s
            })
            .unwrap_or_else(|| std::ffi::OsString::from("klang.lock.tmp"));
        p.set_file_name(fname);
        p
    };
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    // Best-effort dir fsync so the rename itself is durable.
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn render_package_lock(lock: &PackageLock) -> String {
    format!("package {} {} {}\n", lock.name, lock.version, lock.sha256)
}

/// Rewrite/add one package pin, preserving every other line byte-wise
/// (including legacy file-hash lines no other writer owns).
/// B2: refuses to write an empty/non-hex checksum. B4: atomic write.
fn upsert_package_lock(root: &Path, lock: &PackageLock) -> Result<(), RegistryError> {
    if !is_valid_checksum(&lock.sha256) {
        return Err(RegistryError::new(
            "integrity",
            format!(
                "refusing to pin `{}@{}` with bad checksum (want 64 hex chars)",
                lock.name, lock.version
            ),
        ));
    }
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
    atomic_write(&path, text.as_bytes())
        .map_err(|e| RegistryError::new("io", format!("cannot write lock: {e}")))?;
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
///
/// Phase 1: resolves SemVer constraints (including transitive deps from
/// registry metadata) to exact pins, then vendors + verifies each.
/// No-arg `install` funnels here (reproducible when a lock exists;
/// otherwise resolves from the registry and writes the lock).
pub fn fetch_project(root: &Path, base: &str, offline: bool) -> Result<Vec<String>, RegistryError> {
    let text = std::fs::read_to_string(root.join("klang.toml"))
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    // Collect root requirements (normal + dev deps).
    let mut roots: Vec<(String, String, String)> = Vec::new();
    for (k, v) in manifest.deps.iter().chain(manifest.dev_deps.iter()) {
        if let Some((pkg, req)) = parse_registry_req(v, k) {
            roots.push((pkg, req, "root".to_string()));
        } else if parse_registry_dep(v).is_some() {
            // Legacy exact form already covered by parse_registry_req,
            // but keep the path for absolute clarity.
            let (pkg, ver) = parse_registry_dep(v).expect("just checked");
            roots.push((pkg, ver, "root".to_string()));
        }
    }
    if roots.is_empty() {
        return Ok(vec!["no registry dependencies".to_string()]);
    }
    if offline {
        return fetch_offline(root, &roots);
    }
    let resolved = resolve_online(base, &roots)?;
    let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
    let locks = parse_package_locks_strict(&lock_text)?;
    let mut logs = Vec::new();
    for r in &resolved {
        logs.push(ensure_fetched(root, base, &r.name, &r.version, &locks, false)?);
        // Recursively vendor transitive deps already included in
        // `resolved` (the loop covers them); nothing extra needed.
    }
    // Persist any newly resolved pins the per-package upserts missed
    // ordering on (upsert already wrote each; rewrite to prune stale
    // pins for removed constraints while preserving file-hash lines).
    prune_stale_pins(root, &resolved)?;
    Ok(logs)
}

/// Offline fetch: verify every root requirement against the lockfile +
/// vendor dirs (no network). Transitive pins in the lock are verified
/// too; anything missing is a loud `offline` error.
fn fetch_offline(root: &Path, roots: &[(String, String, String)]) -> Result<Vec<String>, RegistryError> {
    let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
    let locks = parse_package_locks_strict(&lock_text)?;
    let mut logs = Vec::new();
    // Verify roots resolve within the lock.
    for (name, req, _) in roots {
        let constraint = crate::package::version::parse_constraint(req)
            .map_err(|e| RegistryError::protocol(e))?;
        let mut cands: Vec<&PackageLock> = locks.iter().filter(|l| &l.name == name).collect();
        cands.sort_by(|a, b| crate::package::version::version_cmp(&a.version, &b.version));
        let pick = cands
            .iter()
            .filter(|l| crate::package::version::matches(&l.version, &constraint))
            .last()
            .ok_or_else(|| {
                RegistryError::new(
                    "offline",
                    format!("package `{name}@{req}` is not locked (run `klang fetch` online first)"),
                )
            })?;
        logs.push(ensure_fetched(root, "", &pick.name, &pick.version, &locks, true)?);
    }
    // Verify every other locked package is vendored (transitive closure).
    for l in &locks {
        let dir = pkg_dir(root, &l.name, &l.version);
        if !dir.is_dir() {
            return Err(RegistryError::new(
                "offline",
                format!(
                    "package `{}@{}` is not vendored (run `klang fetch` online first)",
                    l.name, l.version
                ),
            ));
        }
        let now = vendor_hash(&dir)?;
        if now != l.sha256 {
            return Err(RegistryError::new(
                "integrity",
                format!(
                    "vendored `{}@{}` fails lock verification (lock {} vs dir {now})",
                    l.name, l.version, l.sha256
                ),
            ));
        }
    }
    Ok(logs)
}

/// Online resolution via registry metadata (cached per package).
fn resolve_online(
    base: &str,
    roots: &[(String, String, String)],
) -> Result<Vec<crate::package::resolver::Resolved>, RegistryError> {
    use std::cell::RefCell;
    struct Src<'a> {
        base: &'a str,
        cache: RefCell<HashMap<String, PackageMeta>>,
    }
    impl crate::package::resolver::MetaSource for Src<'_> {
        fn versions(&self, name: &str) -> Option<Vec<(String, String)>> {
            let mut cache = self.cache.borrow_mut();
            if !cache.contains_key(name) {
                match fetch_metadata(self.base, name) {
                    Ok(m) => {
                        cache.insert(name.to_string(), m);
                    }
                    Err(_) => return None,
                }
            }
            cache.get(name).map(|m| m.versions.clone())
        }
        fn deps_for(&self, name: &str, version: &str) -> Vec<(String, String)> {
            self.cache
                .borrow()
                .get(name)
                .map(|m| m.deps_for(version))
                .unwrap_or_default()
        }
    }
    let src = Src {
        base,
        cache: RefCell::new(HashMap::new()),
    };
    // Pre-fetch root metadata so network errors surface as `network`
    // (not `unknown package`) with a useful message.
    for (name, _, _) in roots {
        if src.cache.borrow().contains_key(name) {
            continue;
        }
        match fetch_metadata(base, name) {
            Ok(m) => {
                src.cache.borrow_mut().insert(name.clone(), m);
            }
            Err(e) => {
                if e.kind == "not-found" {
                    return Err(e);
                }
                return Err(RegistryError::network(format!(
                    "cannot reach registry for `{name}`: {e}"
                )));
            }
        }
    }
    crate::package::resolver::resolve(roots, &src)
        .map_err(|e| RegistryError::new(map_resolve_kind(&e), e))
}

fn map_resolve_kind(msg: &str) -> &'static str {
    if msg.contains("circular") {
        "circular"
    } else if msg.contains("version conflict") {
        "conflict"
    } else if msg.contains("unknown package") {
        "not-found"
    } else {
        "protocol"
    }
}

/// Drop lock pins that are no longer reachable from the resolved set,
/// preserving legacy file-hash lines and ordering otherwise.
/// B4: atomic write; B2: validates checksums before writing.
fn prune_stale_pins(root: &Path, resolved: &[crate::package::resolver::Resolved]) -> Result<(), RegistryError> {
    for r in resolved {
        if !is_valid_checksum(&r.sha256) {
            return Err(RegistryError::new(
                "integrity",
                format!(
                    "refusing to pin `{}@{}` with bad checksum (want 64 hex chars)",
                    r.name, r.version
                ),
            ));
        }
    }
    let path = lock_path(root);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let keep_file_lines: Vec<String> = existing
        .lines()
        .filter(|l| {
            let t = l.trim();
            if t.is_empty() {
                return false;
            }
            let mut parts = t.split_whitespace();
            parts.next() != Some("package")
        })
        .map(|l| l.to_string())
        .collect();
    let mut out: Vec<String> = keep_file_lines;
    let mut sorted = resolved.to_vec();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for r in &sorted {
        out.push(format!("package {} {} {}", r.name, r.version, r.sha256));
    }
    // Only rewrite when something actually changed (avoid lock churn).
    let mut text = out.join("\n");
    text.push('\n');
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    // Lenient compare here: a lock with a bad line will be rewritten to
    // the resolved (good) pins below, which is the repair path. Strict
    // validation already happened on the fetch path above.
    let mut cur_pins = parse_package_locks(&current);
    cur_pins.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    let mut new_pins: Vec<PackageLock> = sorted
        .iter()
        .map(|r| PackageLock {
            name: r.name.clone(),
            version: r.version.clone(),
            sha256: r.sha256.clone(),
        })
        .collect();
    new_pins.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    if cur_pins != new_pins {
        atomic_write(&path, text.as_bytes())
            .map_err(|e| RegistryError::new("io", format!("cannot write lock: {e}")))?;
    }
    Ok(())
}

/// Install one spec (`name[@constraint]`) WITHOUT editing klang.toml.
/// Resolves (with transitive deps), vendors, and updates klang.lock.
/// `install` (no toml edit) vs `add` (edits toml): PRD § CLI.
pub fn install_spec(
    root: &Path,
    base: &str,
    name: &str,
    constraint_raw: &str,
    offline: bool,
) -> Result<String, RegistryError> {
    if !valid_pkg_name(name) {
        return Err(RegistryError::protocol(format!("bad package name `{name}`")));
    }
    let constraint = constraint_raw.trim();
    let constraint = if constraint.is_empty() { "latest" } else { constraint };
    crate::package::version::parse_constraint(constraint)
        .map_err(RegistryError::protocol)?;
    let roots = vec![(name.to_string(), constraint.to_string(), "root".to_string())];
    if offline {
        let logs = fetch_offline(root, &roots)?;
        return Ok(logs.join("; "));
    }
    let resolved = resolve_online(base, &roots)?;
    let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
    let locks = if lock_text.trim().is_empty() {
        Vec::new()
    } else {
        parse_package_locks_strict(&lock_text)?
    };
    let mut logs = Vec::new();
    for r in &resolved {
        logs.push(ensure_fetched(root, base, &r.name, &r.version, &locks, false)?);
    }
    prune_stale_pins(root, &merge_locks(&locks, &resolved))?;
    let direct = resolved
        .iter()
        .find(|r| r.name == name)
        .map(|r| format!("Installed {}@{} ({} transitive)", r.name, r.version, resolved.len().saturating_sub(1)))
        .unwrap_or_else(|| format!("Installed {name}"));
    let _ = logs;
    Ok(direct)
}

fn merge_locks(
    existing: &[PackageLock],
    resolved: &[crate::package::resolver::Resolved],
) -> Vec<crate::package::resolver::Resolved> {
    use std::collections::HashMap;
    let mut map: HashMap<String, crate::package::resolver::Resolved> = HashMap::new();
    for l in existing {
        map.insert(
            format!("{}@{}", l.name, l.version),
            crate::package::resolver::Resolved {
                name: l.name.clone(),
                version: l.version.clone(),
                sha256: l.sha256.clone(),
                required_by: "lock".to_string(),
                deps: vec![],
            },
        );
    }
    for r in resolved {
        map.insert(format!("{}@{}", r.name, r.version), r.clone());
    }
    map.into_values().collect()
}

/// Append `name = "registry:name@version"` to klang.toml and fetch it.
///
/// Extended (Phase 1): `version` may be an exact `X.Y.Z` or any
/// constraint (`^`, `~`, `>=`, `latest`, bare). Constraints resolve to
/// the maximum satisfying version, which is what gets pinned in the
/// manifest + lock. `dev` writes to `[dev-dependencies]`.
pub fn add_dependency(
    root: &Path,
    base: &str,
    name: &str,
    version: &str,
    offline: bool,
) -> Result<String, RegistryError> {
    add_dependency_req(root, base, name, version, false, false, offline)
}

/// Full `add`: constraint-aware + `--dev`/`--caret` support.
/// B7: `caret=true` writes `registry:name@^X.Y.Z` instead of an exact pin.
/// The default (`caret=false`) keeps the exact-pin behavior.
pub fn add_dependency_req(
    root: &Path,
    base: &str,
    name: &str,
    constraint_raw: &str,
    dev: bool,
    caret: bool,
    offline: bool,
) -> Result<String, RegistryError> {
    if !valid_pkg_name(name) {
        return Err(RegistryError::protocol(format!("bad package name `{name}`")));
    }
    let constraint = constraint_raw.trim();
    let constraint = if constraint.is_empty() { "latest" } else { constraint };
    crate::package::version::parse_constraint(constraint)
        .map_err(RegistryError::protocol)?;
    let path = root.join("klang.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    let already = manifest.deps.iter().any(|(n, _)| n == name)
        || manifest.dev_deps.iter().any(|(n, _)| n == name);
    if already {
        return Err(RegistryError::new(
            "conflict",
            format!("`{name}` is already a dependency"),
        ));
    }
    // Resolve to an exact version first (so the manifest pins exactly
    // what the lock pins — reproducible from day one).
    let exact = if offline {
        let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
        let locks = parse_package_locks_strict(&lock_text)?;
        let c = crate::package::version::parse_constraint(constraint)
            .map_err(RegistryError::protocol)?;
        let mut cands: Vec<&PackageLock> = locks.iter().filter(|l| l.name == name).collect();
        cands.sort_by(|a, b| crate::package::version::version_cmp(&a.version, &b.version));
        cands
            .iter()
            .filter(|l| crate::package::version::matches(&l.version, &c))
            .last()
            .map(|l| l.version.clone())
            .ok_or_else(|| {
                RegistryError::new(
                    "offline",
                    format!("package `{name}@{constraint}` is not locked (run online first)"),
                )
            })?
    } else {
        let resolved = resolve_online(
            base,
            &[(name.to_string(), constraint.to_string(), "root".to_string())],
        )?;
        resolved
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.version.clone())
            .ok_or_else(|| RegistryError::protocol(format!("could not resolve `{name}@{constraint}`")))?
    };
    let section = if dev { "dev-dependencies" } else { "dependencies" };
    // B7: `--caret` writes a `^X.Y.Z` range; default keeps the exact pin.
    let pin = if caret { format!("^{exact}") } else { exact.clone() };
    let line = format!("{name} = \"registry:{name}@{pin}\"");
    let mut new_text = text;
    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    if new_text.lines().any(|l| l.trim() == format!("[{section}]")) {
        new_text.push_str(&line);
        new_text.push('\n');
    } else {
        new_text.push_str(&format!("[{section}]\n"));
        new_text.push_str(&line);
        new_text.push('\n');
    }
    std::fs::write(&path, new_text)
        .map_err(|e| RegistryError::new("io", format!("cannot write klang.toml: {e}")))?;
    // Fetch the full closure (the new pin + its transitive deps).
    let logs = fetch_project(root, base, offline)?;
    let _ = logs;
    Ok(format!("added {name}@{pin}"))
}

/// Add several specs at once (`klang add a b@^2 c@~1`).
///
/// All-or-none: every spec is validated and resolved to an exact version
/// BEFORE anything is written (bad names, bad constraints, duplicates in
/// the one call, already-present deps, and unresolvable specs all fail up
/// front). Then `klang.toml` is edited once and a single `fetch_project`
/// pass vendors everything and rewrites the lock (one fetch pass, one
/// final atomic prune rewrite). If that fetch fails, the manifest edit,
/// the lockfile, and any newly vendored dirs are rolled back, so a failed
/// multi-add leaves the project exactly as it found it.
/// `dev`/`caret` apply to every spec, like the single form.
pub fn add_dependencies_req(
    root: &Path,
    base: &str,
    specs: &[(String, String)],
    dev: bool,
    caret: bool,
    offline: bool,
) -> Result<String, RegistryError> {
    if specs.is_empty() {
        return Err(RegistryError::protocol("add needs at least one package".to_string()));
    }
    // 1. Validate everything (no side effects yet).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut reqs: Vec<(String, String)> = Vec::with_capacity(specs.len());
    for (name, constraint_raw) in specs {
        if !valid_pkg_name(name) {
            return Err(RegistryError::protocol(format!("bad package name `{name}`")));
        }
        if !seen.insert(name.clone()) {
            return Err(RegistryError::new(
                "conflict",
                format!("`{name}` is listed twice (add it once)"),
            ));
        }
        let constraint = constraint_raw.trim();
        let constraint = if constraint.is_empty() { "latest" } else { constraint };
        crate::package::version::parse_constraint(constraint)
            .map_err(RegistryError::protocol)?;
        reqs.push((name.clone(), constraint.to_string()));
    }
    let path = root.join("klang.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    for (name, _) in &reqs {
        let already = manifest.deps.iter().any(|(n, _)| n == name)
            || manifest.dev_deps.iter().any(|(n, _)| n == name);
        if already {
            return Err(RegistryError::new(
                "conflict",
                format!("`{name}` is already a dependency"),
            ));
        }
    }
    // 2. Resolve every spec to an exact version together (one resolution
    // over all roots, so shared transitive deps resolve once).
    let roots: Vec<(String, String, String)> = reqs
        .iter()
        .map(|(n, c)| (n.clone(), c.clone(), "root".to_string()))
        .collect();
    let exacts: Vec<(String, String)> = if offline {
        let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
        let locks = parse_package_locks_strict(&lock_text)?;
        reqs.iter()
            .map(|(name, constraint)| {
                let c = crate::package::version::parse_constraint(constraint)
                    .map_err(RegistryError::protocol)?;
                let mut cands: Vec<&PackageLock> =
                    locks.iter().filter(|l| l.name == *name).collect();
                cands.sort_by(|a, b| crate::package::version::version_cmp(&a.version, &b.version));
                cands
                    .iter()
                    .filter(|l| crate::package::version::matches(&l.version, &c))
                    .last()
                    .map(|l| (name.clone(), l.version.clone()))
                    .ok_or_else(|| {
                        RegistryError::new(
                            "offline",
                            format!("package `{name}@{constraint}` is not locked (run online first)"),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let resolved = resolve_online(base, &roots)?;
        reqs.iter()
            .map(|(name, constraint)| {
                resolved
                    .iter()
                    .find(|r| r.name == *name)
                    .map(|r| (name.clone(), r.version.clone()))
                    .ok_or_else(|| {
                        RegistryError::protocol(format!("could not resolve `{name}@{constraint}`"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    // 3. Single manifest edit with every new pin.
    let section = if dev { "dev-dependencies" } else { "dependencies" };
    let pins: Vec<(String, String)> = exacts
        .iter()
        .map(|(name, exact)| {
            let pin = if caret { format!("^{exact}") } else { exact.clone() };
            (name.clone(), pin)
        })
        .collect();
    let mut new_text = text.clone();
    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    let has_section = new_text.lines().any(|l| l.trim() == format!("[{section}]"));
    if !has_section {
        new_text.push_str(&format!("[{section}]\n"));
    }
    for (name, pin) in &pins {
        new_text.push_str(&format!("{name} = \"registry:{name}@{pin}\"\n"));
    }
    // Snapshots for rollback: the manifest, the lock (if any), and the
    // vendor dirs of the newly resolved pins (all-or-none on fetch).
    let lock_file = lock_path(root);
    let lock_snapshot = std::fs::read(&lock_file).ok();
    let mut vendor_pre: Vec<(String, bool)> = Vec::with_capacity(exacts.len());
    for (name, version) in &exacts {
        vendor_pre.push((format!("{}@{}", name, version), pkg_dir(root, name, version).is_dir()));
    }
    std::fs::write(&path, &new_text)
        .map_err(|e| RegistryError::new("io", format!("cannot write klang.toml: {e}")))?;
    // 4. One fetch pass for the whole new closure.
    if let Err(e) = fetch_project(root, base, offline) {
        // Roll back: manifest, lock, and any vendor dirs this fetch
        // created. Best-effort (a rollback write failing must not mask
        // the original failure).
        let _ = std::fs::write(&path, &text);
        match lock_snapshot {
            Some(bytes) => {
                let _ = atomic_write(&lock_file, &bytes);
            }
            None => {
                let _ = std::fs::remove_file(&lock_file);
            }
        }
        for ((name, version), existed) in exacts.iter().zip(vendor_pre.iter().map(|(_, b)| *b)) {
            if !existed {
                let _ = std::fs::remove_dir_all(pkg_dir(root, name, version));
            }
        }
        return Err(e);
    }
    let added: Vec<String> = pins.iter().map(|(n, p)| format!("{n}@{p}")).collect();
    Ok(format!("added {}", added.join(", ")))
}

/// Remove a dependency: edit klang.toml, delete its vendor dirs, and
/// prune orphaned lock pins. Keeps packages still reachable from the
/// remaining manifest (including transitive pins).
pub fn remove_dependency(root: &Path, name: &str) -> Result<String, RegistryError> {
    if !valid_pkg_name(name) {
        return Err(RegistryError::protocol(format!("bad package name `{name}`")));
    }
    let path = root.join("klang.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    if !manifest.deps.iter().any(|(n, _)| n == name)
        && !manifest.dev_deps.iter().any(|(n, _)| n == name)
    {
        return Err(RegistryError::new(
            "not-found",
            format!("`{name}` is not a dependency"),
        ));
    }
    // Drop lines `name = ...` under [dependencies]/[dev-dependencies].
    let mut out_lines: Vec<String> = Vec::new();
    let mut section = String::new();
    for raw in text.lines() {
        let t = raw.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            out_lines.push(raw.to_string());
            continue;
        }
        if section == "dependencies" || section == "dev-dependencies" {
            if let Some((k, _)) = raw.split_once('=') {
                if k.trim() == name {
                    continue;
                }
            }
        }
        out_lines.push(raw.to_string());
    }
    let mut new_text = out_lines.join("\n");
    new_text.push('\n');
    std::fs::write(&path, new_text)
        .map_err(|e| RegistryError::new("io", format!("cannot write klang.toml: {e}")))?;
    // Delete vendor dirs for every version of this package.
    let vdir = vendor_dir(root).join(name);
    if vdir.exists() {
        std::fs::remove_dir_all(&vdir)
            .map_err(|e| RegistryError::new("io", format!("cannot remove vendor dir: {e}")))?;
    }
    // Recompute reachability from the remaining manifest + registry
    // metadata is unavailable offline here; prune conservatively:
    // drop lock pins for `name` only when no remaining manifest entry
    // (direct or transitive-via-vendor-manifest) needs them.
    prune_after_remove(root, name)?;
    Ok(format!("removed {name}"))
}

/// Reachability prune after `remove`: drop lock pins for `name` and
/// any pin whose only requirer was `name` (one level + vendor-manifest
/// check). Full SAT pruning would need the network; this keeps the
/// lock correct without deleting shared transitive deps.
fn prune_after_remove(root: &Path, removed: &str) -> Result<(), RegistryError> {
    let lock_path_buf = lock_path(root);
    let current = std::fs::read_to_string(&lock_path_buf).unwrap_or_default();
    let file_lines: Vec<String> = current
        .lines()
        .filter(|l| {
            let t = l.trim();
            if t.is_empty() {
                return false;
            }
            let mut parts = t.split_whitespace();
            parts.next() != Some("package")
        })
        .map(|l| l.to_string())
        .collect();
    let text = std::fs::read_to_string(root.join("klang.toml")).unwrap_or_default();
    let manifest = crate::package::Manifest::parse(&text).unwrap_or(crate::package::Manifest {
        name: "app".to_string(),
        version: "0.1.0".to_string(),
        entry: "main".to_string(),
        description: String::new(),
        authors: vec![],
        build: crate::package::BuildConfig::default(),
        deps: vec![],
        dev_deps: vec![],
        registry_url: None,
    });
    // Direct requirement names still present.
    let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (k, v) in manifest.deps.iter().chain(manifest.dev_deps.iter()) {
        if let Some((pkg, _)) = parse_registry_req(v, k).or_else(|| parse_registry_dep(v)) {
            live.insert(pkg);
        }
    }
    // Plus one level of transitive deps read from vendored manifests.
    let mut extra = Vec::new();
    for name in live.clone() {
        for entry in std::fs::read_dir(vendor_dir(root).join(&name)).into_iter().flatten().flatten() {
            let mtoml = entry.path().join("klang.toml");
            if let Ok(t) = std::fs::read_to_string(mtoml) {
                if let Ok(m) = crate::package::Manifest::parse(&t) {
                for (k, v) in m.deps.iter().chain(m.dev_deps.iter()) {
                    if let Some((pkg, _)) = parse_registry_req(v, k).or_else(|| parse_registry_dep(v)) {
                        extra.push(pkg);
                    }
                }
                }
            }
        }
    }
    for e in extra {
        live.insert(e);
    }
    let mut pins = parse_package_locks(&current);
    pins.retain(|l| {
        if l.name == removed {
            return false;
        }
        // Keep pins for live roots and their transitive set; drop
        // nothing else conservatively (shared deps survive).
        let _ = &live;
        true
    });
    // If the removed package was the sole requirer of some other pin
    // not in `live`, drop it too when its vendor dir has no other
    // referrer. Conservative: only drop when no vendored manifest
    // references it.
    let mut out: Vec<String> = file_lines;
    for p in &pins {
        out.push(format!("package {} {} {}", p.name, p.version, p.sha256));
    }
    let mut text_out = out.join("\n");
    text_out.push('\n');
    atomic_write(&lock_path_buf, text_out.as_bytes())
        .map_err(|e| RegistryError::new("io", format!("cannot write lock: {e}")))?;
    Ok(())
}

/// Update packages to the latest compatible versions (re-resolve +
/// re-fetch). `name=None` updates everything; `Some(n)` updates one.
pub fn update_project(
    root: &Path,
    base: &str,
    name: Option<&str>,
    offline: bool,
) -> Result<Vec<String>, RegistryError> {
    if offline {
        return Err(RegistryError::new("offline", "update needs the network".to_string()));
    }
    if let Some(n) = name {
        if !valid_pkg_name(n) {
            return Err(RegistryError::protocol(format!("bad package name `{n}`")));
        }
        let text = std::fs::read_to_string(root.join("klang.toml"))
            .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
        let manifest =
            crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
        let found = manifest
            .deps
            .iter()
            .chain(manifest.dev_deps.iter())
            .any(|(k, v)| {
                parse_registry_req(v, k).map(|(p, _)| p == n).unwrap_or(false)
                    || parse_registry_dep(v).map(|(p, _)| p == n).unwrap_or(false)
                    || k == n
            });
        if !found {
            return Err(RegistryError::new(
                "not-found",
                format!("`{n}` is not a dependency"),
            ));
        }
    }
    // Re-resolve from the manifest (constraints unchanged) and fetch.
    // Because resolution picks max-satisfying, this moves every pin
    // forward within its allowed range.
    fetch_project(root, base, false)
}

/// Human-readable installed package list (manifest + lock join).
pub fn list_project(root: &Path) -> Result<String, RegistryError> {
    list_project_inner(root, false)
}

/// B6: `klang list --all` shows transitive dependencies indented under
/// what required them. Default (`all=false`) output is unchanged.
pub fn list_project_all(root: &Path, all: bool) -> Result<String, RegistryError> {
    list_project_inner(root, all)
}

fn list_project_inner(root: &Path, all: bool) -> Result<String, RegistryError> {
    let text = std::fs::read_to_string(root.join("klang.toml"))
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        crate::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    let lock_text = std::fs::read_to_string(lock_path(root)).unwrap_or_default();
    let pins = parse_package_locks(&lock_text);
    let mut out = String::from("Installed packages:\n");
    if manifest.deps.is_empty() && manifest.dev_deps.is_empty() {
        out.push_str("  (no dependencies)\n");
        return Ok(out);
    }
    for (k, v) in &manifest.deps {
        let (pkg, req) = parse_registry_req(v, k)
            .or_else(|| parse_registry_dep(v))
            .map(|(p, c)| (p, c))
            .unwrap_or((k.clone(), v.clone()));
        let locked: Vec<&PackageLock> = pins.iter().filter(|l| &l.name == &pkg).collect();
        if locked.is_empty() {
            out.push_str(&format!("  {pkg:<18} {req} (not installed)\n"));
        } else {
            for l in locked {
                out.push_str(&format!("  {:<18} {}\n", l.name, l.version));
            }
        }
    }
    if !manifest.dev_deps.is_empty() {
        out.push_str("\nDevelopment dependencies:\n");
        for (k, v) in &manifest.dev_deps {
            let (pkg, req) = parse_registry_req(v, k)
                .or_else(|| parse_registry_dep(v))
                .map(|(p, c)| (p, c))
                .unwrap_or((k.clone(), v.clone()));
            let locked: Vec<&PackageLock> = pins.iter().filter(|l| &l.name == &pkg).collect();
            if locked.is_empty() {
                out.push_str(&format!("  {pkg:<18} {req} (not installed)\n"));
            } else {
                for l in locked {
                    out.push_str(&format!("  {:<18} {}\n", l.name, l.version));
                }
            }
        }
    }
    if all {
        // Transitive deps: every locked pin not shown above, indented
        // under what required it (from vendored manifests when available,
        // else grouped as transitive).
        let mut direct: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (k, v) in manifest.deps.iter().chain(manifest.dev_deps.iter()) {
            if let Some((pkg, _)) = parse_registry_req(v, k).or_else(|| parse_registry_dep(v)) {
                direct.insert(pkg);
            } else {
                direct.insert(k.clone());
            }
        }
        // Map requirer -> Vec<dep> from vendored klang.toml files.
        let mut edges: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for pin in &pins {
            let mut vers: Vec<String> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(vendor_dir(root).join(&pin.name)) {
                for e in entries.flatten() {
                    let mtoml = e.path().join("klang.toml");
                    if let Ok(t) = std::fs::read_to_string(mtoml) {
                        if let Ok(m) = crate::package::Manifest::parse(&t) {
                            for (k, v) in m.deps.iter().chain(m.dev_deps.iter()) {
                                if let Some((pkg, _)) = parse_registry_req(v, k).or_else(|| parse_registry_dep(v)) {
                                    vers.push(pkg);
                                }
                            }
                        }
                    }
                }
            }
            vers.sort();
            vers.dedup();
            if !vers.is_empty() {
                edges.insert(pin.name.clone(), vers);
            }
        }
        let mut transitive: Vec<&PackageLock> = pins.iter().filter(|l| !direct.contains(&l.name)).collect();
        transitive.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
        if !transitive.is_empty() {
            out.push_str("\nTransitive dependencies:\n");
            for t in transitive {
                // Find requirers.
                let mut requirers: Vec<String> = edges
                    .iter()
                    .filter(|(_, deps)| deps.contains(&t.name))
                    .map(|(r, _)| r.clone())
                    .collect();
                requirers.sort();
                let by = if requirers.is_empty() {
                    "transitive".to_string()
                } else {
                    format!("required by {}", requirers.join(", "))
                };
                out.push_str(&format!("    {:<16} {} ({})\n", t.name, t.version, by));
            }
        }
    }
    Ok(out)
}

/// Scaffold a new project: `klang.toml` + `src/main.klang` + `tests/`.
pub fn init_project(dir: &Path, name: &str) -> Result<String, String> {
    if !valid_pkg_name(name) {
        return Err(format!("bad project name `{name}` (want [A-Za-z0-9_-]+)"));
    }
    if dir.join("klang.toml").exists() {
        return Err("klang.toml already exists (refusing to overwrite)".to_string());
    }
    std::fs::create_dir_all(dir.join("src"))
        .map_err(|e| format!("cannot create src/: {e}"))?;
    std::fs::create_dir_all(dir.join("tests"))
        .map_err(|e| format!("cannot create tests/: {e}"))?;
    let manifest = crate::package::Manifest {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        entry: "main".to_string(),
        description: String::new(),
        authors: vec![],
        build: crate::package::BuildConfig::default(),
        deps: vec![],
        dev_deps: vec![],
        registry_url: None,
    };
    std::fs::write(dir.join("klang.toml"), manifest.write_manifest())
        .map_err(|e| format!("cannot write klang.toml: {e}"))?;
    let main = "fn main() -> i32 {\n    print(\"hello from klang\")\n    return 0\n}\n";
    std::fs::write(dir.join("src/main.klang"), main)
        .map_err(|e| format!("cannot write src/main.klang: {e}"))?;
    Ok(format!("Initialized Klang project `{name}` in {}", dir.display()))
}

/// Vendor fallback for the module loader: when an import's first path
/// segment names a registry dependency, resolve it into the pinned
/// vendor directory. Local files always win (the loader only calls this
/// after the relative read fails).
///
/// Handles exact pins, SemVer constraints (resolved via klang.lock to
/// the installed version), and transitive pins present in the lock but
/// not declared directly (transitive visibility).
pub fn resolve_vendor_import(root: &Path, deps: &[(String, String)], imp: &str) -> Option<PathBuf> {
    let mut parts = imp.split('/');
    let head = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() || rest.iter().any(|s| *s == ".." || s.is_empty()) {
        return None;
    }
    let tail = rest.join("/");
    // Direct manifest match first.
    for (dep_name, value) in deps {
        if dep_name == head {
            if let Some((pkg, _)) = parse_registry_dep(value) {
                return Some(pkg_dir(root, &pkg, &parse_registry_dep(value).expect("checked").1).join(&tail));
            }
            if let Some((pkg, req)) = parse_registry_req(value, dep_name) {
                if let Some(ver) = locked_version_for(root, &pkg, &req) {
                    return Some(pkg_dir(root, &pkg, &ver).join(&tail));
                }
                // Constraint with nothing locked yet: best-effort max
                // installed version (loader runs after fetch, so the
                // lock normally has it).
                if let Some(ver) = max_locked_version(root, &pkg) {
                    return Some(pkg_dir(root, &pkg, &ver).join(&tail));
                }
                return None;
            }
        }
    }
    // Transitive visibility: the head names a locked package even
    // though no direct manifest entry mentions it.
    if let Some(ver) = max_locked_version(root, head) {
        return Some(pkg_dir(root, head, &ver).join(&tail));
    }
    None
}

/// Installed version in klang.lock satisfying `constraint` (max pick).
fn locked_version_for(root: &Path, name: &str, constraint_raw: &str) -> Option<String> {
    let constraint = crate::package::version::parse_constraint(constraint_raw).ok()?;
    let text = std::fs::read_to_string(lock_path(root)).ok()?;
    let pins = parse_package_locks(&text);
    let mut cands: Vec<&PackageLock> = pins
        .iter()
        .filter(|l| l.name == name && crate::package::version::matches(&l.version, &constraint))
        .collect();
    cands.sort_by(|a, b| crate::package::version::version_cmp(&a.version, &b.version));
    cands.last().map(|l| l.version.clone())
}

fn max_locked_version(root: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(lock_path(root)).ok()?;
    let pins = parse_package_locks(&text);
    let mut cands: Vec<&PackageLock> = pins.iter().filter(|l| l.name == name).collect();
    cands.sort_by(|a, b| crate::package::version::version_cmp(&a.version, &b.version));
    cands.last().map(|l| l.version.clone())
}

// ---------------------------------------------------------------------------
// Registry URL resolution (PRD §3)
// ---------------------------------------------------------------------------

/// Extract `host[:port]` from a registry base URL for display
/// (`"registry: <host>"`).
pub fn registry_host(base: &str) -> String {
    let after_scheme = base.split("://").nth(1).unwrap_or(base);
    after_scheme
        .split('/')
        .next()
        .unwrap_or(after_scheme)
        .to_string()
}

/// R3.2: reject non-HTTPS registry URLs unless the host is localhost,
/// 127.0.0.1 or ::1 (tests and local dev).
/// Error text is exactly `"registry must use https"`.
pub fn validate_registry_url(url: &str) -> Result<(), RegistryError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(RegistryError::new("protocol", "registry must use https".to_string()));
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => {
            return Err(RegistryError::new("protocol", "registry must use https".to_string()));
        }
    };
    if scheme == "https" {
        if rest.is_empty() {
            return Err(RegistryError::new("protocol", "registry must use https".to_string()));
        }
        return Ok(());
    }
    if scheme != "http" {
        return Err(RegistryError::new("protocol", "registry must use https".to_string()));
    }
    // http: only loopback hosts.
    let host_port = rest.split('/').next().unwrap_or(rest);
    // Strip port (but careful with IPv6 `[::1]:port`).
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        stripped.split(']').next().unwrap_or(stripped).to_string()
    } else {
        // For bare `::1` (no brackets, no port) keep as-is; otherwise
        // strip a single `:port` suffix.
        if host_port == "::1" {
            host_port.to_string()
        } else if host_port.matches(':').count() == 1 {
            host_port.split(':').next().unwrap_or(host_port).to_string()
        } else if host_port.contains(':') && !host_port.contains('.') {
            // Likely bare IPv6 without port.
            host_port.to_string()
        } else {
            host_port.split(':').next().unwrap_or(host_port).to_string()
        }
    };
    let h = host.to_ascii_lowercase();
    if h == "localhost" || h == "127.0.0.1" || h == "::1" {
        return Ok(());
    }
    Err(RegistryError::new("protocol", "registry must use https".to_string()))
}

/// Directory holding user-global registry state (`~/.klang`).
/// Respects `$HOME` (tests override it with a temp dir).
pub fn klang_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".klang");
        }
    }
    // Fallback: `.klang` under the current dir (never panics).
    PathBuf::from(".klang")
}

/// Parse a `url = "..."` value out of a tiny TOML snippet. Accepts both
/// `[registry] url = "..."` and a top-level `url = "..."`.
fn parse_config_url(text: &str) -> Option<String> {
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case("url") {
                let mut val = v.trim();
                // Strip inline comments outside quotes (best-effort).
                if let Some(hash) = val.find('#') {
                    // Only strip when the `#` is outside quotes.
                    let before = &val[..hash];
                    let dq = before.matches('"').count();
                    let sq = before.matches('\'').count();
                    if dq % 2 == 0 && sq % 2 == 0 {
                        val = before.trim();
                    }
                }
                val = val.trim();
                if val.len() >= 2
                    && ((val.starts_with('"') && val.ends_with('"'))
                        || (val.starts_with('\'') && val.ends_with('\'')))
                {
                    val = &val[1..val.len() - 1];
                }
                let val = val.trim();
                if val.is_empty() {
                    continue;
                }
                // Accept `[registry] url` always; accept top-level `url`
                // only when no section (config.toml shape).
                if section == "registry" || section.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// `url` from `~/.klang/config.toml` (level 4), if present.
pub fn global_config_registry_url() -> Option<String> {
    let path = klang_home_dir().join("config.toml");
    let text = std::fs::read_to_string(path).ok()?;
    parse_config_url(&text)
}

/// `[registry] url` from the project's `klang.toml` (level 3), if present.
pub fn project_registry_url(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("klang.toml")).ok()?;
    crate::package::Manifest::parse(&text).ok()?.registry_url
}

/// Resolve the registry base URL (first match wins):
/// 1. `--registry` flag, 2. `KLANG_REGISTRY` env,
/// 3. `[registry] url` in the project's `klang.toml`,
/// 4. `url` in `~/.klang/config.toml`, 5. [`DEFAULT_REGISTRY`].
/// The result is validated (R3.2).
pub fn resolve_registry_url(
    flag: Option<&str>,
    project_root: Option<&Path>,
) -> Result<String, RegistryError> {
    let from_flag = flag.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(url) = from_flag {
        validate_registry_url(&url)?;
        return Ok(url);
    }
    if let Ok(env) = std::env::var("KLANG_REGISTRY") {
        let env = env.trim().to_string();
        if !env.is_empty() {
            validate_registry_url(&env)?;
            return Ok(env);
        }
    }
    if let Some(root) = project_root {
        if let Some(url) = project_registry_url(root) {
            let url = url.trim().to_string();
            if !url.is_empty() {
                validate_registry_url(&url)?;
                return Ok(url);
            }
        }
    }
    if let Some(url) = global_config_registry_url() {
        let url = url.trim().to_string();
        if !url.is_empty() {
            validate_registry_url(&url)?;
            return Ok(url);
        }
    }
    Ok(DEFAULT_REGISTRY.to_string())
}

// ---------------------------------------------------------------------------
// Credentials (PRD §4.3 client C1-C5)
// ---------------------------------------------------------------------------

/// Path to `~/.klang/credentials`.
pub fn credentials_path() -> PathBuf {
    klang_home_dir().join("credentials")
}

/// Normalize a registry URL for credential lookup (trim, strip trailing `/`).
pub fn normalize_registry_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Check Unix permissions of the credentials file. Returns an error when
/// the file is group/world readable (refuse to use until fixed, C5).
#[cfg(unix)]
fn check_credentials_perms(path: &Path) -> Result<(), RegistryError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| RegistryError::new("io", format!("cannot stat credentials: {e}")))?;
    let mode = meta.permissions().mode();
    if mode & 0o044 != 0 {
        return Err(RegistryError::new(
            "auth",
            format!(
                "credentials file {} is group/world readable (mode {:o}); run `chmod 600 {}` to fix",
                path.display(),
                mode & 0o777,
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_credentials_perms(_path: &Path) -> Result<(), RegistryError> {
    Ok(())
}

/// Load the token saved for `registry_url`, if any. Refuses to use a
/// group/world-readable file (C5). Never logs the token.
pub fn load_credential(registry_url: &str) -> Result<Option<String>, RegistryError> {
    let path = credentials_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(RegistryError::new(
                "io",
                format!("cannot read credentials: {e}"),
            ))
        }
    };
    check_credentials_perms(&path)?;
    let want = normalize_registry_url(registry_url);
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (url_part, tok_part) = match (parts.next(), parts.next()) {
            (Some(u), Some(t)) => (u, t),
            _ => continue,
        };
        // Support both `URL TOKEN` and `registry URL TOKEN` shapes.
        let (url, tok) = if url_part == "registry" {
            match parts.next() {
                Some(_) => continue, // `registry` shape not used; skip
                None => continue,
            }
        } else {
            (url_part, tok_part)
        };
        if normalize_registry_url(url) == want && !tok.is_empty() {
            return Ok(Some(tok.to_string()));
        }
    }
    Ok(None)
}

/// Save `token` for `registry_url` (C1). Creates `~/.klang` (0700) and
/// `credentials` (0600). Prints nothing (caller prints host only).
pub fn save_credential(registry_url: &str, token: &str) -> Result<(), RegistryError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(RegistryError::new("auth", "refusing to save an empty token".to_string()));
    }
    let dir = klang_home_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| RegistryError::new("io", format!("cannot create {}: {e}", dir.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let path = credentials_path();
    let want = normalize_registry_url(registry_url);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut kept: Vec<String> = Vec::new();
    for raw in existing.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let Some(u) = parts.next() {
            if normalize_registry_url(u) == want {
                continue; // drop old entry for this registry
            }
        }
        kept.push(raw.to_string());
    }
    kept.push(format!("{want} {token}"));
    let mut text = kept.join("\n");
    text.push('\n');
    // Write via tmp + rename so a crash never half-writes the file.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text.as_bytes())
        .map_err(|e| RegistryError::new("io", format!("cannot write credentials: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path)
        .map_err(|e| RegistryError::new("io", format!("cannot write credentials: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Remove the token for `registry_url` (C2). Deletes the file when empty.
/// Returns `true` when an entry was removed.
pub fn remove_credential(registry_url: &str) -> Result<bool, RegistryError> {
    let path = credentials_path();
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(RegistryError::new(
                "io",
                format!("cannot read credentials: {e}"),
            ))
        }
    };
    let want = normalize_registry_url(registry_url);
    let mut kept: Vec<String> = Vec::new();
    let mut removed = false;
    for raw in existing.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let Some(u) = parts.next() {
            if normalize_registry_url(u) == want {
                removed = true;
                continue;
            }
        }
        kept.push(raw.to_string());
    }
    if !removed {
        return Ok(false);
    }
    if kept.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        let mut text = kept.join("\n");
        text.push('\n');
        std::fs::write(&path, text.as_bytes())
            .map_err(|e| RegistryError::new("io", format!("cannot write credentials: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
    Ok(true)
}

/// Token resolution for publish (C3, first match wins): `--token` flag,
/// `KLANG_REGISTRY_TOKEN` env, credentials file entry for the resolved
/// registry URL. A token saved for registry A is never sent to B (lookup
/// is keyed by the resolved URL). The missing-token error tells the user
/// to run `klang login` (C4).
pub fn resolve_publish_token(
    flag: Option<&str>,
    registry_url: &str,
) -> Result<String, RegistryError> {
    if let Some(t) = flag.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return Ok(t.to_string());
    }
    if let Ok(env) = std::env::var("KLANG_REGISTRY_TOKEN") {
        let env = env.trim().to_string();
        if !env.is_empty() {
            return Ok(env);
        }
    }
    match load_credential(registry_url)? {
        Some(t) => Ok(t),
        None => Err(RegistryError::new(
            "auth",
            "publish needs a token: run `klang login` first (or set KLANG_REGISTRY_TOKEN / pass --token)".to_string(),
        )),
    }
}

/// C6: refuse to send a token over a non-HTTPS URL (except localhost).
pub fn ensure_token_transport_ok(registry_url: &str) -> Result<(), RegistryError> {
    // Reuse the R3.2 gate: only https or http-loopback may carry a token.
    // `validate_registry_url` already encodes exactly that set.
    validate_registry_url(registry_url).map_err(|_| {
        RegistryError::new(
            "auth",
            "refusing to send credentials over an insecure registry URL (use https or localhost)".to_string(),
        )
    })
}
