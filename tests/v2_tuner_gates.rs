//! Phase 6 — v2 tuner gates: compatibility, tune proofs, no forged casts.

use klang::ai_safety::schema::{DataValue, FieldTy, Schema, SchemaField};
use klang::ast::resonance::QualifiedType;
use klang::sema::{schema_check::SchemaRegistry, tuner};
use klang::stdlib::verify;
use std::collections::HashMap;

fn profile_schema() -> Schema {
    Schema::new("Profile", "1", vec![SchemaField::new("name", FieldTy::Str)])
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
fn v2_tuner_unproven_without_tune() {
    let given = QualifiedType::parse("?Data");
    let wanted = QualifiedType::parse("!Data");
    assert!(!tuner::can_pass(&given, &wanted));
    let e = tuner::check_call(&given, &wanted, "prog.v2", 0, 5).expect_err("fails");
    assert_eq!(e.code, "E-RESONANCE-UNPROVEN");
    assert!(e.message.contains("?Data"));
    assert!(!e.fixes.is_empty());
    // `?T` into plain `T` also needs proof.
    let e = tuner::check_call(&given, &QualifiedType::parse("Data"), "prog.v2", 0, 5)
        .expect_err("fails");
    assert_eq!(e.code, "E-RESONANCE-UNPROVEN");
}

#[test]
fn v2_tuner_tune_success_is_harmonic() {
    let s = profile_schema();
    let v = record(vec![("name", DataValue::Str("ada".to_string()))]);
    let out = verify::tune(&s, v.clone(), "prog.v2", (0, 4), "tune");
    match out {
        verify::Tuned::Harmonic { value, proof } => {
            assert_eq!(value, v);
            assert!(proof.satisfies(&s.identity()));
            assert_eq!(proof.boundary, "tune");
        }
        verify::Tuned::Rejected(e) => panic!("should validate: {e:?}"),
    }
    // `!T` decays to `T` but never back to `?T`.
    assert!(tuner::can_pass(
        &QualifiedType::parse("!Profile"),
        &QualifiedType::parse("Profile")
    ));
    assert!(!tuner::can_pass(
        &QualifiedType::parse("!Profile"),
        &QualifiedType::parse("?Profile")
    ));
}

#[test]
fn v2_tuner_invalid_data_is_schema_invalid() {
    let mut reg = SchemaRegistry::new();
    reg.register(profile_schema());
    let bad = record(vec![("name", DataValue::Int(7))]);
    let e = reg
        .check_boundary("Profile", &bad, "prog.v2", 2, 6)
        .expect_err("fails");
    assert_eq!(e.code, "E-SCHEMA-INVALID");
    assert!(e.message.contains("name"));
    assert!(!e.fixes.is_empty());
    // Unknown schema names stay lookup errors, not validation errors.
    let e = reg
        .check_boundary("Missing", &bad, "prog.v2", 0, 1)
        .expect_err("fails");
    assert_eq!(e.code, "E-SCHEMA-NOT-FOUND");
    let _m: HashMap<String, String> = HashMap::new();
}

#[test]
fn v2_tuner_cast_never_proves() {
    // Even `!T -> !T` via cast is rejected: only tune/verify mint proofs.
    for src in ["?Data", "Data", "!Data"] {
        let e = tuner::check_cast(
            &QualifiedType::parse(src),
            &QualifiedType::parse("!Data"),
            "prog.v2",
            0,
            3,
        )
        .expect_err("cast to !T always fails");
        assert_eq!(e.code, "E-RESONANCE-MISMATCH");
    }
    // Non-harmonic cast targets are not the tuner's business.
    assert!(tuner::check_cast(
        &QualifiedType::parse("?Data"),
        &QualifiedType::parse("?Data"),
        "prog.v2",
        0,
        1
    )
    .is_ok());
}

#[test]
fn v2_tuner_v1_regression() {
    let mut p = klang::parser::Parser::new(
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(1, 2) }",
    );
    let prog = p.parse_program().expect("v1 parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}
