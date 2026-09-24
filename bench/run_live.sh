#!/usr/bin/env bash
# Phase 4 harness-signal check (replaces the Phase 3 live-model runner).
#
# `klang repair` no longer dials a model (Phase 4 correction), so
# repair-assisted convergence is measured inside harnesses, not here.
# This script verifies the signal those harnesses consume: for each
# corpus task (mirrored from tests/benchmark_gates.rs) it drives
# `klang mcp` exactly as a harness would — klang_check for diagnostics,
# klang_scope_plan for the edit scope — with zero model calls, and
# records one CSV row per task:
#   task,tier,check_clean,diag_code,scope
# Usage: bench/run_live.sh
# Needs: python3 (JSON-RPC framing), KLANG_BIN or ./target/debug/klang.
set -u
BIN=""
# NOTE: `[ -x ]` is unreliable for root on noexec mounts (it reports true
# for target/debug/klang and exec still fails), so every candidate is
# probe-executed, not just permission-checked.
for cand in "$CARGO_TARGET_DIR/debug/klang" "${KLANG_BIN:-}" ./target/debug/klang; do
  [ -n "$cand" ] || continue
  if [ -x "$cand" ] && printf '' | "$cand" mcp >/dev/null 2>&1; then
    BIN="$cand"
    break
  fi
done
if [ -z "$BIN" ]; then
  echo "bench/run_live.sh: no working klang binary (set KLANG_BIN or CARGO_TARGET_DIR)" >&2
  exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "bench/run_live.sh needs python3 for MCP framing" >&2
  exit 2
fi
TMP="$(mktemp -d)"
write_tasks() { # writes the 8 corpus broken sources into $TMP
  cat > "$TMP/S1.klang" <<'EOF'
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    return add(1)
}
EOF
  cat > "$TMP/S2.klang" <<'EOF'
fn main() -> i32 {
    return x
}
EOF
  cat > "$TMP/S3.klang" <<'EOF'
fn main() -> i32 {
    return "hi"
}
EOF
  cat > "$TMP/M1.klang" <<'EOF'
enum Opt { Some(x: i32), None }

fn pick(v: Opt) -> i32 {
    return match v { Opt::Some(n) => n }
}

fn main() -> i32 {
    return pick(Opt::Some(1))
}
EOF
  cat > "$TMP/M2.klang" <<'EOF'
fn same<T>(a: T, b: T) -> T {
    return a
}

fn main() -> i32 {
    return same(1, "hi")
}
EOF
  cat > "$TMP/M3.klang" <<'EOF'
fn f() -> i32 throws {
    return 1
}

fn g() -> i32 {
    return f()
}

fn main() -> i32 {
    return 0
}
EOF
  cat > "$TMP/C1.klang" <<'EOF'
enum Result<T> { Ok(v: T), Err }
struct Store<T> { val: T }
mod math {
    pub fn double(x: i32) -> i32 {
        return x + x
    }
}
fn get(r: Result) -> i32 {
    return match r { Result::Ok(v) => v }
}
fn main() -> i32 {
    let r = Result::Ok(10)
    let s = Store { val: get(r) }
    return math::double(s.val)
}
EOF
  cat > "$TMP/C2.klang" <<'EOF'
fn bad_a() -> i32 {
    return nope_a
}

fn bad_b() -> i32 {
    return nope_b
}

fn keep(x: i32) -> i32 {
    return x + 1
}

fn main() -> i32 {
    return keep(1)
}
EOF
}
write_tasks
OUT="bench/results_live.csv"
KLANG_BIN_PATH="$BIN" TASK_DIR="$TMP" OUT_PATH="$OUT" python3 - <<'PYEOF'
import json, os, subprocess

binary = os.environ["KLANG_BIN_PATH"]
task_dir = os.environ["TASK_DIR"]
tasks = [
    ("S1-arity", "simple", "S1.klang"),
    ("S2-undefined", "simple", "S2.klang"),
    ("S3-type", "simple", "S3.klang"),
    ("M1-exhaustive", "moderate", "M1.klang"),
    ("M2-generic", "moderate", "M2.klang"),
    ("M3-effect", "moderate", "M3.klang"),
    ("C1-full-stack", "complex", "C1.klang"),
    ("C2-multi-target", "complex", "C2.klang"),
]

proc = subprocess.Popen(
    [binary, "mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
)

def rpc(method, params, rid):
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params}) + "\n")
    proc.stdin.flush()
    return json.loads(proc.stdout.readline())

rid = 1
rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "bench", "version": "0"}}, rid)
rid += 1
rows = []
for tid, tier, fname in tasks:
    with open(os.path.join(task_dir, fname)) as f:
        src = f.read()
    check = rpc("tools/call", {"name": "klang_check", "arguments": {"source": src}}, rid)
    rid += 1
    diags = json.loads(check["result"]["content"][0]["text"])["diagnostics"]
    if not diags:
        rows.append((tid, tier, "true", "", ""))
        continue
    code = diags[0]["code"]
    scope = rpc(
        "tools/call",
        {"name": "klang_scope_plan", "arguments": {"source": src, "diagnostics": diags}},
        rid,
    )
    rid += 1
    plan = json.loads(scope["result"]["content"][0]["text"])
    if plan.get("scope") == "function":
        scope_txt = "function:" + ",".join(plan.get("functions", []))
    else:
        scope_txt = "file"
    rows.append((tid, tier, "false", code, scope_txt))

proc.stdin.close()
proc.wait(timeout=60)

with open(os.environ["OUT_PATH"], "w") as f:
    f.write("task,tier,check_clean,diag_code,scope\n")
    for r in rows:
        f.write(",".join(r) + "\n")
PYEOF
rm -rf "$TMP"
echo "wrote $OUT"
cat "$OUT"
