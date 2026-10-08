//! Minimal canonical formatter: AST -> stable source text.
//!
//! Rules: 4-space indent, one statement per line, binary ops fully
//! parenthesized (always correct, idempotent: re-parse drops parens,
//! re-emit restores them). `fmt(fmt(x)) == fmt(x)`.

use crate::ast::{
    AssignTarget, Block, EnumDecl, Expr, FunctionDecl, ModDecl, Program, Stmt, StructDecl,
};

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
