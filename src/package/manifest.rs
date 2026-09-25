//! Klang v2 package manifest (Phase 10).
//!
//! Parses `klang.toml` v2 keys offline — no registry, no network:
//!
//! ```toml
//! name = "demo"
//! version = "0.1.0"
//! entry = "main"
//! klang = "2"
//! [dependencies]
//! mylib = "./mylib.klang"
//! [schemas]
//! Profile = "1:deadbeef"
//! [repair]
//! max_iters = "5"
//! scope = "function"
//! ```
//!
//! v1 files (without `klang`/`schemas`/`repair`) parse with defaults, so
//! existing packages keep working.

/// One schema lock entry: `Name = "version:hash"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaLock {
    /// Schema name.
    pub name: String,
    /// Locked version.
    pub version: String,
    /// Locked content hash (hex).
    pub hash: String,
}

/// `[repair]` config.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepairConfig {
    /// Attempt budget, if set.
    pub max_iters: Option<u32>,
    /// `function` or `file`, if set.
    pub scope: Option<String>,
}

/// Parsed v2 manifest (superset of the v1 shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Manifest {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Entry function.
    pub entry: String,
    /// Language version (`1` default preserves v1 packages).
    pub klang: String,
    /// Local path dependencies.
    pub deps: Vec<(String, String)>,
    /// Locked schemas.
    pub schemas: Vec<SchemaLock>,
    /// Repair config.
    pub repair: RepairConfig,
}

impl V2Manifest {
    /// Parse manifest text. Unknown keys are ignored (forward-compat);
    /// malformed v2 values are errors.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut name: Option<String> = None;
        let mut version: Option<String> = None;
        let mut entry: Option<String> = None;
        let mut klang: Option<String> = None;
        let mut deps: Vec<(String, String)> = Vec::new();
        let mut schemas: Vec<SchemaLock> = Vec::new();
        let mut max_iters: Option<u32> = None;
        let mut scope: Option<String> = None;
        let mut section = String::new();
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let val = unquote(v.trim());
            match section.as_str() {
                "dependencies" => deps.push((k.trim().to_string(), val)),
                "schemas" => schemas.push(parse_schema_lock(k.trim(), &val)?),
                "repair" => match k.trim() {
                    "max_iters" => {
                        let n: u32 = val
                            .parse()
                            .map_err(|_| format!("bad [repair] max_iters `{val}` (want 1..=10)"))?;
                        if !(1..=10).contains(&n) {
                            return Err(format!("bad [repair] max_iters `{val}` (want 1..=10)"));
                        }
                        max_iters = Some(n);
                    }
                    "scope" => {
                        if val != "function" && val != "file" {
                            return Err(format!("bad [repair] scope `{val}` (want function|file)"));
                        }
                        scope = Some(val);
                    }
                    _ => {}
                },
                _ => match k.trim() {
                    "name" => name = Some(val),
                    "version" => version = Some(val),
                    "entry" => entry = Some(val),
                    "klang" => klang = Some(val),
                    _ => {}
                },
            }
        }
        Ok(Self {
            name: name.unwrap_or_else(|| "app".to_string()),
            version: version.unwrap_or_else(|| "0.1.0".to_string()),
            entry: entry.unwrap_or_else(|| "combine".to_string()),
            klang: klang.unwrap_or_else(|| "1".to_string()),
            deps,
            schemas,
            repair: RepairConfig { max_iters, scope },
        })
    }
}

fn parse_schema_lock(name: &str, val: &str) -> Result<SchemaLock, String> {
    let Some((version, hash)) = val.split_once(':') else {
        return Err(format!(
            "bad [schemas] entry `{name}` (want \"version:hash\")"
        ));
    };
    if version.is_empty() {
        return Err(format!("bad [schemas] entry `{name}` (empty version)"));
    }
    if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("bad [schemas] entry `{name}` (hash must be hex)"));
    }
    Ok(SchemaLock {
        name: name.to_string(),
        version: version.to_string(),
        hash: hash.to_string(),
    })
}

fn strip_comment(line: &str) -> &str {
    let mut single = false;
    let mut double = false;
    for (i, c) in line.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '#' if !single && !double => return line[..i].trim_end(),
            _ => {}
        }
    }
    line
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
    {
        v[1..v.len() - 1].to_string()
    } else {
        v.trim_matches('"').trim_matches('\'').trim().to_string()
    }
}
