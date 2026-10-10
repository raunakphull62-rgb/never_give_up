//! Minimal canonical formatter: AST -> stable source text.
//!
//! Rules: 4-space indent, one statement per line, binary ops fully
//! parenthesized (always correct, idempotent: re-parse drops parens,
//! re-emit restores them). `fmt(fmt(x)) == fmt(x)`.
//!
//! Comments (S3a): the lexer keeps `//...` and `/* ... */` as trivia
//! ([`crate::parser::Comment`]) and [`fmt_source`] re-attaches every
//! comment to the canonical output, so `fmt` never deletes comments.
//! Canonical comment rules (see `docs/progress/S3-notes.md`):
//! 4-space-consistent standalone indent, trailing comments moved to end
//! of line with a single space, line comments stripped of trailing
//! whitespace/`\r`, block bodies verbatim, exactly one trailing newline.

use crate::ast::{
    AssignTarget, Block, EnumDecl, Expr, FunctionDecl, ModDecl, Program, Stmt, StructDecl,
};
use crate::diagnostics::Diagnostic;
use crate::parser::{lex_with_comments, Comment, Parser, Token, TokenKind};

fn indent(n: usize) -> String {
    "    ".repeat(n)
}

fn esc_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

pub fn fmt_program(p: &Program) -> String {
    let mut out = String::new();
    for imp in &p.imports {
        out.push_str(&format!("import \"{imp}\"\n"));
    }
    for aliased in &p.aliased_imports {
        out.push_str(&format!(
            "import \"{}\" as {}\n",
            aliased.path, aliased.alias
        ));
    }
    for sel in &p.selective_imports {
        out.push_str(&format!(
            "import {{ {} }} from \"{}\"\n",
            sel.names.join(", "),
            sel.path
        ));
    }
    if !p.imports.is_empty()
        || !p.selective_imports.is_empty()
        || !p.aliased_imports.is_empty()
    {
        out.push('\n');
    }
    for s in &p.structs {
        out.push_str(&fmt_struct(s));
        out.push('\n');
    }
    for e in &p.enums {
        out.push_str(&fmt_enum(e));
        out.push('\n');
    }
    for m in &p.mods {
        out.push_str(&fmt_mod(m));
        out.push('\n');
    }
    if (!p.structs.is_empty() || !p.enums.is_empty() || !p.mods.is_empty())
        && !p.functions.is_empty()
    {
        out.push('\n');
    }
    for (i, f) in p.functions.iter().enumerate() {
        out.push_str(&fmt_function(f));
        if i + 1 < p.functions.len() {
            out.push('\n');
        }
    }
    out
}

fn fmt_tparams(tps: &[String]) -> String {
    if tps.is_empty() {
        String::new()
    } else {
        format!("<{}>", tps.join(", "))
    }
}

fn fmt_pub(is_pub: bool) -> &'static str {
    if is_pub {
        "pub "
    } else {
        ""
    }
}

fn fmt_mod(m: &ModDecl) -> String {
    let mut out = format!("mod {} {{\n", m.name);
    for s in &m.structs {
        out.push_str(&indent(1));
        out.push_str(&fmt_struct(s));
        out.push('\n');
    }
    for e in &m.enums {
        out.push_str(&indent(1));
        out.push_str(&fmt_enum(e));
        out.push('\n');
    }
    for f in &m.functions {
        for line in fmt_function(f).lines() {
            out.push_str(&indent(1));
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str("}\n");
    out
}

fn fmt_struct(s: &StructDecl) -> String {
    let fields: Vec<String> = s
        .fields
        .iter()
        .map(|f| format!("{}: {}", f.name, f.ty))
        .collect();
    format!(
        "{}struct {}{} {{ {} }}",
        fmt_pub(s.is_pub),
        s.name,
        fmt_tparams(&s.type_params),
        fields.join(", ")
    )
}

fn fmt_enum(e: &EnumDecl) -> String {
    let variants: Vec<String> = e
        .variants
        .iter()
        .map(|v| {
            if v.fields.is_empty() {
                v.name.clone()
            } else {
                // Tuple payloads (`V(i32, str)`, synthesized `f0..fN`
                // names) print bare; named fields keep `name: Type`.
                // Payload names are inert (binding is positional), so
                // normalizing an explicit `V(f0: i32)` to `V(i32)` is
                // behavior-preserving and keeps `fmt(fmt(x)) == fmt(x)`.
                let tuple = v.fields.iter().enumerate().all(|(i, f)| f.name == format!("f{i}"));
                let fs: Vec<String> = if tuple {
                    v.fields.iter().map(|f| f.ty.clone()).collect()
                } else {
                    v.fields
                        .iter()
                        .map(|f| format!("{}: {}", f.name, f.ty))
                        .collect()
                };
                format!("{}({})", v.name, fs.join(", "))
            }
        })
        .collect();
    format!(
        "{}enum {}{} {{ {} }}",
        fmt_pub(e.is_pub),
        e.name,
        fmt_tparams(&e.type_params),
        variants.join(", ")
    )
}

fn fmt_function(f: &FunctionDecl) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.ty))
        .collect();
    let mut effects = String::new();
    for e in &f.effects {
        effects.push(' ');
        effects.push_str(e.name());
    }
    format!(
        "{}fn {}{}({}) -> {}{} {{\n{}}}\n",
        fmt_pub(f.is_pub),
        f.name,
        fmt_tparams(&f.type_params),
        params.join(", "),
        f.return_ty,
        effects,
        fmt_block(&f.body, 1)
    )
}

fn fmt_block(b: &Block, depth: usize) -> String {
    let mut out = String::new();
    for s in &b.stmts {
        out.push_str(&indent(depth));
        out.push_str(&fmt_stmt(s, depth));
        out.push('\n');
    }
    // Closing `}` is emitted by the caller (`{{...{}}}`); leave its indent.
    out.push_str(&indent(depth - 1));
    out
}

fn fmt_stmt(s: &Stmt, depth: usize) -> String {
    match s {
        Stmt::Let(l) => format!("let {} = {}", l.name, fmt_expr(&l.value)),
        Stmt::Assign(a) => format!("{} = {}", fmt_target(&a.target), fmt_expr(&a.value)),
        Stmt::Return(r) => format!("return {}", fmt_expr(&r.value)),
        Stmt::TaskGroup(g) => format!("task_group {{\n{}}}", fmt_block(&g.body, depth + 1)),
        Stmt::If(i) => {
            let mut out = format!(
                "if {} {{\n{}}}",
                fmt_expr(&i.cond),
                fmt_block(&i.then_block, depth + 1)
            );
            if let Some(e) = &i.else_block {
                // `else if` chains re-emit flat when the else block holds one If.
                if e.stmts.len() == 1 {
                    if let Stmt::If(inner) = &e.stmts[0] {
                        out.push_str(&format!(
                            " else {}",
                            fmt_stmt(&Stmt::If(inner.clone()), depth)
                        ));
                        return out;
                    }
                }
                out.push_str(&format!(" else {{\n{}}}", fmt_block(e, depth + 1)));
            }
            out
        }
        Stmt::Print(p) => format!("print({})", fmt_expr(&p.value)),
        Stmt::While(w) => format!(
            "while {} {{\n{}}}",
            fmt_expr(&w.cond),
            fmt_block(&w.body, depth + 1)
        ),
        Stmt::ForRange(fr) => format!(
            "for {} in {}..{} {{\n{}}}",
            fr.var,
            fmt_expr(&fr.start),
            fmt_expr(&fr.end),
            fmt_block(&fr.body, depth + 1)
        ),
        Stmt::ForIn(fi) => format!(
            "for {} in {} {{\n{}}}",
            fi.var,
            fmt_expr(&fi.iter),
            fmt_block(&fi.body, depth + 1)
        ),
        Stmt::Break(_) => "break".to_string(),
        Stmt::Continue(_) => "continue".to_string(),
        Stmt::TryCatch(t) => format!(
            "try {{\n{}}} catch {} {{\n{}}}",
            fmt_block(&t.body, depth + 1),
            t.var,
            fmt_block(&t.handler, depth + 1)
        ),
        Stmt::Expr(e) => fmt_expr(e),
    }
}

fn fmt_target(t: &AssignTarget) -> String {
    match t {
        AssignTarget::Var { name } => name.clone(),
        AssignTarget::Index { base, index } => {
            format!("{}[{}]", fmt_expr(base), fmt_expr(index))
        }
        AssignTarget::Field { base, field } => format!("{}.{field}", fmt_expr(base)),
    }
}

fn fmt_expr(e: &Expr) -> String {
    match e {
        Expr::Int { value, .. } => value.to_string(),
        Expr::Float { bits, .. } => {
            let v = f64::from_bits(*bits);
            if v.fract() == 0.0 {
                format!("{v:.1}")
            } else {
                v.to_string()
            }
        }
        Expr::Str { value, .. } => format!("\"{}\"", esc_str(value)),
        Expr::Bool { value, .. } => value.to_string(),
        Expr::ArrayLit { elems, .. } => {
            format!(
                "[{}]",
                elems.iter().map(fmt_expr).collect::<Vec<_>>().join(", ")
            )
        }
        Expr::MapLit { entries, .. } => {
            let fs: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("\"{k}\": {}", fmt_expr(v)))
                .collect();
            format!("{{{}}}", fs.join(", "))
        }
        Expr::StructLit { name, fields, .. } => {
            let fs: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", fmt_expr(v)))
                .collect();
            format!("{name} {{ {} }}", fs.join(", "))
        }
        Expr::EnumCtor {
            enum_name,
            variant,
            args,
            type_args,
            ..
        } => {
            // BUGHUNT-2: render explicit `m::f<T>(args)` round-trip; plain
            // ctors (empty type_args) render exactly as before.
            let targs = if type_args.is_empty() {
                String::new()
            } else {
                format!("<{}>", type_args.join(", "))
            };
            if args.is_empty() {
                format!("{enum_name}::{variant}{targs}")
            } else {
                format!(
                    "{enum_name}::{variant}{targs}({})",
                    args.iter().map(fmt_expr).collect::<Vec<_>>().join(", ")
                )
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            let as_: Vec<String> = arms
                .iter()
                .map(|a| {
                    let pat = match (&a.enum_name, &a.variant) {
                        (Some(en), Some(v)) => {
                            if a.bindings.is_empty() {
                                format!("{en}::{v}")
                            } else {
                                let bs: Vec<String> =
                                    a.bindings.iter().map(fmt_binding).collect();
                                format!("{en}::{v}({})", bs.join(", "))
                            }
                        }
                        _ => "_".to_string(),
                    };
                    let head = match &a.guard {
                        Some(g) => format!("{pat} if {}", fmt_expr(g)),
                        None => pat,
                    };
                    if a.stmts.is_empty() {
                        format!("{head} => {}", fmt_expr(&a.body))
                    } else {
                        // Multi-statement arm: statements in order, trailing
                        // expression last. Newline-separated (the formatter's
                        // one-statement-per-line rule); re-parses to the
                        // same arm, so `fmt(fmt(x)) == fmt(x)` holds.
                        let mut lines: Vec<String> =
                            a.stmts.iter().map(|s| fmt_stmt(s, 2)).collect();
                        lines.push(fmt_expr(&a.body));
                        format!("{head} => {{\n{}\n}}", lines.join("\n"))
                    }
                })
                .collect();
            format!("match {} {{ {} }}", fmt_expr(scrutinee), as_.join(", "))
        }
        Expr::Index { base, index, .. } => {
            format!("{}[{}]", fmt_expr(base), fmt_expr(index))
        }
        Expr::Field { base, field, .. } => format!("{}.{field}", fmt_expr(base)),
        Expr::MethodCall {
            base, method, args, ..
        } => {
            format!(
                "{}.{method}({})",
                fmt_expr(base),
                args.iter().map(fmt_expr).collect::<Vec<_>>().join(", ")
            )
        }
        Expr::Spawn { call, .. } => format!("spawn {}", fmt_expr(call)),
        Expr::Await { name, .. } => format!("await {name}"),
        Expr::Closure {
            params, return_ty, body, ..
        } => {
            // Anonymous function value. The body is statement-only (the
            // value comes from `return` or fall-off-the-end `last`); fixed
            // depth keeps `fmt(fmt(x)) == fmt(x)` (re-parse drops
            // whitespace, so any fixed depth is idempotent).
            let ps: Vec<String> = params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.ty))
                .collect();
            format!(
                "fn({}) -> {} {{\n{}}}",
                ps.join(", "),
                return_ty,
                fmt_block(body, 1)
            )
        }
        Expr::Call {
            func,
            type_args,
            args,
            ..
        } => {
            let targs = if type_args.is_empty() {
                String::new()
            } else {
                format!("<{}>", type_args.join(", "))
            };
            format!(
                "{func}{targs}({})",
                args.iter().map(fmt_expr).collect::<Vec<_>>().join(", ")
            )
        }
        Expr::Var { name, .. } => name.clone(),
        Expr::Add { left, right, .. } => bin(left, right, "+"),
        Expr::Sub { left, right, .. } => bin(left, right, "-"),
        Expr::Mul { left, right, .. } => bin(left, right, "*"),
        Expr::Div { left, right, .. } => bin(left, right, "/"),
        Expr::Mod { left, right, .. } => bin(left, right, "%"),
        Expr::Eq { left, right, .. } => bin(left, right, "=="),
        Expr::NotEq { left, right, .. } => bin(left, right, "!="),
        Expr::Lt { left, right, .. } => bin(left, right, "<"),
        Expr::LtEq { left, right, .. } => bin(left, right, "<="),
        Expr::Gt { left, right, .. } => bin(left, right, ">"),
        Expr::GtEq { left, right, .. } => bin(left, right, ">="),
        Expr::And { left, right, .. } => bin(left, right, "&&"),
        Expr::Or { left, right, .. } => bin(left, right, "||"),
        Expr::Not { inner, .. } => format!("!{}", fmt_expr(inner)),
        Expr::Neg { inner, .. } => format!("-{}", fmt_expr(inner)),
    }
}

fn fmt_binding(b: &crate::ast::MatchBinding) -> String {
    match b {
        crate::ast::MatchBinding::Ignore => "_".to_string(),
        crate::ast::MatchBinding::Bind(n) => n.clone(),
        crate::ast::MatchBinding::Nested {
            enum_name,
            variant,
            bindings,
        } => {
            if bindings.is_empty() {
                format!("{enum_name}::{variant}")
            } else {
                let bs: Vec<String> = bindings.iter().map(fmt_binding).collect();
                format!("{enum_name}::{variant}({})", bs.join(", "))
            }
        }
    }
}

fn bin(l: &Expr, r: &Expr, op: &str) -> String {
    format!("({} {op} {})", fmt_expr(l), fmt_expr(r))
}

// ---------------------------------------------------------------------------
// S3a: comment-preserving entry point + trivia re-attachment
// ---------------------------------------------------------------------------

/// Format `source` canonically while preserving every comment.
///
/// Parse errors (including unterminated strings/comments) return the
/// diagnostic and format nothing, so callers can leave the file untouched.
/// Comment-free sources return exactly [`fmt_program`] output.
pub fn fmt_source(source: &str, file: &str) -> Result<String, Diagnostic> {
    let (otokens, comments, lex_err) = lex_with_comments(source, file);
    if let Some(e) = lex_err {
        return Err(e);
    }
    let mut p = Parser::new_with_file(source, file);
    let prog = p.parse_program()?;
    let base = fmt_program(&prog);
    if comments.is_empty() {
        return Ok(base);
    }
    let (ftokens, _, flex_err) = lex_with_comments(&base, file);
    if flex_err.is_some() {
        return Ok(base);
    }
    Ok(merge_comments(source, &comments, &otokens, &base, &ftokens))
}

/// Segmented token mapping between the original stream `o` and the
/// formatted stream `f` (both without the trailing `Eof`): `align[i]` is
/// the formatted index of `o[i]` (if any).
///
/// `fmt_program` groups top-level items by kind (imports, structs, enums,
/// mods, then functions — and the same grouping for members inside `mod`),
/// so the two streams agree on the *multiset* of items but not their order.
/// Mapping splits both spans into items, matches by key (order-insensitive),
/// and maps each pair greedily (order-preserving within one item, where
/// `fmt` only drops `;`, adds `(`/`)` and `-> void`, and collapses `fK: T`
/// names — each with a confirmed lookahead). Linear time; unmatchable
/// tokens stay `None` (their comments fall back to file end, in order).
fn align_tokens(o: &[TokenKind], f: &[TokenKind]) -> Vec<Option<usize>> {
    let mut map = vec![None; o.len()];
    map_span(o, 0, o.len(), f, 0, f.len(), &mut map);
    map
}

fn greedy_map(
    o_full: &[TokenKind],
    o_from: usize,
    o_to: usize,
    f_full: &[TokenKind],
    f_from: usize,
    f_to: usize,
    map: &mut [Option<usize>],
) {
    let n = o_to.saturating_sub(o_from);
    let m = f_to.saturating_sub(f_from);
    let o = &o_full[o_from..o_to.min(o_full.len())];
    let f = &f_full[f_from..f_to.min(f_full.len())];
    let n = n.min(o.len());
    let m = m.min(f.len());
    let mut i = 0usize;
    let mut j = 0usize;
    while i < n && j < m {
        if o[i] == f[j] {
            map[o_from + i] = Some(f_from + j);
            i += 1;
            j += 1;
            continue;
        }
        if o[i] == TokenKind::Semi {
            i += 1;
            continue;
        }
        if f[j] == TokenKind::LParen || f[j] == TokenKind::RParen {
            j += 1;
            continue;
        }
        if f[j] == TokenKind::Arrow
            && matches!(f.get(j + 1), Some(TokenKind::Ident(s)) if s == "void")
            && f.get(j + 2) == o.get(i)
        {
            j += 2;
            continue;
        }
        if matches!(&o[i], TokenKind::Ident(_))
            && o.get(i + 1) == Some(&TokenKind::Colon)
            && o.get(i + 2) == f.get(j)
        {
            i += 2;
            continue;
        }
        // fmt drops `()` on empty-arg `m::f()` / `E::V()` constructions
        // (both spellings parse to the same `EnumCtor` with no args, so
        // this is a spelling normalization, not a meaning change): skip an
        // O empty pair confirmed by the following token.
        if o[i] == TokenKind::LParen
            && o.get(i + 1) == Some(&TokenKind::RParen)
            && o.get(i + 2) == f.get(j)
        {
            i += 2;
            continue;
        }
        let mut advanced = false;
        for k in 1..=8 {
            if f.get(j + k) == o.get(i) {
                j += k;
                advanced = true;
                break;
            }
        }
        if advanced {
            continue;
        }
        for k in 1..=8 {
            if o.get(i + k) == Some(&f[j]) {
                i += k;
                advanced = true;
                break;
            }
        }
        if advanced {
            continue;
        }
        i += 1;
        j += 1;
    }
}

/// One item (top-level, or member inside `mod`): token span `[start, end)`.
#[derive(Debug)]
struct ItemSeg {
    key: String,
    start: usize,
    end: usize,
}

fn item_opener(k: &TokenKind) -> bool {
    matches!(
        k,
        TokenKind::Import
            | TokenKind::Mod
            | TokenKind::Enum
            | TokenKind::Struct
            | TokenKind::Fn
            | TokenKind::Pub
    )
}

fn item_key(full: &[TokenKind], start: usize, end: usize) -> String {
    let mut tag = String::new();
    let mut idx = start;
    if full.get(idx) == Some(&TokenKind::Pub) {
        tag.push_str("pub ");
        idx += 1;
    }
    let kw = match full.get(idx) {
        Some(TokenKind::Import) => "import",
        Some(TokenKind::Mod) => "mod",
        Some(TokenKind::Enum) => "enum",
        Some(TokenKind::Struct) => "struct",
        Some(TokenKind::Fn) => "fn",
        _ => "other",
    };
    tag.push_str(kw);
    tag.push(':');
    idx += 1;
    // First identifier or string after the keyword (fn/struct/enum/mod
    // name, import path or first selective name). Collisions fall back to
    // k-th-occurrence matching, a bijection over the same multiset.
    let mut p = idx;
    while p < end.min(full.len()) {
        match &full[p] {
            TokenKind::Ident(s) | TokenKind::StrLit(s) => {
                tag.push_str(s);
                break;
            }
            _ => {}
        }
        p += 1;
    }
    tag
}

/// Split `full[from..to]` into items at brace depth 0 (absolute indices).
/// Every token of a successfully parsed stream belongs to exactly one item
/// (there are no top-level expressions, and closures sit at depth >= 1).
fn split_items(full: &[TokenKind], from: usize, to: usize) -> Vec<ItemSeg> {
    let mut segs = Vec::new();
    let mut depth = 0i32;
    let mut cur: Option<usize> = None;
    let mut i = from;
    while i < to.min(full.len()) {
        if depth == 0 && item_opener(&full[i]) {
            if let Some(s) = cur {
                segs.push(ItemSeg { key: item_key(full, s, i), start: s, end: i });
            }
            cur = Some(i);
        }
        match &full[i] {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if let Some(s) = cur {
        segs.push(ItemSeg {
            key: item_key(full, s, to),
            start: s,
            end: to,
        });
    }
    segs
}

/// Match `o_segs` to `f_segs` by key (k-th occurrence among collisions).
/// Returns, per O segment, the matched F segment index (if any).
fn match_items(o_segs: &[ItemSeg], f_segs: &[ItemSeg]) -> Vec<Option<usize>> {
    use std::collections::{HashMap, VecDeque};
    let mut by_key: HashMap<&str, VecDeque<usize>> = HashMap::new();
    for (idx, s) in f_segs.iter().enumerate() {
        by_key.entry(s.key.as_str()).or_default().push_back(idx);
    }
    o_segs
        .iter()
        .map(|s| by_key.get_mut(s.key.as_str()).and_then(|q| q.pop_front()))
        .collect()
}

fn seg_is_mod(kinds: &[TokenKind], seg: &ItemSeg) -> bool {
    let mut idx = seg.start;
    if kinds.get(idx) == Some(&TokenKind::Pub) {
        idx += 1;
    }
    kinds.get(idx) == Some(&TokenKind::Mod)
}

/// First `{` at span depth 0 and its match. `None` on imbalance (only on
/// already-rejected input; callers fall back to whole-span greedy).
fn brace_span(kinds: &[TokenKind], start: usize, end: usize) -> Option<(usize, usize)> {
    let mut depth = 0i32;
    let mut open: Option<usize> = None;
    let mut i = start;
    while i < end {
        match &kinds[i] {
            TokenKind::LBrace => {
                if depth == 0 && open.is_none() {
                    open = Some(i);
                }
                depth += 1;
            }
            TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    if let Some(o) = open {
                        return Some((o, i));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Map one matched item pair. Mod pairs recurse into members (same
/// kind-grouping one level down); every other pair maps greedily.
/// All indices are absolute into the full streams.
fn map_pair(
    o_full: &[TokenKind],
    o_seg: &ItemSeg,
    f_full: &[TokenKind],
    f_seg: &ItemSeg,
    map: &mut [Option<usize>],
) {
    if seg_is_mod(o_full, o_seg) && seg_is_mod(f_full, f_seg) {
        if let (Some((oo, oc)), Some((fo, fc))) = (
            brace_span(o_full, o_seg.start, o_seg.end),
            brace_span(f_full, f_seg.start, f_seg.end),
        ) {
            greedy_map(o_full, o_seg.start, oo + 1, f_full, f_seg.start, fo + 1, map);
            map_span(o_full, oo + 1, oc, f_full, fo + 1, fc, map);
            greedy_map(o_full, oc, o_seg.end, f_full, fc, f_seg.end, map);
            return;
        }
    }
    greedy_map(
        o_full,
        o_seg.start,
        o_seg.end,
        f_full,
        f_seg.start,
        f_seg.end,
        map,
    );
}

/// Segmented mapping over a span pair: split into items, match by key,
/// map each pair. All indices absolute. Unmatchable O tokens stay `None`
/// (their comments fall back to file end, in order).
fn map_span(
    o_full: &[TokenKind],
    o_from: usize,
    o_to: usize,
    f_full: &[TokenKind],
    f_from: usize,
    f_to: usize,
    map: &mut [Option<usize>],
) {
    let o_segs = split_items(o_full, o_from, o_to);
    let f_segs = split_items(f_full, f_from, f_to);
    if o_segs.is_empty() || f_segs.is_empty() {
        greedy_map(o_full, o_from, o_to, f_full, f_from, f_to, map);
        return;
    }
    let matched = match_items(&o_segs, &f_segs);
    for (o_seg, f_opt) in o_segs.iter().zip(matched.iter()) {
        if let Some(fi) = f_opt {
            map_pair(o_full, o_seg, f_full, &f_segs[*fi], map);
        }
    }
}

fn line_indent_of(line: &str) -> &str {
    let n = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..n]
}

/// Re-attach `comments` (source order, byte offsets into `source`) to the
/// canonical `base` text whose token stream is `ftokens` (`otokens` is the
/// original stream, same order as `comments`).
///
/// Each comment maps through [`align_tokens`] to the formatted gap after
/// its nearest aligned neighbors: a comment with code before it on its
/// original line is *trailing* (appended to the end of its anchor code
/// line with one space); otherwise it is *standalone* (its own line(s)
/// before the anchor line at the deeper of the two neighboring indents,
/// block bodies verbatim). Unmappable comments fall back to file
/// top/end in order — always preserved, never dropped. The mapping is a
/// fixpoint: formatting the output again extracts the same comments at
/// the same anchors and emits byte-identical text.
fn merge_comments(
    source: &str,
    comments: &[Comment],
    otokens: &[Token],
    base: &str,
    ftokens: &[Token],
) -> String {
    let o: Vec<TokenKind> = otokens
        .iter()
        .filter(|t| t.kind != TokenKind::Eof)
        .map(|t| t.kind.clone())
        .collect();
    let f: Vec<TokenKind> = ftokens
        .iter()
        .filter(|t| t.kind != TokenKind::Eof)
        .map(|t| t.kind.clone())
        .collect();
    let o_starts: Vec<usize> = otokens
        .iter()
        .filter(|t| t.kind != TokenKind::Eof)
        .map(|t| t.start)
        .collect();
    let mut align: Vec<Option<usize>> = vec![None; o.len()];
    map_span(&o, 0, o.len(), &f, 0, f.len(), &mut align);
    let m = f.len();
    // Program-level segments for segment-aware anchors (an O comment maps
    // through its own item's F match, never across a reordered item).
    let o_segs = split_items(&o, 0, o.len());
    let f_segs = split_items(&f, 0, f.len());
    let o_match = match_items(&o_segs, &f_segs);

    // Base lines + byte offset of each line start (base has no comments).
    let mut lines: Vec<String> = base.split('\n').map(|s| s.to_string()).collect();
    if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    let mut starts: Vec<usize> = Vec::with_capacity(lines.len());
    let mut off = 0usize;
    for (idx, l) in lines.iter().enumerate() {
        if idx == 0 {
            starts.push(0);
        } else {
            off += lines[idx - 1].len() + 1;
            starts.push(off);
        }
        let _ = l.len();
    }
    let line_of = |pos: usize| -> usize {
        match starts.binary_search(&pos) {
            Ok(k) => k,
            Err(k) => k.saturating_sub(1).min(lines.len().saturating_sub(1)),
        }
    };
    // Formatted token -> its base line (tokens never span lines).
    let f_line: Vec<usize> = ftokens
        .iter()
        .filter(|t| t.kind != TokenKind::Eof)
        .map(|t| if lines.is_empty() { 0 } else { line_of(t.start.min(base.len())) })
        .collect();

    let mut inserts: Vec<Vec<String>> = vec![Vec::new(); lines.len() + 1];
    let mut appends: Vec<Vec<String>> = vec![Vec::new(); lines.len() + 1];

    for c in comments {
        let mut raw = c.text(source).to_string();
        let is_line = raw.starts_with("//");
        if is_line {
            raw = raw.trim_end_matches(['\r', ' ', '\t']).to_string();
        }
        // Standalone when only whitespace precedes it on its source line.
        let ls = source[..c.start].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let before = &source[ls..c.start];
        let standalone = before.trim().is_empty();
        // First original token at/after the comment end.
        let mut i = o_starts.partition_point(|&s| s < c.end);
        if i > o.len() {
            i = o.len();
        }
        // Past-the-end comments (footers, or comment-only files) always go
        // to file end: a fixpoint (they are past-the-end again next pass).
        if i >= o.len() {
            if standalone {
                let mut parts: Vec<String> = raw.split('\n').map(|s| s.to_string()).collect();
                inserts[lines.len()].extend(parts.drain(..));
            } else if let Some(pj) = (0..o.len()).rev().find_map(|k| align[k]) {
                let t = *f_line.get(pj).unwrap_or(&0);
                appends[t].push(raw);
            } else {
                inserts[lines.len()].push(raw);
            }
            continue;
        }
        // Owning O item (items partition [0, N)); its F match bounds the
        // forward anchor so reordered items never steal comments.
        let s_idx = o_segs
            .iter()
            .rposition(|s| s.start <= i)
            .unwrap_or(0);
        let f_span: Option<(usize, usize)> = o_match
            .get(s_idx)
            .and_then(|f_opt| f_opt.map(|fi| (f_segs[fi].start, f_segs[fi].end)));
        let s_end = o_segs.get(s_idx).map(|s| s.end).unwrap_or(o.len());
        // Nearest aligned formatted neighbors (forward: within the item).
        let mut jj: Option<usize> = None;
        for k in i..s_end.min(o.len()) {
            if let Some(a) = align[k] {
                jj = Some(a);
                break;
            }
        }
        let mut pp: Option<usize> = None;
        for k in (0..i).rev() {
            if let Some(a) = align[k] {
                pp = Some(a);
                break;
            }
        }
        if !standalone {
            if let Some(pj) = pp {
                let t = *f_line.get(pj).unwrap_or(&0);
                appends[t].push(raw);
                continue;
            }
            // No code before it after all: fall through to standalone.
        }
        // Standalone target: forward anchor line, else the matched span's
        // last line (paranoia fallback), else file end.
        let (target, j_line): (usize, Option<usize>) = match jj {
            Some(a) if a < m => (*f_line.get(a).unwrap_or(&0), Some(a)),
            _ => match f_span {
                Some((fs, fe)) if fe > fs && fe - 1 < m => (*f_line.get(fe - 1).unwrap_or(&0), None),
                _ => (lines.len(), None),
            },
        };
        let mut indent = String::new();
        if target < lines.len() {
            if let Some(a) = j_line {
                if let Some(l) = lines.get(*f_line.get(a).unwrap_or(&0)) {
                    indent = line_indent_of(l).to_string();
                }
            } else if let Some(l) = lines.get(target) {
                indent = line_indent_of(l).to_string();
            }
        }
        if let Some(pj) = pp {
            if let Some(l) = lines.get(*f_line.get(pj).unwrap_or(&0)) {
                let other = line_indent_of(l);
                if target < lines.len() && other.len() > indent.len() {
                    indent = other.to_string();
                }
            }
        }
        // First line takes the anchor indent; block continuations stay
        // verbatim (both choices are fixpoints; verbatim keeps ASCII art).
        let mut parts: Vec<String> = raw.split('\n').map(|s| s.to_string()).collect();
        if !parts.is_empty() {
            parts[0] = format!("{indent}{}", parts[0]);
        }
        inserts[target].extend(parts);
    }

    let mut out = String::new();
    for (idx, l) in lines.iter().enumerate() {
        for ins in &inserts[idx] {
            out.push_str(ins);
            out.push('\n');
        }
        out.push_str(l);
        for ap in &appends[idx] {
            out.push(' ');
            out.push_str(ap);
        }
        out.push('\n');
    }
    for ins in &inserts[lines.len()] {
        out.push_str(ins);
        out.push('\n');
    }
    if out.is_empty() {
        return String::new();
    }
    out
}
