//! Typed HIR: effect + name + type checking over the parsed program.
//!
//! Checks in v0.5 (superset of v0.3):
//! - `spawn` must appear inside a `task_group`
//! - every handle bound by `let x = spawn ...` must be `await`-ed before its
//!   group ends, else E-TASK-CANCEL
//! - a function calling a `throws` callee must itself declare `throws`
//! - call arity must match the callee's params, else E-ARITY
//! - every variable must be a param or a prior `let`, else E-UNDEFINED
//! - `break`/`continue` only inside loops, else E-LOOP
//! - assignment targets must be already-bound names, else E-UNDEFINED
//! - struct literals must name a declared `struct`, else E-UNDEFINED
//! - binary/unary ops, calls, returns and conditions are type-checked,
//!   else E-TYPE (`Unknown` from maps/indexes/method generics is a wildcard)

use std::collections::HashMap;

use crate::ast::{AssignTarget, Block, Effect, Expr, FunctionDecl, Program, Stmt};
use crate::diagnostics::Diagnostic;

pub const FILE: &str = "input.warden";

/// Static type of an expression value.
///
/// `Unknown` is the wildcard: map lookups, dynamic indexes and generic
/// method results coerce to anything without emitting E-TYPE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    Str,
    Bool,
    Array,
    Map,
    Struct(String),
    Void,
    Unknown,
}

impl Ty {
    pub fn name(&self) -> String {
        match self {
            Ty::Int => "i32".to_string(),
            Ty::Float => "f64".to_string(),
            Ty::Str => "str".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Array => "array".to_string(),
            Ty::Map => "map".to_string(),
            Ty::Struct(n) => n.clone(),
            Ty::Void => "void".to_string(),
            Ty::Unknown => "unknown".to_string(),
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float | Ty::Unknown)
    }
}

/// Parse a declared type annotation (`i32`, `f64`, `str`, `bool`, ...).
/// Unknown names become `Unknown` (user struct names resolve via structs map).
pub fn parse_ty(s: &str) -> Ty {
    match s {
        "i32" | "i64" | "u32" | "u64" | "int" => Ty::Int,
        "f32" | "f64" | "float" => Ty::Float,
        "str" | "string" => Ty::Str,
        "bool" => Ty::Bool,
        "void" | "()" => Ty::Void,
        _ => Ty::Unknown,
    }
}

fn type_mismatch(what: &str, want: &str, got: &Ty) -> Diagnostic {
    Diagnostic::error(
        "E-TYPE",
        &format!("{what}: want {want}, got {}", got.name()),
        FILE,
        0,
        0,
        "operand types do not match the operator",
        &["convert the value first", "check the operand types"],
        "types/mismatch",
    )
}

fn method_arity(recv: &str, method: &str, want: usize, got: usize) -> Diagnostic {
    Diagnostic::error(
        "E-ARITY",
        &format!("{recv}.{method}() takes {want} arguments, got {got}"),
        FILE,
        0,
        0,
        "method arity must match",
        &["pass the right number of arguments"],
        "calls/arity",
    )
}

/// A fully effect-checked program.
#[derive(Debug, Clone)]
pub struct TypedHIR {
    pub program: Program,
}

#[derive(Debug, Clone)]
struct Sig {
    effects: Vec<Effect>,
    arity: usize,
    param_tys: Vec<Ty>,
    return_ty: Ty,
}

impl TypedHIR {
    pub fn check(program: Program) -> Result<Self, Vec<Diagnostic>> {
        let mut diags: Vec<Diagnostic> = Vec::new();
        let mut sigs: HashMap<String, Sig> = HashMap::new();
        for f in &program.functions {
            if sigs.contains_key(&f.name) {
                diags.push(duplicate(&format!("duplicate function `{}`", f.name)));
            }
            sigs.insert(
                f.name.clone(),
                Sig {
                    effects: f.effects.clone(),
                    arity: f.params.len(),
                    param_tys: f.params.iter().map(|p| parse_ty(&p.ty)).collect(),
                    return_ty: parse_ty(&f.return_ty),
                },
            );
        }
        // struct name -> (field name -> field type)
        let mut struct_fields: HashMap<String, HashMap<String, Ty>> = HashMap::new();
        let mut struct_names: Vec<String> = Vec::new();
        for s in &program.structs {
            struct_names.push(s.name.clone());
            let mut fields = HashMap::new();
            for fld in &s.fields {
                if fields.contains_key(&fld.name) {
                    diags.push(duplicate(&format!(
                        "duplicate field `{}` in struct `{}`",
                        fld.name, s.name
                    )));
                }
                fields.insert(fld.name.clone(), parse_ty(&fld.ty));
            }
            struct_fields.insert(s.name.clone(), fields);
        }
        struct_names.sort();
        struct_names.dedup();
        if struct_names.len() != program.structs.len() {
            diags.push(Diagnostic::error(
                "E-DUPLICATE",
                "duplicate struct name",
                FILE,
                0,
                0,
                "two structs share one name",
                &["rename one of them"],
                "names/duplicate",
            ));
        }
        let structs: Vec<String> = struct_names;
        for f in &program.functions {
            check_function(f, &sigs, &structs, &struct_fields, &mut diags);
        }
        if diags.is_empty() {
            Ok(Self { program })
        } else {
            Err(diags)
        }
    }

    pub fn program(&self) -> &Program {
        &self.program
    }
}

fn has_effect(effects: &[Effect], want: Effect) -> bool {
    effects.contains(&want)
}

fn check_function(
    f: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    diags: &mut Vec<Diagnostic>,
) {
    let mut defined: HashMap<String, Ty> = HashMap::new();
    for p in &f.params {
        if defined.contains_key(&p.name) {
            diags.push(duplicate(&format!(
                "duplicate parameter `{}` in `{}`",
                p.name, f.name
            )));
        }
        defined.insert(p.name.clone(), parse_ty(&p.ty));
    }
    let mut cx = Ctx { loop_depth: 0 };
    check_block(
        &f.body,
        f,
        sigs,
        structs,
        struct_fields,
        diags,
        0,
        &mut Vec::new(),
        &mut defined,
        &mut cx,
    );
}

#[derive(Debug, Clone)]
struct PendingSpawn {
    handle: String,
    span: (usize, usize),
}

#[derive(Debug, Clone, Default)]
struct Ctx {
    loop_depth: usize,
}

/// Walk a block. `group_stack` holds the pending-spawn list of each enclosing
/// group, innermost last. Returns the set of handles awaited in this block.
#[allow(clippy::too_many_arguments)]
fn check_block(
    block: &Block,
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    diags: &mut Vec<Diagnostic>,
    depth: usize,
    group_stack: &mut Vec<Vec<PendingSpawn>>,
    defined: &mut HashMap<String, Ty>,
    cx: &mut Ctx,
) -> Vec<String> {
    let mut awaited_here: Vec<String> = Vec::new();
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let(l) => {
                // Only a DIRECT `let h = spawn f()` binds a trackable
                // handle. A `spawn` nested anywhere else (or a bare
                // `spawn f();` statement) leaves an unawaitable thread:
                // reject it here instead of leaking at runtime.
                let ty = if let Expr::Spawn { call, .. } = &l.value {
                    if group_stack.is_empty() {
                        diags.push(Diagnostic::spawn_outside_group(FILE, l.span.0, l.span.1));
                    } else {
                        group_stack
                            .last_mut()
                            .expect("group present")
                            .push(PendingSpawn {
                                handle: l.name.clone(),
                                span: l.span,
                            });
                    }
                    check_expr(
                        call,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        diags,
                        depth,
                        &mut awaited_here,
                        defined,
                    )
                } else {
                    check_expr(
                        &l.value,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        diags,
                        depth,
                        &mut awaited_here,
                        defined,
                    )
                };
                // `let x = spawn f()` binds the call's return type; `await x`
                // later yields the same type. Re-`let` shadows (insert, not
                // or_insert): the runtime rebinds, so the new type wins.
                defined.insert(l.name.clone(), ty);
            }
            Stmt::Assign(a) => {
                let target_ty = check_assign_target(
                    &a.target,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                let value_ty = check_expr(
                    &a.value,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                if let Some(want) = target_ty {
                    if !assignable(&value_ty, &want) {
                        diags.push(type_mismatch(
                            "assignment",
                            &want.name(),
                            &value_ty,
                        ));
                    }
                }
            }
            Stmt::Expr(e) => {
                if expr_has_spawn(e) && group_stack.is_empty() {
                    let (s, en) = expr_span_hint(e);
                    diags.push(Diagnostic::spawn_outside_group(FILE, s, en));
                }
                check_expr(
                    e,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
            }
            Stmt::Return(r) => {
                let got = check_expr(
                    &r.value,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                let want = sigs
                    .get(&caller.name)
                    .map(|s| s.return_ty.clone())
                    .unwrap_or(Ty::Unknown);
                if !assignable(&got, &want) {
                    diags.push(type_mismatch(
                        &format!("`{}` return", caller.name),
                        &want.name(),
                        &got,
                    ));
                }
            }
            Stmt::Print(p) => {
                check_expr(
                    &p.value,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
            }
            Stmt::Break(_) | Stmt::Continue(_) => {
                if cx.loop_depth == 0 {
                    diags.push(Diagnostic::error(
                        "E-LOOP",
                        "break/continue outside of any loop",
                        FILE,
                        0,
                        0,
                        "break and continue only make sense inside while/for",
                        &["move it inside a loop", "remove it"],
                        "control/loop",
                    ));
                }
            }
            Stmt::While(w) => {
                let cond_ty = check_expr(
                    &w.cond,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                require_condition(&cond_ty, "while", diags);
                cx.loop_depth += 1;
                let mut body_defined = defined.clone();
                let inner = check_block(
                    &w.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut body_defined,
                    cx,
                );
                cx.loop_depth -= 1;
                awaited_here.extend(inner);
            }
            Stmt::ForRange(fr) => {
                let s_ty = check_expr(
                    &fr.start,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                let e_ty = check_expr(
                    &fr.end,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                for (ty, what) in [(&s_ty, "for range start"), (&e_ty, "for range end")] {
                    if !matches!(ty, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch(what, "i32", ty));
                    }
                }
                cx.loop_depth += 1;
                let mut body_defined = defined.clone();
                // Loop var rebinds at runtime (MIR `Copy`), shadowing any
                // outer binding of the same name.
                body_defined.insert(fr.var.clone(), Ty::Int);
                let inner = check_block(
                    &fr.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut body_defined,
                    cx,
                );
                cx.loop_depth -= 1;
                awaited_here.extend(inner);
            }
            Stmt::ForIn(fi) => {
                let iter_ty = check_expr(
                    &fi.iter,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                match &iter_ty {
                    Ty::Array | Ty::Map | Ty::Str | Ty::Unknown => {}
                    other => diags.push(type_mismatch("for-in iterable", "array/map/str", other)),
                }
                cx.loop_depth += 1;
                let mut body_defined = defined.clone();
                // Element type is dynamic without generics.
                body_defined.insert(fi.var.clone(), Ty::Unknown);
                let inner = check_block(
                    &fi.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut body_defined,
                    cx,
                );
                cx.loop_depth -= 1;
                awaited_here.extend(inner);
            }
            Stmt::If(s) => {
                let cond_ty = check_expr(
                    &s.cond,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                );
                require_condition(&cond_ty, "if", diags);
                // Branches are block-scoped: new lets inside do not leak out.
                let mut then_defined = defined.clone();
                let then_awaited = check_block(
                    &s.then_block,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut then_defined,
                    cx,
                );
                awaited_here.extend(then_awaited);
                if let Some(else_b) = &s.else_block {
                    let mut else_defined = defined.clone();
                    let else_awaited = check_block(
                        else_b,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        diags,
                        depth + 1,
                        group_stack,
                        &mut else_defined,
                        cx,
                    );
                    awaited_here.extend(else_awaited);
                }
            }
            Stmt::TaskGroup(g) => {
                group_stack.push(Vec::new());
                let inner_awaited = check_block(
                    &g.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    diags,
                    depth + 1,
                    group_stack,
                    defined,
                    cx,
                );
                let pending = group_stack.pop().expect("group just pushed");
                for p in &pending {
                    if !inner_awaited.contains(&p.handle) {
                        diags.push(Diagnostic::task_leak(FILE, p.span.0, p.span.1, &p.handle));
                    }
                }
                awaited_here.extend(inner_awaited);
            }
        }
    }
    awaited_here
}

/// `value` of type `got` flows into a slot of type `want`.
/// `Unknown` on either side is a wildcard (dynamic maps/indexes).
fn assignable(got: &Ty, want: &Ty) -> bool {
    if matches!(got, Ty::Unknown) || matches!(want, Ty::Unknown) {
        return true;
    }
    match (got, want) {
        (a, b) if a == b => true,
        // int coerces to float
        (Ty::Int, Ty::Float) => true,
        // struct names must match exactly (handled above)
        _ => false,
    }
}

fn require_condition(ty: &Ty, what: &str, diags: &mut Vec<Diagnostic>) {
    match ty {
        Ty::Bool | Ty::Int | Ty::Unknown => {}
        other => diags.push(type_mismatch(
            &format!("{what} condition"),
            "bool",
            other,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn check_assign_target(
    t: &AssignTarget,
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    diags: &mut Vec<Diagnostic>,
    depth: usize,
    awaited: &mut Vec<String>,
    defined: &HashMap<String, Ty>,
) -> Option<Ty> {
    match t {
        AssignTarget::Var { name } => {
            if let Some(ty) = defined.get(name) {
                Some(ty.clone())
            } else {
                diags.push(undefined(name));
                None
            }
        }
        AssignTarget::Index { base, index } => {
            let base_ty = check_expr(
                base, caller, sigs, structs, struct_fields, diags, depth, awaited, defined,
            );
            let idx_ty = check_expr(
                index, caller, sigs, structs, struct_fields, diags, depth, awaited, defined,
            );
            match &base_ty {
                Ty::Array => {
                    if !matches!(idx_ty, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch("array index", "i32", &idx_ty));
                    }
                    // Element type is dynamic without generics.
                    Some(Ty::Unknown)
                }
                Ty::Map => Some(Ty::Unknown),
                Ty::Str => {
                    if !matches!(idx_ty, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch("string index", "i32", &idx_ty));
                    }
                    Some(Ty::Str)
                }
                Ty::Unknown => Some(Ty::Unknown),
                other => {
                    diags.push(type_mismatch("index base", "array/map/str", other));
                    Some(Ty::Unknown)
                }
            }
        }
        AssignTarget::Field { base, field } => {
            let base_ty = check_expr(
                base, caller, sigs, structs, struct_fields, diags, depth, awaited, defined,
            );
            match &base_ty {
                Ty::Struct(name) => {
                    if let Some(fields) = struct_fields.get(name) {
                        if let Some(ty) = fields.get(field) {
                            Some(ty.clone())
                        } else {
                            diags.push(undefined(field));
                            None
                        }
                    } else {
                        Some(Ty::Unknown)
                    }
                }
                Ty::Unknown => Some(Ty::Unknown),
                other => {
                    diags.push(type_mismatch("field base", "struct", other));
                    Some(Ty::Unknown)
                }
            }
        }
    }
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
        &format!("undefined variable `{name}`"),
        FILE,
        0,
        0,
        "name is not a parameter or a prior `let` binding",
        &["bind it with `let` first", "check the spelling"],
        "names/scope",
    )
}

fn arity_mismatch(caller: &str, callee: &str, want: usize, got: usize) -> Diagnostic {
    Diagnostic::error(
        "E-ARITY",
        &format!("`{caller}` calls `{callee}` with {got} args, want {want}"),
        FILE,
        0,
        0,
        "call arity must match the callee parameter list",
        &["pass the right number of arguments"],
        "calls/arity",
    )
}

#[allow(clippy::too_many_arguments)]
fn check_expr(
    e: &Expr,
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    diags: &mut Vec<Diagnostic>,
    _depth: usize,
    awaited: &mut Vec<String>,
    defined: &HashMap<String, Ty>,
) -> Ty {
    match e {
        Expr::Int { value, .. } => {
            // `run` narrows to i32: reject literals that would wrap.
            if *value > i32::MAX as i64 {
                diags.push(type_mismatch("integer literal", "i32 range", &Ty::Int));
                return Ty::Unknown;
            }
            Ty::Int
        }
        Expr::Float { .. } => Ty::Float,
        Expr::Str { .. } => Ty::Str,
        Expr::Bool { .. } => Ty::Bool,
        Expr::Spawn { call, .. } => {
            // Reached only for non-direct positions (`let`-direct is
            // handled by the caller): an unawaitable thread. Reject.
            diags.push(Diagnostic::spawn_position(FILE));
            check_expr(
                call, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            )
        }
        Expr::Await { name, .. } => {
            if !awaited.contains(name) {
                awaited.push(name.clone());
            }
            defined.get(name).cloned().unwrap_or(Ty::Unknown)
        }
        Expr::Call { func, args, .. } => {
            let mut arg_tys = Vec::with_capacity(args.len());
            for a in args {
                arg_tys.push(check_expr(
                    a, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
                ));
            }
            if is_builtin(func) {
                return check_builtin_call(func, &arg_tys, diags);
            }
            if let Some(sig) = sigs.get(func) {
                if has_effect(&sig.effects, Effect::Throws)
                    && !has_effect(&caller.effects, Effect::Throws)
                {
                    let (s, en) = caller.name_span;
                    diags.push(Diagnostic::effect_mismatch(
                        FILE,
                        s,
                        en,
                        &caller.name,
                        func,
                        "throws",
                    ));
                }
                if args.len() != sig.arity {
                    diags.push(arity_mismatch(&caller.name, func, sig.arity, args.len()));
                    return Ty::Unknown;
                }
                for (i, (got, want)) in arg_tys.iter().zip(sig.param_tys.iter()).enumerate() {
                    if !assignable(got, want) {
                        diags.push(type_mismatch(
                            &format!("`{func}` arg {i}"),
                            &want.name(),
                            got,
                        ));
                    }
                }
                sig.return_ty.clone()
            } else {
                diags.push(undefined(func));
                Ty::Unknown
            }
        }
        Expr::Var { name, .. } => {
            if let Some(ty) = defined.get(name) {
                ty.clone()
            } else {
                diags.push(undefined(name));
                Ty::Unknown
            }
        }
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                check_expr(
                    el, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
                );
            }
            Ty::Array
        }
        Expr::StructLit { name, fields, .. } => {
            if !structs.contains(name) {
                diags.push(undefined(name));
                for (_, v) in fields {
                    check_expr(
                        v, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
                    );
                }
                return Ty::Unknown;
            }
            if let Some(known) = struct_fields.get(name) {
                for (fname, v) in fields {
                    let vty = check_expr(
                        v, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
                    );
                    if let Some(want) = known.get(fname) {
                        if !assignable(&vty, want) {
                            diags.push(type_mismatch(
                                &format!("`{name}.{fname}`"),
                                &want.name(),
                                &vty,
                            ));
                        }
                    } else {
                        diags.push(undefined(fname));
                    }
                }
            } else {
                for (_, v) in fields {
                    check_expr(
                        v, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
                    );
                }
            }
            Ty::Struct(name.clone())
        }
        Expr::Index { base, index, .. } => {
            let b = check_expr(
                base, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let idx = check_expr(
                index, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            match &b {
                Ty::Array => {
                    if !matches!(idx, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch("array index", "i32", &idx));
                    }
                    Ty::Unknown
                }
                Ty::Map => Ty::Unknown,
                Ty::Str => {
                    if !matches!(idx, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch("string index", "i32", &idx));
                    }
                    Ty::Str
                }
                Ty::Unknown => Ty::Unknown,
                other => {
                    diags.push(type_mismatch("index base", "array/map/str", other));
                    Ty::Unknown
                }
            }
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                check_expr(
                    v, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
                );
            }
            Ty::Map
        }
        Expr::Field { base, field, .. } => {
            let b = check_expr(
                base, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            match &b {
                Ty::Struct(name) => {
                    if let Some(fields) = struct_fields.get(name) {
                        if let Some(ty) = fields.get(field) {
                            ty.clone()
                        } else {
                            diags.push(undefined(field));
                            Ty::Unknown
                        }
                    } else {
                        Ty::Unknown
                    }
                }
                Ty::Unknown => Ty::Unknown,
                other => {
                    diags.push(type_mismatch("field base", "struct", other));
                    Ty::Unknown
                }
            }
        }
        Expr::MethodCall { base, method, args, .. } => {
            let b = check_expr(
                base, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let mut arg_tys = Vec::with_capacity(args.len());
            for a in args {
                arg_tys.push(check_expr(
                    a, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
                ));
            }
            check_method_call(&b, method, &arg_tys, diags)
        }
        Expr::Add { left, right, .. } => {
            let l = check_expr(
                left, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let r = check_expr(
                right, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            match (&l, &r) {
                (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                (Ty::Int, Ty::Int) => Ty::Int,
                (Ty::Float, Ty::Float)
                | (Ty::Int, Ty::Float)
                | (Ty::Float, Ty::Int) => Ty::Float,
                (Ty::Str, _) | (_, Ty::Str) => Ty::Str,
                _ => {
                    diags.push(type_mismatch("`+` operands", "i32/f64/str", &l));
                    Ty::Unknown
                }
            }
        }
        Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. } => {
            let op = match e {
                Expr::Sub { .. } => "`-`",
                Expr::Mul { .. } => "`*`",
                Expr::Div { .. } => "`/`",
                _ => "`%`",
            };
            let l = check_expr(
                left, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let r = check_expr(
                right, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            if matches!(e, Expr::Mod { .. }) {
                // `%` is integers only.
                let ok = matches!(
                    (&l, &r),
                    (Ty::Unknown, _)
                        | (_, Ty::Unknown)
                        | (Ty::Int, Ty::Int)
                );
                if !ok {
                    diags.push(type_mismatch("`%` operands", "i32", &l));
                }
                return Ty::Int;
            }
            match (&l, &r) {
                (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                (Ty::Int, Ty::Int) => Ty::Int,
                (Ty::Float, Ty::Float)
                | (Ty::Int, Ty::Float)
                | (Ty::Float, Ty::Int) => Ty::Float,
                _ => {
                    diags.push(type_mismatch(&format!("{op} operands"), "i32/f64", &l));
                    Ty::Unknown
                }
            }
        }
        Expr::Eq { left, right, .. } | Expr::NotEq { left, right, .. } => {
            let l = check_expr(
                left, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let r = check_expr(
                right, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            if !equality_ok(&l, &r) {
                diags.push(type_mismatch("`==` operands", &l.name(), &r));
            }
            Ty::Bool
        }
        Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. } => {
            let l = check_expr(
                left, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let r = check_expr(
                right, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let ok = matches!(
                (&l, &r),
                (Ty::Unknown, _)
                    | (_, Ty::Unknown)
                    | (Ty::Int, Ty::Int)
                    | (Ty::Float, Ty::Float)
                    | (Ty::Int, Ty::Float)
                    | (Ty::Float, Ty::Int)
                    | (Ty::Str, Ty::Str)
            );
            if !ok {
                diags.push(type_mismatch("comparison operands", "i32/f64/str", &l));
            }
            Ty::Bool
        }
        Expr::And { left, right, .. } | Expr::Or { left, right, .. } => {
            let l = check_expr(
                left, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            let r = check_expr(
                right, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            for (ty, what) in [(&l, "left `&&`/`||`"), (&r, "right `&&`/`||`")] {
                if !matches!(ty, Ty::Bool | Ty::Int | Ty::Unknown) {
                    diags.push(type_mismatch(what, "bool", ty));
                }
            }
            Ty::Bool
        }
        Expr::Not { inner, .. } => {
            let t = check_expr(
                inner, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            if !matches!(t, Ty::Bool | Ty::Int | Ty::Unknown) {
                diags.push(type_mismatch("`!` operand", "bool", &t));
            }
            Ty::Bool
        }
        Expr::Neg { inner, .. } => {
            // `-2147483648` is valid i32::MIN although the positive
            // literal alone exceeds the range: check the negated value.
            if let Expr::Int { value, .. } = inner.as_ref() {
                if *value > 2147483648 {
                    diags.push(type_mismatch("integer literal", "i32 range", &Ty::Int));
                    return Ty::Unknown;
                }
                return Ty::Int;
            }
            let t = check_expr(
                inner, caller, sigs, structs, struct_fields, diags, _depth, awaited, defined,
            );
            match &t {
                Ty::Int => Ty::Int,
                Ty::Float => Ty::Float,
                Ty::Unknown => Ty::Unknown,
                other => {
                    diags.push(type_mismatch("unary `-` operand", "i32/f64", other));
                    Ty::Unknown
                }
            }
        }
    }
}

/// `==` allows numeric mixes, identical types, or anything-Unknown.
fn equality_ok(l: &Ty, r: &Ty) -> bool {
    if matches!(l, Ty::Unknown) || matches!(r, Ty::Unknown) {
        return true;
    }
    if l == r {
        return true;
    }
    matches!(
        (l, r),
        (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int)
    )
}

fn check_builtin_call(func: &str, args: &[Ty], diags: &mut Vec<Diagnostic>) -> Ty {
    let arity = match func {
        "len" | "pop" | "keys" => 1,
        "push" | "range" | "write_file" => 2,
        "str" | "int" | "float" => 1,
        "assert" => 1,
        "read_file" | "exists" | "env" => 1,
        _ => return Ty::Unknown,
    };
    if args.len() != arity {
        diags.push(Diagnostic::error(
            "E-ARITY",
            &format!("`{func}` takes {arity} arguments, got {}", args.len()),
            FILE,
            0,
            0,
            "builtin arity must match",
            &["pass the right number of arguments"],
            "calls/arity",
        ));
        return Ty::Unknown;
    }
    match func {
        "len" => {
            if !matches!(&args[0], Ty::Array | Ty::Str | Ty::Map | Ty::Unknown) {
                diags.push(type_mismatch("`len()` argument", "array/str/map", &args[0]));
            }
            Ty::Int
        }
        "push" => {
            if !matches!(&args[0], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch("`push()` target", "array", &args[0]));
            }
            Ty::Int
        }
        "pop" => {
            if !matches!(&args[0], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch("`pop()` target", "array", &args[0]));
            }
            Ty::Unknown
        }
        "range" => {
            for a in args {
                if !matches!(a, Ty::Int | Ty::Unknown) {
                    diags.push(type_mismatch("`range()` bound", "i32", a));
                }
            }
            Ty::Array
        }
        "str" => Ty::Str,
        "int" => Ty::Int,
        "float" => Ty::Float,
        "keys" => {
            if !matches!(&args[0], Ty::Map | Ty::Unknown) {
                diags.push(type_mismatch("`keys()` argument", "map", &args[0]));
            }
            Ty::Array
        }
        "assert" => {
            if !matches!(&args[0], Ty::Bool | Ty::Int | Ty::Unknown) {
                diags.push(type_mismatch("`assert()` argument", "bool", &args[0]));
            }
            Ty::Int
        }
        "read_file" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch("`read_file()` path", "str", &args[0]));
            }
            Ty::Str
        }
        "write_file" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        &format!("`write_file()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Int
        }
        "exists" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch("`exists()` path", "str", &args[0]));
            }
            Ty::Bool
        }
        "env" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch("`env()` name", "str", &args[0]));
            }
            Ty::Str
        }
        _ => Ty::Unknown,
    }
}

/// Method-call result types by receiver type. Unknown receiver -> Unknown.
fn check_method_call(
    base: &Ty,
    method: &str,
    args: &[Ty],
    diags: &mut Vec<Diagnostic>,
) -> Ty {
    match base {
        Ty::Unknown => Ty::Unknown,
        Ty::Str => match method {
            "len" | "upper" | "lower" | "trim" | "chars" | "split" | "contains" | "starts_with"
            | "ends_with" | "replace" => {
                let want = match method {
                    "replace" => 2,
                    "split" | "contains" | "starts_with" | "ends_with" => 1,
                    _ => 0,
                };
                if args.len() != want {
                    diags.push(method_arity("str", method, want, args.len()));
                    return Ty::Unknown;
                }
                match method {
                    "len" => Ty::Int,
                    "split" | "chars" => Ty::Array,
                    "contains" | "starts_with" | "ends_with" => Ty::Bool,
                    _ => Ty::Str,
                }
            }
            _ => {
                diags.push(type_mismatch("string method", "known str method", base));
                Ty::Unknown
            }
        },
        Ty::Array => match method {
            "len" | "push" | "pop" | "contains" | "join" => {
                let want = match method {
                    "push" | "contains" | "join" => 1,
                    _ => 0,
                };
                if args.len() != want {
                    diags.push(method_arity("array", method, want, args.len()));
                    return Ty::Unknown;
                }
                match method {
                    "len" | "push" => Ty::Int,
                    "contains" => Ty::Bool,
                    "join" => Ty::Str,
                    _ => Ty::Unknown,
                }
            }
            _ => {
                diags.push(type_mismatch("array method", "known array method", base));
                Ty::Unknown
            }
        },
        Ty::Map => match method {
            "len" | "keys" | "contains" => {
                let want = if method == "contains" { 1 } else { 0 };
                if args.len() != want {
                    diags.push(method_arity("map", method, want, args.len()));
                    return Ty::Unknown;
                }
                match method {
                    "len" => Ty::Int,
                    "keys" => Ty::Array,
                    _ => Ty::Bool,
                }
            }
            _ => {
                diags.push(type_mismatch("map method", "known map method", base));
                Ty::Unknown
            }
        },
        other => {
            diags.push(type_mismatch("method receiver", "str/array/map", other));
            Ty::Unknown
        }
    }
}

/// Builtin functions available in every program.
pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "len" | "push"
            | "pop"
            | "range"
            | "str"
            | "int"
            | "float"
            | "keys"
            | "assert"
            | "read_file"
            | "write_file"
            | "exists"
            | "env"
    )
}

fn expr_has_spawn(e: &Expr) -> bool {
    match e {
        Expr::Spawn { .. } => true,
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
        | Expr::Or { left, right, .. } => expr_has_spawn(left) || expr_has_spawn(right),
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => expr_has_spawn(inner),
        Expr::Call { args, .. } => args.iter().any(expr_has_spawn),
        Expr::ArrayLit { elems, .. } => elems.iter().any(expr_has_spawn),
        Expr::MapLit { entries, .. } => entries.iter().any(|(_, v)| expr_has_spawn(v)),
        Expr::Index { base, index, .. } => expr_has_spawn(base) || expr_has_spawn(index),
        Expr::Field { base, .. } => expr_has_spawn(base),
        Expr::MethodCall { base, args, .. } => {
            expr_has_spawn(base) || args.iter().any(expr_has_spawn)
        }
        _ => false,
    }
}

fn expr_span_hint(e: &Expr) -> (usize, usize) {
    match e {
        Expr::Spawn { call, .. } => expr_span_hint(call),
        _ => (0, 0),
    }
}
