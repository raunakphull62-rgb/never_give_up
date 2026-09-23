# Phase 0 — Full Foundation Audit (code-verified)

Scope: the audit table the PRD's Phase 0 requires — every roadmap item and
subsystem, each cell backed by file/line evidence, test names, and real
pass/fail runs. No cell is filled from a planning document's claim alone.

Method: (1) source inspection, (2) `cargo test` on a real toolchain
(`cargo 1.98.1`, target dir moved off the non-executable mounted
`target/`), (3) live probes against the built binary
(`target/debug/klang check|run|repair`), (4) adversarial dogfood programs
combining generics + enums + modules + effects.

Environment note: this container has no GPU/LLM endpoint, so Phase 3
(live-model benchmark) cannot run here; everything else is executed, not
narrated.

Suite totals: **172 tests, 0 failed** (`cargo test`, 22 suites).

---

## 1. Roadmap items

### 1.1 Enums + pattern matching
- **Implemented?** Yes. `EnumDecl`/`EnumVariant` (`src/ast.rs:92-107`),
  `parse_enum` (`src/parser.rs:921`), `Expr::Match`/`MatchArm`
  (`src/ast.rs:302,519`), `check_match` (`src/hir.rs:1680`), MIR tag
  dispatch (`src/mir.rs:342-375`), fmt (`src/fmt.rs:117,278`).
- **Tested?** Yes, 15 tests in `tests/enum_gates.rs` (was 3 before this
  audit): dispatch, wildcard, exhaustiveness positive/negative, missing-all
  naming, duplicate arm, post-wildcard unreachable, binding arity,
  binding-type flow, non-enum scrutinee, unknown variant, cross-enum arm,
  repair end-to-end. Plus `tests/module_gates.rs:81` (qualified enums) and
  `tests/generic_gates.rs:62` (generic enums).
- **Documented?** Yes — `SPEC.md` §2b now states the real syntax
  (tuple-style `Variant(x: i32)`, `Enum::Variant(bindings)`) and the real
  codes (`E-MATCH-EXHAUSTIVE`, `E-MATCH-DUPLICATE`, `E-MATCH-UNREACHABLE`).
- **Repair-compatible?** Yes, verified 3 ways: `plan_scope` pins the
  function containing the match; the structured `E-MATCH-EXHAUSTIVE` JSON
  reaches the prompt; `enum_nonexhaustive_repair_end_to_end` converges on a
  mock backend; a live `--dry-run` against the binary shows the diagnostic
  JSON inside the prompt with zero model calls.
- **Known gaps?** PRD-draft code names differ from shipped names
  (`E-MATCH-NOT-EXHAUSTIVE`/`-NOT-ENUM`/`-FIELD-MISMATCH` →
  `E-MATCH-EXHAUSTIVE`/`E-TYPE`/`E-ARITY`); struct-style variants,
  partial `{ field, .. }` destructuring, and match guards are NOT in v1.
  Generic enums already exceed the PRD's NG1.

### 1.2 Generics
- **Implemented?** Yes. `type_params` on struct/enum/fn (`src/ast.rs:98,115,324`),
  `parse_type_params_opt` (`src/parser.rs:709`), rigid `Ty::Param` +
  `unify_generic`/`substitute_ty` (`src/hir.rs:100,1127,1166`).
- **Tested?** Yes, 6 tests in `tests/generic_gates.rs`: two instantiations
  of a generic fn, generic struct, swapped KV struct, generic enum match,
  conflicting instantiation → `E-TYPE`, fmt round-trip.
- **Documented?** Yes — `SPEC.md` §7 now lists generics as shipped (the
  old §7 said "no generics", which was stale).
- **Repair-compatible?** Yes: live probe P5 shows `E-TYPE` ("`same` arg 1:
  want i32, got str") and `repair --dry-run` scopes it to
  `function(s): same`.
- **Known gaps?** No trait bounds/typeclasses (declared out of scope). No
  generic methods on builtins; element types of arrays/maps stay dynamic.

### 1.3 Multi-file modules
- **Implemented?** Yes. `mod` parsing (`src/parser.rs:843`), flattening +
  visibility (`src/modules.rs:158-272`), `E-PRIVATE`
  (`src/diagnostics.rs:171`), plus cross-file `import` merging with
  `with_file_prefix` identity (`src/ast.rs:122`, `src/main.rs:548`).
- **Tested?** Yes, 10 tests in `tests/module_gates.rs` (2 added by this
  audit), incl. private-vs-pub, same-named private items in different
  modules, qualified struct construction + annotation, qualified enum
  match, identity containment, cross-file module use, fmt round-trip.
- **Documented?** Yes — `SPEC.md` §1/§7.
- **Repair-compatible?** Yes for functions; declaration-level privacy
  fixes now escalate to file scope (see F5 below).
- **Known gaps?** See **F1** (fixed during this audit).

---

## 2. Subsystems

| Subsystem | Implemented | Tested | Documented | Repair-compatible | Known gaps |
|---|---|---|---|---|---|
| Parser | `src/parser.rs:1903` lines, recursive descent + structural scopes | `tests/parse_gates.rs` (3), `tests/lang*_gates.rs`, all suites parse | `SPEC.md` §1-§3 | n/a (parse errors reach the loop as `E-PARSE`) | no match guards / tuple variants / partial destructuring |
| HIR / type checker | `src/hir.rs:2236`, effect + name + type + enum + generic checks | `type_gates` (13), `enum_gates` (15), `generic_gates` (6), `tasks_gates` (4) | `SPEC.md` §2, §2b | Yes — `Diagnostic::to_json()` is the prompt payload | most `primary_span`s are `0,0` (see F2/F6) |
| Effect system | `throws`/`async`/`cancel`, `E-EFFECT-MISMATCH`, `E-TASK-CANCEL` | `lang_gates:137,146`, `stage1_gates`, `tasks_gates` | `SPEC.md` §3 | Yes — caller attribution pinned by tests | effect union across calls is conservative |
| Structured diagnostics | `src/diagnostics.rs` (252), JSON + `related` | `foundation_gates`, `type_gates`, `enum_gates` | `architecture.md`, `SPEC.md` | Yes (the contract the loop consumes) | `start/end` often `0,0` |
| `klang repair` | `src/repair/*` (1669 lines): config, backend, prompt, scope, splice, driver, log | `repair_gates` (**19** tests, +6 this audit) + unit tests in each module | `SPEC.md` §5 + scope-boundary rules | n/a (it is the mechanism) | declaration-level edits handled by escalation, not by scope-local editing (F5) |
| Formatter | `src/fmt.rs` (339) | `toolchain_gates` (3), round-trips in `module_gates`/`generic_gates` | `SPEC.md` §5 | Yes | no enum/match-specific golden test beyond round-trip |
| Package manager | `src/package.rs` (220), manifest + FNV lockfile | `package_gates` (3) | `SPEC.md` §6 | n/a | FNV is change-detection, not crypto |
| MIR + runtime | `src/mir.rs` (1104), `src/runtime.rs` (1316) | `foundation_gates`, `lang*_gates`, `tasks_gates` | `SPEC.md` §4 | n/a | interpreter default; JIT is int-only |
| Salsa DB | `src/db.rs` (155) | `salsa_gates` (4), `salsa_spike` (2) | `db.rs` header | n/a | single-file inputs only |
| LSP / ownership / derive / contracts | JSON renderer, Managed-only modes, append-only derive, `repair_loop` | `foundation_gates` (9) | `SPEC.md` §8 | `repair_loop` is the loop primitive used by `klang repair` | no real LSP server; ownership modes deferred |

---

## 3. Live probe log (built binary, this container)

| Probe | Input | Result |
|---|---|---|
| P1 | `enum Opt<T>` + match | `check: OK` |
| P2 | `struct Box<T>` | `check: OK` |
| P3 | `fn identity<T>` | `check: OK` |
| P4 | generic struct + plain struct | `check: OK` |
| P5 | `same(1, "hi")` | `E-TYPE` (`want i32, got str`) |
| P6 | `mod lexer { pub fn scan }` + call | `check: OK`, `run main() = 42`, MIR shows `lexer::scan` |
| P7 | private cross-module call | `E-PRIVATE` |
| P8 | duplicate `fn main` | parse-time `E-DUPLICATE` |
| P9 | `import "mylib.klang"` | merged `check: OK`, `run main() = 42` |
| P10 | `add(1)` | `E-ARITY` |
| P11 | `return nope + 1` | `E-UNDEFINED` |
| P12 | non-exhaustive match | `E-MATCH-EXHAUSTIVE` naming `Opt::None` |
| P13 | enum + struct + generic + mod + effects in one program | **FAILED before F1 fix**, `run main() = 42` after |
| P14 | `repair --dry-run` on P5 | scope `function(s): same`, zero model calls |
| P15 | `repair --dry-run` on P12 | scope `function(s): f`, diagnostic JSON in prompt |

---

## 4. Findings (each fixed-with-regression-test or explicitly triaged)

### F1 — BUG (high): modules double-counted top-level declarations — FIXED
- Symptom: any file containing BOTH a `mod` block and a top-level
  `enum`/`struct` was rejected with spurious `E-DUPLICATE` diagnostics and
  could not be checked or run at all (probe P13: 3 duplicates for valid
  code). This breaks exactly the v0.2 combination the roadmap targets.
- Cause: `src/modules.rs:209-215` seeded the flattened program with
  `program.enums.clone()`/`structs.clone()` and then lines 222-239 pushed
  the same (rewritten) top-level items again. The `mods.is_empty()` early
  return at line 159 masked it for module-free files, and no existing gate
  combined the two constructs.
- Fix: initialize `flat.structs`/`flat.enums` empty; the top-level loops
  are the single source of truth (`src/modules.rs:209-221`).
- Regression tests: `module_with_top_level_enum_no_spurious_duplicates`,
  `module_with_top_level_struct_no_spurious_duplicates`
  (`tests/module_gates.rs`) — assert clean check, flattening counts, and
  execution.
- Verified live: probe P13 now `check: OK` + `run main() = 42`.

### F2 — BUG (medium): `match/*` diagnostics over-widened repair scope — FIXED
- Symptom: `E-MATCH-EXHAUSTIVE` names the enum, so the use-site scan put
  every function mentioning that enum into scope (`pick, main`), and a
  single-function model response was then refused as "missing repaired
  function `main`" — the repair could not converge even when correct.
- Cause: `scope::attribute` fell through to the generic token scan.
- Fix: `match/*` diagnostics prefer the function whose body both contains
  a `match` and mentions the diagnostic's non-function token
  (`src/repair/scope.rs` step 2b).
- Regression test: `match_diagnostic_scopes_to_function_containing_the_match`
  (`tests/repair_gates.rs`) — asserts `Functions(["pick"])` exactly, plus
  convergence with declarations and untouched functions preserved.
- Verified live: a fn-only model response now converges (`repair: OK in 1
  attempt(s)`) with enum + `helper` + `main` byte-identical.

### F3 — BUG (high): function-scope splice silently dropped declaration edits — FIXED
- Symptom: if the correct fix was a declaration edit (or the model re-emitted
  declarations), the splicer replaced only the target `fn` and silently
  discarded the declaration text; the loop then re-failed with the original
  diagnostic, or worse, wrote a file that no longer matched the model's fix.
- Fix: `splice::response_declarations` detects `enum`/`struct`/`mod`/`import`
  text outside function spans (statement-position lexing, so comments and
  strings never register). A response containing declarations is accepted
  only as a complete file (all original functions AND declarations present);
  otherwise it fails loudly with a message naming what is missing. Full
  rationale and ordering in `src/repair/splice.rs`.
- Regression tests: `declaration_edit_in_incomplete_response_is_never_silently_dropped`,
  `declaration_edit_with_complete_file_is_accepted`,
  `declaration_free_response_preserves_original_declarations`,
  `declaration_detection_ignores_comments_and_strings`
  (`tests/repair_gates.rs`).
- Verified live: incomplete-declaration response → loud failure, exit 1, no
  output file written; complete-file response → success; fn-only response →
  success with declarations untouched.

### F4 — DOC DRIFT: `SPEC.md` claimed "no generics" — FIXED
- Generics shipped (`tests/generic_gates.rs`) but §7 listed them as absent,
  and enums/modules were not listed anywhere. `SPEC.md` §7/§8 rewritten
  against actual behavior.

### F5 — DECISION (Phase 1 requirement): declaration-level repair boundary — IMPLEMENTED
- Decision: declaration-level diagnostics never claim function scope.
  `scope::DECLARATION_RULES = ["names/duplicate", "modules/visibility",
  "ownership/mode"]` + `is_declaration_level()`; `plan_scope` returns
  `File` for them. Rationale: a declaration edit is not expressible by
  function-scoped splicing, and name-keyed splicing is ambiguous exactly
  when two items share a name.
- Boundary control test proves body-level diagnostics still take the
  function-scope path: `declaration_level_diagnostics_force_file_scope`.
- Documented in `SPEC.md` §5 and `src/repair/scope.rs`.

### F6 — TRIAGED (not changed): `MAX_FUNCTION_SCOPE = 2` has no recorded rationale
- Single named constant (`src/repair/scope.rs:14`), referenced once
  (line ~190). No justification is recorded for 2 vs 3/4 — stated plainly,
  not invented. Raising it is a one-line change with no structural
  knock-ons; the only real effect is prompt size (`prompt.rs:47-65` embeds
  every target's source plus the full file). Left at 2 deliberately.

### F7 — TRIAGED (known limit): most HIR diagnostics carry `0,0` spans
- `primary_span` is `input.warden,0,0` for most HIR diagnostics, so scope
  attribution is heuristic (name/use-site based) rather than span-derived.
  Mitigations in place: full-program re-check after every splice, loud
  refusal instead of silent drops (F3), and declaration escalation (F5).
  Threading real token spans through HIR checks remains the principled fix
  and is the largest known debt in the repair path.

---

## 5. Phase 0 exit criteria

Every cell above is filled with file/line references, named tests, or live
probe output. Three real bugs (F1, F2, F3) and one doc drift (F4) were found
by doing this audit; all three bugs are fixed with permanent regression
tests, and none were known before this pass.

## 6. Test-count delta (this audit)

| Suite | Before | After |
|---|---|---|
| `enum_gates` | 3 | 15 |
| `module_gates` | 8 | 10 |
| `repair_gates` | 13 | 19 |
| `robustness_gates` (new) | 0 | 4 |
| **Total (all suites)** | **152** | **176** |

All 23 suites pass: `cargo test` → `176 passed; 0 failed`. Examples
regression: `examples/{simple,full,lib,stdlib_demo}.klang` all
`check: OK` and `main()` runs to 42; the no-argument gate demo still
prints `all klang full-language stages hold`.

## 7. Phase 2 — bug-finding budget (set explicitly, per PRD risk row)

Proposed budget, to be honoured as-is unless revised in writing:
- **4 sessions**, each time-boxed to one subsystem sweep, in this order:
  (1) parser+fmt fuzz/edge cases, (2) HIR/type-checker adversarial cases,
  (3) repair-loop adversarial sessions across every diagnostic rule
  (`names/duplicate`, `modules/visibility`, `match/*` were untested before
  this audit — they are the highest-yield targets), (4) dogfood a
  non-trivial program using generics + enums + modules + effects together.
- Exit: every finding either fixed with a regression test or recorded here
  as a documented limitation. No finding may be dropped silently.
- Session 3 was effectively started during this audit and produced F1-F3.

## 8. Phase 3 — benchmark status

Not runnable in this environment: it requires at least two live model
endpoints (no GPU/network model access here), and the PRD sequences it
strictly after Phase 2. What exists for it already:
- offline harness pieces: `MockBackend` (deterministic multi-response queue),
  `.klang-repair-log.json` attempt records, `iters_used`, and
  `--dry-run` for zero-cost prompt inspection;
- the metrics the PRD asks for map onto existing fields: syntax validity and
  semantic correctness come from `check_candidate`/`runtime::run_with_output`,
  iterations-to-convergence from `RepairOutcome::iters_used`, and the
  "almost-right" category from the diagnostic codes
  (`E-TYPE`/`E-ARITY`/`E-EFFECT-MISMATCH`/`E-MATCH-EXHAUSTIVE`).
- Remaining work for Phase 3: task corpus (simple/moderate/complex tiers),
  two-endpoint runner script, and a results table checked into the repo.

### F8 — BUG (high): deep expressions aborted the process — FIXED
- Found by Phase 2 Session 1 (parser/HIR fuzzing), not by any hand-written
  test: `fn main() -> i32 { return 1+1+...+1 }` with ~800+ operands parsed
  fine, then blew the stack during checking and aborted with
  `fatal runtime error: stack overflow` (exit 134). Bisected: 400 operands
  OK, 800 aborts. Cause: the AST for a flat left-associative chain is
  left-deep, so `check_expr`/`lower_expr_to_value`/`emit_listing` recurse
  once per operand on the default 8 MiB main-thread stack.
- Why it matters here specifically: this project's value proposition is
  "AI mistakes surface as compile errors, not silent runtime bugs". A
  compiler that *aborts* instead of reporting on large generated input
  breaks that promise at exactly the input class AI produces.
- Fix: `klang::with_deep_stack` / `klang::DEEP_STACK_BYTES` (256 MiB worker
  thread) in `src/lib.rs`; `main.rs` runs the whole CLI inside it.
- Regression tests: `tests/robustness_gates.rs` —
  `deep_flat_expression_does_not_crash_the_compiler` (2000 operands:
  parse → check → lower → listing),
  `deep_flat_expression_runs_and_returns_expected_value` (1000 operands
  → 1001, i.e. correctness not just survival),
  `deep_nesting_does_not_crash_the_compiler` (200 nested parens, 300 nested
  blocks), `malformed_input_is_a_diagnostic_never_a_panic` (empty file,
  truncated `fn`, unterminated string/block comment, garbage bytes, 100 KB
  comment, 2000 statements).
- Verified live: the original repro now `check: OK` (exit 0) and a
  20 000-operand chain also checks clean.
- Residual limit (triaged): the ceiling is now ~10^5 operands rather than
  ~10^3; an explicit AST-depth limit with a dedicated diagnostic would be
  the principled bound and is recorded as remaining debt.
