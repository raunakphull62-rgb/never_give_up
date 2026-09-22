//! Step 1 backend: Cranelift JIT (Phase 0 probe + Phase 1 int-only).
//!
//! Phase 0: prove `cranelift-jit` builds and runs here — JIT-compile
//! `add(i64, i64) -> i64`, call it with (7, 35), expect 42.
//! Phase 1 (next): lower the int-only MIR subset through the same
//! `JITModule`/`FunctionBuilder` path, behind `--backend jit`.

use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use cranelift_codegen::ir::{types, AbiParam, InstBuilder};

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
// Two known divergences from the interpreter, both documented:
// - `Div`/`Mod` by zero traps (process aborts) instead of `E-RUNTIME`.
//   (Same fail-fast spirit as Rust's own `/` panic; a checked-div host
//   call with error propagation lands in a later phase.)
// - Calls require all callees defined in the module (no stub fallback)
//   and entry arity must match exactly (calling-convention safety).
// ---------------------------------------------------------------------------

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::Block;
use cranelift_frontend::Variable;
use cranelift_module::FuncId;

use crate::mir::{MirModule, MirOp};

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

/// Compile `entry` in `mir` to machine code and run it.
/// `args` must match the entry's param count exactly.
pub fn run_jit(mir: &MirModule, entry: &str, args: &[i64]) -> Result<(i64, Vec<String>), String> {
    let _run = JIT_RUN.lock().unwrap();
    jit_output().lock().unwrap().clear();

    let mut builder = JITBuilder::new(cranelift_module::default_libcall_names())
        .map_err(|e| format!("jit builder: {e}"))?;
    builder.symbol(
        "klang_print_i64",
        host_print_i64 as *const u8,
    );
    let mut module = JITModule::new(builder);

    let mut print_sig = module.make_signature();
    print_sig.params.push(AbiParam::new(types::I64));
    let print_id = module
        .declare_function("klang_print_i64", Linkage::Import, &print_sig)
        .map_err(|e| format!("declare print import: {e}"))?;

    // Declare every function first so calls resolve (incl. recursion).
    let mut ids: HashMap<String, FuncId> = HashMap::new();
    for f in &mir.functions {
        let mut sig = module.make_signature();
        for _ in &f.params {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = module
            .declare_function(&f.name, Linkage::Local, &sig)
            .map_err(|e| format!("declare {}: {e}", f.name))?;
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
            .ok_or_else(|| format!("jit: unknown entry `{name}`"))?;
        let id = ids[name];
        compile_func(&mut module, print_id, &ids, mir, f, id)?;
    }

    module
        .finalize_definitions()
        .map_err(|e| format!("finalize: {e}"))?;

    let entry_fn = mir
        .find(entry)
        .ok_or_else(|| format!("jit: unknown entry `{entry}`"))?;
    let code = module.get_finalized_function(ids[entry]);
    // SAFETY: arity is checked against the compiled signature below, so
    // each arm transmutes to exactly the function type being called.
    let n_params = entry_fn.params.len();
    if args.len() != n_params {
        return Err(format!(
            "jit phase-1: entry `{entry}` takes {n_params} args, got {}",
            args.len()
        ));
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
        let f =
            unsafe { std::mem::transmute::<*const u8, fn(i64, i64, i64) -> i64>(code) };
        f(args[0], args[1], args[2])
    } else if n_params == 4 {
        let f =
            unsafe { std::mem::transmute::<*const u8, fn(i64, i64, i64, i64) -> i64>(code) };
        f(args[0], args[1], args[2], args[3])
    } else {
        return Err("jit phase-1: entry arity > 4 unsupported".to_string());
    };
    let out = jit_output().lock().unwrap().clone();
    Ok((v, out))
}

fn op_name(op: &MirOp) -> &'static str {
    match op {
        MirOp::EnterGroup { .. } => "EnterGroup",
        MirOp::LeaveGroup { .. } => "LeaveGroup",
        MirOp::Spawn { .. } => "Spawn",
        MirOp::Await { .. } => "Await",
        MirOp::Call { .. } => "Call",
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
    print_id: FuncId,
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
    // default 0). The Rust-side `Option<Value>` alone does NOT dominate the
    // `end` block when the final value comes from a conditionally-executed
    // block (invalid Cranelift IR). Track it in a Cranelift `Variable`
    // (`last_var`) so reads in `end` are SSA-safe via phi insertion; the
    // Option is kept only for the empty-function fast path below.
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
    macro_rules! arith_v {
        ($into:expr, $left:expr, $right:expr, $op:ident) => {{
            let l = use_v!($left);
            let r = use_v!($right);
            let v = fb.ins().$op(l, r);
            def_v!($into, v);
            fb.def_var(last_var, v);
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

    let n = f.instrs.len();
    if n == 0 {
        let b = fb.create_block();
        fb.switch_to_block(b);
        fb.seal_block(b);
        let z = fb.ins().iconst(types::I64, 0);
        fb.ins().return_(&[z]);
        fb.finalize();
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("define {}: {e}", f.name))?;
        module.clear_context(&mut ctx);
        return Ok(());
    }

    let mut blocks: Vec<Block> = Vec::with_capacity(n);
    for _ in 0..n {
        blocks.push(fb.create_block());
    }
    let end = fb.create_block();

    // Params live in block 0. Declare block params up front; the actual
    // `def_var` seeding happens after switching to block 0 inside the loop
    // below (emitting any instruction before the first switch would leave
    // block 0 non-empty yet unterminated, tripping "fill before switching").
    fb.append_block_params_for_function_params(blocks[0]);

    // Jump targets may equal `n` (past-the-end = halt, e.g. `if` without
    // `else`); those map to the `end` block returning `last_var`.
    let target_block = |t: usize| -> Block { if t < n { blocks[t] } else { end } };
    for (i, ins) in f.instrs.iter().enumerate() {
        fb.switch_to_block(blocks[i]);
        if i == 0 {
            let mut pv = Vec::new();
            for k in 0..f.params.len() {
                pv.push(fb.block_params(blocks[0])[k]);
            }
            for (name, val) in f.params.iter().zip(pv) {
                let v = declare_var(&mut vars, &mut next, &mut fb, name);
                defined.insert(name.clone());
                fb.def_var(v, val);
            }
            // Fall-off-the-end yields 0 (documented interpreter rule); seed
            // the `last` variable so `end` always reads a dominating
            // definition.
            {
                let z = fb.ins().iconst(types::I64, 0);
                fb.def_var(last_var, z);
            }
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
                let v = use_v!(from);
                def_v!(into, v);
                fb.def_var(last_var, v);
            }
            MirOp::Add { into, left, right } => arith_v!(into, left, right, iadd),
            MirOp::Sub { into, left, right } => arith_v!(into, left, right, isub),
            MirOp::Mul { into, left, right } => arith_v!(into, left, right, imul),
            MirOp::Div { into, left, right } => {
                // Traps on zero divisor (documented phase-1 divergence).
                arith_v!(into, left, right, sdiv)
            }
            MirOp::Mod { into, left, right } => arith_v!(into, left, right, srem),
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
                let v = fb.ins().ineg(x);
                def_v!(into, v);
                fb.def_var(last_var, v);
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
                    format!("jit phase-1: unknown function `{func}` called from `{}`", f.name)
                })?;
                let callee_fn = mir.find(func).ok_or_else(|| {
                    format!("jit phase-1: unknown function `{func}` called from `{}`", f.name)
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
            }
            MirOp::Print { value } => {
                let v = use_v!(value);
                let func_ref = module.declare_func_in_func(print_id, fb.func);
                fb.ins().call(func_ref, &[v]);
                fb.def_var(last_var, v);
            }
            MirOp::JumpIfFalse { cond, target } => {
                let c = use_v!(cond);
                let zero = fb.ins().iconst(types::I64, 0);
                let nz = fb.ins().icmp(IntCC::NotEqual, c, zero);
                fb.ins().brif(nz, fallthrough, &[], target_block(*target), &[]);
                continue;
            }
            MirOp::Jump { target } => {
                let t = target_block(*target);
                fb.ins().jump(t, &[]);
                continue;
            }
            MirOp::Return { value } => {
                let v = use_v!(value);
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
    {
        let v = fb.use_var(last_var);
        fb.ins().return_(&[v]);
    }

    for b in blocks.iter().chain(std::iter::once(&end)) {
        fb.seal_block(*b);
    }
    fb.finalize();
    module
        .define_function(id, &mut ctx)
        .map_err(|e| format!("define {}: {e}", f.name))?;
    module.clear_context(&mut ctx);
    Ok(())
}
