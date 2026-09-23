PRD: Klang Compiler-in-the-Loop Repair (klang repair)
Status: Draft v1 Owner: Raunak Phull Component: Klang compiler toolchain (Rust) Related: contracts.rs (repair_loop, FixIt), diagnostics.rs (Diagnostic), main.rs (check/run/build CLI)

1. Problem Statement
Research across two prior investigations established:

Grammar-constrained decoding gets AI-generated Klang code to near-100% syntactic validity, but does essentially nothing for semantic correctness.
The dominant failure mode for AI-generated code in a low-resource/novel language is not wrong algorithm choice (models pick the right approach ~80% of the time) but buggy implementation of a correct idea — "almost-right" mistakes.
Iterative compiler-feedback loops (ATLAS, LLMLOOP) close this gap far more reliably than one-shot generation, typically within 2–8 attempts.
Klang already emits structured, machine-parseable diagnostics (Diagnostic::to_json()) and has a bounded-retry primitive (contracts::repair_loop). Neither is currently wired to an actual model call — repair_loop is exercised today only with a hand-written no-op closure. There is no path from "AI generates broken Klang" to "AI regenerates working Klang using the compiler's own errors."

Goal of this feature: close that loop. Given Klang source that fails to check, automatically drive an LLM through bounded, diagnostic-guided regeneration until the code compiles clean or the attempt budget is exhausted.

2. Goals
G1: A klang repair <file> command that takes failing Klang source and attempts to fix it automatically using an LLM, guided by the compiler's own structured diagnostics.
G2: Work with any general-purpose model (not a fine-tuned one) reachable via an OpenAI-compatible or Anthropic-compatible HTTP endpoint — including the user's existing self-hosted Kaggle/ngrok setup.
G3: Scoped regeneration — repair the smallest unit necessary (function-level, using existing NodeId structural paths) rather than always regenerating the whole file.
G4: Full transparency — every attempt, prompt, and diagnostic diff is logged and inspectable; nothing is silently rewritten.
G5: Bounded and safe — hard iteration cap, no infinite loops, no network calls without explicit opt-in.
3. Non-Goals (v1)
NG1: Not a general "AI pair programmer" — no free-form chat, no feature generation from natural language. Input is always existing Klang source that fails check.
NG2: Not model training/fine-tuning. This feature is prompt-only, per the project's "any AI" design goal.
NG3: Not an IDE/LSP integration in v1 — CLI only. LSP wiring is a fast-follow once the core loop is proven.
NG4: Not a guarantee of success — some diagnostics (deep logic errors, missing business requirements) are not compiler-detectable and won't be fixed by this loop.
4. Users & Use Cases
User	Use case
Raunak, developing Klang	Validate that the diagnostics format + effect system are actually sufficient signal for a model to self-correct
A developer using an AI assistant to write Klang	Assistant generates code, klang check fails, klang repair closes the gap without a human round-trip
CI pipeline	Fail-fast: if klang repair can't converge within budget, block merge and surface the final diagnostics to a human
5. Functional Requirements
5.1 CLI surface
klang repair <file.klang> [entry] \
  [--max-iters N]         # default 5
  [--model <endpoint>]    # default: value from klang.toml or $KLANG_MODEL_ENDPOINT
  [--scope function|file] # default: function
  [--dry-run]             # show planned prompts/diffs, make no model calls
  [--write]               # apply successful repair in place (default: write to .repaired.klang)
  [--verbose]             # print every attempt's diagnostics + model response
Backward compatible with existing check/run/build/fmt subcommand pattern already in main.rs.

5.2 Repair loop behavior
Parse + TypedHIR::check the input file.
If clean → exit 0, no-op, print "check: OK (0 diagnostics), nothing to repair".
If failing: a. Serialize diagnostics via existing Diagnostic::to_json(). b. Identify the smallest enclosing scope for each diagnostic's primary_span using existing NodeId structural paths (function → containing block → statement). c. If --scope function (default) and all diagnostics fall within a bounded set of functions, construct a repair prompt containing: those functions' current source, their diagnostics as JSON, and a fixed system prompt describing Klang's grammar rules (reuse the existing SPEC.md/language-reference content as the system prompt body). d. If diagnostics span more than N functions or scope is file, fall back to whole-file regeneration. e. Call the model. Parse the response as Klang source for the target scope(s) only. f. Splice the regenerated scope(s) back into the full program, preserving everything untouched (use existing NodeId prefixing/import-merging logic in load_with_imports as the pattern for safe recombination). g. Re-run TypedHIR::check on the recombined program. h. If clean → success, exit loop. i. If still failing → feed the new diagnostics into the next iteration (do not repeat prior failed prompts verbatim; include a short history of what was already tried, to avoid the model repeating the same mistake).
This entire sequence is one call to contracts::repair_loop(max_iters, attempt), where attempt is the new closure performing steps (c)–(h) instead of today's no-op.
On exhausting max_iters without success: exit nonzero, print the final diagnostics (same JSON format as check), and (if --verbose) the full attempt history.
5.3 Model integration
Model calls go through a small trait (ModelBackend) with one method: fn complete(&self, system: &str, user: &str) -> Result<String, ModelError>.
v1 ships one implementation targeting an OpenAI-compatible /v1/chat/completions endpoint (covers the user's existing Kaggle/ngrok DeepSeek Harness setup and most self-hosted/hosted options without new integration work per provider).
Endpoint, model name, and API key resolved from (in order): --model flag → klang.toml [repair] section → $KLANG_MODEL_ENDPOINT/$KLANG_MODEL_KEY env vars.
No network call is made without one of the above being explicitly configured; absence is a clear error, not a silent default to some hosted service.
5.4 Diagnostics → prompt contract
The existing Diagnostic struct already carries everything needed: code, severity, message, primary_span, cause, fixes: Vec<Fix>, related.

The prompt template renders each diagnostic as-is from to_json() plus the relevant source slice — no new diagnostic fields are required for v1. If early testing shows the model needs more (e.g., the full type of both sides of a mismatch), extend Diagnostic rather than inventing a parallel format.

5.5 Safety / bounds
Hard iteration ceiling (--max-iters, default 5, hard max 10 regardless of flag).
Per-attempt timeout on the model call (default 60s), configurable.
--dry-run must never make a network call.
Regenerated code is never written to the original file unless --write is passed; default output is a sibling .repaired.klang file so a human can diff before accepting.
Import safety checks already in load_with_imports (rejecting absolute paths, .. escapes) apply unchanged to any model-generated import statements.
6. Non-Functional Requirements
Determinism of the non-model parts: given the same diagnostics and model output, splicing and re-checking must be fully deterministic — no reliance on wall-clock or randomness outside the model call itself.
Observability: every attempt logged with: diagnostics in, prompt sent (redacted of nothing — full transparency), raw model response, diagnostics out. Written to .klang-repair-log.json alongside the target file when --verbose or always in CI mode.
No regression to existing commands: check/run/build/fmt behavior and exit codes must be unchanged.
7. Success Metrics
Given the field's own uncertainty here (no published benchmark separates "almost-right" errors from general semantic errors), v1 success is evaluated on Klang's own test corpus, not against external claims:

Metric	Target for v1
Convergence rate within 5 iterations, on the existing examples/*.klang corpus deliberately mutated to introduce known "almost-right" bugs (off-by-one, wrong effect annotation, type mismatch)	≥ 70%
Convergence rate on the same corpus with unrelated code untouched (no regression introduced by repair)	100% (hard requirement — a repair that fixes one diagnostic while breaking previously-passing code is a failure, not a partial success)
Median iterations to convergence	≤ 2 (in line with ATLAS's reported average)
Function-scoped vs. whole-file regeneration: fraction of successful repairs achieved at function scope	Tracked, no hard target for v1 — informs whether scope logic is worth its complexity
8. Rollout Plan
Phase 1 — Proof of concept (internal only) Wire contracts::repair_loop's closure in main.rs's existing gate-5 leak-demo path to a real model call against the Kaggle/ngrok endpoint. No new CLI surface. Validates the core signal (does the model actually fix E-TASK-CANCEL-class errors given the JSON diagnostic?) before investing in scoping/splicing logic.

Phase 2 — klang repair CLI, file-scope only Ship the command with --scope file as the only mode. Simplest correct implementation; establishes the logging/safety scaffolding.

Phase 3 — Function-scoped regeneration Add --scope function using NodeId paths for splicing. This is the harder, higher-value piece — defer until file-scope is proven and there's real data on how often diagnostics cluster in one function vs. spread across many.

Phase 4 — CI integration + docs klang repair --dry-run as a CI check; documentation and examples folded into SPEC.md.

9. Risks & Open Questions
Risk	Notes
Model regenerates function-scoped code that's syntactically valid but breaks an invariant enforced elsewhere (e.g., a caller's effect expectations)	Mitigated by always re-running full-program TypedHIR::check after splicing, not just checking the regenerated scope in isolation
Repeated identical failed attempts (model gives the same wrong answer twice)	Attempt history included in later prompts (5.2.i); if a repeat is detected, escalate to whole-file scope on the next attempt rather than retrying the same scope
Endpoint cost/latency at CI scale	Out of scope for v1 (single Kaggle-hosted endpoint, no rate-limit handling beyond timeout); revisit if CI usage grows
No current data on how often diagnostics span multiple functions vs. stay local	Open — Phase 1/2 data should answer this before committing to Phase 3's complexity
Should FixIt labels be used as additional prompt hints, or are raw diagnostics sufficient?	Open — try both in Phase 1 and compare convergence rate
10. Out of Scope for This Doc
Language design changes (mandatory requires/ensures extension to the effect system) — tracked separately.
Fine-tuning path — explicitly rejected per project's "any AI" goal; would only be revisited if repair-loop convergence proves insufficient on its own.
