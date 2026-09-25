//! Klang v2 resonance tuner (Phase 6).
//!
//! Compatibility among `?T` (Dissonant), `T` (Consonant), and `!T`
//! (Harmonic). Rules (PRD §4.1):
//!
//! - `?T` flows only where `?T` is expected.
//! - `T` flows where `T` is expected.
//! - `!T` flows where `!T`, `T`, or a read-only `T` view is expected
//!   (modeled here as `!T` → `T` decay).
//! - `?T` never satisfies `T`/`!T` without `tune` (E-RESONANCE-UNPROVEN).
//! - A cast never produces `!T`, even from `!T` (E-RESONANCE-MISMATCH).
//! - `tune<T>` validates `?T` and mints an identity-bound [`Provenance`].

use crate::ai_safety::provenance::Provenance;
use crate::ai_safety::schema::{DataValue, Schema};
use crate::ast::resonance::QualifiedType;
use crate::diagnostics::Diagnostic;

/// True when `actual` may flow into a slot of type `expected`.
///
/// Bases must match exactly; qualifiers follow the decay rule above.
pub fn can_pass(actual: &QualifiedType, expected: &QualifiedType) -> bool {
    if actual.base != expected.base {
        return false;
    }
    use crate::ast::resonance::ResonanceQualifier as Q;
    match (actual.qualifier, expected.qualifier) {
        (Q::Dissonant, Q::Dissonant) => true,
        (Q::Consonant, Q::Consonant) => true,
        (Q::Harmonic, Q::Harmonic) => true,
        (Q::Harmonic, Q::Consonant) => true,
        _ => false,
    }
}

/// Check one argument flow; on failure emit the precise v2 diagnostic.
///
/// `?T` into `T`/`!T` is `E-RESONANCE-UNPROVEN` (names `tune` as the fix);
/// every other mismatch is `E-RESONANCE-MISMATCH`.
pub fn check_call(
    actual: &QualifiedType,
    expected: &QualifiedType,
    file: &str,
    start: usize,
    end: usize,
) -> Result<(), Diagnostic> {
    if can_pass(actual, expected) {
        return Ok(());
    }
    if actual.is_dissonant() && !expected.is_dissonant() {
        return Err(crate::ai_safety::diagnostics::resonance_unproven(
            file,
            start,
            end,
            &actual.display(),
            &expected.display(),
        )
        .with_types(&expected.display(), &actual.display()));
    }
    Err(crate::ai_safety::diagnostics::resonance_mismatch(
        file,
        start,
        end,
        &actual.display(),
        &expected.display(),
    )
    .with_types(&expected.display(), &actual.display()))
}

/// A cast never establishes Harmonic proof: any cast whose target is `!T`
/// is rejected, regardless of the source qualifier.
pub fn check_cast(
    actual: &QualifiedType,
    target: &QualifiedType,
    file: &str,
    start: usize,
    end: usize,
) -> Result<(), Diagnostic> {
    if target.is_harmonic() {
        return Err(Diagnostic::error(
            "E-RESONANCE-MISMATCH",
            &format!(
                "cast from `{}` to `{}` cannot establish Harmonic proof",
                actual.display(),
                target.display()
            ),
            file,
            start,
            end,
            "casts never validate against a schema",
            &["validate with tune<T>(value) instead"],
            "resonance/cast",
        ));
    }
    Ok(())
}

/// `tune<T>`: validate Dissonant `value` against `schema`, minting an
/// identity-bound proof on success or the nested [`VerifyError`](crate::ai_safety::schema::VerifyError)
/// path on failure.
pub fn tune(
    schema: &Schema,
    value: &DataValue,
    file: &str,
    span: (usize, usize),
) -> Result<Provenance, crate::ai_safety::schema::VerifyError> {
    schema.validate(value)?;
    Ok(Provenance::new(schema.identity(), file, span, "tune"))
}
