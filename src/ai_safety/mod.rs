//! Klang v2 AI-safety boundary (Phase 5).
//!
//! Runtime schemas ([`schema`]), Harmonic proof provenance
//! ([`provenance`]), and v2 diagnostic codes ([`diagnostics`]).
//! Validation is deterministic and local: no registry, no network.

/// v2 structured diagnostics (existing `Diagnostic` shape, v2 codes).
pub mod diagnostics;
/// Where a Harmonic value was validated.
pub mod provenance;
/// Runtime schemas and deterministic local validation.
pub mod schema;

pub use schema::SchemaId;
