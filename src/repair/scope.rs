//! Scope planner: smallest enclosing function(s) per diagnostic (PRD 5.2.b-d).
//!
//! v1 reality: most HIR diagnostics carry `0,0` spans today (F12 second
//! series threaded the real `file` into every checker diagnostic, so the
//! file field is accurate; byte offsets remain unthreaded),
//! so byte-offset mapping alone cannot locate them. The planner therefore
//! layers: (1) real-span slice mapping when spans are non-degenerate,
//! (2) name-based attribution from diagnostic messages (caller/callee in
//! backticks, `undefined variable \`x\`` use/def scan, arity call sites),
//! (3) fallback to whole-file when diagnostics span > MAX_FUNCTION_SCOPE
//! functions or attribution fails.

use crate::diagnostics::Diagnostic;

/// Max functions repaired at function scope before falling back to file.
pub const MAX_FUNCTION_SCOPE: usize = 2;

/// Diagnostic rules whose fix target is a **declaration**, not a function
/// body: the repair has to rename a duplicated item, add a `pub`, or change
/// a module boundary. Function-scoped splicing cannot express these, and
/// name-keyed splicing is ambiguous exactly when two items share a name, so
/// the planner escalates to whole-file scope instead of guessing.
///
/// This is the explicit decision the Phase 1 audit required for the
/// declaration-level gap: diagnostics in these rules always repair at file
/// scope, and `--scope function` therefore never claims to handle them.
pub const DECLARATION_RULES: &[&str] = &["names/duplicate", "modules/visibility", "ownership/mode"];

/// True when this diagnostic's fix target is a declaration (see
/// [`DECLARATION_RULES`]).
pub fn is_declaration_level(d: &Diagnostic) -> bool {
    DECLARATION_RULES.contains(&d.rule.as_str())
}

/// Planned repair target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairScope {
    /// Regenerate these functions only (in listed order).
    Functions(Vec<String>),
    /// Regenerate the whole target file.
    File,
}

impl RepairScope {
    pub fn desc(&self) -> String {
        match self {
            Self::Functions(fs) => format!("function(s): {}", fs.join(", ")),
            Self::File => "whole file".to_string(),
        }
    }
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File)
    }
}

/// Byte span of one top-level `fn` in `src` (brace-balanced from the
/// `fn <name>` match). Returns (name, start, end) triples in file order.
pub fn function_spans(src: &str) -> Vec<(String, usize, usize)> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if is_fn_at(bytes, i) {
            let start = i;
            let mut j = i + 2;
            while j < bytes.len()
                && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n' || bytes[j] == b'\r')
            {
                j += 1;
            }
            // optional `pub `
            if src[j..].starts_with("pub ") {
                j += 4;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
            }
            let ns = j;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let name = src[ns..j].to_string();
            // find opening brace of the body
            let mut k = j;
            let mut depth = 0i32;
            let mut end = bytes.len();
            let mut opened = false;
            let mut in_str = false;
            let mut in_line_comment = false;
            let mut in_block_comment = false;
            while k < bytes.len() {
                let c = bytes[k];
                if in_block_comment {
                    // Non-nesting, mirroring the lexer: the first `*/`
                    // closes. Braces inside never affect depth.
                    if c == b'*' && k + 1 < bytes.len() && bytes[k + 1] == b'/' {
                        in_block_comment = false;
                        k += 2;
                        continue;
                    }
                    k += 1;
                    continue;
                }
                if in_line_comment {
                    if c == b'\n' {
                        in_line_comment = false;
                    }
                    k += 1;
                    continue;
                }
                if in_str {
                    if c == b'\\' {
                        k += 2;
                        continue;
                    }
                    if c == b'"' {
                        in_str = false;
                    }
                    k += 1;
                    continue;
                }
                match c {
                    b'"' => in_str = true,
                    b'/' if k + 1 < bytes.len() && bytes[k + 1] == b'/' => in_line_comment = true,
                    b'/' if k + 1 < bytes.len() && bytes[k + 1] == b'*' => {
                        in_block_comment = true;
                        k += 1;
                    }
                    b'{' => {
                        depth += 1;
                        opened = true;
                    }
                    b'}' => {
                        depth -= 1;
                        if opened && depth == 0 {
                            end = k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            if !name.is_empty() {
                out.push((name, start, end));
            }
            i = end.max(start + 1);
        } else {
            i += 1;
        }
    }
    out
}

fn is_fn_at(b: &[u8], i: usize) -> bool {
    if b[i] != b'f' || b.get(i + 1) != Some(&b'n') {
        return false;
    }
    let prev_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
    let next_ok = b
        .get(i + 2)
        .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_');
    prev_ok && next_ok
}

/// Extract backtick-quoted names from a message (caller/callee/var names).
fn backticked(msg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = msg.split('`');
    let mut idx = 0;
    for part in it.by_ref() {
        idx += 1;
        // odd segments (1-based even index) are inside backticks
        if idx % 2 == 0 && !part.is_empty() {
            // callee may be `a::b`; keep full path and leaf
            out.push(part.to_string());
            if let Some(leaf) = part.rsplit("::").next() {
                if leaf != part {
                    out.push(leaf.to_string());
                }
            }
        }
    }
    out
}

/// Plan the repair scope for these diagnostics over `src`.
/// `force_file` implements `--scope file`.
pub fn plan_scope(diags: &[Diagnostic], src: &str, force_file: bool) -> RepairScope {
    if force_file {
        return RepairScope::File;
    }
    // Declaration-level diagnostics (duplicate names, visibility, ownership
    // mode) always repair at file scope: function-scoped splicing cannot
    // express a declaration edit, and name-keyed splicing is ambiguous
    // exactly when names collide.
    if diags.iter().any(is_declaration_level) {
        return RepairScope::File;
    }
    let spans = function_spans(src);
    if spans.is_empty() {
        return RepairScope::File;
    }
    let names: Vec<String> = spans.iter().map(|(n, _, _)| n.clone()).collect();
    let mut hits: Vec<String> = Vec::new();
    for d in diags {
        for f in attribute(d, src, &spans, &names) {
            if !hits.contains(&f) {
                hits.push(f);
            }
        }
    }
    if hits.is_empty() || hits.len() > MAX_FUNCTION_SCOPE {
        return RepairScope::File;
    }
    // keep file order for deterministic prompts/splices
    let mut ordered: Vec<String> = spans
        .iter()
        .map(|(n, _, _)| n.clone())
        .filter(|n| hits.contains(n))
        .collect();
    ordered.dedup();
    RepairScope::Functions(ordered)
}

/// Attribute one diagnostic to candidate function names.
fn attribute(
    d: &Diagnostic,
    src: &str,
    spans: &[(String, usize, usize)],
    names: &[String],
) -> Vec<String> {
    // 1. Real byte spans: map slice -> enclosing function(s).
    let s = d.primary_span.start;
    let e = d.primary_span.end;
    if !(s == 0 && e == 0) && s < e && e <= src.len() {
        let mut v = Vec::new();
        for (n, fs, fe) in spans {
            if s >= *fs && e <= *fe {
                v.push(n.clone());
            }
        }
        if !v.is_empty() {
            return v;
        }
    }
    // 2. Backticked names that match known functions (caller preferred).
    // E-EFFECT-MISMATCH: "`caller` calls throwing `callee`..." -> caller.
    // E-ARITY: "`caller` calls `callee` with..." -> caller.
    // E-MATCH-*: "non-exhaustive match on `Enum`" names the TYPE, and arm
    //   diagnostics name variants -- neither is usually a function name, so
    //   fall through to the use-site scan below, which attributes via the
    //   enum/pattern tokens mentioned in the message (the function whose
    //   body contains the `match` on that enum wins).
    // E-UNDEFINED / E-TYPE: backticked var/fn if it names a function, else
    // scan for the name's use/def sites.
    let mut cands = Vec::new();
    for tok in backticked(&d.message) {
        if names.contains(&tok) && !cands.contains(&tok) {
            cands.push(tok);
        }
    }
    // E-ARITY / E-EFFECT-MISMATCH mention caller first: prefer first hit.
    if (d.code == "E-ARITY" || d.code == "E-EFFECT-MISMATCH") && !cands.is_empty() {
        return vec![cands[0].clone()];
    }
    if !cands.is_empty() {
        return cands;
    }
    // 2b. `match/*` diagnostics name the enum (or a variant path), not the
    // function. The real target is the function whose body contains the
    // `match`; the generic use-site scan below would also match every
    // caller that merely mentions the enum, over-widening the scope (and
    // then forcing the model to re-emit unrelated functions). Tokens that
    // are themselves function names were already handled in step 2.
    if d.rule.starts_with("match/") {
        if let Some(tok) = backticked(&d.message)
            .into_iter()
            .find(|t| !names.contains(t))
        {
            let hits: Vec<String> = spans
                .iter()
                .filter(|(_, s, e)| {
                    let body = &src[*s..*e];
                    body.contains("match") && token_in(body, &tok)
                })
                .map(|(n, _, _)| n.clone())
                .collect();
            if !hits.is_empty() {
                return hits;
            }
        }
    }
    // 3. Undefined-variable use/def scan: which functions mention the name?
    for tok in backticked(&d.message) {
        let users = functions_using(src, spans, &tok);
        if !users.is_empty() {
            return users;
        }
    }
    // E-TASK-CANCEL / spawn / await mentions: functions containing them.
    if d.code == "E-TASK-CANCEL"
        || d.message.contains("spawn")
        || d.message.contains("await")
        || d.message.contains("task_group")
    {
        let mut v = Vec::new();
        for (n, fs, fe) in spans {
            let body = &src[*fs..*fe];
            if body.contains("spawn") || body.contains("task_group") || body.contains("await") {
                v.push(n.clone());
            }
        }
        if !v.is_empty() {
            return v;
        }
    }
    vec![]
}

/// Functions whose body text mentions `name` as a token.
fn functions_using(src: &str, spans: &[(String, usize, usize)], name: &str) -> Vec<String> {
    let mut v = Vec::new();
    for (n, fs, fe) in spans {
        if token_in(&src[*fs..*fe], name) {
            v.push(n.clone());
        }
    }
    v
}

fn token_in(hay: &str, tok: &str) -> bool {
    if tok.is_empty() {
        return false;
    }
    let hb = hay.as_bytes();
    let tb = tok.as_bytes();
    if tb.len() > hb.len() {
        return false;
    }
    let mut i = 0usize;
    while i + tb.len() <= hb.len() {
        if &hb[i..i + tb.len()] == tb {
            let prev = if i == 0 { None } else { Some(hb[i - 1]) };
            let next = hb.get(i + tb.len()).copied();
            let pok = prev.is_none_or(|c| !c.is_ascii_alphanumeric() && c != b'_');
            let nok = next.is_none_or(|c| !c.is_ascii_alphanumeric() && c != b'_');
            if pok && nok {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Diagnostic;

    fn diag(code: &str, msg: &str) -> Diagnostic {
        Diagnostic::error(code, msg, "input.warden", 0, 0, "c", &["fix"], "r")
    }

    #[test]
    fn spans_find_functions() {
        let src = "fn a() -> i32 { return 1 }\nfn bad() -> i32 { return await b }\n";
        let sp = function_spans(src);
        assert_eq!(sp.len(), 2);
        assert_eq!(sp[0].0, "a");
        assert_eq!(sp[1].0, "bad");
        assert!(src[sp[1].1..sp[1].2].contains("await b"));
    }

    #[test]
    fn arity_maps_to_caller() {
        let src = "fn add(a: i32) -> i32 { return a }\nfn main() -> i32 { return add(1, 2) }\n";
        let d = diag("E-ARITY", "`main` calls `add` with 2 args, want 1");
        assert_eq!(
            plan_scope(&[d], src, false),
            RepairScope::Functions(vec!["main".into()])
        );
    }

    #[test]
    fn effect_maps_to_caller() {
        let src = "fn f() -> i32 throws { return 1 }\nfn g() -> i32 { return f() }\n";
        let d = diag(
            "E-EFFECT-MISMATCH",
            "`g` calls throwing `f` without `throws`",
        );
        assert_eq!(
            plan_scope(&[d], src, false),
            RepairScope::Functions(vec!["g".into()])
        );
    }

    #[test]
    fn cancel_maps_to_task_group_owner() {
        let src = "fn fetch_a() -> i32 throws { return 1 }\nfn bad() -> i32 throws async { task_group { let a = spawn fetch_a() return await b } }\n";
        let d = diag("E-TASK-CANCEL", "child task may outlive its task group");
        assert_eq!(
            plan_scope(&[d], src, false),
            RepairScope::Functions(vec!["bad".into()])
        );
    }

    #[test]
    fn too_many_functions_falls_back_to_file() {
        let src =
            "fn a() -> i32 { return x }\nfn b() -> i32 { return y }\nfn c() -> i32 { return z }\n";
        let ds = vec![
            diag("E-UNDEFINED", "undefined variable `x`"),
            diag("E-UNDEFINED", "undefined variable `y`"),
            diag("E-UNDEFINED", "undefined variable `z`"),
        ];
        assert_eq!(plan_scope(&ds, src, false), RepairScope::File);
    }

    #[test]
    fn force_file() {
        let src = "fn main() -> i32 { return x }\n";
        let d = diag("E-UNDEFINED", "undefined variable `x`");
        assert_eq!(plan_scope(&[d], src, true), RepairScope::File);
    }
}
