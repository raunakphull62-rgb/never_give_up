//! Klang full-foundation demo binary.
//!
//! Stage 1 gates first (prints real structural path values and asserts the
//! hard-gate properties in code, panics on failure), then Stages 2-7
//! foundation demo through the same milestone program.

use std::collections::HashMap;

use klang::ast::{Program, Stmt};
use klang::hir::TypedHIR;
use klang::parser::Parser;

pub const MILESTONE: &str = r#"
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

pub const NESTED: &str = r#"
fn outer() -> i32 {
    task_group {
        let a = spawn fetch_a()
        task_group {
            let b = spawn fetch_b()
            return await b
        }
        return await a
    }
}
"#;

pub const LEAK: &str = r#"
fn fetch_a() -> i32 throws {
}

fn bad() -> i32 throws async {
    task_group {
        let a = spawn fetch_a()
        return await b
    }
}
"#;

fn stmt_paths(program: &Program, fn_name: &str) -> Vec<String> {
    let f = program
        .functions
        .iter()
        .find(|f| f.name == fn_name)
        .expect("function present");
    f.body.stmts.iter().map(|s| s.id().display()).collect()
}

fn main() {
    // Run the whole CLI on a deep-stack worker: the front end and checker
    // recurse over the AST, so a large generated file (e.g. `1+1+...+1` with
    // hundreds of operands) would otherwise overflow the default 8 MiB main
    // stack and abort the process instead of reporting a diagnostic. See
    // `klang::with_deep_stack` / `klang::DEEP_STACK_BYTES`.
    klang::with_deep_stack(real_main);
}

fn real_main() {
    // CLI: `cargo run -- <cmd> <file.klang> [entry]`
    //   check <file>        parse + type-check, print diagnostics JSON
    //   fmt <file> [--write] print canonical source (or rewrite in place)
    //   run <file> [entry]  parse + check + lower + run (default entry `main`)
    //   build <file>        parse + check + print MIR listing
    //   repair <file> [--max-iters N] [--scope function|file] [--dry-run]
    //     repair-mechanism inspector: --dry-run prints the planned prompt.
    //     Live model-calling was removed in Phase 4; harnesses drive the
    //     loop via `klang mcp` (klang_check, klang_scope_plan).
    // Backcompat: `cargo run -- <file.klang> [entry]` == `run`.
    // No args = full gate demo below.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("klang {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.len() > 1 {
        run_file_mode(&args[1..]);
        return;
    }
    // ---- Gate 1: two top-level sibling functions have distinct paths ----
    let mut p = Parser::new(MILESTONE);
    let program = match p.parse_program() {
        Ok(prog) => prog,
        Err(d) => {
            eprintln!("parse error: {d}");
            eprintln!("{}", d.to_json());
            std::process::exit(1);
        }
    };
    assert!(program.functions.len() >= 3, "milestone has 3 functions");
    let fa = &program.functions[0];
    let fb = &program.functions[1];
    let comb = &program.functions[2];
    println!("top-level sibling fetch_a path: {}", fa.id);
    println!("top-level sibling fetch_b path: {}", fb.id);
    println!("top-level sibling combine path: {}", comb.id);
    assert_ne!(fa.id, fb.id, "sibling functions must differ");
    assert_ne!(fa.id.path, fb.id.path, "sibling paths must differ");

    // Every function body block is contained in its function.
    for f in &program.functions {
        println!("function {} body block path: {}", f.name, f.body.id);
        assert!(
            f.body.id.starts_with(&f.id),
            "block {} must extend function {}",
            f.body.id,
            f.id
        );
    }

    // ---- Gate 2: two statements inside the same task_group ----
    let tg_stmt = comb
        .body
        .stmts
        .iter()
        .find_map(|s| match s {
            Stmt::TaskGroup(g) => Some(g),
            _ => None,
        })
        .expect("combine has a task_group");
    println!("task_group path inside combine: {}", tg_stmt.id);
    assert!(
        tg_stmt.id.starts_with(&comb.id),
        "task_group must extend enclosing function"
    );
    assert!(
        tg_stmt.body.id.starts_with(&tg_stmt.id),
        "group body block must extend the group"
    );
    let inner: Vec<String> = tg_stmt
        .body
        .stmts
        .iter()
        .map(|s| s.id().display())
        .collect();
    for (i, s) in tg_stmt.body.stmts.iter().enumerate() {
        println!("task_group stmt[{i}] path: {}", s.id());
        assert!(
            s.id().starts_with(&tg_stmt.id),
            "group child must extend the group path"
        );
        assert!(
            s.id().starts_with(&tg_stmt.body.id),
            "group child must extend the body block path"
        );
    }
    assert!(inner.len() >= 2, "group has at least 2 statements");
    assert_ne!(inner[0], inner[1], "sibling stmts in one group must differ");
    let _ = stmt_paths(&program, "combine");

    // ---- Gate 3: nested task_group inside a function ----
    let mut pn = Parser::new(NESTED);
    let nested = pn.parse_program().expect("nested demo parses");
    let outer = &nested.functions[0];
    let outer_group = match &outer.body.stmts[0] {
        Stmt::TaskGroup(g) => g,
        other => panic!("expected outer task_group, got {other:?}"),
    };
    println!("outer function path: {}", outer.id);
    println!("outer task_group path: {}", outer_group.id);
    assert!(outer_group.id.starts_with(&outer.id));
    let inner_group = outer_group
        .body
        .stmts
        .iter()
        .find_map(|s| match s {
            Stmt::TaskGroup(g) => Some(g),
            _ => None,
        })
        .expect("nested demo has inner task_group");
    println!("inner (nested) task_group path: {}", inner_group.id);
    assert!(inner_group.id.starts_with(&outer_group.id));
    assert!(inner_group.id.starts_with(&outer.id));
    assert_ne!(outer_group.id, inner_group.id);

    // ---- Gate 4: milestone parses + effect-checks clean ----
    match TypedHIR::check(program.clone()) {
        Ok(_) => println!("milestone effect check: OK (0 diagnostics)"),
        Err(diags) => {
            for d in &diags {
                println!("{}", d.to_json());
            }
            panic!("milestone should check clean, got {}", diags.len());
        }
    }

    // ---- Gate 5: leak demo emits the scope-violation diagnostic ----
    let mut pl = Parser::new(LEAK);
    let leak_prog = pl.parse_program().expect("leak demo parses");
    match TypedHIR::check(leak_prog) {
        Ok(_) => panic!("leak demo should NOT check clean"),
        Err(diags) => {
            println!("leak demo diagnostics (expected):");
            for d in &diags {
                println!("{}", d.to_json());
            }
            assert!(
                diags.iter().any(|d| d.code == "E-TASK-CANCEL"),
                "leak must emit E-TASK-CANCEL"
            );
        }
    }

    println!("all stage-1 gates hold");

    // ---- Foundation (Stages 2-7) through the same milestone ----
    // Stage 2: incremental DB with early cutoff.
    let mut db = klang::db::Database::new();
    db.set_file("main.klang", MILESTONE);
    let rev1 = db.revision();
    assert!(db.check("main.klang").is_ok(), "db check clean");
    let runs_after_first = db.parse_runs();
    assert!(db.check("main.klang").is_ok(), "db re-check clean");
    assert_eq!(
        db.parse_runs(),
        runs_after_first,
        "unchanged content must hit memo (early cutoff)"
    );
    println!("stage-2 incremental: rev={rev1} parse_runs={runs_after_first} (memo hit OK)");

    // Stage 3: MIR lowering + single-threaded run (20 + 22 = 42).
    let mut p2 = Parser::new(MILESTONE);
    let prog2 = p2.parse_program().expect("parses");
    let mir = klang::mir::lower(&prog2);
    let combine_mir = mir.find("combine").expect("combine lowered");
    println!(
        "stage-3 mir instrs for combine: {}",
        combine_mir.instrs.len()
    );
    assert!(
        combine_mir.instrs.len() >= 5,
        "enter/spawns/await/add/return lowered"
    );
    let result = klang::runtime::run(&mir, "combine", &HashMap::new()).expect("runs");
    println!("stage-3 run combine() = {result}");
    assert_eq!(result, 42, "fetch_a(20) + fetch_b(22)");

    // Stage 4: manifest + lockfile + commands.
    let manifest = klang::package::Manifest::parse(
        "name = \"demo\"\nversion = \"0.1.0\"\nentry = \"combine\"\n",
    )
    .expect("manifest parses");
    println!(
        "stage-4 manifest: {} {} entry={}",
        manifest.name, manifest.version, manifest.entry
    );
    assert_eq!(manifest.entry, "combine");
    let locks = klang::package::lock_for(&[("main.klang".to_string(), MILESTONE.to_string())]);
    println!("stage-4 lock hash for main.klang: {}", locks[0].hash);
    assert!(klang::package::Command::parse("build").is_some());

    // Stage 5: append-only derive with origin chain.
    let mut reg = klang::derive::DeriveRegistry::new();
    let fn_id = &prog2.functions[2].id;
    let d0 = reg.expand(fn_id, "Debug", "CombineDebug", 0).clone();
    println!(
        "stage-5 derived {} @{} from {}",
        d0.name, d0.id, d0.origin.generated_from
    );
    assert!(d0.id.starts_with(fn_id), "derived extends source");
    assert_eq!(reg.len(), 1);

    // Stage 6: contracts + fix-its + LSP + bounded repair.
    let contract = klang::contracts::Contract {
        on_function: "combine".to_string(),
        precondition: "non-empty".to_string(),
        message: "combine needs a task".to_string(),
    };
    assert!(contract.check("a").is_ok());
    assert!(contract.check("").is_err(), "empty violates non-empty");
    let mut pl2 = Parser::new(LEAK);
    let leak2 = pl2.parse_program().expect("parses");
    let leak_diags = TypedHIR::check(leak2).expect_err("leak fails");
    let fixes = klang::contracts::FixIt::for_diagnostic(&leak_diags[0]);
    println!(
        "stage-6 fix-it: {} -> {}",
        fixes[0].for_code, fixes[0].label
    );
    println!(
        "stage-6 lsp: {}",
        klang::lsp::diagnostic_to_lsp(&leak_diags[0])
    );
    let mut attempts = 0u32;
    assert!(klang::contracts::repair_loop(3, |_| {
        attempts += 1;
        Ok::<(), String>(())
    })
    .is_ok());
    assert_eq!(attempts, 1);

    // Stage 7: ownership (managed only) + codegen listing.
    assert!(klang::ownership::check_resource("buf", klang::ownership::Mode::Managed).is_ok());
    assert!(klang::ownership::check_resource("buf", klang::ownership::Mode::Owned).is_err());
    let listing = klang::codegen::emit_listing(&mir);
    println!("stage-7 codegen listing:\n{listing}");
    assert!(listing.contains("func combine"), "listing has combine");

    println!("all klang foundation stages hold");

    // ---- Real language layer (v0.2): literals, args, if/else, print ----
    let lang_src = r#"
fn add(a: i32, b: i32) -> i32 {
    return a + b
}
fn max(a: i32, b: i32) -> i32 {
    if a < b {
        return b
    } else {
        return a
    }
}
fn main() -> i32 {
    let x = 2 + 3 * 4
    print(x)
    print(max(add(20, 2), 21))
    if x == 14 {
        return add(40, 2)
    } else {
        return 0
    }
}
"#;
    let mut pl = Parser::new(lang_src);
    let lang_prog = pl.parse_program().expect("lang demo parses");
    assert!(
        TypedHIR::check(lang_prog.clone()).is_ok(),
        "lang demo checks clean"
    );
    let lang_mir = klang::mir::lower(&lang_prog);
    let (lang_v, lang_out) =
        klang::runtime::run_with_output(&lang_mir, "main", &[], &HashMap::new()).expect("runs");
    println!("lang demo main() = {lang_v}, printed={lang_out:?}");
    assert_eq!(lang_v, 42, "lang demo computes 42");
    assert_eq!(lang_out, vec!["14".to_string(), "22".to_string()]);
    println!("all klang language stages hold");

    // ---- Real language layer (v0.3): loops, arrays, structs, floats ----
    let real_src = r#"
struct Point { x: i32, y: i32 }
fn sum_to(n: i32) -> i32 {
    let total = 0
    for i in 0..n {
        total = total + i
    }
    return total
}
fn main() -> i32 {
    let total = 0
    let i = 1
    while i <= 10 {
        total = total + i
        i = i + 1
    }
    let nums = [20, 1, 21]
    push(nums, 0)
    let s = 0
    for x in nums {
        s = s + x
    }
    let p = Point { x: 19, y: 23 }
    let f = 0.5 * 4.0 + 40.0
    print("sum=" + str(total))
    if s == 42 && f == 42.0 && p.x + p.y == 42 && sum_to(10) == 45 {
        return total - 13
    } else {
        return 0
    }
}
"#;
    let mut pr = Parser::new(real_src);
    let real_prog = pr.parse_program().expect("real demo parses");
    assert!(
        TypedHIR::check(real_prog.clone()).is_ok(),
        "real demo checks clean"
    );
    let real_mir = klang::mir::lower(&real_prog);
    let (real_v, real_out) =
        klang::runtime::run_with_output(&real_mir, "main", &[], &HashMap::new()).expect("runs");
    println!("real demo main() = {real_v}, printed={real_out:?}");
    assert_eq!(real_v, 42, "1+..+10=55, 55-13=42");
    assert_eq!(real_out, vec!["sum=55".to_string()]);
    println!("all klang real-language stages hold");

    // ---- Full language (v0.4): else-if, methods, maps, threads ----
    let full_src = r#"
fn grade(s: i32) -> i32 {
    if s >= 90 {
        return 4
    } else if s >= 80 {
        return 3
    } else if s >= 70 {
        return 2
    } else {
        return 0
    }
}
fn fetch(n: i32) -> i32 {
    return n * 2
}
fn main() -> i32 async {
    let user = {"name": "klang", "v": 40}
    user["v"] = user["v"] + 2
    let words = "hello world".split(" ")
    let total = 0
    task_group {
        let a = spawn fetch(20)
        let b = spawn fetch(1)
        total = await a + await b
    }
    if user["v"] == 42 && words.join(" ").upper() == "HELLO WORLD" && grade(85) == 3 {
        return total
    } else {
        return 0
    }
}
"#;
    let mut pf = Parser::new(full_src);
    let full_prog = pf.parse_program().expect("full demo parses");
    assert!(
        TypedHIR::check(full_prog.clone()).is_ok(),
        "full demo checks clean"
    );
    let full_mir = klang::mir::lower(&full_prog);
    let (full_v, full_out) =
        klang::runtime::run_with_output(&full_mir, "main", &[], &HashMap::new()).expect("runs");
    println!("full demo main() = {full_v}, printed={full_out:?}");
    assert_eq!(full_v, 42, "threads + maps + methods = 42");
    println!("all klang full-language stages hold");
}

/// File CLI with `import` support: check / fmt / run / build.
fn run_file_mode(args: &[String]) {
    use std::collections::HashMap;
    // Split leading subcommand from file args. Backcompat: a bare `<file>`
    // first arg means `run <file> [entry]`.
    let (cmd, rest) = match args.first().map(|s| s.as_str()) {
        Some("check") | Some("check-v2") | Some("fmt") | Some("run") | Some("build")
        | Some("repair") | Some("mcp") => (args[0].as_str(), &args[1..]),
        _ => ("run", args),
    };
    if cmd == "mcp" {
        run_mcp_mode();
        return;
    }
    // Explicit v2 mode: `check --lang v2 <file>` or `check-v2 <file>`.
    // v1 files never silently use v2 semantics: the mode is always chosen
    // by the caller, never inferred.
    if cmd == "check-v2" {
        if rest.is_empty() {
            eprintln!("usage: check-v2 <file.v2>");
            std::process::exit(2);
        }
        run_v2_check_mode(&rest[0]);
        return;
    }
    if cmd == "check" {
        let (is_v2, files) = split_check_lang(rest);
        if is_v2 {
            if files.is_empty() {
                eprintln!("usage: check --lang v2 <file.v2>");
                std::process::exit(2);
            }
            run_v2_check_mode(&files[0]);
            return;
        }
        if files.len() != rest.len() {
            // Only `--lang v1` (or similar) flags were present: same as a
            // plain check on the file. Recurse once on flag-free args.
            if files.is_empty() {
                eprintln!(
                    "usage: <check|fmt|run|build|repair|mcp> <file.klang> [entry] [--backend-jit|--write]"
                );
                std::process::exit(2);
            }
            let mut owned = vec!["check".to_string()];
            owned.extend(files);
            run_file_mode(&owned);
            return;
        }
    }
    if rest.is_empty() {
        eprintln!(
            "usage: <check|fmt|run|build|repair|mcp> <file.klang> [entry] [--backend-jit|--write]"
        );
        eprintln!("       check --lang v2 <file.v2> | check-v2 <file.v2>");
        std::process::exit(2);
    }
    let path = &rest[0];
    if cmd == "repair" {
        run_repair_mode(path, &rest[1..]);
        return;
    }
    if cmd == "fmt" {
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        });
        let mut p = Parser::new_with_file(&src, path);
        let prog = match p.parse_program() {
            Ok(prog) => prog,
            Err(d) => {
                println!("parse: FAIL");
                println!("{}", d.to_json());
                std::process::exit(1);
            }
        };
        let out = klang::fmt::fmt_program(&prog);
        if rest.iter().any(|a| a == "--write") {
            std::fs::write(path, &out).unwrap_or_else(|e| {
                eprintln!("cannot write {path}: {e}");
                std::process::exit(1);
            });
            println!("fmt: wrote {path}");
        } else {
            print!("{out}");
        }
        return;
    }
    let entry = rest
        .get(1)
        .filter(|s| !s.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "main".to_string());
    let use_jit = rest.iter().any(|a| a == "--backend-jit");
    let prog = match load_with_imports(path) {
        Ok(prog) => {
            println!(
                "parse: OK ({} functions, {} structs, {} enums)",
                prog.functions.len(),
                prog.structs.len(),
                prog.enums.len()
            );
            for f in &prog.functions {
                println!("  fn {} @{} effects={:?}", f.name, f.id, f.effects);
            }
            prog
        }
        Err(d) => {
            println!("parse: FAIL");
            println!("{d}");
            std::process::exit(1);
        }
    };
    match TypedHIR::check_with_file(prog.clone(), path) {
        Ok(_) => println!("check: OK (0 diagnostics)"),
        Err(diags) => {
            println!("check: FAIL ({} diagnostics)", diags.len());
            for d in &diags {
                println!("{}", d.to_json());
            }
            std::process::exit(1);
        }
    }
    if cmd == "check" {
        return;
    }
    let mir = klang::mir::lower(&prog);
    println!("mir:\n{}", klang::codegen::emit_listing(&mir));
    if cmd == "build" {
        if use_jit {
            println!("note: --backend-jit only affects `run`; `build` stops at MIR");
        }
        return;
    }
    if use_jit {
        match klang::jit::run_jit(&mir, &entry, &[]) {
            Ok((v, out)) => {
                for line in &out {
                    println!("print: {line}");
                }
                println!("run {entry}() = {v} [jit]");
            }
            Err(e) => {
                println!("run: JIT-FAIL");
                println!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    match klang::runtime::run_with_output(&mir, &entry, &[], &HashMap::new()) {
        Ok((v, out)) => {
            for line in &out {
                println!("print: {line}");
            }
            println!("run {entry}() = {v}");
        }
        Err(d) => {
            println!("run: FAIL");
            println!("{}", d.to_json());
            std::process::exit(1);
        }
    }
}

/// Load a file plus its transitive `import`s, prefixing each file's `NodeId`
/// paths so identity stays unique and parent-prefixed after merging.
fn load_with_imports(entry: &str) -> Result<klang::ast::Program, String> {
    use std::collections::{HashSet, VecDeque};
    let mut merged = klang::ast::Program {
        mods: vec![],
        enums: vec![],
        structs: vec![],
        imports: vec![],
        functions: vec![],
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::from([entry.to_string()]);
    let mut file_idx: u32 = 0;
    let mut fn_names: HashSet<String> = HashSet::new();
    while let Some(path) = queue.pop_front() {
        let canon = std::path::Path::new(&path)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.clone());
        if !seen.insert(canon) {
            continue;
        }
        let src = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "cannot read {}: {e}",
                klang::diagnostics::sanitize_for_terminal(&path)
            )
        })?;
        println!(
            "file: {} ({} bytes)",
            klang::diagnostics::sanitize_for_terminal(&path),
            src.len()
        );
        let mut p = Parser::new_with_file(&src, &path);
        let prog = p.parse_program().map_err(|d| d.to_json())?;
        let dir = std::path::Path::new(&path)
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        for imp in &prog.imports {
            // Root confinement: reject absolute imports and `..` escapes so
            // untrusted `.klang` files cannot pull in `../../…` or absolute
            // paths. Plain relative imports (`mylib.klang`, `./x.klang`)
            // keep working. Single predicate shared with the repair
            // model-output screen (`repair::is_unsafe_import_path`).
            if klang::repair::is_unsafe_import_path(imp) {
                return Err(format!(
                    "unsafe import `{}` from `{path}`",
                    klang::diagnostics::sanitize_for_terminal(imp)
                ));
            }
            queue.push_back(dir.join(imp).to_string_lossy().to_string());
        }
        let prefixed = prog.with_file_prefix(file_idx);
        file_idx += 1;
        merged.mods.extend(prefixed.mods);
        merged.enums.extend(prefixed.enums);
        merged.structs.extend(prefixed.structs);
        for f in prefixed.functions {
            if !fn_names.insert(f.name.clone()) {
                return Err(format!("duplicate function `{}`", f.name));
            }
            merged.functions.push(f);
        }
    }
    Ok(merged)
}

/// `klang repair <file.klang> [--max-iters N] [--scope function|file] [--dry-run]`
///
/// Phase 4: Klang no longer calls a model directly — the repair *loop*
/// (parse -> check -> prompt -> splice -> re-check) is driven by the
/// harness, which owns the model connection. What remains here is the
/// mechanism's observable surface: `--dry-run` prints the exact planned
/// first prompt (scope + diagnostics JSON) a harness would send, and a
/// clean file still exits 0. Anything else errors with a pointer to
/// `klang mcp`, which exposes the same mechanism as tools.
fn run_repair_mode(path: &str, rest: &[String]) {
    use klang::repair::{RepairCliOverrides, RepairConfig, RepairFileConfig, Scope};
    let flag_val = |names: &[&str]| -> Option<String> {
        let mut it = rest.iter().peekable();
        while let Some(a) = it.next() {
            for n in names {
                if a == n {
                    match it.next() {
                        Some(v) if !v.starts_with("--") => return Some(v.clone()),
                        _ => {
                            eprintln!("repair: {n} needs a value");
                            std::process::exit(2);
                        }
                    }
                } else if let Some(v) = a.strip_prefix(&format!("{n}=")) {
                    if v.is_empty() {
                        eprintln!("repair: {n} needs a value");
                        std::process::exit(2);
                    }
                    return Some(v.to_string());
                }
            }
        }
        None
    };
    let has = |n: &str| rest.iter().any(|a| a == n);
    if has("--help") || has("-h") {
        println!("usage: repair <file.klang> [--max-iters N] [--scope function|file] [--dry-run]");
        println!("  --max-iters N   attempt budget 1..10 (default 5)");
        println!("  --scope s       function (default) or file");
        println!("  --dry-run       print the planned first prompt, make no model calls");
        println!("Klang no longer calls a model directly (Phase 4): the repair loop is");
        println!("driven by your harness via `klang mcp` (klang_check, klang_scope_plan).");
        return;
    }
    let max_iters = flag_val(&["--max-iters"]).map(|v| {
        v.parse::<u32>().unwrap_or_else(|_| {
            eprintln!("repair: bad --max-iters `{v}`");
            std::process::exit(2);
        })
    });
    let scope = flag_val(&["--scope"]).map(|v| {
        Scope::parse(&v).unwrap_or_else(|| {
            eprintln!("repair: bad --scope `{v}` (want function|file)");
            std::process::exit(2);
        })
    });
    let dry_run = has("--dry-run");
    let overrides = RepairCliOverrides { max_iters, scope };
    let toml_path = target_dir(path).join("klang.toml");
    let toml_text = std::fs::read_to_string(&toml_path).unwrap_or_default();
    let file_cfg = RepairFileConfig::parse_toml(&toml_text);
    let cfg = RepairConfig::resolve(&overrides, &file_cfg);
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });
    // No backend exists in-tree: the only model-free operation is the
    // dry-run plan. MockBackend is test-only on purpose.
    let backend = klang::repair::MockBackend::new(vec![]);
    let outcome = klang::repair::run_repair(&src, path, &cfg, &backend, true);
    if outcome.attempts.is_empty() {
        println!("check: OK (0 diagnostics), nothing to repair");
        return;
    }
    if dry_run {
        let a = &outcome.attempts[0];
        println!("repair: DRY-RUN (no model calls)");
        println!("scope: {}", a.target);
        println!("--- system prompt ---\n{}", a.prompt_system);
        println!("--- user prompt ---\n{}", a.prompt_user);
        for d in &outcome.final_diagnostics {
            println!("{}", d.to_json());
        }
        return;
    }
    eprintln!("repair: Klang no longer calls a model directly (Phase 4).");
    eprintln!("repair: drive the loop from your harness via `klang mcp`, or inspect the planned prompt with `klang repair {path} --dry-run`.");
    std::process::exit(2);
}

/// `klang mcp`: serve the verification engine as MCP tools on stdio.
///
/// One JSON-RPC message per line in, one per line out (responses only;
/// notifications get silence). Each request is panic-isolated so a
/// hostile input yields an error, never a dead server. Runs inside the
/// CLI's deep-stack worker (see `main`), like every other subcommand.
fn run_mcp_mode() {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let resp = std::panic::catch_unwind(|| klang::mcp::handle_request(&line))
            .unwrap_or_else(|_| Some(klang::mcp::internal_error()));
        if let Some(r) = resp {
            if writeln!(stdout, "{r}").is_err() {
                break;
            }
            if stdout.flush().is_err() {
                break;
            }
        }
    }
}

/// Directory containing `path` (for `klang.toml` lookup).
fn target_dir(path: &str) -> std::path::PathBuf {
    std::path::Path::new(path)
        .parent()
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// `check --lang v2 <file>`: split `(is_v2, files)` out of `rest`.
///
/// Only `v2` switches modes; `--lang v1` (or no flag) stays on the v1
/// path with flags stripped. Unknown `--lang` values are ignored here and
/// surface as file errors downstream, never as silent mode switches.
fn split_check_lang(rest: &[String]) -> (bool, Vec<String>) {
    let mut lang: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if a == "--lang" {
            match it.next() {
                Some(v) => lang = Some(v.clone()),
                None => {
                    eprintln!("check: --lang needs a value (want v1|v2)");
                    std::process::exit(2);
                }
            }
        } else if let Some(v) = a.strip_prefix("--lang=") {
            lang = Some(v.to_string());
        } else {
            files.push(a.clone());
        }
    }
    (lang.as_deref() == Some("v2"), files)
}

/// `klang check-v2 <file>` / `klang check --lang v2 <file>`: run the
/// shared [`klang::mcp::v2_check_source`] front end and print the same
/// `Diagnostic::to_json()` objects the MCP tool embeds.
fn run_v2_check_mode(path: &str) {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });
    match klang::with_deep_stack(move || klang::mcp::v2_check_source(&src)) {
        diags if diags.is_empty() => println!("check: OK (0 diagnostics)"),
        diags => {
            println!("check: FAIL ({} diagnostics)", diags.len());
            for d in &diags {
                println!("{}", d.to_json());
            }
            std::process::exit(1);
        }
    }
}
