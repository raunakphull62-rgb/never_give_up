//! Semantic version constraints (Phase 1: Core Package Manager).
//!
//! Exact pins (`1.2.3`) keep working as before; manifests and the CLI
//! may also declare ranges while the lockfile still pins exact
//! versions. Supported (PRD §4):
//!
//! - exact: `1.2.3`, `=1.2.3`
//! - caret: `^1.2.3` (`0.x`-aware: `^0.5.0` → `<0.6.0`, `^0.0.3` → `<0.0.4`)
//! - tilde: `~1.2.3` (→ `<1.3.0`), partials (`~1.2` → `>=1.2.0 <1.3.0`)
//! - comparisons: `>`, `>=`, `<`, `<=`
//! - wildcard: `*` (or `latest`) — any version
//! - AND groups: `>=1.0.0 <2.0.0` (whitespace-separated, all must hold)
//!
//! Versions are numeric triples with 1-3 parts (`^1.0` means
//! `^1.0.0`). A `Constraint` is a conjunction of [`Predicate`]s.

use std::cmp::Ordering;

/// Parsed numeric version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn cmp_versions(a: &Version, b: &Version) -> Ordering {
        (a.major, a.minor, a.patch).cmp(&(b.major, b.minor, b.patch))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Parse a strict `X.Y.Z` version.
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut nums = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        nums[i] = p.parse().ok()?;
    }
    Some(Version {
        major: nums[0],
        minor: nums[1],
        patch: nums[2],
    })
}

/// Parse a loose version with 1-3 numeric parts (`1` → `1.0.0`).
fn parse_version_loose(s: &str) -> Option<Version> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() > 3 || parts.is_empty() {
        return None;
    }
    for p in &parts {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    let nums: Vec<u64> = parts.iter().filter_map(|p| p.parse().ok()).collect();
    if nums.len() != parts.len() {
        return None;
    }
    Some(Version {
        major: *nums.first().unwrap_or(&0),
        minor: *nums.get(1).unwrap_or(&0),
        patch: *nums.get(2).unwrap_or(&0),
    })
}

/// Compare two version strings numerically (`X.Y.Z`).
/// Unknown shapes fall back to string order (never panics).
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    match (parse_version_loose(a), parse_version_loose(b)) {
        (Some(va), Some(vb)) => Version::cmp_versions(&va, &vb),
        _ => a.cmp(b),
    }
}

/// One version predicate. A [`Constraint`] holds when every one of its
/// predicates holds (AND logic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    /// `1.2.3` / `=1.2.3` — exactly this version.
    Exact(Version),
    /// `^1.2.3` — compatible: `>=base, <next-breaking`.
    Caret(Version),
    /// `~1.2.3` — reasonably close: `>=base, <X.(Y+1).0`.
    Tilde(Version),
    /// `>1.2.3` — strictly greater.
    Gt(Version),
    /// `>=1.2.0` — greater than or equal.
    Gte(Version),
    /// `<2.0.0` — strictly less.
    Lt(Version),
    /// `<=2.0.0` — less than or equal.
    Lte(Version),
    /// `*` / `latest` — any version.
    Any,
}

/// A version constraint: a conjunction of [`Predicate`]s.
/// Single-operator forms (`^1.0.0`, `>=2.0`, …) are one-element
/// conjunctions; `">=1.0.0 <2.0.0"` has two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    /// All predicates; every one must hold.
    pub predicates: Vec<Predicate>,
}

impl Constraint {
    /// Single-predicate constraint.
    pub fn single(p: Predicate) -> Self {
        Self {
            predicates: vec![p],
        }
    }

    /// Raw string back (predicates joined by space).
    pub fn as_str(&self) -> String {
        if self.predicates.is_empty() {
            return "latest".to_string();
        }
        self.predicates
            .iter()
            .map(|p| match p {
                Predicate::Exact(v) => v.to_string(),
                Predicate::Caret(v) => format!("^{v}"),
                Predicate::Tilde(v) => format!("~{v}"),
                Predicate::Gt(v) => format!(">{v}"),
                Predicate::Gte(v) => format!(">={v}"),
                Predicate::Lt(v) => format!("<{v}"),
                Predicate::Lte(v) => format!("<={v}"),
                Predicate::Any => "latest".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Parse one operator predicate (no whitespace inside).
fn parse_single(s: &str, whole: &str) -> Result<Predicate, String> {
    let err = |want: &str| format!("bad version constraint `{whole}` (want {want})");
    if s == "*" {
        return Ok(Predicate::Any);
    }
    if s.eq_ignore_ascii_case("latest") {
        return Ok(Predicate::Any);
    }
    if let Some(rest) = s.strip_prefix('^') {
        let v = parse_version_loose(rest).ok_or_else(|| err("^X.Y.Z"))?;
        return Ok(Predicate::Caret(v));
    }
    if let Some(rest) = s.strip_prefix('~') {
        let v = parse_version_loose(rest).ok_or_else(|| err("~X.Y.Z"))?;
        return Ok(Predicate::Tilde(v));
    }
    if let Some(rest) = s.strip_prefix(">=") {
        let v = parse_version_loose(rest.trim_start()).ok_or_else(|| err(">=X.Y.Z"))?;
        return Ok(Predicate::Gte(v));
    }
    if let Some(rest) = s.strip_prefix("<=") {
        let v = parse_version_loose(rest.trim_start()).ok_or_else(|| err("<=X.Y.Z"))?;
        return Ok(Predicate::Lte(v));
    }
    if let Some(rest) = s.strip_prefix('>') {
        let v = parse_version_loose(rest.trim_start()).ok_or_else(|| err(">X.Y.Z"))?;
        return Ok(Predicate::Gt(v));
    }
    if let Some(rest) = s.strip_prefix('<') {
        let v = parse_version_loose(rest.trim_start()).ok_or_else(|| err("<X.Y.Z"))?;
        return Ok(Predicate::Lt(v));
    }
    if let Some(rest) = s.strip_prefix('=') {
        // Accept both `=1.2.3` and `==1.2.3`.
        let rest = rest.strip_prefix('=').unwrap_or(rest);
        let v = parse_version_loose(rest.trim_start()).ok_or_else(|| err("=X.Y.Z"))?;
        return Ok(Predicate::Exact(v));
    }
    let v = parse_version_loose(s).ok_or_else(|| {
        format!(
            "bad version constraint `{whole}` (want X.Y.Z, ^X.Y.Z, ~X.Y.Z, >/>=/</<=X.Y.Z, =X.Y.Z, * or latest)"
        )
    })?;
    Ok(Predicate::Exact(v))
}

/// Parse a constraint string: `latest`, `*`, one operator form, or
/// whitespace-separated AND groups (`>=1.0.0 <2.0.0`). Empty input
/// means `latest` (PRD wildcard).
pub fn parse_constraint(s: &str) -> Result<Constraint, String> {
    let s = s.trim();
    if s.is_empty() || s == "*" || s.eq_ignore_ascii_case("latest") {
        return Ok(Constraint::single(Predicate::Any));
    }
    let mut predicates = Vec::new();
    for part in s.split_whitespace() {
        predicates.push(parse_single(part, s)?);
    }
    Ok(Constraint { predicates })
}

/// True when `version` satisfies every predicate of `constraint`.
pub fn matches(version: &str, constraint: &Constraint) -> bool {
    let Some(v) = parse_version_loose(version) else {
        return false;
    };
    constraint.predicates.iter().all(|p| match p {
        Predicate::Any => true,
        Predicate::Exact(base) => Version::cmp_versions(&v, base) == Ordering::Equal,
        Predicate::Gt(base) => Version::cmp_versions(&v, base) == Ordering::Greater,
        Predicate::Gte(base) => Version::cmp_versions(&v, base) != Ordering::Less,
        Predicate::Lt(base) => Version::cmp_versions(&v, base) == Ordering::Less,
        Predicate::Lte(base) => Version::cmp_versions(&v, base) != Ordering::Greater,
        Predicate::Caret(base) => {
            if Version::cmp_versions(&v, base) == Ordering::Less {
                return false;
            }
            let upper = if base.major > 0 {
                Version {
                    major: base.major + 1,
                    minor: 0,
                    patch: 0,
                }
            } else if base.minor > 0 {
                Version {
                    major: 0,
                    minor: base.minor + 1,
                    patch: 0,
                }
            } else {
                Version {
                    major: 0,
                    minor: 0,
                    patch: base.patch + 1,
                }
            };
            Version::cmp_versions(&v, &upper) == Ordering::Less
        }
        Predicate::Tilde(base) => {
            if Version::cmp_versions(&v, base) == Ordering::Less {
                return false;
            }
            let upper = Version {
                major: base.major,
                minor: base.minor + 1,
                patch: 0,
            };
            Version::cmp_versions(&v, &upper) == Ordering::Less
        }
    })
}

/// Highest version in `versions` satisfying `constraint`.
/// Returns the version string as stored (not normalized).
pub fn max_satisfying<'a, I>(versions: I, constraint: &Constraint) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(Version, &'a str)> = None;
    for raw in versions {
        if !matches(raw, constraint) {
            continue;
        }
        if let Some(v) = parse_version_loose(raw) {
            let better = match &best {
                None => true,
                Some((bv, _)) => Version::cmp_versions(&v, bv) == Ordering::Greater,
            };
            if better {
                best = Some((v, raw));
            }
        }
    }
    best.map(|(_, raw)| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        let c = parse_constraint("1.2.3").unwrap();
        assert!(matches("1.2.3", &c));
        assert!(!matches("1.2.4", &c));
    }

    #[test]
    fn caret_major() {
        let c = parse_constraint("^1.2.3").unwrap();
        assert!(matches("1.2.3", &c));
        assert!(matches("1.9.0", &c));
        assert!(!matches("2.0.0", &c));
        assert!(!matches("1.2.2", &c));
    }

    #[test]
    fn caret_zero_minor() {
        let c = parse_constraint("^0.5.0").unwrap();
        assert!(matches("0.5.2", &c));
        assert!(!matches("0.6.0", &c));
    }

    #[test]
    fn tilde_match() {
        let c = parse_constraint("~1.2.3").unwrap();
        assert!(matches("1.2.5", &c));
        assert!(!matches("1.3.0", &c));
    }

    #[test]
    fn gte_and_latest() {
        let c = parse_constraint(">=1.2.0").unwrap();
        assert!(matches("2.0.0", &c));
        assert!(!matches("1.1.9", &c));
        let l = parse_constraint("latest").unwrap();
        assert!(matches("99.0.0", &l));
    }

    #[test]
    fn max_pick() {
        let vs = vec!["1.0.0", "1.2.0", "2.0.0"];
        let c = parse_constraint("^1.0.0").unwrap();
        assert_eq!(max_satisfying(vs.iter().copied(), &c).as_deref(), Some("1.2.0"));
    }

    #[test]
    fn comparison_operators() {
        let gt = parse_constraint(">1.0.0").unwrap();
        assert!(matches("1.0.1", &gt));
        assert!(!matches("1.0.0", &gt));
        let lt = parse_constraint("<2.0.0").unwrap();
        assert!(matches("1.9.9", &lt));
        assert!(!matches("2.0.0", &lt));
        let lte = parse_constraint("<=2.0.0").unwrap();
        assert!(matches("2.0.0", &lte));
        assert!(!matches("2.0.1", &lte));
        let eq = parse_constraint("=1.2.3").unwrap();
        assert!(matches("1.2.3", &eq));
        assert!(!matches("1.2.4", &eq));
    }

    #[test]
    fn wildcard_and_empty() {
        for raw in ["*", "latest", ""] {
            let c = parse_constraint(raw).unwrap();
            assert!(matches("0.0.1", &c), "{raw}");
            assert!(matches("99.99.99", &c), "{raw}");
        }
    }

    #[test]
    fn and_groups() {
        let c = parse_constraint(">=1.0.0 <2.0.0").unwrap();
        assert!(matches("1.5.0", &c));
        assert!(!matches("0.9.9", &c));
        assert!(!matches("2.0.0", &c));
        // Mixed operators.
        let d = parse_constraint("^1.2.0 <=1.4.0").unwrap();
        assert!(matches("1.3.0", &d));
        assert!(!matches("1.5.0", &d));
    }

    #[test]
    fn caret_zero_patch() {
        let c = parse_constraint("^0.0.3").unwrap();
        assert!(matches("0.0.3", &c));
        assert!(!matches("0.0.4", &c));
    }

    #[test]
    fn tilde_partial() {
        let c = parse_constraint("~1.2").unwrap();
        assert!(matches("1.2.0", &c));
        assert!(matches("1.2.9", &c));
        assert!(!matches("1.3.0", &c));
    }

    #[test]
    fn bad_constraints_rejected() {
        for raw in ["^", "~", ">=", "abc", "1.2.3.4", "^x.y"] {
            assert!(parse_constraint(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn max_satisfying_and_group() {
        let vs = vec!["0.9.0", "1.0.0", "1.5.0", "2.0.0"];
        let c = parse_constraint(">=1.0.0 <2.0.0").unwrap();
        assert_eq!(max_satisfying(vs.iter().copied(), &c).as_deref(), Some("1.5.0"));
    }
}
