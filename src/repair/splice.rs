//! Splicing: deterministic recombination + full-program re-check (PRD 5.2.f-g).
//!
//! Text-level splice (replace the target `fn` slice(s) with model output,
//! re-parse the whole file, re-run full `TypedHIR::check`): preserves
//! untouched bytes, never checks a scope in isolation. Import-safety
//! rules from `main.rs::load_with_imports` apply unchanged to any
//! model-generated import lines.

use crate::diagnostics::Diagnostic;
use crate::hir::TypedHIR;
use crate::parser::Parser;

use super::scope::function_spans;

/// Recombination result.
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Recombined full-file source.
    pub source: String,
    /// Diagnostics after re-check (empty = success).
    pub diagnostics: Vec<Diagnostic>,
}

/// Single source of truth for "this import path must never be followed":
/// absolute paths, `..` anywhere, leading `~`, NUL bytes. Used by the
/// CLI loader and by the model-output screen alike, so the two can never
/// drift apart again. (The runtime file builtins additionally allow
/// absolute paths under the temp dir; imports never need that carve-out.)
pub fn is_unsafe_import_path(imp: &str) -> bool {
    let p = std::path::Path::new(imp);
    p.is_absolute() || imp.contains("..") || imp.starts_with('~') || imp.contains('\0')
}

/// Reject unsafe model-generated imports (same rule as load_with_imports).
pub fn has_unsafe_import(src: &str) -> Option<String> {
    for line in src.lines() {
        let t = line.trim();
        // Accept every separator the parser accepts between the keyword
        // and the path: runs of whitespace, tabs, or none at all
        // (`import"..."` lexes identically). A bare `import` prefix with
        // any other continuation (`important ...`) is not an import.
        let rest = match t.strip_prefix("import") {
            Some(r) if r.starts_with('"') => r,
            Some(r) if r.starts_with(|c: char| c.is_whitespace()) => r.trim_start(),
            _ => continue,
        };
        // import "path"
        let q1 = match rest.find('"') {
            Some(i) => i,
            None => return Some(t.to_string()),
        };
        let after = &rest[q1 + 1..];
        let q2 = match after.find('"') {
            Some(i) => i,
            None => return Some(t.to_string()),
        };
        let imp = &after[..q2];
        if is_unsafe_import_path(imp) {
            return Some(imp.to_string());
        }
    }
    None
}

/// Extract candidate function bodies by name from model output.
/// Returns (name, body-text) for each top-level `fn` found.
pub fn extract_functions(raw: &str) -> Vec<(String, String)> {
    function_spans(raw)
        .into_iter()
        .map(|(n, s, e)| {
            let body = raw[s..e].to_string();
            (n, body)
        })
        .collect()
}

/// Lex a top-level declaration keyword (`enum`/`struct`/`mod`/`import`)
/// and the identifier that follows it, requiring statement position (first
/// token on its line) so that mentions inside comments/strings on an
/// indented line are not mistaken for declarations.
fn decl_at(seg: &str, at: usize, kw: &str) -> bool {
    let b = seg.as_bytes();
    let prev_ok = at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_');
    let after = at + kw.len();
    let next_ok = b
        .get(after)
        .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_');
    // Statement position: only whitespace between the previous newline and
    // the keyword (so `// enum Fake` and `print("enum X")` never register).
    let line_start = match seg[..at].rsplit_once('\n') {
        Some((_, tail)) => tail.trim().is_empty(),
        None => seg[..at].trim().is_empty(),
    };
    prev_ok && next_ok && line_start
}

/// Names after a declaration keyword (`enum Opt` -> `Opt`).
fn decl_name(rest: &str) -> String {
    let rest = rest.trim_start();
    let n: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '"' || *c == '.' || *c == '/')
        .collect();
    n
}

/// Top-level declarations (`enum`/`struct`/`mod`/`import`) found OUTSIDE any
/// `fn` body in `src`, as (keyword, name) pairs.
///
/// Used to detect file-level edits in a model response: a function-scope
/// splice cannot express them, so they must either be accepted via a
/// whole-file splice or refused loudly -- never silently discarded.
pub fn response_declarations(src: &str) -> Vec<(String, String)> {
    let mut gaps: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0usize;
    for (_, s, e) in function_spans(src) {
        if s > cursor {
            gaps.push((cursor, s));
        }
        cursor = e;
    }
    if cursor < src.len() {
        gaps.push((cursor, src.len()));
    }
    let mut out = Vec::new();
    for (gs, ge) in gaps {
        let seg = &src[gs..ge];
        for kw in ["enum", "struct", "mod", "import"] {
            let mut from = 0usize;
            while let Some(rel) = seg[from..].find(kw) {
                let at = from + rel;
                if decl_at(seg, at, kw) {
                    let name = decl_name(&seg[at + kw.len()..]);
                    let name = if name.is_empty() {
                        kw.to_string()
                    } else {
                        name
                    };
                    if !out.iter().any(|(k, n)| k == kw && n == &name) {
                        out.push((kw.to_string(), name));
                    }
                }
                from = at + 1;
            }
        }
    }
    out
}

/// Splice candidate function(s) into the original source.
/// - `targets`: functions under repair (file order).
/// - `raw`: model output (may contain exactly those `fn`s, possibly plus
///   unchanged copies of others, possibly the whole file).
///
/// Strategy, in order:
/// 1. If the response contains top-level declaration text (enum/struct/
///    mod/import -- i.e. a file-level edit), it is only accepted when it is
///    a complete file: every original `fn` name AND every original
///    declaration name is present. Otherwise it is refused with an explicit
///    error, because silently dropping the declaration edit (or the
///    declarations it replaces) would be a correctness bug in a tool that
///    is meant to be trusted.
/// 2. Otherwise, if the response contains every original `fn` name, treat it
///    as whole-file output and use it directly.
/// 3. Otherwise extract per-function slices for `targets` and replace them
///    in the original source, leaving every other byte untouched.
pub fn splice_functions(orig: &str, targets: &[String], raw: &str) -> Result<String, String> {
    let orig_spans = function_spans(orig);
    let orig_names: Vec<String> = orig_spans.iter().map(|(n, _, _)| n.clone()).collect();
    let orig_decls = response_declarations(orig);
    let raw_decls = response_declarations(raw);
    let cands = extract_functions(raw);
    let cand_names: Vec<String> = cands.iter().map(|(n, _)| n.clone()).collect();
    let complete_file =
        orig_names.iter().all(|n| cand_names.contains(n)) && cands.len() >= orig_names.len();
    if !raw_decls.is_empty() {
        let keeps_decls = orig_decls.iter().all(|d| raw_decls.contains(d));
        if complete_file && keeps_decls {
            return Ok(raw.trim().to_string() + "\n");
        }
        let kinds: Vec<String> = raw_decls.iter().map(|(k, n)| format!("{k} {n}")).collect();
        return Err(format!(
            "response contains declaration-level edit(s) ({}) but is not a complete file \
             (original functions: {}; keeping declarations: {}) -- refusing to silently \
             drop declaration changes; re-emit the whole file",
            kinds.join(", "),
            orig_names.join(", "),
            keeps_decls
        ));
    }
    if cands.is_empty() {
        return Err("model response contains no `fn` block".to_string());
    }
    // Whole-file output without declarations (declaration-free programs).
    if complete_file && orig_decls.is_empty() {
        return Ok(raw.trim().to_string() + "\n");
    }
    // Per-function splice from back to front (offsets stay valid).
    let mut out = orig.to_string();
    let mut spans = orig_spans;
    // unknown-function guard: every candidate must name a real target
    // (or an original function, tolerated) -- else the model invented scope.
    for (n, _) in &cands {
        if !orig_names.contains(n) {
            return Err(format!("model introduced unknown function `{n}`"));
        }
    }
    for t in targets {
        let body = match cands.iter().find(|(n, _)| n == t) {
            Some((_, b)) => b.clone(),
            None => return Err(format!("model response missing repaired function `{t}`")),
        };
        if let Some((_, s, e)) = spans.iter().find(|(n, _, _)| n == t) {
            let (s, e) = (*s, *e);
            out.replace_range(s..e, body.trim());
            // recompute spans after each replacement (simple + deterministic)
            spans = function_spans(&out);
        } else {
            return Err(format!("target function `{t}` not found in original"));
        }
    }
    Ok(out)
}

/// Parse + full-program `TypedHIR::check` of a candidate source string.
/// Parse failures become a synthetic `E-PARSE` diagnostic so the loop can
/// feed them back to the model like any other diagnostic.
pub fn check_candidate(source: &str) -> CheckResult {
    let mut p = Parser::new(source);
    let prog = match p.parse_program() {
        Ok(prog) => prog,
        Err(d) => {
            return CheckResult { source: source.to_string(), diagnostics: vec![d] };
        }
    };
    match TypedHIR::check(prog) {
        Ok(_) => CheckResult { source: source.to_string(), diagnostics: vec![] },
        Err(ds) => CheckResult { source: source.to_string(), diagnostics: ds },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splice_replaces_only_target() {
        let orig = "fn a() -> i32 {\n    return 1\n}\n\nfn bad() -> i32 {\n    return x\n}\n";
        let raw = "fn bad() -> i32 {\n    return 2\n}\n";
        let out = splice_functions(orig, &["bad".to_string()], raw).expect("splices");
        assert!(out.contains("fn a() -> i32"), "{out}");
        assert!(out.contains("return 1"), "{out}");
        assert!(out.contains("return 2"), "{out}");
        assert!(!out.contains("return x"), "{out}");
        let r = check_candidate(&out);
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    }
    #[test]
    fn whole_file_passthrough() {
        let orig = "fn a() -> i32 { return 1 }\nfn b() -> i32 { return 2 }\n";
        let raw = "fn a() -> i32 { return 1 }\nfn b() -> i32 { return 3 }\n";
        let out = splice_functions(orig, &["b".to_string()], raw).expect("splices");
        assert!(out.contains("return 3"), "{out}");
    }
    #[test]
    fn unknown_function_rejected() {
        let orig = "fn a() -> i32 { return 1 }\n";
        let e = splice_functions(orig, &["a".to_string()], "fn evil() -> i32 { return 1 }\n").expect_err("reject");
        assert!(e.contains("unknown function"), "{e}");
    }
    #[test]
    fn unsafe_import_flagged() {
        assert!(has_unsafe_import("import \"../../etc/passwd\"").is_some());
        assert!(has_unsafe_import("import \"lib.klang\"").is_none());
    }
    #[test]
    fn parse_error_becomes_diagnostic() {
        let r = check_candidate("fn broken( -> i32 { return }");
        assert!(!r.diagnostics.is_empty());
        assert_eq!(r.diagnostics[0].code, "E-PARSE");
    }
}
