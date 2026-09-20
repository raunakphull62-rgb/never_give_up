# Planning Document for Stage 1 Implementation

## Overview
This document outlines the plan for implementing Stage 1 of the language project, focusing on a typed core with effect representation (`throws`, `async`, `cancel`). The plan includes required tasks, verification steps, and hard gates that must be satisfied before Stage 1 is considered complete.

## Required Tasks
1. **Parser and AST Construction**
   - Implement a parser that generates an Abstract Syntax Tree (AST) for the language.
   - Ensure the AST captures all necessary constructs (`throws`, `async`, `cancel`, `task_group`, etc.).

2. **Typed HIR (High-Level Intermediate Representation)**
   - Convert the AST into a typed HIR that includes effect annotations.
   - Validate that each function signature explicitly declares visible effects.

3. **Structured Concurrency Model**
   - Implement `task_group` and `spawn` constructs.
   - Ensure child tasks are tied to the enclosing `task_group` and cannot outlive it.

4. **Error Handling with `throws`**
   - Design a typed failure channel mechanism.
   - Propagate failures using a low-friction operator.

5. **Diagnostic System**
   - Emit structured JSON diagnostics for every compiler error.
   - Ensure diagnostics are machine-parseable.

## Hard Gates (Verification Steps)
These steps must be satisfied before Stage 1 is marked complete.

1. **Unique Node Identifiers**
   - Verify that there is exactly one `NodeId` struct and one `Parser` struct in the entire codebase.
   - Use automated tests to ensure no duplicate definitions exist.

2. **Scope Path Assertions**
   - Implement assertions that every child node's path `starts_with()` its parent's path while the parent's scope is active.
   - Run tests to confirm this invariant holds for all constructs (`FunctionDecl`, `TaskGroup`, `Block`).

3. **Zero Build Errors/Warnings**
   - Ensure `cargo build` produces no errors or warnings related to dead/unused `NodeId` code.

4. **Real Path Value Output**
   - Modify the runtime to print distinct path values for:
     - Two top-level sibling functions.
     - Two statements inside the same `task_group`.
     - A nested `task_group` inside a function.
   - Use assertions (`assert_ne!`, `assert!(path.starts_with(parent_path))`) to enforce correctness.

## Milestone Program
The following code snippet must be parsed, effect-checked, and emit the correct scope-violation diagnostic if any:
```rust
fn fetch_a() -> i32 throws { /* ... */ }
fn fetch_b() -> i32 throws { /* ... */ }

fn combine() -> i32 throws async {
    task_group {
        let a = spawn fetch_a();
        let b = spawn fetch_b();
        return await a + await b;
    }
}
```

## Next Steps
1. **Implement Parser**
   - Start with lexical analysis, then AST construction.
2. **Add Effect Annotations**
   - Extend the AST nodes to carry `throws`, `async`, and `cancel` markers.
3. **Build Diagnostic Emitter**
   - Create a JSON emitter that outputs structured diagnostics.
4. **Run Verification Tests**
   - Execute the hard gate tests to ensure all invariants hold.

## References
- **FEATURES.md**: Mandatory Test Requirements and effect definitions.
- **ROADMAP.md**: Stage 1 description and subsequent milestones.
- **ARCHITECTURE.md**: High-level design of the compiler pipeline.

---
*Prepared by the planning assistant on 2026-09-20.*