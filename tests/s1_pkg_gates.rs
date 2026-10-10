//! S1a gates: package imports by bare name (`import "math"`).
//!
//! Install-less fixtures: hand-made `.klang_pkgs` + `klang.lock`, no network.
//! Each test uses its own unique temp dir (pid + atomic counter).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static CTR: AtomicU64 = AtomicU64::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "klang-s1-pkg-{}-{}-{}",
        tag,
        std::process::id(),
        c
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(dir: &PathBuf, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, content).unwrap();
}

fn fake_sha() -> String {
    "a".repeat(64)
}

fn load_ok(dir: &PathBuf, entry: &str) -> klang::ast::Program {
    let path = dir.join(entry).to_string_lossy().to_string();
    klang::imports::load_program(&path)
        .unwrap_or_else(|e| panic!("load failed for {entry}: {}", e.to_json()))
        .program
}

fn load_err(dir: &PathBuf, entry: &str) -> klang::imports::ImportError {
    let path = dir.join(entry).to_string_lossy().to_string();
    klang::imports::load_program(&path).expect_err("load must fail")
}

fn check_and_run(prog: &klang::ast::Program) -> (i32, Vec<String>) {
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs")
}

#[test]
fn s1_plain_bare_import_runs() {
    let dir = tmpdir("plain");
    let sha = fake_sha();
    write_file(
        &dir,
        "klang.toml",
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nmath = \"registry:math@1.0.0\"\n",
    );
    write_file(&dir, "klang.lock", &format!("package math 1.0.0 {sha}\n"));
    write_file(&dir, ".klang_pkgs/math/1.0.0/lib.klang", "import \"src/ops.klang\"\n");
    write_file(
        &dir,
        ".klang_pkgs/math/1.0.0/src/ops.klang",
        "fn math_double(n: i32) -> i32 { return n * 2 }\n",
    );
    write_file(
        &dir,
        "main.klang",
        "import \"math\"\nfn main() -> i32 { return math_double(21) }\n",
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
}

#[test]
fn s1_alias_bare_import_runs() {
    let dir = tmpdir("alias");
    let sha = fake_sha();
    write_file(
        &dir,
        "klang.toml",
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nmath = \"registry:math@1.0.0\"\n",
    );
    write_file(&dir, "klang.lock", &format!("package math 1.0.0 {sha}\n"));
    write_file(&dir, ".klang_pkgs/math/1.0.0/lib.klang", "import \"src/ops.klang\"\n");
    write_file(
        &dir,
        ".klang_pkgs/math/1.0.0/src/ops.klang",
        "fn math_double(n: i32) -> i32 { return n * 2 }\n",
    );
    write_file(
        &dir,
        "main.klang",
        "import \"math\" as m\nfn main() -> i32 { return m.math_double(21) }\n",
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
}

#[test]
fn s1_transitive_bare_dep_resolves() {
    // User lists only collections; collections itself imports itertools
    // (bare) though the user never listed itertools. The lock pins both.
    let dir = tmpdir("transitive");
    let sha = fake_sha();
    write_file(
        &dir,
        "klang.toml",
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncollections = \"registry:collections@1.0.0\"\n",
    );
    write_file(
        &dir,
        "klang.lock",
        &format!("package collections 1.0.0 {sha}\npackage itertools 1.0.0 {sha}\n"),
    );
    write_file(
        &dir,
        ".klang_pkgs/itertools/1.0.0/lib.klang",
        "import \"src/iter.klang\"\n",
    );
    write_file(
        &dir,
        ".klang_pkgs/itertools/1.0.0/src/iter.klang",
        "fn iter_double(n: i32) -> i32 { return n * 2 }\n",
    );
    write_file(
        &dir,
        ".klang_pkgs/collections/1.0.0/lib.klang",
        "import \"src/colls.klang\"\n",
    );
    // Bare transitive import inside the vendor package.
    write_file(
        &dir,
        ".klang_pkgs/collections/1.0.0/src/colls.klang",
        "import \"itertools\"\nfn coll_four() -> i32 { return iter_double(21) }\n",
    );
    write_file(
        &dir,
        "main.klang",
        "import \"collections\"\nfn main() -> i32 { return coll_four() }\n",
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
}

#[test]
fn s1_local_file_wins_over_package() {
    let dir = tmpdir("localwins");
    let sha = fake_sha();
    write_file(
        &dir,
        "klang.toml",
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nmath = \"registry:math@1.0.0\"\n",
    );
    write_file(&dir, "klang.lock", &format!("package math 1.0.0 {sha}\n"));
    // Vendored package would return 999.
    write_file(&dir, ".klang_pkgs/math/1.0.0/lib.klang", "fn math_val() -> i32 { return 999 }\n");
    // Local file shadows it and returns 7.
    write_file(&dir, "math.klang", "fn math_val() -> i32 { return 7 }\n");
    write_file(
        &dir,
        "main.klang",
        "import \"math\"\nfn main() -> i32 { return math_val() }\n",
    );
    // Both exist, so the loader must print the one-line note (to stderr)
    // and use the local file.
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 7, "local file must win over vendored package");
    assert!(
        dir.join(".klang_pkgs/math/1.0.0/lib.klang").is_file(),
        "vendor entry must exist to prove shadowing"
    );
}

#[test]
fn s1_unknown_name_error_names_both_places() {
    let dir = tmpdir("unknown");
    write_file(
        &dir,
        "klang.toml",
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\n",
    );
    write_file(&dir, "klang.lock", "");
    write_file(
        &dir,
        "main.klang",
        "import \"ghostpkg\"\nfn main() -> i32 { return 0 }\n",
    );
    let e = load_err(&dir, "main.klang");
    assert_eq!(e.code, "E-IO-NOT-FOUND", "{}", e.to_json());
    let msg = e.to_json();
    assert!(msg.contains("ghostpkg"), "names the package: {msg}");
    assert!(
        msg.contains(".klang_pkgs"),
        "names the vendor place searched: {msg}"
    );
    assert!(
        msg.contains("klang add ghostpkg"),
        "suggests the fix: {msg}"
    );
}

#[test]
fn s1_missing_lock_entry_errors() {
    let dir = tmpdir("missinglock");
    // Manifest lists math but the lock has no pin for it.
    write_file(
        &dir,
        "klang.toml",
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nmath = \"registry:math@1.0.0\"\n",
    );
    write_file(&dir, "klang.lock", "");
    write_file(&dir, ".klang_pkgs/math/1.0.0/lib.klang", "fn math_val() -> i32 { return 1 }\n");
    write_file(
        &dir,
        "main.klang",
        "import \"math\"\nfn main() -> i32 { return math_val() }\n",
    );
    let e = load_err(&dir, "main.klang");
    assert_eq!(e.code, "E-IO-NOT-FOUND", "{}", e.to_json());
    let msg = e.to_json();
    assert!(msg.contains("math"), "names the package: {msg}");
    assert!(
        msg.contains("klang.lock"),
        "mentions the missing lock entry: {msg}"
    );
}
