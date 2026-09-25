//! Klang v2 full program AST (Phase 10+).
//!
//! AST nodes for complete v2 programs with schemas, echo fns, and functions
//! using resonance types.

use crate::ast::echo::EchoDecl;
use crate::ast::flow::{FlowDecl, FlowExpr};
use crate::ast::resonance::QualifiedType;
use crate::ast::NodeId;
use std::collections::HashMap;

/// A v2 schema declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDecl {
    pub id: NodeId,
    pub name: String,
    pub name_span: (usize, usize),
    pub version: String,
    pub fields: Vec<SchemaField>,
}

/// A schema field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaField {
    pub name: String,
    pub ty: String,
}

/// A v2 function parameter with qualified type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Param {
    pub name: String,
    pub ty: QualifiedType,
}

/// A v2 function declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2FunctionDecl {
    pub id: NodeId,
    pub name: String,
    pub name_span: (usize, usize),
    pub is_pub: bool,
    pub params: Vec<V2Param>,
    pub return_ty: QualifiedType,
    pub body: V2Block,
}

/// A v2 block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Block {
    pub id: NodeId,
    pub stmts: Vec<V2Stmt>,
}

/// A v2 statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V2Stmt {
    Let(V2LetStmt),
    Assign(V2AssignStmt),
    Return(V2ReturnStmt),
    If(V2IfStmt),
    Print(V2PrintStmt),
    Expr(V2Expr),
}

/// A v2 let statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2LetStmt {
    pub id: NodeId,
    pub name: String,
    pub name_span: (usize, usize),
    pub value: V2Expr,
    pub span: (usize, usize),
}

/// A v2 assignment statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2AssignStmt {
    pub id: NodeId,
    pub target: V2AssignTarget,
    pub value: V2Expr,
}

/// A v2 assignment target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V2AssignTarget {
    Var { name: String },
    Index { base: Box<V2Expr>, index: Box<V2Expr> },
    Field { base: Box<V2Expr>, field: String },
}

/// A v2 return statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2ReturnStmt {
    pub id: NodeId,
    pub value: V2Expr,
    pub span: (usize, usize),
}

/// A v2 if statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2IfStmt {
    pub id: NodeId,
    pub cond: V2Expr,
    pub then_block: V2Block,
    pub else_block: Option<V2Block>,
}

/// A v2 print statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2PrintStmt {
    pub id: NodeId,
    pub value: V2Expr,
}

/// A v2 expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V2Expr {
    Int { id: NodeId, value: i64 },
    Str { id: NodeId, value: String },
    Bool { id: NodeId, value: bool },
    Var { id: NodeId, name: String },
    Call { id: NodeId, func: String, args: Vec<V2Expr> },
    Listen { id: NodeId, handle: String },
    Flow(FlowExpr),
    Tune(V2TuneExpr),
    Verify(V2VerifyExpr),
    StructLit { id: NodeId, name: String, fields: Vec<(String, V2Expr)> },
    Add { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Sub { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Mul { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Div { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Mod { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Eq { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    NotEq { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Lt { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    LtEq { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Gt { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    GtEq { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    And { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Or { id: NodeId, left: Box<V2Expr>, right: Box<V2Expr> },
    Index { id: NodeId, base: Box<V2Expr>, index: Box<V2Expr> },
    Field { id: NodeId, base: Box<V2Expr>, field: String },
}

/// A v2 tune expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2TuneExpr {
    pub id: NodeId,
    pub target_ty: QualifiedType,
    pub value: Box<V2Expr>,
}

/// A v2 verify expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2VerifyExpr {
    pub id: NodeId,
    pub target_ty: QualifiedType,
    pub value: Box<V2Expr>,
}

/// A parsed v2 program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Program {
    pub schemas: Vec<SchemaDecl>,
    pub echo_fns: Vec<EchoDecl>,
    /// Executable bodies of `echo fn`s, keyed by echo name.
    /// (EchoDecl itself stays body-free for the check path.)
    pub echo_bodies: HashMap<String, V2Block>,
    pub flows: Vec<FlowDecl>,
    pub functions: Vec<V2FunctionDecl>,
}