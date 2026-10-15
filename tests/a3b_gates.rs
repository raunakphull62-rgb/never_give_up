//! A3b Postgres client gates (Wave 2 Stage A3b).
//!
//! Live-DB tests run only when `KLANG_TEST_PG_URL` is set (e.g.
//! `postgres://klang:klangtest@localhost/klang_test`); otherwise they
//! skip cleanly (`eprintln!("SKIP ...")` + return) so the normal suite
//! stays green. Tests that need no server (URL parsing, redaction,
//! arity/type, sandbox denial, JIT/v2 rejection, hostile no-panic,
//! refused-host) always run. All live tests use `TEMP` tables (per
//! connection, auto-cleanup) plus explicit `pg_close`; unique temp dirs
//! per test, removed afterwards. Loopback only, no fixed ports.

use std::collections::HashMap;

use klang::parser::Parser;

static A3B_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn parse_ok(src: &str) -> klang::ast::Program {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    prog
}

fn run_ok(src: &str) -> (i32, Vec<String>) {
    let _guard = A3B_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    klang::sandbox::set_sandbox(false);
    r
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let _guard = A3B_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let e = klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new())
        .expect_err("must fail");
    klang::sandbox::set_sandbox(false);
    e
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&want), "want {want} in {codes:?}: {src}");
}

fn run_sandboxed(src: &str) -> Result<(i32, Vec<String>), String> {
    let _guard = A3B_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(true);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    klang::sandbox::set_sandbox(false);
    match r {
        Ok(v) => Ok(v),
        Err(d) => Err(format!("{} {}", d.code, d.to_json())),
    }
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

/// Live-DB URL or `None` (skip). Appends `?sslmode=disable` for
/// localhost URLs without an explicit sslmode (local snakeoil certs
/// fail verification; production URLs with valid certs use `prefer`).
fn pg_url() -> Option<String> {
    let raw = std::env::var("KLANG_TEST_PG_URL").ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    let raw = raw.trim().to_string();
    if raw.contains("sslmode=") {
        return Some(raw);
    }
    // Only auto-disable for loopback (never for remote hosts).
    if raw.contains("@localhost") || raw.contains("@127.0.0.1") || raw.contains("@[::1]") {
        if raw.contains('?') {
            return Some(format!("{raw}&sslmode=disable"));
        } else {
            return Some(format!("{raw}?sslmode=disable"));
        }
    }
    Some(raw)
}

fn require_pg_url() -> Option<String> {
    match pg_url() {
        Some(u) => Some(u),
        None => {
            eprintln!("SKIP live PG test (KLANG_TEST_PG_URL not set)");
            None
        }
    }
}

fn klang_str_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---- always-run: URL parsing / error codes (no server) ----

#[test]
fn a3b_url_bad_scheme_is_connect() {
    let e = run_err("fn main() -> i32 { let db = pg_connect(\"http://localhost/db\") return 0 }");
    assert_eq!(e.code, "E-PG-CONNECT", "got: {}", e.to_json());
}

#[test]
fn a3b_url_missing_host_is_connect() {
    let e = run_err("fn main() -> i32 { let db = pg_connect(\"postgres://bob@/mydb\") return 0 }");
    assert_eq!(e.code, "E-PG-CONNECT", "got: {}", e.to_json());
}

#[test]
fn a3b_url_bad_port_is_connect() {
    let e = run_err("fn main() -> i32 { let db = pg_connect(\"postgres://bob@localhost:99999/mydb?sslmode=disable\") return 0 }");
    assert_eq!(e.code, "E-PG-CONNECT", "got: {}", e.to_json());
}

#[test]
fn a3b_url_bad_sslmode_is_connect() {
    let e = run_err("fn main() -> i32 { let db = pg_connect(\"postgres://bob@localhost/mydb?sslmode=bogus\") return 0 }");
    assert_eq!(e.code, "E-PG-CONNECT", "got: {}", e.to_json());
}

#[test]
fn a3b_password_never_leaks() {
    // Bad host guarantees failure; the password must not appear anywhere.
    let secret = "s3cr3t-pw-9zqx";
    let src = format!("fn main() -> i32 {{ let db = pg_connect(\"postgres://bob:{secret}@127.0.0.1:1/mydb?sslmode=disable\") return 0 }}");
    let e = run_err(&src);
    assert_eq!(e.code, "E-PG-CONNECT", "got: {}", e.to_json());
    let j = e.to_json();
    assert!(!j.contains(secret), "password leaked: {j}");
    assert!(j.contains("***"), "redacted marker missing: {j}");
    // Unit-level redaction too.
    let r = klang::stdlib::a3b_pg::redact_url(&format!("postgres://bob:{secret}@h/db"));
    assert!(r.contains("***") && !r.contains(secret), "redact_url: {r}");
    let r2 = klang::stdlib::a3b_pg::redact_url("postgres://bob@h/db");
    assert!(r2.contains("bob"), "no-password URL unchanged: {r2}");
}

#[test]
fn a3b_connect_refused_is_connect() {
    // Loopback port 1 refuses fast (no hang, no internet).
    let e = run_err("fn main() -> i32 { let db = pg_connect(\"postgres://u@localhost:1/db?sslmode=disable\") return 0 }");
    assert_eq!(e.code, "E-PG-CONNECT", "got: {}", e.to_json());
}

#[test]
fn a3b_static_arity_and_types() {
    check_err_code("fn main() -> i32 { let x = pg_connect() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = pg_exec(\"a\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = pg_query(\"a\", \"b\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = pg_begin() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = pg_commit(\"a\", \"b\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = pg_rollback() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = pg_close() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = pg_connect(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = pg_exec(42, \"s\", []) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = pg_exec(\"a\", 42, []) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = pg_exec(\"a\", \"s\", 42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = pg_query(42, \"s\", []) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = pg_begin(42) return 0 }", "E-TYPE");
}

#[test]
fn a3b_dynamic_etype_no_panic() {
    // Dynamic (Unknown-typed) wrong handle kinds are runtime E-TYPE, never panic.
    // Use map lookups (Unknown statically) so HIR passes and runtime reports E-TYPE.
    let e = run_err("fn main() -> i32 { let m = {\"k\": 42} let x = pg_exec(m[\"k\"], \"SELECT 1\", []) return 0 }");
    assert_eq!(e.code, "E-TYPE", "got: {}", e.to_json());
    let e = run_err("fn main() -> i32 { let m = {\"k\": 42} let x = pg_query(m[\"k\"], \"SELECT 1\", []) return 0 }");
    assert_eq!(e.code, "E-TYPE", "got: {}", e.to_json());
    let e = run_err("fn main() -> i32 { let m = {\"k\": 42} let x = pg_begin(m[\"k\"]) return 0 }");
    assert_eq!(e.code, "E-TYPE", "got: {}", e.to_json());
}

#[test]
fn a3b_hostile_no_panic() {
    // NUL bytes, huge inputs, wrong shapes: loud codes, never host panic.
    let e = run_err("fn main() -> i32 { let db = pg_connect(\"/tmp/a\\u0000b\") return 0 }");
    assert_eq!(e.code, "E-PG-CONNECT", "got: {}", e.to_json());
    // Static wrong-handle is check-time E-TYPE (never a panic).
    check_err_code("fn main() -> i32 { let x = pg_close(42) return 0 }", "E-TYPE");
    // Runtime wrong-handle via Unknown (passes check, loud at runtime).
    let e = run_err("fn main() -> i32 { let m = {\"k\": 42} let x = pg_close(m[\"k\"]) return 0 }");
    assert_eq!(e.code, "E-TYPE", "got: {}", e.to_json());
    // Direct Rust-level refused connect (no panic).
    let h = klang::stdlib::a3b_pg::pg_connect("postgres://u@127.0.0.1:1/db?sslmode=disable")
        .expect_err("refused");
    assert_eq!(h.code, "E-PG-CONNECT");
}

// ---- always-run: sandbox denies all pg_* ----

#[test]
fn a3b_sandbox_denies_all() {
    // pg_connect takes Str (passes check), others take Pg handles: use
    // Unknown-typed map lookups so HIR passes and runtime hits the sandbox
    // denial first (before type checks).
    let denied = [
        "fn main() -> i32 { let db = pg_connect(\"postgres://u@h/db?sslmode=disable\") return 0 }",
        "fn main() -> i32 { let m = {\"k\": \"x\"} let r = pg_exec(m[\"k\"], \"SELECT 1\", []) return 0 }",
        "fn main() -> i32 { let m = {\"k\": \"x\"} let r = pg_query(m[\"k\"], \"SELECT 1\", []) return 0 }",
        "fn main() -> i32 { let m = {\"k\": \"x\"} let r = pg_begin(m[\"k\"]) return 0 }",
        "fn main() -> i32 { let m = {\"k\": \"x\"} let r = pg_commit(m[\"k\"]) return 0 }",
        "fn main() -> i32 { let m = {\"k\": \"x\"} let r = pg_rollback(m[\"k\"]) return 0 }",
        "fn main() -> i32 { let m = {\"k\": \"x\"} let r = pg_close(m[\"k\"]) return 0 }",
    ];
    for src in denied {
        let e = run_sandboxed(src).expect_err("sandbox must deny");
        assert!(e.starts_with("E-SANDBOX-NETWORK"), "got: {e} for {src}");
    }
}

// ---- always-run: JIT + v2 reject loudly ----

#[test]
fn a3b_jit_rejects_new_builtins() {
    for src in [
        "fn main() -> i32 { let db = pg_connect(\"x\") return 0 }",
        "fn main() -> i32 { let r = pg_exec(\"a\", \"b\", []) return 0 }",
        "fn main() -> i32 { let r = pg_query(\"a\", \"b\", []) return 0 }",
        "fn main() -> i32 { let r = pg_begin(\"a\") return 0 }",
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses");
        // pg_exec/pg_query with Str handles fail HIR (E-TYPE); use check to
        // confirm loudness, but JIT test needs HIR-clean programs. For
        // Str-handle cases HIR fails, so only assert JIT for HIR-clean ones.
        if klang::hir::TypedHIR::check(prog.clone()).is_err() {
            continue;
        }
        let mir = klang::mir::lower(&prog);
        let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject");
        assert!(err.contains("unsupported"), "loud JIT-FAIL, got: {err}");
    }
    // Int-handle programs are HIR-clean (Unknown? no, Int fails too). Use
    // Unknown via map to get HIR-clean JIT rejection naming the builtin.
    // Simplest loud check: pg_connect with Str passes HIR (Str expected).
    let src = "fn main() -> i32 { let db = pg_connect(\"x\") return 0 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject");
    assert!(err.contains("unsupported"), "got: {err}");
}

#[test]
fn a3b_v2_rejects_new_builtins() {
    let src = "fn main() -> i32 {\n    print(pg_connect(\"x\"))\n    return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let err = klang::runtime::v2::run_v2_program(&prog, "main").expect_err("v2 must reject");
    assert_eq!(err.code, "E-UNDEFINED", "got: {}", err.to_json());
}

// ---- live-DB tests (skip cleanly without KLANG_TEST_PG_URL) ----

fn live_url_or_skip() -> Option<String> {
    require_pg_url()
}

#[test]
fn a3b_live_crud_bound_params() {
    let Some(url) = live_url_or_skip() else { return };
    let u = klang_str_escape(&url);
    let src = format!(
        "fn main() -> i32 {{\n    let db = pg_connect(\"{u}\")\n    pg_exec(db, \"DROP TABLE IF EXISTS a3b_crud\", [])\n    pg_exec(db, \"CREATE TEMP TABLE a3b_crud (id INT PRIMARY KEY, name TEXT)\", [])\n    let ins = pg_exec(db, \"INSERT INTO a3b_crud (id, name) VALUES ($1, $2)\", [1, \"alice\"])\n    print(ins[\"changes\"])\n    let rows = pg_query(db, \"SELECT id, name FROM a3b_crud WHERE id = $1\", [1])\n    print(len(rows))\n    print(rows[0][\"id\"])\n    print(rows[0][\"name\"])\n    let upd = pg_exec(db, \"UPDATE a3b_crud SET name = $1 WHERE id = $2\", [\"bob\", 1])\n    print(upd[\"changes\"])\n    let rows2 = pg_query(db, \"SELECT name FROM a3b_crud WHERE id = $1\", [1])\n    print(rows2[0][\"name\"])\n    let del = pg_exec(db, \"DELETE FROM a3b_crud WHERE id = $1\", [1])\n    print(del[\"changes\"])\n    pg_close(db)\n    return 0\n}}"
    );
    let (v, out) = run_ok(&src);
    assert_eq!(v, 0);
    assert_eq!(out, vec!["1", "1", "1", "alice", "1", "bob", "1"]);
}

#[test]
fn a3b_live_null_both_ways() {
    let Some(url) = live_url_or_skip() else { return };
    let u = klang_str_escape(&url);
    // Null param -> SQL NULL; NULL column -> Klang Null (is_null).
    let src = format!(
        "fn main() -> i32 {{\n    let db = pg_connect(\"{u}\")\n    pg_exec(db, \"DROP TABLE IF EXISTS a3b_null\", [])\n    pg_exec(db, \"CREATE TEMP TABLE a3b_null (id INT, note TEXT)\", [])\n    pg_exec(db, \"INSERT INTO a3b_null (id, note) VALUES ($1, $2)\", [1, null])\n    let rows = pg_query(db, \"SELECT id, note FROM a3b_null\", [])\n    print(rows[0][\"id\"])\n    print(is_null(rows[0][\"note\"]))\n    let n2 = pg_query(db, \"SELECT null as v\", [])\n    print(is_null(n2[0][\"v\"]))\n    pg_close(db)\n    return 7\n}}"
    );
    // Note: `null` has no literal; use a Null-producing expr: is_null needs a Null value.
    // `pg_query("SELECT null")` returns Null; for param Null we use `is_null` trick via closed-timeout? Instead use json_decode("null") which is Null per A3.
    let src = src.replace("[1, null]", "[1, json_decode(\"null\")]");
    let (v, out) = run_ok(&src);
    assert_eq!(v, 7);
    assert_eq!(out, vec!["1", "1", "1"]);
}

#[test]
fn a3b_live_types_map() {
    let Some(url) = live_url_or_skip() else { return };
    let u = klang_str_escape(&url);
    let src = format!(
        "fn main() -> i32 {{\n    let db = pg_connect(\"{u}\")\n    pg_exec(db, \"DROP TABLE IF EXISTS a3b_types\", [])\n    pg_exec(db, \"CREATE TEMP TABLE a3b_types (i INT, b BIGINT, f DOUBLE PRECISION, n NUMERIC(10,2), t TEXT, bl BOOL, ts TIMESTAMP, js JSONB)\", [])\n    pg_exec(db, \"INSERT INTO a3b_types (i,b,f,n,t,bl,ts,js) VALUES ($1, $2, $3, 12.30, $4, true, '2024-02-29T12:00:00'::timestamp, '[1,2]'::jsonb)\", [7, 7000000000, 3.5, \"hi\"])\n    let rows = pg_query(db, \"SELECT i, b, f, n::text as n, t, bl, ts, js FROM a3b_types\", [])\n    print(rows[0][\"i\"])\n    print(rows[0][\"b\"])\n    print(rows[0][\"f\"])\n    print(rows[0][\"n\"])\n    print(rows[0][\"t\"])\n    print(rows[0][\"bl\"])\n    print(rows[0][\"ts\"])\n    print(rows[0][\"js\"])\n    pg_close(db)\n    return 0\n}}"
    );
    let (v, out) = run_ok(&src);
    assert_eq!(v, 0);
    // i=7, b=7000000000, f=3.5, n="12.30" (NUMERIC via ::text cast, exact), t="hi", bl=1 (true->1), ts contains 2024-02-29, js is [1,2].
    assert_eq!(out[0], "7");
    assert_eq!(out[1], "7000000000");
    assert!(out[2].starts_with("3.5"), "float: {out:?}");
    assert_eq!(out[3], "12.30", "numeric via ::text: {out:?}");
    assert_eq!(out[4], "hi");
    assert_eq!(out[5], "1");
    assert!(out[6].contains("2024-02-29"), "ts: {out:?}");
    assert!(out[7].contains("1") && out[7].contains("2"), "json: {out:?}");
}

#[test]
fn a3b_live_params_are_bound_not_interpolated() {
    let Some(url) = live_url_or_skip() else { return };
    let u = klang_str_escape(&url);
    // Injection string must land as data, not as SQL.
    let src = format!(
        "fn main() -> i32 {{\n    let db = pg_connect(\"{u}\")\n    pg_exec(db, \"DROP TABLE IF EXISTS a3b_inj\", [])\n    pg_exec(db, \"CREATE TEMP TABLE a3b_inj (id INT, v TEXT)\", [])\n    let evil = \"'; DROP TABLE a3b_inj; --\"\n    pg_exec(db, \"INSERT INTO a3b_inj VALUES ($1, $2)\", [1, evil])\n    let rows = pg_query(db, \"SELECT v FROM a3b_inj WHERE id = $1\", [1])\n    print(rows[0][\"v\"])\n    let still = pg_query(db, \"SELECT count(*) as c FROM a3b_inj\", [])\n    print(still[0][\"c\"])\n    pg_close(db)\n    return 0\n}}"
    );
    let (v, out) = run_ok(&src);
    assert_eq!(v, 0);
    assert_eq!(out[0], "'; DROP TABLE a3b_inj; --");
    assert_eq!(out[1], "1");
}

#[test]
fn a3b_live_err_sql_has_state_no_password() {
    let Some(url) = live_url_or_skip() else { return };
    let u = klang_str_escape(&url);
    let src = format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") print(pg_query(db, \"SELECT * FROM no_such_table_xyz\", [])) return 0 }}");
    let e = run_err(&src);
    assert_eq!(e.code, "E-PG-SQL", "got: {}", e.to_json());
    let j = e.to_json();
    // Must include SQLSTATE (42P01 undefined_table) and never the password.
    assert!(j.contains("42P01"), "SQLSTATE missing: {j}");
    assert!(!j.contains("klangtest"), "password leaked: {j}");
    // Syntax error is also E-PG-SQL, never panic.
    let src2 = format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") print(pg_exec(db, \"THIS IS NOT SQL\", [])) return 0 }}");
    let e2 = run_err(&src2);
    assert_eq!(e2.code, "E-PG-SQL", "got: {}", e2.to_json());
}

#[test]
fn a3b_live_err_auth_never_leaks() {
    let Some(url) = live_url_or_skip() else { return };
    // Same host/db, wrong password -> E-PG-AUTH, no leak.
    let mut bad = url.clone();
    // Replace password segment with wrong value (keep structure).
    // Our test URL always has user:pass@; swap pass.
    if let Some(at) = bad.find('@') {
        if let Some(col) = bad[..at].rfind(':') {
            bad.replace_range(col + 1..at, "wrong-pw-xyz");
        }
    }
    // Ensure sslmode=disable preserved for localhost (already in url).
    let b = klang_str_escape(&bad);
    let src = format!("fn main() -> i32 {{ let db = pg_connect(\"{b}\") return 0 }}");
    let e = run_err(&src);
    assert_eq!(e.code, "E-PG-AUTH", "got: {}", e.to_json());
    let j = e.to_json();
    assert!(!j.contains("wrong-pw-xyz"), "password leaked: {j}");
    assert!(j.contains("***") || j.contains("authentication"), "redacted/auth: {j}");
}

#[test]
fn a3b_live_txn_commit_and_rollback() {
    let Some(url) = live_url_or_skip() else { return };
    let u = klang_str_escape(&url);
    let src = format!(
        "fn main() -> i32 {{\n    let db = pg_connect(\"{u}\")\n    pg_exec(db, \"DROP TABLE IF EXISTS a3b_txn\", [])\n    pg_exec(db, \"CREATE TEMP TABLE a3b_txn (id INT)\", [])\n    pg_begin(db)\n    pg_exec(db, \"INSERT INTO a3b_txn VALUES ($1)\", [1])\n    pg_rollback(db)\n    let r1 = pg_query(db, \"SELECT count(*) as c FROM a3b_txn\", [])\n    print(r1[0][\"c\"])\n    pg_begin(db)\n    pg_exec(db, \"INSERT INTO a3b_txn VALUES ($1)\", [2])\n    pg_commit(db)\n    let r2 = pg_query(db, \"SELECT count(*) as c FROM a3b_txn\", [])\n    print(r2[0][\"c\"])\n    pg_close(db)\n    return 0\n}}"
    );
    let (v, out) = run_ok(&src);
    assert_eq!(v, 0);
    assert_eq!(out, vec!["0", "1"]);
}

#[test]
fn a3b_live_err_tx_and_closed_and_type() {
    let Some(url) = live_url_or_skip() else { return };
    let u = klang_str_escape(&url);
    // TX misuse.
    let e = run_err(&format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") pg_commit(db) return 0 }}"));
    assert_eq!(e.code, "E-PG-TX", "got: {}", e.to_json());
    let e = run_err(&format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") pg_rollback(db) return 0 }}"));
    assert_eq!(e.code, "E-PG-TX", "got: {}", e.to_json());
    let e = run_err(&format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") pg_begin(db) pg_begin(db) return 0 }}"));
    assert_eq!(e.code, "E-PG-TX", "got: {}", e.to_json());
    // CLOSED: close then use -> E-PG-CLOSED; close is idempotent.
    let (v, out) = run_ok(&format!(
        "fn main() -> i32 {{ let db = pg_connect(\"{u}\") pg_close(db) print(pg_close(db)) return 5 }}"
    ));
    assert_eq!(v, 5);
    assert_eq!(out, vec!["1".to_string()]);
    let e = run_err(&format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") pg_close(db) print(pg_exec(db, \"SELECT 1\", [])) return 0 }}"));
    assert_eq!(e.code, "E-PG-CLOSED", "got: {}", e.to_json());
    let e = run_err(&format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") print(pg_query(db, \"SELECT 1\", [])) pg_close(db) print(pg_query(db, \"SELECT 1\", [])) return 0 }}"));
    assert_eq!(e.code, "E-PG-CLOSED", "got: {}", e.to_json());
    // TYPE: Map as a param value -> E-PG-TYPE (needs a live handle).
    let e = run_err(&format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") print(pg_exec(db, \"SELECT $1\", [{{\"a\": 1}}])) return 0 }}"));
    assert_eq!(e.code, "E-PG-TYPE", "got: {}", e.to_json());
}

// ---- live: notes_api example (needs PG + loopback HTTP) ----

#[test]
fn a3b_live_notes_api_crud() {
    let Some(url) = live_url_or_skip() else { return };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let prog_path = root.join("examples/apps/notes_api/main.klang");
    assert!(prog_path.is_file(), "notes_api example exists");
    // fmt must stay clean.
    let fmt = std::process::Command::new(klang_bin())
        .args(["fmt", "--check", prog_path.to_str().unwrap()])
        .output()
        .expect("fmt runs");
    assert!(fmt.status.success(), "fmt --check: {}", String::from_utf8_lossy(&fmt.stderr));
    // Unique temp dir for PORT_FILE; ensure clean `notes` table afterwards via direct SQL.
    let dir = {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        p.push(format!("a3b_notes_{}_{}", std::process::id(), nanos % 1_000_000_000));
        std::fs::create_dir_all(&p).expect("mktemp");
        p
    };
    let port_file = dir.join("port.txt");
    let mut child = std::process::Command::new(klang_bin())
        .args(["run", prog_path.to_str().unwrap()])
        .env("DATABASE_URL", &url)
        .env("PORT", "0")
        .env("PORT_FILE", &port_file)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn notes_api");
    // Wait for PORT_FILE (up to ~10s).
    let mut port: Option<String> = None;
    for _ in 0..100 {
        if let Ok(t) = std::fs::read_to_string(&port_file) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                port = Some(t);
                break;
            }
        }
        // If child died early, fail with logs.
        if let Ok(Some(st)) = child.try_wait() {
            let out = child.wait_with_output().expect("wait");
            panic!(
                "notes_api exited early ({st}): stdout={} stderr={}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let port = port.expect("PORT_FILE written");
    let base = format!("http://127.0.0.1:{port}");
    // Minimal HTTP client via ureq (already a dependency) + serde_json (direct dep).
    // Use `Connection: close` (like hello_api raw TCP) so server workers exit
    // immediately after each response; keep-alive would leave workers in read
    // timeout (10s) and break the 5s SIGTERM shutdown assertion.
    let get_json = |path: &str| -> serde_json::Value {
        let mut res = ureq::get(&format!("{base}{path}"))
            .header("Connection", "close")
            .call()
            .expect("get");
        assert_eq!(res.status().as_u16(), 200, "{path}");
        let body = res.body_mut().read_to_string().expect("body");
        serde_json::from_str(&body).expect("json")
    };
    // GET /health
    let health = get_json("/health");
    assert_eq!(health["status"], "ok");
    // POST /notes
    let mut res = ureq::post(&format!("{base}/notes"))
        .header("Content-Type", "application/json")
        .header("Connection", "close")
        .send("{\"text\": \"hello\"}")
        .expect("create");
    assert_eq!(res.status().as_u16(), 200);
    let body = res.body_mut().read_to_string().expect("body");
    let created: serde_json::Value = serde_json::from_str(&body).expect("create json");
    let id = created["id"].as_i64().expect("id int");
    // GET /notes contains it.
    let list = get_json("/notes");
    assert!(list.as_array().expect("array").iter().any(|n| n["id"] == id && n["text"] == "hello"));
    // GET /notes/:id
    let one = get_json(&format!("/notes/{id}"));
    assert_eq!(one["text"], "hello");
    // DELETE /notes/:id then 404.
    let del_res = ureq::delete(&format!("{base}/notes/{id}"))
        .header("Connection", "close")
        .call()
        .expect("delete");
    assert_eq!(del_res.status().as_u16(), 200);
    match ureq::get(&format!("{base}/notes/{id}"))
        .header("Connection", "close")
        .call()
    {
        Ok(_) => panic!("deleted note should 404"),
        Err(ureq::Error::StatusCode(code)) => assert_eq!(code, 404),
        Err(e) => panic!("expected 404, got {e:?}"),
    }
    // Clean shutdown: SIGTERM like hello_api (exit 0 <5s).
    let pid = child.id();
    std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output()
        .expect("kill runs");
    let status = child.wait_timeout_or_kill(std::time::Duration::from_secs(5));
    assert!(status.success(), "notes_api SIGTERM exit 0");
    std::fs::remove_dir_all(&dir).ok();
    // Drop the example table (example uses plain `notes`, not TEMP).
    let u = url.clone();
    let _ = run_ok(&format!(
        "fn main() -> i32 {{ let db = pg_connect(\"{}\") pg_exec(db, \"DROP TABLE IF EXISTS notes\", []) pg_close(db) return 0 }}",
        klang_str_escape(&u)
    ));
}

// Helper: wait with timeout then kill (unix only in this env).
trait WaitTimeoutOrKill {
    fn wait_timeout_or_kill(&mut self, dur: std::time::Duration) -> std::process::ExitStatus;
}

impl WaitTimeoutOrKill for std::process::Child {
    fn wait_timeout_or_kill(&mut self, dur: std::time::Duration) -> std::process::ExitStatus {
        let start = std::time::Instant::now();
        loop {
            if let Ok(Some(st)) = self.try_wait() {
                return st;
            }
            if start.elapsed() > dur {
                let _ = self.kill();
                return self.wait().expect("wait after kill");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
