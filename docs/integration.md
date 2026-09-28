# Integration

Covers how Warden's compiler development is actually being built (harness +
model hosting), and how the eventual AI code-generation feature will
integrate with the compiler. Two separate concerns - do not conflate them.

## Part 1: Development Harness (building the compiler itself)

### Current setup
- Compiler written in Rust, developed via an AI coding harness (Claude
  Code or Cline) driven against `master_prompt.md`'s rules.
- Model hosting: self-hosted on Kaggle (dual T4 GPUs) via Ollama, tunneled
  through ngrok, for zero API cost.

### Hard lessons from model/hosting selection (read before swapping models)

1. **Active parameters, not total parameters, determine speed on this
   hardware.** Qwen3-Coder-30B (3B active, MoE) was fast. Devstral-24B
   (24B active, dense) was slow despite being nominally "smaller." Prefer
   MoE architectures with few active parameters for this hardware.

2. **Check actual downloaded size with `ollama list` / `ollama show`
   before trusting a model's marketed VRAM footprint.** qwen3-coder-next
   was reported elsewhere as "fits ~16GB" but the actual Q4_K_M pull was
   51GB - 21GB over the 30GB dual-T4 budget. A model that doesn't fit will
   show a brief VRAM spike then collapse to near-zero as it falls back to
   CPU; `ollama ps`'s PROCESSOR column showing anything other than 100%
   GPU is a hard stop, not a tuning problem.

3. **LiteLLM's Anthropic-format (`/v1/messages`) endpoint had a confirmed
   tool-schema translation bug** when proxying to `ollama_chat` backends -
   tools sent in Anthropic's `name`/`description`/`input_schema` shape were
   silently dropped, while the same tools in OpenAI's
   `{"type":"function","function":{...}}` shape worked. Diagnosed by
   testing the identical request through both LiteLLM endpoints directly
   with `requests`, bypassing the harness entirely.

4. **The fix in use: a small custom Flask bridge**, not LiteLLM, translates
   Anthropic Messages format to/from Ollama's native `/api/chat` format.
   It is intentionally minimal (no streaming, no retries) and short enough
   to read end to end - trust it because it's auditable, not because it's
   a well-known library.

5. **Non-streaming is mandatory.** Every model/runtime combination tested
   showed unreliable structured tool calls under streaming
   (`Legacy <function> text seen: True`) that cleared up non-streaming.
   `stream: false` is set in every config in this project on purpose.

6. **`MAX_THINKING_TOKENS=0`** must be set in Claude Code's environment
   when pointing at any non-Anthropic backend - Claude Code sends a
   `thinking` parameter by default that most self-hosted models reject
   outright with a 400 error.

7. **Even with all of the above fixed, tool-call reliability on emulated
   (non-native) tool calling remains probabilistic, not guaranteed** -
   occasional malformed `<function=...>` leaks still occur under real,
   long agentic sessions even after every fix above. If this becomes
   disruptive, the honest fallback is real Anthropic API access
   (`claude` CLI logged into an actual account/API key), accepting the
   real cost for reliability on the compiler's own development.

### Verification discipline for whichever harness is used
Follow `master_prompt.md`'s 8 rules without exception, in particular:
- Never accept a claim of "fixed" without seeing the actual diff at every
  real call site (not just a new method defined next to the old one).
- Never accept a narrative summary in place of `grep`/`cargo build`/
  `cargo run` output you can verify yourself.
- Trace any identity/uniqueness claim through a concrete example with real
  printed values before accepting it.

## Part 2: Warden's Own AI Code-Generation Feature (future, post-Stage-1)

This is a completely separate system from Part 1 - it is a feature *of the
language*, built once the compiler itself is stable enough to verify
training examples.

- Approach: fine-tune (LoRA/QLoRA) an existing small open coder model on
  synthetic (prompt -> Warden code) pairs, rather than training from
  scratch.
- Data generation depends on Stage 1 being complete: every synthetic
  example must be verified by actually compiling it, so there is no
  training signal until the compiler exists.
- See ROADMAP.md for exact sequencing - this is deliberately one of the
  last things built, not the first.
