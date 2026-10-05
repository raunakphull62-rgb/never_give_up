//! Klang value utilities: `ord` / `chr` / `slice` / `sort` (SYS-DATA-1).
//!
//! Flat builtins like the rest (`hir::is_builtin`,
//! `runtime::exec_builtin`): `ord(s)`, `chr(n)`, `slice(v, lo, hi)`,
//! `sort(list)`. All bounds/type failures are loud diagnostics, never
//! silent clamps — matching the `s[i]` / `insert` conventions.
//!
//! `sort` accepts homogeneous lists only (all ints, all floats, or all
//! strings); mixed lists are `E-TYPE`. Floats order by `total_cmp`
//! (deterministic with NaN, no failure).

use crate::diagnostics::Diagnostic;

/// `E-CHAR-INVALID`: `ord` of an empty string, or `chr` of a value
/// that is not a Unicode scalar (negative, surrogate, or above
/// `U+10FFFF`).
pub(crate) fn char_invalid(op: &str, detail: &str) -> Diagnostic {
    Diagnostic::error(
        "E-CHAR-INVALID",
        &format!("{op} failed: {detail}"),
        "runtime",
        0,
        0,
        "ord needs a non-empty string; chr needs 0..0x10FFFF minus surrogates",
        &["check the character or codepoint first"],
        "char/invalid",
    )
}

/// Unicode codepoint of the first character of `s`. Empty strings are
/// `E-CHAR-INVALID` (there is no codepoint to return).
pub fn ord(s: &str) -> Result<u32, Diagnostic> {
    s.chars()
        .next()
        .map(|c| c as u32)
        .ok_or_else(|| char_invalid("ord", "ord() of empty string"))
}

/// Single-character string for Unicode scalar `n`. Surrogates
/// (`0xD800..0xDFFF`), negatives, and values above `0x10FFFF` are
/// `E-CHAR-INVALID`.
pub fn chr(n: i64) -> Result<String, Diagnostic> {
    let cp = u32::try_from(n)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| {
            char_invalid("chr", &format!("{n} is not a Unicode scalar value"))
        })?;
    Ok(cp.to_string())
}

/// Character-based slice of `s` over `[start, end)`: negative bounds
/// and `end < start` are `E-RUNTIME` (same loudness as `s[i]`), never
/// a silent clamp. Byte messages mirror the indexing errors.
pub fn slice_str(s: &str, start: i64, end: i64) -> Result<String, Diagnostic> {
    if start < 0 || end < 0 {
        return Err(crate::runtime::runtime_err("negative index"));
    }
    if end < start {
        return Err(crate::runtime::runtime_err(
            "slice() end before start",
        ));
    }
    let chars: Vec<char> = s.chars().collect();
    if (end as usize) > chars.len() {
        return Err(crate::runtime::runtime_err(
            "string slice index out of bounds",
        ));
    }
    Ok(chars[start as usize..end as usize].iter().collect())
}

/// Sorted copy of `list`. Homogeneous ints, floats (by `total_cmp`),
/// or strings sort; anything else — including mixed-type lists — is
/// `E-TYPE`.
pub fn sort_list(items: &[crate::runtime::Value]) -> Result<Vec<crate::runtime::Value>, Diagnostic> {
    use crate::runtime::Value;
    if items.iter().all(|v| matches!(v, Value::Int(_))) {
        let mut out: Vec<i64> = items
            .iter()
            .map(|v| match v {
                Value::Int(n) => *n,
                _ => 0,
            })
            .collect();
        out.sort();
        return Ok(out.into_iter().map(Value::Int).collect());
    }
    if items.iter().all(|v| matches!(v, Value::Float(_))) {
        let mut out: Vec<f64> = items
            .iter()
            .map(|v| match v {
                Value::Float(n) => *n,
                _ => 0.0,
            })
            .collect();
        out.sort_by(|a, b| a.total_cmp(b));
        return Ok(out.into_iter().map(Value::Float).collect());
    }
    if items.iter().all(|v| matches!(v, Value::Str(_))) {
        let mut out: Vec<String> = items
            .iter()
            .map(|v| match v {
                Value::Str(s) => s.clone(),
                _ => String::new(),
            })
            .collect();
        out.sort();
        return Ok(out.into_iter().map(Value::Str).collect());
    }
    Err(Diagnostic::error(
        "E-TYPE",
        "sort() needs a homogeneous list (all ints, all floats, or all strings)",
        "runtime",
        0,
        0,
        "mixed-type lists have no total order; convert explicitly first",
        &["convert elements with int()/float()/str() first"],
        "calls/sort",
    ))
}
