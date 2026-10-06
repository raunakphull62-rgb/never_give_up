//! Lock file system (PRD §1, §3).
//!
//! The Phase 1 lock format is the line format (per plan decision —
//! the PRD's `[[packages]]` TOML was set aside to avoid breaking the
//! existing `parse_package_locks` readers and tests):
//!
//! ```text
//! package <name> <version> <sha256-hex>
//! ```
//!
//! Legacy file-hash lines (`<file> <hex>`) are preserved byte-wise on
//! write; they belong to [`crate::package`]'s content-addressed locks.

use std::path::{Path, PathBuf};

use crate::registry::{parse_package_locks, RegistryError};

/// One pinned package (PRD `PackageEntry`: name, version, checksum, source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEntry {
    /// Package name.
    pub name: String,
    /// Pinned exact version (`X.Y.Z`).
    pub version: String,
    /// SHA-256 of the published archive (hex, 64 chars).
    pub checksum: String,
    /// Pin source: always `"registry"` in Phase 1.
    pub source: String,
}

impl PackageEntry {
    /// `"registry"`-sourced entry.
    pub fn new(name: &str, version: &str, checksum: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            checksum: checksum.to_string(),
            source: "registry".to_string(),
        }
    }
}

/// Parsed lock file: package pins plus the lock path they came from.
#[derive(Debug, Clone, Default)]
pub struct LockFile {
    /// Pinned packages (sorted by name on write).
    pub packages: Vec<PackageEntry>,
}

impl LockFile {
    /// Default `klang.lock` location for a project root.
    pub fn path_for(root: &Path) -> PathBuf {
        root.join("klang.lock")
    }

    /// Read + parse `path` (`Ok(empty)` when the file is absent).
    /// B2: any `package` line with an empty or non-64-hex checksum is
    /// an error (never silently skipped).
    pub fn read(path: &Path) -> Result<Self, RegistryError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(RegistryError::new(
                    "io",
                    format!("cannot read {}: {e}", path.display()),
                ))
            }
        };
        Self::try_from_text(&text)
    }

    /// Parse lock text (missing/garbage lines are skipped, as readers do).
    /// Lenient legacy entry point (kept for backcompat); new code should
    /// prefer [`Self::try_from_text`] which rejects bad checksums.
    pub fn from_text(text: &str) -> Self {
        Self {
            packages: parse_package_locks(text)
                .into_iter()
                .map(|l| PackageEntry::new(&l.name, &l.version, &l.sha256))
                .collect(),
        }
    }

    /// Strict parse: any `package` line with a missing/empty/non-64-hex
    /// checksum (or bad name/version) is an error. Non-`package` lines
    /// (legacy file-hash lines, blanks, comments) pass through ignored.
    pub fn try_from_text(text: &str) -> Result<Self, RegistryError> {
        let mut packages = Vec::new();
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
            if !crate::registry::valid_pkg_name(name) || !crate::registry::valid_version(version) {
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
            packages.push(PackageEntry::new(name, version, sha));
        }
        Ok(Self { packages })
    }

    /// Look up the pin for `name` at exactly `version`.
    pub fn find(&self, name: &str, version: &str) -> Option<&PackageEntry> {
        self.packages
            .iter()
            .find(|p| p.name == name && p.version == version)
    }

    /// Generate a lock from a [`crate::package::resolver::ResolutionGraph`]
    /// plus the content hashes (`name@version → sha256`). Hashes come
    /// from the resolver's `Resolved` pins or registry metadata.
    /// B2: a missing, empty, or non-64-hex checksum is an error — a lock
    /// entry must never pin an empty hash.
    pub fn from_resolution(
        graph: &crate::package::resolver::ResolutionGraph,
        hashes: &std::collections::HashMap<(String, String), String>,
    ) -> Result<Self, RegistryError> {
        let mut packages: Vec<PackageEntry> = Vec::new();
        for n in &graph.packages {
            let checksum = hashes
                .get(&(n.name.clone(), n.version.clone()))
                .ok_or_else(|| {
                    RegistryError::new(
                        "integrity",
                        format!("missing checksum for `{}@{}` (refusing to pin empty hash)", n.name, n.version),
                    )
                })?;
            if !is_valid_checksum(checksum) {
                return Err(RegistryError::new(
                    "integrity",
                    format!(
                        "bad checksum for `{}@{}` (want 64 hex chars)",
                        n.name, n.version
                    ),
                ));
            }
            packages.push(PackageEntry::new(&n.name, &n.version, checksum));
        }
        packages.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
        Ok(Self { packages })
    }

    /// Serialize pins, preserving non-`package` lines already in `path`.
    /// B4: atomic write via `klang.lock.tmp` + fsync + rename.
    pub fn write(&self, path: &Path) -> Result<(), RegistryError> {
        for p in &self.packages {
            if !is_valid_checksum(&p.checksum) {
                return Err(RegistryError::new(
                    "integrity",
                    format!(
                        "refusing to write lock with bad checksum for `{}@{}`",
                        p.name, p.version
                    ),
                ));
            }
        }
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        let mut out: Vec<String> = existing
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
        let mut sorted = self.packages.clone();
        sorted.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
        for p in &sorted {
            out.push(format!("package {} {} {}", p.name, p.version, p.checksum));
        }
        let mut text = out.join("\n");
        text.push('\n');
        crate::registry::atomic_write(path, text.as_bytes())
            .map_err(|e| RegistryError::new("io", format!("cannot write lock: {e}")))?;
        Ok(())
    }
}

/// True when `s` is a 64-char hex SHA-256 (B2 gate).
pub fn is_valid_checksum(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}
