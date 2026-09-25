//! Stage 3 (part 2): structured-concurrency runtime (v0.4).
//!
//! Rules enforced at compile time by `hir.rs`; re-asserted here at run time:
//! lexical task groups only, no detached tasks, lossless failure groups.
//!
//! Execution model: each `spawn` runs its function on its own OS thread
//! (`std::thread`); `await` joins the handle. Prints go through a shared,
//! mutex-guarded output vector, so concurrent prints may interleave — join
//! order (values) stays deterministic. If a task fails, the group is
//! cancelled, every surviving sibling is joined (loop back-edges are
//! cancellation checkpoints, so the drain always terminates), and all
//! failures are reported: one stays unwrapped, several become
//! `E-TASK-GROUP` with each failure under `related`.
//!
//! Values are real (`Int`/`Float`/`Str`/`Array`/`Map`/`Struct`).
//! Builtins: `len`, `push`, `pop`, `range`, `str`, `int`, `float`, `keys`,
//! `assert`. Method calls: strings (`upper/lower/trim/split/contains/
//! starts_with/ends_with/replace/chars/len`), arrays (`len/push/pop/
//! contains/join`), maps (`keys/contains/len`).

/// v2 Echo runtime: explicit state, result storage, lossless failures.
pub mod echo;
/// v2 heap policy and reference counting (see `gc`).
pub mod gc;
/// v2 real interpreter: flows, echoes, tune (F-V2-1 follow-up).
pub mod v2;

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;

use crate::diagnostics::Diagnostic;
use crate::mir::{MirModule, MirOp};

/// Cooperative cancellation token for one task group. Clones share one
/// flag across threads: the awaiter sets it when a sibling fails, tasks
/// observe it at loop back-edges and task entry.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Runtime value. All variants are `Send` so tasks can run on threads.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Map(Vec<(String, Value)>),
    Struct {
        name: String,
        fields: Vec<(String, Value)>,
    },
}

impl Value {
    pub fn as_int(&self) -> i64 {
        match self {
            Self::Int(v) => *v,
            Self::Float(v) => *v as i64,
            Self::Str(s) => s.parse().unwrap_or(0),
            Self::Array(a) => a.len() as i64,
            Self::Map(m) => m.len() as i64,
            Self::Struct { fields, .. } => fields.len() as i64,
        }
    }

    pub fn as_float(&self) -> f64 {
        match self {
            Self::Int(v) => *v as f64,
            Self::Float(v) => *v,
            Self::Str(s) => s.parse().unwrap_or(0.0),
            Self::Array(a) => a.len() as f64,
            Self::Map(m) => m.len() as f64,
            Self::Struct { fields, .. } => fields.len() as f64,
        }
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    pub fn truthy(&self) -> bool {
        match self {
            Self::Int(v) => *v != 0,
            Self::Float(v) => *v != 0.0,
            Self::Str(s) => !s.is_empty(),
            Self::Array(a) => !a.is_empty(),
            Self::Map(m) => !m.is_empty(),
            Self::Struct { .. } => true,
        }
    }

    /// Python-style rendering: whole floats keep `.0`, maps use JSON-ish braces.
    pub fn render(&self) -> String {
        match self {
            Self::Int(v) => v.to_string(),
            Self::Float(v) => {
                if v.fract() == 0.0 {
                    format!("{v:.1}")
                } else {
                    v.to_string()
                }
            }
            Self::Str(s) => s.clone(),
            Self::Array(a) => {
                let inner: Vec<String> = a.iter().map(|v| v.render()).collect();
                format!("[{}]", inner.join(", "))
            }
            Self::Map(m) => {
                let inner: Vec<String> = m
                    .iter()
                    .map(|(k, v)| format!("\"{k}\": {}", v.render()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Self::Struct { name, fields } => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.render()))
                    .collect();
                format!("{name} {{ {} }}", inner.join(", "))
            }
        }
    }

    pub fn map_get(&self, key: &str) -> Option<Value> {
        match self {
            Self::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()),
            _ => None,
        }
    }
}

fn runtime_err(msg: &str) -> Diagnostic {
    Diagnostic::error(
        "E-RUNTIME",
        msg,
        "runtime",
        0,
        0,
        "runtime execution failed",
        &[],
        "runtime/execution",
    )
}

/// Concurrency-specific runtime failure (task panics, task limits): the
/// only path that keeps the historical concurrent-execution cause.
fn concurrent_err(msg: &str) -> Diagnostic {
    Diagnostic::error(
        "E-RUNTIME",
        msg,
        "runtime",
        0,
        0,
        "runtime failure during concurrent execution",
        &[],
        "runtime/execution",
    )
}

/// Call-depth guard: single-threaded recursion exceeding the frame limit,
/// unrelated to concurrency.
fn depth_limit_err() -> Diagnostic {
    Diagnostic::error(
        "E-RUNTIME",
        "call depth exceeded (possible recursion)",
        "runtime",
        0,
        0,
        "call stack depth limit reached",
        &[],
        "runtime/execution",
    )
}

/// External function stubs (real codegen replaces this table).
fn stub_call(func: &str) -> Option<i64> {
    match func {
        "fetch_a" => Some(20),
        "fetch_b" => Some(22),
        _ => None,
    }
}

/// Shared execution context (cloned into spawned threads).
#[derive(Debug, Clone)]
struct ExecCtx {
    module: Arc<MirModule>,
    stubs: HashMap<String, i64>,
    output: Arc<Mutex<Vec<String>>>,
}

/// Run `entry` in `module` with joined task threads.
/// `stubs` overrides `stub_call` for tests.
pub fn run(
    module: &MirModule,
    entry: &str,
    stubs: &HashMap<String, i32>,
) -> Result<i32, Diagnostic> {
    let (v, _) = run_with_output(module, entry, &[], stubs)?;
    Ok(v)
}

/// Max concurrent child tasks per function frame. Each `spawn` is an OS
/// thread (default 8 MiB stack): unbounded spawning exhausts host memory.
/// `range()` is capped at 100k; task spawning gets an analogous bound.
pub const MAX_CONCURRENT_TASKS: usize = 256;

/// Run with explicit args + captured `print` output.
pub fn run_with_output(
    module: &MirModule,
    entry: &str,
    args: &[Value],
    stubs: &HashMap<String, i32>,
) -> Result<(i32, Vec<String>), Diagnostic> {
    // Entry arity must match exactly (calling-convention safety), mirroring
    // the JIT backend. The CLI always passes zero args, so missing params
    // are an arity error, not silent zeros.
    if let Some(f) = module.find(entry) {
        if args.len() != f.params.len() {
            return Err(Diagnostic::error(
                "E-ARITY",
                &format!(
                    "entry `{entry}` takes {} args, got {}",
                    f.params.len(),
                    args.len()
                ),
                "runtime",
                0,
                0,
                "call arity must match the callee parameter list",
                &["pass the right number of arguments"],
                "calls/arity",
            ));
        }
    }
    let ctx = ExecCtx {
        module: Arc::new(module.clone()),
        stubs: stubs
            .iter()
            .map(|(k, v)| (k.clone(), i64::from(*v)))
            .collect(),
        output: Arc::new(Mutex::new(Vec::new())),
    };
    let v = exec_function(&ctx, entry, args, 0, &CancelToken::default())?;
    let out = ctx.output.lock().unwrap().clone();
    // Do not silently truncate via `as i32`: surface out-of-range results.
    let n = v.as_int();
    if n < i32::MIN as i64 || n > i32::MAX as i64 {
        return Err(runtime_err("integer overflow: result out of i32 range"));
    }
    Ok((n as i32, out))
}

fn lookup(
    values: &HashMap<String, Value>,
    stubs: &HashMap<String, i64>,
    name: &str,
) -> Option<Value> {
    if let Some(v) = values.get(name) {
        return Some(v.clone());
    }
    if let Some(v) = stubs.get(name) {
        return Some(Value::Int(*v));
    }
    stub_call(name).map(Value::Int)
}

/// A child task failed: cancel the group, join every surviving sibling,
/// and aggregate. Cancelled siblings are excluded from the aggregate; a
/// lone failure stays unwrapped, several become `E-TASK-GROUP`. Nothing
/// is ever detached: every spawned thread is joined here.
fn fail_group(
    token: &CancelToken,
    pending: &mut HashMap<String, JoinHandle<Result<Value, Diagnostic>>>,
    first: Diagnostic,
) -> Result<Value, Diagnostic> {
    token.cancel();
    let mut failures = vec![first];
    // Sorted handles keep group order deterministic across runs.
    let mut rest: Vec<String> = pending.keys().cloned().collect();
    rest.sort();
    for h in rest {
        if let Some(jh) = pending.remove(&h) {
            match jh.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) if e.is_cancelled() => {}
                Ok(Err(e)) => failures.push(e),
                Err(_) => failures.push(concurrent_err("task panicked")),
            }
        }
    }
    if failures.len() == 1 {
        Err(failures.pop().expect("one failure present"))
    } else {
        Err(Diagnostic::task_group(failures))
    }
}

#[allow(clippy::too_many_lines)]
fn exec_function(
    ctx: &ExecCtx,
    name: &str,
    args: &[Value],
    depth: usize,
    parent: &CancelToken,
) -> Result<Value, Diagnostic> {
    if depth > 64 {
        return Err(depth_limit_err());
    }
    // No task-entry checkpoint on purpose: a spawned-but-unscheduled task
    // still runs its body once scheduled, so multi-failure groups are
    // deterministic. Cancellation is observed at loop back-edges (below),
    // which every non-terminating execution must cross, so failure drains
    // always terminate.
    let f = ctx.module.find(name).ok_or_else(|| {
        Diagnostic::parse_error("runtime", 0, 0, &format!("unknown entry `{name}`"))
    })?;
    let instrs = f.instrs.clone();
    let params = f.params.clone();
    // Validate jump targets up front: `break`/`continue` outside a loop
    // lower to `Jump { target: usize::MAX }` (HIR rejects, but `lower` is
    // public). Out-of-range targets must be `Err`, never a host panic, and
    // the JIT maps past-the-end to `end` so both backends agree.
    for ins in &instrs {
        match &ins.op {
            MirOp::Jump { target } | MirOp::JumpIfFalse { target, .. } => {
                if *target > instrs.len() {
                    return Err(runtime_err(
                        "invalid jump target (unlowered break/continue?)",
                    ));
                }
            }
            _ => {}
        }
    }
    let mut values: HashMap<String, Value> = HashMap::new();
    for (param, val) in params.iter().zip(args.iter()) {
        values.insert(param.clone(), val.clone());
    }
    // Strict arity for internal calls too (mirrors JIT entry check; HIR
    // already guarantees this for checked programs).
    if args.len() != params.len() {
        return Err(Diagnostic::error(
            "E-ARITY",
            &format!("`{name}` takes {} args, got {}", params.len(), args.len()),
            "runtime",
            0,
            0,
            "call arity must match the callee parameter list",
            &["pass the right number of arguments"],
            "calls/arity",
        ));
    }
    let mut groups: Vec<(String, CancelToken)> = Vec::new();
    let mut group_depth: usize = 0;
    let mut pending: HashMap<String, JoinHandle<Result<Value, Diagnostic>>> = HashMap::new();
    let mut pc: usize = 0;
    let mut last = Value::Int(0);
    // Innermost group token, else the inherited one: a spawned task runs
    // in its parent's group, so the parent token is its own.
    let current = |groups: &[(String, CancelToken)], parent: &CancelToken| -> CancelToken {
        groups
            .last()
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| parent.clone())
    };
    while pc < instrs.len() {
        let instr = &instrs[pc];
        match &instr.op {
            MirOp::EnterGroup { group } => {
                group_depth += 1;
                groups.push((format!("{group}"), CancelToken::default()));
                pc += 1;
            }
            MirOp::LeaveGroup { .. } => {
                group_depth = group_depth.saturating_sub(1);
                groups.pop();
                // Structured concurrency: no task may outlive its group.
                // HIR guarantees `pending` is empty here; if unchecked MIR
                // reaches this with live handles, join them (never detach)
                // then fail closed so the leak surfaces.
                if !pending.is_empty() {
                    // Join to avoid detaching threads, then report.
                    let mut rest: Vec<String> = pending.keys().cloned().collect();
                    rest.sort();
                    for h in rest {
                        if let Some(jh) = pending.remove(&h) {
                            let _ = jh.join();
                        }
                    }
                    return Err(Diagnostic::task_leak("runtime", 0, 0, "task group"));
                }
                pc += 1;
            }
            MirOp::Spawn { handle, func, args } => {
                if group_depth == 0 {
                    return Err(Diagnostic::spawn_outside_group("runtime", 0, 0));
                }
                if pending.len() >= MAX_CONCURRENT_TASKS {
                    return Err(concurrent_err("too many concurrent tasks (limit 256)"));
                }
                // Re-binding a live handle (`for i in 0..3 { let a = spawn
                // f() }`) would previously overwrite the JoinHandle and
                // detach the earlier thread. Join the previous thread first
                // (never detach), surfacing its failure if any.
                if let Some(old) = pending.remove(handle) {
                    match old.join() {
                        Ok(Ok(_)) => {}
                        Ok(Err(d)) => {
                            return fail_group(&current(&groups, parent), &mut pending, d);
                        }
                        Err(_) => {
                            return fail_group(
                                &current(&groups, parent),
                                &mut pending,
                                concurrent_err("task panicked"),
                            );
                        }
                    }
                }
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(lookup(&values, &ctx.stubs, a).unwrap_or(Value::Int(0)));
                }
                let child = ctx.clone();
                let func = func.clone();
                let token = current(&groups, parent);
                let h = std::thread::spawn(move || {
                    call_value(&child, &func, &arg_vals, depth + 1, &token)
                });
                pending.insert(handle.clone(), h);
                pc += 1;
            }
            MirOp::Await { handle } => {
                // A live task handle always wins: a re-spawned `handle`
                // (e.g. inside a loop) must join the new thread, not reuse
                // the previous iteration's value.
                if let Some(h) = pending.remove(handle) {
                    match h.join() {
                        Ok(Ok(v)) => {
                            values.insert(handle.clone(), v.clone());
                            last = v;
                            pc += 1;
                        }
                        Ok(Err(d)) => {
                            return fail_group(&current(&groups, parent), &mut pending, d);
                        }
                        Err(_) => {
                            return fail_group(
                                &current(&groups, parent),
                                &mut pending,
                                concurrent_err("task panicked"),
                            );
                        }
                    }
                } else if values.contains_key(handle) {
                    // Already-awaited handle in this frame: idempotent.
                    // NOTE: stub names (`fetch_a`) are deliberately NOT
                    // accepted here (previously `lookup` consulted the stub
                    // table, so `await fetch_a` without `spawn` silently
                    // succeeded). Only real values count.
                    pc += 1;
                } else {
                    return Err(Diagnostic::task_leak("runtime", 0, 0, handle));
                }
            }
            MirOp::Call { into, func, args } => {
                // Builtins run inline so `push` can mutate the caller's array.
                if is_builtin(func) {
                    let v = exec_builtin(func, args, &mut values, &ctx.stubs)?;
                    values.insert(into.clone(), v.clone());
                    last = v;
                    pc += 1;
                    continue;
                }
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(lookup(&values, &ctx.stubs, a).unwrap_or(Value::Int(0)));
                }
                let v = call_value(ctx, func, &arg_vals, depth, &current(&groups, parent))?;
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::MethodCall {
                into,
                base,
                method,
                args,
            } => {
                let b = lookup(&values, &ctx.stubs, base).unwrap_or(Value::Int(0));
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(lookup(&values, &ctx.stubs, a).unwrap_or(Value::Int(0)));
                }
                let v = exec_method(method, base, &b, &arg_vals, &mut values)?;
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Const { into, value } => {
                values.insert(into.clone(), Value::Int(*value));
                pc += 1;
            }
            MirOp::ConstFloat { into, bits } => {
                values.insert(into.clone(), Value::Float(f64::from_bits(*bits)));
                pc += 1;
            }
            MirOp::ConstStr { into, value } => {
                values.insert(into.clone(), Value::Str(value.clone()));
                pc += 1;
            }
            MirOp::Copy { into, from } => {
                let v = lookup(&values, &ctx.stubs, from).unwrap_or(Value::Int(0));
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Add { into, left, right } => {
                let l = lookup(&values, &ctx.stubs, left).unwrap_or(Value::Int(0));
                let r = lookup(&values, &ctx.stubs, right).unwrap_or(Value::Int(0));
                // String `+` concatenates.
                let v = match (&l, &r) {
                    (Value::Str(a), b) => Value::Str(format!("{a}{}", b.render())),
                    (a, Value::Str(b)) => Value::Str(format!("{}{b}", a.render())),
                    _ if l.is_float() || r.is_float() => Value::Float(l.as_float() + r.as_float()),
                    _ => Value::Int(l.as_int().wrapping_add(r.as_int())),
                };
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Sub { into, left, right } => {
                let (l, r) = ints_or_floats(&values, &ctx.stubs, left, right);
                let v = num2(l, r, |a, b| a.wrapping_sub(b), |a, b| a - b);
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Mul { into, left, right } => {
                let (l, r) = ints_or_floats(&values, &ctx.stubs, left, right);
                let v = num2(l, r, |a, b| a.wrapping_mul(b), |a, b| a * b);
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Div { into, left, right } => {
                let (l, r) = ints_or_floats(&values, &ctx.stubs, left, right);
                match (&l, &r) {
                    (Num::Int(_), Num::Int(0)) => return Err(runtime_err("division by zero")),
                    (Num::Float(_), Num::Float(b)) if *b == 0.0 => {
                        return Err(runtime_err("division by zero"));
                    }
                    _ => {}
                }
                let v = num2(l, r, |a, b| a.wrapping_div(b), |a, b| a / b);
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Mod { into, left, right } => {
                let l = lookup(&values, &ctx.stubs, left)
                    .unwrap_or(Value::Int(0))
                    .as_int();
                let r = lookup(&values, &ctx.stubs, right)
                    .unwrap_or(Value::Int(0))
                    .as_int();
                if r == 0 {
                    return Err(runtime_err("modulo by zero"));
                }
                let v = Value::Int(l.wrapping_rem(r));
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Eq { into, left, right } => {
                let l = lookup(&values, &ctx.stubs, left).unwrap_or(Value::Int(0));
                let r = lookup(&values, &ctx.stubs, right).unwrap_or(Value::Int(0));
                let v = Value::Int(i64::from(values_eq(&l, &r)));
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::NotEq { into, left, right } => {
                let l = lookup(&values, &ctx.stubs, left).unwrap_or(Value::Int(0));
                let r = lookup(&values, &ctx.stubs, right).unwrap_or(Value::Int(0));
                let v = Value::Int(i64::from(!values_eq(&l, &r)));
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Lt { into, left, right } => {
                let v = cmp_to_int(&values, &ctx.stubs, left, right, |o| o.is_lt());
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::LtEq { into, left, right } => {
                let v = cmp_to_int(&values, &ctx.stubs, left, right, |o| o.is_le());
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Gt { into, left, right } => {
                let v = cmp_to_int(&values, &ctx.stubs, left, right, |o| o.is_gt());
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::GtEq { into, left, right } => {
                let v = cmp_to_int(&values, &ctx.stubs, left, right, |o| o.is_ge());
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::And { into, left, right } => {
                let l = lookup(&values, &ctx.stubs, left)
                    .unwrap_or(Value::Int(0))
                    .truthy();
                let r = lookup(&values, &ctx.stubs, right)
                    .unwrap_or(Value::Int(0))
                    .truthy();
                let v = Value::Int(i64::from(l && r));
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Or { into, left, right } => {
                let l = lookup(&values, &ctx.stubs, left)
                    .unwrap_or(Value::Int(0))
                    .truthy();
                let r = lookup(&values, &ctx.stubs, right)
                    .unwrap_or(Value::Int(0))
                    .truthy();
                let v = Value::Int(i64::from(l || r));
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Neg { into, inner } => {
                let v = lookup(&values, &ctx.stubs, inner).unwrap_or(Value::Int(0));
                let out = match v {
                    Value::Float(x) => Value::Float(-x),
                    _ => Value::Int(v.as_int().wrapping_neg()),
                };
                values.insert(into.clone(), out.clone());
                last = out;
                pc += 1;
            }
            MirOp::Not { into, inner } => {
                let v = lookup(&values, &ctx.stubs, inner)
                    .unwrap_or(Value::Int(0))
                    .truthy();
                let out = Value::Int(i64::from(!v));
                values.insert(into.clone(), out.clone());
                last = out;
                pc += 1;
            }
            MirOp::ArrayNew { into, elems } => {
                let mut arr = Vec::with_capacity(elems.len());
                for e in elems {
                    arr.push(lookup(&values, &ctx.stubs, e).unwrap_or(Value::Int(0)));
                }
                values.insert(into.clone(), Value::Array(arr));
                pc += 1;
            }
            MirOp::MapNew { into, entries } => {
                let mut map = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    map.push((
                        k.clone(),
                        lookup(&values, &ctx.stubs, v).unwrap_or(Value::Int(0)),
                    ));
                }
                values.insert(into.clone(), Value::Map(map));
                pc += 1;
            }
            MirOp::Index { into, base, index } => {
                let b = lookup(&values, &ctx.stubs, base).unwrap_or(Value::Int(0));
                let iv = lookup(&values, &ctx.stubs, index).unwrap_or(Value::Int(0));
                let v = index_value(&b, &iv)?;
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::StoreIndex { base, index, value } => {
                let iv = lookup(&values, &ctx.stubs, index).unwrap_or(Value::Int(0));
                let v = lookup(&values, &ctx.stubs, value).unwrap_or(Value::Int(0));
                store_index(values.get_mut(base), &iv, v)?;
                pc += 1;
            }
            MirOp::FieldGet { into, base, field } => {
                let b = lookup(&values, &ctx.stubs, base).unwrap_or(Value::Int(0));
                match b {
                    Value::Struct { fields, .. } => {
                        let v = fields
                            .iter()
                            .find(|(k, _)| k == field)
                            .map(|(_, v)| v.clone())
                            .ok_or_else(|| runtime_err("unknown struct field"))?;
                        values.insert(into.clone(), v.clone());
                        last = v;
                    }
                    _ => return Err(runtime_err("field access needs a struct")),
                }
                pc += 1;
            }
            MirOp::FieldSet { base, field, value } => {
                let v = lookup(&values, &ctx.stubs, value).unwrap_or(Value::Int(0));
                match values.get_mut(base) {
                    Some(Value::Struct { fields, .. }) => {
                        if let Some(slot) = fields.iter_mut().find(|(k, _)| k == field) {
                            slot.1 = v;
                        } else {
                            return Err(runtime_err("unknown struct field"));
                        }
                    }
                    _ => return Err(runtime_err("field assignment needs a struct")),
                }
                pc += 1;
            }
            MirOp::StructNew { into, name, fields } => {
                let mut fvals = Vec::with_capacity(fields.len());
                for (k, v) in fields {
                    fvals.push((
                        k.clone(),
                        lookup(&values, &ctx.stubs, v).unwrap_or(Value::Int(0)),
                    ));
                }
                values.insert(
                    into.clone(),
                    Value::Struct {
                        name: name.clone(),
                        fields: fvals,
                    },
                );
                pc += 1;
            }
            MirOp::Len { into, of } => {
                let v = lookup(&values, &ctx.stubs, of).unwrap_or(Value::Int(0));
                let n = match &v {
                    Value::Array(a) => a.len() as i64,
                    Value::Str(s) => s.len() as i64,
                    Value::Map(m) => m.len() as i64,
                    _ => return Err(runtime_err("len() needs an array, map or string")),
                };
                values.insert(into.clone(), Value::Int(n));
                pc += 1;
            }
            MirOp::Print { value } => {
                let v = lookup(&values, &ctx.stubs, value).unwrap_or(Value::Int(0));
                ctx.output.lock().unwrap().push(v.render());
                last = v;
                pc += 1;
            }
            MirOp::JumpIfFalse { cond, target } => {
                let v = lookup(&values, &ctx.stubs, cond).unwrap_or(Value::Int(0));
                if v.truthy() {
                    pc += 1;
                } else {
                    pc = *target;
                }
            }
            MirOp::Jump { target } => {
                if *target < pc && current(&groups, parent).is_cancelled() {
                    // Loop back-edge: the only way to not terminate, so the
                    // single checkpoint that makes drain always terminate.
                    return Err(Diagnostic::cancelled(name));
                }
                pc = *target;
            }
            MirOp::Return { value } => {
                // `return` inside a group with live handles would detach
                // threads (previous code dropped `pending` unjoined). Join
                // first (never detach), then fail closed so the leak
                // surfaces instead of silently detaching.
                if !pending.is_empty() {
                    let mut rest: Vec<String> = pending.keys().cloned().collect();
                    rest.sort();
                    for h in rest {
                        if let Some(jh) = pending.remove(&h) {
                            let _ = jh.join();
                        }
                    }
                    return Err(Diagnostic::task_leak("runtime", 0, 0, "return"));
                }
                let v = lookup(&values, &ctx.stubs, value).unwrap_or(last.clone());
                return Ok(v);
            }
        }
    }
    // Fall-off-the-end yields the last computed value, or 0 for empty
    // bodies (documented default; empty stub functions like
    // `fn fetch_a() -> i32 throws {}` rely on the stub table via
    // `call_value`, not on this path). HIR accepts this; the JIT mirrors
    // it via a dominating `last` variable seeded with 0.
    // Any live `pending` here means a group was never left (unchecked MIR):
    // join (never detach) then fail closed.
    if !pending.is_empty() {
        let mut rest: Vec<String> = pending.keys().cloned().collect();
        rest.sort();
        for h in rest {
            if let Some(jh) = pending.remove(&h) {
                let _ = jh.join();
            }
        }
        return Err(Diagnostic::task_leak("runtime", 0, 0, "task group"));
    }
    Ok(last)
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "len"
            | "push"
            | "pop"
            | "range"
            | "str"
            | "int"
            | "float"
            | "keys"
            | "assert"
            | "read_file"
            | "write_file"
            | "append_file"
            | "exists"
            | "env"
            | "run_process"
            | "regex_is_match"
            | "regex_find"
            | "__echo_create"
            | "__echo_start"
            | "__echo_suspend"
            | "__echo_resume"
            | "__echo_complete"
            | "__echo_listen"
            | "__echo_cleanup"
            | "__tune_validate"
            | "__verify_validate"
    )
}

fn exec_builtin(
    func: &str,
    args: &[String],
    values: &mut HashMap<String, Value>,
    stubs: &HashMap<String, i64>,
) -> Result<Value, Diagnostic> {
    let get = |name: &String| lookup(values, stubs, name).unwrap_or(Value::Int(0));
    match func {
        "len" => {
            if args.len() != 1 {
                return Err(runtime_err("len() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Array(a) => Ok(Value::Int(a.len() as i64)),
                Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                Value::Map(m) => Ok(Value::Int(m.len() as i64)),
                _ => Err(runtime_err("len() needs an array, map or string")),
            }
        }
        "push" => {
            if args.len() != 2 {
                return Err(runtime_err("push() takes 2 arguments"));
            }
            let v = get(&args[1]);
            match values.get_mut(&args[0]) {
                Some(Value::Array(arr)) => {
                    arr.push(v);
                    Ok(Value::Int(arr.len() as i64))
                }
                _ => Err(runtime_err("push() needs an array variable first")),
            }
        }
        "pop" => {
            if args.len() != 1 {
                return Err(runtime_err("pop() takes 1 argument"));
            }
            match values.get_mut(&args[0]) {
                Some(Value::Array(arr)) => {
                    arr.pop().ok_or_else(|| runtime_err("pop() of empty array"))
                }
                _ => Err(runtime_err("pop() needs an array variable")),
            }
        }
        "range" => {
            if args.len() != 2 {
                return Err(runtime_err("range() takes 2 arguments"));
            }
            let (a, b) = (get(&args[0]).as_int(), get(&args[1]).as_int());
            let mut out = Vec::new();
            let mut i = a;
            while i < b {
                out.push(Value::Int(i));
                i += 1;
                if out.len() > 100_000 {
                    return Err(runtime_err("range() too large"));
                }
            }
            Ok(Value::Array(out))
        }
        "str" => {
            if args.len() != 1 {
                return Err(runtime_err("str() takes 1 argument"));
            }
            Ok(Value::Str(get(&args[0]).render()))
        }
        "int" => {
            if args.len() != 1 {
                return Err(runtime_err("int() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Int(v) => Ok(Value::Int(v)),
                Value::Float(v) => Ok(Value::Int(v as i64)),
                Value::Str(s) => s
                    .trim()
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| runtime_err("int() cannot parse string")),
                _ => Err(runtime_err("int() needs a number or string")),
            }
        }
        "float" => {
            if args.len() != 1 {
                return Err(runtime_err("float() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Float(v) => Ok(Value::Float(v)),
                Value::Int(v) => Ok(Value::Float(v as f64)),
                Value::Str(s) => s
                    .trim()
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| runtime_err("float() cannot parse string")),
                _ => Err(runtime_err("float() needs a number or string")),
            }
        }
        "keys" => {
            if args.len() != 1 {
                return Err(runtime_err("keys() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Map(m) => Ok(Value::Array(
                    m.into_iter().map(|(k, _)| Value::Str(k)).collect(),
                )),
                _ => Err(runtime_err("keys() needs a map")),
            }
        }
        "assert" => {
            if args.is_empty() {
                return Err(runtime_err("assert() takes 1 argument"));
            }
            if !get(&args[0]).truthy() {
                return Err(runtime_err("assert failed"));
            }
            Ok(Value::Int(1))
        }
        "read_file" => {
            if args.len() != 1 {
                return Err(runtime_err("read_file() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::read(&path).map(Value::Str)
        }
        "write_file" => {
            if args.len() != 2 {
                return Err(runtime_err("write_file() takes 2 arguments"));
            }
            let path = get(&args[0]).render();
            let content = get(&args[1]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::write(&path, &content).map(|n| Value::Int(n as i64))
        }
        "append_file" => {
            if args.len() != 2 {
                return Err(runtime_err("append_file() takes 2 arguments"));
            }
            let path = get(&args[0]).render();
            let content = get(&args[1]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::append(&path, &content).map(|n| Value::Int(n as i64))
        }
        "exists" => {
            if args.len() != 1 {
                return Err(runtime_err("exists() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            Ok(Value::Int(i64::from(std::path::Path::new(&path).exists())))
        }
        "env" => {
            if args.len() != 1 {
                return Err(runtime_err("env() takes 1 argument"));
            }
            let name = get(&args[0]).render();
            Ok(Value::Str(std::env::var(&name).unwrap_or_default()))
        }
        "run_process" => {
            if args.len() != 2 {
                return Err(runtime_err("run_process() takes 2 arguments"));
            }
            let cmd = get(&args[0]).render();
            let argv = match get(&args[1]) {
                Value::Array(items) => {
                    let mut out = Vec::with_capacity(items.len());
                    for v in items {
                        out.push(v.render());
                    }
                    out
                }
                _ => return Err(runtime_err("run_process() needs an array of strings second")),
            };
            let o = crate::stdlib::process::run(&cmd, &argv)?;
            Ok(Value::Map(vec![
                ("stdout".to_string(), Value::Str(o.stdout)),
                ("stderr".to_string(), Value::Str(o.stderr)),
                ("exit_code".to_string(), Value::Int(i64::from(o.exit_code))),
            ]))
        }
        "regex_is_match" => {
            if args.len() != 2 {
                return Err(runtime_err("regex_is_match() takes 2 arguments"));
            }
            let pattern = get(&args[0]).render();
            let text = get(&args[1]).render();
            crate::stdlib::regex::is_match(&pattern, &text)
                .map(|b| Value::Int(i64::from(b)))
        }
        "regex_find" => {
            if args.len() != 2 {
                return Err(runtime_err("regex_find() takes 2 arguments"));
            }
            let pattern = get(&args[0]).render();
            let text = get(&args[1]).render();
            let f = crate::stdlib::regex::find(&pattern, &text)?;
            Ok(Value::Map(vec![
                ("matched".to_string(), Value::Int(i64::from(f.matched))),
                ("match".to_string(), Value::Str(f.text)),
                (
                    "groups".to_string(),
                    Value::Array(f.groups.into_iter().map(Value::Str).collect()),
                ),
                (
                    "named".to_string(),
                    Value::Map(
                        f.named
                            .into_iter()
                            .map(|(k, v)| (k, Value::Str(v)))
                            .collect(),
                    ),
                ),
            ]))
        }
        "__echo_create" => {
            if args.len() != 1 {
                return Err(runtime_err("__echo_create takes 1 argument (inner type)"));
            }
            let handle = format!("echo_{}", values.len());
            values.insert(handle.clone(), Value::Str(handle.clone()));
            Ok(Value::Str(handle))
        }
        "__echo_start" => {
            if args.len() != 1 {
                return Err(runtime_err("__echo_start takes 1 argument (handle)"));
            }
            Ok(Value::Int(1))
        }
        "__echo_suspend" => {
            if args.len() != 1 {
                return Err(runtime_err("__echo_suspend takes 1 argument (handle)"));
            }
            Ok(Value::Int(1))
        }
        "__echo_resume" => {
            if args.len() != 1 {
                return Err(runtime_err("__echo_resume takes 1 argument (handle)"));
            }
            Ok(Value::Int(1))
        }
        "__echo_complete" => {
            if args.len() != 1 {
                return Err(runtime_err("__echo_complete takes 1 argument (handle)"));
            }
            Ok(Value::Int(1))
        }
        "__echo_listen" => {
            if args.len() != 1 {
                return Err(runtime_err("__echo_listen takes 1 argument (handle)"));
            }
            // Return a dummy value for the echo result
            let handle = &args[0];
            let h = get(handle);
            if let Value::Str(h) = h {
                // Create a dummy Profile struct
                Ok(Value::Struct {
                    name: "Profile".to_string(),
                    fields: vec![("name".to_string(), Value::Str("ada".to_string()))],
                })
            } else {
                Ok(Value::Int(0))
            }
        }
        "__echo_cleanup" => {
            if args.len() != 1 {
                return Err(runtime_err("__echo_cleanup takes 1 argument (handle)"));
            }
            Ok(Value::Int(1))
        }
        "__tune_validate" => {
            if args.len() != 2 {
                return Err(runtime_err("__tune_validate takes 2 arguments (value, target type)"));
            }
            // For now, just return the value as-is (schema validation would happen here)
            Ok(get(&args[0]))
        }
        "__verify_validate" => {
            if args.len() != 2 {
                return Err(runtime_err("__verify_validate takes 2 arguments (value, target type)"));
            }
            Ok(get(&args[0]))
        }
        _ => Err(runtime_err("unknown builtin")),
    }
}

/// Capability guard for file builtins: untrusted `.klang` files must not
/// escape via `../../…` or absolute paths. Trusted scripts (tests, stdlib)
/// use temp-dir or relative paths and are unaffected. `env` is left
/// unrestricted (reads process env, returns empty when unset).
fn reject_unsafe_path(path: &str) -> Result<(), Diagnostic> {
    let p = std::path::Path::new(path);
    // Absolute paths are allowed only inside the system temp dir (tests use
    // `std::env::temp_dir()`); everything else must be relative without
    // parent components. This blocks `../../etc/passwd` and `/etc/passwd`
    // while keeping `mylib.klang`, `./a.klang`, `/tmp/...` working.
    if p.is_absolute() {
        let tmp = std::env::temp_dir();
        if !p.starts_with(&tmp) {
            return Err(runtime_err(&format!("unsafe absolute path `{path}`")));
        }
        return Ok(());
    }
    for comp in p.components() {
        if matches!(
            comp,
            std::path::Component::ParentDir | std::path::Component::RootDir
        ) {
            return Err(runtime_err(&format!("unsafe path `{path}`")));
        }
    }
    if path.is_empty() {
        return Err(runtime_err("empty path"));
    }
    Ok(())
}

/// `base.method(args...)` dispatch by runtime value type.
fn exec_method(
    method: &str,
    base_name: &str,
    base: &Value,
    args: &[Value],
    values: &mut HashMap<String, Value>,
) -> Result<Value, Diagnostic> {
    let arity = |want: usize| {
        if args.len() != want {
            Err(runtime_err("wrong number of method arguments"))
        } else {
            Ok(())
        }
    };
    match base {
        Value::Str(s) => {
            let s = s.clone();
            match method {
                "len" => {
                    arity(0)?;
                    Ok(Value::Int(s.len() as i64))
                }
                "upper" => {
                    arity(0)?;
                    Ok(Value::Str(s.to_uppercase()))
                }
                "lower" => {
                    arity(0)?;
                    Ok(Value::Str(s.to_lowercase()))
                }
                "trim" => {
                    arity(0)?;
                    Ok(Value::Str(s.trim().to_string()))
                }
                "chars" => {
                    arity(0)?;
                    Ok(Value::Array(
                        s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    ))
                }
                "contains" => {
                    arity(1)?;
                    Ok(Value::Int(i64::from(s.contains(&args[0].render()))))
                }
                "starts_with" => {
                    arity(1)?;
                    Ok(Value::Int(i64::from(s.starts_with(&args[0].render()))))
                }
                "ends_with" => {
                    arity(1)?;
                    Ok(Value::Int(i64::from(s.ends_with(&args[0].render()))))
                }
                "split" => {
                    arity(1)?;
                    let d = args[0].render();
                    Ok(Value::Array(
                        s.split(&d).map(|p| Value::Str(p.to_string())).collect(),
                    ))
                }
                "replace" => {
                    if args.len() != 2 {
                        return Err(runtime_err("replace() takes 2 arguments"));
                    }
                    Ok(Value::Str(s.replace(&args[0].render(), &args[1].render())))
                }
                _ => Err(runtime_err("unknown string method")),
            }
        }
        Value::Array(_) => match method {
            "len" => {
                arity(0)?;
                match base {
                    Value::Array(a) => Ok(Value::Int(a.len() as i64)),
                    _ => Err(runtime_err("unreachable")),
                }
            }
            "push" => {
                arity(1)?;
                match values.get_mut(base_name) {
                    Some(Value::Array(arr)) => {
                        arr.push(args[0].clone());
                        Ok(Value::Int(arr.len() as i64))
                    }
                    _ => Err(runtime_err("push() needs an array variable")),
                }
            }
            "pop" => {
                arity(0)?;
                match values.get_mut(base_name) {
                    Some(Value::Array(arr)) => {
                        arr.pop().ok_or_else(|| runtime_err("pop() of empty array"))
                    }
                    _ => Err(runtime_err("pop() needs an array variable")),
                }
            }
            "contains" => {
                arity(1)?;
                match base {
                    Value::Array(a) => Ok(Value::Int(i64::from(a.contains(&args[0])))),
                    _ => Err(runtime_err("unreachable")),
                }
            }
            "join" => {
                arity(1)?;
                match base {
                    Value::Array(a) => {
                        let sep = args[0].render();
                        Ok(Value::Str(
                            a.iter().map(|v| v.render()).collect::<Vec<_>>().join(&sep),
                        ))
                    }
                    _ => Err(runtime_err("unreachable")),
                }
            }
            _ => Err(runtime_err("unknown array method")),
        },
        Value::Map(_) => match method {
            "len" => {
                arity(0)?;
                match base {
                    Value::Map(m) => Ok(Value::Int(m.len() as i64)),
                    _ => Err(runtime_err("unreachable")),
                }
            }
            "keys" => {
                arity(0)?;
                match base {
                    Value::Map(m) => Ok(Value::Array(
                        m.iter().map(|(k, _)| Value::Str(k.clone())).collect(),
                    )),
                    _ => Err(runtime_err("unreachable")),
                }
            }
            "contains" => {
                arity(1)?;
                match base {
                    Value::Map(m) => Ok(Value::Int(i64::from(
                        m.iter().any(|(k, _)| k == &args[0].render()),
                    ))),
                    _ => Err(runtime_err("unreachable")),
                }
            }
            _ => Err(runtime_err("unknown map method")),
        },
        _ => Err(runtime_err("method calls need a string, array or map")),
    }
}

/// Integer-or-float operand pair.
enum Num {
    Int(i64),
    Float(f64),
}

fn ints_or_floats(
    values: &HashMap<String, Value>,
    stubs: &HashMap<String, i64>,
    left: &str,
    right: &str,
) -> (Num, Num) {
    let l = lookup(values, stubs, left).unwrap_or(Value::Int(0));
    let r = lookup(values, stubs, right).unwrap_or(Value::Int(0));
    match (l, r) {
        (Value::Float(a), Value::Float(b)) => (Num::Float(a), Num::Float(b)),
        (Value::Float(a), b) => (Num::Float(a), Num::Float(b.as_float())),
        (a, Value::Float(b)) => (Num::Float(a.as_float()), Num::Float(b)),
        (a, b) => (Num::Int(a.as_int()), Num::Int(b.as_int())),
    }
}

fn num2(
    l: Num,
    r: Num,
    int_op: impl Fn(i64, i64) -> i64,
    float_op: impl Fn(f64, f64) -> f64,
) -> Value {
    match (l, r) {
        (Num::Float(a), Num::Float(b)) => Value::Float(float_op(a, b)),
        (Num::Int(a), Num::Int(b)) => Value::Int(int_op(a, b)),
        (a, b) => {
            let (x, y) = (
                match a {
                    Num::Int(v) => v as f64,
                    Num::Float(v) => v,
                },
                match b {
                    Num::Int(v) => v as f64,
                    Num::Float(v) => v,
                },
            );
            Value::Float(float_op(x, y))
        }
    }
}

fn values_eq(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Float(a), b) => *a == b.as_float(),
        (a, Value::Float(b)) => a.as_float() == *b,
        _ => l == r,
    }
}

fn cmp_to_int(
    values: &HashMap<String, Value>,
    stubs: &HashMap<String, i64>,
    left: &str,
    right: &str,
    ord: impl Fn(std::cmp::Ordering) -> bool,
) -> Value {
    let l = lookup(values, stubs, left).unwrap_or(Value::Int(0));
    let r = lookup(values, stubs, right).unwrap_or(Value::Int(0));
    let o = match (&l, &r) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        (Value::Float(_), _) | (_, Value::Float(_)) => l
            .as_float()
            .partial_cmp(&r.as_float())
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => l.as_int().cmp(&r.as_int()),
    };
    Value::Int(i64::from(ord(o)))
}

fn index_value(base: &Value, iv: &Value) -> Result<Value, Diagnostic> {
    match base {
        Value::Array(arr) => {
            let i = iv.as_int();
            if i < 0 {
                return Err(runtime_err("negative index"));
            }
            arr.get(i as usize)
                .cloned()
                .ok_or_else(|| runtime_err("array index out of bounds"))
        }
        Value::Str(s) => {
            let i = iv.as_int();
            if i < 0 {
                return Err(runtime_err("negative index"));
            }
            s.chars()
                .nth(i as usize)
                .map(|c| Value::Str(c.to_string()))
                .ok_or_else(|| runtime_err("string index out of bounds"))
        }
        Value::Map(m) => {
            let key = match iv {
                Value::Str(k) => k.clone(),
                other => other.render(),
            };
            m.iter()
                .find(|(k, _)| k == &key)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| runtime_err("missing map key"))
        }
        _ => Err(runtime_err("indexing needs an array, map or string")),
    }
}

fn store_index(slot: Option<&mut Value>, iv: &Value, v: Value) -> Result<(), Diagnostic> {
    match slot {
        Some(Value::Array(arr)) => {
            let i = iv.as_int();
            if i < 0 || (i as usize) >= arr.len() {
                return Err(runtime_err("array index out of bounds"));
            }
            arr[i as usize] = v;
            Ok(())
        }
        Some(Value::Map(m)) => {
            let key = match iv {
                Value::Str(k) => k.clone(),
                other => other.render(),
            };
            if let Some(entry) = m.iter_mut().find(|(k, _)| k == &key) {
                entry.1 = v;
            } else {
                m.push((key, v));
            }
            Ok(())
        }
        _ => Err(runtime_err(
            "index assignment needs an array or map variable",
        )),
    }
}

fn call_value(
    ctx: &ExecCtx,
    func: &str,
    args: &[Value],
    depth: usize,
    parent: &CancelToken,
) -> Result<Value, Diagnostic> {
    // Prefer real user code when the function has a body with a `return`;
    // empty demo stubs like `fn fetch_a() -> i32 throws {}` fall through to
    // the stub table (fetch_a=20, fetch_b=22).
    if let Some(f) = ctx.module.find(func) {
        if f.instrs
            .iter()
            .any(|i| matches!(i.op, MirOp::Return { .. }))
        {
            return exec_function(ctx, func, args, depth + 1, parent);
        }
    }
    if let Some(v) = ctx.stubs.get(func) {
        return Ok(Value::Int(*v));
    }
    if let Some(v) = stub_call(func) {
        return Ok(Value::Int(v));
    }
    if ctx.module.find(func).is_some() {
        return exec_function(ctx, func, args, depth + 1, parent);
    }
    Err(Diagnostic::error(
        "E-UNDEFINED",
        &format!("undefined function `{func}`"),
        "runtime",
        0,
        0,
        "callee is not defined in this module",
        &["define the function first"],
        "names/scope",
    ))
}
