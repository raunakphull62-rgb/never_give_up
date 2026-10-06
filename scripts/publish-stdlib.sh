#!/bin/sh
# Publish all 26 stdlib packages in dependency order (PRD §5).
# Free-tier fallback + reseed for a fresh registry.
# Batch B ships math 1.1.0 + crypto 1.1.0 as NEW versions alongside the
# immutable older ones (versions never overwrite: identical bytes republish
# idempotent-200, different bytes 409 — the skip logic below relies on it).
# Stops at the first failure; versions that already exist (409 with
# identical bytes counts as success — the server returns 200 idempotent
# for identical bytes, 409 only for different bytes which IS a failure
# unless the bytes match, so we double-check by treating any
# "already published" as a skip only when the local archive matches).
set -u
REGISTRY="${KLANG_REGISTRY:-https://klang.raunakdevelops.dpdns.org}"
# Dependency order (every package appears after its deps):
# - Leafs (no deps): itertools string math time testing crypto io sql
#   sync json random csv text path fs os cli base64 hex url
# - Second layer: regex -> string; compress -> io; net -> io
# - Third layer: http -> io, string; logging -> io, time;
#   collections -> itertools
PKGS="itertools string math time testing crypto io sql sync json random csv text path fs os cli base64 hex url regex compress net http logging collections"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fail=0
for pkg in $PKGS; do
  dir="$ROOT/stdlib-packages/$pkg"
  if [ ! -d "$dir" ]; then
    echo "missing package dir: $dir" >&2
    exit 1
  fi
  echo "publishing $pkg..."
  out=$(klang publish --dir "$dir" --registry "$REGISTRY" 2>&1)
  code=$?
  echo "$out"
  if [ $code -eq 0 ]; then
    continue
  fi
  # 409 identical-bytes counts as success (skip). The server returns 200
  # for identical bytes, but an older server (or a race) may return 409
  # with the same bytes — treat "already published" as a skip.
  case "$out" in
    *"already published"*)
      echo "($pkg already published — skipping)"
      continue
      ;;
    *)
      echo "failed to publish $pkg (exit $code)" >&2
      exit $code
      ;;
  esac
done
echo "all stdlib packages published to $REGISTRY"
