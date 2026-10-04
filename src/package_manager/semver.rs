//! Semantic version constraints (PRD §4).
//!
//! Re-exports the engine from [`crate::package::version`] under the
//! PRD module path (`semver.rs`: `Version` with parse/compare/
//! matches-constraint). Full PRD syntax: exact, `=`, caret, tilde,
//! `>`/`>=`/`<`/`<=`, `*`/`latest` wildcards, and whitespace-separated
//! AND groups (`>=1.0.0 <2.0.0`).

pub use crate::package::version::{
    matches as matches_constraint, max_satisfying, parse_constraint, parse_version, version_cmp,
    Constraint, Predicate, Version,
};
