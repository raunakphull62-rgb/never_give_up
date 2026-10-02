# Deploying the Klang package registry on Render (FOUNDATION-3 Part 2B)

Target reader: the operator deploying the service (you). No Render API
credentials were requested or used to produce this setup — everything
below was verified against the service running locally, and the Render
manifest (`render.yaml`) is declarative only.

## 0. What you are deploying

- One Rust web service (`klang-registry` binary in this repo).
- Storage: local files under the data dir (`index.json` + one
  `<name>/<version>.kpkg` archive per package), on a 1 GB Render
  persistent disk. No database in v1 (documented scope decision: files
  are the simplest correct store at v1 sizes; Postgres is the migration
  path when the index outgrows them — see
  `docs/foundation3-part2-design.md` §B).
- Security posture: single admin bearer token, no TLS in the service
  (Render terminates TLS), SHA-256 integrity end to end, no overwrite
  of published versions.

## 1. Prerequisites

- This repo pushed to a Git provider Render can read.
- A Render account with permission to create a Web Service + disk.

## 2. Create the service (dashboard, ~5 minutes)

1. Render dashboard → **New +** → **Web Service** → connect the repo.
2. When Render detects `render.yaml`, it proposes the `klang-registry`
   service. Confirm:
   - **Runtime:** Rust. **Build command:**
     `cargo build --release --bin klang-registry`.
     **Start command:** `./target/release/klang-registry --data-dir /data/registry`
     (the binary reads `$PORT` automatically).
   - **Disk:** name `registry-data`, mount `/data`, 1 GB.
   - **Health check path:** `/api/packages` (must return HTTP 200).
3. Environment → add variable:
   - `REGISTRY_ADMIN_TOKEN` = a fresh 64-hex-char secret, e.g. from
     `python3 -c "import secrets; print(secrets.token_hex(32))"`.
     Keep "Sync" OFF (the value lives only in the dashboard, never in
     git — `render.yaml` declares it with `sync: false`).
4. **Deploy.** Open `https://<your-service>.onrender.com/api/packages`
   in a browser: expect `{"packages":[]}` (HTTP 200). That is the
   health proof.

## 3. Publish your first package (from your laptop)

```sh
export KLANG_REGISTRY=https://<your-service>.onrender.com
export KLANG_REGISTRY_TOKEN=<the token from step 2.3>
cd my-pkg   # contains klang.toml (name, X.Y.Z version) + .klang files
klang publish
# published my-pkg@1.0.0 (3 files, 512 bytes, sha256:<hex>)
```

Rules enforced server-side: exact `X.Y.Z` versions, names limited to
`[A-Za-z0-9_-]`, no overwriting a published version (republish →
HTTP 409 → clean `registry conflict` error), archives re-validated on
receipt (bad magic/truncation/oversize/unsafe names → HTTP 400).

## 4. Depend on it from another project

```sh
cd my-app   # contains klang.toml
klang add my-pkg@1.0.0
# added my-pkg@1.0.0
# (appends `my-pkg = "registry:my-pkg@1.0.0"` and fetches + pins it)
```

Then in any `.klang` file in that project (illustrative — it resolves
against the fetched vendor dir, not standalone):

```text
import "my-pkg/lib.klang"
```

`klang run` / `klang check` / `klang build` fetch automatically when
online (`--registry` / `KLANG_REGISTRY` select the server;
`--offline` forbids network and uses the vendor dir or fails loudly).
Fetched files land in `.klang_pkgs/<name>/<version>/`, pinned by
`klang.lock` (`package <name> <version> <sha256>`).

## 5. Integrity model (what is and isn't guaranteed)

- Guaranteed: every download is SHA-256-checked against the hash the
  server recorded at publish; the lockfile pins that hash and any drift
  (registry-side or local tamper) fails loudly instead of executing.
  Verified by `tests/registry_gates.rs` (`tampered_vendor_offline_fails`,
  `lock_pin_conflict_fails`, `stale_vendor_refreshes_online`).
- Assumed: transport to the registry is trusted (localhost, or
  Render's TLS in front of the service — the service itself speaks
  plain HTTP). Token distribution is out of band (you hand it to
  publishers yourself in v1).
- Known v1 limitations (not silently missing — see the design doc):
  first-come-first-served names, no yanking/deletion, no private
  packages, no version ranges, no transitive registry deps, no web UI.

## 6. Operating notes

- Back up `/data/registry` (disk snapshots) — it holds `index.json`
  plus every archive; either alone is useless without the other, and
  startup drops index entries whose archives are missing (fail closed,
  logged on stderr).
- Logs: one line per accept error; publish events are visible to the
  publisher CLI (`published … sha256:…`). No request logging in v1.
- To rotate the token: change `REGISTRY_ADMIN_TOKEN` in the dashboard
  and redeploy; old tokens stop working immediately (single token).
