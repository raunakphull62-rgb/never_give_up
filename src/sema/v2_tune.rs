//! Check-time validation of v2 `tune<T>()` / `verify<T>()` constants.
//!
//! BUGHUNT-2 (resolves HEAVY-TEST-2). Design decision, stated plainly:
//! `check-v2` validates tune/verify arguments **only when they are
//! compile-time constants** (struct literals of literal fields). A
//! constant bad value is rejected with the same `E-SCHEMA-INVALID` /
//! `E-SCHEMA-NOT-FOUND` the runtime already emits; a dynamic value
//! (`Var`, `Call`, `listen()`, arithmetic, …) is skipped and stays
//! runtime-checked. This deliberately does NOT build a full v2 semantic
//! pass — flow capture, echo lifetimes, and resonance qualifier checking
//! remain where they were (unit-tested library pieces, future roadmap).
//!
//! Runtime agreement (verified, not assumed): the v2 interpreter encodes
//! `Bool` literals as `Int(0/1)` (`runtime::v2`, `V2Expr::Bool` arm) and
//! `convert_for_ty` accepts `0`/`1` for `bool` fields while rejecting
//! other ints; whole-int/str/record literals validate identically under
//! `Schema::validate`. The constant conversion below mirrors exactly
//! that: `Bool`/`0`/`1` for a `bool` field become `Bool`, everything
//! else converts raw so `Schema::validate` (shared with the runtime via
//! `SchemaRegistry::check_boundary` — no parallel validator) decides.
//! There are no float literals in the v2 expression grammar, so no
//! float coercion exists to mirror.

use crate::ai_safety::schema::{DataValue, FieldTy};
use crate::ast::v2::{V2Block, V2Expr, V2Program, V2Stmt};
use crate::diagnostics::Diagnostic;
use crate::sema::schema_check::SchemaRegistry;
use std::collections::HashMap;

/// Check every `tune`/`verify` in a whole v2 program's function and echo
/// bodies, returning one diagnostic per statically-known-bad boundary.
/// Dynamic arguments produce no diagnostic (runtime-checked).
pub fn check_program_tunes(prog: &V2Program, file: &str) -> Vec<Diagnostic> {
    let mut reg = SchemaRegistry::new();
    let mut diags = Vec::new();
    for s in &prog.schemas {
        let mut fields = Vec::new();
        let mut bad = false;
        for f in &s.fields {
            match map_field_ty(&f.ty) {
                Ok(ty) => fields.push(crate::ai_safety::schema::SchemaField::new(
                    &f.name, ty,
                )),
                Err(d) => {
                    // Same code the runtime emits at startup for an
                    // undeclared field type (runtime::v2::map_schema_ty).
                    diags.push(with_file(d, file));
                    bad = true;
                }
            }
        }
        if !bad {
            reg.register(crate::ai_safety::schema::Schema::new(
                &s.name, &s.version, fields,
            ));
        }
    }
    for f in &prog.functions {
        check_block(&reg, &f.body, file, &mut diags);
    }
    for body in prog.echo_bodies.values() {
        check_block(&reg, body, file, &mut diags);
    }
    diags
}

/// Schema field-type mapping shared in meaning (not in code path) with
/// the runtime's startup mapping: `str`, `i32`/`int`, `bool`.
fn map_field_ty(s: &str) -> Result<FieldTy, Diagnostic> {
    match s.trim().to_lowercase().as_str() {
        "str" | "string" => Ok(FieldTy::Str),
        "i32" | "i64" | "int" => Ok(FieldTy::Int),
        "bool" => Ok(FieldTy::Bool),
        other => Err(crate::diagnostics::Diagnostic::error(
            "E-SCHEMA-NOT-FOUND",
            &format!("unknown schema field type `{other}`"),
            "input.v2",
            0,
            0,
            "schema field types are str, i32, bool",
            &["use str, i32, or bool"],
            "schema/scope",
        )),
    }
}

/// Re-label a diagnostic's file (sibling constructors hardcode the
/// `input.v2` placeholder; check-time knows the real path).
fn with_file(mut d: Diagnostic, file: &str) -> Diagnostic {
    d.primary_span.file = file.to_string();
    d
}

fn check_block(
    reg: &SchemaRegistry,
    block: &V2Block,
    file: &str,
    diags: &mut Vec<Diagnostic>,
) {
    for stmt in &block.stmts {
        match stmt {
            V2Stmt::Let(s) => check_expr(reg, &s.value, file, diags),
            V2Stmt::Assign(s) => check_expr(reg, &s.value, file, diags),
            V2Stmt::Return(s) => check_expr(reg, &s.value, file, diags),
            V2Stmt::Print(s) => check_expr(reg, &s.value, file, diags),
            V2Stmt::Expr(e) => check_expr(reg, e, file, diags),
            V2Stmt::If(s) => {
                check_expr(reg, &s.cond, file, diags);
                check_block(reg, &s.then_block, file, diags);
                if let Some(else_block) = &s.else_block {
                    check_block(reg, else_block, file, diags);
                }
            }
        }
    }
}

fn check_expr(
    reg: &SchemaRegistry,
    expr: &V2Expr,
    file: &str,
    diags: &mut Vec<Diagnostic>,
) {
    match expr {
        V2Expr::Tune(t) => {
            check_boundary(reg, &t.target_ty.base, &t.value, file, diags);
            check_expr(reg, &t.value, file, diags);
        }
        V2Expr::Verify(v) => {
            check_boundary(reg, &v.target_ty.base, &v.value, file, diags);
            check_expr(reg, &v.value, file, diags);
        }
        V2Expr::Call { args, .. } => {
            for arg in args {
                check_expr(reg, arg, file, diags);
            }
        }
        V2Expr::StructLit { fields, .. } => {
            for (_, v) in fields {
                check_expr(reg, v, file, diags);
            }
        }
        V2Expr::Add { left, right, .. }
        | V2Expr::Sub { left, right, .. }
        | V2Expr::Mul { left, right, .. }
        | V2Expr::Div { left, right, .. }
        | V2Expr::Mod { left, right, .. }
        | V2Expr::Eq { left, right, .. }
        | V2Expr::NotEq { left, right, .. }
        | V2Expr::Lt { left, right, .. }
        | V2Expr::LtEq { left, right, .. }
        | V2Expr::Gt { left, right, .. }
        | V2Expr::GtEq { left, right, .. }
        | V2Expr::And { left, right, .. }
        | V2Expr::Or { left, right, .. } => {
            check_expr(reg, left, file, diags);
            check_expr(reg, right, file, diags);
        }
        V2Expr::Index { base, index, .. } => {
            check_expr(reg, base, file, diags);
            check_expr(reg, index, file, diags);
        }
        V2Expr::Field { base, .. } => check_expr(reg, base, file, diags),
        // Flow bodies are held as source text, not expressions; leaf
        // nodes (Int/Str/Bool/Var/Listen) hold no tune sites.
        V2Expr::Flow(_) | V2Expr::Int { .. } | V2Expr::Str { .. } | V2Expr::Bool { .. } | V2Expr::Var { .. } | V2Expr::Listen { .. } => {}
    }
}

/// Validate one `tune`/`verify` boundary when its value is a constant.
/// Unknown schemas are `E-SCHEMA-NOT-FOUND` (same as the runtime);
/// dynamic values are skipped (runtime-checked).
fn check_boundary(
    reg: &SchemaRegistry,
    name: &str,
    value: &V2Expr,
    file: &str,
    diags: &mut Vec<Diagnostic>,
) {
    let schema = match reg.lookup(name, file, 0, 0) {
        Ok(s) => s.clone(),
        Err(d) => {
            diags.push(d);
            return;
        }
    };
    let fields: Vec<(String, FieldTy)> = schema
        .fields
        .iter()
        .map(|f| (f.name.clone(), f.ty.clone()))
        .collect();
    let Some(data) = const_record(value, &fields) else {
        return;
    };
    if let Err(d) = reg.check_boundary(name, &data, file, 0, 0) {
        diags.push(d);
    }
}

/// Convert a constant tune value to data, guided by the declared field
/// types so runtime-accepted coercions (`true`/`0`/`1` for `bool`)
/// validate the same statically. Returns `None` for dynamic values
/// (caller skips those); always returns `Some` for constants, even on
/// type mismatch, so `Schema::validate` names the real disagreement
/// exactly as the runtime does.
fn const_record(value: &V2Expr, fields: &[(String, FieldTy)]) -> Option<DataValue> {
    match value {
        V2Expr::StructLit { fields: lit, .. } => {
            let mut map = HashMap::new();
            for (name, v) in lit {
                let expected = fields.iter().find(|(n, _)| n == name).map(|(_, t)| t);
                match const_scalar(v, expected) {
                    Some(dv) => {
                        map.insert(name.clone(), dv);
                    }
                    // A dynamic *declared* field makes the value genuinely
                    // dynamic (runtime-checked). A dynamic *extra* field
                    // is ignored by the runtime, so omit it and keep
                    // checking the declared fields.
                    None if expected.is_some() => return None,
                    None => {}
                }
            }
            Some(DataValue::Record(map))
        }
        // A constant non-record (e.g. `tune<S>(42)`) is statically
        // known-bad the same way at runtime ("must be a record struct").
        V2Expr::Int { value: n, .. } => Some(DataValue::Int(*n)),
        V2Expr::Str { value: s, .. } => Some(DataValue::Str(s.clone())),
        V2Expr::Bool { value: b, .. } => Some(DataValue::Bool(*b)),
        _ => None,
    }
}

/// Convert one constant field value; `None` only for dynamic values.
fn const_scalar(expr: &V2Expr, expected: Option<&FieldTy>) -> Option<DataValue> {
    match expr {
        V2Expr::Int { value: n, .. } => match expected {
            // Runtime mirror: the interpreter encodes `Bool` as
            // `Int(0/1)` and accepts both for `bool` fields.
            Some(FieldTy::Bool) if *n == 0 => Some(DataValue::Bool(false)),
            Some(FieldTy::Bool) if *n == 1 => Some(DataValue::Bool(true)),
            _ => Some(DataValue::Int(*n)),
        },
        V2Expr::Str { value: s, .. } => Some(DataValue::Str(s.clone())),
        V2Expr::Bool { value: b, .. } => Some(DataValue::Bool(*b)),
        V2Expr::StructLit { .. } => {
            // Guided when the field declares a nested record; raw
            // otherwise (extra fields are ignored by validation anyway,
            // exactly as the runtime ignores them — they must never
            // block checking the declared fields).
            let guided: Vec<(String, FieldTy)> = match expected {
                Some(FieldTy::Record(inner)) => inner
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone()))
                    .collect(),
                _ => Vec::new(),
            };
            const_record(expr, &guided)
        }
        _ => None,
    }
}
