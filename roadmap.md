# Build Roadmap

Restarted at Stage 1 after clearing the codebase - the planning docs
(ARCHITECTURE.md, FEATURES.md, this file) survive; the code does not.
Two real bugs shipped in the previous attempt are now hard gates below,
not just lessons - Stage 1 is not "done" until they demonstrably cannot
recur.

## Stage 1: Typed core + effect representation (current)

**Deliverable:** parser, AST, typed HIR, `throws`/`async`/`cancel`
effects, structured (JSON-object) diagnostics.

**Hard gates before Stage 1 is considered complete** (see FEATURES.md
"Mandatory Test Requirements" for the full list):
- [ ] Exactly one `NodeId` struct, one `Parser` struct, in the whole codebase
- [ ] Every construct with children generates that child's path while the
      parent's `enter_scope()` is still active, verified by an automated
      `starts_with()` assertion test, not a printed demo
- [ ] `cargo build` with zero errors, zero warnings about dead/unused
      NodeId-related code
- [ ] `cargo run` prints real, distinct path values for: two top-level
      siblings, two statements inside the same task_group, a task_group
      nested inside a function

**Milestone program (must fully parse + effect-check + emit the
scope-violation diagnostic):**
```
fn fetch_a() -> i32 throws { ... }
fn fetch_b() -> i32 throws { ... }

fn combine() -> i32 throws async {
    task_group {
        let a = spawn fetch_a()
        let b = spawn fetch_b()
        return await a + await b
    }
}
```

## Stage 2: Incremental query compiler
Salsa-style inputs, tracked queries, memoization, early cutoff. Blocked on
Stage 1's node-identity gates actually holding - an unstable identity
scheme poisons every query built on top of it.

## Stage 3: Structured concurrency runtime
Single-threaded scheduler, lexical task groups, cancellation tokens,
exception groups. No detached tasks yet.

## Stage 4: Unified toolchain and package graph
Manifest, lockfile, registry client, build/test/check/fmt/doc commands.

## Stage 5: Inspectable derive/metaprogramming
Typed, append-only derives with origin maps and an IDE expansion view.

## Stage 6: AI-legible diagnostics and contracts
JSON diagnostic schema, fix-it protocol, executable contracts, a bounded
AI repair loop.

## Stage 7: Gradual ownership and optimization modes
Managed default, explicit resource types, move-only values, scoped
borrows, unsafe FFI boundary.

## Project Ladder (after the core compiler works)
1. Public playground/demo site (WASM build)
2. AI code reviewer CLI - verify AI-generated code from any language
3. Self-host the compiler in its own language
4. Real dogfood target - one utility inside Meridian OS or Vex tooling
5. MCP server exposing the verification engine to other AI coding agents

## Fine-Tuning Pipeline (parallel track, blocked on Stage 1)
See INTEGRATION.md Part 2. Not started until Stage 1's compiler can verify
synthetic training examples by actually compiling them.
