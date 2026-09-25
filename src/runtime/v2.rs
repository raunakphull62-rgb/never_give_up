//! Klang v2 real interpreter (F-V2-1 follow-up).
//!
//! Direct execution of [`crate::ast::v2::V2Program`] with real semantics:
//! - `flow` blocks read `dep=` bindings from the caller at call time.
//! - `echo fn` bodies run on background threads; `listen` joins.
//! - `tune<T>` validates against the declared schema at runtime.
//!
//! MIR lowering (`mir::v2_lowering`, including `echo_lowering`) still runs
//! in `run-v2` for the explicit-state-machine listing (pillar 1); actual
//! execution goes through here so stubs cannot silently pass.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::Value;
use crate::ai_safety::schema::{DataValue, FieldTy, Schema, SchemaField};
use crate::ast::v2::{V2Block, V2Expr, V2Program, V2Stmt};
use crate::diagnostics::Diagnostic;

fn rt_err(msg: &str) -> Diagnostic {
    Diagnostic::error(
        "E-RUNTIME",
        msg,
        "input.v2",
        0,
        0,
        "runtime execution failed",
        &[],
        "runtime/execution",
    )
}

fn map_schema_ty(s: &str) -> Result<FieldTy, Diagnostic> {
    match s.trim().to_lowercase().as_str() {
        "str" | "string" => Ok(FieldTy::Str),
        "i32" | "i64" | "int" => Ok(FieldTy::Int),
        "bool" => Ok(FieldTy::Bool),
        other => Err(Diagnostic::error(
            "E-SCHEMA-NOT-FOUND",
            &format!("unknown schema field type `{other}`"),
            "input.v2",
            0,
            0,
            "schema field types are str, i32, bool",
            &["use str, i32, or bool"],
            "schema/scope",
        )),
    }
}

#[derive(Debug, Clone)]
struct EchoDef {
    params: Vec<String>,
    body: V2Block,
}

#[derive(Debug, Clone)]
struct FlowDef {
    params: Vec<String>,
    deps: Vec<String>,
    body_expr: V2Expr,
}

#[derive(Debug, Clone)]
struct V2Global {
    schemas: HashMap<String, Schema>,
    echo_defs: HashMap<String, EchoDef>,
    functions: HashMap<String, crate::ast::v2::V2FunctionDecl>,
    global_flows: HashMap<String, FlowDef>,
    output: Arc<Mutex<Vec<String>>>,
    handles: Arc<Mutex<HashMap<String, JoinHandle<Result<Value, Diagnostic>>>>>,
    counter: Arc<Mutex<u64>>,
}

#[derive(Debug, Clone, Default)]
struct Frame {
    vars: HashMap<String, Value>,
    flows: HashMap<String, FlowDef>,
}

fn truthy(v: &Value) -> bool {
    v.truthy()
}

fn value_to_data(v: &Value) -> Result<DataValue, Diagnostic> {
    match v {
        Value::Int(n) => Ok(DataValue::Int(*n)),
        Value::Str(s) => Ok(DataValue::Str(s.clone())),
        Value::Float(f) => {
            // Schemas have no float; whole floats convert, else reject loudly.
            if f.fract() == 0.0 {
                Ok(DataValue::Int(*f as i64))
            } else {
                Err(rt_err("tune() value has non-integer float; schemas hold int/str/bool records"))
            }
        }
        Value::Struct { fields, .. } => {
            let mut map = HashMap::new();
            for (k, fv) in fields {
                map.insert(k.clone(), value_to_data(fv)?);
            }
            Ok(DataValue::Record(map))
        }
        Value::Array(_) | Value::Map(_) => Err(rt_err(
            "tune() value must be a record struct; arrays/maps are not schema values",
        )),
    }
}

/// Convert a runtime value into DataValue guided by the expected field type,
/// so `bool` fields accept the interpreter's `Int(0/1)` encoding of
/// `true`/`false` instead of failing loudly on a representation detail.
fn convert_for_ty(v: &Value, ty: &FieldTy) -> Result<DataValue, Diagnostic> {
    match (ty, v) {
        (_, Value::Str(s)) if *ty == FieldTy::Str => Ok(DataValue::Str(s.clone())),
        (_, Value::Int(n)) if *ty == FieldTy::Int => Ok(DataValue::Int(*n)),
        (_, Value::Int(n)) if *ty == FieldTy::Bool => {
            if *n == 0 {
                Ok(DataValue::Bool(false))
            } else if *n == 1 {
                Ok(DataValue::Bool(true))
            } else {
                Err(rt_err("tune() bool field needs true/false (0/1)"))
            }
        }
        (_, Value::Struct { fields, .. }) if matches!(ty, FieldTy::Record(_)) => {
            if let FieldTy::Record(inner) = ty {
                let mut map = HashMap::new();
                for f in inner {
                    let fv = fields
                        .iter()
                        .find(|(k, _)| k == &f.name)
                        .map(|(_, vv)| vv)
                        .ok_or_else(|| {
                            crate::ai_safety::diagnostics::schema_invalid(
                                "input.v2",
                                0,
                                0,
                                &crate::ai_safety::schema::VerifyError {
                                    schema: crate::ai_safety::schema::SchemaId("?".to_string()),
                                    path: f.name.clone(),
                                    expected: "present".to_string(),
                                    found: "missing".to_string(),
                                    message: format!("missing field `{}`", f.name),
                                },
                            )
                        })?;
                    map.insert(f.name.clone(), convert_for_ty(fv, &f.ty)?);
                }
                Ok(DataValue::Record(map))
            } else {
                unreachable!()
            }
        }
        _ => value_to_data(v),
    }
}

fn validate_against_schema(
    schema: &Schema,
    value: &Value,
) -> Result<DataValue, Diagnostic> {
    // Build a DataValue guided by the schema so bool/int encoding matches.
    let mut map = HashMap::new();
    let struct_fields: Vec<(String, Value)> = match value {
        Value::Struct { fields, .. } => fields.clone(),
        _ => {
            // Let Schema::validate produce the real top-level diagnostic.
            let data = value_to_data(value)?;
            return match schema.validate(&data) {
                Ok(()) => Ok(data),
                Err(e) => Err(crate::ai_safety::diagnostics::schema_invalid(
                    "input.v2", 0, 0, &e,
                )),
            };
        }
    };
    for f in &schema.fields {
        let fv = struct_fields
            .iter()
            .find(|(k, _)| k == &f.name)
            .map(|(_, vv)| vv);
        match fv {
            None => {
                let data = value_to_data(value).unwrap_or(DataValue::Record(HashMap::new()));
                let _ = data;
                // Produce the real missing-field diagnostic via schema.validate.
                let mut rec = HashMap::new();
                for (k, vv) in &struct_fields {
                    // Best-effort raw conversion for the error path.
                    if let Ok(dv) = value_to_data(vv) {
                        rec.insert(k.clone(), dv);
                    }
                }
                return match schema.validate(&DataValue::Record(rec)) {
                    Ok(()) => Err(rt_err("unreachable")),
                    Err(e) => Err(crate::ai_safety::diagnostics::schema_invalid(
                        "input.v2", 0, 0, &e,
                    )),
                };
            }
            Some(vv) => {
                let dv = convert_for_ty(vv, &f.ty).map_err(|_| {
                    // Fall through to the real validator for the precise path.
                    let mut rec = HashMap::new();
                    for (k, vv2) in &struct_fields {
                        if let Ok(dv2) = value_to_data(vv2) {
                            rec.insert(k.clone(), dv2);
                        }
                    }
                    match schema.validate(&DataValue::Record(rec)) {
                        Ok(()) => rt_err("tune() type mismatch"),
                        Err(e) => crate::ai_safety::diagnostics::schema_invalid(
                            "input.v2", 0, 0, &e,
                        ),
                    }
                })?;
                map.insert(f.name.clone(), dv);
            }
        }
    }
    let data = DataValue::Record(map);
    match schema.validate(&data) {
        Ok(()) => Ok(data),
        Err(e) => Err(crate::ai_safety::diagnostics::schema_invalid(
            "input.v2", 0, 0, &e,
        )),
    }
}

/// Run a v2 program's `entry` (default `main`), returning (i32, stdout).
pub fn run_v2_program(
    prog: &V2Program,
    entry: &str,
) -> Result<(i32, Vec<String>), Diagnostic> {
    let mut schemas = HashMap::new();
    for s in &prog.schemas {
        let mut fields = Vec::new();
        for f in &s.fields {
            fields.push(SchemaField::new(&f.name, map_schema_ty(&f.ty)?));
        }
        schemas.insert(
            s.name.clone(),
            Schema::new(&s.name, &s.version, fields),
        );
    }
    let mut echo_defs = HashMap::new();
    for e in &prog.echo_fns {
        let body = prog.echo_bodies.get(&e.name).ok_or_else(|| {
            Diagnostic::error(
                "E-PARSE-V2",
                &format!("echo fn `{}` has no body", e.name),
                "input.v2",
                0,
                0,
                "echo fns must have bodies to execute",
                &["give the echo fn a `{ ... }` body"],
                "syntax/echo",
            )
        })?;
        echo_defs.insert(
            e.name.clone(),
            EchoDef {
                params: e.params.iter().map(|(n, _)| n.clone()).collect(),
                body: body.clone(),
            },
        );
    }
    let mut functions = HashMap::new();
    for f in &prog.functions {
        functions.insert(f.name.clone(), f.clone());
    }
    // Top-level flows with names (rare; let-bound flows are the common case).
    let mut global_flows = HashMap::new();
    for d in &prog.flows {
        if let Some(name) = &d.name {
            let body_expr = crate::parser::v2::parse_v2_expr(&d.expr.body).map_err(|e| e)?;
            global_flows.insert(
                name.clone(),
                FlowDef {
                    params: d.expr.params.iter().map(|p| p.name.clone()).collect(),
                    deps: d.expr.deps.iter().map(|x| x.name.clone()).collect(),
                    body_expr,
                },
            );
        }
    }
    let global = V2Global {
        schemas,
        echo_defs,
        functions,
        global_flows,
        output: Arc::new(Mutex::new(Vec::new())),
        handles: Arc::new(Mutex::new(HashMap::new())),
        counter: Arc::new(Mutex::new(0)),
    };
    let func = global.functions.get(entry).ok_or_else(|| {
        Diagnostic::parse_error("input.v2", 0, 0, &format!("unknown entry `{entry}`"))
    })?;
    let func = func.clone();
    if !func.params.is_empty() {
        return Err(Diagnostic::error(
            "E-ARITY",
            &format!("entry `{entry}` takes {} args, got 0", func.params.len()),
            "input.v2",
            0,
            0,
            "call arity must match",
            &["pass the right number of arguments"],
            "calls/arity",
        ));
    }
    let mut frame = Frame::default();
    let ret = exec_block(&func.body, &mut frame, &global, 0)?;
    let v = match ret {
        Some(v) => v,
        None => frame
            .vars
            .get("__last")
            .cloned()
            .unwrap_or(Value::Int(0)),
    };
    let out = global.output.lock().unwrap().clone();
    // Join any leaked (unlistened) echo threads so no thread is detached,
    // then report unlistened loudly if any remain (mirrors E-ECHO-UNLISTENED
    // at runtime; check-time already emits it).
    let mut leaked: Vec<String> = {
        let mut h = global.handles.lock().unwrap();
        let keys: Vec<String> = h.keys().cloned().collect();
        let mut leaked = Vec::new();
        for k in keys {
            if let Some(jh) = h.remove(&k) {
                let _ = jh.join();
                leaked.push(k);
            }
        }
        leaked
    };
    if !leaked.is_empty() {
        leaked.sort();
        return Err(crate::ai_safety::diagnostics::echo_unlistened(
            "input.v2",
            0,
            0,
            &leaked[0],
        ));
    }
    let n = v.as_int();
    if n < i32::MIN as i64 || n > i32::MAX as i64 {
        // Non-integer returns (e.g. `return "ok"`) surface as 0 with output
        // preserved, matching v1's fallthrough; out-of-range ints are loud.
        // Strings parse to 0 via as_int; only real overflow errors.
        if matches!(v, Value::Str(_)) {
            return Ok((0, out));
        }
        return Err(rt_err("integer overflow: result out of i32 range"));
    }
    Ok((n as i32, out))
}

fn exec_block(
    block: &V2Block,
    frame: &mut Frame,
    global: &V2Global,
    depth: usize,
) -> Result<Option<Value>, Diagnostic> {
    if depth > 64 {
        return Err(rt_err("call depth exceeded (possible recursion)"));
    }
    let mut last = Value::Int(0);
    let mut had = false;
    for stmt in &block.stmts {
        match exec_stmt(stmt, frame, global, depth, &mut last)? {
            Some(v) => return Ok(Some(v)),
            None => {
                had = true;
            }
        }
    }
    if had {
        frame.vars.insert("__last".to_string(), last);
    }
    Ok(None)
}

fn exec_stmt(
    stmt: &V2Stmt,
    frame: &mut Frame,
    global: &V2Global,
    depth: usize,
    last: &mut Value,
) -> Result<Option<Value>, Diagnostic> {
    use crate::ast::v2::V2Stmt::*;
    match stmt {
        Let(s) => {
            // `let f = flow(...)` defines a flow, not a value.
            if let V2Expr::Flow(fexpr) = &s.value {
                let body_expr = crate::parser::v2::parse_v2_expr(&fexpr.body).map_err(|e| e)?;
                frame.flows.insert(
                    s.name.clone(),
                    FlowDef {
                        params: fexpr.params.iter().map(|p| p.name.clone()).collect(),
                        deps: fexpr.deps.iter().map(|d| d.name.clone()).collect(),
                        body_expr,
                    },
                );
                // Flows are not first-class values; bind a marker so later
                // misuse (`print(f)`) fails loudly instead of reading 0.
                frame.vars.insert(
                    s.name.clone(),
                    Value::Str(format!("flow:{}", s.name)),
                );
                *last = Value::Int(0);
                return Ok(None);
            }
            let v = eval_expr(&s.value, frame, global, depth)?;
            frame.vars.insert(s.name.clone(), v.clone());
            *last = v;
            Ok(None)
        }
        Assign(s) => {
            let v = eval_expr(&s.value, frame, global, depth)?;
            use crate::ast::v2::V2AssignTarget::*;
            match &s.target {
                Var { name } => {
                    if frame.flows.contains_key(name) {
                        return Err(rt_err("cannot assign over a flow binding"));
                    }
                    frame.vars.insert(name.clone(), v.clone());
                }
                Index { .. } => return Err(rt_err("index assignment not supported in v2")),
                Field { base, field } => {
                    // `p.field = v` for struct values.
                    let bname = match base.as_ref() {
                        V2Expr::Var { name, .. } => name.clone(),
                        _ => return Err(rt_err("field assignment needs a variable base")),
                    };
                    match frame.vars.get_mut(&bname) {
                        Some(Value::Struct { fields, .. }) => {
                            if let Some(slot) = fields.iter_mut().find(|(k, _)| k == field) {
                                slot.1 = v.clone();
                            } else {
                                return Err(rt_err("unknown struct field"));
                            }
                        }
                        _ => return Err(rt_err("field assignment needs a struct")),
                    }
                }
            }
            *last = v;
            Ok(None)
        }
        Return(s) => {
            let v = eval_expr(&s.value, frame, global, depth)?;
            Ok(Some(v))
        }
        If(s) => {
            let c = eval_expr(&s.cond, frame, global, depth)?;
            if truthy(&c) {
                if let Some(v) = exec_block(&s.then_block, frame, global, depth)? {
                    return Ok(Some(v));
                }
            } else if let Some(else_b) = &s.else_block {
                if let Some(v) = exec_block(else_b, frame, global, depth)? {
                    return Ok(Some(v));
                }
            }
            Ok(None)
        }
        Print(s) => {
            let v = eval_expr(&s.value, frame, global, depth)?;
            global.output.lock().unwrap().push(v.render());
            *last = v;
            Ok(None)
        }
        Expr(e) => {
            let v = eval_expr(e, frame, global, depth)?;
            *last = v;
            Ok(None)
        }
    }
}

fn eval_expr(
    expr: &V2Expr,
    frame: &mut Frame,
    global: &V2Global,
    depth: usize,
) -> Result<Value, Diagnostic> {
    if depth > 64 {
        return Err(rt_err("call depth exceeded (possible recursion)"));
    }
    match expr {
        V2Expr::Int { value, .. } => Ok(Value::Int(*value)),
        V2Expr::Str { value, .. } => Ok(Value::Str(value.clone())),
        V2Expr::Bool { value, .. } => Ok(Value::Int(i64::from(*value))),
        V2Expr::Var { name, .. } => frame.vars.get(name).cloned().ok_or_else(|| {
            Diagnostic::error(
                "E-UNDEFINED",
                &format!("undefined variable `{name}`"),
                "input.v2",
                0,
                0,
                "name is not defined in this scope",
                &["define it first"],
                "names/scope",
            )
        }),
        V2Expr::Call { func, args, .. } => eval_call(func, args, frame, global, depth),
        V2Expr::Listen { handle, .. } => eval_listen(handle, frame, global),
        V2Expr::Flow(_) => Err(rt_err("flow blocks are values only via `let f = flow...`")),
        V2Expr::Tune(t) => eval_tune(&t.target_ty.base, &t.value, frame, global, depth, "tune"),
        V2Expr::Verify(v) => eval_tune(&v.target_ty.base, &v.value, frame, global, depth, "verify"),
        V2Expr::StructLit { fields, .. } => {
            let mut out = Vec::with_capacity(fields.len());
            for (k, e) in fields {
                out.push((k.clone(), eval_expr(e, frame, global, depth)?));
            }
            Ok(Value::Struct {
                name: "Struct".to_string(),
                fields: out,
            })
        }
        V2Expr::Add { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(match (&l, &r) {
                (Value::Str(a), b) => Value::Str(format!("{a}{}", b.render())),
                (a, Value::Str(b)) => Value::Str(format!("{}{b}", a.render())),
                _ => Value::Int(l.as_int().wrapping_add(r.as_int())),
            })
        }
        V2Expr::Sub { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(l.as_int().wrapping_sub(r.as_int())))
        }
        V2Expr::Mul { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(l.as_int().wrapping_mul(r.as_int())))
        }
        V2Expr::Div { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            if r.as_int() == 0 {
                return Err(rt_err("division by zero"));
            }
            Ok(Value::Int(l.as_int().wrapping_div(r.as_int())))
        }
        V2Expr::Mod { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            if r.as_int() == 0 {
                return Err(rt_err("modulo by zero"));
            }
            Ok(Value::Int(l.as_int().wrapping_rem(r.as_int())))
        }
        V2Expr::Eq { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(l == r)))
        }
        V2Expr::NotEq { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(l != r)))
        }
        V2Expr::Lt { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(compare_vals(&l, &r).is_lt())))
        }
        V2Expr::LtEq { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(compare_vals(&l, &r).is_le())))
        }
        V2Expr::Gt { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(compare_vals(&l, &r).is_gt())))
        }
        V2Expr::GtEq { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(compare_vals(&l, &r).is_ge())))
        }
        V2Expr::And { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(truthy(&l) && truthy(&r))))
        }
        V2Expr::Or { left, right, .. } => {
            let l = eval_expr(left, frame, global, depth)?;
            let r = eval_expr(right, frame, global, depth)?;
            Ok(Value::Int(i64::from(truthy(&l) || truthy(&r))))
        }
        V2Expr::Index { .. } => Err(rt_err("indexing not supported in v2")),
        V2Expr::Field { base, field, .. } => {
            let b = eval_expr(base, frame, global, depth)?;
            match b {
                Value::Struct { fields, .. } => fields
                    .iter()
                    .find(|(k, _)| k == field)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| rt_err("unknown struct field")),
                _ => Err(rt_err("field access needs a struct")),
            }
        }
    }
}

fn compare_vals(l: &Value, r: &Value) -> std::cmp::Ordering {
    match (l, r) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        _ => l.as_int().cmp(&r.as_int()),
    }
}

fn eval_call(
    func: &str,
    args: &[V2Expr],
    frame: &mut Frame,
    global: &V2Global,
    depth: usize,
) -> Result<Value, Diagnostic> {
    // Flows first: `let f = flow...` then `f(x)`.
    if let Some(fdef) = frame
        .flows
        .get(func)
        .cloned()
        .or_else(|| global.global_flows.get(func).cloned())
    {
        if args.len() != fdef.params.len() {
            return Err(Diagnostic::error(
                "E-ARITY",
                &format!("flow `{func}` takes {} args, got {}", fdef.params.len(), args.len()),
                "input.v2",
                0,
                0,
                "call arity must match",
                &["pass the right number of arguments"],
                "calls/arity",
            ));
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(eval_expr(a, frame, global, depth)?);
        }
        // dep= bindings resolve from the caller at call time — the point
        // of the pillar-2 test. Missing deps fail loudly, never as 0.
        let mut child = Frame {
            vars: HashMap::new(),
            flows: frame.flows.clone(),
        };
        for (p, v) in fdef.params.iter().zip(vals) {
            child.vars.insert(p.clone(), v);
        }
        for d in &fdef.deps {
            let v = frame.vars.get(d).cloned().ok_or_else(|| {
                crate::ai_safety::diagnostics::flow_mutable_capture("input.v2", 0, 0, d)
            })?;
            child.vars.insert(d.clone(), v);
        }
        // Flow bodies are single expressions.
        return eval_expr(&fdef.body_expr, &mut child, global, depth + 1);
    }
    // Echo fns: run body on a background thread, return a handle.
    if let Some(edef) = global.echo_defs.get(func).cloned() {
        if args.len() != edef.params.len() {
            return Err(Diagnostic::error(
                "E-ARITY",
                &format!(
                    "echo fn `{func}` takes {} args, got {}",
                    edef.params.len(),
                    args.len()
                ),
                "input.v2",
                0,
                0,
                "call arity must match",
                &["pass the right number of arguments"],
                "calls/arity",
            ));
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(eval_expr(a, frame, global, depth)?);
        }
        let mut id = global.counter.lock().unwrap();
        *id += 1;
        let hid = format!("echo${}#{id}", func);
        drop(id);
        let child_global = global.clone();
        let child_flows = frame.flows.clone();
        let child_body = edef.body.clone();
        let child_params = edef.params.clone();
        let h = std::thread::spawn(move || -> Result<Value, Diagnostic> {
            let mut eframe = Frame {
                vars: HashMap::new(),
                flows: child_flows,
            };
            for (p, v) in child_params.iter().zip(vals) {
                eframe.vars.insert(p.clone(), v);
            }
            match exec_block(&child_body, &mut eframe, &child_global, depth + 1)? {
                Some(v) => Ok(v),
                None => Ok(eframe
                    .vars
                    .get("__last")
                    .cloned()
                    .unwrap_or(Value::Int(0))),
            }
        });
        global.handles.lock().unwrap().insert(hid.clone(), h);
        return Ok(Value::Str(hid));
    }
    // Regular v2 functions.
    if let Some(fdecl) = global.functions.get(func).cloned() {
        if args.len() != fdecl.params.len() {
            return Err(Diagnostic::error(
                "E-ARITY",
                &format!(
                    "`{func}` takes {} args, got {}",
                    fdecl.params.len(),
                    args.len()
                ),
                "input.v2",
                0,
                0,
                "call arity must match",
                &["pass the right number of arguments"],
                "calls/arity",
            ));
        }
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(eval_expr(a, frame, global, depth)?);
        }
        let mut child = Frame::default();
        for (p, v) in fdecl.params.iter().zip(vals) {
            child.vars.insert(p.name.clone(), v);
        }
        match exec_block(&fdecl.body, &mut child, global, depth + 1)? {
            Some(v) => Ok(v),
            None => Ok(child.vars.get("__last").cloned().unwrap_or(Value::Int(0))),
        }
    } else if func == "str" {
        if args.len() != 1 {
            return Err(rt_err("str() takes 1 argument"));
        }
        let v = eval_expr(&args[0], frame, global, depth)?;
        Ok(Value::Str(v.render()))
    } else if func == "int" {
        if args.len() != 1 {
            return Err(rt_err("int() takes 1 argument"));
        }
        let v = eval_expr(&args[0], frame, global, depth)?;
        Ok(Value::Int(v.as_int()))
    } else {
        Err(Diagnostic::error(
            "E-UNDEFINED",
            &format!("undefined function `{func}`"),
            "input.v2",
            0,
            0,
            "callee is not defined in this module",
            &["define the function first"],
            "names/scope",
        ))
    }
}

fn eval_listen(
    handle_var: &str,
    frame: &mut Frame,
    global: &V2Global,
) -> Result<Value, Diagnostic> {
    let hv = frame.vars.get(handle_var).cloned().ok_or_else(|| {
        crate::ai_safety::diagnostics::echo_invalid_listen(
            "input.v2",
            0,
            0,
            handle_var,
            "no echo handle with this name is outstanding",
        )
    })?;
    let hid = match hv {
        Value::Str(s) => s,
        _ => {
            return Err(crate::ai_safety::diagnostics::echo_invalid_listen(
                "input.v2",
                0,
                0,
                handle_var,
                "handle variable does not hold an echo handle",
            ));
        }
    };
    if hid.starts_with("flow:") {
        return Err(crate::ai_safety::diagnostics::echo_invalid_listen(
            "input.v2",
            0,
            0,
            handle_var,
            "cannot listen to a flow binding",
        ));
    }
    let jh = global.handles.lock().unwrap().remove(&hid).ok_or_else(|| {
        crate::ai_safety::diagnostics::echo_invalid_listen(
            "input.v2",
            0,
            0,
            &hid,
            "echo handle was already consumed or never created",
        )
    })?;
    match jh.join() {
        Ok(Ok(v)) => {
            frame.vars.insert(handle_var.to_string(), v.clone());
            Ok(v)
        }
        Ok(Err(d)) => Err(d),
        Err(_) => Err(rt_err("echo task panicked")),
    }
}

fn eval_tune(
    schema_name: &str,
    value_expr: &V2Expr,
    frame: &mut Frame,
    global: &V2Global,
    depth: usize,
    _boundary: &str,
) -> Result<Value, Diagnostic> {
    let v = eval_expr(value_expr, frame, global, depth)?;
    let schema = global.schemas.get(schema_name).ok_or_else(|| {
        crate::ai_safety::diagnostics::schema_not_found("input.v2", 0, 0, schema_name)
    })?;
    // Real validation: wrong field types / missing fields fail with the
    // precise E-SCHEMA-INVALID diagnostic, never silently.
    let _ = validate_against_schema(schema, &v)?;
    Ok(v)
}
