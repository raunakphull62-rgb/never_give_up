//! Klang compiler core: AST, parser, typed HIR, diagnostics + full foundation.

pub mod ast;
pub mod codegen;
pub mod contracts;
pub mod db;
pub mod derive;
pub mod diagnostics;
pub mod fmt;
pub mod hir;
pub mod jit;
pub mod lsp;
pub mod mir;
pub mod ownership;
pub mod package;
pub mod parser;
pub mod runtime;

pub use ast::{Block, Effect, Expr, FunctionDecl, NodeId, Param, Program, Stmt};
pub use diagnostics::{Diagnostic, Fix, Span};
pub use hir::TypedHIR;
pub use parser::Parser;
