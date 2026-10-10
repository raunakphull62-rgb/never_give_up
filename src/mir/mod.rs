//! Stage 3 (part 1): ownership-agnostic MIR (v0.3).
//!
//! Lowering: typed HIR (`Program`) -> flat `MirModule` with explicit
//! task-group regions and jump-based `if`/`while`/`for`. Every MIR item keeps
//! its source `NodeId` as origin so diagnostics and later derives can map back.

//! v2 Flow lowering (explicit environments) lives in [`flow_lowering`];
//! v2 Echo state machines arrive in Phase 9.
//! v2 full program lowering (schemas, echoes, flows, functions).
pub mod v2_lowering;

/// v2 Echo lowering (explicit state machines).
pub mod echo_lowering;
/// v2 Flow lowering (explicit `dep=` environments).
pub mod flow_lowering;

use std::collections::HashMap;

use crate::ast::{AssignTarget, Expr, MatchBinding, NodeId, Program, Stmt};

/// One MIR instruction with source origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirInstr {
    pub origin: NodeId,
    pub op: MirOp,
}

/// MIR operations (v0.3: real values, calls with args, branches, loops,
/// arrays, structs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirOp {
    /// Enter lexical task group.
    EnterGroup {
        group: NodeId,
    },
    /// Leave lexical task group.
    LeaveGroup {
        group: NodeId,
    },
    /// Spawn `func(args...)` into `handle` (runs inline, single-threaded).
    Spawn {
        handle: String,
        func: String,
        args: Vec<String>,
    },
    /// Await `handle`.
    Await {
        handle: String,
    },
    /// Call `func(args...)` into `into` (user func or builtin).
    Call {
        into: String,
        func: String,
        args: Vec<String>,
    },
    /// Snapshot captures into a closure value: `into` becomes the
    /// closure of lifted function `func` over the current values of
    /// `captures` (binding names, read at this site: by-value copy).
    /// Calling through the value dispatches dynamically in `Call`.
    ClosureNew {
        into: String,
        func: String,
        captures: Vec<String>,
    },
    /// Integer / float / string constants.
    Const {
        into: String,
        value: i64,
    },
    ConstFloat {
        into: String,
        bits: u64,
    },
    ConstStr {
        into: String,
        value: String,
    },
    /// Copy one binding to another (`let x = <var>`).
    Copy {
        into: String,
        from: String,
    },
    /// Binary ops over values (`Eq` family yields 1/0).
    Add {
        into: String,
        left: String,
        right: String,
    },
    Sub {
        into: String,
        left: String,
        right: String,
    },
    Mul {
        into: String,
        left: String,
        right: String,
    },
    Div {
        into: String,
        left: String,
        right: String,
    },
    Mod {
        into: String,
        left: String,
        right: String,
    },
    Eq {
        into: String,
        left: String,
        right: String,
    },
    NotEq {
        into: String,
        left: String,
        right: String,
    },
    Lt {
        into: String,
        left: String,
        right: String,
    },
    LtEq {
        into: String,
        left: String,
        right: String,
    },
    Gt {
        into: String,
        left: String,
        right: String,
    },
    GtEq {
        into: String,
        left: String,
        right: String,
    },
    And {
        into: String,
        left: String,
        right: String,
    },
    Or {
        into: String,
        left: String,
        right: String,
    },
    /// Unary minus / logical not (1/0).
    Neg {
        into: String,
        inner: String,
    },
    Not {
        into: String,
        inner: String,
    },
    /// Aggregate values.
    ArrayNew {
        into: String,
        elems: Vec<String>,
    },
    MapNew {
        into: String,
        entries: Vec<(String, String)>,
    },
    /// `base.method(args...)` — dispatched at runtime by value type.
    MethodCall {
        into: String,
        base: String,
        method: String,
        args: Vec<String>,
    },
    Index {
        into: String,
        base: String,
        index: String,
    },
    StoreIndex {
        base: String,
        index: String,
        value: String,
    },
    FieldGet {
        into: String,
        base: String,
        field: String,
    },
    FieldSet {
        base: String,
        field: String,
        value: String,
    },
    StructNew {
        into: String,
        name: String,
        fields: Vec<(String, String)>,
    },
    /// Length of an array or string.
    Len {
        into: String,
        of: String,
    },
    /// Builtin print.
    Print {
        value: String,
    },
    /// Structured-branch jumps (targets are instruction indices).
    JumpIfFalse {
        cond: String,
        target: usize,
    },
    Jump {
        target: usize,
    },
    /// Return handle/value.
    Return {
        value: String,
    },
    /// `try { body } catch var { handler }`: run `body` inline in the
    /// caller's scope; on a runtime error bind `var` to a
    /// `{code, message}` map and run `handler` instead. Both slices use
    /// slice-relative jump targets (lowered into fresh vectors, so all
    /// loop/break patching stays inside the slice); `break`/`continue`
    /// crossing into an outer loop is rejected at check time.
    Try {
        code_var: String,
        body: Vec<MirInstr>,
        handler: Vec<MirInstr>,
    },
}

/// One lowered function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirFunction {
    pub name: String,
    pub origin: NodeId,
    pub params: Vec<String>,
    /// Raw declared annotation per parameter (D3: the runtime enforces
    /// `i32`/`u32`/`u64`/`u8` ranges at call boundaries from these;
    /// `""` means "no annotation", e.g. closure capture prefixes).
    pub param_tys: Vec<String>,
    /// Raw declared return annotation (D3: enforced at `return`).
    pub return_ty: String,
    pub instrs: Vec<MirInstr>,
}

/// Whole lowered module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirModule {
    pub functions: Vec<MirFunction>,
    /// Struct name -> (field name -> raw declared field type). D3 uses
    /// this for construction (`StructNew`) and field-assignment
    /// (`FieldSet`) boundary checks. Enum payloads are not listed
    /// (they lower through `StructNew` under the enum name and stay
    /// statically checked only — a documented gap).
    pub struct_fields: HashMap<String, HashMap<String, String>>,
}

impl MirModule {
    pub fn find(&self, name: &str) -> Option<&MirFunction> {
        self.functions.iter().find(|f| f.name == name)
    }
}

/// Loop context: pending `break`/`continue` jump fixups.
#[derive(Debug, Default)]
struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

fn push(instrs: &mut Vec<MirInstr>, origin: &NodeId, op: MirOp) -> usize {
    instrs.push(MirInstr {
        origin: origin.clone(),
        op,
    });
    instrs.len() - 1
}

fn tmp_name(tmp: &mut u32) -> String {
    let name = format!("t{tmp}");
    *tmp += 1;
    name
}

/// Lower one pattern binding element against one payload position.
/// `base` holds the parent value, `i` the field index. `_` emits
/// nothing; a variable emits `FieldGet`; a nested pattern extracts the
/// field into a temp, tag-checks it (mismatch falls through to the next
/// arm via `pending`, exactly like a top-level tag mismatch), then
/// recurses. Runs strictly after the parent pattern matched and before
/// arm setup, so setup/guard/body only ever see a fully-matched arm.
#[allow(clippy::too_many_arguments)]
fn lower_pattern(
    b: &MatchBinding,
    base: &str,
    i: usize,
    arm_id: &NodeId,
    expr_id: &NodeId,
    instrs: &mut Vec<MirInstr>,
    tmp: &mut u32,
    pending: &mut Vec<usize>,
) {
    match b {
        MatchBinding::Ignore => {}
        MatchBinding::Bind(name) => {
            push(
                instrs,
                arm_id,
                MirOp::FieldGet {
                    into: name.clone(),
                    base: base.to_string(),
                    field: format!("f{i}"),
                },
            );
        }
        MatchBinding::Nested { variant, bindings, .. } => {
            // Extract first: a non-struct field errors loudly here, the
            // same treatment a non-enum top-level scrutinee gets
            // (checked programs only reach this with enum-typed fields —
            // anything else is `E-TYPE` at check time).
            let ft = tmp_name(tmp);
            push(
                instrs,
                arm_id,
                MirOp::FieldGet {
                    into: ft.clone(),
                    base: base.to_string(),
                    field: format!("f{i}"),
                },
            );
            let ftag = tmp_name(tmp);
            push(
                instrs,
                arm_id,
                MirOp::FieldGet {
                    into: ftag.clone(),
                    base: ft.clone(),
                    field: "__variant".to_string(),
                },
            );
            let want = tmp_name(tmp);
            push(
                instrs,
                arm_id,
                MirOp::ConstStr {
                    into: want.clone(),
                    value: variant.clone(),
                },
            );
            let cmp = tmp_name(tmp);
            push(
                instrs,
                arm_id,
                MirOp::Eq {
                    into: cmp.clone(),
                    left: ftag,
                    right: want,
                },
            );
            pending.push(push(
                instrs,
                expr_id,
                MirOp::JumpIfFalse {
                    cond: cmp,
                    target: usize::MAX,
                },
            ));
            for (j, sb) in bindings.iter().enumerate() {
                lower_pattern(sb, &ft, j, arm_id, expr_id, instrs, tmp, pending);
            }
        }
    }
}

fn lower_expr_to_value(e: &Expr, instrs: &mut Vec<MirInstr>, tmp: &mut u32) -> String {
    match e {
        Expr::Int { value, .. } => {
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::Const {
                    into: dst.clone(),
                    value: *value,
                },
            );
            dst
        }
        Expr::Float { bits, .. } => {
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::ConstFloat {
                    into: dst.clone(),
                    bits: *bits,
                },
            );
            dst
        }
        Expr::Str { value, .. } => {
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::ConstStr {
                    into: dst.clone(),
                    value: value.clone(),
                },
            );
            dst
        }
        Expr::Bool { value, .. } => {
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::Const {
                    into: dst.clone(),
                    value: i64::from(*value),
                },
            );
            dst
        }
        Expr::ArrayLit { elems, .. } => {
            let mut lowered = Vec::with_capacity(elems.len());
            for el in elems {
                lowered.push(lower_expr_to_value(el, instrs, tmp));
            }
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::ArrayNew {
                    into: dst.clone(),
                    elems: lowered,
                },
            );
            dst
        }
        Expr::MapLit { entries, .. } => {
            let mut lowered = Vec::with_capacity(entries.len());
            for (k, v) in entries {
                lowered.push((k.clone(), lower_expr_to_value(v, instrs, tmp)));
            }
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::MapNew {
                    into: dst.clone(),
                    entries: lowered,
                },
            );
            dst
        }
        Expr::StructLit { name, fields, .. } => {
            let mut lowered = Vec::with_capacity(fields.len());
            for (fname, val) in fields {
                lowered.push((fname.clone(), lower_expr_to_value(val, instrs, tmp)));
            }
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::StructNew {
                    into: dst.clone(),
                    name: name.clone(),
                    fields: lowered,
                },
            );
            dst
        }
        Expr::EnumCtor {
            enum_name,
            variant,
            args,
            ..
        } => {
            // Enums reuse the struct runtime value: the tag lives in a
            // hidden `__variant` string field, payloads in positional
            // `f0`, `f1`, ... fields. No new MIR op is needed, so the
            // existing backends keep working unchanged.
            let tag = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::ConstStr {
                    into: tag.clone(),
                    value: variant.clone(),
                },
            );
            let mut fields = vec![("__variant".to_string(), tag)];
            for (i, a) in args.iter().enumerate() {
                let v = lower_expr_to_value(a, instrs, tmp);
                fields.push((format!("f{i}"), v));
            }
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::StructNew {
                    into: dst.clone(),
                    name: enum_name.clone(),
                    fields,
                },
            );
            dst
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            // Tag dispatch over existing ops: read `__variant`, compare
            // against each arm in order, bind payloads with `FieldGet`.
            let s = lower_expr_to_value(scrutinee, instrs, tmp);
            let tag = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::FieldGet {
                    into: tag.clone(),
                    base: s.clone(),
                    field: "__variant".to_string(),
                },
            );
            let dst = tmp_name(tmp);
            // Arm bindings share the flat value namespace, so save any
            // same-named outer bindings and restore them per arm. This
            // covers payload bindings AND top-level setup `let`s: a setup
            // `let n` shadowing an outer `n` must not clobber it after a
            // taken arm, nor leak into later arms on guard-false
            // fallthrough (the post-body and per-entry restores below).
            // (Nested lets inside setup control flow share the general
            // flat-namespace quirk of all blocks — same as `if` bodies,
            // out of scope here.)
            let mut names: Vec<String> = Vec::new();
            for arm in arms.iter() {
                // Pattern bindings at every nesting level share the flat
                // namespace (collected recursively, so nested bindings
                // restore exactly like top-level ones on fallthrough).
                for b in arm.bindings.iter() {
                    let mut bound = Vec::new();
                    b.bound_names(&mut bound);
                    for n in bound {
                        if !names.contains(&n) {
                            names.push(n);
                        }
                    }
                }
                for s in arm.stmts.iter() {
                    if let crate::ast::Stmt::Let(l) = s {
                        if !names.contains(&l.name) {
                            names.push(l.name.clone());
                        }
                    }
                }
            }
            let mut saved: Vec<(String, String)> = Vec::new();
            for n in names.iter() {
                let sv = tmp_name(tmp);
                push(
                    instrs,
                    e.id(),
                    MirOp::Copy {
                        into: sv.clone(),
                        from: n.clone(),
                    },
                );
                saved.push((n.clone(), sv));
            }
            let mut pending: Vec<usize> = Vec::new();
            let mut end_jumps: Vec<usize> = Vec::new();
            for arm in arms.iter() {
                // Every arm entry first patches all pending fallthrough
                // jumps (tag mismatch or false guard) here, then restores
                // possibly-clobbered bindings: a previous arm may have
                // bound payloads before falling through on a false guard.
                // Restores copy from saved temps (which never change), so
                // they are idempotent and safe on every path, including
                // the first arm (a plain no-op there).
                if !pending.is_empty() {
                    let target = instrs.len();
                    for jf in pending.drain(..) {
                        patch_target(instrs, jf, target);
                    }
                }
                for (n, sv) in saved.iter() {
                    push(
                        instrs,
                        e.id(),
                        MirOp::Copy {
                            into: n.clone(),
                            from: sv.clone(),
                        },
                    );
                }
                if !arm.is_wildcard() {
                    let want = tmp_name(tmp);
                    push(
                        instrs,
                        e.id(),
                        MirOp::ConstStr {
                            into: want.clone(),
                            value: arm.variant.clone().unwrap_or_default(),
                        },
                    );
                    let cmp = tmp_name(tmp);
                    push(
                        instrs,
                        e.id(),
                        MirOp::Eq {
                            into: cmp.clone(),
                            left: tag.clone(),
                            right: want,
                        },
                    );
                    pending.push(push(
                        instrs,
                        e.id(),
                        MirOp::JumpIfFalse {
                            cond: cmp,
                            target: usize::MAX,
                        },
                    ));
                }
                for (i, b) in arm.bindings.iter().enumerate() {
                    lower_pattern(
                        b,
                        &s,
                        i,
                        &arm.id,
                        e.id(),
                        instrs,
                        tmp,
                        &mut pending,
                    );
                }
                if !arm.stmts.is_empty() {
                    // Setup statements run inline in the taken branch, in
                    // order, BEFORE the guard is checked (SPEC §2b: the
                    // guard observes setup bindings) and before the
                    // trailing value expression. Fresh loop stack: loops
                    // inside the arm are self-contained; `break`/`continue`
                    // crossing the arm boundary is an `E-LOOP` at check
                    // time (unchecked MIR reaching here surfaces as a loud
                    // invalid-jump error, never silent).
                    let mut arm_loops: Vec<LoopCtx> = Vec::new();
                    lower_block(&arm.stmts, instrs, tmp, &mut arm_loops);
                }
                if let Some(g) = &arm.guard {
                    // The guard runs only after the pattern matched (the
                    // tag jump above skips setup AND guard entirely
                    // otherwise). A false guard falls through to the next
                    // arm, never a crash and never a silent wrong match.
                    // Setup effects on outer variables persist on
                    // fallthrough (same as an `if` block body); setup
                    // `let`s are arm-local via the save/restore above.
                    let c = lower_expr_to_value(g, instrs, tmp);
                    pending.push(push(
                        instrs,
                        e.id(),
                        MirOp::JumpIfFalse {
                            cond: c,
                            target: usize::MAX,
                        },
                    ));
                }
                let v = lower_expr_to_value(&arm.body, instrs, tmp);
                if v != dst {
                    push(
                        instrs,
                        e.id(),
                        MirOp::Copy {
                            into: dst.clone(),
                            from: v,
                        },
                    );
                }
                for (n, sv) in saved.iter() {
                    push(
                        instrs,
                        e.id(),
                        MirOp::Copy {
                            into: n.clone(),
                            from: sv.clone(),
                        },
                    );
                }
                end_jumps.push(push(instrs, e.id(), MirOp::Jump { target: usize::MAX }));
            }
            // No arm matched (unreachable when HIR accepted the program —
            // every variant has an unguarded arm or wildcard — reachable
            // only through unchecked MIR, e.g. total guard failure):
            // force a loud runtime error, never a silent default value.
            if !pending.is_empty() {
                let target = instrs.len();
                for jf in pending.drain(..) {
                    patch_target(instrs, jf, target);
                }
            }
            push(
                instrs,
                e.id(),
                MirOp::FieldGet {
                    into: dst.clone(),
                    base: s.clone(),
                    field: "__match_fallthrough".to_string(),
                },
            );
            let end_at = instrs.len();
            for j in end_jumps {
                patch_target(instrs, j, end_at);
            }
            dst
        }
        Expr::Index { base, index, .. } => {
            let b = lower_expr_to_value(base, instrs, tmp);
            let i = lower_expr_to_value(index, instrs, tmp);
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::Index {
                    into: dst.clone(),
                    base: b,
                    index: i,
                },
            );
            dst
        }
        Expr::Field { base, field, .. } => {
            let b = lower_expr_to_value(base, instrs, tmp);
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::FieldGet {
                    into: dst.clone(),
                    base: b,
                    field: field.clone(),
                },
            );
            dst
        }
        Expr::MethodCall {
            base, method, args, ..
        } => {
            let b = lower_expr_to_value(base, instrs, tmp);
            let mut lowered = Vec::with_capacity(args.len());
            for a in args {
                lowered.push(lower_expr_to_value(a, instrs, tmp));
            }
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::MethodCall {
                    into: dst.clone(),
                    base: b,
                    method: method.clone(),
                    args: lowered,
                },
            );
            dst
        }
        Expr::Await { name, .. } => {
            push(
                instrs,
                e.id(),
                MirOp::Await {
                    handle: name.clone(),
                },
            );
            name.clone()
        }
        Expr::Var { name, .. } => name.clone(),
        Expr::Closure { id, params, body, .. } => {
            // Lambda lifting over existing ops: the literal's lifted
            // function (synthesized in `lower`, same name) takes the
            // captures first, then the declared parameters. Here we only
            // snapshot the captures into the value; dispatch happens in
            // `Call`. Capture order is `closure_captures` (sorted,
            // deterministic), matching the lifted parameter prefix.
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::ClosureNew {
                    into: dst.clone(),
                    func: crate::closures::closure_fn_name(id),
                    captures: crate::closures::closure_captures(params, body),
                },
            );
            dst
        }
        Expr::Call { func, args, .. } => {
            let mut lowered = Vec::with_capacity(args.len());
            for a in args {
                lowered.push(lower_expr_to_value(a, instrs, tmp));
            }
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::Call {
                    into: dst.clone(),
                    func: func.clone(),
                    args: lowered,
                },
            );
            dst
        }
        Expr::Add { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Add),
        Expr::Sub { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Sub),
        Expr::Mul { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Mul),
        Expr::Div { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Div),
        Expr::Mod { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Mod),
        Expr::Eq { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Eq),
        Expr::NotEq { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::NotEq),
        Expr::Lt { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Lt),
        Expr::LtEq { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::LtEq),
        Expr::Gt { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Gt),
        Expr::GtEq { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::GtEq),
        Expr::And { left, right, .. } => short_circuit(e, left, right, instrs, tmp, true),
        Expr::Or { left, right, .. } => short_circuit(e, left, right, instrs, tmp, false),
        Expr::Not { inner, .. } => {
            let v = lower_expr_to_value(inner, instrs, tmp);
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::Not {
                    into: dst.clone(),
                    inner: v,
                },
            );
            dst
        }
        Expr::Neg { inner, .. } => {
            // D3: fold a negated literal. In particular the 2^63
            // magnitude placeholder (`i64::MIN` from the lexer) negates
            // to `i64::MIN`, which cannot go through `MirOp::Neg` (that
            // would overflow on it); every other magnitude folds exactly.
            if let Expr::Int { value, .. } = inner.as_ref() {
                let folded = if *value == i64::MIN {
                    i64::MIN
                } else {
                    -value
                };
                let dst = tmp_name(tmp);
                push(
                    instrs,
                    e.id(),
                    MirOp::Const {
                        into: dst.clone(),
                        value: folded,
                    },
                );
                return dst;
            }
            let v = lower_expr_to_value(inner, instrs, tmp);
            let dst = tmp_name(tmp);
            push(
                instrs,
                e.id(),
                MirOp::Neg {
                    into: dst.clone(),
                    inner: v,
                },
            );
            dst
        }
        Expr::Spawn { call, .. } => {
            if let Expr::Call { func, args, .. } = call.as_ref() {
                let mut lowered = Vec::with_capacity(args.len());
                for a in args {
                    lowered.push(lower_expr_to_value(a, instrs, tmp));
                }
                let h = tmp_name(tmp);
                push(
                    instrs,
                    e.id(),
                    MirOp::Spawn {
                        handle: h.clone(),
                        func: func.clone(),
                        args: lowered,
                    },
                );
                h
            } else {
                lower_expr_to_value(call, instrs, tmp)
            }
        }
    }
}

enum BinKind {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

fn bin(
    e: &Expr,
    left: &Expr,
    right: &Expr,
    instrs: &mut Vec<MirInstr>,
    tmp: &mut u32,
    kind: BinKind,
) -> String {
    let l = lower_expr_to_value(left, instrs, tmp);
    let r = lower_expr_to_value(right, instrs, tmp);
    let dst = tmp_name(tmp);
    let op = match kind {
        BinKind::Add => MirOp::Add {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Sub => MirOp::Sub {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Mul => MirOp::Mul {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Div => MirOp::Div {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Mod => MirOp::Mod {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Eq => MirOp::Eq {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::NotEq => MirOp::NotEq {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Lt => MirOp::Lt {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::LtEq => MirOp::LtEq {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Gt => MirOp::Gt {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::GtEq => MirOp::GtEq {
            into: dst.clone(),
            left: l,
            right: r,
        },
    };
    push(instrs, e.id(), op);
    dst
}

/// `&&` (`is_and`) / `||` with short-circuit evaluation. Unlike [`bin`]
/// (which lowers both sides up front), the right-hand side lowers inside
/// the taken branch only: a `JumpIfFalse` on the left skips the RHS — and
/// all of its effects — when the left side already decides the result.
/// The merged value stays `Int` 0/1: the existing `MirOp::And`/`Or` runs
/// on the evaluated path (so truthy coercion of non-bool values is
/// unchanged) and a `Const` 0/1 lands on the skipped path. Same
/// jump/patch shape as `Stmt::If` lowering, so the interpreter, the
/// int-only JIT, and jump validation all treat it as ordinary control
/// flow.
fn short_circuit(
    e: &Expr,
    left: &Expr,
    right: &Expr,
    instrs: &mut Vec<MirInstr>,
    tmp: &mut u32,
    is_and: bool,
) -> String {
    let l = lower_expr_to_value(left, instrs, tmp);
    let dst = tmp_name(tmp);
    let jfalse = push(
        instrs,
        e.id(),
        MirOp::JumpIfFalse {
            cond: l.clone(),
            target: usize::MAX,
        },
    );
    if is_and {
        // Left truthy: the result is the truthiness of the right side.
        let r = lower_expr_to_value(right, instrs, tmp);
        push(
            instrs,
            e.id(),
            MirOp::And {
                into: dst.clone(),
                left: l,
                right: r,
            },
        );
        let jend = push(instrs, e.id(), MirOp::Jump { target: usize::MAX });
        let false_at = instrs.len();
        patch_target(instrs, jfalse, false_at);
        push(
            instrs,
            e.id(),
            MirOp::Const {
                into: dst.clone(),
                value: 0,
            },
        );
        let end_at = instrs.len();
        patch_target(instrs, jend, end_at);
    } else {
        // Left truthy: the result is decided (`1`) without the right side.
        push(
            instrs,
            e.id(),
            MirOp::Const {
                into: dst.clone(),
                value: 1,
            },
        );
        let jend = push(instrs, e.id(), MirOp::Jump { target: usize::MAX });
        let false_at = instrs.len();
        patch_target(instrs, jfalse, false_at);
        let r = lower_expr_to_value(right, instrs, tmp);
        push(
            instrs,
            e.id(),
            MirOp::Or {
                into: dst.clone(),
                left: l,
                right: r,
            },
        );
        let end_at = instrs.len();
        patch_target(instrs, jend, end_at);
    }
    dst
}

/// Lower a fully-checked program to MIR.
pub fn lower(program: &Program) -> MirModule {
    // Same module flattening as the checker (callers must check first;
    // unresolved references lower best-effort, as before).
    let (flat, _) = crate::modules::resolve(program);
    let program = &flat;
    let mut out = MirModule::default();
    for s in &program.structs {
        let mut fields = HashMap::new();
        for fld in &s.fields {
            fields.insert(fld.name.clone(), fld.ty.clone());
        }
        out.struct_fields.insert(s.name.clone(), fields);
    }
    for f in &program.functions {
        let mut instrs = Vec::new();
        let mut tmp: u32 = 0;
        let mut loops: Vec<LoopCtx> = Vec::new();
        lower_block(&f.body.stmts, &mut instrs, &mut tmp, &mut loops);
        out.functions.push(MirFunction {
            name: f.name.clone(),
            origin: f.id.clone(),
            params: f.params.iter().map(|p| p.name.clone()).collect(),
            param_tys: f.params.iter().map(|p| p.ty.clone()).collect(),
            return_ty: f.return_ty.clone(),
            instrs,
        });
        // Lambda-lifted closure bodies: one synthetic function per
        // literal (including nested ones — `collect_closures` recurses),
        // named by `closure_fn_name` (NUL-containing, unspellable, so no
        // user function can collide). Leading parameters are the captures
        // in `closure_captures` order, then the declared parameters; the
        // body lowers like any function body (`return` returns from the
        // closure, loops get a fresh stack). Each has its own `tmp`
        // counter so generated names never collide across functions.
        let mut lits = Vec::new();
        crate::closures::collect_closures(&f.body, &mut lits);
        for lit in &lits {
            let caps = crate::closures::closure_captures(&lit.params, &lit.body);
            let mut cinstrs = Vec::new();
            let mut ctmp: u32 = 0;
            let mut cloops: Vec<LoopCtx> = Vec::new();
            lower_block(&lit.body.stmts, &mut cinstrs, &mut ctmp, &mut cloops);
            let mut cparams = caps;
            cparams.extend(lit.params.iter().map(|p| p.name.clone()));
            // Captures carry no annotation (unchecked prefix); the
            // declared parameters keep theirs for boundary checks.
            let mut cparam_tys: Vec<String> =
                vec![String::new(); cparams.len().saturating_sub(lit.params.len())];
            cparam_tys.extend(lit.params.iter().map(|p| p.ty.clone()));
            out.functions.push(MirFunction {
                name: crate::closures::closure_fn_name(&lit.id),
                origin: lit.id.clone(),
                params: cparams,
                param_tys: cparam_tys,
                return_ty: lit.return_ty.clone(),
                instrs: cinstrs,
            });
        }
    }
    out
}

fn lower_block(
    stmts: &[Stmt],
    instrs: &mut Vec<MirInstr>,
    tmp: &mut u32,
    loops: &mut Vec<LoopCtx>,
) {
    for s in stmts {
        match s {
            Stmt::Let(l) => {
                if let Expr::Spawn { call, .. } = &l.value {
                    if let Expr::Call { func, args, .. } = call.as_ref() {
                        let mut lowered = Vec::with_capacity(args.len());
                        for a in args {
                            lowered.push(lower_expr_to_value(a, instrs, tmp));
                        }
                        push(
                            instrs,
                            &l.id,
                            MirOp::Spawn {
                                handle: l.name.clone(),
                                func: func.clone(),
                                args: lowered,
                            },
                        );
                        continue;
                    }
                }
                let v = lower_expr_to_value(&l.value, instrs, tmp);
                if v != l.name {
                    push(
                        instrs,
                        &l.id,
                        MirOp::Copy {
                            into: l.name.clone(),
                            from: v,
                        },
                    );
                }
            }
            Stmt::Assign(a) => {
                let v = lower_expr_to_value(&a.value, instrs, tmp);
                match &a.target {
                    AssignTarget::Var { name } => {
                        push(
                            instrs,
                            &a.id,
                            MirOp::Copy {
                                into: name.clone(),
                                from: v,
                            },
                        );
                    }
                    AssignTarget::Index { base, index } => {
                        let b = lower_expr_to_value(base, instrs, tmp);
                        let i = lower_expr_to_value(index, instrs, tmp);
                        push(
                            instrs,
                            &a.id,
                            MirOp::StoreIndex {
                                base: b,
                                index: i,
                                value: v,
                            },
                        );
                    }
                    AssignTarget::Field { base, field } => {
                        let b = lower_expr_to_value(base, instrs, tmp);
                        push(
                            instrs,
                            &a.id,
                            MirOp::FieldSet {
                                base: b,
                                field: field.clone(),
                                value: v,
                            },
                        );
                    }
                }
            }
            Stmt::Return(r) => {
                let v = lower_expr_to_value(&r.value, instrs, tmp);
                push(instrs, &r.id, MirOp::Return { value: v });
            }
            Stmt::Print(p) => {
                let v = lower_expr_to_value(&p.value, instrs, tmp);
                push(instrs, &p.id, MirOp::Print { value: v });
            }
            Stmt::Break(b) => {
                let idx = push(instrs, &b.id, MirOp::Jump { target: usize::MAX });
                if let Some(ctx) = loops.last_mut() {
                    ctx.breaks.push(idx);
                }
            }
            Stmt::Continue(c) => {
                let idx = push(instrs, &c.id, MirOp::Jump { target: usize::MAX });
                if let Some(ctx) = loops.last_mut() {
                    ctx.continues.push(idx);
                }
            }
            Stmt::If(s) => {
                let cond = lower_expr_to_value(&s.cond, instrs, tmp);
                let jfalse = push(
                    instrs,
                    &s.id,
                    MirOp::JumpIfFalse {
                        cond,
                        target: usize::MAX,
                    },
                );
                lower_block(&s.then_block.stmts, instrs, tmp, loops);
                if let Some(else_b) = &s.else_block {
                    let jend = push(instrs, &s.id, MirOp::Jump { target: usize::MAX });
                    let else_at = instrs.len();
                    patch_target(instrs, jfalse, else_at);
                    lower_block(&else_b.stmts, instrs, tmp, loops);
                    let end_at = instrs.len();
                    patch_target(instrs, jend, end_at);
                } else {
                    let end_at = instrs.len();
                    patch_target(instrs, jfalse, end_at);
                }
            }
            Stmt::While(w) => {
                let loop_start = instrs.len();
                let cond = lower_expr_to_value(&w.cond, instrs, tmp);
                let jfalse = push(
                    instrs,
                    &w.id,
                    MirOp::JumpIfFalse {
                        cond,
                        target: usize::MAX,
                    },
                );
                loops.push(LoopCtx::default());
                lower_block(&w.body.stmts, instrs, tmp, loops);
                push(instrs, &w.id, MirOp::Jump { target: loop_start });
                let end_at = instrs.len();
                let ctx = loops.pop().expect("loop ctx");
                patch_target(instrs, jfalse, end_at);
                for b in ctx.breaks {
                    patch_target(instrs, b, end_at);
                }
                for c in ctx.continues {
                    patch_target(instrs, c, loop_start);
                }
            }
            Stmt::ForRange(fr) => {
                let s = lower_expr_to_value(&fr.start, instrs, tmp);
                let e = lower_expr_to_value(&fr.end, instrs, tmp);
                push(
                    instrs,
                    &fr.id,
                    MirOp::Copy {
                        into: fr.var.clone(),
                        from: s,
                    },
                );
                let _ = e;
                // Evaluate `end` once into a hidden temp.
                let end_tmp = tmp_name(tmp);
                push(
                    instrs,
                    &fr.id,
                    MirOp::Copy {
                        into: end_tmp.clone(),
                        from: e,
                    },
                );
                let loop_start = instrs.len();
                let c = tmp_name(tmp);
                push(
                    instrs,
                    &fr.id,
                    MirOp::Lt {
                        into: c.clone(),
                        left: fr.var.clone(),
                        right: end_tmp.clone(),
                    },
                );
                let jfalse = push(
                    instrs,
                    &fr.id,
                    MirOp::JumpIfFalse {
                        cond: c,
                        target: usize::MAX,
                    },
                );
                loops.push(LoopCtx::default());
                lower_block(&fr.body.stmts, instrs, tmp, loops);
                let step_at = instrs.len();
                let one = tmp_name(tmp);
                push(
                    instrs,
                    &fr.id,
                    MirOp::Const {
                        into: one.clone(),
                        value: 1,
                    },
                );
                push(
                    instrs,
                    &fr.id,
                    MirOp::Add {
                        into: fr.var.clone(),
                        left: fr.var.clone(),
                        right: one,
                    },
                );
                push(instrs, &fr.id, MirOp::Jump { target: loop_start });
                let end_at = instrs.len();
                let ctx = loops.pop().expect("loop ctx");
                patch_target(instrs, jfalse, end_at);
                for b in ctx.breaks {
                    patch_target(instrs, b, end_at);
                }
                for c in ctx.continues {
                    patch_target(instrs, c, step_at);
                }
            }
            Stmt::ForIn(fi) => {
                let arr = lower_expr_to_value(&fi.iter, instrs, tmp);
                let idx = tmp_name(tmp);
                let len = tmp_name(tmp);
                push(
                    instrs,
                    &fi.id,
                    MirOp::Const {
                        into: idx.clone(),
                        value: 0,
                    },
                );
                let loop_start = instrs.len();
                push(
                    instrs,
                    &fi.id,
                    MirOp::Len {
                        into: len.clone(),
                        of: arr.clone(),
                    },
                );
                let c = tmp_name(tmp);
                push(
                    instrs,
                    &fi.id,
                    MirOp::Lt {
                        into: c.clone(),
                        left: idx.clone(),
                        right: len.clone(),
                    },
                );
                let jfalse = push(
                    instrs,
                    &fi.id,
                    MirOp::JumpIfFalse {
                        cond: c,
                        target: usize::MAX,
                    },
                );
                let elem = tmp_name(tmp);
                push(
                    instrs,
                    &fi.id,
                    MirOp::Index {
                        into: elem.clone(),
                        base: arr.clone(),
                        index: idx.clone(),
                    },
                );
                push(
                    instrs,
                    &fi.id,
                    MirOp::Copy {
                        into: fi.var.clone(),
                        from: elem,
                    },
                );
                loops.push(LoopCtx::default());
                lower_block(&fi.body.stmts, instrs, tmp, loops);
                let step_at = instrs.len();
                let one = tmp_name(tmp);
                push(
                    instrs,
                    &fi.id,
                    MirOp::Const {
                        into: one.clone(),
                        value: 1,
                    },
                );
                push(
                    instrs,
                    &fi.id,
                    MirOp::Add {
                        into: idx.clone(),
                        left: idx.clone(),
                        right: one,
                    },
                );
                push(instrs, &fi.id, MirOp::Jump { target: loop_start });
                let end_at = instrs.len();
                let ctx = loops.pop().expect("loop ctx");
                patch_target(instrs, jfalse, end_at);
                for b in ctx.breaks {
                    patch_target(instrs, b, end_at);
                }
                for c in ctx.continues {
                    patch_target(instrs, c, step_at);
                }
            }
            Stmt::TaskGroup(g) => {
                push(
                    instrs,
                    &g.id,
                    MirOp::EnterGroup {
                        group: g.id.clone(),
                    },
                );
                lower_block(&g.body.stmts, instrs, tmp, loops);
                push(
                    instrs,
                    &g.id,
                    MirOp::LeaveGroup {
                        group: g.id.clone(),
                    },
                );
            }
            Stmt::TryCatch(t) => {
                // Fresh instruction vectors (slice-relative targets) and a
                // fresh loop stack: `break`/`continue` inside either slice
                // can only target loops inside the same slice (crossing is
                // an `E-LOOP` at check time). The `tmp` counter stays
                // shared so generated names never collide with the outer
                // stream. Spawns lower to thread handles owned by the
                // caller's pending map at runtime, so awaiting across the
                // boundary still joins the right thread.
                let mut body = Vec::new();
                let mut body_loops = Vec::new();
                lower_block(&t.body.stmts, &mut body, tmp, &mut body_loops);
                let mut handler = Vec::new();
                let mut handler_loops = Vec::new();
                lower_block(&t.handler.stmts, &mut handler, tmp, &mut handler_loops);
                push(
                    instrs,
                    &t.id,
                    MirOp::Try {
                        code_var: t.var.clone(),
                        body,
                        handler,
                    },
                );
            }
            Stmt::Expr(e) => {
                let _ = lower_expr_to_value(e, instrs, tmp);
            }
        }
    }
}

fn patch_target(instrs: &mut [MirInstr], idx: usize, target: usize) {
    match &mut instrs[idx].op {
        MirOp::JumpIfFalse { target: t, .. } | MirOp::Jump { target: t } => *t = target,
        _ => {}
    }
}
