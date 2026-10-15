#!/bin/sh
# A3b Postgres gate runner (Wave 2 Stage A3b).
# Runs a3b_gates twice: without KLANG_TEST_PG_URL (live tests must skip
# cleanly) and with it (live tests must run). In this persistent-Ubuntu
# env a local PostgreSQL is already running; pass the URL explicitly.
# Usage: KLANG_TEST_PG_URL=postgres://klang:klangtest@localhost/klang_test sh scripts/a3b_pg_test.sh
set -e
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT"
. "$HOME/.cargo/env"
echo "== a3b_gates without PG URL (must skip cleanly) =="
env -u KLANG_TEST_PG_URL cargo test --locked --test a3b_gates -- --nocapture
echo "== a3b_gates with PG URL (must run live) =="
if [ -z "${KLANG_TEST_PG_URL:-}" ]; then
  echo "KLANG_TEST_PG_URL not set; using local default for live run"
  export KLANG_TEST_PG_URL="postgres://klang:klangtest@localhost/klang_test"
fi
cargo test --locked --test a3b_gates -- --nocapture
echo "a3b_pg_test.sh OK"
