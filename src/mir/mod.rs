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

use crate::ast::{AssignTarget, Expr, NodeId, Program, Stmt};

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
}

/// One lowered function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirFunction {
    pub name: String,
    pub origin: NodeId,
    pub params: Vec<String>,
    pub instrs: Vec<MirInstr>,
}

/// Whole lowered module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirModule {
    pub functions: Vec<MirFunction>,
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
            // same-named outer bindings and restore them per arm.
            let mut names: Vec<String> = Vec::new();
            for arm in arms.iter() {
                for b in arm.bindings.iter() {
                    if !names.contains(b) {
                        names.push(b.clone());
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
            let mut pending_jfalse: Option<usize> = None;
            let mut end_jumps: Vec<usize> = Vec::new();
            for arm in arms.iter() {
                if let Some(jf) = pending_jfalse.take() {
                    let target = instrs.len();
                    patch_target(instrs, jf, target);
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
                    pending_jfalse = Some(push(
                        instrs,
                        e.id(),
                        MirOp::JumpIfFalse {
                            cond: cmp,
                            target: usize::MAX,
                        },
                    ));
                }
                for (i, b) in arm.bindings.iter().enumerate() {
                    push(
                        instrs,
                        &arm.id,
                        MirOp::FieldGet {
                            into: b.clone(),
                            base: s.clone(),
                            field: format!("f{i}"),
                        },
                    );
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
            // No arm matched (unreachable when HIR accepted the program):
            // force a loud runtime error, never a silent default value.
            if let Some(jf) = pending_jfalse.take() {
                let target = instrs.len();
                patch_target(instrs, jf, target);
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
        Expr::And { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::And),
        Expr::Or { left, right, .. } => bin(e, left, right, instrs, tmp, BinKind::Or),
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
    And,
    Or,
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
        BinKind::And => MirOp::And {
            into: dst.clone(),
            left: l,
            right: r,
        },
        BinKind::Or => MirOp::Or {
            into: dst.clone(),
            left: l,
            right: r,
        },
    };
    push(instrs, e.id(), op);
    dst
}

/// Lower a fully-checked program to MIR.
pub fn lower(program: &Program) -> MirModule {
    // Same module flattening as the checker (callers must check first;
    // unresolved references lower best-effort, as before).
    let (flat, _) = crate::modules::resolve(program);
    let program = &flat;
    let mut out = MirModule::default();
    for f in &program.functions {
        let mut instrs = Vec::new();
        let mut tmp: u32 = 0;
        let mut loops: Vec<LoopCtx> = Vec::new();
        lower_block(&f.body.stmts, &mut instrs, &mut tmp, &mut loops);
        out.functions.push(MirFunction {
            name: f.name.clone(),
            origin: f.id.clone(),
            params: f.params.iter().map(|p| p.name.clone()).collect(),
            instrs,
        });
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
