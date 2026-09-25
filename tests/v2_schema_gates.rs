//! Phase 5 — v2 schema gates: nested paths, identity-bound proofs.

use klang::ai_safety::diagnostics;
use klang::ai_safety::provenance::Provenance;
use klang::ai_safety::schema::{DataValue, FieldTy, Schema, SchemaField};
use std::collections::HashMap;

fn profile_schema() -> Schema {
    Schema::new(
        "Profile",
        "1",
        vec![SchemaField::new(
            "user",
            FieldTy::Record(vec![SchemaField::new(
                "address",
                FieldTy::Record(vec![SchemaField::new("zip", FieldTy::Str)]),
            )]),
        )],
    )
}

fn record(fields: Vec<(&str, DataValue)>) -> DataValue {
    DataValue::Record(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

#[test]
fn v2_schema_valid_nested_record() {
    let s = profile_schema();
    let v = record(vec![(
        "user",
        record(vec![(
            "address",
            record(vec![("zip", DataValue::Str("12345".to_string()))]),
        )]),
    )]);
    assert!(s.validate(&v).is_ok());
    // Deterministic: same input validates the same way twice.
    assert!(s.validate(&v).is_ok());
}

#[test]
fn v2_schema_invalid_reports_nested_path() {
    let s = profile_schema();
    let v = record(vec![(
        "user",
        record(vec![("address", record(vec![("zip", DataValue::Int(42))]))]),
    )]);
    let e = s.validate(&v).expect_err("wrong nested type fails");
    assert_eq!(e.path, "user.address.zip");
    assert_eq!(e.expected, "str");
    assert_eq!(e.found, "int");
    // The diagnostic carries the code, path, and a fix.
    let d = diagnostics::schema_invalid("prog.v2", 0, 4, &e);
    assert_eq!(d.code, "E-SCHEMA-INVALID");
    assert!(d.message.contains("user.address.zip"));
    assert!(!d.fixes.is_empty());
    let _m: HashMap<String, String> = HashMap::new();
}

#[test]
fn v2_schema_proof_bound_to_identity() {
    let a = Schema::new("A", "1", vec![SchemaField::new("x", FieldTy::Int)]);
    let b = Schema::new("B", "1", vec![SchemaField::new("x", FieldTy::Int)]);
    assert_ne!(a.identity(), b.identity());
    let proof = Provenance::new(a.identity(), "prog.v2", (0, 8), "tune");
    assert!(proof.satisfies(&a.identity()));
    assert!(!proof.satisfies(&b.identity()));
    // Same name, different version: different identity, proof unusable.
    let a2 = Schema::new("A", "2", vec![SchemaField::new("x", FieldTy::Int)]);
    assert!(!proof.satisfies(&a2.identity()));
    // Provenance preserves span and boundary.
    assert_eq!(proof.span, (0, 8));
    assert_eq!(proof.boundary, "tune");
}

#[test]
fn v2_schema_not_found_diagnostic() {
    let d = diagnostics::schema_not_found("prog.v2", 1, 3, "Missing");
    assert_eq!(d.code, "E-SCHEMA-NOT-FOUND");
    assert!(!d.fixes.is_empty());
}
