use std::collections::HashMap;

use klang::parser::Parser;

const MILESTONE: &str = r#"
fn fetch_a() -> i32 throws {
}

fn fetch_b() -> i32 throws {
}

fn combine() -> i32 throws async {
    task_group {
        let a = spawn fetch_a()
        let b = spawn fetch_b()
        return await a + await b
    }
}
"#;

#[test]
fn stage2_memo_hits_on_unchanged_content() {
    let mut db = klang::db::Database::new();
    db.set_file("main.klang", MILESTONE);
    assert!(db.check("main.klang").is_ok());
    let runs = db.parse_runs();
    assert!(runs >= 1, "first parse runs, got {runs}");
    assert!(db.check("main.klang").is_ok());
    assert_eq!(db.parse_runs(), runs, "memo must prevent re-parse");
    println!("stage2 memo OK: parse_runs={runs} rev={}", db.revision());
}

#[test]
fn stage2_revision_bumps_only_on_change() {
    let mut db = klang::db::Database::new();
    let r1 = db.set_file("a.klang", MILESTONE);
    let r2 = db.set_file("a.klang", MILESTONE);
    assert_eq!(r1, r2, "same content must not bump revision");
    let r3 = db.set_file("a.klang", &format!("{MILESTONE}\n"));
    assert!(r3 > r2, "changed content must bump revision");
    println!("stage2 revision OK: {r1} == {r2} < {r3}");
}

#[test]
fn stage3_mir_lower_and_run_is_42() {
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    let mir = klang::mir::lower(&prog);
    let combine = mir.find("combine").expect("combine lowered");
    assert!(combine.instrs.len() >= 5);
    let v = klang::runtime::run(&mir, "combine", &HashMap::new()).expect("runs");
    assert_eq!(v, 42, "fetch_a(20)+fetch_b(22)");
    println!(
        "stage3 run OK: combine() = {v}, instrs={}",
        combine.instrs.len()
    );
}

#[test]
fn stage3_unknown_handle_is_task_leak() {
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    let mir = klang::mir::lower(&prog);
    let err = klang::runtime::run(&mir, "nope", &HashMap::new()).expect_err("unknown entry");
    assert_eq!(err.code, "E-PARSE");
    println!("stage3 unknown-entry diag OK: {}", err.to_json());
}

#[test]
fn stage4_manifest_and_lock() {
    let m = klang::package::Manifest::parse(
        "name = \"demo\"\nversion = \"0.1.0\"\nentry = \"combine\"\n",
    )
    .expect("manifest");
    assert_eq!(m.name, "demo");
    assert_eq!(m.entry, "combine");
    let locks = klang::package::lock_for(&[("main.klang".to_string(), MILESTONE.to_string())]);
    assert_eq!(locks.len(), 1);
    assert_ne!(locks[0].hash, 0);
    // Same content => same hash; different content => different hash.
    let again = klang::package::lock_for(&[("main.klang".to_string(), MILESTONE.to_string())]);
    assert_eq!(locks[0].hash, again[0].hash);
    assert!(klang::package::Command::parse("build").is_some());
    assert!(klang::package::Command::parse("nope").is_none());
    println!("stage4 OK: lock hash {}", locks[0].hash);
}

#[test]
fn stage5_derive_is_append_only_with_origin() {
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    let fn_id = prog.functions[2].id.clone();
    let mut reg = klang::derive::DeriveRegistry::new();
    assert!(reg.is_empty());
    let d0 = reg.expand(&fn_id, "Debug", "CombineDebug", 0).clone();
    let d1 = reg.expand(&fn_id, "Clone", "CombineClone", 1).clone();
    assert_eq!(reg.len(), 2);
    assert!(d0.id.starts_with(&fn_id));
    assert!(d1.id.starts_with(&fn_id));
    assert_ne!(d0.id, d1.id);
    assert_eq!(d0.origin.generated_from, fn_id);
    println!("stage5 OK: {} {} distinct", d0.id, d1.id);
}

#[test]
fn stage6_contracts_fixits_lsp_repair() {
    let c = klang::contracts::Contract {
        on_function: "combine".to_string(),
        precondition: "non-empty".to_string(),
        message: "needs task".to_string(),
    };
    assert!(c.check("a").is_ok());
    assert!(c.check("").is_err());
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    let mir = klang::mir::lower(&prog);
    let listing = klang::codegen::emit_listing(&mir);
    assert!(listing.contains("func combine"));
    // Fix-it + LSP from a real diagnostic.
    let bad = "fn fetch_a() -> i32 throws {} fn bad() -> i32 throws async { task_group { let a = spawn fetch_a() return await b } }";
    let mut pb = Parser::new(bad);
    let progb = pb.parse_program().expect("parses");
    let diags = klang::hir::TypedHIR::check(progb).expect_err("leak");
    let diag = diags
        .iter()
        .find(|d| d.code == "E-TASK-CANCEL")
        .expect("leak diag");
    let fixes = klang::contracts::FixIt::for_diagnostic(diag);
    assert!(!fixes.is_empty());
    let lsp = klang::lsp::diagnostic_to_lsp(diag);
    assert!(lsp.contains("E-TASK-CANCEL"));
    let mut n = 0u32;
    assert!(klang::contracts::repair_loop(3, |_| {
        n += 1;
        Ok::<(), String>(())
    })
    .is_ok());
    assert_eq!(n, 1);
    println!("stage6 OK: {} | {lsp}", fixes[0].label);
}

#[test]
fn stage7_ownership_managed_only_and_codegen() {
    use klang::ownership::{check_resource, Mode};
    assert!(check_resource("buf", Mode::Managed).is_ok());
    for m in [Mode::Value, Mode::Owned, Mode::UnsafeFfi] {
        let e = check_resource("buf", m).expect_err("deferred");
        assert_eq!(e.code, "E-OWNERSHIP-MODE");
    }
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    let mir = klang::mir::lower(&prog);
    let listing = klang::codegen::emit_listing(&mir);
    assert!(listing.contains("spawn a = fetch_a"));
    assert!(listing.contains("spawn b = fetch_b"));
    println!("stage7 OK:\n{listing}");
}

#[test]
fn repair_loop_runs_exactly_max_iters() {
    // Failing closure: exactly `max_iters` attempts, last error returned.
    let mut n = 0u32;
    let err = klang::contracts::repair_loop(3, |_| {
        n += 1;
        Err::<(), String>(format!("e{n}"))
    })
    .expect_err("must fail");
    assert_eq!(n, 3, "exactly max_iters attempts");
    assert_eq!(err, "e3", "last error surfaces");
    // Zero budget: tries nothing.
    let mut m = 0u32;
    assert!(klang::contracts::repair_loop(0, |_| {
        m += 1;
        Err::<(), String>("x".to_string())
    })
    .is_ok());
    assert_eq!(m, 0);
    println!("repair-loop OK");
}
