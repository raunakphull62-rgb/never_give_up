//! Klang package manager (Phase 1).
//!
//! PRD module layout (`KLANG_PHASE1_IMPLEMENTATION_PRD.txt` §1):
//! `semver` (version constraints), `manifest` (klang.toml I/O),
//! `lock_file` (klang.lock), `resolver` (dependency resolution),
//! `registry_client` (registry HTTP), `package_installer`
//! (download/verify/extract), `cli` (subcommand handlers).
//!
//! The selection/resolution/fetch engine lives in [`crate::package`]
//! and [`crate::registry`]; this module provides the PRD-named surface
//! over it. Storage stays `.klang_pkgs/<name>/<version>/` and the lock
//! stays the line format (`package <name> <version> <sha256>`).

pub mod cli;
pub mod lock_file;
pub mod manifest;
pub mod package_installer;
pub mod registry_client;
pub mod resolver;
pub mod semver;
