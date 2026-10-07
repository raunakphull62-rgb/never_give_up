//! CLI command handlers (PRD §1: `cli.rs`, §6).
//!
//! PRD-named `cmd_*` functions used by `klang`'s dispatcher
//! (`run_registry_mode` in `src/main.rs`). Each returns log/output
//! lines on success; the dispatcher renders them and maps failures to
//! `FAIL` + non-zero exit. `install` never edits `klang.toml`
//! (PRD); `add` pins and edits it.

use std::path::Path;

use crate::registry::{self, RegistryError};

/// `klang install [name[@constraint]] [--offline]`: vendor without
/// editing `klang.toml`. No spec reproduces the locked environment.
pub fn cmd_install(
    root: &Path,
    base: &str,
    spec: Option<&str>,
    offline: bool,
) -> Result<Vec<String>, RegistryError> {
    match spec {
        None => registry::fetch_project(root, base, offline),
        Some(s) => {
            let (name, constraint) = split_spec(s);
            if name.starts_with('.') || name.starts_with('/') || name.contains('/') {
                return Err(RegistryError::new(
                    "protocol",
                    "local-path install is not supported in Phase 1 (copy the files manually)"
                        .to_string(),
                ));
            }
            registry::install_spec(root, base, &name, &constraint, offline).map(|line| vec![line])
        }
    }
}

/// `klang add name[@constraint] [--dev] [--caret] [--offline]`: pin in
/// `klang.toml`, install immediately, update `klang.lock`.
/// B7: `--caret` writes `^X.Y.Z` instead of an exact pin.
pub fn cmd_add(
    root: &Path,
    base: &str,
    spec: &str,
    is_dev: bool,
    offline: bool,
) -> Result<String, RegistryError> {
    cmd_add_full(root, base, spec, is_dev, false, offline)
}

/// Full `add` with `--caret` support.
pub fn cmd_add_full(
    root: &Path,
    base: &str,
    spec: &str,
    is_dev: bool,
    caret: bool,
    offline: bool,
) -> Result<String, RegistryError> {
    let (name, constraint) = split_spec(spec);
    registry::add_dependency_req(root, base, &name, &constraint, is_dev, caret, offline)
}

/// Multi-name `add`: every spec is `name[@constraint]`; resolution covers
/// all specs together, the manifest is edited once, and the install is
/// all-or-none (a failure leaves manifest, lock, and vendor dirs
/// untouched). Single-name callers keep using [`cmd_add_full`], whose
/// output (`added <name>@<pin>`) is unchanged.
pub fn cmd_add_multi(
    root: &Path,
    base: &str,
    specs: &[String],
    is_dev: bool,
    caret: bool,
    offline: bool,
) -> Result<String, RegistryError> {
    let parsed: Vec<(String, String)> = specs.iter().map(|s| split_spec(s)).collect();
    registry::add_dependencies_req(root, base, &parsed, is_dev, caret, offline)
}

/// `klang remove <name>`: drop from `klang.toml`, delete vendor dirs,
/// prune orphaned lock pins.
pub fn cmd_remove(root: &Path, name: &str) -> Result<String, RegistryError> {
    registry::remove_dependency(root, name)
}

/// `klang update [name]`: re-resolve within constraints and re-fetch.
/// A `@version` suffix is rejected (edit `klang.toml` instead).
pub fn cmd_update(
    root: &Path,
    base: &str,
    name: Option<&str>,
    offline: bool,
) -> Result<Vec<String>, RegistryError> {
    if let Some(n) = name {
        if n.contains('@') {
            let bare = n.split_once('@').map(|(a, _)| a).unwrap_or(n);
            return Err(RegistryError::new(
                "protocol",
                format!(
                    "`update {n}` is ambiguous — edit klang.toml to change the constraint, then run `update {bare}`"
                ),
            ));
        }
    }
    registry::update_project(root, base, name, offline)
}

/// `klang list [--all]`: installed packages + versions.
/// B6: `--all` includes transitive deps indented under requirers.
pub fn cmd_list(root: &Path) -> Result<String, RegistryError> {
    registry::list_project(root)
}

/// `klang list --all` variant.
pub fn cmd_list_all(root: &Path, all: bool) -> Result<String, RegistryError> {
    registry::list_project_all(root, all)
}

/// `klang init [name]`: scaffold `klang.toml` + `src/` + `tests/`.
pub fn cmd_init(dir: &Path, project_name: &str) -> Result<String, String> {
    registry::init_project(dir, project_name)
}

/// Split `name[@constraint]`; a bare name means `latest`.
pub fn split_spec(spec: &str) -> (String, String) {
    match spec.split_once('@') {
        Some((n, c)) if !c.is_empty() => (n.to_string(), c.to_string()),
        _ => (spec.trim_end_matches('@').to_string(), "latest".to_string()),
    }
}
