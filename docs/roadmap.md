# Build Roadmap

## Stage 1: Typed core + effect representation - VERIFIED COMPLETE

Parser, AST, typed HIR, throws/async/cancel effects, structured JSON
diagnostics, structured concurrency (task_group/spawn/await), Salsa
incremental compilation, a formatter, and a basic package manager.

Verified via 79 passing tests across 12 test files, run with real cargo
test output (not narration), including the exact hard gates this project
required after two earlier broken attempts:
- Exactly one NodeId struct, one Parser struct
- parent_prefix_containment_everywhere: every child's path starts_with
  its parent's path, generated while the parent's scope was still
  entered
- sibling_functions_have_distinct_paths / stmts_in_same_block_are_distinct
- unawaited_spawn_emits_task_cancel / spawn_outside_group_is_error

## Stage 1.5: Maturity Phase (current) - enums, generics, modules

See features.md's "v0.2 - Maturity Phase" section for full scope. Build
order: enums/match first, then generics, then modules - one at a time,
each verified with real passing tests before the next begins. Do not
attempt self-hosting during this phase.

## Stage 2: Self-hosting attempt (blocked on Stage 1.5)

Not started until every v0.2 feature has real, passing tests. First
target: rewrite only the lexer in Klang itself, not the full compiler -
the smallest possible self-hosting slice, to surface any remaining
maturity gap cheaply before committing to a full self-hosted parser and
type checker.

## Stage 3+: Everything else already partially built ahead of schedule

The codebase currently contains substantial work beyond Stage 1's
original scope (mir.rs, codegen.rs, jit.rs, lsp.rs, ownership.rs,
package.rs, contracts.rs, derive.rs) built during a session that
exceeded its assigned task. This code is currently left in place because
it passes its own tests, but it was not reviewed against this roadmap's
original sequencing. Treat it as unverified-against-plan until each
piece is explicitly revisited in its proper stage, rather than assuming
it is production-ready simply because it exists and compiles.

## Project Ladder (after Stage 1.5, before or alongside Stage 2)
1. Public playground/demo site (WASM build) - buildable now, since
   Stage 1's parser/type checker/diagnostics all work end to end
2. AI code reviewer CLI
3. Self-host the compiler in its own language (see Stage 2)
4. Real dogfood target - one utility inside Meridian OS or Vex tooling
5. MCP server exposing the verification engine to other AI coding agents
