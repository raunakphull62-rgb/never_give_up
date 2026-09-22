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
    /// Strips inline `#` comments outside quotes so
    /// `entry = "main" # comment` parses as `main`, not `main" # comment`.
    /// Unknown keys are ignored (forward-compat); missing fields default.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut version: Option<String> = None;
        let mut entry: Option<String> = None;
        let mut deps: Vec<(String, String)> = Vec::new();
        let mut section = String::new();
        for raw in text.lines() {
            let line = strip_inline_comment(raw).trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let val = parse_toml_value(v.trim());
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

/// Strip an inline `#` comment, respecting single/double quotes.
fn strip_inline_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (i, c) in line.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return line[..i].trim_end(),
            _ => {}
        }
    }
    line
}

/// Parse a TOML value: quoted strings (single/double) unwrap, bare values
/// trim. `entry = "main" # x` -> `main` (comment already stripped).
fn parse_toml_value(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"'))
            || (v.starts_with('\'') && v.ends_with('\'')))
    {
        v[1..v.len() - 1].to_string()
    } else {
        // Bare or malformed: strip trailing quotes if any, trim.
        v.trim_matches('"').trim_matches('\'').trim().to_string()
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
/// Also reports unlocked (extra) files: previously `verify_lock` ignored
/// files not listed in the lock, so new malicious files passed silently.
/// FNV-1a is change-detection, not collision-resistant: do not rely on it
/// alone for adversarial integrity (use out-of-band signatures for that).
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
    for (name, _) in files {
        if !locks.iter().any(|l| &l.file == name) {
            problems.push(format!("unlocked file `{name}` not in lockfile"));
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
