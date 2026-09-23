#!/usr/bin/env bash
# Phase 3 live-model runner (PRD 6.1/6.2).
# Usage: bench/run_live.sh <endpoint-A> <endpoint-B>
# For each corpus task (mirrored from tests/benchmark_gates.rs) this runs:
#   1. `klang check`  (zero-shot verdict: clean vs diagnostic code)
#   2. `klang repair --model <ep> --max-iters 5` per endpoint (repair-assisted)
# and appends one CSV row per (task, endpoint) to bench/results_live.csv:
#   task,tier,endpoint,check_clean,repair_success,iters,notes
# `iters` is read from the emitted .klang-repair-log.json ("iter" count).
# Cross-model variance is flagged, not gated (PRD 6.2).
set -u
if [ $# -ne 2 ]; then
  echo "usage: bench/run_live.sh <endpoint-A> <endpoint-B>" >&2
  exit 2
fi
EPA="$1"; EPB="$2"
BIN="${KLANG_BIN:-./target/debug/klang}"
if [ ! -x "$BIN" ] && [ -n "${CARGO_TARGET_DIR:-}" ] && [ -x "$CARGO_TARGET_DIR/debug/klang" ]; then
  BIN="$CARGO_TARGET_DIR/debug/klang"
fi
OUT="bench/results_live.csv"
echo "task,tier,endpoint,check_clean,repair_success,iters,notes" > "$OUT"
run_one() { # id tier endpoint broken_file
  id="$1"; tier="$2"; ep="$3"; f="$4"
  if "$BIN" check "$f" >/dev/null 2>&1; then clean="true"; else clean="false"; fi
  rm -f "$f.repaired.klang" "$f.klang-repair-log.json" .klang-repair-log.json "$(dirname "$f")/.klang-repair-log.json" 2>/dev/null
  if "$BIN" repair "$f" --model "$ep" --max-iters 5 >/dev/null 2>&1; then ok="true"; else ok="false"; fi
  log=""
  for cand in "$f.klang-repair-log.json" "$(dirname "$f")/.klang-repair-log.json" .klang-repair-log.json; do
    [ -f "$cand" ] && log="$cand" && break
  done
  iters=""
  [ -n "$log" ] && iters="$(grep -o '"iter": [0-9]*' "$log" | wc -l | tr -d ' ')"
  echo "$id,$tier,$ep,$clean,$ok,$iters," >> "$OUT"
  rm -f "$f.repaired.klang" "$log" 2>/dev/null
}
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
for ep in "$EPA" "$EPB"; do
  run_one S1-arity simple "$ep" "$TMP/S1.klang"
  run_one S2-undefined simple "$ep" "$TMP/S2.klang"
  run_one S3-type simple "$ep" "$TMP/S3.klang"
  run_one M1-exhaustive moderate "$ep" "$TMP/M1.klang"
  run_one M2-generic moderate "$ep" "$TMP/M2.klang"
  run_one M3-effect moderate "$ep" "$TMP/M3.klang"
  run_one C1-full-stack complex "$ep" "$TMP/C1.klang"
  run_one C2-multi-target complex "$ep" "$TMP/C2.klang"
done
rm -rf "$TMP"
echo "wrote $OUT"
cat "$OUT"
