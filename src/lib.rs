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
pub mod mcp;
pub mod mir;
pub mod modules;
pub mod ownership;
pub mod package;
pub mod parser;
pub mod repair;
pub mod runtime;

pub use ast::{Block, Effect, Expr, FunctionDecl, NodeId, Param, Program, Stmt};
pub use diagnostics::{Diagnostic, Fix, Span};
pub use hir::TypedHIR;
pub use parser::Parser;

/// Stack size for the deep-recursion worker (see [`with_deep_stack`]).
///
/// The front end is recursive descent and the checker/lowering walk the AST
/// recursively, so expression depth maps 1:1 onto native stack depth. The
/// default 8 MiB main-thread stack aborts the process at roughly 400-800
/// chained operands (`1+1+...`, which parses into a left-deep AST) -- a
/// generated or hostile file must never crash the compiler, so the CLI runs
/// its work on a worker with this budget instead.
pub const DEEP_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Run `f` on a worker thread with [`DEEP_STACK_BYTES`] of stack and return
/// its result. Panics inside `f` (including a stack overflow) are surfaced
/// as a panic here rather than aborting silently mid-compile.
pub fn with_deep_stack<T, F>(f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name("klang-deep".to_string())
        .stack_size(DEEP_STACK_BYTES)
        .spawn(f)
        .expect("cannot spawn deep-stack worker");
    match handle.join() {
        Ok(v) => v,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
