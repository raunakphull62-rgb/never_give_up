//! KLANG-FOUNDATION-3 Part 2B — package registry gates.
//!
//! A real but scoped v1: local HTTP service (std-only networking) +
//! `klang publish/add/fetch` + SHA-256 end-to-end integrity + lockfile
//! pins + vendor-dir loader fallback. Render deployment is docs-only
//! here (see `render.yaml` + `docs/deploy-registry.md`); everything in
//! this file runs against an ephemeral localhost server.
//!
//! BUG-A discipline: the silent failure is a swapped/tampered package
//! being USED (wrong code, no error), so tamper, lock-pin, and
//! hash-mismatch paths all assert loud failures and that the bad bytes
//! never execute.

use std::collections::HashMap;
use std::path::PathBuf;

use klang::parser::Parser;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("klang-reg-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn start_server(tag: &str) -> (String, PathBuf) {
    let data = tmp(&format!("{tag}-data"));
    let base = klang::registry::spawn_ephemeral(&data, TOKEN).expect("server starts");
    (base, data)
}

fn write_pkg(dir: &PathBuf, name: &str, version: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("klang.toml"),
        format!("name = \"{name}\"\nversion = \"{version}\"\nentry = \"main\"\n"),
    )
    .unwrap();
    for (name, content) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn run_cli(dir: &PathBuf, args: &[&str], token: bool) -> (String, String, i32) {
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(args).current_dir(dir);
    if token {
        cmd.env("KLANG_REGISTRY_TOKEN", TOKEN);
    }
    let output = cmd.output().expect("klang binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn sha256_vectors() {
    // Pins the hash wiring to NIST vectors (a wrong hash silently
    // breaks every integrity check below).
    assert_eq!(
        klang::registry::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        klang::registry::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        klang::registry::sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn archive_round_trip() {
    // Nested layout + manifest survive pack/unpack byte-identically.
    let files = vec![
        ("klang.toml".to_string(), b"name=\"x\"".to_vec()),
        ("lib.klang".to_string(), b"fn f() -> i32 { return 1 }".to_vec()),
        ("sub/h.klang".to_string(), b"fn g() -> i32 { return 2 }".to_vec()),
    ];
    let bytes = klang::registry::pack_archive(&files).expect("packs");
    assert!(bytes.starts_with(b"KLANGPKG1"));
    let back = klang::registry::unpack_archive(&bytes).expect("unpacks");
    let mut want = files.clone();
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(back, want);
}

#[test]
fn archive_rejects_unsafe_names() {
    // Traversal, absolute, backslash, empty, and duplicate names all
    // fail loudly on pack; a forged archive fails on unpack.
    for bad in ["../evil.klang", "/abs.klang", "a\\b.klang", "", ".hidden.klang"] {
        let files = vec![(bad.to_string(), b"x".to_vec())];
        assert!(
            klang::registry::pack_archive(&files).is_err(),
            "packs unsafe `{bad}`"
        );
    }
    let dup = vec![
        ("a.klang".to_string(), b"1".to_vec()),
        ("a.klang".to_string(), b"2".to_vec()),
    ];
    assert!(klang::registry::pack_archive(&dup).is_err());
    assert!(klang::registry::unpack_archive(b"NOPE").is_err());
    let mut good = klang::registry::pack_archive(&[("a.klang".to_string(), b"1".to_vec())]).unwrap();
    good.push(0x00);
    assert!(klang::registry::unpack_archive(&good).is_err(), "trailing garbage");
}

#[test]
fn archive_caps_enforced() {
    // 513 files and a 1 MiB+1 file both fail fast (zip-bomb defense).
    let many: Vec<(String, Vec<u8>)> = (0..513).map(|i| (format!("f{i}.klang"), b"x".to_vec())).collect();
    assert!(klang::registry::pack_archive(&many).is_err());
    let big = vec![("big.klang".to_string(), vec![b'x'; 1024 * 1024 + 1])];
    assert!(klang::registry::pack_archive(&big).is_err());
}

#[test]
fn publish_then_metadata() {
    // Publish two versions; the index lists both with hashes, latest first.
    let (base, _) = start_server("meta");
    let dir = tmp("meta-pkg");
    write_pkg(&dir, "calc", "1.0.0", &[("lib.klang", "fn t(n: i32) -> i32 { return n }")]);
    let files = klang::registry::collect_package_files(&dir).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("publishes");
    let meta = klang::registry::fetch_metadata(&base, "calc").expect("metadata");
    assert_eq!(meta.latest, "1.0.0");
    assert_eq!(meta.versions.len(), 1);
    assert_eq!(meta.versions[0].1, klang::registry::sha256_hex(&archive));
    write_pkg(&dir, "calc", "1.1.0", &[("lib.klang", "fn t(n: i32) -> i32 { return n + 1 }")]);
    let files = klang::registry::collect_package_files(&dir).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.1.0", &archive).expect("publishes");
    let meta = klang::registry::fetch_metadata(&base, "calc").expect("metadata");
    assert_eq!(meta.latest, "1.1.0");
    assert_eq!(meta.versions.len(), 2);
}

#[test]
fn republish_conflict() {
    // S6: same bytes -> 200 idempotent; different bytes -> 409, original untouched.
    let (base, _) = start_server("conflict");
    let dir = tmp("conflict-pkg");
    write_pkg(&dir, "calc", "1.0.0", &[("lib.klang", "fn t(n: i32) -> i32 { return n }")]);
    let files = klang::registry::collect_package_files(&dir).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("publishes");
    // Identical bytes republish is idempotent (200).
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("idempotent");
    // Different bytes at the same version conflict.
    write_pkg(&dir, "calc", "1.0.0", &[("lib.klang", "fn t(n: i32) -> i32 { return n + 1 }")]);
    let files2 = klang::registry::collect_package_files(&dir).expect("collects");
    let archive2 = klang::registry::pack_archive(&files2).expect("packs");
    let err = klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive2).expect_err("conflicts");
    assert_eq!(err.kind, "conflict", "{err}");
    let meta = klang::registry::fetch_metadata(&base, "calc").expect("metadata");
    assert_eq!(meta.versions.len(), 1, "no overwrite, no duplicate");
}

#[test]
fn bad_token_rejected() {
    let (base, _) = start_server("auth");
    let archive =
        klang::registry::pack_archive(&[("a.klang".to_string(), b"fn f() -> i32 { return 1 }".to_vec())])
            .expect("packs");
    let err = klang::registry::publish_pkg(&base, "wrong", "calc", "1.0.0", &archive)
        .expect_err("rejects");
    assert_eq!(err.kind, "auth", "{err}");
    assert!(klang::registry::fetch_metadata(&base, "calc").is_err(), "nothing stored");
}

#[test]
fn unknown_package_clean() {
    // Unknown names/versions are typed errors, never panics or HTML soup.
    let (base, _) = start_server("unknown");
    let err = klang::registry::fetch_metadata(&base, "ghost").expect_err("missing");
    assert_eq!(err.kind, "not-found", "{err}");
    let err = klang::registry::download(&base, "ghost", "1.0.0").expect_err("missing");
    assert_eq!(err.kind, "not-found", "{err}");
}

#[test]
fn dep_string_parsing() {
    assert_eq!(
        klang::registry::parse_registry_dep("registry:calc@1.0.0"),
        Some(("calc".to_string(), "1.0.0".to_string()))
    );
    assert_eq!(klang::registry::parse_registry_dep("./mylib.klang"), None);
    assert_eq!(klang::registry::parse_registry_dep("registry:calc"), None);
    assert_eq!(klang::registry::parse_registry_dep("registry:../x@1.0.0"), None);
    assert_eq!(klang::registry::parse_registry_dep("registry:calc@1.0"), None);
    assert_eq!(klang::registry::parse_registry_dep("registry:calc@1.0.0 "), None);
}

#[test]
fn e2e_publish_fetch_run() {
    // THE end-to-end: publish a real package, depend on it from a
    // separate project, resolve through the loader, run it.
    let (base, _) = start_server("e2e");
    let pkg = tmp("e2e-pkg");
    write_pkg(
        &pkg,
        "calc",
        "1.0.0",
        &[("lib.klang", "fn triple(n: i32) -> i32 { return n * 3 }")],
    );
    let files = klang::registry::collect_package_files(&pkg).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    let sha = klang::registry::sha256_hex(&archive);
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("publishes");

    let app = tmp("e2e-app");
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncalc = \"registry:calc@1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        app.join("app.klang"),
        "import \"calc/lib.klang\"\nfn main() -> i32 { return triple(14) }\n",
    )
    .unwrap();
    let logs = klang::registry::fetch_project(&app, &base, false).expect("fetches");
    assert!(logs.iter().any(|l| l.contains("calc@1.0.0")), "{logs:?}");
    let lock = std::fs::read_to_string(app.join("klang.lock")).expect("lock written");
    assert!(
        lock.contains(&format!("package calc 1.0.0 {sha}")),
        "lock pins the hash:\n{lock}"
    );
    let entry = app.join("app.klang").to_string_lossy().to_string();
    let loaded = klang::imports::load_program(&entry).expect("loads through vendor dir");
    assert!(klang::hir::TypedHIR::check(loaded.program.clone()).is_ok());
    let mir = klang::mir::lower(&loaded.program);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn cli_publish_add_fetch_run() {
    // CLI surface end-to-end: publish, add into a fresh project, run it.
    let (base, _) = start_server("cli");
    let pkg = tmp("cli-pkg");
    write_pkg(
        &pkg,
        "calc",
        "2.0.0",
        &[("lib.klang", "fn triple(n: i32) -> i32 { return n * 3 }")],
    );
    let (out, err, code) = run_cli(
        &pkg,
        &["publish", "--registry", &base],
        true,
    );
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("published calc@2.0.0"), "{out}");

    let app = tmp("cli-app");
    std::fs::write(app.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n")
        .unwrap();
    std::fs::write(
        app.join("app.klang"),
        "import \"calc/lib.klang\"\nfn main() -> i32 { return triple(14) }\n",
    )
    .unwrap();
    let (out, err, code) = run_cli(&app, &["add", "calc@2.0.0", "--registry", &base], false);
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("added calc@2.0.0"), "{out}");
    let manifest = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    assert!(manifest.contains("calc = \"registry:calc@2.0.0\""), "{manifest}");

    let (out, err, code) = run_cli(&app, &["run", "app.klang", "--registry", &base], false);
    assert_eq!(code, 42, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.is_empty(), "quiet run prints nothing, got:\n{out}");
}

#[test]
fn tampered_vendor_offline_fails() {
    // BUG-A class: corrupted vendor bytes must fail LOUDLY offline (no
    // re-fetch possible) — never execute, never silently pass.
    let (base, _) = start_server("tamper");
    let pkg = tmp("tamper-pkg");
    write_pkg(&pkg, "calc", "1.0.0", &[("lib.klang", "fn triple(n: i32) -> i32 { return n * 3 }")]);
    let files = klang::registry::collect_package_files(&pkg).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("publishes");
    let app = tmp("tamper-app");
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncalc = \"registry:calc@1.0.0\"\n",
    )
    .unwrap();
    klang::registry::fetch_project(&app, &base, false).expect("fetches");
    std::fs::write(app.join(".klang_pkgs/calc/1.0.0/lib.klang"), "fn triple(n: i32) -> i32 { return 999 }\n")
        .unwrap();
    let err = klang::registry::fetch_project(&app, &base, true).expect_err("must fail offline");
    assert_eq!(err.kind, "integrity", "{err}");
}

#[test]
fn lock_pin_conflict_fails() {
    // A registry entry that moved under a lock pin is a hard error —
    // never a silent re-pin. (Simulated by hand-editing the lock, which
    // is exactly what an attacker-swapped registry would look like.)
    let (base, _) = start_server("pin");
    let pkg = tmp("pin-pkg");
    write_pkg(&pkg, "calc", "1.0.0", &[("lib.klang", "fn triple(n: i32) -> i32 { return n * 3 }")]);
    let files = klang::registry::collect_package_files(&pkg).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("publishes");
    let app = tmp("pin-app");
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncalc = \"registry:calc@1.0.0\"\n",
    )
    .unwrap();
    klang::registry::fetch_project(&app, &base, false).expect("fetches");
    std::fs::write(
        app.join("klang.lock"),
        "package calc 1.0.0 0000000000000000000000000000000000000000000000000000000000000000\n",
    )
    .unwrap();
    let err = klang::registry::fetch_project(&app, &base, false).expect_err("must fail");
    assert_eq!(err.kind, "integrity", "{err}");
    assert!(err.message.contains("lockfile"), "{err}");
}

#[test]
fn stale_vendor_refreshes_online() {
    // Online, a corrupted vendor dir is a cache miss: re-download the
    // pinned bytes and run correctly (proves the lock — not the cache —
    // is the trust anchor).
    let (base, _) = start_server("refresh");
    let pkg = tmp("refresh-pkg");
    write_pkg(&pkg, "calc", "1.0.0", &[("lib.klang", "fn triple(n: i32) -> i32 { return n * 3 }")]);
    let files = klang::registry::collect_package_files(&pkg).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("publishes");
    let app = tmp("refresh-app");
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncalc = \"registry:calc@1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        app.join("app.klang"),
        "import \"calc/lib.klang\"\nfn main() -> i32 { return triple(14) }\n",
    )
    .unwrap();
    klang::registry::fetch_project(&app, &base, false).expect("fetches");
    std::fs::write(app.join(".klang_pkgs/calc/1.0.0/lib.klang"), "fn triple(n: i32) -> i32 { return 999 }\n")
        .unwrap();
    let logs = klang::registry::fetch_project(&app, &base, false).expect("refreshes");
    assert!(logs.iter().any(|l| l.contains("fetched")), "{logs:?}");
    let entry = app.join("app.klang").to_string_lossy().to_string();
    let loaded = klang::imports::load_program(&entry).expect("loads");
    let mir = klang::mir::lower(&loaded.program);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42, "refreshed bytes execute, not the tampered 999*14");
}

#[test]
fn offline_missing_vendor_fails() {
    // Offline with nothing vendored: loud error telling the user to
    // fetch online first (not a hang, not a silent zero).
    let (base, _) = start_server("offmiss");
    let app = tmp("offmiss-app");
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncalc = \"registry:calc@1.0.0\"\n",
    )
    .unwrap();
    let _ = &base;
    let err = klang::registry::fetch_project(&app, &base, true).expect_err("must fail");
    assert_eq!(err.kind, "offline", "{err}");
}

#[test]
fn invalid_names_rejected() {
    // Client-side validation fires before any network use.
    let (base, _) = start_server("badname");
    let archive =
        klang::registry::pack_archive(&[("a.klang".to_string(), b"fn f() -> i32 { return 1 }".to_vec())])
            .expect("packs");
    for (name, version) in [("../x", "1.0.0"), ("calc", "1.0"), ("", "1.0.0"), ("calc", "")] {
        let err = klang::registry::publish_pkg(&base, TOKEN, name, version, &archive)
            .expect_err("rejects");
        assert_eq!(err.kind, "protocol", "{name}@{version}: {err}");
    }
}
