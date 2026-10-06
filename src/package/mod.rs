//! Stage 4: unified toolchain + package graph (foundation).
//!
//! Single blessed CLI: `klang build/run/test/fmt/check/doc`.
//! One manifest as source of truth + content-addressed lockfile.

//! v2 manifest keys (`klang`, `schemas`, `repair`) live in [`manifest`];
//! they parse offline and default so v1 files keep working.

/// v2 `klang.toml` keys (offline; no registry).
pub mod manifest;

/// Semantic version constraints (exact, `=`, `^`, `~`, `>`, `>=`,
/// `<`, `<=`, `*`/`latest`, whitespace AND groups).
pub mod version;

/// Dependency resolution (SemVer + transitive closure + conflicts).
pub mod resolver;

/// Build configuration from `[build]` (parsed; inert in Phase 1 —
/// the compiler does not read it yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildConfig {
    /// e.g. `"release"` / `"debug"`.
    pub target: String,
    /// e.g. `3`.
    pub optimization_level: u32,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            target: "debug".to_string(),
            optimization_level: 0,
        }
    }
}

/// Parsed `klang.toml` manifest (minimal v0.1 shape + local path deps).
///
/// ```toml
/// name = "demo"
/// version = "0.1.0"
/// entry = "main"
/// [dependencies]
/// mylib = "./mylib.klang"
/// ```
///
/// Phase 1 extensions (all backcompat):
/// - `[project]` is an alias for top-level `name`/`version`/`entry`
///   (PRD shape) and also carries `description` + `authors`.
/// - `[dev-dependencies]` parses into `dev_deps` (same value shapes).
/// - `[build]` parses into [`BuildConfig`] (inert in Phase 1).
/// - Dependency values may be exact pins (`registry:name@1.2.3`),
///   SemVer constraints (`^1.2.0`, `~1.2.0`, `>=1.2.0`, `<2.0.0`,
///   `*`, `latest`, bare `1.2.3`), or local paths.
/// - [`Manifest::write_manifest`] serializes back deterministically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub entry: String,
    /// Free-text description from `[project]`.
    pub description: String,
    /// Author list from `[project]` (`authors = ["Name"]`).
    pub authors: Vec<String>,
    /// `[build]` config (inert in Phase 1).
    pub build: BuildConfig,
    /// (dep name, raw value) pairs from `[dependencies]`.
    pub deps: Vec<(String, String)>,
    /// (dep name, raw value) pairs from `[dev-dependencies]`.
    pub dev_deps: Vec<(String, String)>,
    /// Optional `[registry] url = "..."` (R3.1). `None` when absent.
    /// Only `url` is accepted in `[registry]`; other keys are ignored.
    pub registry_url: Option<String>,
}

impl Manifest {
    /// Minimal `key = "value"` parser (no external TOML dep yet).
    /// Strips inline `#` comments outside quotes so
    /// `entry = "main" # comment` parses as `main`, not `main" # comment`.
    /// Unknown keys are ignored (forward-compat); missing fields default.
    /// `[project]` feeds `name`/`version`/`entry`/`description`/`authors`;
    /// `[dev-dependencies]` feeds `dev_deps`; `[build]` feeds [`BuildConfig`].
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut version: Option<String> = None;
        let mut entry: Option<String> = None;
        let mut description: Option<String> = None;
        let mut authors: Vec<String> = Vec::new();
        let mut build = BuildConfig::default();
        let mut build_seen = false;
        let mut deps: Vec<(String, String)> = Vec::new();
        let mut dev_deps: Vec<(String, String)> = Vec::new();
        let mut registry_url: Option<String> = None;
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
                let key = k.trim();
                let val_raw = v.trim();
                let val = parse_toml_value(val_raw);
                if section == "dependencies" {
                    deps.push((key.to_string(), val));
                } else if section == "dev-dependencies" {
                    dev_deps.push((key.to_string(), val));
                } else if section == "project" {
                    match key {
                        "name" => name = Some(val),
                        "version" => version = Some(val),
                        "entry" => entry = Some(val),
                        "description" => description = Some(val),
                        "authors" => authors = parse_string_array(val_raw),
                        _ => {}
                    }
                } else if section == "build" {
                    build_seen = true;
                    match key {
                        "target" => build.target = val,
                        "optimization-level" | "optimization_level" => {
                            if let Ok(n) = val.parse::<u32>() {
                                build.optimization_level = n;
                            }
                        }
                        _ => {}
                    }
                } else if section == "registry" {
                    // R3.1: only `url` is accepted.
                    if key == "url" && !val.is_empty() {
                        registry_url = Some(val);
                    }
                } else if section.is_empty() || section == "package" {
                    match key {
                        "name" => name = Some(val),
                        "version" => version = Some(val),
                        "entry" => entry = Some(val),
                        "description" => description = Some(val),
                        "authors" => authors = parse_string_array(val_raw),
                        _ => {}
                    }
                } else {
                    // Unknown sections ([repair], ...) ignored.
                }
            }
        }
        Ok(Self {
            name: name.unwrap_or_else(|| "app".to_string()),
            version: version.unwrap_or_else(|| "0.1.0".to_string()),
            entry: entry.unwrap_or_else(|| "combine".to_string()),
            description: description.unwrap_or_default(),
            authors,
            build: if build_seen {
                build
            } else {
                BuildConfig::default()
            },
            deps,
            dev_deps,
            registry_url,
        })
    }

    /// All registry-relevant deps (normal + dev).
    pub fn all_deps(&self) -> Vec<(String, String)> {
        let mut out = self.deps.clone();
        out.extend(self.dev_deps.clone());
        out
    }

    /// Serialize deterministically (PRD `write_manifest`). Sections in
    /// fixed order; `[dev-dependencies]` and `[build]` are emitted only
    /// when non-default so minimal manifests stay minimal. This is a
    /// full rewrite (comments/formatting are not preserved) — used for
    /// fresh files (`klang init`); `add`/`remove` keep line-splicing to
    /// preserve user formatting.
    pub fn write_manifest(&self) -> String {
        let mut out = String::new();
        out.push_str("[project]\n");
        out.push_str(&format!("name = \"{}\"\n", escape_toml_str(&self.name)));
        out.push_str(&format!("version = \"{}\"\n", escape_toml_str(&self.version)));
        out.push_str(&format!("entry = \"{}\"\n", escape_toml_str(&self.entry)));
        if !self.description.is_empty() {
            out.push_str(&format!(
                "description = \"{}\"\n",
                escape_toml_str(&self.description)
            ));
        }
        if !self.authors.is_empty() {
            let quoted: Vec<String> = self
                .authors
                .iter()
                .map(|a| format!("\"{}\"", escape_toml_str(a)))
                .collect();
            out.push_str(&format!("authors = [{}]\n", quoted.join(", ")));
        }
        if let Some(url) = &self.registry_url {
            out.push('\n');
            out.push_str("[registry]\n");
            out.push_str(&format!("url = \"{}\"\n", escape_toml_str(url)));
        }
        out.push('\n');
        out.push_str("[dependencies]\n");
        for (k, v) in &self.deps {
            out.push_str(&format!("{} = \"{}\"\n", k, escape_toml_str(v)));
        }
        if !self.dev_deps.is_empty() {
            out.push('\n');
            out.push_str("[dev-dependencies]\n");
            for (k, v) in &self.dev_deps {
                out.push_str(&format!("{} = \"{}\"\n", k, escape_toml_str(v)));
            }
        }
        if self.build != BuildConfig::default() {
            out.push('\n');
            out.push_str("[build]\n");
            out.push_str(&format!("target = \"{}\"\n", escape_toml_str(&self.build.target)));
            out.push_str(&format!("optimization-level = {}\n", self.build.optimization_level));
        }
        out
    }
}

/// Escape a TOML basic string (quotes + backslashes).
fn escape_toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Parse `["a", "b"]` (also accepts single-quoted items and bare
/// words). Anything else yields an empty vec (never an error —
/// manifest parsing stays total).
fn parse_string_array(raw: &str) -> Vec<String> {
    let t = raw.trim();
    let inner = match t.strip_prefix('[') {
        Some(rest) => match rest.rfind(']') {
            Some(end) => &rest[..end],
            None => return Vec::new(),
        },
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in inner.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                cur.push(c);
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    cur.push(c);
                } else if c == ',' {
                    let item = unescape_basic(&cur);
                    if !item.trim().is_empty() {
                        out.push(item.trim().to_string());
                    }
                    cur.clear();
                } else {
                    cur.push(c);
                }
            }
        }
    }
    let item = unescape_basic(&cur);
    if !item.trim().is_empty() {
        out.push(item.trim().to_string());
    }
    out
}

/// Undo the escapes [`escape_toml_str`] produces (best-effort).
fn unescape_basic(s: &str) -> String {
    let t = s.trim();
    let inner = if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"'))
            || (t.starts_with('\'') && t.ends_with('\'')))
    {
        &t[1..t.len() - 1]
    } else {
        t
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
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
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
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
