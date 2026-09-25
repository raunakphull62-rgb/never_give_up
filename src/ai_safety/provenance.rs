//! Klang v2 proof provenance (Phase 5).
//!
//! A Harmonic value (`!T`) carries where it was validated: the schema
//! identity, the source span of the validation boundary, and a note naming
//! the boundary (`tune`/`verify`). Proofs bind to exactly one schema
//! identity and cannot be reused across schemas.

use super::schema::SchemaId;

/// Where a Harmonic value was validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// Schema that validated the value.
    pub schema: SchemaId,
    /// File containing the validation boundary.
    pub file: String,
    /// Byte span of the validating call.
    pub span: (usize, usize),
    /// Boundary kind (`tune`, `verify`).
    pub boundary: String,
}

impl Provenance {
    /// Record a new proof.
    pub fn new(schema: SchemaId, file: &str, span: (usize, usize), boundary: &str) -> Self {
        Self {
            schema,
            file: file.to_string(),
            span,
            boundary: boundary.to_string(),
        }
    }

    /// True when this proof validates `wanted` (exact identity match).
    ///
    /// A proof for schema `A` never satisfies schema `B`: identity covers
    /// name, version, and content hash.
    pub fn satisfies(&self, wanted: &SchemaId) -> bool {
        &self.schema == wanted
    }
}
