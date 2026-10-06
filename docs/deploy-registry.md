# Deploying the Klang package registry on Render (FOUNDATION-3 Part 2B + PRD §3-§5)

Target reader: the operator deploying the service (you). No Render API
credentials were requested or used to produce this setup — everything
below was verified against the service running locally, and the Render
manifest (`render.yaml`) is declarative only.

Official registry (built into the CLI as `DEFAULT_REGISTRY`):
`https://klang.raunakdevelops.dpdns.org`. A fresh install runs
`klang add collections` with zero setup. Override precedence (first match
wins): `--registry URL` → `KLANG_REGISTRY` env → `[registry] url` in the
project's `klang.toml` → `url` in `~/.klang/config.toml` → compiled-in
default. Non-HTTPS URLs are rejected unless the host is `localhost`,
`127.0.0.1` or `::1` (`registry must use https`).

## 0. What you are deploying

- One Rust web service (`klang-registry` binary in this repo).
- Storage: a PRIVATE Backblaze B2 bucket through its S3-compatible API
  (package archives + checksums + per-package index docs; the service
  itself holds no state, so no Render disk is attached and the free
  tier stays free). Local disk remains the default for dev/tests
  (`KLANG_STORAGE` unset); `KLANG_STORAGE=b2` selects B2. No database
  in v1 (documented scope decision — see
  `docs/foundation3-part2-design.md` §B).
- Security posture: single owner bearer token (anyone can read, only the
  owner can write), no TLS in the service (Render terminates TLS),
  SHA-256 integrity end to end, no overwrite of published versions
  (identical bytes republish → 200 idempotent; different bytes → 409).

## 1. Prerequisites

- This repo pushed to a Git provider Render can read.
- A Render account with permission to create a Web Service.
- A Backblaze B2 account (free tier is plenty at v1 sizes).

## 2. Create the B2 bucket + key (~5 minutes, Backblaze dashboard)

1. **Buckets → Create a Bucket**: pick a name (e.g. `klang-registry`),
   keep **Files in Bucket are: Private**. Note the bucket's region
   endpoint shown in the dashboard (host only, e.g.
   `s3.us-west-004.backblazeb2.com` — the exact host depends on the
   region; never a secret, but copy it exactly).
2. **Application Keys → Add a New Application Key**: name it (e.g.
   `klang-registry`), restrict it to **only the bucket from step 1**,
   with **read and write** access (no delete needed — the registry
   never deletes). Save the `keyID` and the `applicationKey` somewhere
   safe (password manager): the key value is shown once.
   Never commit either value to git, docs, or chat.

## 3. Create the Render service (dashboard, ~5 minutes)

1. Render dashboard → **New +** → **Web Service** → connect the repo.
2. When Render detects `render.yaml`, it proposes the `klang-registry`
   service. Confirm:
   - **Runtime:** Rust. **Build command:**
     `cargo build --release --bin klang-registry`.
     **Start command:** `./target/release/klang-registry`
     (the binary reads `$PORT` automatically).
   - **No disk** (the service is stateless; B2 holds everything —
     this is what keeps the free tier free).
   - **Health check path:** `/api/packages` (must return HTTP 200).
3. Environment → add variables (values live only in the dashboard,
   never in git — `render.yaml` declares them with `sync: false`):
   - `KLANG_STORAGE` = `b2`.
   - `KLANG_REGISTRY_TOKEN` = a fresh 48-hex-char secret, e.g. from
     `openssl rand -hex 24`.
     (`REGISTRY_ADMIN_TOKEN` is kept as a backcompat alias.)
   - `B2_KEY_ID` = the application key ID from §2.
   - `B2_APP_KEY` = the application key itself from §2.
   - `B2_BUCKET` = the private bucket name from §2.
   - `B2_ENDPOINT` = the bucket's S3 endpoint host from §2
     (host only, no `https://`).
   - `B2_REGION` = optional; leave unset unless the endpoint is
     non-standard (the region is derived from the endpoint host).
4. **Deploy.** Open `https://klang.raunakdevelops.dpdns.org/api/packages`
   in a browser (or `https://<your-service>.onrender.com/api/packages`
   for a staging service): expect `{"packages":[]}` (HTTP 200). That is
   the health proof. `GET /health` is the same liveness check without
   touching storage; `GET /health/storage` additionally probes the
   bucket and reports `{"ok":true,"storage":"b2"}` (or `ok:false`
   with HTTP 503 when B2 is unreachable — never any credentials).
   The CLI prints `registry: <host>` on publish and on
   first fetch so a wrong URL is obvious.

## 4. Publish your first package (from your laptop, owner only)

```sh
# One-time login (token stored in ~/.klang/credentials, mode 0600, dir 0700).
# The token is read with hidden input (no echo); piping works for CI:
#   echo "$TOKEN" | klang login
klang login
# logged in to klang.raunakdevelops.dpdns.org

cd my-pkg   # contains klang.toml (name, X.Y.Z version) + .klang files
klang publish
# registry: klang.raunakdevelops.dpdns.org
# published my-pkg@1.0.0 (3 files, 512 bytes, sha256:<hex>)
```

Without `klang login`, `klang publish` fails and tells you to run
`klang login`. `--token` still works but is the least safe option (shows
up in shell history). Tokens are never written to `klang.toml`,
`klang.lock`, logs, or verbose output, and are never sent over non-HTTPS
(except `localhost`). To publish to a different registry, `klang login
--registry URL` there — a token saved for registry A is never sent to
registry B. `klang logout [--registry URL]` removes it (deletes the file
when empty).

Rules enforced server-side: exact `X.Y.Z` versions, names limited to
`[A-Za-z0-9_-]`, max archive 5 MiB (`KLANG_MAX_ARCHIVE_BYTES` override),
max 500 files, no overwriting a published version (identical bytes →
200; different bytes → HTTP 409 → clean `registry conflict` error),
archives re-validated on receipt (bad magic/truncation/oversize/unsafe
names → HTTP 400; oversize → 413). The server computes SHA-256 itself
(never trusts a client hash), compares auth in constant time (SHA-256
both sides first, so the length never leaks), never logs tokens/headers/
bodies, and rate-limits failed auth per client IP (10/min → 429 with
`Retry-After`; behind Render's proxy the left-most `X-Forwarded-For`
entry is used — see §7).

## 5. Depend on it from another project (no login needed)

```sh
cd my-app   # contains klang.toml
klang add my-pkg@1.0.0
# registry: klang.raunakdevelops.dpdns.org
# added my-pkg@1.0.0
# (appends `my-pkg = "registry:my-pkg@1.0.0"` and fetches + pins it)
# `klang add my-pkg --caret` writes `registry:my-pkg@^X.Y.Z` instead.
```

Then in any `.klang` file in that project (illustrative — it resolves
against the fetched vendor dir, not standalone):

```text
import "my-pkg/lib.klang"
```

`klang run` / `klang check` / `klang build` fetch automatically when
online (`--registry` / `KLANG_REGISTRY` / project `klang.toml`
`[registry] url` / `~/.klang/config.toml` select the server;
`--offline` forbids network and uses the vendor dir or fails loudly).
Downloads never send or require a token. Fetched files land in
`.klang_pkgs/<name>/<version>/`, pinned by `klang.lock`
(`package <name> <version> <sha256>`, written atomically via
`klang.lock.tmp` + fsync + rename). `klang list` shows direct deps;
`klang list --all` shows transitive deps indented under requirers.

## 6. Integrity model (what is and isn't guaranteed)

- Guaranteed: every download is SHA-256-checked against the hash the
  server recorded at publish; the lockfile pins that hash and any drift
  (registry-side or local tamper) fails loudly instead of executing.
  Verified by `tests/registry_gates.rs` (`tampered_vendor_offline_fails`,
  `lock_pin_conflict_fails`, `stale_vendor_refreshes_online`).
- Assumed: transport to the registry is trusted (localhost, or
  Render's TLS in front of the service — the service itself speaks
  plain HTTP). Token distribution is `klang login` (or the env var for
  CI) — the token lives only in the Render env + the owner's
  `~/.klang/credentials`, never in the repo, binary, `klang.toml` or
  `klang.lock`.
- Known v1 limitations (not silently missing — see the design doc):
  first-come-first-served names, no yanking/deletion, no private
  packages, no web UI.

## 7. Operating notes

- Persistence (the service is stateless — a Render restart, including
  on the free tier, loses nothing because every byte lives in B2):
  - Bucket layout (all under the private bucket):
    `packages/<name>/<version>.klangpkg` (archive),
    `packages/<name>/<version>.sha256` (hex checksum),
    `index/<name>.json` (version list + metadata).
  - A missing or corrupt `index/<name>.json` is rebuilt from the
    `packages/` listing at startup (archives + sidecars are the source
    of truth). A missing sidecar is re-hashed from the archive.
  - Per-package version lists are cached in memory for 30 s and
    invalidated on publish (so two instances converge quickly).
  - Back up with B2's own lifecycle/snapshot rules — the bucket alone
    is the whole registry. Local dev keeps files under the data dir
    (`REGISTRY_DATA_DIR`, default `./registry-data`).
  - Free-tier fallback / reseed: `scripts/publish-stdlib.sh` publishes
    all 16 stdlib packages in dependency order, stops at the first
    failure, and treats `409` with identical bytes as success.
  - Missing B2 configuration fails fast at startup (`KLANG_STORAGE=b2`
    with any of `B2_KEY_ID` / `B2_APP_KEY` / `B2_BUCKET` /
    `B2_ENDPOINT` unset exits naming the variable, never its value).
- Logs: one line per accept error; publish events are visible to the
  publisher CLI (`published … sha256:…`). Never logs tokens,
  `Authorization` headers, or bodies. Behind Render's proxy, rate
  limiting uses the left-most `X-Forwarded-For` entry (first hop only);
  that header is spoofable unless a trusted proxy strips it — direct
  origins should not trust it.
- Token rotation (the previous token appeared in a chat transcript and
  MUST be rotated before the registry holds anything important):
  1. `openssl rand -hex 24`
  2. Set it as `KLANG_REGISTRY_TOKEN` in the Render service environment; redeploy.
  3. Run `klang login` locally with the new value.
  The previous token stops working immediately after the redeploy.
  When no token (or <32 chars) is configured, the server still serves
  reads but every write returns `503 publishing disabled: server token
  not configured`.
