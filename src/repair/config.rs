//! Repair configuration: attempt budget + scope, flag -> klang.toml.
//!
//! Phase 4 correction: Klang no longer dials a model, so there is no
//! endpoint / model / key / timeout surface here at all — no network
//! configuration to get wrong, no secrets to leak. Unknown `[repair]`
//! keys (including the retired `endpoint`, `model`, `api_key`,
//! `timeout`) are ignored, so old `klang.toml` files keep parsing.

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
    pub max_iters: Option<u32>,
    pub scope: Option<Scope>,
}

impl RepairFileConfig {
    pub fn is_empty(&self) -> bool {
        self.max_iters.is_none() && self.scope.is_none()
    }
    /// Parse the `[repair]` section of raw `klang.toml` text. Same minimal
    /// `key = "value"` dialect as `package::Manifest::parse` (no TOML dep).
    /// Retired model-connector keys are silently ignored.
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
}

/// Fully resolved repair configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairConfig {
    pub max_iters: u32,
    pub scope: Scope,
}

impl RepairConfig {
    pub const DEFAULT_ITERS: u32 = 5;
    pub const HARD_MAX_ITERS: u32 = 10;
    /// Resolve flag -> klang.toml [repair]. Infallible: both sources are
    /// optional, defaults cover the rest.
    pub fn resolve(flags: &RepairCliOverrides, file: &RepairFileConfig) -> Self {
        let max_iters = flags
            .max_iters
            .or(file.max_iters)
            .unwrap_or(Self::DEFAULT_ITERS)
            .clamp(1, Self::HARD_MAX_ITERS);
        let scope = flags.scope.or(file.scope).unwrap_or(Scope::Function);
        Self { max_iters, scope }
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
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
    {
        return v[1..v.len() - 1].to_string();
    }
    v.trim_matches('"').trim_matches('\'').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_flag_beats_file() {
        let file = RepairFileConfig {
            max_iters: Some(3),
            scope: Some(Scope::File),
        };
        let flags = RepairCliOverrides {
            max_iters: Some(7),
            ..Default::default()
        };
        let cfg = RepairConfig::resolve(&flags, &file);
        assert_eq!(cfg.max_iters, 7);
        assert_eq!(cfg.scope, Scope::File);
    }
    #[test]
    fn defaults_apply_when_nothing_configured() {
        let cfg =
            RepairConfig::resolve(&RepairCliOverrides::default(), &RepairFileConfig::default());
        assert_eq!(cfg.max_iters, RepairConfig::DEFAULT_ITERS);
        assert_eq!(cfg.scope, Scope::Function);
    }
    #[test]
    fn iters_clamped_to_hard_max() {
        let flags = RepairCliOverrides {
            max_iters: Some(99),
            ..Default::default()
        };
        let cfg = RepairConfig::resolve(&flags, &RepairFileConfig::default());
        assert_eq!(cfg.max_iters, RepairConfig::HARD_MAX_ITERS);
    }
    #[test]
    fn toml_section_parses_and_ignores_retired_keys() {
        let t = "name = \"demo\"\n[repair]\nmax_iters = 4\nscope = \"file\"\nendpoint = \"http://retired/v1\"\nmodel = \"old\"\napi_key = \"old\"\n";
        let c = RepairFileConfig::parse_toml(t);
        // Retired connector keys are ignored, not errors.
        assert_eq!(c.max_iters, Some(4));
        assert_eq!(c.scope, Some(Scope::File));
    }
}
