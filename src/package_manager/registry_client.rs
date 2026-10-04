//! Registry HTTP client (PRD §1: `registry_client.rs`).
//!
//! Thin PRD-named struct over the [`crate::registry`] client
//! (`fetch_metadata` / `download` / SHA-256 verification). Base URL
//! comes from `--registry`, then `KLANG_REGISTRY`, then
//! [`crate::registry::DEFAULT_REGISTRY`].

use crate::registry::{self, PackageMeta, RegistryError};

/// Client for one registry instance.
#[derive(Debug, Clone)]
pub struct RegistryClient {
    base_url: String,
}

impl RegistryClient {
    /// Use an explicit base URL (e.g. from `--registry`).
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
        }
    }

    /// Resolve the base URL from `--registry`-value → `KLANG_REGISTRY`
    /// env → localhost default (same precedence as the CLI).
    pub fn from_env(registry_flag: Option<&str>) -> Self {
        let base = registry_flag
            .map(|s| s.to_string())
            .or_else(|| std::env::var("KLANG_REGISTRY").ok())
            .unwrap_or_else(|| registry::DEFAULT_REGISTRY.to_string());
        Self::new(&base)
    }

    /// Base URL in use.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// PRD `query_package(name)`: versions, checksums, transitive deps.
    pub fn query_package(&self, name: &str) -> Result<PackageMeta, RegistryError> {
        registry::fetch_metadata(&self.base_url, name)
    }

    /// PRD `download_package(name, version)`: raw archive bytes.
    pub fn download_package(&self, name: &str, version: &str) -> Result<Vec<u8>, RegistryError> {
        registry::download(&self.base_url, name, version)
    }

    /// PRD `verify_checksum(data, expected_sha256)`: recompute SHA-256
    /// and fail loudly on mismatch (never warn-and-continue).
    pub fn verify_checksum(&self, data: &[u8], expected_sha256: &str) -> Result<(), RegistryError> {
        let actual = registry::sha256_hex(data);
        if actual != expected_sha256 {
            return Err(RegistryError::new(
                "integrity",
                format!("checksum mismatch (wanted {expected_sha256}, got {actual})"),
            ));
        }
        Ok(())
    }
}

impl crate::package::resolver::MetaSource for RegistryClient {
    fn versions(&self, name: &str) -> Option<Vec<(String, String)>> {
        self.query_package(name).ok().map(|m| m.versions)
    }

    fn deps_for(&self, name: &str, version: &str) -> Vec<(String, String)> {
        self.query_package(name)
            .map(|m| m.deps_for(version))
            .unwrap_or_default()
    }
}
