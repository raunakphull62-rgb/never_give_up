//! Project manifest I/O (PRD §1, §3).
//!
//! Re-exports [`Manifest`] / [`BuildConfig`] and adds the PRD
//! path-based entry points `parse_manifest` / `write_manifest`.
//! Manifest shape: `[project]` (`name`, `version`, `entry`,
//! `description`, `authors`), `[dependencies]`, `[dev-dependencies]`,
//! `[build]` (parsed; inert in Phase 1).

use std::path::Path;

pub use crate::package::{BuildConfig, Manifest};

/// Read + parse the manifest at `path` (usually `klang.toml`).
pub fn parse_manifest(path: &Path) -> Result<Manifest, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Manifest::parse(&text)
}

/// Serialize `manifest` and write it to `path` (deterministic full
/// rewrite — for fresh files; `add`/`remove` splice lines instead to
/// preserve user formatting).
pub fn write_manifest(manifest: &Manifest, path: &Path) -> Result<(), String> {
    std::fs::write(path, manifest.write_manifest())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}
