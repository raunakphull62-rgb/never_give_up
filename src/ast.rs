//! Klang AST with structural node identity.
//!
//! Identity rule: a child path is generated while the parent scope is still
//! entered, so every child path extends its parent path. No global counter is
//! used alone as identity; identity is the full path from the root.

use std::fmt;

/// Stable structural identity for every syntax node.
///
/// The path is a sequence of child indices from the program root.
/// Siblings differ in their final segment; a child always extends its parent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub path: Vec<u32>,
}

impl NodeId {
    pub fn new(path: Vec<u32>) -> Self {
        Self { path }
    }

    pub fn root_child(index: u32) -> Self {
        Self { path: vec![index] }
    }

    /// True when `self` is `parent` or nested inside it.
    pub fn starts_with(&self, parent: &NodeId) -> bool {
        self.path.len() >= parent.path.len() && self.path[..parent.path.len()] == parent.path[..]
    }

    pub fn display(&self) -> String {
        let mut out = String::from("[");
        for (i, seg) in self.path.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&seg.to_string());
        }
        out.push(']');
        out
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// Visible effects in v0.1. Only these three exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    Throws,
    Async,
    Cancel,
}

impl Effect {
    pub fn name(&self) -> &'static str {
        match self {
            Effect::Throws => "throws",
            Effect::Async => "async",
            Effect::Cancel => "cancel",
        }
    }
}

/// Whole parsed program: modules, enums, structs, imports, then functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub mods: Vec<ModDecl>,
    pub enums: Vec<EnumDecl>,
    pub structs: Vec<StructDecl>,
    pub imports: Vec<String>,
    pub functions: Vec<FunctionDecl>,
}

/// A named `mod name { ... }` module: its own namespace of items.
/// Members are visible outside only when `pub` and only via a qualified
/// path (`name::item`); identical private names in different modules never
/// collide because resolution qualifies them (`a::h` vs `b::h`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModDecl {
    pub id: NodeId,
    pub name: String,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub functions: Vec<FunctionDecl>,
}

/// A top-level `enum Name<T> { Variant, Other(field: Type, ...), ... }` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDecl {
    pub id: NodeId,
    pub name: String,
    pub is_pub: bool,
    pub type_params: Vec<String>,
    pub variants: Vec<EnumVariant>,
}

/// One enum variant with optional positional payload fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Param>,
}

/// A top-level `struct Name<T> { field: Type, ... }` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDecl {
    pub id: NodeId,
    pub name: String,
    pub is_pub: bool,
    pub type_params: Vec<String>,
    pub fields: Vec<Param>,
}

impl Program {
    /// Prefix every path with `file_idx` so merged multi-file programs keep
    /// globally unique, parent-prefixed identity.
    pub fn with_file_prefix(&self, file_idx: u32) -> Program {
        let mut prog = self.clone();
        for m in &mut prog.mods {
            prefix_id(&mut m.id, file_idx);
            for e in &mut m.enums {
                prefix_id(&mut e.id, file_idx);
            }
            for s in &mut m.structs {
                prefix_id(&mut s.id, file_idx);
            }
            for f in &mut m.functions {
                prefix_id(&mut f.id, file_idx);
                prefix_block(&mut f.body, file_idx);
            }
        }
        for e in &mut prog.enums {
            prefix_id(&mut e.id, file_idx);
        }
        for s in &mut prog.structs {
            prefix_id(&mut s.id, file_idx);
        }
        for f in &mut prog.functions {
            prefix_id(&mut f.id, file_idx);
            prefix_block(&mut f.body, file_idx);
        }
        prog
    }
}

fn prefix_id(id: &mut NodeId, file_idx: u32) {
    let mut path = Vec::with_capacity(id.path.len() + 1);
    path.push(file_idx);
    path.extend(id.path.iter().copied());
    id.path = path;
}

fn prefix_block(b: &mut Block, file_idx: u32) {
    prefix_id(&mut b.id, file_idx);
    for s in &mut b.stmts {
        prefix_stmt(s, file_idx);
    }
}

fn prefix_stmt(s: &mut Stmt, file_idx: u32) {
    match s {
        Stmt::Let(l) => {
            prefix_id(&mut l.id, file_idx);
            prefix_expr(&mut l.value, file_idx);
        }
        Stmt::Assign(a) => {
            prefix_id(&mut a.id, file_idx);
            match &mut a.target {
                AssignTarget::Var { .. } => {}
                AssignTarget::Index { base, index } => {
                    prefix_expr(base, file_idx);
                    prefix_expr(index, file_idx);
                }
                AssignTarget::Field { base, .. } => prefix_expr(base, file_idx),
            }
            prefix_expr(&mut a.value, file_idx);
        }
        Stmt::Return(r) => {
            prefix_id(&mut r.id, file_idx);
            prefix_expr(&mut r.value, file_idx);
        }
        Stmt::TaskGroup(g) => {
            prefix_id(&mut g.id, file_idx);
            prefix_block(&mut g.body, file_idx);
        }
        Stmt::If(i) => {
            prefix_id(&mut i.id, file_idx);
            prefix_expr(&mut i.cond, file_idx);
            prefix_block(&mut i.then_block, file_idx);
            if let Some(e) = &mut i.else_block {
                prefix_block(e, file_idx);
            }
        }
        Stmt::Print(p) => {
            prefix_id(&mut p.id, file_idx);
            prefix_expr(&mut p.value, file_idx);
        }
        Stmt::While(w) => {
            prefix_id(&mut w.id, file_idx);
            prefix_expr(&mut w.cond, file_idx);
            prefix_block(&mut w.body, file_idx);
        }
        Stmt::ForRange(fr) => {
            prefix_id(&mut fr.id, file_idx);
            prefix_expr(&mut fr.start, file_idx);
            prefix_expr(&mut fr.end, file_idx);
            prefix_block(&mut fr.body, file_idx);
        }
        Stmt::ForIn(fi) => {
            prefix_id(&mut fi.id, file_idx);
            prefix_expr(&mut fi.iter, file_idx);
            prefix_block(&mut fi.body, file_idx);
        }
        Stmt::Break(b) => prefix_id(&mut b.id, file_idx),
        Stmt::Continue(c) => prefix_id(&mut c.id, file_idx),
        Stmt::Expr(e) => prefix_expr(e, file_idx),
    }
}

fn prefix_expr(e: &mut Expr, file_idx: u32) {
    prefix_id(e.id_mut(), file_idx);
    match e {
        Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Str { .. }
        | Expr::Bool { .. }
        | Expr::Var { .. }
        | Expr::Await { .. } => {}
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                prefix_expr(el, file_idx);
            }
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                prefix_expr(v, file_idx);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields {
                prefix_expr(v, file_idx);
            }
        }
        Expr::EnumCtor { args, .. } => {
            for a in args {
                prefix_expr(a, file_idx);
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            prefix_expr(scrutinee, file_idx);
            for arm in arms {
                prefix_id(&mut arm.id, file_idx);
                prefix_expr(&mut arm.body, file_idx);
            }
        }
        Expr::Index { base, index, .. } => {
            prefix_expr(base, file_idx);
            prefix_expr(index, file_idx);
        }
        Expr::Field { base, .. } => prefix_expr(base, file_idx),
        Expr::MethodCall { base, args, .. } => {
            prefix_expr(base, file_idx);
            for a in args {
                prefix_expr(a, file_idx);
            }
        }
        Expr::Spawn { call, .. } => prefix_expr(call, file_idx),
        Expr::Call { args, .. } => {
            for a in args {
                prefix_expr(a, file_idx);
            }
        }
        Expr::Add { left, right, .. }
        | Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. }
        | Expr::Eq { left, right, .. }
        | Expr::NotEq { left, right, .. }
        | Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. }
        | Expr::And { left, right, .. }
        | Expr::Or { left, right, .. } => {
            prefix_expr(left, file_idx);
            prefix_expr(right, file_idx);
        }
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => prefix_expr(inner, file_idx),
    }
}

/// One `match` arm: `Enum::Variant(bindings...) => body`, or `_ => body`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    pub id: NodeId,
    pub enum_name: Option<String>,
    pub variant: Option<String>,
    pub bindings: Vec<String>,
    pub body: Expr,
}

impl MatchArm {
    /// True for the `_` wildcard arm.
    pub fn is_wildcard(&self) -> bool {
        self.variant.is_none()
    }
}

/// A top-level `fn` declaration, optionally generic (`fn f<T>(...)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDecl {
    pub id: NodeId,
    pub name: String,
    pub name_span: (usize, usize),
    pub is_pub: bool,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_ty: String,
    pub effects: Vec<Effect>,
    pub body: Block,
}

/// One `name: Type` parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: String,
}

/// A brace-delimited sequence of statements with its own identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: NodeId,
    pub stmts: Vec<Stmt>,
}

/// Lexical owner of all tasks spawned inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGroup {
    pub id: NodeId,
    pub body: Block,
}

/// `let name = value` binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetStmt {
    pub id: NodeId,
    pub name: String,
    pub name_span: (usize, usize),
    pub value: Expr,
    pub span: (usize, usize),
}

/// `return expr` terminator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnStmt {
    pub id: NodeId,
    pub value: Expr,
    pub span: (usize, usize),
}

/// Every statement form (v0.4: else-if desugared to nested `If`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Let(LetStmt),
    Assign(AssignStmt),
    Return(ReturnStmt),
    TaskGroup(TaskGroup),
    If(IfStmt),
    Print(PrintStmt),
    While(WhileStmt),
    ForRange(ForRangeStmt),
    ForIn(ForInStmt),
    Break(BreakStmt),
    Continue(ContinueStmt),
    Expr(Expr),
}

impl Stmt {
    pub fn id(&self) -> &NodeId {
        match self {
            Stmt::Let(s) => &s.id,
            Stmt::Assign(s) => &s.id,
            Stmt::Return(s) => &s.id,
            Stmt::TaskGroup(g) => &g.id,
            Stmt::If(s) => &s.id,
            Stmt::Print(s) => &s.id,
            Stmt::While(s) => &s.id,
            Stmt::ForRange(s) => &s.id,
            Stmt::ForIn(s) => &s.id,
            Stmt::Break(s) => &s.id,
            Stmt::Continue(s) => &s.id,
            Stmt::Expr(e) => e.id(),
        }
    }
}

/// `x = value` / `arr[i] = value` / `obj.field = value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignStmt {
    pub id: NodeId,
    pub target: AssignTarget,
    pub value: Expr,
}

/// Assignment target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignTarget {
    Var { name: String },
    Index { base: Box<Expr>, index: Box<Expr> },
    Field { base: Box<Expr>, field: String },
}

/// `while cond { ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhileStmt {
    pub id: NodeId,
    pub cond: Expr,
    pub body: Block,
}

/// `for name in start..end { ... }` (int range, end-exclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForRangeStmt {
    pub id: NodeId,
    pub var: String,
    pub start: Expr,
    pub end: Expr,
    pub body: Block,
}

/// `for name in iterable { ... }` (arrays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForInStmt {
    pub id: NodeId,
    pub var: String,
    pub iter: Expr,
    pub body: Block,
}

/// `break` / `continue` (loops only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakStmt {
    pub id: NodeId,
}

/// `continue` (loops only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinueStmt {
    pub id: NodeId,
}

/// `if cond { ... } else { ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfStmt {
    pub id: NodeId,
    pub cond: Expr,
    pub then_block: Block,
    pub else_block: Option<Block>,
}

/// `print expr` builtin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintStmt {
    pub id: NodeId,
    pub value: Expr,
}

/// Every expression form (v0.3: + floats, arrays, structs, logic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Int {
        id: NodeId,
        value: i64,
    },
    /// Float stored as raw bits (`f64::to_bits`) so `Eq` still holds.
    Float {
        id: NodeId,
        bits: u64,
    },
    Str {
        id: NodeId,
        value: String,
    },
    Bool {
        id: NodeId,
        value: bool,
    },
    ArrayLit {
        id: NodeId,
        elems: Vec<Expr>,
    },
    /// `{"key": value, ...}` map literal (string keys).
    MapLit {
        id: NodeId,
        entries: Vec<(String, Expr)>,
    },
    StructLit {
        id: NodeId,
        name: String,
        fields: Vec<(String, Expr)>,
    },
    /// `Enum::Variant(args...)` value construction.
    EnumCtor {
        id: NodeId,
        enum_name: String,
        variant: String,
        args: Vec<Expr>,
    },
    /// `match scrutinee { Enum::Variant(b...) => body, ... _ => body }`.
    Match {
        id: NodeId,
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Index {
        id: NodeId,
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Field {
        id: NodeId,
        base: Box<Expr>,
        field: String,
    },
    /// `base.method(args...)` — strings, arrays, maps.
    MethodCall {
        id: NodeId,
        base: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    Spawn {
        id: NodeId,
        call: Box<Expr>,
    },
    Await {
        id: NodeId,
        name: String,
    },
    Call {
        id: NodeId,
        func: String,
        /// Explicit `<T, ...>` type arguments at the call site (`f<T>(x)`).
        /// Empty for implicit-inference calls (`f(x)`); checked in HIR
        /// against the callee's `type_params` when present.
        type_args: Vec<String>,
        args: Vec<Expr>,
    },
    Var {
        id: NodeId,
        name: String,
    },
    Add {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Sub {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Mul {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Div {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Mod {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Eq {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    NotEq {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Lt {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    LtEq {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Gt {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    GtEq {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    And {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Or {
        id: NodeId,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Not {
        id: NodeId,
        inner: Box<Expr>,
    },
    Neg {
        id: NodeId,
        inner: Box<Expr>,
    },
}

impl Expr {
    pub fn id(&self) -> &NodeId {
        match self {
            Expr::Int { id, .. }
            | Expr::Float { id, .. }
            | Expr::Str { id, .. }
            | Expr::Bool { id, .. }
            | Expr::ArrayLit { id, .. }
            | Expr::MapLit { id, .. }
            | Expr::StructLit { id, .. }
            | Expr::EnumCtor { id, .. }
            | Expr::Match { id, .. }
            | Expr::Index { id, .. }
            | Expr::Field { id, .. }
            | Expr::MethodCall { id, .. }
            | Expr::Spawn { id, .. }
            | Expr::Await { id, .. }
            | Expr::Call { id, .. }
            | Expr::Var { id, .. }
            | Expr::Add { id, .. }
            | Expr::Sub { id, .. }
            | Expr::Mul { id, .. }
            | Expr::Div { id, .. }
            | Expr::Mod { id, .. }
            | Expr::Eq { id, .. }
            | Expr::NotEq { id, .. }
            | Expr::Lt { id, .. }
            | Expr::LtEq { id, .. }
            | Expr::Gt { id, .. }
            | Expr::GtEq { id, .. }
            | Expr::And { id, .. }
            | Expr::Or { id, .. }
            | Expr::Not { id, .. }
            | Expr::Neg { id, .. } => id,
        }
    }

    /// Decode a `Float` node's value.
    pub fn float_value(&self) -> Option<f64> {
        match self {
            Expr::Float { bits, .. } => Some(f64::from_bits(*bits)),
            _ => None,
        }
    }

    /// Mutable access to a node's identity (prefixing only).
    pub fn id_mut(&mut self) -> &mut NodeId {
        match self {
            Expr::Int { id, .. }
            | Expr::Float { id, .. }
            | Expr::Str { id, .. }
            | Expr::Bool { id, .. }
            | Expr::ArrayLit { id, .. }
            | Expr::MapLit { id, .. }
            | Expr::StructLit { id, .. }
            | Expr::EnumCtor { id, .. }
            | Expr::Match { id, .. }
            | Expr::Index { id, .. }
            | Expr::Field { id, .. }
            | Expr::MethodCall { id, .. }
            | Expr::Spawn { id, .. }
            | Expr::Await { id, .. }
            | Expr::Call { id, .. }
            | Expr::Var { id, .. }
            | Expr::Add { id, .. }
            | Expr::Sub { id, .. }
            | Expr::Mul { id, .. }
            | Expr::Div { id, .. }
            | Expr::Mod { id, .. }
            | Expr::Eq { id, .. }
            | Expr::NotEq { id, .. }
            | Expr::Lt { id, .. }
            | Expr::LtEq { id, .. }
            | Expr::Gt { id, .. }
            | Expr::GtEq { id, .. }
            | Expr::And { id, .. }
            | Expr::Or { id, .. }
            | Expr::Not { id, .. }
            | Expr::Neg { id, .. } => id,
        }
    }
}
