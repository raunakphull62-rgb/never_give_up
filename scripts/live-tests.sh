#!/bin/sh
# Optional live-internet checks for the http package.
# Never run by `cargo test` (which must stay offline + deterministic).
# Usage: KLANG_LIVE_TESTS=1 sh scripts/live-tests.sh [--registry URL]
# No tokens or credentials anywhere in this script.
set -u
if [ "${KLANG_LIVE_TESTS:-0}" != "1" ]; then
  echo "skip: set KLANG_LIVE_TESTS=1 to run live-internet checks"
  exit 0
fi
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KLANG_BIN="${KLANG_BIN:-$ROOT/target/debug/klang}"
if [ ! -x "$KLANG_BIN" ]; then
  echo "building klang..."
  (cd "$ROOT" && cargo build) || exit $?
  KLANG_BIN="$ROOT/target/debug/klang"
fi
echo "live: stdlib-packages/http/http_live_test.klang"
(cd "$ROOT/stdlib-packages/http" && "$KLANG_BIN" run http_live_test.klang) || exit $?
echo "live checks passed"
