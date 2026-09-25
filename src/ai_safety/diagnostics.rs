//! Klang v2 diagnostic codes (Phase 5 foundation).
//!
//! Compatibility rule: every constructor returns the existing
//! [`crate::diagnostics::Diagnostic`] shape with v2 `code`/`rule` values,
//! so CLI, MCP, and tests consume v2 diagnostics identically to v1 ones.
//! Later phases add their own constructors here; codes are never reused
//! for a different rule.

use super::schema::VerifyError;
use crate::diagnostics::Diagnostic;

/// `E-SCHEMA-INVALID`: data failed schema validation at `path`.
pub fn schema_invalid(file: &str, start: usize, end: usize, err: &VerifyError) -> Diagnostic {
    Diagnostic::error(
        "E-SCHEMA-INVALID",
        &format!(
            "schema `{}` rejected `{}`: want {}, got {}",
            err.schema.0, err.path, err.expected, err.found
        ),
        file,
        start,
        end,
        &err.message,
        &[&format!(
            "fix `{}` so it matches schema `{}`",
            err.path, err.schema.0
        )],
        "schema/validation",
    )
}

/// `E-SCHEMA-NOT-FOUND`: no schema with this name is registered.
pub fn schema_not_found(file: &str, start: usize, end: usize, name: &str) -> Diagnostic {
    Diagnostic::error(
        "E-SCHEMA-NOT-FOUND",
        &format!("unknown schema `{name}`"),
        file,
        start,
        end,
        "no schema with this name is in scope",
        &["declare the schema first", "check the schema name"],
        "schema/scope",
    )
}

/// `E-RESONANCE-UNPROVEN`: `?T` used where `T`/`!T` is required.
pub fn resonance_unproven(
    file: &str,
    start: usize,
    end: usize,
    actual: &str,
    expected: &str,
) -> Diagnostic {
    Diagnostic::error(
        "E-RESONANCE-UNPROVEN",
        &format!("{actual} value cannot be passed where {expected} is required"),
        file,
        start,
        end,
        "dissonant values carry no safety proof",
        &[&format!(
            "validate the value first: tune<{expected}>({actual})"
        )],
        "resonance/proof",
    )
}

/// `E-RESONANCE-MISMATCH`: qualifier or base mismatch that `tune` cannot fix.
pub fn resonance_mismatch(
    file: &str,
    start: usize,
    end: usize,
    actual: &str,
    expected: &str,
) -> Diagnostic {
    Diagnostic::error(
        "E-RESONANCE-MISMATCH",
        &format!("cannot pass {actual} where {expected} is required"),
        file,
        start,
        end,
        "resonance qualifiers do not match",
        &["check the ?/!/unmarked qualifier on both sides"],
        "resonance/mismatch",
    )
}

/// `E-FLOW-MUTABLE-CAPTURE`: mutable outer read without `dep=`.
pub fn flow_mutable_capture(file: &str, start: usize, end: usize, name: &str) -> Diagnostic {
    Diagnostic::error(
        "E-FLOW-MUTABLE-CAPTURE",
        &format!("flow reads mutable outer `{name}` without an explicit dependency"),
        file,
        start,
        end,
        "flows cannot implicitly capture mutable outer state",
        &[&format!("pass it explicitly: dep={name}: <Type>")],
        "flow/capture",
    )
}

/// `E-FLOW-DUPLICATE-DEPENDENCY`: the same `dep=` name twice.
pub fn flow_duplicate_dependency(file: &str, start: usize, end: usize, name: &str) -> Diagnostic {
    Diagnostic::error(
        "E-FLOW-DUPLICATE-DEPENDENCY",
        &format!("duplicate flow dependency `{name}`"),
        file,
        start,
        end,
        "each dependency name must appear once",
        &["remove the duplicate entry", "rename one of them"],
        "flow/dependency",
    )
}

/// `E-ECHO-UNLISTENED`: an Echo handle reaches a scope exit unlistened.
pub fn echo_unlistened(file: &str, start: usize, end: usize, handle: &str) -> Diagnostic {
    Diagnostic::error(
        "E-ECHO-UNLISTENED",
        &format!("echo `{handle}` is never listened to"),
        file,
        start,
        end,
        "every echo must be listened to, transferred, or joined",
        &[&format!("retrieve it with listen({handle})")],
        "echo/lifetime",
    )
}

/// `E-ECHO-INVALID-LISTEN`: `listen` of an unknown/consumed handle.
pub fn echo_invalid_listen(
    file: &str,
    start: usize,
    end: usize,
    handle: &str,
    why: &str,
) -> Diagnostic {
    Diagnostic::error(
        "E-ECHO-INVALID-LISTEN",
        &format!("cannot listen to echo `{handle}`: {why}"),
        file,
        start,
        end,
        "listen requires an outstanding echo handle",
        &["check the handle name", "listen exactly once"],
        "echo/lifetime",
    )
}

/// `E-ECHO-OUTLIVES-SCOPE`: an Echo handle escapes its lexical scope
/// without being transferred to an owner.
pub fn echo_outlives_scope(file: &str, start: usize, end: usize, handle: &str) -> Diagnostic {
    Diagnostic::error(
        "E-ECHO-OUTLIVES-SCOPE",
        &format!("echo `{handle}` outlives its scope"),
        file,
        start,
        end,
        "echo handles cannot escape the scope that created them",
        &[
            &format!("listen to it with listen({handle}) before leaving"),
            "transfer it to an owner that will listen",
        ],
        "echo/lifetime",
    )
}

/// `E-ECHO-FAILURE-LOST`: an Echo failure path drops the cause instead of
/// delivering it to the listener.
pub fn echo_failure_lost(file: &str, start: usize, end: usize, handle: &str) -> Diagnostic {
    Diagnostic::error(
        "E-ECHO-FAILURE-LOST",
        &format!("echo `{handle}` failure would be lost"),
        file,
        start,
        end,
        "echo failures must reach the listener lossless",
        &[
            "listen for the failure on this path",
            "clean up only after listen",
        ],
        "echo/failure",
    )
}

/// `E-PARSE-FLOW`: input does not match the v2 Flow grammar.
pub fn parse_flow(file: &str, start: usize, end: usize, message: &str) -> Diagnostic {
    Diagnostic::error(
        "E-PARSE-FLOW",
        message,
        file,
        start,
        end,
        "input does not match the v2 flow grammar",
        &["write flow(params, dep=name: Type) -> Ret { body }"],
        "syntax/flow",
    )
}

/// `E-PARSE-ECHO`: input does not match the v2 Echo grammar.
pub fn parse_echo(file: &str, start: usize, end: usize, message: &str) -> Diagnostic {
    Diagnostic::error(
        "E-PARSE-ECHO",
        message,
        file,
        start,
        end,
        "input does not match the v2 echo grammar",
        &[
            "write echo fn name(params) -> Type { body }",
            "retrieve handles with listen(handle)",
        ],
        "syntax/echo",
    )
}
