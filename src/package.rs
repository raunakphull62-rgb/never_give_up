//! Stage 4: unified toolchain + package graph (foundation).
//!
//! Single blessed CLI: `klang build/run/test/fmt/check/doc`.
//! One manifest as source of truth + content-addressed lockfile.

/// Parsed `klang.toml` manifest (minimal v0.1 shape + local path deps).
///
/// ```toml
/// name = "demo"
/// version = "0.1.0"
/// entry = "main"
/// [dependencies]
/// mylib = "./mylib.klang"
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub entry: String,
    /// (dep name, local path) pairs from `[dependencies]`.
    pub deps: Vec<(String, String)>,
}

impl Manifest {
    /// Minimal `key = "value"` parser (no external TOML dep yet).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut version: Option<String> = None;
        let mut entry: Option<String> = None;
        let mut deps: Vec<(String, String)> = Vec::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let val = v.trim().trim_matches('"').trim_matches('\'').to_string();
                if section == "dependencies" {
                    deps.push((k.trim().to_string(), val));
                } else {
                    match k.trim() {
                        "name" => name = Some(val),
                        "version" => version = Some(val),
                        "entry" => entry = Some(val),
                        _ => {}
                    }
                }
            }
        }
        Ok(Self {
            name: name.unwrap_or_else(|| "app".to_string()),
            version: version.unwrap_or_else(|| "0.1.0".to_string()),
            entry: entry.unwrap_or_else(|| "combine".to_string()),
            deps,
        })
    }
}

/// Content-addressed lock entry (FNV-1a hash of file bytes, std-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    pub file: String,
    pub hash: u64,
}

pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn lock_for(files: &[(String, String)]) -> Vec<LockEntry> {
    files
        .iter()
        .map(|(name, content)| LockEntry {
            file: name.clone(),
            hash: content_hash(content.as_bytes()),
        })
        .collect()
}

/// Serialize a lockfile: one `"<file> <hex-hash>"` line per entry.
pub fn write_lock(entries: &[LockEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!("{} {:016x}\n", e.file, e.hash));
    }
    out
}

/// Parse `write_lock` output back. Malformed lines are skipped.
pub fn parse_lock(text: &str) -> Vec<LockEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let (Some(file), Some(hash_s)) = (parts.next(), parts.next()) {
            if let Ok(hash) = u64::from_str_radix(hash_s.trim_start_matches("0x"), 16) {
                out.push(LockEntry {
                    file: file.to_string(),
                    hash,
                });
            }
        }
    }
    out
}

/// Verify current file contents match the lock. `Err` lists every mismatch.
pub fn verify_lock(files: &[(String, String)], locks: &[LockEntry]) -> Result<(), String> {
    let mut problems = Vec::new();
    for lock in locks {
        match files.iter().find(|(n, _)| n == &lock.file) {
            None => problems.push(format!("missing locked file `{}`", lock.file)),
            Some((_, content)) => {
                let h = content_hash(content.as_bytes());
                if h != lock.hash {
                    problems.push(format!(
                        "hash mismatch for `{}`: lock {:016x} vs now {:016x}",
                        lock.file, lock.hash, h
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Blessed subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Build,
    Run,
    Test,
    Fmt,
    Check,
    Doc,
}

impl Command {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "build" => Some(Self::Build),
            "run" => Some(Self::Run),
            "test" => Some(Self::Test),
            "fmt" => Some(Self::Fmt),
            "check" => Some(Self::Check),
            "doc" => Some(Self::Doc),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Run => "run",
            Self::Test => "test",
            Self::Fmt => "fmt",
            Self::Check => "check",
            Self::Doc => "doc",
        }
    }
}
