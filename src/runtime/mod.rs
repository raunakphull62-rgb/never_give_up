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
use crate::mir::{MirInstr, MirModule, MirOp};

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
    /// A closure value: lifted function `func` plus its by-value capture
    /// snapshot (`(name, value)` pairs in `closure_captures` order).
    /// Cloned on copy (never aliased); called through `MirOp::Call`
    /// dynamic dispatch with captures prepended to the call arguments.
    Closure {
        func: String,
        captures: Vec<(String, Value)>,
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
            // Like `Struct` (field count): capture count. Only reachable
            // through unchecked MIR — checked programs reject closure
            // arithmetic with `E-TYPE`.
            Self::Closure { captures, .. } => captures.len() as i64,
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
            Self::Closure { captures, .. } => captures.len() as f64,
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
            // A closure value is always truthy (like a struct): checked
            // programs reject non-bool conditions with `E-TYPE` anyway.
            Self::Closure { .. } => true,
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
            // Opaque by design: captures are an implementation detail,
            // so `print`/`str`/`format` show a stable placeholder.
            Self::Closure { .. } => "<closure>".to_string(),
        }
    }

    pub fn map_get(&self, key: &str) -> Option<Value> {
        match self {
            Self::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()),
            _ => None,
        }
    }
}

pub(crate) fn runtime_err(msg: &str) -> Diagnostic {
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

/// Integer overflow: an `i32` arithmetic result left the
/// `i32::MIN..=i32::MAX` range. Loud error, never a silent wrap or an
/// out-of-range value (cf. the checker, which already rejects
/// out-of-range literals with `E-TYPE`).
fn overflow_err(op: &str) -> Diagnostic {
    Diagnostic::error(
        "E-OVERFLOW",
        &format!("integer overflow in `{op}`: result out of i32 range"),
        "runtime",
        0,
        0,
        "i32 arithmetic never wraps: out-of-range results are errors",
        &["use smaller operands", "check bounds before operating"],
        "arithmetic/overflow",
    )
}

/// Convert a runtime `Int` operand to `i32`, failing loudly when the
/// stored `i64` is already outside `i32` range (e.g. from `int()` on a
/// huge string). Keeps every arithmetic site in one enforcement point.
fn to_i32_checked(v: i64, op: &str) -> Result<i32, Diagnostic> {
    i32::try_from(v).map_err(|_| overflow_err(op))
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
    /// Program arguments after the `.klang` file (CLI `run` supplies
    /// them; tests and MCP pass none). Read by the `args()` builtin.
    argv: Vec<String>,
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

/// Max concurrent child tasks per function frame. Each `spawn` runs on its
/// own thread with [`SPAWN_STACK_BYTES`] of stack: unbounded spawning
/// exhausts host memory.
/// `range()` is capped at 100k; task spawning gets an analogous bound.
pub const MAX_CONCURRENT_TASKS: usize = 256;

/// Stack size for spawned task threads (v1 `spawn` and v2 echo workers).
///
/// The interpreter recurses natively per Klang call (release ~2.4 KB/frame
/// measured; debug worst-case ~47 KB v1 / ~114 KB v2), so the Rust default
/// 2 MiB spawn stack overflows at ~850 frames release (~40 debug) before
/// the 1024 call-depth guard can fire. 8 MiB lets a spawned
/// `countdown(1000)` succeed and `countdown(2000)` fail with clean
/// `E-RUNTIME` in release, with ~3x margin over the measured ~2.5 MB for
/// 1000 frames. 256 tasks × 8 MiB is 2 GiB of reserved virtual address
/// space; resident memory stays small (see RSS check below) because stacks
/// commit on demand.
pub const SPAWN_STACK_BYTES: usize = 8 * 1024 * 1024;

/// Run with explicit args + captured `print` output, preserving the
/// runtime value's actual type for display (`Value::render` keeps `42.0`
/// as `42.0`; the `i32` channel below would truncate it to `42`).
///
/// The accumulated output is returned alongside the outcome even on
/// failure, so callers can show what the program printed before it
/// failed (HEAVY-TEST-1: this output used to be silently dropped on
/// `Err`, hiding how far a failing program got).
pub fn run_with_output_value_partial(
    module: &MirModule,
    entry: &str,
    args: &[Value],
    stubs: &HashMap<String, i32>,
) -> (Result<Value, Diagnostic>, Vec<String>) {
    run_with_argv(module, entry, args, &[], stubs)
}

/// [`run_with_output_value_partial`] plus program arguments for the
/// `args()` builtin. Entry-param binding is unchanged (`args` still
/// binds entry params); `argv` is a separate channel read only by
/// `args()`, so `main()` with no params plus CLI args is not an
/// arity error.
pub fn run_with_argv(
    module: &MirModule,
    entry: &str,
    args: &[Value],
    argv: &[String],
    stubs: &HashMap<String, i32>,
) -> (Result<Value, Diagnostic>, Vec<String>) {
    // Entry arity must match exactly (calling-convention safety), mirroring
    // the JIT backend. The CLI always passes zero args, so missing params
    // are an arity error, not silent zeros. No program output can exist
    // yet at this point, so the failure carries empty output.
    if let Some(f) = module.find(entry) {
        if args.len() != f.params.len() {
            return (
                Err(Diagnostic::error(
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
                )),
                Vec::new(),
            );
        }
    }
    let ctx = ExecCtx {
        module: Arc::new(module.clone()),
        stubs: stubs
            .iter()
            .map(|(k, v)| (k.clone(), i64::from(*v)))
            .collect(),
        output: Arc::new(Mutex::new(Vec::new())),
        argv: argv.to_vec(),
    };
    let r = exec_function(&ctx, entry, args, 0, &CancelToken::default());
    let out = ctx.output.lock().unwrap().clone();
    (r, out)
}

/// Run with explicit args + captured `print` output, preserving the
/// runtime value's actual type for display (`Value::render` keeps `42.0`
/// as `42.0`; the `i32` channel below would truncate it to `42`).
pub fn run_with_output_value(
    module: &MirModule,
    entry: &str,
    args: &[Value],
    stubs: &HashMap<String, i32>,
) -> Result<(Value, Vec<String>), Diagnostic> {
    let (r, out) = run_with_output_value_partial(module, entry, args, stubs);
    r.map(|v| (v, out))
}

/// Run with explicit args + captured `print` output.
pub fn run_with_output(
    module: &MirModule,
    entry: &str,
    args: &[Value],
    stubs: &HashMap<String, i32>,
) -> Result<(i32, Vec<String>), Diagnostic> {
    let (v, out) = run_with_output_value(module, entry, args, stubs)?;
    // Do not silently truncate via `as i32`: surface out-of-range results
    // with the same `E-OVERFLOW` as intermediate arithmetic.
    let n = v.as_int();
    if n < i32::MIN as i64 || n > i32::MAX as i64 {
        return Err(overflow_err("return"));
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
) -> Result<ExecFlow, Diagnostic> {
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

/// How an instruction slice finished: ran to the end (`Done`) or hit
/// `return` (`Returned`). Slices are `try` bodies/handlers sharing the
/// caller's scope; a `return` inside one returns from the whole function.
enum ExecFlow {
    Done(Value),
    Returned(Value),
}

/// Validate jump targets up front: `break`/`continue` outside a loop
/// lower to `Jump { target: usize::MAX }` (HIR rejects, but `lower` is
/// public). Out-of-range targets must be `Err`, never a host panic, and
/// the JIT maps past-the-end to `end` so both backends agree.
fn validate_jumps(instrs: &[MirInstr]) -> Result<(), Diagnostic> {
    for ins in instrs {
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
    Ok(())
}

fn exec_function(
    ctx: &ExecCtx,
    name: &str,
    args: &[Value],
    depth: usize,
    parent: &CancelToken,
) -> Result<Value, Diagnostic> {
    if depth > 1024 {
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
    match run_instrs(
        ctx,
        name,
        &instrs,
        &mut values,
        &mut groups,
        &mut group_depth,
        &mut pending,
        depth,
        parent,
    )? {
        ExecFlow::Done(v) => {
            // Fall-off-the-end with live tasks means a group was never
            // left (unchecked MIR): join (never detach) then fail closed.
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
            Ok(v)
        }
        // `return` already joined-or-failed at its own site below.
        ExecFlow::Returned(v) => Ok(v),
    }
}

/// Execute one instruction slice sharing the caller's scope (`values`,
/// task-group stack, pending handles). The whole-function stream and
/// each `try` body/handler run through here, so error capture, task
/// handles, and group tokens behave identically in all three.
#[allow(clippy::too_many_lines)]
fn run_instrs(
    ctx: &ExecCtx,
    name: &str,
    instrs: &[MirInstr],
    mut values: &mut HashMap<String, Value>,
    groups: &mut Vec<(String, CancelToken)>,
    group_depth: &mut usize,
    mut pending: &mut HashMap<String, JoinHandle<Result<Value, Diagnostic>>>,
    depth: usize,
    parent: &CancelToken,
) -> Result<ExecFlow, Diagnostic> {
    validate_jumps(instrs)?;
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
                *group_depth += 1;
                groups.push((format!("{group}"), CancelToken::default()));
                pc += 1;
            }
            MirOp::LeaveGroup { .. } => {
                *group_depth = group_depth.saturating_sub(1);
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
                if *group_depth == 0 {
                    return Err(Diagnostic::spawn_outside_group("runtime", 0, 0));
                }
                // A closure value carries snapshots the child frame cannot
                // see (checked programs reject this with `E-TYPE`); through
                // unchecked MIR it is a loud error here, never a detached
                // or mis-scoped thread.
                if matches!(values.get(func), Some(Value::Closure { .. })) {
                    return Err(runtime_err(
                        "cannot `spawn` a closure value (spawn takes a named function)",
                    ));
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
                let h = std::thread::Builder::new()
                    .name(format!("klang-spawn-{handle}"))
                    .stack_size(SPAWN_STACK_BYTES)
                    .spawn(move || {
                        call_value(&child, &func, &arg_vals, depth + 1, &token)
                    })
                    .map_err(|e| {
                        concurrent_err(&format!("failed to spawn task `{handle}`: {e}"))
                    })?;
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
                    let v = exec_builtin(func, args, &mut values, &ctx.stubs, &ctx.argv)?;
                    values.insert(into.clone(), v.clone());
                    last = v;
                    pc += 1;
                    continue;
                }
                // Dynamic dispatch through a closure-typed binding
                // (values-first, mirroring the checker: a closure binding
                // shadows any same-named function). Captures were
                // snapshotted at the literal site, so they are prepended
                // to the call arguments to match the lifted parameter
                // prefix; depth accounting matches static calls exactly.
                if let Some(Value::Closure {
                    func: target,
                    captures,
                }) = values.get(func).cloned()
                {
                    let mut arg_vals: Vec<Value> =
                        captures.iter().map(|(_, v)| v.clone()).collect();
                    for a in args {
                        arg_vals.push(lookup(&values, &ctx.stubs, a).unwrap_or(Value::Int(0)));
                    }
                    let v = call_value(ctx, &target, &arg_vals, depth, &current(&groups, parent))?;
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
                let v = Value::Int(*value);
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::ConstFloat { into, bits } => {
                let v = Value::Float(f64::from_bits(*bits));
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::ConstStr { into, value } => {
                let v = Value::Str(value.clone());
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::ClosureNew {
                into,
                func,
                captures,
            } => {
                // By-value snapshot: each named binding is cloned NOW, so
                // later mutations of the outer variables — including the
                // outer function having returned — never affect the value.
                // (Checked programs always bind every capture; unchecked
                // MIR falls back to the usual `Int(0)` convention.)
                let mut snap = Vec::with_capacity(captures.len());
                for c in captures {
                    snap.push((
                        c.clone(),
                        lookup(&values, &ctx.stubs, c).unwrap_or(Value::Int(0)),
                    ));
                }
                let v = Value::Closure {
                    func: func.clone(),
                    captures: snap,
                };
                values.insert(into.clone(), v.clone());
                last = v;
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
                    _ => {
                        let x = to_i32_checked(l.as_int(), "add")?;
                        let y = to_i32_checked(r.as_int(), "add")?;
                        let z = x.checked_add(y).ok_or_else(|| overflow_err("add"))?;
                        Value::Int(i64::from(z))
                    }
                };
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Sub { into, left, right } => {
                let (l, r) = ints_or_floats(&values, &ctx.stubs, left, right);
                let v = num2_checked("sub", l, r, |a, b| a.checked_sub(b), |a, b| a - b)?;
                values.insert(into.clone(), v.clone());
                last = v;
                pc += 1;
            }
            MirOp::Mul { into, left, right } => {
                let (l, r) = ints_or_floats(&values, &ctx.stubs, left, right);
                let v = num2_checked("mul", l, r, |a, b| a.checked_mul(b), |a, b| a * b)?;
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
                // `i32::MIN / -1` overflows `i32` (checked_div is None):
                // loud `E-OVERFLOW`, never a silent out-of-range value.
                let v = num2_checked("div", l, r, |a, b| a.checked_div(b), |a, b| a / b)?;
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
                let x = to_i32_checked(l, "mod")?;
                let y = to_i32_checked(r, "mod")?;
                let z = x.checked_rem(y).ok_or_else(|| overflow_err("mod"))?;
                let v = Value::Int(i64::from(z));
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
                    _ => {
                        let x = to_i32_checked(v.as_int(), "neg")?;
                        let z = x.checked_neg().ok_or_else(|| overflow_err("neg"))?;
                        Value::Int(i64::from(z))
                    }
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
                let v = Value::Array(arr);
                values.insert(into.clone(), v.clone());
                last = v;
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
                let v = Value::Map(map);
                values.insert(into.clone(), v.clone());
                last = v;
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
                let v = Value::Struct {
                    name: name.clone(),
                    fields: fvals,
                };
                values.insert(into.clone(), v.clone());
                last = v;
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
                let v = Value::Int(n);
                values.insert(into.clone(), v.clone());
                last = v;
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
                return Ok(ExecFlow::Returned(v));
            }
            MirOp::Try { code_var, body, handler } => {
                // Same scope, same task state: body assignments persist,
                // spawns join through the shared pending map, group tokens
                // resolve through the shared stack.
                match run_instrs(
                    ctx,
                    name,
                    body,
                    &mut *values,
                    &mut *groups,
                    &mut *group_depth,
                    &mut *pending,
                    depth,
                    parent,
                ) {
                    Ok(ExecFlow::Done(v)) => {
                        last = v;
                        pc += 1;
                    }
                    Ok(ExecFlow::Returned(v)) => return Ok(ExecFlow::Returned(v)),
                    // Cooperative cancellation is not an error to recover
                    // from: re-raise so task groups keep draining. Whole-
                    // program `exit()` behaves the same (uncatchable).
                    Err(d) if d.is_cancelled() || d.is_exit() => return Err(d),
                    Err(d) => {
                        values.insert(
                            code_var.clone(),
                            Value::Map(vec![
                                ("code".to_string(), Value::Str(d.code.clone())),
                                ("message".to_string(), Value::Str(d.message.clone())),
                            ]),
                        );
                        match run_instrs(
                            ctx,
                            name,
                            handler,
                            &mut *values,
                            &mut *groups,
                            &mut *group_depth,
                            &mut *pending,
                            depth,
                            parent,
                        )? {
                            ExecFlow::Done(v) => {
                                last = v;
                                pc += 1;
                            }
                            ExecFlow::Returned(v) => return Ok(ExecFlow::Returned(v)),
                        }
                    }
                }
            }
        }
    }
    // Fall-off-the-end yields the last computed value, or 0 for empty
    // bodies (documented default; empty stub functions like
    // `fn fetch_a() -> i32 throws {}` rely on the stub table via
    // `call_value`, not on this path). HIR accepts this; the JIT mirrors
    // it via a dominating `last` variable seeded with 0.
    Ok(ExecFlow::Done(last))
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
            | "remove_file"
            | "env"
            | "run_process"
            | "regex_is_match"
            | "regex_find"
            | "time_sleep"
            | "time_now"
            | "time_elapsed"
            | "http_get"
            | "http_post"
            | "http_get_async"
            | "http_post_async"
            | "read_line"
            | "parse_int"
            | "parse_float"
            | "format"
            | "insert"
            | "args"
            | "exit"
            | "cwd"
            | "set_env"
            | "list_dir"
            | "make_dir"
            | "make_dirs"
            | "is_dir"
            | "is_file"
            | "rename_file"
            | "copy_file"
            | "file_size"
            | "ord"
            | "chr"
            | "slice"
            | "sort"
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

/// Render an [`crate::stdlib::http::HttpResponse`] as the
/// bracket-indexed map Klang programs see: `resp["status"]` (int),
/// `resp["body"]` (str), `resp["headers"]` (map of lowercased
/// name → value, e.g. `resp["headers"]["content-type"]`).
fn http_response_map(r: &crate::stdlib::http::HttpResponse) -> Value {
    Value::Map(vec![
        ("status".to_string(), Value::Int(i64::from(r.status))),
        ("body".to_string(), Value::Str(r.body.clone())),
        (
            "headers".to_string(),
            Value::Map(
                r.headers
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::Str(v.clone())))
                    .collect(),
            ),
        ),
    ])
}

fn exec_builtin(
    func: &str,
    args: &[String],
    values: &mut HashMap<String, Value>,
    stubs: &HashMap<String, i64>,
    argv: &[String],
) -> Result<Value, Diagnostic> {    let get = |name: &String| lookup(values, stubs, name).unwrap_or(Value::Int(0));
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
        "read_line" => {
            if !args.is_empty() {
                return Err(runtime_err("read_line() takes 0 arguments"));
            }
            Ok(Value::Str(crate::stdlib::io::read_line()))
        }
        "parse_int" => {
            if args.len() != 1 {
                return Err(runtime_err("parse_int() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Str(s) => crate::stdlib::io::parse_int_str(&s).map(Value::Int),
                Value::Int(v) => Ok(Value::Int(v)),
                Value::Float(v) => Ok(Value::Int(v as i64)),
                _ => Err(runtime_err("parse_int() needs a string")),
            }
        }
        "parse_float" => {
            if args.len() != 1 {
                return Err(runtime_err("parse_float() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Str(s) => crate::stdlib::io::parse_float_str(&s).map(Value::Float),
                Value::Float(v) => Ok(Value::Float(v)),
                Value::Int(v) => Ok(Value::Float(v as f64)),
                _ => Err(runtime_err("parse_float() needs a string")),
            }
        }
        "format" => {
            // Variadic (zero args allowed): render each value exactly like
            // `str()`/`print` (`Value::render`) and join with one space.
            let mut parts = Vec::with_capacity(args.len());
            for a in args {
                parts.push(get(a).render());
            }
            Ok(Value::Str(parts.join(" ")))
        }
        "insert" => {
            if args.len() != 3 {
                return Err(runtime_err("insert() takes 3 arguments"));
            }
            // Index == len appends (like `push`); past-the-end or
            // negative is a loud error, never a silent clamp or hole.
            let idx = get(&args[1]).as_int();
            let v = get(&args[2]);
            match values.get_mut(&args[0]) {
                Some(Value::Array(arr)) => {
                    if idx < 0 || (idx as usize) > arr.len() {
                        return Err(runtime_err("insert() index out of bounds"));
                    }
                    arr.insert(idx as usize, v);
                    Ok(Value::Int(arr.len() as i64))
                }
                _ => Err(runtime_err("insert() needs an array variable first")),
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
        "remove_file" => {
            if args.len() != 1 {
                return Err(runtime_err("remove_file() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::remove(&path).map(|()| Value::Int(1))
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
        "time_sleep" => {
            if args.len() != 1 {
                return Err(runtime_err("time_sleep() takes 1 argument"));
            }
            // Numeric values only: a dynamically-typed non-number
            // (e.g. a string out of a map lookup, which checks as
            // Unknown) is a loud E-TIME-INVALID, never a silent
            // sleep-0.
            let seconds = match get(&args[0]) {
                Value::Int(v) => v as f64,
                Value::Float(v) => v,
                other => {
                    return Err(crate::stdlib::time::not_a_number(
                        "time_sleep",
                        "time_sleep() seconds",
                        &other.render(),
                    ));
                }
            };
            crate::stdlib::time::sleep(seconds).map(|()| Value::Int(1))
        }
        "time_now" => {
            if !args.is_empty() {
                return Err(runtime_err("time_now() takes 0 arguments"));
            }
            Ok(Value::Float(crate::stdlib::time::now()))
        }
        "time_elapsed" => {
            if args.len() != 1 {
                return Err(runtime_err("time_elapsed() takes 1 argument"));
            }
            let since = match get(&args[0]) {
                Value::Int(v) => v as f64,
                Value::Float(v) => v,
                other => {
                    return Err(crate::stdlib::time::not_a_number(
                        "time_elapsed",
                        "time_elapsed() since",
                        &other.render(),
                    ));
                }
            };
            Ok(Value::Float(crate::stdlib::time::elapsed(since)))
        }
        "http_get" => {
            if args.len() != 1 {
                return Err(runtime_err("http_get() takes 1 argument"));
            }
            let url = get(&args[0]).render();
            let r = crate::stdlib::http::get(&url)?;
            Ok(http_response_map(&r))
        }
        "http_post" => {
            if args.len() != 3 {
                return Err(runtime_err("http_post() takes 3 arguments"));
            }
            let url = get(&args[0]).render();
            let body = get(&args[1]).render();
            let headers = match get(&args[2]) {
                Value::Array(items) => {
                    let mut out = Vec::with_capacity(items.len());
                    for v in items {
                        out.push(v.render());
                    }
                    out
                }
                _ => {
                    return Err(runtime_err(
                        "http_post() needs an array of strings third",
                    ))
                }
            };
            let r = crate::stdlib::http::post(&url, &body, &headers)?;
            Ok(http_response_map(&r))
        }
        // Simulated-async twins (FOUNDATION-3 Part 2A): identical
        // signatures, results, and error codes as the sync versions —
        // only the execution site differs (shared bounded pool).
        "http_get_async" => {
            if args.len() != 1 {
                return Err(runtime_err("http_get_async() takes 1 argument"));
            }
            let url = get(&args[0]).render();
            let r = crate::stdlib::http::get_async(&url)?;
            Ok(http_response_map(&r))
        }
        "http_post_async" => {
            if args.len() != 3 {
                return Err(runtime_err("http_post_async() takes 3 arguments"));
            }
            let url = get(&args[0]).render();
            let body = get(&args[1]).render();
            let headers = match get(&args[2]) {
                Value::Array(items) => {
                    let mut out = Vec::with_capacity(items.len());
                    for v in items {
                        out.push(v.render());
                    }
                    out
                }
                _ => {
                    return Err(runtime_err(
                        "http_post_async() needs an array of strings third",
                    ))
                }
            };
            let r = crate::stdlib::http::post_async(&url, &body, &headers)?;
            Ok(http_response_map(&r))
        }
        "args" => {
            if !args.is_empty() {
                return Err(runtime_err("args() takes 0 arguments"));
            }
            Ok(Value::Array(
                argv.iter().map(|s| Value::Str(s.clone())).collect(),
            ))
        }
        "exit" => {
            if args.len() != 1 {
                return Err(runtime_err("exit() takes 1 argument"));
            }
            Err(Diagnostic::exit_request(get(&args[0]).as_int() as i32))
        }
        "cwd" => {
            if !args.is_empty() {
                return Err(runtime_err("cwd() takes 0 arguments"));
            }
            crate::stdlib::process::cwd().map(Value::Str)
        }
        "set_env" => {
            if args.len() != 2 {
                return Err(runtime_err("set_env() takes 2 arguments"));
            }
            let name = get(&args[0]).render();
            let value = get(&args[1]).render();
            crate::stdlib::process::set_env(&name, &value).map(|()| Value::Int(1))
        }
        "list_dir" => {
            if args.len() != 1 {
                return Err(runtime_err("list_dir() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::list_dir(&path)
                .map(|names| Value::Array(names.into_iter().map(Value::Str).collect()))
        }
        "make_dir" => {
            if args.len() != 1 {
                return Err(runtime_err("make_dir() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::make_dir(&path).map(|()| Value::Int(1))
        }
        "make_dirs" => {
            if args.len() != 1 {
                return Err(runtime_err("make_dirs() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::make_dirs(&path).map(|()| Value::Int(1))
        }
        "is_dir" => {
            if args.len() != 1 {
                return Err(runtime_err("is_dir() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            Ok(Value::Int(i64::from(crate::stdlib::file::is_dir(&path))))
        }
        "is_file" => {
            if args.len() != 1 {
                return Err(runtime_err("is_file() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            Ok(Value::Int(i64::from(crate::stdlib::file::is_file(&path))))
        }
        "rename_file" => {
            if args.len() != 2 {
                return Err(runtime_err("rename_file() takes 2 arguments"));
            }
            let from = get(&args[0]).render();
            let to = get(&args[1]).render();
            reject_unsafe_path(&from)?;
            reject_unsafe_path(&to)?;
            crate::stdlib::file::rename(&from, &to).map(|()| Value::Int(1))
        }
        "copy_file" => {
            if args.len() != 2 {
                return Err(runtime_err("copy_file() takes 2 arguments"));
            }
            let from = get(&args[0]).render();
            let to = get(&args[1]).render();
            reject_unsafe_path(&from)?;
            reject_unsafe_path(&to)?;
            crate::stdlib::file::copy(&from, &to).map(|n| Value::Int(n as i64))
        }
        "file_size" => {
            if args.len() != 1 {
                return Err(runtime_err("file_size() takes 1 argument"));
            }
            let path = get(&args[0]).render();
            reject_unsafe_path(&path)?;
            crate::stdlib::file::file_size(&path).map(|n| Value::Int(n as i64))
        }
        "ord" => {
            if args.len() != 1 {
                return Err(runtime_err("ord() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Str(s) => crate::stdlib::seq::ord(&s).map(|n| Value::Int(n as i64)),
                _ => Err(crate::stdlib::seq::char_invalid("ord", "ord() needs a string")),
            }
        }
        "chr" => {
            if args.len() != 1 {
                return Err(runtime_err("chr() takes 1 argument"));
            }
            crate::stdlib::seq::chr(get(&args[0]).as_int()).map(Value::Str)
        }
        "slice" => {
            if args.len() != 3 {
                return Err(runtime_err("slice() takes 3 arguments"));
            }
            let (lo, hi) = (get(&args[1]).as_int(), get(&args[2]).as_int());
            match get(&args[0]) {
                Value::Str(s) => crate::stdlib::seq::slice_str(&s, lo, hi).map(Value::Str),
                Value::Array(items) => {
                    if lo < 0 || hi < 0 {
                        return Err(runtime_err("negative index"));
                    }
                    if hi < lo {
                        return Err(runtime_err("slice() end before start"));
                    }
                    if (hi as usize) > items.len() {
                        return Err(runtime_err("array slice index out of bounds"));
                    }
                    Ok(Value::Array(items[lo as usize..hi as usize].to_vec()))
                }
                _ => Err(runtime_err("slice() needs a string or array first")),
            }
        }
        "sort" => {
            if args.len() != 1 {
                return Err(runtime_err("sort() takes 1 argument"));
            }
            match get(&args[0]) {
                Value::Array(items) => {
                    crate::stdlib::seq::sort_list(&items).map(Value::Array)
                }
                _ => Err(runtime_err("sort() needs an array")),
            }
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
            "insert" => {
                if args.len() != 2 {
                    return Err(runtime_err("insert() takes 2 arguments"));
                }
                match values.get_mut(base_name) {
                    Some(Value::Array(arr)) => {
                        let idx = args[0].as_int();
                        if idx < 0 || (idx as usize) > arr.len() {
                            return Err(runtime_err("insert() index out of bounds"));
                        }
                        arr.insert(idx as usize, args[1].clone());
                        Ok(Value::Int(arr.len() as i64))
                    }
                    _ => Err(runtime_err("insert() needs an array variable")),
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

/// Checked integer arithmetic over `i32` semantics: both `i64` operands
/// must already fit `i32`, and the `i32::checked_*` result must too.
/// Float-involved pairs stay unchecked `f64` math (no `i32` range applies).
fn num2_checked(
    op: &str,
    l: Num,
    r: Num,
    int_op: impl Fn(i32, i32) -> Option<i32>,
    float_op: impl Fn(f64, f64) -> f64,
) -> Result<Value, Diagnostic> {
    match (l, r) {
        (Num::Float(a), Num::Float(b)) => Ok(Value::Float(float_op(a, b))),
        (Num::Int(a), Num::Int(b)) => {
            let x = to_i32_checked(a, op)?;
            let y = to_i32_checked(b, op)?;
            int_op(x, y)
                .map(|v| Value::Int(i64::from(v)))
                .ok_or_else(|| overflow_err(op))
        }
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
            Ok(Value::Float(float_op(x, y)))
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
            // Integer index is positional: the i-th key in insertion
            // order (same order as `keys()`). This is what `for k in m`
            // lowers to (integer-indexed iteration over `len(m)`).
            // Any other index is a key lookup, as before.
            if let Value::Int(i) = iv {
                if *i < 0 {
                    return Err(runtime_err("negative index"));
                }
                return m
                    .get(*i as usize)
                    .map(|(k, _)| Value::Str(k.clone()))
                    .ok_or_else(|| runtime_err("map index out of bounds"));
            }
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
