//! Klang v2 MIR lowering (Phase 10+).
//!
//! Lowers v2 AST to MIR with v2-specific operations for schemas, echoes, flows,
//! and resonance types.

use crate::ast::v2::{
    SchemaDecl, V2AssignStmt, V2AssignTarget, V2Block, V2Expr, V2FunctionDecl,
    V2IfStmt, V2LetStmt, V2PrintStmt, V2ReturnStmt, V2Stmt, V2TuneExpr,
    V2VerifyExpr,
};
use crate::ast::{echo::EchoDecl, flow::FlowExpr, NodeId};
use crate::mir::{MirFunction, MirInstr, MirModule, MirOp};
use crate::mir::echo_lowering::{lower_success, EchoMirNode, EchoMirOp, EchoState};
use crate::mir::flow_lowering::{lower_flow, FlowStep, LoweredFlow};
use std::collections::HashMap;

/// Lower a v2 program to MIR.
pub fn lower_v2_program(
    schemas: &[SchemaDecl],
    echo_fns: &[EchoDecl],
    echo_bodies: &std::collections::HashMap<String, crate::ast::v2::V2Block>,
    flows: &[crate::ast::flow::FlowDecl],
    functions: &[V2FunctionDecl],
) -> MirModule {
    let mut module = MirModule::default();
    
    // Create a schema registry for validation
    let schema_registry = build_schema_registry(schemas);
    
    // Lower echo fns to MIR functions with echo state machine
    for echo_fn in echo_fns {
        let mir_fn = lower_echo_fn(echo_fn);
        module.functions.push(mir_fn);
    }
    
    // Lower flows to MIR functions
    for flow_decl in flows {
        let mir_fn = lower_flow_decl(flow_decl);
        module.functions.push(mir_fn);
    }
    
    // Lower regular functions
    for func in functions {
        let mut ctx = LowerContext::new(&schema_registry);
        let mir_fn = ctx.lower_function(func);
        module.functions.push(mir_fn);
    }
    
    module
}

/// Build a schema registry from schema declarations.
fn build_schema_registry(schemas: &[SchemaDecl]) -> HashMap<String, SchemaDecl> {
    let mut registry = HashMap::new();
    for schema in schemas {
        registry.insert(schema.name.clone(), schema.clone());
    }
    registry
}

/// Context for lowering v2 functions to MIR.
struct LowerContext {
    schema_registry: HashMap<String, SchemaDecl>,
    tmp_counter: u32,
    flow_counter: u32,
    echo_counter: u32,
}

impl LowerContext {
    fn new(schema_registry: &HashMap<String, SchemaDecl>) -> Self {
        Self {
            schema_registry: schema_registry.clone(),
            tmp_counter: 0,
            flow_counter: 0,
            echo_counter: 0,
        }
    }
    
    fn next_tmp(&mut self) -> String {
        let name = format!("t{}", self.tmp_counter);
        self.tmp_counter += 1;
        name
    }
    
    fn next_flow_name(&mut self) -> String {
        let name = format!("flow{}", self.flow_counter);
        self.flow_counter += 1;
        name
    }
    
    fn next_echo_name(&mut self) -> String {
        let name = format!("echo{}", self.echo_counter);
        self.echo_counter += 1;
        name
    }
    
    fn lower_function(&mut self, func: &V2FunctionDecl) -> MirFunction {
        let mut instrs = Vec::new();
        for stmt in &func.body.stmts {
            self.lower_stmt(stmt, &mut instrs);
        }
        MirFunction {
            name: func.name.clone(),
            origin: func.id.clone(),
            params: func.params.iter().map(|p| p.name.clone()).collect(),
            instrs,
        }
    }
    
    fn lower_stmt(&mut self, stmt: &V2Stmt, instrs: &mut Vec<MirInstr>) {
        match stmt {
            V2Stmt::Let(s) => self.lower_let(s, instrs),
            V2Stmt::Assign(s) => self.lower_assign(s, instrs),
            V2Stmt::Return(s) => self.lower_return(s, instrs),
            V2Stmt::If(s) => self.lower_if(s, instrs),
            V2Stmt::Print(s) => self.lower_print(s, instrs),
            V2Stmt::Expr(e) => {
                self.lower_expr(e, instrs);
            }
        }
    }
    
    fn lower_let(&mut self, stmt: &V2LetStmt, instrs: &mut Vec<MirInstr>) {
        let value = self.lower_expr(&stmt.value, instrs);
        if value != stmt.name {
            instrs.push(MirInstr {
                origin: stmt.id.clone(),
                op: MirOp::Copy {
                    into: stmt.name.clone(),
                    from: value,
                },
            });
        }
    }
    
    fn lower_assign(&mut self, stmt: &V2AssignStmt, instrs: &mut Vec<MirInstr>) {
        let value = self.lower_expr(&stmt.value, instrs);
        match &stmt.target {
            V2AssignTarget::Var { name } => {
                instrs.push(MirInstr {
                    origin: stmt.id.clone(),
                    op: MirOp::Copy {
                        into: name.clone(),
                        from: value,
                    },
                });
            }
            V2AssignTarget::Index { base, index } => {
                let b = self.lower_expr(base, instrs);
                let i = self.lower_expr(index, instrs);
                instrs.push(MirInstr {
                    origin: stmt.id.clone(),
                    op: MirOp::StoreIndex {
                        base: b,
                        index: i,
                        value: value,
                    },
                });
            }
            V2AssignTarget::Field { base, field } => {
                let b = self.lower_expr(base, instrs);
                instrs.push(MirInstr {
                    origin: stmt.id.clone(),
                    op: MirOp::FieldSet {
                        base: b,
                        field: field.clone(),
                        value: value,
                    },
                });
            }
        }
    }
    
    fn lower_return(&mut self, stmt: &V2ReturnStmt, instrs: &mut Vec<MirInstr>) {
        let value = self.lower_expr(&stmt.value, instrs);
        instrs.push(MirInstr {
            origin: stmt.id.clone(),
            op: MirOp::Return { value },
        });
    }
    
    fn lower_if(&mut self, stmt: &V2IfStmt, instrs: &mut Vec<MirInstr>) {
        let cond = self.lower_expr(&stmt.cond, instrs);
        let jfalse = instrs.len();
        instrs.push(MirInstr {
            origin: stmt.id.clone(),
            op: MirOp::JumpIfFalse {
                cond,
                target: usize::MAX,
            },
        });
        for s in &stmt.then_block.stmts {
            self.lower_stmt(s, instrs);
        }
        if let Some(else_block) = &stmt.else_block {
            let jend = instrs.len();
            instrs.push(MirInstr {
                origin: stmt.id.clone(),
                op: MirOp::Jump { target: usize::MAX },
            });
            let else_at = instrs.len();
            if let Some(instr) = instrs.get_mut(jfalse) {
                if let MirOp::JumpIfFalse { target, .. } = &mut instr.op {
                    *target = else_at;
                }
            }
            for s in &else_block.stmts {
                self.lower_stmt(s, instrs);
            }
            let end_at = instrs.len();
            if let Some(instr) = instrs.get_mut(jend) {
                if let MirOp::Jump { target } = &mut instr.op {
                    *target = end_at;
                }
            }
        } else {
            let end_at = instrs.len();
            if let Some(instr) = instrs.get_mut(jfalse) {
                if let MirOp::JumpIfFalse { target, .. } = &mut instr.op {
                    *target = end_at;
                }
            }
        }
    }
    
    fn lower_print(&mut self, stmt: &V2PrintStmt, instrs: &mut Vec<MirInstr>) {
        let value = self.lower_expr(&stmt.value, instrs);
        instrs.push(MirInstr {
            origin: stmt.id.clone(),
            op: MirOp::Print { value },
        });
    }
    
    fn lower_expr(&mut self, expr: &V2Expr, instrs: &mut Vec<MirInstr>) -> String {
        match expr {
            V2Expr::Int { id: _, value } => {
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::Const {
                        into: dst.clone(),
                        value: *value,
                    },
                });
                dst
            }
            V2Expr::Str { id: _, value } => {
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::ConstStr {
                        into: dst.clone(),
                        value: value.clone(),
                    },
                });
                dst
            }
            V2Expr::Bool { id: _, value } => {
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::Const {
                        into: dst.clone(),
                        value: if *value { 1 } else { 0 },
                    },
                });
                dst
            }
            V2Expr::Var { id: _, name } => name.clone(),
            V2Expr::Call { id: _, func, args } => {
                let mut lowered_args = Vec::new();
                for arg in args {
                    lowered_args.push(self.lower_expr(arg, instrs));
                }
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::Call {
                        into: dst.clone(),
                        func: func.clone(),
                        args: lowered_args,
                    },
                });
                dst
            }
            V2Expr::Listen { id: _, handle } => {
                // Lower listen to a call to the echo runtime
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::Call {
                        into: dst.clone(),
                        func: "__echo_listen".to_string(),
                        args: vec![handle.clone()],
                    },
                });
                dst
            }
            V2Expr::Flow(flow) => {
                // Lower flow to a function call
                let flow_name = self.next_flow_name();
                let lowered_flow = lower_flow(flow, Some(&flow_name));
                // For now, just call the flow as a function
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: flow.id.clone(),
                    op: MirOp::Call {
                        into: dst.clone(),
                        func: flow_name,
                        args: vec![], // Flow args would need special handling
                    },
                });
                dst
            }
            V2Expr::Tune(tune) => self.lower_tune(tune, instrs),
            V2Expr::Verify(verify) => self.lower_verify(verify, instrs),
            V2Expr::StructLit { id: _, name: _, fields } => {
                let mut field_vals = Vec::new();
                for (fname, fexpr) in fields {
                    let val = self.lower_expr(fexpr, instrs);
                    field_vals.push((fname.clone(), val));
                }
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::StructNew {
                        into: dst.clone(),
                        name: "Struct".to_string(),
                        fields: field_vals,
                    },
                });
                dst
            }
            V2Expr::Add { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Add { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Add { into, left: l, right: r }),
            V2Expr::Sub { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Sub { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Sub { into, left: l, right: r }),
            V2Expr::Mul { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Mul { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Mul { into, left: l, right: r }),
            V2Expr::Div { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Div { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Div { into, left: l, right: r }),
            V2Expr::Mod { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Mod { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Mod { into, left: l, right: r }),
            V2Expr::Eq { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Eq { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Eq { into, left: l, right: r }),
            V2Expr::NotEq { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::NotEq { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::NotEq { into, left: l, right: r }),
            V2Expr::Lt { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Lt { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Lt { into, left: l, right: r }),
            V2Expr::LtEq { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::LtEq { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::LtEq { into, left: l, right: r }),
            V2Expr::Gt { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Gt { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Gt { into, left: l, right: r }),
            V2Expr::GtEq { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::GtEq { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::GtEq { into, left: l, right: r }),
            V2Expr::And { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::And { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::And { into, left: l, right: r }),
            V2Expr::Or { id: _, left, right } => self.bin_op(left, right, instrs, MirOp::Or { into: String::new(), left: String::new(), right: String::new() }, |into, l, r| MirOp::Or { into, left: l, right: r }),
            V2Expr::Index { id: _, base, index } => {
                let b = self.lower_expr(base, instrs);
                let i = self.lower_expr(index, instrs);
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::Index { into: dst.clone(), base: b, index: i },
                });
                dst
            }
            V2Expr::Field { id: _, base, field } => {
                let b = self.lower_expr(base, instrs);
                let dst = self.next_tmp();
                instrs.push(MirInstr {
                    origin: NodeId::new(vec![]),
                    op: MirOp::FieldGet { into: dst.clone(), base: b, field: field.clone() },
                });
                dst
            }
        }
    }
    
    fn bin_op<F>(&mut self, left: &V2Expr, right: &V2Expr, instrs: &mut Vec<MirInstr>, _template: MirOp, make_op: F) -> String
    where
        F: FnOnce(String, String, String) -> MirOp,
    {
        let l = self.lower_expr(left, instrs);
        let r = self.lower_expr(right, instrs);
        let dst = self.next_tmp();
        instrs.push(MirInstr {
            origin: NodeId::new(vec![]),
            op: make_op(dst.clone(), l, r),
        });
        dst
    }
    
    fn lower_tune(&mut self, tune: &V2TuneExpr, instrs: &mut Vec<MirInstr>) -> String {
        let value = self.lower_expr(&tune.value, instrs);
        let dst = self.next_tmp();
        // Lower tune to a call to the schema validation runtime
        instrs.push(MirInstr {
            origin: tune.id.clone(),
            op: MirOp::Call {
                into: dst.clone(),
                func: "__tune_validate".to_string(),
                args: vec![value, tune.target_ty.display()],
            },
        });
        dst
    }
    
    fn lower_verify(&mut self, verify: &V2VerifyExpr, instrs: &mut Vec<MirInstr>) -> String {
        let value = self.lower_expr(&verify.value, instrs);
        let dst = self.next_tmp();
        instrs.push(MirInstr {
            origin: verify.id.clone(),
            op: MirOp::Call {
                into: dst.clone(),
                func: "__verify_validate".to_string(),
                args: vec![value, verify.target_ty.display()],
            },
        });
        dst
    }
}

/// Lower an echo fn to MIR with echo state machine.
fn lower_echo_fn(echo_fn: &EchoDecl) -> MirFunction {
    let mut instrs = Vec::new();
    let handle = echo_fn.name.clone();
    let inner_ty = echo_fn.return_ty.trim_start_matches('!').trim_start_matches('?');
    
    // Generate echo state machine nodes
    let echo_nodes = lower_success(&handle, inner_ty);
    
    for node in echo_nodes {
        match node.op {
            EchoMirOp::Create { handle, inner } => {
                instrs.push(MirInstr {
                    origin: echo_fn.id.clone(),
                    op: MirOp::Call {
                        into: handle.clone(),
                        func: "__echo_create".to_string(),
                        args: vec![inner],
                    },
                });
            }
            EchoMirOp::Start { handle } => {
                instrs.push(MirInstr {
                    origin: echo_fn.id.clone(),
                    op: MirOp::Call {
                        into: format!("{}_started", handle),
                        func: "__echo_start".to_string(),
                        args: vec![handle],
                    },
                });
            }
            EchoMirOp::Suspend { handle } => {
                instrs.push(MirInstr {
                    origin: echo_fn.id.clone(),
                    op: MirOp::Call {
                        into: format!("{}_suspended", handle),
                        func: "__echo_suspend".to_string(),
                        args: vec![handle],
                    },
                });
            }
            EchoMirOp::Resume { handle } => {
                instrs.push(MirInstr {
                    origin: echo_fn.id.clone(),
                    op: MirOp::Call {
                        into: format!("{}_resumed", handle),
                        func: "__echo_resume".to_string(),
                        args: vec![handle],
                    },
                });
            }
            EchoMirOp::Complete { handle } => {
                instrs.push(MirInstr {
                    origin: echo_fn.id.clone(),
                    op: MirOp::Call {
                        into: format!("{}_completed", handle),
                        func: "__echo_complete".to_string(),
                        args: vec![handle],
                    },
                });
            }
            EchoMirOp::Listen { handle } => {
                instrs.push(MirInstr {
                    origin: echo_fn.id.clone(),
                    op: MirOp::Call {
                        into: handle.clone(),
                        func: "__echo_listen".to_string(),
                        args: vec![handle],
                    },
                });
            }
            EchoMirOp::Cleanup { handle } => {
                instrs.push(MirInstr {
                    origin: echo_fn.id.clone(),
                    op: MirOp::Call {
                        into: format!("{}_cleaned", handle),
                        func: "__echo_cleanup".to_string(),
                        args: vec![handle],
                    },
                });
            }
            _ => {}
        }
    }
    
    // Return the echo handle
    instrs.push(MirInstr {
        origin: echo_fn.id.clone(),
        op: MirOp::Return { value: echo_fn.name.clone() },
    });
    
    MirFunction {
        name: echo_fn.name.clone(),
        origin: echo_fn.id.clone(),
        params: echo_fn.params.iter().map(|(n, _)| n.clone()).collect(),
        instrs,
    }
}

/// Lower a flow declaration to MIR.
fn lower_flow_decl(flow_decl: &crate::ast::flow::FlowDecl) -> MirFunction {
    let lowered = lower_flow(&flow_decl.expr, flow_decl.name.as_deref());
    let mut instrs = Vec::new();
    
    // For now, just create a function that represents the flow
    // The actual flow execution would need a more sophisticated lowering
    instrs.push(MirInstr {
        origin: flow_decl.id.clone(),
        op: MirOp::Return { value: "flow_result".to_string() },
    });
    
    MirFunction {
        name: flow_decl.name.clone().unwrap_or_else(|| "flow_anon".to_string()),
        origin: flow_decl.id.clone(),
        params: lowered.env,
        instrs,
    }
}