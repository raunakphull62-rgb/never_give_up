//! PRD KLANG_REGISTRY_FIXES gates T1-T10.
//! Keeps every existing test passing; covers B1-B7, §3, §4.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const LONG_TOKEN: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn home_lock() -> std::sync::MutexGuard<'static, ()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "klang-fix-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

// ---------------------------------------------------------------------------
// T1: B1 nested imports
// ---------------------------------------------------------------------------

#[test]
fn t1_nested_import_in_vendored_pkg_runs() {
    // Vendored package whose lib imports src/x; entry imports the package.
    let app = tmp("t1a");
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncollections = \"registry:collections@1.0.0\"\n",
    )
    .unwrap();
    // Fake a valid 64-hex pin (integrity is not under test here; the
    // loader only needs the lock for version lookup).
    let fake_sha = "a".repeat(64);
    std::fs::write(
        app.join("klang.lock"),
        format!("package collections 1.0.0 {fake_sha}\n"),
    )
    .unwrap();
    let pkgdir = app.join(".klang_pkgs/collections/1.0.0");
    std::fs::create_dir_all(pkgdir.join("src")).unwrap();
    std::fs::write(pkgdir.join("lib.klang"), "import \"src/colls.klang\"\n").unwrap();
    std::fs::write(
        pkgdir.join("src/colls.klang"),
        "fn coll_value() -> i32 { return 42 }\n",
    )
    .unwrap();
    std::fs::write(
        app.join("main.klang"),
        "import \"collections/lib.klang\"\nfn main() -> i32 { return coll_value() }\n",
    )
    .unwrap();
    let entry = app.join("main.klang").to_string_lossy().to_string();
    let loaded = klang::imports::load_program(&entry).expect("B1 nested import loads");
    assert!(klang::hir::TypedHIR::check(loaded.program.clone()).is_ok());
    let mir = klang::mir::lower(&loaded.program);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn t1_package_to_package_import_runs() {
    // Package A imports package B (collections -> itertools shape).
    let app = tmp("t1b");
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\ncola = \"registry:cola@1.0.0\"\ncolb = \"registry:colb@1.0.0\"\n",
    )
    .unwrap();
    let fake = "b".repeat(64);
    std::fs::write(
        app.join("klang.lock"),
        format!("package cola 1.0.0 {fake}\npackage colb 1.0.0 {fake}\n"),
    )
    .unwrap();
    let adir = app.join(".klang_pkgs/cola/1.0.0");
    let bdir = app.join(".klang_pkgs/colb/1.0.0");
    std::fs::create_dir_all(&adir).unwrap();
    std::fs::create_dir_all(bdir.join("src")).unwrap();
    std::fs::write(bdir.join("lib.klang"), "import \"src/base.klang\"\n").unwrap();
    std::fs::write(bdir.join("src/base.klang"), "fn base_val() -> i32 { return 21 }\n").unwrap();
    std::fs::write(
        adir.join("lib.klang"),
        "import \"colb/lib.klang\"\nfn a_val() -> i32 { return base_val() * 2 }\n",
    )
    .unwrap();
    std::fs::write(
        app.join("main.klang"),
        "import \"cola/lib.klang\"\nfn main() -> i32 { return a_val() }\n",
    )
    .unwrap();
    let entry = app.join("main.klang").to_string_lossy().to_string();
    let loaded = klang::imports::load_program(&entry).expect("pkg->pkg loads");
    let mir = klang::mir::lower(&loaded.program);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

// ---------------------------------------------------------------------------
// T2: B2 empty/bad checksum
// ---------------------------------------------------------------------------

#[test]
fn t2_from_resolution_missing_hash_errs() {
    let graph = klang::package::resolver::ResolutionGraph {
        packages: vec![klang::package::resolver::GraphNode {
            name: "a".to_string(),
            version: "1.0.0".to_string(),
            deps: vec![],
        }],
        root_deps: vec!["a".to_string()],
        install_order: vec![],
    };
    let empty: HashMap<(String, String), String> = HashMap::new();
    let err = klang::package_manager::lock_file::LockFile::from_resolution(&graph, &empty)
        .expect_err("missing hash must err");
    assert!(
        err.message.contains("checksum") || err.to_string().contains("checksum"),
        "{err}"
    );
    // Empty string hash also errs.
    let mut bad = HashMap::new();
    bad.insert(("a".to_string(), "1.0.0".to_string()), "".to_string());
    assert!(klang::package_manager::lock_file::LockFile::from_resolution(&graph, &bad).is_err());
    // Non-64-hex errs.
    let mut bad2 = HashMap::new();
    bad2.insert(("a".to_string(), "1.0.0".to_string()), "xyz".to_string());
    assert!(klang::package_manager::lock_file::LockFile::from_resolution(&graph, &bad2).is_err());
}

#[test]
fn t2_lock_bad_checksum_read_errs() {
    assert!(klang::registry::parse_package_locks_strict(
        "package a 1.0.0 0000000000000000000000000000000000000000000000000000000000000000\n"
    )
    .is_ok());
    assert!(klang::registry::parse_package_locks_strict("package a 1.0.0 \n").is_err());
    assert!(klang::registry::parse_package_locks_strict("package a 1.0.0 xyz\n").is_err());
    assert!(klang::registry::parse_package_locks_strict(
        "package a 1.0.0 00000000000000000000000000000000000000000000000000000000000000\n"
    )
    .is_err());
    // LockFile::read path.
    let dir = tmp("t2read");
    let p = dir.join("klang.lock");
    std::fs::write(&p, "package a 1.0.0 nothex\n").unwrap();
    assert!(klang::package_manager::lock_file::LockFile::read(&p).is_err());
}

// ---------------------------------------------------------------------------
// T3: B3 topo must err on cycle
// ---------------------------------------------------------------------------

#[test]
fn t3_topo_cycle_errs() {
    use klang::package::resolver::{GraphNode, ResolutionGraph};
    // Direct topo cycle.
    let nodes = vec![
        GraphNode {
            name: "a".to_string(),
            version: "1.0.0".to_string(),
            deps: vec![("b".to_string(), "1.0.0".to_string())],
        },
        GraphNode {
            name: "b".to_string(),
            version: "1.0.0".to_string(),
            deps: vec![("a".to_string(), "1.0.0".to_string())],
        },
    ];
    assert!(klang::package::resolver::topological_sort(&nodes).is_err());
    // Wrapper (B3) also errs instead of alphabetical fallback.
    struct Stub;
    impl klang::package::resolver::MetaSource for Stub {
        fn versions(&self, _n: &str) -> Option<Vec<(String, String)>> {
            None
        }
        fn deps_for(&self, _n: &str, _v: &str) -> Vec<(String, String)> {
            vec![]
        }
    }
    let stub = Stub;
    let r = klang::package_manager::resolver::DependencyResolver::new(&stub);
    assert!(r.topological_sort(&nodes).is_err());
    // Inconsistent graph (missing dep target is ignored, but a cycle
    // through present nodes still errs).
    let _ = ResolutionGraph {
        packages: nodes,
        root_deps: vec![],
        install_order: vec![],
    };
}

// ---------------------------------------------------------------------------
// T4: B4 atomic write
// ---------------------------------------------------------------------------

#[test]
fn t4_atomic_no_tmp_and_leftover_safe() {
    let dir = tmp("t4");
    let path = dir.join("klang.lock");
    let lock = klang::package_manager::lock_file::LockFile {
        packages: vec![klang::package_manager::lock_file::PackageEntry::new(
            "a",
            "1.0.0",
            &"c".repeat(64),
        )],
    };
    lock.write(&path).expect("writes");
    assert!(!dir.join("klang.lock.tmp").exists(), "no .tmp remains");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("package a 1.0.0"), "{text}");
    // Simulate an interrupted write: leftover .tmp must not corrupt the lock.
    std::fs::write(dir.join("klang.lock.tmp"), "garbage\n").unwrap();
    let lock2 = klang::package_manager::lock_file::LockFile {
        packages: vec![klang::package_manager::lock_file::PackageEntry::new(
            "b",
            "2.0.0",
            &"d".repeat(64),
        )],
    };
    lock2.write(&path).expect("rewrites over leftover tmp");
    assert!(!dir.join("klang.lock.tmp").exists());
    let text2 = std::fs::read_to_string(&path).unwrap();
    assert!(text2.contains("package b 2.0.0"), "{text2}");
}

// ---------------------------------------------------------------------------
// T5: B5 flags
// ---------------------------------------------------------------------------

fn run_cli_raw(dir: &PathBuf, args: &[&str], extra_env: &[(&str, &str)]) -> (String, String, i32) {
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(args).current_dir(dir);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    // Ensure no ambient registry env leaks into the test.
    cmd.env_remove("KLANG_REGISTRY");
    cmd.env_remove("KLANG_REGISTRY_TOKEN");
    let out = cmd.output().expect("klang runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn t5_registry_flag_forms_and_unknown() {
    let (base, _) = {
        let data = tmp("t5data");
        let b = klang::registry::spawn_ephemeral(&data, LONG_TOKEN).expect("server");
        (b, data)
    };
    // Seed one package so install has something to fetch.
    let pkg = tmp("t5pkg");
    std::fs::write(pkg.join("klang.toml"), "name = \"t5p\"\nversion = \"1.0.0\"\nentry = \"main\"\n").unwrap();
    std::fs::write(pkg.join("a.klang"), "fn f() -> i32 { return 1 }\n").unwrap();
    let files = klang::registry::collect_package_files(&pkg).unwrap();
    let archive = klang::registry::pack_archive(&files).unwrap();
    klang::registry::publish_pkg(&base, LONG_TOKEN, "t5p", "1.0.0", &archive).unwrap();

    let app = tmp("t5app");
    std::fs::write(app.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n").unwrap();

    // `--registry=URL` form works.
    let (out, err, code) = run_cli_raw(&app, &["install", "t5p@1.0.0", &format!("--registry={base}")], &[]);
    assert_eq!(code, 0, "stdout:{out}\nstderr:{err}");

    // `--registry URL` form works.
    let app2 = tmp("t5app2");
    std::fs::write(app2.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n").unwrap();
    let (out, err, code) = run_cli_raw(&app2, &["install", "t5p@1.0.0", "--registry", &base], &[]);
    assert_eq!(code, 0, "stdout:{out}\nstderr:{err}");

    // Unknown flag is a loud error, never a package spec.
    let ( _out, err, code) = run_cli_raw(&app2, &["install", "--bogus-flag"], &[]);
    assert_ne!(code, 0);
    assert!(err.contains("unknown option --bogus-flag"), "{err}");
}

// ---------------------------------------------------------------------------
// T6: URL precedence (5 levels) + https rejection
// ---------------------------------------------------------------------------

#[test]
fn t6_url_precedence_and_https() {
    let _guard = home_lock();
    let old_home = std::env::var("HOME").ok();
    let old_reg = std::env::var("KLANG_REGISTRY").ok();
    let home = tmp("t6home");
    std::env::set_var("HOME", &home);
    std::env::remove_var("KLANG_REGISTRY");
    let proj = tmp("t6proj");
    std::fs::write(proj.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n").unwrap();

    // Level 5: default.
    let u = klang::registry::resolve_registry_url(None, Some(&proj)).unwrap();
    assert_eq!(u, klang::registry::DEFAULT_REGISTRY);

    // Level 4: global config.
    std::fs::create_dir_all(home.join(".klang")).unwrap();
    std::fs::write(home.join(".klang/config.toml"), "[registry]\nurl = \"https://global.example.com\"\n").unwrap();
    let u = klang::registry::resolve_registry_url(None, Some(&proj)).unwrap();
    assert_eq!(u, "https://global.example.com");

    // Level 3: project klang.toml overrides global.
    std::fs::write(
        proj.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[registry]\nurl = \"https://proj.example.com\"\n",
    )
    .unwrap();
    let u = klang::registry::resolve_registry_url(None, Some(&proj)).unwrap();
    assert_eq!(u, "https://proj.example.com");

    // Level 2: env overrides project.
    std::env::set_var("KLANG_REGISTRY", "https://env.example.com");
    let u = klang::registry::resolve_registry_url(None, Some(&proj)).unwrap();
    assert_eq!(u, "https://env.example.com");

    // Level 1: flag overrides env.
    let u = klang::registry::resolve_registry_url(Some("https://flag.example.com"), Some(&proj)).unwrap();
    assert_eq!(u, "https://flag.example.com");

    // Non-https non-localhost rejected.
    assert!(klang::registry::validate_registry_url("http://example.com").is_err());
    let e = klang::registry::validate_registry_url("http://example.com").unwrap_err();
    assert!(e.to_string().contains("registry must use https"), "{e}");
    // Localhost http allowed.
    assert!(klang::registry::validate_registry_url("http://127.0.0.1:8765").is_ok());
    assert!(klang::registry::validate_registry_url("http://localhost:9999").is_ok());
    assert!(klang::registry::validate_registry_url("https://example.com").is_ok());

    // Restore.
    if let Some(h) = old_home {
        std::env::set_var("HOME", h);
    } else {
        std::env::remove_var("HOME");
    }
    if let Some(r) = old_reg {
        std::env::set_var("KLANG_REGISTRY", r);
    } else {
        std::env::remove_var("KLANG_REGISTRY");
    }
}

// ---------------------------------------------------------------------------
// T7: manifest [registry] round-trip
// ---------------------------------------------------------------------------

#[test]
fn t7_manifest_registry_roundtrip() {
    let without = "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n";
    let m = klang::package::Manifest::parse(without).unwrap();
    assert!(m.registry_url.is_none());
    let back = klang::package::Manifest::parse(&m.write_manifest()).unwrap();
    assert_eq!(m, back);

    let with = "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[registry]\nurl = \"https://example.com\"\n";
    let m2 = klang::package::Manifest::parse(with).unwrap();
    assert_eq!(m2.registry_url.as_deref(), Some("https://example.com"));
    let text = m2.write_manifest();
    assert!(text.contains("[registry]"), "{text}");
    let back2 = klang::package::Manifest::parse(&text).unwrap();
    assert_eq!(m2, back2);
}

// ---------------------------------------------------------------------------
// B6/B7: list --all + add --caret
// ---------------------------------------------------------------------------

#[test]
fn t_b6_list_all_shows_transitive() {
    let data = tmp("b6data");
    let base = klang::registry::spawn_ephemeral(&data, LONG_TOKEN).unwrap();
    // Publish dep + root.
    let dep = tmp("b6dep");
    std::fs::write(dep.join("klang.toml"), "name = \"b6dep\"\nversion = \"1.0.0\"\nentry = \"main\"\n").unwrap();
    std::fs::write(dep.join("lib.klang"), "fn d() -> i32 { return 1 }\n").unwrap();
    let files = klang::registry::collect_package_files(&dep).unwrap();
    let a = klang::registry::pack_archive(&files).unwrap();
    klang::registry::publish_pkg(&base, LONG_TOKEN, "b6dep", "1.0.0", &a).unwrap();
    let rootp = tmp("b6root");
    std::fs::write(
        rootp.join("klang.toml"),
        "name = \"b6root\"\nversion = \"1.0.0\"\nentry = \"main\"\n[dependencies]\nb6dep = \"registry:b6dep@1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(rootp.join("lib.klang"), "import \"b6dep/lib.klang\"\nfn r() -> i32 { return d() }\n").unwrap();
    let files = klang::registry::collect_package_files(&rootp).unwrap();
    let a2 = klang::registry::pack_archive(&files).unwrap();
    klang::registry::publish_pkg(&base, LONG_TOKEN, "b6root", "1.0.0", &a2).unwrap();

    let app = tmp("b6app");
    std::fs::write(app.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nb6root = \"registry:b6root@1.0.0\"\n").unwrap();
    klang::registry::fetch_project(&app, &base, false).unwrap();
    let normal = klang::registry::list_project(&app).unwrap();
    assert!(normal.contains("b6root"), "{normal}");
    let all = klang::registry::list_project_all(&app, true).unwrap();
    assert!(all.contains("b6root"), "{all}");
    assert!(all.contains("b6dep"), "{all}");
    // Transitive section indents.
    assert!(all.contains("Transitive") || all.contains("    b6dep"), "{all}");
}

#[test]
fn t_b7_caret_writes_range() {
    let data = tmp("b7data");
    let base = klang::registry::spawn_ephemeral(&data, LONG_TOKEN).unwrap();
    let pkg = tmp("b7pkg");
    std::fs::write(pkg.join("klang.toml"), "name = \"b7p\"\nversion = \"1.2.3\"\nentry = \"main\"\n").unwrap();
    std::fs::write(pkg.join("lib.klang"), "fn f() -> i32 { return 1 }\n").unwrap();
    let files = klang::registry::collect_package_files(&pkg).unwrap();
    let a = klang::registry::pack_archive(&files).unwrap();
    klang::registry::publish_pkg(&base, LONG_TOKEN, "b7p", "1.2.3", &a).unwrap();

    let app = tmp("b7app");
    std::fs::write(app.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n").unwrap();
    // Exact by default.
    klang::registry::add_dependency_req(&app, &base, "b7p", "1.2.3", false, false, false).unwrap();
    let text = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    assert!(text.contains("registry:b7p@1.2.3"), "{text}");

    let app2 = tmp("b7app2");
    std::fs::write(app2.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n").unwrap();
    klang::registry::add_dependency_req(&app2, &base, "b7p", "1.2.3", false, true, false).unwrap();
    let text2 = std::fs::read_to_string(app2.join("klang.toml")).unwrap();
    assert!(text2.contains("registry:b7p@^1.2.3"), "{text2}");
}

// ---------------------------------------------------------------------------
// T8: credentials
// ---------------------------------------------------------------------------

#[test]
fn t8_credentials_perms_and_isolation() {
    let _guard = home_lock();
    let old_home = std::env::var("HOME").ok();
    let home = tmp("t8home");
    std::env::set_var("HOME", &home);

    let reg_a = "https://a.example.com";
    let reg_b = "https://b.example.com";
    klang::registry::save_credential(reg_a, LONG_TOKEN).unwrap();
    // Dir 0700, file 0600 (unix).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dm = std::fs::metadata(home.join(".klang")).unwrap().permissions().mode() & 0o777;
        let fm = std::fs::metadata(home.join(".klang/credentials")).unwrap().permissions().mode() & 0o777;
        assert_eq!(dm, 0o700, "dir mode {dm:o}");
        assert_eq!(fm, 0o600, "file mode {fm:o}");
    }
    // Token for A not used for B.
    assert!(klang::registry::load_credential(reg_a).unwrap().is_some());
    assert!(klang::registry::load_credential(reg_b).unwrap().is_none());
    // Resolve prefers env, then file; file is keyed per-registry.
    std::env::remove_var("KLANG_REGISTRY_TOKEN");
    assert!(klang::registry::resolve_publish_token(None, reg_a).is_ok());
    assert!(klang::registry::resolve_publish_token(None, reg_b).is_err());

    // Group-readable file is refused.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            home.join(".klang/credentials"),
            std::fs::Permissions::from_mode(0o640),
        )
        .unwrap();
        let err = klang::registry::load_credential(reg_a).unwrap_err();
        assert!(err.to_string().contains("chmod 600"), "{err}");
        // Restore for cleanup.
        std::fs::set_permissions(
            home.join(".klang/credentials"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }

    if let Some(h) = old_home {
        std::env::set_var("HOME", h);
    } else {
        std::env::remove_var("HOME");
    }
    std::env::remove_var("KLANG_REGISTRY_TOKEN");
}

// ---------------------------------------------------------------------------
// T9: server gates
// ---------------------------------------------------------------------------

fn raw_http(base: &str, method: &str, path: &str, auth: Option<&str>, body: &[u8]) -> (u16, Vec<u8>, String) {
    use std::io::{Read, Write};
    let addr = base.trim_start_matches("http://").trim_start_matches("https://");
    let mut stream = std::net::TcpStream::connect(addr).expect("connect");
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    if let Some(a) = auth {
        head.push_str(&format!("Authorization: {a}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut resp = Vec::new();
    stream.read_to_end(&mut resp).unwrap();
    let text = String::from_utf8_lossy(&resp).into_owned();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, resp, text)
}

fn crafted_traversal_archive() -> Vec<u8> {
    // KLANGPKG1 with one unsafe name `../evil.klang`.
    let mut out = Vec::new();
    out.extend_from_slice(b"KLANGPKG1");
    out.extend_from_slice(&1u32.to_be_bytes());
    let name = b"../evil.klang";
    out.extend_from_slice(&(name.len() as u16).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(&1u64.to_be_bytes());
    out.push(b'x');
    out
}

#[test]
fn t9_server_gates() {
    // No token -> 503 on publish, reads still work.
    let data = tmp("t9data");
    let base = klang::registry::spawn_ephemeral(&data, "").expect("server starts without token");
    let archive = klang::registry::pack_archive(&[("a.klang".to_string(), b"fn f() -> i32 { return 1 }".to_vec())]).unwrap();
    let (status, _, _) = raw_http(&base, "POST", "/api/publish?name=x&version=1.0.0", Some("Bearer anything"), &archive);
    assert_eq!(status, 503, "publishing disabled without token");
    let (status, _, _) = raw_http(&base, "GET", "/api/packages", None, &[]);
    assert_eq!(status, 200, "reads work with no token");

    // With token: wrong -> 401, right -> 200.
    let data2 = tmp("t9data2");
    let base2 = klang::registry::spawn_ephemeral(&data2, LONG_TOKEN).expect("server");
    let err = klang::registry::publish_pkg(&base2, "wrong-token", "p1", "1.0.0", &archive).expect_err("401");
    assert_eq!(err.kind, "auth", "{err}");
    klang::registry::publish_pkg(&base2, LONG_TOKEN, "p1", "1.0.0", &archive).expect("200");

    // Reads need no token.
    klang::registry::fetch_metadata(&base2, "p1").expect("read with no token");

    // 11th bad attempt -> 429.
    let data3 = tmp("t9data3");
    let base3 = klang::registry::spawn_ephemeral(&data3, LONG_TOKEN).expect("server");
    for _ in 0..10 {
        let (s, _, _) = raw_http(&base3, "POST", "/api/publish?name=q&version=1.0.0", Some("Bearer wrong"), &archive);
        assert_eq!(s, 401);
    }
    let (s, _, text) = raw_http(&base3, "POST", "/api/publish?name=q&version=1.0.0", Some("Bearer wrong"), &archive);
    assert_eq!(s, 429, "rate limited: {text}");
    assert!(text.to_ascii_lowercase().contains("retry-after"), "{text}");

    // Republish same bytes -> 200, different bytes -> 409.
    let dir = tmp("t9pkg");
    std::fs::write(dir.join("klang.toml"), "name = \"rp\"\nversion = \"1.0.0\"\nentry = \"main\"\n").unwrap();
    std::fs::write(dir.join("a.klang"), "fn f() -> i32 { return 1 }\n").unwrap();
    let files = klang::registry::collect_package_files(&dir).unwrap();
    let a1 = klang::registry::pack_archive(&files).unwrap();
    klang::registry::publish_pkg(&base2, LONG_TOKEN, "rp", "1.0.0", &a1).unwrap();
    klang::registry::publish_pkg(&base2, LONG_TOKEN, "rp", "1.0.0", &a1).expect("idempotent 200");
    std::fs::write(dir.join("a.klang"), "fn f() -> i32 { return 2 }\n").unwrap();
    let files2 = klang::registry::collect_package_files(&dir).unwrap();
    let a2 = klang::registry::pack_archive(&files2).unwrap();
    let err = klang::registry::publish_pkg(&base2, LONG_TOKEN, "rp", "1.0.0", &a2).expect_err("409");
    assert_eq!(err.kind, "conflict", "{err}");

    // Oversize -> 413 (6 MiB body).
    let big = vec![0u8; 6 * 1024 * 1024];
    let bearer = format!("Bearer {LONG_TOKEN}");
    let (s, _, _) = raw_http(&base2, "POST", "/api/publish?name=big&version=1.0.0", Some(&bearer), &big);
    assert_eq!(s, 413, "oversize");

    // Traversal path -> 400.
    let trav = crafted_traversal_archive();
    let (s, _, _) = raw_http(&base2, "POST", "/api/publish?name=trav&version=1.0.0", Some(&bearer), &trav);
    assert_eq!(s, 400, "traversal");
}

// ---------------------------------------------------------------------------
// T10: end to end (offline, local registry)
// ---------------------------------------------------------------------------

#[test]
fn t10_e2e_login_publish_add_install_run() {
    let _guard = home_lock();
    let old_home = std::env::var("HOME").ok();
    let home = tmp("t10home");
    std::env::set_var("HOME", &home);
    std::env::remove_var("KLANG_REGISTRY");
    std::env::remove_var("KLANG_REGISTRY_TOKEN");

    let data = tmp("t10data");
    let base = klang::registry::spawn_ephemeral(&data, LONG_TOKEN).expect("server");

    // login via piped stdin (CI shape).
    let mut child = std::process::Command::new(klang_bin())
        .args(["login", "--registry", &base])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("login spawns");
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{LONG_TOKEN}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("login runs");
    assert!(out.status.success(), "login: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("logged in to"), "{stdout}");
    assert!(!stdout.contains(LONG_TOKEN), "token never in output");
    assert!(!String::from_utf8_lossy(&out.stderr).contains(LONG_TOKEN));

    // Publish A (uses credentials file, no env token).
    let pkga = tmp("t10a");
    std::fs::write(pkga.join("klang.toml"), "name = \"t10a\"\nversion = \"1.0.0\"\nentry = \"main\"\n").unwrap();
    std::fs::write(pkga.join("lib.klang"), "fn aval() -> i32 { return 40 }\n").unwrap();
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(["publish", "--dir", pkga.to_str().unwrap(), "--registry", &base])
        .env("HOME", &home);
    cmd.env_remove("KLANG_REGISTRY_TOKEN");
    let out = cmd.output().expect("publish A");
    assert!(out.status.success(), "publish A: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("registry:"), "prints host");

    // Publish B depending on A.
    let pkgb = tmp("t10b");
    std::fs::write(
        pkgb.join("klang.toml"),
        "name = \"t10b\"\nversion = \"1.0.0\"\nentry = \"main\"\n[dependencies]\nt10a = \"registry:t10a@1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(pkgb.join("lib.klang"), "import \"t10a/lib.klang\"\nfn bval() -> i32 { return aval() + 2 }\n").unwrap();
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(["publish", "--dir", pkgb.to_str().unwrap(), "--registry", &base])
        .env("HOME", &home);
    cmd.env_remove("KLANG_REGISTRY_TOKEN");
    let out = cmd.output().expect("publish B");
    assert!(out.status.success(), "publish B: {}", String::from_utf8_lossy(&out.stderr));

    // Fresh project: add B (A vendored transitively).
    let app = tmp("t10app");
    std::fs::write(app.join("klang.toml"), "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n").unwrap();
    std::fs::write(
        app.join("app.klang"),
        "import \"t10b/lib.klang\"\nfn main() -> i32 { return bval() }\n",
    )
    .unwrap();
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(["add", "t10b@1.0.0", "--registry", &base])
        .current_dir(&app)
        .env("HOME", &home);
    cmd.env_remove("KLANG_REGISTRY_TOKEN");
    let out = cmd.output().expect("add B");
    assert!(out.status.success(), "add B: {}", String::from_utf8_lossy(&out.stderr));
    assert!(app.join(".klang_pkgs/t10b/1.0.0/lib.klang").is_file());
    assert!(app.join(".klang_pkgs/t10a/1.0.0/lib.klang").is_file(), "transitive A vendored");

    // Delete vendor dir, install restores from lock.
    std::fs::remove_dir_all(app.join(".klang_pkgs")).unwrap();
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(["install", "--registry", &base])
        .current_dir(&app)
        .env("HOME", &home);
    let out = cmd.output().expect("install");
    assert!(out.status.success(), "install: {}", String::from_utf8_lossy(&out.stderr));
    assert!(app.join(".klang_pkgs/t10a/1.0.0/lib.klang").is_file());

    // Run prints expected output (exit code is main's return).
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(["run", "app.klang", "--registry", &base])
        .current_dir(&app)
        .env("HOME", &home);
    let out = cmd.output().expect("run");
    // main returns 42 -> exit code 42.
    assert_eq!(out.status.code(), Some(42), "run: stdout={} stderr={}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));

    if let Some(h) = old_home {
        std::env::set_var("HOME", h);
    } else {
        std::env::remove_var("HOME");
    }
}

// ---------------------------------------------------------------------------
// Phase 1b: multi-name `klang add`
// ---------------------------------------------------------------------------

/// Publish one tiny package to the ephemeral registry.
fn publish_tiny(base: &str, tag: &str, name: &str, version: &str, body: &str) {
    let pkg = tmp(&format!("mm-{tag}-{name}"));
    std::fs::write(
        pkg.join("klang.toml"),
        format!("name = \"{name}\"\nversion = \"{version}\"\nentry = \"main\"\n"),
    )
    .unwrap();
    std::fs::write(pkg.join("lib.klang"), body).unwrap();
    let files = klang::registry::collect_package_files(&pkg).unwrap();
    let a = klang::registry::pack_archive(&files).unwrap();
    klang::registry::publish_pkg(base, LONG_TOKEN, name, version, &a).unwrap();
}

fn fresh_app(tag: &str) -> PathBuf {
    let app = tmp(&format!("mm-app-{tag}"));
    std::fs::write(
        app.join("klang.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n",
    )
    .unwrap();
    app
}

#[test]
fn t_multi_add_two_names_one_call() {
    // End to end through the real CLI: `add mma mmb@1.0.0` installs both,
    // pins both, and reports both in one line.
    let data = tmp("mm-data");
    let base = klang::registry::spawn_ephemeral(&data, LONG_TOKEN).unwrap();
    publish_tiny(&base, "one", "mma", "1.0.0", "fn aval() -> i32 { return 40 }\n");
    publish_tiny(&base, "one", "mmb", "1.0.0", "fn bval() -> i32 { return 2 }\n");

    let app = fresh_app("one");
    let mut cmd = std::process::Command::new(klang_bin());
    cmd.args(["add", "mma", "mmb@1.0.0", "--registry", &base])
        .current_dir(&app);
    cmd.env_remove("KLANG_REGISTRY");
    cmd.env_remove("KLANG_REGISTRY_TOKEN");
    let out = cmd.output().expect("add runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "add: stdout={stdout} stderr={}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("added mma@1.0.0, mmb@1.0.0"), "{stdout}");
    let toml = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    assert!(toml.contains("mma = \"registry:mma@1.0.0\""), "{toml}");
    assert!(toml.contains("mmb = \"registry:mmb@1.0.0\""), "{toml}");
    let lock = std::fs::read_to_string(app.join("klang.lock")).unwrap();
    assert!(lock.contains("package mma 1.0.0"), "{lock}");
    assert!(lock.contains("package mmb 1.0.0"), "{lock}");
    assert!(app.join(".klang_pkgs/mma/1.0.0/lib.klang").is_file());
    assert!(app.join(".klang_pkgs/mmb/1.0.0/lib.klang").is_file());

    // The single-name form still works with its historical output.
    let app2 = fresh_app("one-single");
    let line = klang::package_manager::cli::cmd_add_full(&app2, &base, "mma", false, false, false)
        .expect("single add runs");
    assert_eq!(line, "added mma@1.0.0");
    println!("multi-add one-call OK");
}

#[test]
fn t_multi_add_all_or_none() {
    // One good spec + one unresolvable spec: nothing is written anywhere.
    let data = tmp("mm-non-data");
    let base = klang::registry::spawn_ephemeral(&data, LONG_TOKEN).unwrap();
    publish_tiny(&base, "none", "mmgood", "1.0.0", "fn g() -> i32 { return 1 }\n");

    let app = fresh_app("none");
    let before = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    let err = klang::registry::add_dependencies_req(
        &app,
        &base,
        &[
            ("mmgood".to_string(), "1.0.0".to_string()),
            ("mmmissing".to_string(), "1.0.0".to_string()),
        ],
        false,
        false,
        false,
    )
    .expect_err("missing package fails the whole call");
    assert!(
        err.to_string().contains("mmmissing"),
        "names the bad spec: {err}"
    );
    let after = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    assert_eq!(before, after, "manifest untouched");
    assert!(!app.join("klang.lock").exists(), "no lock created");
    assert!(!app.join(".klang_pkgs").exists(), "nothing vendored");
    println!("multi-add all-or-none OK");
}

#[test]
fn t_multi_add_rejects_dup_and_present() {
    let data = tmp("mm-dup-data");
    let base = klang::registry::spawn_ephemeral(&data, LONG_TOKEN).unwrap();
    publish_tiny(&base, "dup", "mmd", "1.0.0", "fn d() -> i32 { return 1 }\n");
    publish_tiny(&base, "dup", "mme", "1.0.0", "fn e() -> i32 { return 2 }\n");

    // Same name twice in one call.
    let app = fresh_app("dup");
    let before = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    let err = klang::registry::add_dependencies_req(
        &app,
        &base,
        &[
            ("mmd".to_string(), "1.0.0".to_string()),
            ("mmd".to_string(), "1.0.0".to_string()),
        ],
        false,
        false,
        false,
    )
    .expect_err("duplicate in one call fails");
    assert!(err.to_string().contains("listed twice"), "{err}");
    assert_eq!(std::fs::read_to_string(app.join("klang.toml")).unwrap(), before);

    // One already-present dep poisons the whole call (the new one is not added).
    klang::registry::add_dependency_req(&app, &base, "mmd", "1.0.0", false, false, false).unwrap();
    let with_first = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    let err = klang::registry::add_dependencies_req(
        &app,
        &base,
        &[
            ("mmd".to_string(), "1.0.0".to_string()),
            ("mme".to_string(), "1.0.0".to_string()),
        ],
        false,
        false,
        false,
    )
    .expect_err("present dep fails the whole call");
    assert!(err.to_string().contains("already a dependency"), "{err}");
    let after = std::fs::read_to_string(app.join("klang.toml")).unwrap();
    assert_eq!(with_first, after, "manifest untouched");
    assert!(!after.contains("mme = "), "{after}");
    println!("multi-add dup/present OK");
}

#[test]
fn t_add_usage_names_plural() {
    // No specs: usage names the multi-name form and exits 2.
    let (stdout, stderr, code) = run_cli_raw(&tmp("mm-usage"), &["add"], &[]);
    assert_eq!(code, 2, "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("add <name[@constraint]...>"),
        "usage names several specs: {stderr}"
    );
    println!("add usage OK");
}
