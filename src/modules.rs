//! Named modules: `mod name { ... }` namespaces with `pub`/private visibility.
//!
//! Mechanism: [`resolve`] flattens every module into plain top-level items
//! with qualified names (`m::f`) and rewrites all references to match.
//! Anything downstream (HIR tables, MIR function names, runtime dispatch)
//! is already keyed by plain name strings, so qualified names flow through
//! untouched: `a::h` vs `b::h` vs top-level `h` are simply distinct keys and
//! can never collide. Programs without `mod` blocks resolve to an identical
//! clone with no diagnostics, so all pre-module behavior is unchanged.
//!
//! Scoping: bare names inside `mod m` resolve to `m`'s members first, then
//! to top-level items (lexical shadowing). Bare names at top level never see
//! module members. Qualified paths (`m::x`) work from anywhere but honor
//! visibility: referencing a private member from outside its own module is
//! an `E-PRIVATE` diagnostic, never a silent success.

use std::collections::{HashMap, HashSet};

use crate::ast::{AssignTarget, Block, Expr, FunctionDecl, MatchArm, Program, Stmt};
use crate::diagnostics::Diagnostic;
use crate::hir::FILE;

/// What the head `m` of a qualified `m::item` path turned out to be.
enum Qualified {
    /// `m` is a top-level type, not a module: leave it for the existing
    /// enum/struct logic (backcompat wins over module interpretation).
    TopType,
    /// `m` names no module at all: leave it for the existing logic, which
    /// already reports unknown names.
    Unknown,
    /// `m` is a known module but `item` is not a member: reported here with
    /// the precise path (the existing fallback would only name `m`).
    NoSuchItem,
    /// A member of module `m`. The bool is `is_pub`.
    Member {
        kind: MemberKind,
        is_pub: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberKind {
    Fn,
    Struct,
    Enum,
}

fn duplicate(message: &str) -> Diagnostic {
    Diagnostic::error(
        "E-DUPLICATE",
        message,
        FILE,
        0,
        0,
        "two items share one name",
        &["rename one of them"],
        "names/duplicate",
    )
}

fn undefined(name: &str) -> Diagnostic {
    Diagnostic::error(
        "E-UNDEFINED",
        &format!("undefined `{name}`"),
        FILE,
        0,
        0,
        "name is not visible at this path",
        &["check the module name and item spelling"],
        "names/scope",
    )
}

fn not_a_value(path: &str, what: &str, use_instead: &str) -> Diagnostic {
    Diagnostic::error(
        "E-TYPE",
        &format!("`{path}` is {what}, not a value ({use_instead})"),
        FILE,
        0,
        0,
        "only enum variants construct values with `()`; functions need a call",
        &["write the full path"],
        "types/mismatch",
    )
}

/// Membership tables for one module: member name -> visibility.
struct ModInfo {
    fns: HashMap<String, bool>,
    structs: HashMap<String, bool>,
    enums: HashMap<String, bool>,
}

struct Ctx<'a> {
    mods: HashMap<&'a str, ModInfo>,
    top_structs: HashSet<&'a str>,
    top_enums: HashSet<&'a str>,
    diags: Vec<Diagnostic>,
}

impl<'a> Ctx<'a> {
    /// Classify the head `m` of a qualified `m::item` path.
    fn classify(&self, m: &str, item: &str) -> Qualified {
        // A top-level type named `m` keeps its historical meaning: existing
        // `Enum::Variant` programs keep working even if a module shares the
        // head name.
        if self.top_enums.contains(m) || self.top_structs.contains(m) {
            return Qualified::TopType;
        }
        match self.mods.get(m) {
            None => Qualified::Unknown,
            Some(info) => {
                if let Some(pub_) = info.fns.get(item) {
                    Qualified::Member {
                        kind: MemberKind::Fn,
                        is_pub: *pub_,
                    }
                } else if let Some(pub_) = info.structs.get(item) {
                    Qualified::Member {
                        kind: MemberKind::Struct,
                        is_pub: *pub_,
                    }
                } else if let Some(pub_) = info.enums.get(item) {
                    Qualified::Member {
                        kind: MemberKind::Enum,
                        is_pub: *pub_,
                    }
                } else {
                    Qualified::NoSuchItem
                }
            }
        }
    }

    /// Gate a resolved member access: private members are visible only from
    /// inside their own module. `cur` is the module of the use site.
    fn gate(&mut self, module: &str, is_pub: bool, cur: Option<&str>, full: &str) {
        if !is_pub && cur != Some(module) {
            self.diags
                .push(Diagnostic::private_access(FILE, 0, 0, full));
        }
    }
}

/// Split the head off a qualified path: `m::rest` -> (`m`, `rest`).
fn split_head(path: &str) -> Option<(&str, &str)> {
    path.find("::").map(|i| (&path[..i], &path[i + 2..]))
}

/// First `::` segment item: `E` of `m::E`, or `m::E::V`.
fn head_item(rest: &str) -> &str {
    rest.split("::").next().unwrap_or(rest)
}

/// Flatten modules into qualified top-level items, rewriting references and
/// enforcing visibility. Total: never panics; problems come back as
/// diagnostics alongside a best-effort program.
pub fn resolve(program: &Program) -> (Program, Vec<Diagnostic>) {
    if program.mods.is_empty() {
        return (program.clone(), Vec::new());
    }
    let mut ctx = Ctx {
        mods: HashMap::new(),
        top_structs: program.structs.iter().map(|s| s.name.as_str()).collect(),
        top_enums: program.enums.iter().map(|e| e.name.as_str()).collect(),
        diags: Vec::new(),
    };
    for m in &program.mods {
        if ctx.mods.contains_key(m.name.as_str()) {
            ctx.diags
                .push(duplicate(&format!("duplicate module `{}`", m.name)));
            continue;
        }
        let mut info = ModInfo {
            fns: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
        };
        for f in &m.functions {
            if info.fns.contains_key(&f.name) {
                ctx.diags.push(duplicate(&format!(
                    "duplicate function `{}` in module `{}`",
                    f.name, m.name
                )));
            }
            info.fns.insert(f.name.clone(), f.is_pub);
        }
        for s in &m.structs {
            if info.structs.contains_key(&s.name) {
                ctx.diags.push(duplicate(&format!(
                    "duplicate struct `{}` in module `{}`",
                    s.name, m.name
                )));
            }
            info.structs.insert(s.name.clone(), s.is_pub);
        }
        for e in &m.enums {
            if info.enums.contains_key(&e.name) {
                ctx.diags.push(duplicate(&format!(
                    "duplicate enum `{}` in module `{}`",
                    e.name, m.name
                )));
            }
            info.enums.insert(e.name.clone(), e.is_pub);
        }
        ctx.mods.insert(m.name.as_str(), info);
    }

    let mut flat = Program {
        mods: Vec::new(),
        enums: program.enums.clone(),
        structs: program.structs.clone(),
        imports: program.imports.clone(),
        functions: Vec::with_capacity(program.functions.len()),
    };
    // Top-level items: validate qualified references, rewrite nothing.
    for f in &program.functions {
        let mut f = f.clone();
        rewrite_fn(&mut f, None, &mut ctx);
        flat.functions.push(f);
    }
    for s in &program.structs {
        let mut s = s.clone();
        let tps = s.type_params.clone();
        for fld in s.fields.iter_mut() {
            fld.ty = rewrite_ty(&fld.ty, &tps, None, &mut ctx);
        }
        flat.structs.push(s);
    }
    for e in &program.enums {
        let mut e = e.clone();
        let tps = e.type_params.clone();
        for v in e.variants.iter_mut() {
            for fld in v.fields.iter_mut() {
                fld.ty = rewrite_ty(&fld.ty, &tps, None, &mut ctx);
            }
        }
        flat.enums.push(e);
    }
    // Module members: rewrite bare refs to qualified, validate qualified.
    for m in &program.mods {
        let cur = Some(m.name.as_str());
        for s in &m.structs {
            let mut s = s.clone();
            let tps = s.type_params.clone();
            for fld in s.fields.iter_mut() {
                fld.ty = rewrite_ty(&fld.ty, &tps, cur, &mut ctx);
            }
            s.name = format!("{}::{}", m.name, s.name);
            flat.structs.push(s);
        }
        for e in &m.enums {
            let mut e = e.clone();
            let tps = e.type_params.clone();
            for v in e.variants.iter_mut() {
                for fld in v.fields.iter_mut() {
                    fld.ty = rewrite_ty(&fld.ty, &tps, cur, &mut ctx);
                }
            }
            e.name = format!("{}::{}", m.name, e.name);
            flat.enums.push(e);
        }
        for f in &m.functions {
            let mut f = f.clone();
            rewrite_fn(&mut f, cur, &mut ctx);
            f.name = format!("{}::{}", m.name, f.name);
            flat.functions.push(f);
        }
    }
    let diags = std::mem::take(&mut ctx.diags);
    (flat, diags)
}

/// Rewrite one type annotation. Type parameters pass through; qualified
/// paths are validated; bare names gain their module qualifier when they
/// name a member type of the enclosing module.
fn rewrite_ty(ty: &str, type_params: &[String], cur: Option<&str>, ctx: &mut Ctx) -> String {
    if type_params.iter().any(|t| t == ty) {
        return ty.to_string();
    }
    if let Some((m, rest)) = split_head(ty) {
        validate_type_ref(m, rest, cur, ctx, ty);
        return ty.to_string();
    }
    if let Some(cur) = cur {
        if let Some(info) = ctx.mods.get(cur) {
            if info.structs.contains_key(ty) || info.enums.contains_key(ty) {
                return format!("{cur}::{ty}");
            }
        }
    }
    ty.to_string()
}

/// Validate a qualified type reference in an annotation. Annotations stay
/// lenient (unknown names erase to `Unknown`, as they always have): only
/// genuine privacy violations, kind errors, and malformed paths report.
fn validate_type_ref(m: &str, rest: &str, cur: Option<&str>, ctx: &mut Ctx, full: &str) {
    if rest.contains("::") {
        ctx.diags.push(not_a_value(
            full,
            "a qualified path with more segments",
            "use a plain `module::Type` path",
        ));
        return;
    }
    match ctx.classify(m, rest) {
        Qualified::TopType | Qualified::Unknown | Qualified::NoSuchItem => {}
        Qualified::Member { kind, is_pub } => match kind {
            MemberKind::Struct | MemberKind::Enum => {
                ctx.gate(m, is_pub, cur, full);
            }
            MemberKind::Fn => {
                ctx.diags.push(not_a_value(
                    full,
                    &format!("the function `{full}`"),
                    &format!("call it as `{full}(...)`"),
                ));
            }
        },
    }
}

fn rewrite_fn(f: &mut FunctionDecl, cur: Option<&str>, ctx: &mut Ctx) {
    let tps = f.type_params.clone();
    for p in f.params.iter_mut() {
        p.ty = rewrite_ty(&p.ty, &tps, cur, ctx);
    }
    f.return_ty = rewrite_ty(&f.return_ty, &tps, cur, ctx);
    rewrite_block(&mut f.body, cur, ctx);
}

fn rewrite_block(b: &mut Block, cur: Option<&str>, ctx: &mut Ctx) {
    for s in b.stmts.iter_mut() {
        rewrite_stmt(s, cur, ctx);
    }
}

fn rewrite_stmt(s: &mut Stmt, cur: Option<&str>, ctx: &mut Ctx) {
    match s {
        Stmt::Let(l) => rewrite_expr(&mut l.value, cur, ctx),
        Stmt::Assign(a) => {
            match &mut a.target {
                AssignTarget::Var { .. } => {}
                AssignTarget::Index { base, index } => {
                    rewrite_expr(base, cur, ctx);
                    rewrite_expr(index, cur, ctx);
                }
                AssignTarget::Field { base, .. } => rewrite_expr(base, cur, ctx),
            }
            rewrite_expr(&mut a.value, cur, ctx);
        }
        Stmt::Return(r) => rewrite_expr(&mut r.value, cur, ctx),
        Stmt::TaskGroup(g) => rewrite_block(&mut g.body, cur, ctx),
        Stmt::If(i) => {
            rewrite_expr(&mut i.cond, cur, ctx);
            rewrite_block(&mut i.then_block, cur, ctx);
            if let Some(e) = &mut i.else_block {
                rewrite_block(e, cur, ctx);
            }
        }
        Stmt::Print(p) => rewrite_expr(&mut p.value, cur, ctx),
        Stmt::While(w) => {
            rewrite_expr(&mut w.cond, cur, ctx);
            rewrite_block(&mut w.body, cur, ctx);
        }
        Stmt::ForRange(fr) => {
            rewrite_expr(&mut fr.start, cur, ctx);
            rewrite_expr(&mut fr.end, cur, ctx);
            rewrite_block(&mut fr.body, cur, ctx);
        }
        Stmt::ForIn(fi) => {
            rewrite_expr(&mut fi.iter, cur, ctx);
            rewrite_block(&mut fi.body, cur, ctx);
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Expr(e) => rewrite_expr(e, cur, ctx),
    }
}

fn rewrite_expr(e: &mut Expr, cur: Option<&str>, ctx: &mut Ctx) {
    // `A::B` shapes parse as `EnumCtor`; module function calls among them
    // convert to `Call` nodes up front (in-place conversion cannot happen
    // while the main match below holds the borrow, hence the split).
    if matches!(e, Expr::EnumCtor { .. }) {
        rewrite_ctor(e, cur, ctx);
        return;
    }
    match e {
        Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Str { .. }
        | Expr::Bool { .. }
        | Expr::Var { .. }
        | Expr::Await { .. } => {}
        // Already handled above; unreachable here (kept for exhaustiveness).
        Expr::EnumCtor { .. } => {}
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                rewrite_expr(el, cur, ctx);
            }
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                rewrite_expr(v, cur, ctx);
            }
        }
        Expr::StructLit { name, fields, .. } => {
            for (_, v) in fields.iter_mut() {
                rewrite_expr(v, cur, ctx);
            }
            if name.contains("::") {
                validate_struct_lit(name, cur, ctx);
            } else if let Some(cur) = cur {
                if let Some(info) = ctx.mods.get(cur) {
                    if info.structs.contains_key(name) {
                        *name = format!("{cur}::{name}");
                    }
                }
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            rewrite_expr(scrutinee, cur, ctx);
            for arm in arms.iter_mut() {
                rewrite_expr(&mut arm.body, cur, ctx);
                rewrite_arm(arm, cur, ctx);
            }
        }
        Expr::Index { base, index, .. } => {
            rewrite_expr(base, cur, ctx);
            rewrite_expr(index, cur, ctx);
        }
        Expr::Field { base, .. } => rewrite_expr(base, cur, ctx),
        Expr::MethodCall { base, args, .. } => {
            rewrite_expr(base, cur, ctx);
            for a in args.iter_mut() {
                rewrite_expr(a, cur, ctx);
            }
        }
        Expr::Spawn { call, .. } => rewrite_expr(call, cur, ctx),
        Expr::Call { func, args, .. } => {
            for a in args.iter_mut() {
                rewrite_expr(a, cur, ctx);
            }
            if !func.contains("::") {
                if let Some(cur) = cur {
                    let is_local = ctx
                        .mods
                        .get(cur)
                        .map(|info| info.fns.contains_key(func))
                        .unwrap_or(false);
                    if is_local {
                        *func = format!("{cur}::{func}");
                    }
                }
            } else if let Some((m, rest)) = split_head(func) {
                // A qualified call spelled directly (only producible in an
                // already-rewritten program, never from the parser).
                validate_call_ref(m, head_item(rest), cur, ctx, func);
            }
        }
        Expr::Add { left, right, .. }
        | Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. }
        | Expr::Eq { left, right, .. }
        | Expr::NotEq { left, right, .. }
        | Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. }
        | Expr::And { left, right, .. }
        | Expr::Or { left, right, .. } => {
            rewrite_expr(left, cur, ctx);
            rewrite_expr(right, cur, ctx);
        }
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => rewrite_expr(inner, cur, ctx),
    }
}

/// Rewrite one `A::B` / `A::B(args)` / `M::E::V(args)` constructor-shaped
/// node: bare enums gain their module qualifier, module function calls
/// (`m::f(args)`, whose head is the bare module name) become plain
/// qualified `Call` nodes, everything else is validated in place.
fn rewrite_ctor(slot: &mut Expr, cur: Option<&str>, ctx: &mut Ctx) {
    if let Expr::EnumCtor { args, .. } = slot {
        for a in args.iter_mut() {
            rewrite_expr(a, cur, ctx);
        }
    }
    struct Shape {
        path: String,
        head: String,
        item: String,
        three_seg: bool,
    }
    let shape = match slot {
        Expr::EnumCtor { enum_name, variant, .. } => match split_head(enum_name) {
            Some((m, rest)) => Shape {
                path: enum_name.clone(),
                head: m.to_string(),
                item: head_item(rest).to_string(),
                three_seg: true,
            },
            None => Shape {
                path: enum_name.clone(),
                head: enum_name.clone(),
                item: variant.clone(),
                three_seg: false,
            },
        },
        _ => return,
    };
    if shape.three_seg {
        // `m::E::V`: the enum itself is qualified; only validation applies.
        match ctx.classify(&shape.head, &shape.item) {
            Qualified::TopType | Qualified::Unknown => {}
            Qualified::NoSuchItem => {
                ctx.diags.push(undefined(&shape.path));
            }
            Qualified::Member { kind, is_pub } => {
                ctx.gate(&shape.head, is_pub, cur, &shape.path);
                if kind != MemberKind::Enum {
                    ctx.diags.push(not_a_value(
                        &shape.path,
                        "not an enum",
                        "match only on enum variants",
                    ));
                }
            }
        }
        return;
    }
    // Two-segment `E::V`.
    if let Some(cur) = cur {
        if ctx
            .mods
            .get(cur)
            .map(|info| info.enums.contains_key(&shape.head))
            .unwrap_or(false)
        {
            if let Expr::EnumCtor { enum_name, .. } = slot {
                *enum_name = format!("{cur}::{head}", head = shape.head);
            }
            return;
        }
    }
    // A top-level type named `E` keeps its historical enum meaning.
    if ctx.top_enums.contains(shape.head.as_str()) {
        return;
    }
    if !ctx.mods.contains_key(shape.head.as_str()) {
        return;
    }
    let full = format!("{}::{}", shape.head, shape.item);
    match ctx.classify(&shape.head, &shape.item) {
        Qualified::Member {
            kind: MemberKind::Fn,
            is_pub,
        } => {
            ctx.gate(&shape.head, is_pub, cur, &full);
            // Convert regardless of arity: HIR reports `E-ARITY` for a
            // wrong argument count, so the call site stays precise.
            let (oid, otaken) = match slot {
                Expr::EnumCtor { id, args, .. } => (id.clone(), std::mem::take(args)),
                _ => unreachable!("ToCall is only decided from an EnumCtor"),
            };
            *slot = Expr::Call {
                id: oid,
                func: full,
                args: otaken,
            };
        }
        Qualified::Member {
            kind: MemberKind::Enum,
            is_pub,
        } => {
            ctx.gate(&shape.head, is_pub, cur, &full);
            ctx.diags.push(not_a_value(
                &full,
                &format!("the enum `{full}`"),
                &format!("use a variant such as `{full}::Variant`"),
            ));
        }
        Qualified::Member {
            kind: MemberKind::Struct,
            is_pub,
        } => {
            ctx.gate(&shape.head, is_pub, cur, &full);
            ctx.diags.push(not_a_value(
                &full,
                &format!("the struct `{full}`"),
                &format!("construct it as `{full} {{ ... }}`"),
            ));
        }
        Qualified::NoSuchItem => {
            ctx.diags.push(undefined(&full));
        }
        Qualified::TopType | Qualified::Unknown => {}
    }
}

/// Validate one match arm's pattern path, qualifying bare module enums.
fn rewrite_arm(arm: &mut MatchArm, cur: Option<&str>, ctx: &mut Ctx) {
    let Some(n) = arm.enum_name.clone() else {
        return;
    };
    if !n.contains("::") {
        if let Some(cur) = cur {
            let is_local = ctx
                .mods
                .get(cur)
                .map(|info| info.enums.contains_key(&n))
                .unwrap_or(false);
            if is_local {
                arm.enum_name = Some(format!("{cur}::{n}"));
            }
        }
        return;
    }
    let (m, rest) = split_head(&n).expect("contains ::");
    match ctx.classify(m, head_item(rest)) {
        Qualified::TopType | Qualified::Unknown => {}
        Qualified::NoSuchItem => {
            ctx.diags.push(undefined(&n));
        }
        Qualified::Member { kind, is_pub } => {
            ctx.gate(m, is_pub, cur, &n);
            if kind != MemberKind::Enum {
                ctx.diags.push(not_a_value(
                    &n,
                    "a function or struct path",
                    "match only on enum variants",
                ));
            }
        }
    }
}

/// Validate a qualified struct literal `m::T { ... }`.
fn validate_struct_lit(name: &str, cur: Option<&str>, ctx: &mut Ctx) {
    let (m, rest) = split_head(name).expect("contains ::");
    match ctx.classify(m, head_item(rest)) {
        Qualified::TopType | Qualified::Unknown | Qualified::NoSuchItem => {}
        Qualified::Member { kind, is_pub } => {
            ctx.gate(m, is_pub, cur, name);
            if kind != MemberKind::Struct {
                ctx.diags.push(not_a_value(
                    name,
                    "not a struct",
                    "construct the enum variant or call the function instead",
                ));
            }
        }
    }
}

/// Validate a qualified call `m::f(...)` spelled directly (only producible
/// in an already-rewritten program, never from the parser).
fn validate_call_ref(m: &str, item: &str, cur: Option<&str>, ctx: &mut Ctx, full: &str) {
    match ctx.classify(m, item) {
        Qualified::TopType | Qualified::Unknown | Qualified::NoSuchItem => {}
        Qualified::Member { kind, is_pub } => {
            ctx.gate(m, is_pub, cur, full);
            if kind != MemberKind::Fn {
                ctx.diags.push(not_a_value(
                    full,
                    "not a function",
                    "call a function path instead",
                ));
            }
        }
    }
}
