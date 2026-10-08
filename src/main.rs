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
        klang::lsp::diagnostic_to_lsp_json(&leak_diags[0], LEAK)
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
        Some("check") | Some("check-v2") | Some("fmt") | Some("run") | Some("run-v2") | Some("build")
        | Some("repair") | Some("mcp") | Some("lsp") | Some("publish") | Some("add") | Some("fetch")
        | Some("install") | Some("remove") | Some("update") | Some("list") | Some("init")
        | Some("login") | Some("logout") => {
            (args[0].as_str(), &args[1..])
        }
        _ => ("run", args),
    };
    if cmd == "mcp" {
        run_mcp_mode();
        return;
    }
    if cmd == "lsp" {
        run_lsp_mode();
        return;
    }
    if cmd == "publish"
    || cmd == "add"
    || cmd == "fetch"
    || cmd == "install"
    || cmd == "remove"
    || cmd == "update"
    || cmd == "list"
    || cmd == "init"
    || cmd == "login"
    || cmd == "logout"
    {
        run_registry_mode(cmd, rest);
        return;
    }
    // `--registry URL` takes a value: strip it (both `--registry URL`
    // and `--registry=URL`) before positional parsing so the URL is
    // never mistaken for a file or entry name. Other commands parse
    // their own flags and are untouched.
    let reg_cli_base = fetch_registry_base(rest);
    let rest_owned: Vec<String>;
    let rest: &[String] = if cmd == "run" || cmd == "check" || cmd == "build" {
        let mut filtered = Vec::new();
        let mut skip_next = false;
        for a in rest {
            if skip_next {
                skip_next = false;
                continue;
            }
            if a == "--registry" {
                skip_next = true;
                continue;
            }
            if a.starts_with("--registry=") {
                continue;
            }
            filtered.push(a.clone());
        }
        rest_owned = filtered;
        &rest_owned
    } else {
        rest
    };
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
    if cmd == "run-v2" {
        if rest.is_empty() {
            eprintln!("usage: run-v2 <file.v2> [entry] [--verbose|-v]");
            std::process::exit(2);
        }
        let verbose = has_verbose_flag(rest);
        let positional: Vec<&String> = rest.iter().filter(|a| !a.starts_with('-')).collect();
        if positional.is_empty() {
            eprintln!("usage: run-v2 <file.v2> [entry] [--verbose|-v]");
            std::process::exit(2);
        }
        let entry = positional
            .get(1)
            .copied()
            .cloned()
            .unwrap_or_else(|| "main".to_string());
        run_v2_mode(positional[0], entry, verbose);
        return;
    }
    // Plain `run` on v2 files: `klang run <file.v2>` and
    // `klang run --lang v2 <file.v2> [entry]` route to the v2 interpreter.
    // This is explicit (extension or flag), never silent: v1 `.klang` files
    // without the flag keep the v1 path below untouched.
    if cmd == "run" {
        let (is_v2_flag, run_files) = split_run_lang(rest);
        // Positional args with all flags (`--verbose`/`-v`, `--quiet`/`-q`,
        // `--backend-jit`) removed, so flags may come before or after the
        // file/entry without being mistaken for them.
        let positional: Vec<&String> = run_files
            .iter()
            .filter(|a| !a.starts_with('-'))
            .collect();
        if is_v2_flag
            || positional
                .first()
                .map(|p| p.ends_with(".v2"))
                .unwrap_or(false)
        {
            if positional.is_empty() {
                eprintln!("usage: run --lang v2 <file.v2> [entry]");
                std::process::exit(2);
            }
            let entry = positional
                .get(1)
                .copied()
                .cloned()
                .unwrap_or_else(|| "main".to_string());
            run_v2_mode(positional[0], entry, has_verbose_flag(rest));
            return;
        }
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
                    "usage: <check|fmt|run|run-v2|build|repair|mcp|lsp|publish|add|fetch|install|remove|update|list|init> <file.klang> [entry] [--backend-jit|--write]"
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
            "usage: <check|fmt|run|run-v2|build|repair|mcp|lsp|publish|add|fetch|install|remove|update|list|init|login|logout> <file.klang> [entry] [--backend-jit|--verbose|-v]"
        );
        eprintln!("       check --lang v2 <file.v2> | check-v2 <file.v2>");
        eprintln!("       run [--verbose|-v] [--offline] [--registry URL] <file.klang> [entry]  (default: only program output; exit code is main's return)");
        eprintln!("       publish [--dir PATH] [--registry URL] [--token TOKEN] (least safe; prefer `klang login` or KLANG_REGISTRY_TOKEN)");
        eprintln!("       add <name[@constraint]> [--dev] [--caret] [--registry URL] [--offline] | fetch [--registry URL] [--offline]");
        eprintln!("       install [name[@constraint]] [--registry URL] [--offline] | remove <name> | update [name] | list [--all] | init [name] [--dir PATH]");
        eprintln!("       login [--registry URL] | logout [--registry URL]");
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
    // RUN-DEFAULT-1: `run` prints only the program's own output by default
    // and the entry return value becomes the process exit code (never
    // printed). `--verbose`/`-v` restores the old full dump (`file:` /
    // `parse:` / `check:` / `mir:`, `print:`-prefixed lines, `run e() = v`).
    // `--quiet`/`-q` is accepted as a no-op alias so old scripts keep
    // working; it changes nothing.
    let verbose = has_verbose_flag(rest);
    // For `run`, flags may come before or after the file/entry
    // (`run --verbose prog.klang` == `run prog.klang main --verbose`).
    // Entry names never start with `-`, so non-flag args are path/entry
    // in order. Other subcommands keep the exact historical indexing.
    let (run_path, run_entry): (&str, String) = if cmd == "run" {
        // Positional scan stops at `--` (everything after is program
        // argv, even entry-looking names).
        let dash_at = rest.iter().position(|a| a == "--");
        let pre: &[String] = match dash_at {
            Some(pos) => &rest[..pos],
            None => rest,
        };
        let plain: Vec<&str> = pre
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(String::as_str)
            .collect();
        if plain.is_empty() {
            eprintln!("usage: run [--verbose|-v] <file.klang> [entry]  (default: only program output)");
            std::process::exit(2);
        }
        (
            plain.first().copied().unwrap_or(rest[0].as_str()),
            plain.get(1).copied().unwrap_or("main").to_string(),
        )
    } else {
        (path.as_str(), entry.clone())
    };
    // Program arguments for the `args()` builtin: everything after a
    // `--` separator, else non-flag positionals after path/entry.
    // `klang run prog.klang -- --help` gives ["--help"]; entry stays
    // `main`. Without `--`, historical indexing holds (second
    // positional is the entry name).
    let prog_argv: Vec<String> = if cmd == "run" {
        if let Some(pos) = rest.iter().position(|a| a == "--") {
            rest[pos + 1..].to_vec()
        } else {
            rest.iter()
                .filter(|a| !a.starts_with('-'))
                .skip(2)
                .cloned()
                .collect()
        }
    } else {
        Vec::new()
    };
    // Errors during `run` go to stderr so stdout carries only program
    // output: `klang run bad.klang 2>/dev/null` prints nothing.
    // `check`/`build` keep the legacy stdout behavior that
    // scripts/verify_docs.py parses.
    let loud_stdout = cmd != "run";
    let dump = verbose || loud_stdout;
    // Registry gate: projects with registry deps fetch first (a no-op
    // for every project without them — no network, no behavior change).
    if cmd == "run" || cmd == "check" || cmd == "build" {
        let offline = rest.iter().any(|a| a == "--offline");
        ensure_registry_deps(run_path, offline, &reg_cli_base);
    }
    let prog = match load_with_imports(run_path, !dump) {
        Ok(prog) => {
            if dump {
                println!(
                    "parse: OK ({} functions, {} structs, {} enums)",
                    prog.functions.len(),
                    prog.structs.len(),
                    prog.enums.len()
                );
                for f in &prog.functions {
                    println!("  fn {} @{} effects={:?}", f.name, f.id, f.effects);
                }
            }
            prog
        }
        Err(d) => {
            if loud_stdout {
                println!("parse: FAIL");
                println!("{d}");
            } else {
                eprintln!("parse: FAIL");
                eprintln!("{d}");
            }
            std::process::exit(1);
        }
    };
    match TypedHIR::check_with_file(prog.clone(), run_path) {
        Ok(_) => {
            if dump {
                println!("check: OK (0 diagnostics)");
            }
            // Phase 1d: non-failing `W-TYPE-NARROW` warnings go to stderr
            // (stdout stays the program's own output under `run`).
            for w in klang::hir::narrow_type_warnings(&prog, run_path) {
                eprintln!("warning: {}", w.to_json());
            }
        }
        Err(diags) => {
            if loud_stdout {
                println!("check: FAIL ({} diagnostics)", diags.len());
                for d in &diags {
                    println!("{}", d.to_json());
                }
            } else {
                eprintln!("check: FAIL ({} diagnostics)", diags.len());
                for d in &diags {
                    eprintln!("{}", d.to_json());
                }
            }
            std::process::exit(1);
        }
    }
    if cmd == "check" {
        return;
    }
    let mir = klang::mir::lower(&prog);
    if dump {
        println!("mir:\n{}", klang::codegen::emit_listing(&mir));
    }
    if cmd == "build" {
        if use_jit {
            println!("note: --backend-jit only affects `run`; `build` stops at MIR");
        }
        return;
    }
    if use_jit {
        // Phase 3b: the JIT is experimental and never the default (only
        // this explicit flag selects it). Say so on stderr every time so
        // a wrapped exit code or a JIT-FAIL is never mistaken for the
        // reference interpreter's verdict.
        eprintln!(
            "warning: --backend-jit is experimental: it does not check division by zero, overflow, or recursion depth yet"
        );
        let omitted = entry_omitted_return(&prog, &run_entry);
        match klang::jit::run_jit(&mir, &run_entry, &[]) {
            Ok((v, out)) => {
                for line in &out {
                    if verbose {
                        println!("print: {line}");
                    } else {
                        println!("{line}");
                    }
                }
                if verbose {
                    println!("run {run_entry}() = {v} [jit]");
                }
                // Exit code is the low 8 bits (OS wrapping): 256 -> 0,
                // -1 -> 255. No clamping. An entry with no declared
                // return type always exits 0 (Phase 3b).
                std::process::exit(if omitted { 0 } else { v as i32 });
            }
            Err(e) => {
                eprintln!("run: JIT-FAIL");
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    match klang::runtime::run_with_argv(&mir, &run_entry, &[], &prog_argv, &HashMap::new()) {
        (Ok(v), out) => {
            for line in &out {
                if verbose {
                    println!("print: {line}");
                } else {
                    println!("{line}");
                }
            }
            // Out-of-range `Int` returns stay loud `E-OVERFLOW`, matching
            // `run_with_output`.
            if let klang::runtime::Value::Int(n) = &v {
                if *n < i32::MIN as i64 || *n > i32::MAX as i64 {
                    let d = klang::diagnostics::Diagnostic::error(
                        "E-OVERFLOW",
                        "integer overflow in `return`: result out of i32 range",
                        "runtime",
                        0,
                        0,
                        "i32 arithmetic never wraps: out-of-range results are errors",
                        &["use smaller operands", "check bounds before operating"],
                        "arithmetic/overflow",
                    );
                    eprintln!("run: FAIL");
                    eprintln!("{}", d.to_json());
                    std::process::exit(1);
                }
            }
            if verbose {
                println!("run {run_entry}() = {}", v.render());
            }
            // An entry with no declared return type always exits 0 (its
            // fall-off value is discarded); a declared type maps as before.
            std::process::exit(exit_code_for_entry(&v, entry_omitted_return(&prog, &run_entry)));
        }
        (Err(d), out) => {
            // HEAVY-TEST-1: flush whatever the program printed before it
            // failed — the diagnostic alone hides how far execution got.
            // Program lines go to stdout (`print:`-prefixed in verbose,
            // raw by default); the diagnostic goes to stderr so
            // `2>/dev/null` shows only program output.
            for line in &out {
                if verbose {
                    println!("print: {line}");
                } else {
                    println!("{line}");
                }
            }
            // Whole-program `exit(code)`: the low 8 bits, like normal
            // returns (256 -> 0, -1 -> 255). Uncatchable in the program,
            // so reaching here with E-EXIT is always a real request.
            if d.is_exit() {
                std::process::exit(d.exit_code() & 0xFF);
            }
            eprintln!("run: FAIL");
            eprintln!("{}", d.to_json());
            std::process::exit(1);
        }
    }
}

/// CLI arg parsing for registry commands (B5).
/// Splits `--flag value` / `--flag=value` from positionals, validates
/// against `value_flags` + `bool_flags`, and rejects any other `--flag`
/// with `unknown option --flag` (non-zero exit). Values are never
/// mistaken for package specs (fixes `install --registry URL` treating
/// the URL as a spec).
struct ParsedArgs {
    values: std::collections::HashMap<String, String>,
    bools: std::collections::HashSet<String>,
    positional: Vec<String>,
}

fn parse_registry_cli(
    cmd: &str,
    rest: &[String],
    value_flags: &[&str],
    bool_flags: &[&str],
) -> ParsedArgs {
    let mut values = std::collections::HashMap::new();
    let mut bools = std::collections::HashSet::new();
    let mut positional = Vec::new();
    // `--help`/`-h` prints usage (not an unknown-option error).
    if rest.iter().any(|a| a == "--help" || a == "-h") {
        print_registry_usage(cmd);
        std::process::exit(0);
    }
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if let Some((flag, inline_val)) = a.split_once('=') {
            if flag.starts_with("--") {
                if value_flags.contains(&flag) {
                    if inline_val.is_empty() {
                        eprintln!("{flag} needs a value");
                        std::process::exit(2);
                    }
                    values.insert(flag.to_string(), inline_val.to_string());
                    i += 1;
                    continue;
                }
                eprintln!("unknown option {flag}");
                std::process::exit(2);
            }
        }
        if a.starts_with("--") {
            if value_flags.contains(&a.as_str()) {
                match rest.get(i + 1) {
                    Some(v) if !v.starts_with("--") => {
                        values.insert(a.clone(), v.clone());
                        i += 2;
                        continue;
                    }
                    _ => {
                        eprintln!("{a} needs a value");
                        std::process::exit(2);
                    }
                }
            } else if bool_flags.contains(&a.as_str()) {
                bools.insert(a.clone());
                i += 1;
                continue;
            } else {
                eprintln!("unknown option {a}");
                std::process::exit(2);
            }
        } else if a.starts_with('-') && a.len() > 1 && !a.starts_with("--") {
            // Single-dash flags are not part of the registry CLI; treat
            // `-h`/`-V` as help/version passthrough, anything else as
            // unknown.
            if a == "-h" {
                print_registry_usage(cmd);
                std::process::exit(0);
            }
            eprintln!("unknown option {a}");
            std::process::exit(2);
        } else {
            positional.push(a.clone());
            i += 1;
        }
    }
    ParsedArgs {
        values,
        bools,
        positional,
    }
}

fn print_registry_usage(cmd: &str) {
    match cmd {
        "publish" => {
            eprintln!("usage: publish [--dir PATH] [--registry URL] [--token TOKEN]");
            eprintln!("  --token is the least safe option (shows up in shell history); prefer `klang login` or KLANG_REGISTRY_TOKEN");
        }
        "add" => {
            eprintln!("usage: add <name[@constraint]...> [--dev] [--caret] [--registry URL] [--offline]");
            eprintln!("  constraint: X.Y.Z, =X.Y.Z, ^X.Y.Z, ~X.Y.Z, >/>=/</<=X.Y.Z, * (default: latest)");
            eprintln!("  one or more names; all resolve together and install all-or-none");
            eprintln!("  --caret writes ^MAJOR.MINOR.PATCH ranges instead of exact pins");
        }
        "install" => {
            eprintln!("usage: install [name[@constraint]] [--registry URL] [--offline]");
        }
        "remove" => {
            eprintln!("usage: remove <name> [--registry URL]");
        }
        "update" => {
            eprintln!("usage: update [name] [--registry URL] [--offline]");
        }
        "list" => {
            eprintln!("usage: list [--all] [--registry URL]");
            eprintln!("  --all shows transitive dependencies indented under what required them");
        }
        "login" => {
            eprintln!("usage: login [--registry URL]");
        }
        "logout" => {
            eprintln!("usage: logout [--registry URL]");
        }
        _ => {
            eprintln!("usage: {cmd} [--registry URL]");
        }
    }
}

/// Resolve the registry base URL for CLI commands (PRD §3, 5 levels).
/// Exits non-zero with `registry must use https` on rejection.
fn resolve_cli_registry(flag: Option<&str>, project_root: Option<&std::path::Path>) -> String {
    match klang::registry::resolve_registry_url(flag, project_root) {
        Ok(url) => url,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// `klang publish|add|fetch|install|remove|update|list|init`: package
/// management (FOUNDATION-3 Part 2B + Phase 1 Core PM).
/// `--registry URL` overrides `KLANG_REGISTRY` which overrides the
/// localhost default; publish auth comes from `KLANG_REGISTRY_TOKEN`
/// (env preferred) or `--token` (overrides env).
///
/// Semantics (PRD § CLI, reconciled with backcompat):
/// - `install [spec]` — vendor without editing klang.toml; no args
///   reproduces from klang.lock/manifest.
/// - `add spec... [--dev] [--caret]` — pin + edit klang.toml + install
///   (keeps the legacy `add name@version` form working; now also
///   accepts ranges and several names at once, resolved together and
///   installed all-or-none; `--caret` writes `^X.Y.Z`).
/// - `remove name` / `update [name]` / `list [--all]` / `init [name]`
/// - `fetch` (legacy alias, kept) and `publish` unchanged.
/// - `login`/`logout` manage `~/.klang/credentials` (owner-only publishing).
fn run_registry_mode(cmd: &str, rest: &[String]) {
    // B5: parse flags first (both `--registry URL` and `--registry=URL`);
    // unknown `--flag` => "unknown option --flag" + exit 2. Values are
    // excluded from positionals so a URL is never treated as a spec.
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    match cmd {
        "publish" => {
            let args = parse_registry_cli(cmd, rest, &["--registry", "--token", "--dir"], &[]);
            let flag_reg = args.values.get("--registry").map(|s| s.as_str());
            let base = resolve_cli_registry(flag_reg, Some(&cwd));
            let dir = args
                .values
                .get("--dir")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| cwd.clone());
            let flag_tok = args.values.get("--token").map(|s| s.as_str());
            let token = match klang::registry::resolve_publish_token(flag_tok, &base) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            };
            if let Err(e) = klang::registry::ensure_token_transport_ok(&base) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            // R3.4: print the registry host so a wrong URL is obvious.
            println!("registry: {}", klang::registry::registry_host(&base));
            match klang_publish(&dir, &base, &token) {
                Ok(line) => println!("{line}"),
                Err(e) => {
                    // C5: never print the token (RegistryError never holds it).
                    eprintln!("publish: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "add" => {
            let args = parse_registry_cli(
                cmd,
                rest,
                &["--registry"],
                &["--dev", "--offline", "--caret"],
            );
            if args.positional.is_empty() {
                eprintln!("usage: add <name[@constraint]...> [--dev] [--caret] [--registry URL] [--offline]");
                eprintln!("  constraint: X.Y.Z, =X.Y.Z, ^X.Y.Z, ~X.Y.Z, >/>=/</<=X.Y.Z, * (default: latest)");
                eprintln!("  one or more names; all resolve together and install all-or-none");
                eprintln!("  --caret writes ^MAJOR.MINOR.PATCH ranges instead of exact pins");
                std::process::exit(2);
            }
            let dev = args.bools.contains("--dev");
            let caret = args.bools.contains("--caret");
            let offline = args.bools.contains("--offline");
            let flag_reg = args.values.get("--registry").map(|s| s.as_str());
            let base = resolve_cli_registry(flag_reg, Some(&cwd));
            if !offline {
                println!("registry: {}", klang::registry::registry_host(&base));
            }
            let result = if args.positional.len() == 1 {
                klang::package_manager::cli::cmd_add_full(
                    &cwd,
                    &base,
                    &args.positional[0],
                    dev,
                    caret,
                    offline,
                )
            } else {
                klang::package_manager::cli::cmd_add_multi(
                    &cwd,
                    &base,
                    &args.positional,
                    dev,
                    caret,
                    offline,
                )
            };
            match result {
                Ok(line) => println!("{line}"),
                Err(e) => {
                    eprintln!("add: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "install" => {
            // `install` never edits klang.toml (PRD): no args reproduces
            // the locked environment; with a spec it vendors that
            // package + transitive deps and updates klang.lock.
            let args = parse_registry_cli(cmd, rest, &["--registry"], &["--offline"]);
            if args.positional.len() > 1 {
                eprintln!("usage: install [name[@constraint]] [--registry URL] [--offline]");
                std::process::exit(2);
            }
            let offline = args.bools.contains("--offline");
            let flag_reg = args.values.get("--registry").map(|s| s.as_str());
            let base = resolve_cli_registry(flag_reg, Some(&cwd));
            if !offline {
                println!("registry: {}", klang::registry::registry_host(&base));
            }
            let spec = args.positional.first().map(|s| s.as_str());
            match klang::package_manager::cli::cmd_install(&cwd, &base, spec, offline) {
                Ok(logs) => {
                    for l in logs {
                        println!("{l}");
                    }
                }
                Err(e) => {
                    eprintln!("install: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "remove" => {
            let args = parse_registry_cli(cmd, rest, &["--registry"], &[]);
            let Some(name) = args.positional.first() else {
                eprintln!("usage: remove <name>");
                std::process::exit(2);
            };
            if args.positional.len() > 1 {
                eprintln!("usage: remove <name>");
                std::process::exit(2);
            }
            match klang::package_manager::cli::cmd_remove(&cwd, name) {
                Ok(line) => println!("{line}"),
                Err(e) => {
                    eprintln!("remove: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "update" => {
            let args = parse_registry_cli(cmd, rest, &["--registry"], &["--offline"]);
            if args.positional.len() > 1 {
                eprintln!("usage: update [name] [--registry URL] [--offline]");
                std::process::exit(2);
            }
            let offline = args.bools.contains("--offline");
            let flag_reg = args.values.get("--registry").map(|s| s.as_str());
            let base = resolve_cli_registry(flag_reg, Some(&cwd));
            if !offline {
                println!("registry: {}", klang::registry::registry_host(&base));
            }
            let name = args.positional.first().map(|s| s.as_str());
            match klang::package_manager::cli::cmd_update(&cwd, &base, name, offline) {
                Ok(logs) => {
                    for l in logs {
                        println!("{l}");
                    }
                }
                Err(e) => {
                    eprintln!("update: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "list" => {
            let args = parse_registry_cli(cmd, rest, &["--registry"], &["--all"]);
            if !args.positional.is_empty() {
                eprintln!("usage: list [--all] [--registry URL]");
                std::process::exit(2);
            }
            let all = args.bools.contains("--all");
            match klang::package_manager::cli::cmd_list_all(&cwd, all) {
                Ok(out) => print!("{out}"),
                Err(e) => {
                    eprintln!("list: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "init" => {
            // `init [name] [--dir PATH]`: scaffold klang.toml + src/ + tests/.
            let args = parse_registry_cli(cmd, rest, &["--dir"], &[]);
            if args.positional.len() > 1 {
                eprintln!("usage: init [name] [--dir PATH]");
                std::process::exit(2);
            }
            let name = args
                .positional
                .first()
                .cloned()
                .unwrap_or_else(|| "my-project".to_string());
            let dir_opt = args.values.get("--dir").map(std::path::PathBuf::from);
            let target = if args.positional.first().is_some() && dir_opt.is_none() {
                // `init my-project` with no --dir creates ./my-project/.
                let sub = cwd.join(&name);
                if sub.exists() {
                    // Name is a project name AND the dir exists: init in place?
                    // Prefer explicitness: if ./name exists as a dir without
                    // klang.toml, use it; otherwise init cwd with that name.
                    if sub.is_dir() && !sub.join("klang.toml").exists() {
                        sub
                    } else {
                        cwd.clone()
                    }
                } else {
                    sub
                }
            } else {
                dir_opt.unwrap_or_else(|| cwd.clone())
            };
            match klang::package_manager::cli::cmd_init(&target, &name) {
                Ok(line) => println!("{line}"),
                Err(e) => {
                    eprintln!("init: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "fetch" => {
            // Legacy alias for `install` with no spec (kept for backcompat).
            let args = parse_registry_cli(cmd, rest, &["--registry"], &["--offline"]);
            if !args.positional.is_empty() {
                eprintln!("usage: fetch [--registry URL] [--offline]");
                std::process::exit(2);
            }
            let offline = args.bools.contains("--offline");
            let flag_reg = args.values.get("--registry").map(|s| s.as_str());
            let base = resolve_cli_registry(flag_reg, Some(&cwd));
            if !offline {
                println!("registry: {}", klang::registry::registry_host(&base));
            }
            match klang::package_manager::cli::cmd_install(&cwd, &base, None, offline) {
                Ok(logs) => {
                    for l in logs {
                        println!("{l}");
                    }
                }
                Err(e) => {
                    eprintln!("fetch: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "login" => {
            let args = parse_registry_cli(cmd, rest, &["--registry"], &[]);
            if !args.positional.is_empty() {
                eprintln!("usage: login [--registry URL]");
                std::process::exit(2);
            }
            let flag_reg = args.values.get("--registry").map(|s| s.as_str());
            let base = resolve_cli_registry(flag_reg, Some(&cwd));
            let token = read_login_token();
            if token.trim().is_empty() {
                eprintln!("login: FAIL\nempty token");
                std::process::exit(1);
            }
            match klang::registry::save_credential(&base, token.trim()) {
                Ok(()) => println!("logged in to {}", klang::registry::registry_host(&base)),
                Err(e) => {
                    eprintln!("login: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        "logout" => {
            let args = parse_registry_cli(cmd, rest, &["--registry"], &[]);
            if !args.positional.is_empty() {
                eprintln!("usage: logout [--registry URL]");
                std::process::exit(2);
            }
            let flag_reg = args.values.get("--registry").map(|s| s.as_str());
            let base = resolve_cli_registry(flag_reg, Some(&cwd));
            match klang::registry::remove_credential(&base) {
                Ok(true) => println!("logged out from {}", klang::registry::registry_host(&base)),
                Ok(false) => println!("not logged in to {}", klang::registry::registry_host(&base)),
                Err(e) => {
                    eprintln!("logout: FAIL\n{e}");
                    std::process::exit(1);
                }
            }
        }
        _ => unreachable!("registry dispatcher"),
    }
}

/// Read the publish token for `klang login` (C1): hidden input (no
/// echo) when interactive, piped stdin for CI. Never echoes the token.
fn read_login_token() -> String {
    use std::io::{IsTerminal, Read};
    let stdin = std::io::stdin();
    // Piped stdin (CI): read everything.
    if !stdin.is_terminal() {
        let mut buf = String::new();
        // Best-effort: read piped token (first line).
        let mut handle = stdin.lock();
        let _ = handle.read_to_string(&mut buf);
        return buf.lines().next().unwrap_or("").trim().to_string();
    }
    // Interactive TTY: disable echo via `stty -echo`, read one line.
    #[cfg(unix)]
    {
        use std::io::BufRead;
        eprint!("token: ");
        use std::io::Write;
        let _ = std::io::stderr().flush();
        // Save stty state (best-effort).
        let saved = std::process::Command::new("stty")
            .arg("-g")
            .stdin(std::process::Stdio::inherit())
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        let _ = std::process::Command::new("stty")
            .arg("-echo")
            .stdin(std::process::Stdio::inherit())
            .status();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        // Restore echo.
        if let Some(state) = saved {
            let _ = std::process::Command::new("stty")
                .arg(state)
                .stdin(std::process::Stdio::inherit())
                .status();
        } else {
            let _ = std::process::Command::new("stty")
                .arg("echo")
                .stdin(std::process::Stdio::inherit())
                .status();
        }
        eprintln!();
        return line.trim().to_string();
    }
    #[cfg(not(unix))]
    {
        use std::io::BufRead;
        eprint!("token: ");
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        return line.trim().to_string();
    }
}

/// Pack the project in `dir` and publish it. Returns the success line.
fn klang_publish(
    dir: &std::path::Path,
    base: &str,
    token: &str,
) -> Result<String, klang::registry::RegistryError> {
    use klang::registry::RegistryError;
    let text = std::fs::read_to_string(dir.join("klang.toml"))
        .map_err(|e| RegistryError::new("io", format!("cannot read klang.toml: {e}")))?;
    let manifest =
        klang::package::Manifest::parse(&text).map_err(|e| RegistryError::new("io", e))?;
    if !klang::registry::valid_pkg_name(&manifest.name) {
        return Err(RegistryError::new(
            "protocol",
            format!("klang.toml name `{}` is not a publishable package name", manifest.name),
        ));
    }
    if !klang::registry::valid_version(&manifest.version) {
        return Err(RegistryError::new(
            "protocol",
            format!("klang.toml version `{}` must be X.Y.Z", manifest.version),
        ));
    }
    let files = klang::registry::collect_package_files(dir)
        .map_err(|e| klang::registry::RegistryError::new("io", e))?;
    let archive = klang::registry::pack_archive(&files)
        .map_err(|e| klang::registry::RegistryError::new("io", e))?;
    let sha = klang::registry::sha256_hex(&archive);
    klang::registry::publish_pkg(base, token, &manifest.name, &manifest.version, &archive)?;
    Ok(format!(
        "published {}@{} ({} files, {} bytes, sha256:{sha})",
        manifest.name,
        manifest.version,
        files.len(),
        archive.len()
    ))
}

/// Registry fetch gate for `run`/`check`/`build`: when the entry file
/// lives in a project whose `klang.toml` declares registry deps, ensure
/// they are vendored + verified first (network unless `--offline`).
/// Projects without registry deps behave exactly as before (no network,
/// no new failure modes). Failures print as `E-IMPORT` JSON.
fn ensure_registry_deps(entry: &str, offline: bool, base: &str) {
    let start = std::path::Path::new(entry);
    let Some(root) = klang::registry::find_project_root(start) else {
        return;
    };
    let text = match std::fs::read_to_string(root.join("klang.toml")) {
        Ok(t) => t,
        Err(_) => return,
    };
    let manifest = match klang::package::Manifest::parse(&text) {
        Ok(m) => m,
        Err(_) => return,
    };
    if !manifest
        .deps
        .iter()
        .chain(manifest.dev_deps.iter())
        .any(|(k, v)| {
            klang::registry::parse_registry_dep(v).is_some()
                || klang::registry::parse_registry_req(v, k).is_some()
        })
    {
        return;
    }
    if let Err(e) = klang::registry::fetch_project(&root, base, offline) {
        let d = klang::diagnostics::Diagnostic::error(
            "E-IMPORT",
            &e.to_string(),
            "import",
            0,
            0,
            "registry dependency resolution failed",
            &["run `klang fetch` online first", "check klang.toml pins and klang.lock hashes"],
            "modules/import",
        );
        eprintln!("parse: FAIL");
        eprintln!("{}", d.to_json());
        std::process::exit(1);
    }
}

/// Registry base URL for dependency fetching during run/check/build
/// (PRD §3, 5 levels: `--registry`, `KLANG_REGISTRY`, project
/// `klang.toml`, global config, compiled-in default).
fn fetch_registry_base(rest: &[String]) -> String {
    let mut flag: Option<String> = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--registry=") {
            if !v.is_empty() {
                flag = Some(v.to_string());
            }
        } else if a == "--registry" {
            if let Some(v) = it.next() {
                if !v.starts_with("--") {
                    flag = Some(v.clone());
                }
            }
        }
    }
    // Project root for level 3: walk up from the entry file (first
    // non-flag arg), if any.
    let entry = rest.iter().find(|a| !a.starts_with('-')).map(|s| s.as_str());
    let root = entry.and_then(|e| klang::registry::find_project_root(std::path::Path::new(e)));
    match klang::registry::resolve_registry_url(flag.as_deref(), root.as_deref()) {
        Ok(url) => url,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Load a file plus its transitive `import`s, prefixing each file's `NodeId`
/// paths so identity stays unique and parent-prefixed after merging.
/// Delegates to the library loader (`klang::imports`), which handles both
/// whole-file `import "x.klang"` merges and selective
/// `import { name } from "x.klang"` merges with cycle-safe BFS and clean
/// `E-*` diagnostics (never a panic).
fn load_with_imports(entry: &str, quiet: bool) -> Result<klang::ast::Program, String> {
    match klang::imports::load_program(entry) {
        Ok(loaded) => {
            // RUN-DEFAULT-1: the `file:` preamble shows only with `--verbose`.
            // The `quiet` arg means "suppress preamble"; callers pass
            // `!verbose` (see run_file_mode).
            if !quiet {
                for f in &loaded.files {
                    println!(
                        "file: {} ({} bytes)",
                        klang::diagnostics::sanitize_for_terminal(&f.path),
                        f.bytes
                    );
                }
            }
            Ok(loaded.program)
        }
        Err(e) => Err(e.to_json()),
    }
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
    // Programs executed via `klang_run` share this process: reserve stdin
    // for the JSON-RPC loop so a `read_line()` call sees EOF ("") instead
    // of stealing protocol bytes off this stream.
    klang::stdlib::io::reserve_stdin_for_mcp();
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

/// `klang lsp`: stdio JSON-RPC language server (FOUNDATION-3 Part 2C).
/// Same stdin reservation as `klang mcp`: protocol owns the stream, so a
/// `read_line()` call in a checked program sees EOF instead of stealing
/// protocol bytes.
fn run_lsp_mode() {
    klang::stdlib::io::reserve_stdin_for_mcp();
    klang::lsp::serve_stdio();
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

/// `run --lang v2 <file>`: split `(is_v2, files)` out of `rest`.
/// Same convention as `check --lang v2`; `--backend-jit` etc. pass through
/// as positional entries for the v1 path and are ignored for v2 routing
/// (v2 has no JIT backend).
fn split_run_lang(rest: &[String]) -> (bool, Vec<String>) {
    let mut lang: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if a == "--lang" {
            match it.next() {
                Some(v) => lang = Some(v.clone()),
                None => {
                    eprintln!("run: --lang needs a value (want v1|v2)");
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

/// RUN-DEFAULT-1: `--verbose`/`-v` restores the old full dump.
/// `--quiet`/`-q` is a no-op alias (accepted, changes nothing) so existing
/// scripts keep working.
fn has_verbose_flag(rest: &[String]) -> bool {
    rest.iter().any(|a| a == "--verbose" || a == "-v")
}

/// Map a v1 return value to a process exit code. `Int` becomes the exit
/// code (the OS keeps the low 8 bits: 256 -> 0, -1 -> 255; never clamped).
/// Any non-`Int` value (including fall-off-the-end `0` and `Float`/`Str`
/// returns) exits 0. Out-of-range `Int` never reaches here (loud
/// `E-OVERFLOW`, exit 1, at the call site).
fn exit_code_for_value(v: &klang::runtime::Value) -> i32 {
    match v {
        klang::runtime::Value::Int(n) => *n as i32,
        _ => 0,
    }
}

/// Phase 3b decision: an entry with NO declared return type (omitted `->`,
/// desugared to `void`) always exits 0 — its fall-off value is discarded
/// for exit purposes. A declared return type (`-> T`, including an
/// explicit `-> void`) keeps today's mapping via [`exit_code_for_value`].
fn exit_code_for_entry(v: &klang::runtime::Value, omitted: bool) -> i32 {
    if omitted {
        0
    } else {
        exit_code_for_value(v)
    }
}

/// True when the named entry declared no return type (omitted `->`).
/// Mirrors MIR entry lookup: top-level functions by plain name, `mod`
/// members by qualified `m::f` name.
fn entry_omitted_return(prog: &klang::ast::Program, entry: &str) -> bool {
    if let Some(f) = prog.functions.iter().find(|f| f.name == entry) {
        return f.return_ty_omitted;
    }
    for m in &prog.mods {
        if let Some(f) = m
            .functions
            .iter()
            .find(|f| format!("{}::{}", m.name, f.name) == entry)
        {
            return f.return_ty_omitted;
        }
    }
    false
}

/// `klang check-v2 <file>` / `klang check --lang v2 <file>`: run the
/// shared [`klang::mcp::v2_check_source`] front end and print the same
/// `Diagnostic::to_json()` objects the MCP tool embeds.
fn run_v2_check_mode(path: &str) {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });
    // Owned for the 'static deep-stack worker.
    let file = path.to_string();
    match klang::with_deep_stack(move || klang::mcp::v2_check_source_with_file(&src, &file)) {
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

/// `klang run-v2 <file.v2> [entry]` (and `klang run <file.v2>` /
/// `klang run --lang v2 <file.v2>`): parse, lower through MIR (pillar 1),
/// then execute with the real v2 interpreter (pillars 2-5).
/// RUN-DEFAULT-1 applies here too: default prints only program output and
/// exits with main's return; `--verbose` restores the parse/mir listing
/// plus the `run entry() = v` line.
fn run_v2_mode(path: &str, entry: String, verbose: bool) {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });
    // Parse v2 program
    let prog = match klang::with_deep_stack(move || klang::parser::v2::parse_v2_program(&src)) {
        Ok(prog) => prog,
        Err(d) => {
            eprintln!("parse: FAIL");
            eprintln!("{}", d.to_json());
            std::process::exit(1);
        }
    };
    if verbose {
        println!(
            "parse: OK ({} schemas, {} echo fns, {} flows, {} functions)",
            prog.schemas.len(),
            prog.echo_fns.len(),
            prog.flows.len(),
            prog.functions.len()
        );
    }
    // Lower v2 program through MIR, invoking echo_lowering (pillar 1).
    // The listing below proves the lowering ran; execution itself uses the
    // real v2 interpreter so dep=/echo/tune semantics are genuine.
    let mir = klang::with_deep_stack({
        let (schemas, echo_fns, echo_bodies, flows, functions) = (
            prog.schemas.clone(),
            prog.echo_fns.clone(),
            prog.echo_bodies.clone(),
            prog.flows.clone(),
            prog.functions.clone(),
        );
        move || {
            klang::mir::v2_lowering::lower_v2_program(
                &schemas,
                &echo_fns,
                &echo_bodies,
                &flows,
                &functions,
            )
        }
    });
    if verbose {
        println!("mir: OK ({} functions)", mir.functions.len());
        for f in &mir.functions {
            println!("  fn {} ({} params, {} instrs)", f.name, f.params.len(), f.instrs.len());
        }
    }
    // Real execution (pillars 2-5, no stubs).
    let prog2 = prog.clone();
    let entry2 = entry.clone();
    let omitted = prog
        .functions
        .iter()
        .find(|f| f.name == entry)
        .map(|f| f.return_ty_omitted)
        .unwrap_or(false);
    match klang::with_deep_stack(move || klang::runtime::v2::run_v2_program_partial(&prog2, &entry2)) {
        (Ok(v), out) => {
            for line in &out {
                if verbose {
                    println!("print: {line}");
                } else {
                    println!("{line}");
                }
            }
            if verbose {
                println!("run {entry}() = {v}");
            }
            // Low 8 bits wrap (256 -> 0); no clamping. An entry with no
            // declared return type always exits 0 (Phase 3b).
            std::process::exit(if omitted { 0 } else { v });
        }
        (Err(d), out) => {
            // HEAVY-TEST-1 (v2 mirror): program lines to stdout
            // (`print:`-prefixed in verbose); the diagnostic to stderr.
            for line in &out {
                if verbose {
                    println!("print: {line}");
                } else {
                    println!("{line}");
                }
            }
            eprintln!("run: FAIL");
            eprintln!("{}", d.to_json());
            std::process::exit(1);
        }
    }
}
