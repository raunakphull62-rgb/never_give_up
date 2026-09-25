//! Klang v2 `verify`/`tune` runtime entry points (Phase 6).
//!
//! Only these functions mint Harmonic proofs. A raw cast can never do so
//! (see `sema::tuner::check_cast`, which rejects every `!T` target).

use crate::ai_safety::provenance::Provenance;
use crate::ai_safety::schema::{DataValue, Schema, VerifyError};

/// Outcome of `tune<T>(value)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tuned {
    /// Validation succeeded: the value plus its identity-bound proof.
    Harmonic {
        /// The validated value.
        value: DataValue,
        /// Where it was validated.
        proof: Provenance,
    },
    /// Validation failed with the nested path.
    Rejected(VerifyError),
}

/// Validate `value` against `schema`, minting a proof on success.
///
/// `boundary` names the call site (`tune` or `verify`); the span marks the
/// validation boundary in `file`.
pub fn tune(
    schema: &Schema,
    value: DataValue,
    file: &str,
    span: (usize, usize),
    boundary: &str,
) -> Tuned {
    match schema.validate(&value) {
        Ok(()) => Tuned::Harmonic {
            value,
            proof: Provenance::new(schema.identity(), file, span, boundary),
        },
        Err(e) => Tuned::Rejected(e),
    }
}

/// `verify` is the boundary-checked alias of `tune`.
pub fn verify(schema: &Schema, value: DataValue, file: &str, span: (usize, usize)) -> Tuned {
    tune(schema, value, file, span, "verify")
}
