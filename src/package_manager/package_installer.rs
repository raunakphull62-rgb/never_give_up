//! Package installation (PRD §1: `package_installer.rs`).
//!
//! PRD-named struct over [`crate::registry`]'s fetch/verify/extract
//! pipeline. Packages land in `.klang_pkgs/<name>/<version>/`;
//! every download is SHA-256 verified against the registry-advertised
//! hash and pinned in `klang.lock` (the lock is the trust anchor).

use std::path::{Path, PathBuf};

use crate::registry::{self, RegistryError};

/// Installer bound to one project root.
#[derive(Debug, Clone)]
pub struct PackageInstaller {
    root: PathBuf,
}

impl PackageInstaller {
    /// Bind to a project root (dir holding `klang.toml`).
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// PRD `install(name, version)`: vendor + verify one exact pin.
    /// Returns the registry log line (e.g. `calc@1.0.0: fetched (…)`).
    pub fn install(
        &self,
        base: &str,
        name: &str,
        version: &str,
        offline: bool,
    ) -> Result<String, RegistryError> {
        let lock_text =
            std::fs::read_to_string(self.root.join("klang.lock")).unwrap_or_default();
        let locks = registry::parse_package_locks(&lock_text);
        registry::ensure_fetched(&self.root, base, name, version, &locks, offline)
    }

    /// PRD `extract_to_cache(data, name, version)`: unpack validated
    /// archive bytes into `.klang_pkgs/<name>/<version>/`, wiping any
    /// stale directory first so version switches never mix files.
    pub fn extract_to_cache(
        &self,
        data: &[u8],
        name: &str,
        version: &str,
    ) -> Result<(), RegistryError> {
        if !registry::valid_pkg_name(name) || !registry::valid_version(version) {
            return Err(RegistryError::new(
                "protocol",
                "bad name or version".to_string(),
            ));
        }
        let files = registry::unpack_archive(data)
            .map_err(|e| RegistryError::new("protocol", format!("bad archive: {e}")))?;
        let dir = registry::pkg_dir(&self.root, name, version);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| {
                RegistryError::new("io", format!("cannot clear vendor dir: {e}"))
            })?;
        }
        for (rel, content) in &files {
            let dest = dir.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    RegistryError::new("io", format!("cannot create dir: {e}"))
                })?;
            }
            std::fs::write(&dest, content).map_err(|e| {
                RegistryError::new("io", format!("cannot write `{rel}`: {e}"))
            })?;
        }
        Ok(())
    }

    /// PRD `link_or_copy(source, dest)`: hard-link when the filesystem
    /// allows it, plain copy otherwise. Missing parents are created.
    pub fn link_or_copy(&self, source: &Path, dest: &Path) -> Result<(), RegistryError> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RegistryError::new("io", format!("cannot create dir: {e}"))
            })?;
        }
        if dest.exists() {
            std::fs::remove_file(dest)
                .map_err(|e| RegistryError::new("io", format!("cannot clear dest: {e}")))?;
        }
        match std::fs::hard_link(source, dest) {
            Ok(()) => Ok(()),
            Err(_) => std::fs::copy(source, dest)
                .map(|_| ())
                .map_err(|e| RegistryError::new("io", format!("cannot copy: {e}"))),
        }
    }

    /// PRD `cleanup_on_error()`: drop a half-written vendor directory
    /// so a failed install never leaves corrupt state behind.
    pub fn cleanup_on_error(&self, name: &str, version: &str) {
        let dir = registry::pkg_dir(&self.root, name, version);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
