//! Klang OS-interop regex boundary (STDLIB-OSIO-3).
//!
//! `regex::is_match(pattern, text)` / `regex::find(pattern, text)` in the
//! PRD map onto Klang's existing flat builtins `regex_is_match` /
//! `regex_find` (see `hir::is_builtin` and `runtime::exec_builtin`).
//! Flat names are deliberate: Klang's `::` call syntax only resolves
//! through declared `mod` blocks (`modules::rewrite_ctor` converts
//! `m::f(args)` into a `Call` only when `m` is a program module), so a
//! namespaced `regex::is_match(...)` spelling would parse as an enum
//! constructor and fail type checking. Consistency with the existing
//! builtins matters more than the PRD's illustrative `::` shape.
//!
//! Klang has no `Option` type, so `find` returns a map in both cases:
//! `{"matched": 0/1, "match": full-text-or-"", "groups": [...],
//! "named": {...}}`. A non-match is a normal result (`matched == 0`),
//! NOT an error — parallel to Phase 2's non-zero-exit decision. Only a
//! malformed pattern raises `E-REGEX-INVALID-PATTERN`, carrying the real
//! parse error from the `regex` crate (never a generic message).
//!
//! All matching goes through the free functions here so both the
//! interpreter (`runtime::exec_builtin`) and unit tests share one error
//! mapping. Patterns compile per call (no cache) — correct for v1.

use crate::diagnostics::Diagnostic;

/// `E-REGEX-INVALID-PATTERN`: malformed regex pattern. The underlying
/// `regex`-crate parse error is carried as the cause verbatim.
fn regex_invalid_pattern(op: &str, pattern: &str, cause: &str) -> Diagnostic {
    Diagnostic::error(
        "E-REGEX-INVALID-PATTERN",
        &format!("{op}({pattern}) failed: invalid regex pattern: {cause}"),
        "runtime",
        0,
        0,
        cause,
        &["check the pattern syntax"],
        "regex/pattern",
    )
}

/// Map a `regex`-crate build error to `E-REGEX-INVALID-PATTERN` for `op`.
pub fn map_regex_error(op: &str, pattern: &str, err: &regex::Error) -> Diagnostic {
    regex_invalid_pattern(op, pattern, &err.to_string())
}

/// Test whether `pattern` matches anywhere in `text`.
pub fn is_match(pattern: &str, text: &str) -> Result<bool, Diagnostic> {
    let re = regex::Regex::new(pattern)
        .map_err(|e| map_regex_error("regex_is_match", pattern, &e))?;
    Ok(re.is_match(text))
}

/// First-match result. `matched == false` means "no match" (normal
/// result, not an error): `text` is `""`, `groups` is empty, `named`
/// is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindResult {
    /// Whether the pattern matched.
    pub matched: bool,
    /// Full matched text (`""` when no match).
    pub text: String,
    /// Indexed capture groups 1..n in order (`""` for groups that did
    /// not participate in the match).
    pub groups: Vec<String>,
    /// Named capture groups (`name -> captured-or-""`).
    pub named: Vec<(String, String)>,
}

/// Find the first match of `pattern` in `text`, with capture groups.
pub fn find(pattern: &str, text: &str) -> Result<FindResult, Diagnostic> {
    let re =
        regex::Regex::new(pattern).map_err(|e| map_regex_error("regex_find", pattern, &e))?;
    match re.captures(text) {
        None => Ok(FindResult {
            matched: false,
            text: String::new(),
            groups: Vec::new(),
            named: Vec::new(),
        }),
        Some(caps) => {
            let text = caps
                .get(0)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let mut groups = Vec::with_capacity(caps.len().saturating_sub(1));
            for i in 1..caps.len() {
                groups.push(
                    caps.get(i)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                );
            }
            let mut named = Vec::new();
            for (i, name) in re.capture_names().enumerate() {
                if let Some(n) = name {
                    named.push((
                        n.to_string(),
                        caps.get(i)
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default(),
                    ));
                }
            }
            Ok(FindResult {
                matched: true,
                text,
                groups,
                named,
            })
        }
    }
}
