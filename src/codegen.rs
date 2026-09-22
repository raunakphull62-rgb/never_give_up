//! Stage 7 (part 2) + pipeline tail: machine code generation (foundation).
//!
//! Real backend is Cranelift (pure Rust). This foundation emits a stable
//! pseudo-assembly listing from MIR and interprets it via `runtime.rs`, so
//! the full pipeline is testable before wiring the real backend.

use crate::mir::{MirModule, MirOp};

/// Emit a stable text listing (one line per instruction).
pub fn emit_listing(module: &MirModule) -> String {
    fn esc(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out
    }
    let mut out = String::new();
    for f in &module.functions {
        out.push_str(&format!(
            "func {}({}) @{}:\n",
            f.name,
            f.params.join(", "),
            f.origin
        ));
        for (i, ins) in f.instrs.iter().enumerate() {
            let line = match &ins.op {
                MirOp::EnterGroup { group } => format!("  enter_group {group}"),
                MirOp::LeaveGroup { group } => format!("  leave_group {group}"),
                MirOp::Spawn { handle, func, args } => {
                    format!("  spawn {handle} = {func}({})", args.join(", "))
                }
                MirOp::Await { handle } => format!("  await {handle}"),
                MirOp::Call { into, func, args } => {
                    format!("  {into} = call {func}({})", args.join(", "))
                }
                MirOp::Const { into, value } => format!("  {into} = const {value}"),
                MirOp::ConstFloat { into, bits } => {
                    format!("  {into} = const_float {}", f64::from_bits(*bits))
                }
                MirOp::ConstStr { into, value } => format!("  {into} = const_str \"{}\"", esc(value)),
                MirOp::Copy { into, from } => format!("  {into} = copy {from}"),
                MirOp::Add { into, left, right } => format!("  {into} = add {left} {right}"),
                MirOp::Sub { into, left, right } => format!("  {into} = sub {left} {right}"),
                MirOp::Mul { into, left, right } => format!("  {into} = mul {left} {right}"),
                MirOp::Neg { into, inner } => format!("  {into} = neg {inner}"),
                MirOp::Not { into, inner } => format!("  {into} = not {inner}"),
                MirOp::Div { into, left, right } => format!("  {into} = div {left} {right}"),
                MirOp::Mod { into, left, right } => format!("  {into} = mod {left} {right}"),
                MirOp::Eq { into, left, right } => format!("  {into} = eq {left} {right}"),
                MirOp::NotEq { into, left, right } => format!("  {into} = neq {left} {right}"),
                MirOp::Lt { into, left, right } => format!("  {into} = lt {left} {right}"),
                MirOp::LtEq { into, left, right } => format!("  {into} = lte {left} {right}"),
                MirOp::Gt { into, left, right } => format!("  {into} = gt {left} {right}"),
                MirOp::GtEq { into, left, right } => format!("  {into} = gte {left} {right}"),
                MirOp::And { into, left, right } => format!("  {into} = and {left} {right}"),
                MirOp::Or { into, left, right } => format!("  {into} = or {left} {right}"),
                MirOp::ArrayNew { into, elems } => {
                    format!("  {into} = array [{}]", elems.join(", "))
                }
                MirOp::MapNew { into, entries } => {
                    let fs: Vec<String> =
                        entries.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                    format!("  {into} = map {{ {} }}", fs.join(", "))
                }
                MirOp::MethodCall {
                    into,
                    base,
                    method,
                    args,
                } => {
                    format!("  {into} = method {base}.{method}({})", args.join(", "))
                }
                MirOp::Index { into, base, index } => format!("  {into} = index {base}[{index}]"),
                MirOp::StoreIndex { base, index, value } => {
                    format!("  {base}[{index}] = {value}")
                }
                MirOp::FieldGet { into, base, field } => format!("  {into} = field {base}.{field}"),
                MirOp::FieldSet { base, field, value } => {
                    format!("  {base}.{field} = {value}")
                }
                MirOp::StructNew { into, name, fields } => {
                    let fs: Vec<String> = fields.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                    format!("  {into} = struct {name} {{ {} }}", fs.join(", "))
                }
                MirOp::Len { into, of } => format!("  {into} = len {of}"),
                MirOp::Print { value } => format!("  print {value}"),
                MirOp::JumpIfFalse { cond, target } => {
                    format!("  jump_if_false {cond} -> {target}")
                }
                MirOp::Jump { target } => format!("  jump -> {target}"),
                MirOp::Return { value } => format!("  return {value}"),
            };
            out.push_str(&format!("  [{i}] ; origin {}\n{line}\n", ins.origin));
        }
    }
    out
}
