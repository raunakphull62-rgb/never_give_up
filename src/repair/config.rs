//! Repair configuration: precedence flag -> klang.toml [repair] -> env.
//! No network call is made unless an endpoint is explicitly configured.

use std::collections::HashMap;

/// Repair regeneration scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Function,
    File,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "function" => Some(Self::Function),
            "file" => Some(Self::File),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::File => "file",
        }
    }
}

/// Partial `[repair]` section from `klang.toml` (all optional).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairFileConfig {
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_iters: Option<u32>,
    pub scope: Option<Scope>,
}

impl RepairFileConfig {
    pub fn is_empty(&self) -> bool {
        self.endpoint.is_none()
            && self.model.is_none()
            && self.api_key.is_none()
            && self.timeout_secs.is_none()
            && self.max_iters.is_none()
            && self.scope.is_none()
    }
    /// Parse the `[repair]` section of raw `klang.toml` text. Same minimal
    /// `key = "value"` dialect as `package::Manifest::parse` (no TOML dep).
    pub fn parse_toml(text: &str) -> Self {
        let mut cfg = Self::default();
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
            if section != "repair" {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let val = toml_value(v.trim());
                if val.is_empty() {
                    continue;
                }
                match k.trim() {
                    "endpoint" => cfg.endpoint = Some(val),
                    "model" => cfg.model = Some(val),
                    "api_key" | "apikey" | "key" => cfg.api_key = Some(val),
                    "timeout" | "timeout_secs" => {
                        if let Ok(n) = val.parse::<u64>() {
                            cfg.timeout_secs = Some(n);
                        }
                    }
                    "max_iters" | "max-iters" => {
                        if let Ok(n) = val.parse::<u32>() {
                            cfg.max_iters = Some(n);
                        }
                    }
                    "scope" => {
                        if let Some(s) = Scope::parse(&val) {
                            cfg.scope = Some(s);
                        }
                    }
                    _ => {}
                }
            }
        }
        cfg
    }
}

/// Explicit CLI flag overrides.
#[derive(Debug, Clone, Default)]
pub struct RepairCliOverrides {
    pub max_iters: Option<u32>,
    pub scope: Option<Scope>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub timeout_secs: Option<u64>,
}

/// Fully resolved repair configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairConfig {
    pub max_iters: u32,
    pub scope: Scope,
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub timeout_secs: u64,
}

impl RepairConfig {
    pub const DEFAULT_ITERS: u32 = 5;
    pub const HARD_MAX_ITERS: u32 = 10;
    pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
    pub const DEFAULT_MODEL: &'static str = "deepseek-chat";
    /// Resolve flag -> klang.toml [repair] -> env. Missing endpoint errors.
    pub fn resolve(
        flags: &RepairCliOverrides,
        file: &RepairFileConfig,
        env: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let endpoint = flags
            .endpoint
            .clone()
            .or_else(|| file.endpoint.clone())
            .or_else(|| env.get("KLANG_MODEL_ENDPOINT").cloned())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "repair: no model endpoint configured (use --model <url>, klang.toml [repair] endpoint, or $KLANG_MODEL_ENDPOINT)".to_string())?;
        let model = flags
            .model
            .clone()
            .or_else(|| file.model.clone())
            .or_else(|| env.get("KLANG_MODEL_NAME").cloned())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| Self::DEFAULT_MODEL.to_string());
        let api_key = flags
            .api_key
            .clone()
            .or_else(|| file.api_key.clone())
            .or_else(|| {
                env.get("KLANG_MODEL_KEY")
                    .or_else(|| env.get("KLANG_MODEL_API_KEY"))
                    .cloned()
            })
            .unwrap_or_default();
        let max_iters = flags
            .max_iters
            .or(file.max_iters)
            .unwrap_or(Self::DEFAULT_ITERS)
            .clamp(1, Self::HARD_MAX_ITERS);
        let timeout_secs = flags
            .timeout_secs
            .or(file.timeout_secs)
            .or_else(|| env.get("KLANG_MODEL_TIMEOUT").and_then(|s| s.parse::<u64>().ok()))
            .unwrap_or(Self::DEFAULT_TIMEOUT_SECS)
            .clamp(1, 600);
        let scope = flags.scope.or(file.scope).unwrap_or(Scope::Function);
        Ok(Self { max_iters, scope, endpoint, model, api_key, timeout_secs })
    }
    /// Resolve with an explicit env (tests).
    pub fn resolve_with(
        flags: &RepairCliOverrides,
        file: &RepairFileConfig,
        pairs: &[(&str, &str)],
    ) -> Result<Self, String> {
        let env: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Self::resolve(flags, file, &env)
    }
    /// Read the live process environment.
    pub fn live_env() -> HashMap<String, String> {
        let mut m = HashMap::new();
        for k in [
            "KLANG_MODEL_ENDPOINT",
            "KLANG_MODEL_NAME",
            "KLANG_MODEL_KEY",
            "KLANG_MODEL_API_KEY",
            "KLANG_MODEL_TIMEOUT",
        ] {
            if let Ok(v) = std::env::var(k) {
                m.insert(k.to_string(), v);
            }
        }
        m
    }
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

fn toml_value(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"'))
            || (v.starts_with('\'') && v.ends_with('\'')))
    {
        return v[1..v.len() - 1].to_string();
    }
    v.trim_matches('"').trim_matches('\'').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_flag_beats_file_beats_env() {
        let file = RepairFileConfig {
            endpoint: Some("http://file/v1".into()),
            model: Some("file-model".into()),
            api_key: None,
            timeout_secs: None,
            max_iters: Some(3),
            scope: Some(Scope::File),
        };
        let flags = RepairCliOverrides {
            endpoint: Some("http://flag/v1".into()),
            ..Default::default()
        };
        let cfg = RepairConfig::resolve_with(&flags, &file, &[("KLANG_MODEL_ENDPOINT", "http://env/v1")]).expect("resolves");
        assert_eq!(cfg.endpoint, "http://flag/v1");
        assert_eq!(cfg.model, "file-model");
        assert_eq!(cfg.max_iters, 3);
        assert_eq!(cfg.scope, Scope::File);
    }
    #[test]
    fn missing_endpoint_is_error() {
        let e = RepairConfig::resolve_with(&RepairCliOverrides::default(), &RepairFileConfig::default(), &[]).expect_err("must error");
        assert!(e.contains("no model endpoint"), "{e}");
    }
    #[test]
    fn iters_clamped_to_hard_max() {
        let flags = RepairCliOverrides {
            max_iters: Some(99),
            endpoint: Some("http://x/v1".into()),
            ..Default::default()
        };
        let cfg = RepairConfig::resolve_with(&flags, &RepairFileConfig::default(), &[]).expect("resolves");
        assert_eq!(cfg.max_iters, RepairConfig::HARD_MAX_ITERS);
    }
    #[test]
    fn toml_section_parses() {
        let t = "name = \"demo\"\n[repair]\nendpoint = \"http://kaggle/v1\"\nmodel = \"deepseek\"\nmax_iters = 4\nscope = \"file\"\n";
        let c = RepairFileConfig::parse_toml(t);
        assert_eq!(c.endpoint.as_deref(), Some("http://kaggle/v1"));
        assert_eq!(c.max_iters, Some(4));
        assert_eq!(c.scope, Some(Scope::File));
    }
}
