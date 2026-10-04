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
        Ok(Self::from_text(&text))
    }

    /// Parse lock text (missing/garbage lines are skipped, as readers do).
    pub fn from_text(text: &str) -> Self {
        Self {
            packages: parse_package_locks(text)
                .into_iter()
                .map(|l| PackageEntry::new(&l.name, &l.version, &l.sha256))
                .collect(),
        }
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
    pub fn from_resolution(
        graph: &crate::package::resolver::ResolutionGraph,
        hashes: &std::collections::HashMap<(String, String), String>,
    ) -> Self {
        let mut packages: Vec<PackageEntry> = graph
            .packages
            .iter()
            .map(|n| {
                let checksum = hashes
                    .get(&(n.name.clone(), n.version.clone()))
                    .cloned()
                    .unwrap_or_default();
                PackageEntry::new(&n.name, &n.version, &checksum)
            })
            .collect();
        packages.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
        Self { packages }
    }

    /// Serialize pins, preserving non-`package` lines already in `path`.
    pub fn write(&self, path: &Path) -> Result<(), RegistryError> {
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
        std::fs::write(path, text)
            .map_err(|e| RegistryError::new("io", format!("cannot write lock: {e}")))?;
        Ok(())
    }
}
