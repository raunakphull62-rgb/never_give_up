# Architecture

## Core Identity
Warden is a general-purpose language designed around one problem: AI writes
code fast, but verifying it is expensive, and no existing language was
designed for that. Warden makes ambiguity and "almost right" AI mistakes
surface as **compile errors**, not silent runtime bugs.

## Compiler Pipeline

```
Source text
  -> lossless parser with stable syntax-node IDs   (hand-written recursive descent)
  -> typed AST / HIR
  -> effect inference and contract collection       (throws, async, cancel)
  -> incremental query database                      (Salsa)
  -> typed derive expansion with origin maps          (append-only, no arbitrary macros yet)
  -> ownership-agnostic MIR
  -> async/task lowering to explicit state machines   (structured concurrency)
  -> machine code generation                          (Cranelift)
  -> package/build/test/documentation toolchain
  -> structured diagnostics and IDE protocol          (tower-lsp)
```

## Stack

| Layer | Choice | Why |
|---|---|---|
| Compiler core | Rust | Memory safety, mature ecosystem |
| Parser | Hand-written recursive descent | Full control over error recovery + stable node IDs |
| Incremental compilation | Salsa crate | Same framework rust-analyzer uses |
| IR (HIR/MIR) | Custom Rust structs/enums | Bespoke to effect/task design |
| Codegen | Cranelift | Pure Rust, simpler than LLVM, real machine code |
| LSP/IDE | tower-lsp | Free editor integration via LSP |
| Docs | mdBook | Same tool the Rust book itself uses |
| Demo/playground | Compiler -> WASM via wasm-bindgen | Client-side live compile-error demo |
| AI code-gen | Fine-tuned small open coder model | LoRA/QLoRA, see INTEGRATION.md |

## Structural Node Identity — Non-Negotiable Rule

This project has twice shipped a broken version of node-ID generation that
looked correct on the surface. Both failures are now permanent rules:

1. **No sequential/global counters as the sole source of identity.** A
   counter that increments across the whole file makes every node's ID
   shift when an unrelated sibling is added or removed earlier in the
   file. This defeats incremental compilation's early cutoff.

2. **A child's structural path MUST be generated while its parent's scope
   is still entered, not after the parent has already exited that scope.**
   The concrete failure mode already shipped once: `parse_task_group`
   called `enter_scope()` then `exit_scope()` back-to-back, before ever
   parsing its own children - so statements that are supposed to live
   inside a task_group's body were actually generated with the
   grandparent's path as their prefix, silently omitting the task_group's
   own segment. This is checked going forward (see FEATURES.md's test
   requirement) by asserting `child.path.starts_with(&parent.path)` for
   every construct that has children.

## Diagnostic Design
Diagnostics are structured objects, not strings, consumed identically by the
terminal renderer, IDE, and AI repair loop:

```json
{
  "code": "E-TASK-CANCEL",
  "severity": "error",
  "message": "child task may outlive its task group",
  "primary_span": {"file": "server.wd", "start": 184, "end": 209},
  "cause": "spawned task handle is not awaited or detached explicitly",
  "fixes": [
    {"label": "await task before leaving scope"},
    {"label": "mark task detached"}
  ],
  "rule": "structured-concurrency/lifetime"
}
```

## Design Test
Can a developer, IDE, compiler, test runner, and AI repair tool all agree on
the same semantic explanation of what went wrong? If not, adding another
feature moves the pain rather than removing it.
