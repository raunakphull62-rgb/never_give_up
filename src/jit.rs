//! Step 1 backend: Cranelift JIT (Phase 0 probe + Phase 1 int-only).
//!
//! Phase 0: prove `cranelift-jit` builds and runs here — JIT-compile
//! `add(i64, i64) -> i64`, call it with (7, 35), expect 42.
//! Phase 1 (next): lower the int-only MIR subset through the same
//! `JITModule`/`FunctionBuilder` path, behind `--backend jit`.

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

/// Phase-0 probe: JIT-compile `add(a, b) = a + b`, run it on (7, 35).
pub fn probe_add() -> Result<i64, String> {
    let builder = JITBuilder::new(cranelift_module::default_libcall_names())
        .map_err(|e| format!("jit builder: {e}"))?;
    let mut module = JITModule::new(builder);

    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    let func_id = module
        .declare_function("add", Linkage::Local, &sig)
        .map_err(|e| format!("declare: {e}"))?;

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut builder_ctx = FunctionBuilderContext::new();
    let mut fb = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
    let block = fb.create_block();
    fb.append_block_params_for_function_params(block);
    fb.switch_to_block(block);
    fb.seal_block(block);
    let a = fb.block_params(block)[0];
    let b = fb.block_params(block)[1];
    let sum = fb.ins().iadd(a, b);
    fb.ins().return_(&[sum]);
    fb.finalize();

    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| format!("define: {e}"))?;
    module.clear_context(&mut ctx);
    module
        .finalize_definitions()
        .map_err(|e| format!("finalize: {e}"))?;

    let code = module.get_finalized_function(func_id);
    // SAFETY: we just compiled `add` with exactly this signature.
    let add = unsafe { std::mem::transmute::<*const u8, fn(i64, i64) -> i64>(code) };
    Ok(add(7, 35))
}

// ---------------------------------------------------------------------------
// Phase 1: int-only MIR -> machine code.
//
// Every value is an `I64`, mirroring `Value::Int` in the interpreter.
// Supported ops: Const, Copy, Add/Sub/Mul/Div/Mod, all six comparisons,
// And/Or/Neg/Not, JumpIfFalse/Jump, Return, Call (user fns only), Print
// (via imported host fn), EnterGroup/LeaveGroup (no-ops, single-threaded).
// Anything else (strings, floats, arrays, maps, structs, Spawn/Await,
// builtins) is an explicit `Err`, never silent wrong code.
//
// Wave1 S5 (D6 fallible JIT): arithmetic is checked, exactly like the
// interpreter. `Add`/`Sub`/`Mul`/`Neg` overflow, `Div`/`Mod` by zero,
// and `i64::MIN / -1` report the interpreter's `E-OVERFLOW`/`E-RUNTIME`
// diagnostics (same code, message, cause, fixes, rule) with a normal
// nonzero exit — the `sdiv`/`srem` trap class is gone. Calls carry the
// interpreter's 1024-frame depth guard, and D3 narrow boundaries
// (`i32`/`u32`/`u64`/`u8` params, returns, annotated copies) are
// enforced with the interpreter's `E-RUNTIME` (rule `types/range`).
// Struct/enum field boundaries need no JIT code: `StructNew`/
// `FieldGet`/`FieldSet` are already loud `JIT-FAIL` rejections, so a
// program that could hit them never runs on the JIT.
// ---------------------------------------------------------------------------

use std::collections::{HashMap, HashSet};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::Block;
use cranelift_codegen::ir::Value as IrValue;
use cranelift_frontend::Variable;
use cranelift_module::FuncId;

use crate::diagnostics::Diagnostic;
use crate::mir::{MirModule, MirOp};
use crate::runtime::Value;

fn jit_output() -> &'static Mutex<Vec<String>> {
    static OUT: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    OUT.get_or_init(|| Mutex::new(Vec::new()))
}

/// Serializes whole JIT runs: the print buffer above is process-global,
/// so two threads running `run_jit` concurrently would interleave
/// clear/print/read without this. (No reentry: JIT code never calls back
/// into `run_jit`, so no deadlock.)
static JIT_RUN: Mutex<()> = Mutex::new(());

extern "C" fn host_print_i64(x: i64) {
    jit_output().lock().unwrap().push(x.to_string());
}

// ---------------------------------------------------------------------------
// Fallible-JIT error channel (Wave1 S5, D6 Option A).
//
// Compiled code stays single-`I64`-return, but checked ops call back
// into host helpers below. A helper that detects the interpreter's
// error records the identical `Diagnostic` and sets `JIT_FAIL_FLAG`;
// compiled code loads the flag after every checked op/call and diverts
// to the per-function `err` block (returning 0, ignored) on nonzero.
// `run_jit_fallible` then surfaces the diagnostic with the prints
// emitted so far — the same `run: FAIL` + JSON the interpreter prints.
// The first error wins; anything later is defensive only.
// Everything here is only touched under `JIT_RUN`, so sequential runs
// (including the in-process differential gates) never cross-talk.
// ---------------------------------------------------------------------------

/// Interpreter's call-depth limit, mirrored exactly: a call fails when
/// the current depth is already above this (the entry frame counts as
/// depth 0, like `exec_function`). `countdown(1023)` runs everywhere;
/// `countdown(1024)` fails everywhere with the same `E-RUNTIME`.
pub const JIT_MAX_DEPTH: u64 = 1024;

/// Nonzero while compiled code must abort to its `err` block.
static JIT_FAIL_FLAG: AtomicU64 = AtomicU64::new(0);
/// Current call depth. The entry frame observes 0, each nested call
/// observes one more — the same numbering as the interpreter.
static JIT_DEPTH: AtomicU64 = AtomicU64::new(0);
/// First runtime diagnostic of this run (first error wins).
static JIT_DIAG: Mutex<Option<Diagnostic>> = Mutex::new(None);
/// D3 boundary descriptors registered at compile time (`ty_raw` + `ctx`
/// for `check_int_boundary`); `klang_jit_check_bound` looks its site up
/// here. Cleared per run; only touched under `JIT_RUN`.
static JIT_BOUND_SITES: Mutex<Vec<BoundSite>> = Mutex::new(Vec::new());

#[derive(Debug, Clone)]
struct BoundSite {
    ty_raw: String,
    ctx: String,
}

fn jit_fail(d: Diagnostic) {
    if JIT_FAIL_FLAG
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        *JIT_DIAG.lock().unwrap() = Some(d);
    }
}

fn jit_failed() -> bool {
    JIT_FAIL_FLAG.load(Ordering::SeqCst) != 0
}

fn jit_reset() {
    JIT_FAIL_FLAG.store(0, Ordering::SeqCst);
    JIT_DEPTH.store(0, Ordering::SeqCst);
    *JIT_DIAG.lock().unwrap() = None;
    JIT_BOUND_SITES.lock().unwrap().clear();
}

/// Register one D3 boundary site; returns its id for
/// `klang_jit_check_bound`. Compile time only (under `JIT_RUN`).
fn jit_register_site(ty_raw: &str, ctx: String) -> i64 {
    let mut sites = JIT_BOUND_SITES.lock().unwrap();
    let id = sites.len() as i64;
    sites.push(BoundSite {
        ty_raw: ty_raw.to_string(),
        ctx,
    });
    id
}

// Checked-arithmetic host helpers. Each mirrors its interpreter arm in
// `src/runtime/mod.rs` exactly (same zero checks, same `checked_*`
// call, same `overflow_err(op)`/`runtime_err(msg)` constructor), so the
// recorded diagnostic is identical by construction. When a previous op
// already failed, they return immediately to preserve the first error.

extern "C" fn klang_jit_add(a: i64, b: i64) -> i64 {
    if jit_failed() {
        return 0;
    }
    match a.checked_add(b) {
        Some(v) => v,
        None => {
            jit_fail(crate::runtime::overflow_err("add"));
            0
        }
    }
}

extern "C" fn klang_jit_sub(a: i64, b: i64) -> i64 {
    if jit_failed() {
        return 0;
    }
    match a.checked_sub(b) {
        Some(v) => v,
        None => {
            jit_fail(crate::runtime::overflow_err("sub"));
            0
        }
    }
}

extern "C" fn klang_jit_mul(a: i64, b: i64) -> i64 {
    if jit_failed() {
        return 0;
    }
    match a.checked_mul(b) {
        Some(v) => v,
        None => {
            jit_fail(crate::runtime::overflow_err("mul"));
            0
        }
    }
}

extern "C" fn klang_jit_div(a: i64, b: i64) -> i64 {
    if jit_failed() {
        return 0;
    }
    // Zero first, then the checked op (whose only `None` case is
    // `i64::MIN / -1`) — the interpreter's `Div` arm in this order.
    if b == 0 {
        jit_fail(crate::runtime::runtime_err("division by zero"));
        return 0;
    }
    match a.checked_div(b) {
        Some(v) => v,
        None => {
            jit_fail(crate::runtime::overflow_err("div"));
            0
        }
    }
}

extern "C" fn klang_jit_rem(a: i64, b: i64) -> i64 {
    if jit_failed() {
        return 0;
    }
    if b == 0 {
        jit_fail(crate::runtime::runtime_err("modulo by zero"));
        return 0;
    }
    // `i64::MIN % -1` is `Some(0)`: no error, exactly like the
    // interpreter's `checked_rem` arm.
    match a.checked_rem(b) {
        Some(v) => v,
        None => {
            jit_fail(crate::runtime::overflow_err("mod"));
            0
        }
    }
}

extern "C" fn klang_jit_neg(a: i64) -> i64 {
    if jit_failed() {
        return 0;
    }
    match a.checked_neg() {
        Some(v) => v,
        None => {
            jit_fail(crate::runtime::overflow_err("neg"));
            0
        }
    }
}

/// Function-entry depth guard. Returns 0 (proceed) or 1 (abort to the
/// `err` block). Mirrors `exec_function`: the entry frame observes
/// depth 0 and a call fails exactly when depth is already above 1024.
extern "C" fn klang_jit_enter() -> i64 {
    if jit_failed() {
        return 1;
    }
    let d = JIT_DEPTH.load(Ordering::SeqCst);
    if d > JIT_MAX_DEPTH {
        jit_fail(crate::runtime::depth_limit_err());
        return 1;
    }
    JIT_DEPTH.store(d + 1, Ordering::SeqCst);
    0
}

/// Normal-return path only: error paths skip this (the run aborts and
/// `jit_reset` clears the counter for the next run).
extern "C" fn klang_jit_leave() {
    let d = JIT_DEPTH.load(Ordering::SeqCst);
    JIT_DEPTH.store(d.saturating_sub(1), Ordering::SeqCst);
}

/// D3 boundary check for one value. The diagnostic is built by the SAME
/// `check_int_boundary` constructor as the interpreter, so code,
/// message, cause, fixes and rule are identical by construction.
extern "C" fn klang_jit_check_bound(site: i64, v: i64) -> i64 {
    if jit_failed() {
        return v;
    }
    let (ty_raw, ctx) = match JIT_BOUND_SITES.lock().unwrap().get(site as usize) {
        Some(s) => (s.ty_raw.clone(), s.ctx.clone()),
        None => {
            jit_fail(crate::runtime::runtime_err("internal JIT boundary error"));
            return v;
        }
    };
    match crate::runtime::check_int_boundary(&Value::Int(v), &ty_raw, &ctx) {
        Ok(()) => v,
        Err(d) => {
            jit_fail(d);
            v
        }
    }
}

/// What a JIT run can report.
///
/// - `Unsupported`: the program is outside the int-only subset. Loud
///   compile-time refusal (`run: JIT-FAIL` on the CLI); the program is
///   never executed.
/// - `Runtime`: compiled code hit the interpreter's error (overflow,
///   division/modulo by zero, depth guard, integer boundary). Carries
///   the identical [`Diagnostic`] plus the prints emitted before the
///   failure, so the CLI prints the same `run: FAIL` + JSON as the
///   interpreter.
#[derive(Debug)]
pub enum JitError {
    Unsupported(String),
    Runtime {
        diagnostic: Diagnostic,
        output: Vec<String>,
    },
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitError::Unsupported(s) => write!(f, "{s}"),
            JitError::Runtime { diagnostic, .. } => {
                write!(f, "{}: {}", diagnostic.code, diagnostic.message)
            }
        }
    }
}

/// Imported host helpers available to every compiled function.
struct JitImports {
    print: FuncId,
    add: FuncId,
    sub: FuncId,
    mul: FuncId,
    div: FuncId,
    rem: FuncId,
    neg: FuncId,
    enter: FuncId,
    leave: FuncId,
    check_bound: FuncId,
}

fn declare_import(
    module: &mut JITModule,
    name: &str,
    nargs: usize,
    has_ret: bool,
) -> Result<FuncId, JitError> {
    let mut sig = module.make_signature();
    for _ in 0..nargs {
        sig.params.push(AbiParam::new(types::I64));
    }
    if has_ret {
        sig.returns.push(AbiParam::new(types::I64));
    }
    module
        .declare_function(name, Linkage::Import, &sig)
        .map_err(|e| JitError::Unsupported(format!("declare {name} import: {e}")))
}

/// Compile `entry` in `mir` to machine code and run it.
/// `args` must match the entry's param count exactly.
///
/// Fallible entry point (Wave1 S5): out-of-subset programs are refused
/// with [`JitError::Unsupported`], while runtime failures carry the
/// interpreter's diagnostic plus partial output.
pub fn run_jit_fallible(
    mir: &MirModule,
    entry: &str,
    args: &[i64],
) -> Result<(i64, Vec<String>), JitError> {
    let _run = JIT_RUN.lock().unwrap();
    jit_output().lock().unwrap().clear();
    jit_reset();

    let mut builder = JITBuilder::new(cranelift_module::default_libcall_names())
        .map_err(|e| JitError::Unsupported(format!("jit builder: {e}")))?;
    builder.symbol("klang_print_i64", host_print_i64 as *const u8);
    builder.symbol("klang_jit_add", klang_jit_add as *const u8);
    builder.symbol("klang_jit_sub", klang_jit_sub as *const u8);
    builder.symbol("klang_jit_mul", klang_jit_mul as *const u8);
    builder.symbol("klang_jit_div", klang_jit_div as *const u8);
    builder.symbol("klang_jit_rem", klang_jit_rem as *const u8);
    builder.symbol("klang_jit_neg", klang_jit_neg as *const u8);
    builder.symbol("klang_jit_enter", klang_jit_enter as *const u8);
    builder.symbol("klang_jit_leave", klang_jit_leave as *const u8);
    builder.symbol(
        "klang_jit_check_bound",
        klang_jit_check_bound as *const u8,
    );
    let mut module = JITModule::new(builder);

    let imports = JitImports {
        print: declare_import(&mut module, "klang_print_i64", 1, false)?,
        add: declare_import(&mut module, "klang_jit_add", 2, true)?,
        sub: declare_import(&mut module, "klang_jit_sub", 2, true)?,
        mul: declare_import(&mut module, "klang_jit_mul", 2, true)?,
        div: declare_import(&mut module, "klang_jit_div", 2, true)?,
        rem: declare_import(&mut module, "klang_jit_rem", 2, true)?,
        neg: declare_import(&mut module, "klang_jit_neg", 1, true)?,
        enter: declare_import(&mut module, "klang_jit_enter", 0, true)?,
        leave: declare_import(&mut module, "klang_jit_leave", 0, false)?,
        check_bound: declare_import(&mut module, "klang_jit_check_bound", 2, true)?,
    };

    // Declare every function first so calls resolve (incl. recursion).
    // Lambda-lifted closure bodies (NUL-containing names, unspellable in
    // source) are never declared: no static `Call` can name them, and the
    // `ClosureNew` site that creates their values is itself unsupported
    // below — so any closure use fails loudly at the creation site, and
    // Cranelift never sees a NUL byte in a symbol name.
    let mut ids: HashMap<String, FuncId> = HashMap::new();
    for f in &mir.functions {
        if f.name.contains('\0') {
            continue;
        }        let mut sig = module.make_signature();
        for _ in &f.params {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = module
            .declare_function(&f.name, Linkage::Local, &sig)
            .map_err(|e| JitError::Unsupported(format!("declare {}: {e}", f.name)))?;
        ids.insert(f.name.clone(), id);
    }

    // Compile only functions reachable from `entry` through `Call` ops,
    // so uncalled dynamic functions (e.g. task demos) don't block JIT.
    // Unknown callees error only when reachable.
    let mut reachable: Vec<String> = vec![entry.to_string()];
    let mut seen: HashSet<String> = HashSet::from([entry.to_string()]);
    let mut i = 0;
    while i < reachable.len() {
        let name = reachable[i].clone();
        i += 1;
        let Some(mf) = mir.find(&name) else { continue };
        for ins in &mf.instrs {
            if let MirOp::Call { func, .. } = &ins.op {
                if !crate::hir::is_builtin(func) && seen.insert(func.clone()) {
                    reachable.push(func.clone());
                }
            }
        }
    }

    for name in &reachable {
        let f = mir
            .find(name)
            .ok_or_else(|| JitError::Unsupported(format!("jit: unknown entry `{name}`")))?;
        let id = ids[name];
        compile_func(&mut module, &imports, &ids, mir, f, id).map_err(JitError::Unsupported)?;
    }

    module
        .finalize_definitions()
        .map_err(|e| JitError::Unsupported(format!("finalize: {e}")))?;

    let entry_fn = mir
        .find(entry)
        .ok_or_else(|| JitError::Unsupported(format!("jit: unknown entry `{entry}`")))?;
    let code = module.get_finalized_function(ids[entry]);
    // SAFETY: arity is checked against the compiled signature below, so
    // each arm transmutes to exactly the function type being called.
    let n_params = entry_fn.params.len();
    if args.len() != n_params {
        return Err(JitError::Unsupported(format!(
            "jit phase-1: entry `{entry}` takes {n_params} args, got {}",
            args.len()
        )));
    }
    let v = if n_params == 0 {
        let f = unsafe { std::mem::transmute::<*const u8, fn() -> i64>(code) };
        f()
    } else if n_params == 1 {
        let f = unsafe { std::mem::transmute::<*const u8, fn(i64) -> i64>(code) };
        f(args[0])
    } else if n_params == 2 {
        let f = unsafe { std::mem::transmute::<*const u8, fn(i64, i64) -> i64>(code) };
        f(args[0], args[1])
    } else if n_params == 3 {
        let f = unsafe { std::mem::transmute::<*const u8, fn(i64, i64, i64) -> i64>(code) };
        f(args[0], args[1], args[2])
    } else if n_params == 4 {
        let f = unsafe { std::mem::transmute::<*const u8, fn(i64, i64, i64, i64) -> i64>(code) };
        f(args[0], args[1], args[2], args[3])
    } else {
        return Err(JitError::Unsupported(
            "jit phase-1: entry arity > 4 unsupported".to_string(),
        ));
    };
    let out = jit_output().lock().unwrap().clone();
    if jit_failed() {
        let d = JIT_DIAG.lock().unwrap().take().unwrap_or_else(|| {
            Diagnostic::error(
                "E-RUNTIME",
                "internal JIT error",
                "runtime",
                0,
                0,
                "runtime execution failed",
                &[],
                "runtime/execution",
            )
        });
        jit_reset();
        return Err(JitError::Runtime {
            diagnostic: d,
            output: out,
        });
    }
    // Balanced enter/leave leaves the counter at 0; reset defensively so
    // the next run (same process, e.g. the differential gates) starts clean.
    jit_reset();
    Ok((v, out))
}

/// Compile `entry` in `mir` to machine code and run it.
/// `args` must match the entry's param count exactly.
///
/// Compatibility shim over [`run_jit_fallible`]: both refusal kinds
/// surface as `Err(String)`. Prefer `run_jit_fallible` when the
/// diagnostic and the partial output matter.
pub fn run_jit(mir: &MirModule, entry: &str, args: &[i64]) -> Result<(i64, Vec<String>), String> {
    run_jit_fallible(mir, entry, args).map_err(|e| e.to_string())
}

fn op_name(op: &MirOp) -> &'static str {
    match op {
        MirOp::EnterGroup { .. } => "EnterGroup",
        MirOp::LeaveGroup { .. } => "LeaveGroup",
        MirOp::Spawn { .. } => "Spawn",
        MirOp::Await { .. } => "Await",
        MirOp::Call { .. } => "Call",
        // Closures are values the int-only JIT cannot represent: creation
        // snapshots arbitrary values and dispatch is dynamic. Rejected
        // loudly via the catch-all below (never silent wrong code).
        MirOp::ClosureNew { .. } => "ClosureNew",
        MirOp::Const { .. } => "Const",
        MirOp::ConstFloat { .. } => "ConstFloat",
        MirOp::ConstStr { .. } => "ConstStr",
        MirOp::Copy { .. } => "Copy",
        MirOp::Add { .. } => "Add",
        MirOp::Sub { .. } => "Sub",
        MirOp::Mul { .. } => "Mul",
        MirOp::Div { .. } => "Div",
        MirOp::Mod { .. } => "Mod",
        MirOp::Eq { .. } => "Eq",
        MirOp::NotEq { .. } => "NotEq",
        MirOp::Lt { .. } => "Lt",
        MirOp::LtEq { .. } => "LtEq",
        MirOp::Gt { .. } => "Gt",
        MirOp::GtEq { .. } => "GtEq",
        MirOp::And { .. } => "And",
        MirOp::Or { .. } => "Or",
        MirOp::Neg { .. } => "Neg",
        MirOp::Not { .. } => "Not",
        MirOp::ArrayNew { .. } => "ArrayNew",
        MirOp::MapNew { .. } => "MapNew",
        MirOp::MethodCall { .. } => "MethodCall",
        MirOp::Index { .. } => "Index",
        MirOp::StoreIndex { .. } => "StoreIndex",
        MirOp::FieldGet { .. } => "FieldGet",
        MirOp::FieldSet { .. } => "FieldSet",
        MirOp::StructNew { .. } => "StructNew",
        MirOp::Len { .. } => "Len",
        MirOp::Print { .. } => "Print",
        MirOp::JumpIfFalse { .. } => "JumpIfFalse",
        MirOp::Jump { .. } => "Jump",
        MirOp::Return { .. } => "Return",
        MirOp::Try { .. } => "Try",
    }
}

fn declare_var(
    vars: &mut HashMap<String, Variable>,
    next: &mut u32,
    fb: &mut FunctionBuilder,
    name: &str,
) -> Variable {
    if let Some(v) = vars.get(name) {
        return *v;
    }
    let v = Variable::from_u32(*next);
    *next += 1;
    fb.declare_var(v, types::I64);
    vars.insert(name.to_string(), v);
    v
}

#[allow(clippy::too_many_arguments)]
fn compile_func(
    module: &mut JITModule,
    imports: &JitImports,
    ids: &HashMap<String, FuncId>,
    mir: &MirModule,
    f: &crate::mir::MirFunction,
    id: FuncId,
) -> Result<(), String> {
    let mut ctx = module.make_context();
    let mut sig = module.make_signature();
    for _ in &f.params {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    ctx.func.signature = sig;

    let mut builder_ctx = FunctionBuilderContext::new();
    let mut fb = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
    let mut vars: HashMap<String, Variable> = HashMap::new();
    let mut defined: HashSet<String> = HashSet::new();
    let mut next: u32 = 0;
    // `last` mirrors the interpreter's fall-off-the-end value (documented
    // default 0). The value must dominate the `end` block even when it
    // comes from a conditionally-executed block (invalid Cranelift IR
    // otherwise). Track it in a Cranelift `Variable` (`last_var`) so
    // reads in `end` are SSA-safe via phi insertion; it is seeded in the
    // entry block (so empty functions, which jump straight to `end`,
    // read a defined variable) and re-seeded in body block 0 (preserving
    // the historical loop-back behavior).
    // `__last` cannot collide with user bindings (`__` is reserved).
    let last_var = {
        let v = Variable::from_u32(next);
        next += 1;
        fb.declare_var(v, types::I64);
        // Null byte is unrepresentable in Klang identifiers, so this key
        // can never collide with a user binding (even `__last`).
        vars.insert("\0last".to_string(), v);
        v
    };

    // Reads a binding, planting an explicit 0 the first time a name is
    // read before any definition (mirrors `unwrap_or(Int(0))`).
    macro_rules! use_v {
        ($name:expr) => {{
            let v = declare_var(&mut vars, &mut next, &mut fb, $name);
            if defined.insert($name.to_string()) {
                let z = fb.ins().iconst(types::I64, 0);
                fb.def_var(v, z);
            }
            fb.use_var(v)
        }};
    }
    macro_rules! def_v {
        ($name:expr, $val:expr) => {{
            let v = declare_var(&mut vars, &mut next, &mut fb, $name);
            defined.insert($name.to_string());
            fb.def_var(v, $val);
        }};
    }
    // Operands bind before `ins()` so no `&mut fb` borrow from `use_v!`
    // is live across the builder call. Every defining op also updates
    // `last_var` so the `end` block reads a dominating Variable, never a
    // conditionally-defined SSA value.
    //
    // Branch to `err_b` when a host helper recorded a runtime failure
    // (overflow, division by zero, boundary, ...), else fall through.
    // Every checked op ends with this plus `continue` (replacing the
    // trailing unconditional jump).
    macro_rules! err_check {
        ($err_b:expr, $fallthrough:expr) => {{
            let slot = fb.ins().iconst(types::I64, JIT_FAIL_FLAG.as_ptr() as i64);
            let flag = fb.ins().load(types::I64, MemFlags::new(), slot, 0);
            let zero = fb.ins().iconst(types::I64, 0);
            let bad = fb.ins().icmp(IntCC::NotEqual, flag, zero);
            fb.ins().brif(bad, $err_b, &[], $fallthrough, &[]);
        }};
    }
    // One checked binary op through its host helper (same `checked_*`
    // semantics as the interpreter; the helper records the diagnostic).
    macro_rules! checked_call2 {
        ($into:expr, $helper:expr, $left:expr, $right:expr, $err_b:expr, $fallthrough:expr) => {{
            let l = use_v!($left);
            let r = use_v!($right);
            let href = module.declare_func_in_func($helper, fb.func);
            let call = fb.ins().call(href, &[l, r]);
            let v = fb.inst_results(call)[0];
            def_v!($into, v);
            fb.def_var(last_var, v);
            err_check!($err_b, $fallthrough);
        }};
    }
    macro_rules! cmp_v {
        ($into:expr, $left:expr, $right:expr, $cc:expr) => {{
            let l = use_v!($left);
            let r = use_v!($right);
            // `icmp` yields a narrow boolean; widen to I64 0/1 like the
            // interpreter's `Value::Int(i64::from(...))`.
            let c = fb.ins().icmp($cc, l, r);
            let v = fb.ins().uextend(types::I64, c);
            def_v!($into, v);
            fb.def_var(last_var, v);
        }};
    }

    // D3 boundaries, mirroring the interpreter exactly (`exec_function`
    // param loop, `check_return`, `Copy` via `var_ann`): each narrow
    // position gets a compile-time site id; compiled code calls
    // `klang_jit_check_bound`, which reports through the same
    // `check_int_boundary` constructor. Struct/enum field boundaries need
    // no code here: `StructNew`/`FieldGet`/`FieldSet` are already loud
    // `JIT-FAIL` rejections, so such programs never run on the JIT.
    let var_ann: HashMap<String, String> = f
        .params
        .iter()
        .zip(f.param_tys.iter())
        .filter(|(_, ty)| crate::runtime::int_range_for(ty).is_some())
        .map(|(p, ty)| (p.clone(), ty.clone()))
        .collect();
    let mut param_sites: Vec<(String, i64)> = Vec::new();
    for (i, (p, ty)) in f.params.iter().zip(f.param_tys.iter()).enumerate() {
        if crate::runtime::int_range_for(ty).is_some() {
            let site = jit_register_site(ty, format!("`{}` arg {i}", f.name));
            param_sites.push((p.clone(), site));
        }
    }
    let ret_site: Option<i64> = if crate::runtime::int_range_for(&f.return_ty).is_some() {
        Some(jit_register_site(&f.return_ty, format!("`{}` return", f.name)))
    } else {
        None
    };

    let n = f.instrs.len();
    // Layout-first block stays the ABI entry (receives the fn params).
    // It seeds the params, runs the depth guard, then jumps through the
    // param-boundary chain into the body. Loop back-edges to target 0
    // resume at the first body block: params are NOT re-seeded and the
    // entry guards do NOT re-run — an iteration is not a new call.
    let entry_b = fb.create_block();
    let mut blocks: Vec<Block> = Vec::with_capacity(n);
    for _ in 0..n {
        blocks.push(fb.create_block());
    }
    let end = fb.create_block();
    let err_b = fb.create_block();
    let mut check_blocks: Vec<Block> = Vec::with_capacity(param_sites.len());
    for _ in &param_sites {
        check_blocks.push(fb.create_block());
    }
    // Single-predecessor value carries for deferred `return`s (the value
    // handle is `Copy`; dominance holds by construction).
    let mut ret_blocks: Vec<(Block, IrValue)> = Vec::new();
    let mut tail_ok: Option<(Block, IrValue)> = None;

    let body_first: Block = if n > 0 { blocks[0] } else { end };
    let chain_first: Block = if !check_blocks.is_empty() {
        check_blocks[0]
    } else {
        body_first
    };

    // Entry: seed params, seed the fall-off default, run the depth guard.
    fb.switch_to_block(entry_b);
    fb.append_block_params_for_function_params(entry_b);
    {
        let mut pv = Vec::new();
        for k in 0..f.params.len() {
            pv.push(fb.block_params(entry_b)[k]);
        }
        for (name, val) in f.params.iter().zip(pv) {
            let v = declare_var(&mut vars, &mut next, &mut fb, name);
            defined.insert(name.clone());
            fb.def_var(v, val);
        }
        // Fall-off-the-end yields 0 (documented interpreter rule); a
        // dominating seed so `end` always reads a defined variable
        // (empty functions reach `end` without any body block).
        let z = fb.ins().iconst(types::I64, 0);
        fb.def_var(last_var, z);
    }
    {
        // Depth guard first, exactly like `exec_function` (depth check
        // before arity/boundaries). The branch is mandatory: without it
        // a self-recursive call would descend natively past the guard
        // instead of diverting (flag checks only run between calls).
        let eref = module.declare_func_in_func(imports.enter, fb.func);
        let ecall = fb.ins().call(eref, &[]);
        let estat = fb.inst_results(ecall)[0];
        let zero = fb.ins().iconst(types::I64, 0);
        let failed = fb.ins().icmp(IntCC::NotEqual, estat, zero);
        fb.ins().brif(failed, err_b, &[], chain_first, &[]);
    }

    // Param-boundary chain: one block per narrow param.
    for (j, (pname, site)) in param_sites.iter().enumerate() {
        let cb = check_blocks[j];
        fb.switch_to_block(cb);
        let next_b = if j + 1 < check_blocks.len() {
            check_blocks[j + 1]
        } else {
            body_first
        };
        let cur = use_v!(pname);
        let cref = module.declare_func_in_func(imports.check_bound, fb.func);
        let site_v = fb.ins().iconst(types::I64, *site);
        let call = fb.ins().call(cref, &[site_v, cur]);
        let v = fb.inst_results(call)[0];
        def_v!(pname, v);
        err_check!(err_b, next_b);
    }

    // Jump targets may equal `n` (past-the-end = halt, e.g. `if` without
    // `else`); those map to the `end` block returning `last_var`.
    let target_block = |t: usize| -> Block {
        if t < n {
            blocks[t]
        } else {
            end
        }
    };
    for (i, ins) in f.instrs.iter().enumerate() {
        fb.switch_to_block(blocks[i]);
        if i == 0 {
            // Params were seeded in `entry_b`; re-seed the fall-off
            // default here exactly as before (loop back-edges to target
            // 0 re-run this block, never the entry guards).
            let z = fb.ins().iconst(types::I64, 0);
            fb.def_var(last_var, z);
        }
        let fallthrough = if i + 1 < n { blocks[i + 1] } else { end };
        match &ins.op {
            MirOp::Const { into, value } => {
                let c = fb.ins().iconst(types::I64, *value);
                def_v!(into, c);
                fb.def_var(last_var, c);
            }
            MirOp::ConstFloat { .. } | MirOp::ConstStr { .. } => {
                return Err(format!(
                    "jit phase-1 supports int-only MIR; unsupported op {} in `{}`",
                    op_name(&ins.op),
                    f.name
                ));
            }
            MirOp::Copy { into, from } => {
                let mut v = use_v!(from);
                let checked = var_ann.contains_key(into);
                if checked {
                    // D3 assignment boundary, same message as the
                    // interpreter (`assignment to `<name>``).
                    let ty = var_ann.get(into).expect("checked above");
                    let site = jit_register_site(ty, format!("assignment to `{into}`"));
                    let cref = module.declare_func_in_func(imports.check_bound, fb.func);
                    let site_v = fb.ins().iconst(types::I64, site);
                    let call = fb.ins().call(cref, &[site_v, v]);
                    v = fb.inst_results(call)[0];
                }
                def_v!(into, v);
                fb.def_var(last_var, v);
                if checked {
                    err_check!(err_b, fallthrough);
                    continue;
                }
            }
            MirOp::Add { into, left, right } => {
                checked_call2!(into, imports.add, left, right, err_b, fallthrough);
                continue;
            }
            MirOp::Sub { into, left, right } => {
                checked_call2!(into, imports.sub, left, right, err_b, fallthrough);
                continue;
            }
            MirOp::Mul { into, left, right } => {
                checked_call2!(into, imports.mul, left, right, err_b, fallthrough);
                continue;
            }
            MirOp::Div { into, left, right } => {
                checked_call2!(into, imports.div, left, right, err_b, fallthrough);
                continue;
            }
            MirOp::Mod { into, left, right } => {
                checked_call2!(into, imports.rem, left, right, err_b, fallthrough);
                continue;
            }
            MirOp::Eq { into, left, right } => cmp_v!(into, left, right, IntCC::Equal),
            MirOp::NotEq { into, left, right } => cmp_v!(into, left, right, IntCC::NotEqual),
            MirOp::Lt { into, left, right } => cmp_v!(into, left, right, IntCC::SignedLessThan),
            MirOp::LtEq { into, left, right } => {
                cmp_v!(into, left, right, IntCC::SignedLessThanOrEqual)
            }
            MirOp::Gt { into, left, right } => {
                cmp_v!(into, left, right, IntCC::SignedGreaterThan)
            }
            MirOp::GtEq { into, left, right } => {
                cmp_v!(into, left, right, IntCC::SignedGreaterThanOrEqual)
            }
            MirOp::And { into, left, right } => {
                let l = use_v!(left);
                let r = use_v!(right);
                let zero = fb.ins().iconst(types::I64, 0);
                let bl = fb.ins().icmp(IntCC::NotEqual, l, zero);
                let br = fb.ins().icmp(IntCC::NotEqual, r, zero);
                let b = fb.ins().band(bl, br);
                let v = fb.ins().uextend(types::I64, b);
                def_v!(into, v);
                fb.def_var(last_var, v);
            }
            MirOp::Or { into, left, right } => {
                let l = use_v!(left);
                let r = use_v!(right);
                let zero = fb.ins().iconst(types::I64, 0);
                let bl = fb.ins().icmp(IntCC::NotEqual, l, zero);
                let br = fb.ins().icmp(IntCC::NotEqual, r, zero);
                let b = fb.ins().bor(bl, br);
                let v = fb.ins().uextend(types::I64, b);
                def_v!(into, v);
                fb.def_var(last_var, v);
            }
            MirOp::Neg { into, inner } => {
                let x = use_v!(inner);
                let nref = module.declare_func_in_func(imports.neg, fb.func);
                let call = fb.ins().call(nref, &[x]);
                let v = fb.inst_results(call)[0];
                def_v!(into, v);
                fb.def_var(last_var, v);
                err_check!(err_b, fallthrough);
                continue;
            }
            MirOp::Not { into, inner } => {
                let x = use_v!(inner);
                let zero = fb.ins().iconst(types::I64, 0);
                let c = fb.ins().icmp(IntCC::Equal, x, zero);
                let v = fb.ins().uextend(types::I64, c);
                def_v!(into, v);
                fb.def_var(last_var, v);
            }
            MirOp::Call { into, func, args } => {
                if crate::hir::is_builtin(func) {
                    return Err(format!(
                        "jit phase-1 supports int-only MIR; builtin `{func}` unsupported in `{}`",
                        f.name
                    ));
                }
                let callee = ids.get(func).ok_or_else(|| {
                    format!(
                        "jit phase-1: unknown function `{func}` called from `{}`",
                        f.name
                    )
                })?;
                let callee_fn = mir.find(func).ok_or_else(|| {
                    format!(
                        "jit phase-1: unknown function `{func}` called from `{}`",
                        f.name
                    )
                })?;
                let func_ref = module.declare_func_in_func(*callee, fb.func);
                // Missing args read 0, extras ignored (mirrors zip in calls).
                let mut call_args = Vec::with_capacity(callee_fn.params.len());
                for (i, _) in callee_fn.params.iter().enumerate() {
                    if i < args.len() {
                        call_args.push(use_v!(&args[i]));
                    } else {
                        call_args.push(fb.ins().iconst(types::I64, 0));
                    }
                }
                let call = fb.ins().call(func_ref, &call_args);
                let v = fb.inst_results(call)[0];
                def_v!(into, v);
                fb.def_var(last_var, v);
                // The callee may have recorded a runtime failure (its own
                // error block returns immediately); divert before touching
                // the value.
                err_check!(err_b, fallthrough);
                continue;
            }
            MirOp::Print { value } => {
                let v = use_v!(value);
                let func_ref = module.declare_func_in_func(imports.print, fb.func);
                fb.ins().call(func_ref, &[v]);
                fb.def_var(last_var, v);
            }
            MirOp::JumpIfFalse { cond, target } => {
                let c = use_v!(cond);
                let zero = fb.ins().iconst(types::I64, 0);
                let nz = fb.ins().icmp(IntCC::NotEqual, c, zero);
                fb.ins()
                    .brif(nz, fallthrough, &[], target_block(*target), &[]);
                continue;
            }
            MirOp::Jump { target } => {
                let t = target_block(*target);
                fb.ins().jump(t, &[]);
                continue;
            }
            MirOp::Return { value } => {
                let mut v = use_v!(value);
                if let Some(site) = ret_site {
                    // D3 return boundary, same message as the interpreter.
                    let cref = module.declare_func_in_func(imports.check_bound, fb.func);
                    let site_v = fb.ins().iconst(types::I64, site);
                    let call = fb.ins().call(cref, &[site_v, v]);
                    v = fb.inst_results(call)[0];
                }
                // Normal-return path releases the depth slot; error paths
                // skip this (the run aborts and the counter resets).
                let lref = module.declare_func_in_func(imports.leave, fb.func);
                fb.ins().call(lref, &[]);
                if ret_site.is_some() {
                    let rb = fb.create_block();
                    ret_blocks.push((rb, v));
                    err_check!(err_b, rb);
                    continue;
                }
                fb.ins().return_(&[v]);
                continue;
            }
            MirOp::EnterGroup { .. } | MirOp::LeaveGroup { .. } => {
                fb.ins().jump(fallthrough, &[]);
                continue;
            }
            other => {
                return Err(format!(
                    "jit phase-1 supports int-only MIR; unsupported op {} in `{}`",
                    op_name(other),
                    f.name
                ));
            }
        }
        fb.ins().jump(fallthrough, &[]);
    }

    fb.switch_to_block(end);
    // SSA-safe: read the dominating `last` variable (phi-inserted across
    // conditional predecessors), never a conditionally-defined value.
    // The fall-off value gets the same return-boundary check and depth
    // release as an explicit `return`.
    let mut tail_v = fb.use_var(last_var);
    if let Some(site) = ret_site {
        let cref = module.declare_func_in_func(imports.check_bound, fb.func);
        let site_v = fb.ins().iconst(types::I64, site);
        let call = fb.ins().call(cref, &[site_v, tail_v]);
        tail_v = fb.inst_results(call)[0];
    }
    {
        let lref = module.declare_func_in_func(imports.leave, fb.func);
        fb.ins().call(lref, &[]);
    }
    if ret_site.is_some() {
        let ok = fb.create_block();
        tail_ok = Some((ok, tail_v));
        err_check!(err_b, ok);
    } else {
        fb.ins().return_(&[tail_v]);
    }

    // Error block: value ignored (`run_jit_fallible` reports the recorded
    // diagnostic); no depth release (the run aborts, counter resets).
    fb.switch_to_block(err_b);
    {
        let z = fb.ins().iconst(types::I64, 0);
        fb.ins().return_(&[z]);
    }

    let ret_ids: Vec<Block> = ret_blocks.iter().map(|(b, _)| *b).collect();
    for (rb, v) in ret_blocks {
        fb.switch_to_block(rb);
        fb.ins().return_(&[v]);
    }
    let tail_id = tail_ok.map(|(b, _)| b);
    if let Some((ok, v)) = tail_ok {
        fb.switch_to_block(ok);
        fb.ins().return_(&[v]);
    }

    fb.seal_block(entry_b);
    for b in check_blocks.iter() {
        fb.seal_block(*b);
    }
    for b in blocks.iter().chain(std::iter::once(&end)) {
        fb.seal_block(*b);
    }
    fb.seal_block(err_b);
    for b in ret_ids.iter() {
        fb.seal_block(*b);
    }
    if let Some(ok) = tail_id {
        fb.seal_block(ok);
    }
    fb.finalize();
    module
        .define_function(id, &mut ctx)
        .map_err(|e| format!("define {}: {e}", f.name))?;
    module.clear_context(&mut ctx);
    Ok(())
}
