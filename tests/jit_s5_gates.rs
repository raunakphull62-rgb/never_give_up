//! Wave1 S5 differential gates: the JIT agrees with the interpreter or
//! refuses loudly.
//!
//! - Targeted gates pin every S5 fix against the v1 interpreter
//!   (in-process, both sides): div/mod-by-zero, `MIN / -1`, `MIN % -1`,
//!   negation overflow, add/sub/mul overflow, print-before-failure
//!   flushing, the 1024-frame depth guard (1023 runs, 1024 fails), and
//!   the D3 narrow boundaries (param, return, annotated copy).
//! - `jit_diff_random_arith` generates a few hundred small arithmetic
//!   programs from a fixed seed (in-process LCG, no new crates) and
//!   asserts identical stdout, exit code and error code on both
//!   backends. Programs the JIT rejects as unsupported are skipped and
//!   counted (then reported); any value/code/message mismatch fails.
//!
//! Every test uses its own unique temp dir (pid + atomic counter).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use klang::parser::Parser;

static SCRATCH_CTR: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "klang-s5-{tag}-{}-{}",
        std::process::id(),
        SCRATCH_CTR.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lower_checked(src: &str) -> klang::mir::MirModule {
    try_lower(src).unwrap_or_else(|e| panic!("{e}:\n{src}"))
}

/// Parse + check + lower without panicking (the fuzz loop reports a bad
/// program as a mismatch with its source instead of aborting).
fn try_lower(src: &str) -> Result<klang::mir::MirModule, String> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().map_err(|e| format!("generator emitted unparsable code ({e})"))?;
    klang::hir::TypedHIR::check(prog.clone())
        .map_err(|ds| format!("generator emitted non-checking code ({} diagnostics)", ds.len()))?;
    Ok(klang::mir::lower(&prog))
}

/// Interpreter verdict: `(Ok(value) | Err((code, message)), stdout)`.
/// Value-level (`run_with_argv`, no `i32` truncation) so full-range
/// `i64` results compare exactly; the exit byte derives from the value.
fn run_interp(mir: &klang::mir::MirModule) -> (Result<i64, (String, String)>, Vec<String>) {
    let (r, out) = klang::runtime::run_with_argv(mir, "main", &[], &[], &HashMap::new());
    let v = match r {
        Ok(klang::runtime::Value::Int(n)) => Ok(n),
        Ok(other) => panic!("int-only gen produced non-Int: {}", other.render()),
        Err(d) => Err((d.code.clone(), d.message.clone())),
    };
    (v, out)
}

enum JitVerdict {
    Ok(i64, Vec<String>),
    RuntimeErr((String, String), Vec<String>),
    Unsupported(String),
}

fn run_jit(mir: &klang::mir::MirModule) -> JitVerdict {
    match klang::jit::run_jit_fallible(mir, "main", &[]) {
        Ok((v, out)) => JitVerdict::Ok(v, out),
        Err(klang::jit::JitError::Runtime { diagnostic, output }) => JitVerdict::RuntimeErr(
            (diagnostic.code.clone(), diagnostic.message.clone()),
            output,
        ),
        Err(klang::jit::JitError::Unsupported(msg)) => JitVerdict::Unsupported(msg),
    }
}

fn exit_byte(v: i64) -> u8 {
    (v & 0xFF) as u8
}

/// Assert one program agrees on both backends. Returns true when the
/// JIT refused it as unsupported (caller counts skips).
fn assert_agree(src: &str) -> bool {
    let mir = lower_checked(src);
    let (iv, iout) = run_interp(&mir);
    match (run_jit(&mir), iv) {
        (JitVerdict::Unsupported(_), _) => return true,
        (JitVerdict::Ok(jv, jout), Ok(inv)) => {
            assert_eq!(jout, iout, "stdout differs:\n{src}");
            assert_eq!(jv, inv, "value differs:\n{src}");
            assert_eq!(
                exit_byte(jv),
                exit_byte(inv),
                "exit byte differs:\n{src}"
            );
        }
        (JitVerdict::RuntimeErr((jcode, jmsg), jout), Err((icode, imsg))) => {
            assert_eq!(jout, iout, "pre-failure stdout differs:\n{src}");
            assert_eq!(jcode, icode, "error code differs:\n{src}");
            assert_eq!(jmsg, imsg, "error message differs:\n{src}");
        }
        (j, i) => {
            let jdesc = match &j {
                JitVerdict::Ok(v, o) => format!("Ok({v}, {o:?})"),
                JitVerdict::RuntimeErr((c, m), o) => format!("Err(({c}, {m}), {o:?})"),
                JitVerdict::Unsupported(m) => format!("Unsupported({m})"),
            };
            panic!("backend verdicts differ:\njit:  {jdesc}\ninterp: {i:?}\nprogram:\n{src}");
        }
    }
    false
}

#[test]
fn jit_s5_div_mod_zero_and_min_div() {
    let dir = scratch("divzero");
    // (source, expected error code, expected message, expected stdout)
    let cases: &[(&str, &str, &str, &[&str])] = &[
        ("fn main() -> i32 { return 1 / 0 }", "E-RUNTIME", "division by zero", &[]),
        ("fn main() -> i32 { return 0 / 0 }", "E-RUNTIME", "division by zero", &[]),
        (
            "fn mymin() -> i64 { return 0 - 9223372036854775807 - 1 } fn main() -> i32 { return mymin() / 0 }",
            "E-RUNTIME",
            "division by zero",
            &[],
        ),
        ("fn main() -> i32 { return 1 % 0 }", "E-RUNTIME", "modulo by zero", &[]),
        ("fn main() -> i32 { return 0 % 0 }", "E-RUNTIME", "modulo by zero", &[]),
        // MIN / -1 overflows i64: loud E-OVERFLOW, never a trap.
        (
            "fn mymin() -> i64 { return 0 - 9223372036854775807 - 1 } fn main() -> i32 { let neg = 0 - 1 return mymin() / neg }",
            "E-OVERFLOW",
            "integer overflow in `div`: result out of i64 range",
            &[],
        ),
        // MIN % -1 overflows too (`checked_rem` is None, like the
        // hardware trap): loud E-OVERFLOW in `mod` on both backends.
        (
            "fn mymin() -> i64 { return 0 - 9223372036854775807 - 1 } fn main() -> i32 { let neg = 0 - 1 print(mymin() % neg) return 0 }",
            "E-OVERFLOW",
            "integer overflow in `mod`: result out of i64 range",
            &[],
        ),
        // Prints before the failure flush on both backends.
        (
            "fn main() -> i32 { print(41) print(1 + 1) return 1 / 0 }",
            "E-RUNTIME",
            "division by zero",
            &["41", "2"],
        ),
    ];
    for (src, code, msg, stdout) in cases {
        let mir = lower_checked(src);
        let (iv, iout) = run_interp(&mir);
        let want_out: Vec<String> = stdout.iter().map(|s| s.to_string()).collect();
        assert_eq!(iout, want_out, "interp stdout:\n{src}");
        match run_jit(&mir) {
            JitVerdict::Unsupported(m) => panic!("must not refuse:\n{src}\n{m}"),
            JitVerdict::Ok(v, o) => {
                assert_eq!(*code, "OK", "jit ran but interp must fail:\n{src}");
                assert_eq!(o, want_out);
                let _ = v;
            }
            JitVerdict::RuntimeErr((jcode, jmsg), jout) => {
                assert_eq!(jout, want_out, "jit stdout:\n{src}");
                assert_eq!(jcode, *code, "jit code:\n{src}");
                assert_eq!(jmsg, *msg, "jit message:\n{src}");
                match iv {
                    Err((icode, imsg)) => {
                        assert_eq!(icode, *code);
                        assert_eq!(imsg, *msg);
                    }
                    Ok(v) => panic!("interp ran ({v}) but jit failed:\n{src}"),
                }
            }
        }
        // Full agreement through the shared helper too.
        assert!(!assert_agree(src), "unexpected skip:\n{src}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn jit_s5_checked_overflow_matrix() {
    let dir = scratch("overflow");
    let max = "9223372036854775807";
    let min_expr = "(0 - 9223372036854775807 - 1)";
    let cases = [
        format!("fn main() -> i32 {{ print({max} + 1) return 0 }}"),
        format!("fn main() -> i32 {{ print({min_expr} - 1) return 0 }}"),
        format!("fn main() -> i32 {{ print({max} * 2) return 0 }}"),
        format!("fn main() -> i32 {{ print(0 - {min_expr}) return 0 }}"),
        format!("fn main() -> i64 {{ let a = {max} let b = a + a return b }}"),
        format!("fn main() -> i64 {{ let a = {min_expr} return 0 - a }}"),
        // Just inside the boundary: runs fine on both.
        format!("fn main() -> i64 {{ print({max}) print({min_expr}) return 0 }}"),
        format!("fn main() -> i64 {{ return {max} - 1 + 1 - 1 + 1 }}"),
    ];
    for src in &cases {
        assert!(!assert_agree(src), "unexpected skip:\n{src}");
    }
    // Spot-check the diagnostic shape once (the matrix above proves agreement).
    let mir = lower_checked(&cases[0]);
    match run_jit(&mir) {
        JitVerdict::RuntimeErr((code, msg), _) => {
            assert_eq!(code, "E-OVERFLOW");
            assert_eq!(msg, "integer overflow in `add`: result out of i64 range");
        }
        other => panic!("expected overflow failure, got: {}", match other {
            JitVerdict::Ok(v, _) => format!("Ok({v})"),
            JitVerdict::Unsupported(m) => format!("Unsupported({m})"),
            JitVerdict::RuntimeErr(..) => unreachable!(),
        }),
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn jit_s5_depth_guard_matches_interpreter() {
    let dir = scratch("depth");
    let prog = |n: i32| {
        format!(
            "fn countdown(n: i32) -> i32 {{ if n <= 0 {{ return 0 }} return countdown(n - 1) + 1 }} fn main() -> i32 {{ return countdown({n}) }}"
        )
    };
    // 1023 runs on both (deep-stack worker: the interpreter recurses
    // natively per call, like `robustness_gates`).
    for n in [1000, 1023] {
        let src = prog(n);
        let (iv, iout) = klang::with_deep_stack({
            let src = src.clone();
            move || {
                let mir = lower_checked(&src);
                run_interp(&mir)
            }
        });
        assert_eq!(iv, Ok(n as i64), "interp depth {n}");
        assert!(iout.is_empty());
        let mir = lower_checked(&src);
        match run_jit(&mir) {
            JitVerdict::Ok(jv, jout) => {
                assert_eq!(jv, n as i64);
                assert!(jout.is_empty());
            }
            other => panic!(
                "jit must run depth {n}: {}",
                match other {
                    JitVerdict::RuntimeErr((c, m), _) => format!("Err({c}: {m})"),
                    JitVerdict::Unsupported(m) => format!("Unsupported({m})"),
                    JitVerdict::Ok(_, _) => unreachable!(),
                }
            ),
        }
    }
    // 1024 and beyond fail identically: same code and message.
    for n in [1024, 1025, 2000] {
        let src = prog(n);
        let mir = lower_checked(&src);
        let (iv, _) = klang::with_deep_stack({
            let mir = mir.clone();
            move || run_interp(&mir)
        });
        let (icode, imsg) = iv.expect_err("interp must fail").clone();
        assert_eq!(icode, "E-RUNTIME", "interp depth {n}");
        assert_eq!(imsg, "call depth exceeded (possible recursion)");
        match run_jit(&mir) {
            JitVerdict::RuntimeErr((jcode, jmsg), jout) => {
                assert!(jout.is_empty());
                assert_eq!(jcode, icode, "depth {n} code");
                assert_eq!(jmsg, imsg, "depth {n} message");
            }
            other => panic!(
                "jit must fail depth {n}: {}",
                match other {
                    JitVerdict::Ok(v, _) => format!("Ok({v})"),
                    JitVerdict::Unsupported(m) => format!("Unsupported({m})"),
                    JitVerdict::RuntimeErr(..) => unreachable!(),
                }
            ),
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn jit_s5_narrow_boundaries_match_interpreter() {
    let dir = scratch("bounds");
    let cases = [
        // Param violations (each narrow type).
        "fn id(x: u8) -> i64 { return x } fn main() -> i32 { return id(256) }",
        "fn id(x: u8) -> i64 { return x } fn main() -> i32 { return id(0 - 1) }",
        "fn id(x: i32) -> i64 { return x } fn main() -> i32 { return id(2147483647 + 1) }",
        "fn id(x: u32) -> i64 { return x } fn main() -> i32 { return id(0 - 1) }",
        "fn id(x: u64) -> i64 { return x } fn main() -> i32 { return id(0 - 1) }",
        // Return violations.
        "fn get() -> u8 { return 300 } fn main() -> i32 { return get() }",
        "fn get() -> i32 { return 2147483647 + 1 } fn main() -> i32 { return get() }",
        // Annotated-copy (rebinding an annotated parameter) violations.
        "fn f(x: i32) -> i64 { x = 5000000000 return x } fn main() -> i32 { return f(1) }",
        "fn f(x: u8) -> i64 { x = 300 return x } fn main() -> i32 { return f(1) }",
        // In-range control cases (run identically).
        "fn id(x: u8) -> i64 { return x } fn main() -> i32 { return id(255) }",
        "fn id(x: i32) -> i64 { return x } fn main() -> i32 { return id(0 - 2147483647 - 1) }",
        "fn get() -> u8 { return 255 } fn main() -> i32 { return get() }",
        "fn f(x: u8) -> i64 { x = 200 return x + 1 } fn main() -> i32 { return f(1) }",
    ];
    for src in cases {
        assert!(!assert_agree(src), "unexpected skip:\n{src}");
    }
    // Spot-check one message byte-for-byte.
    let mir = lower_checked(cases[0]);
    match run_jit(&mir) {
        JitVerdict::RuntimeErr((code, msg), _) => {
            assert_eq!(code, "E-RUNTIME");
            assert_eq!(msg, "`id` arg 0: value 256 out of `u8` range 0..255");
        }
        other => panic!(
            "expected boundary failure: {}",
            match other {
                JitVerdict::Ok(v, _) => format!("Ok({v})"),
                JitVerdict::Unsupported(m) => format!("Unsupported({m})"),
                JitVerdict::RuntimeErr(..) => unreachable!(),
            }
        ),
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Deterministic differential fuzz (fixed seed, in-process LCG).
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // Knuth MMIX LCG; high bits only (low bits are weak).
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
}

const INT_POOL: &[&str] = &[
    "0", "1", "0 - 1", "2", "3", "5", "7", "10", "42", "100", "255", "256", "1000",
    "2147483647", "0 - 2147483647 - 1", "4294967295", "9223372036854775807",
    "9223372036854775807 - 1", "0 - 9223372036854775807 - 1",
    "0 - 9223372036854775807", "4611686018427387904", "0 - 4611686018427387904",
];

/// One int expression over `vars` (params + let-bound names).
fn gen_expr(rng: &mut Rng, vars: &[String], depth: usize) -> String {
    // Leaves: pool literal, variable, or the spelled MIN expression.
    if depth == 0 || rng.one_in(3) {
        let r = rng.below(10);
        if r < 6 && !vars.is_empty() {
            return vars[rng.below(vars.len())].clone();
        }
        if r == 6 {
            return "(0 - 9223372036854775807 - 1)".to_string();
        }
        return INT_POOL[rng.below(INT_POOL.len())].to_string();
    }
    match rng.below(7) {
        0 => format!("(0 - ({}))", gen_expr(rng, vars, depth - 1)),
        1 => format!(
            "({} + {})",
            gen_expr(rng, vars, depth - 1),
            gen_expr(rng, vars, depth - 1)
        ),
        2 => format!(
            "({} - {})",
            gen_expr(rng, vars, depth - 1),
            gen_expr(rng, vars, depth - 1)
        ),
        3 => format!(
            "({} * {})",
            gen_expr(rng, vars, depth - 1),
            gen_expr(rng, vars, depth - 1)
        ),
        4 => format!(
            "({} / {})",
            gen_expr(rng, vars, depth - 1),
            gen_expr(rng, vars, depth - 1)
        ),
        5 => format!(
            "({} % {})",
            gen_expr(rng, vars, depth - 1),
            gen_expr(rng, vars, depth - 1)
        ),
        _ => format!("({} + {})", gen_expr(rng, vars, 0), gen_expr(rng, vars, 0)),
    }
}

/// One boolean condition (comparisons over int exprs, `&&`/`||`/`!`).
fn gen_cond(rng: &mut Rng, vars: &[String]) -> String {
    let cmp = ["==", "!=", "<", "<=", ">", ">="];
    let leaf = format!(
        "({} {} {})",
        gen_expr(rng, vars, 1),
        cmp[rng.below(cmp.len())],
        gen_expr(rng, vars, 1)
    );
    match rng.below(4) {
        0 => leaf,
        1 => format!("!({})", leaf),
        2 => format!("({} && {})", leaf, gen_cond_leaf(rng, vars)),
        _ => format!("({} || {})", leaf, gen_cond_leaf(rng, vars)),
    }
}

fn gen_cond_leaf(rng: &mut Rng, vars: &[String]) -> String {
    let cmp = ["==", "!=", "<", "<=", ">", ">="];
    format!(
        "({} {} {})",
        gen_expr(rng, vars, 1),
        cmp[rng.below(cmp.len())],
        gen_expr(rng, vars, 1)
    )
}

struct Gen {
    rng: Rng,
    tmp: usize,
}

impl Gen {
    fn fresh(&mut self) -> String {
        let v = format!("v{}", self.tmp);
        self.tmp += 1;
        v
    }

    /// Statements for a body; `vars` starts as the param names and grows.
    /// `callees` are helpers this body may call (earlier helpers only:
    /// the call graph stays a DAG, so generation always terminates).
    fn gen_body(
        &mut self,
        vars: &mut Vec<String>,
        callees: &[(String, Vec<String>)],
        stmt_budget: usize,
    ) -> String {
        let mut out = String::new();
        for _ in 0..stmt_budget {
            match self.rng.below(10) {
                0 | 1 => {
                    let v = self.fresh();
                    out.push_str(&format!("    let {v} = {}\n", self.gen_expr_top(vars, callees)));
                    vars.push(v);
                }
                2 => {
                    if !vars.is_empty() {
                        let v = vars[self.rng.below(vars.len())].clone();
                        out.push_str(&format!("    {v} = {}\n", self.gen_expr_top(vars, callees)));
                    }
                }
                3 => {
                    out.push_str(&format!("    print({})\n", self.gen_expr_top(vars, callees)));
                }
                4 | 5 => {
                    let t = self.gen_block(vars, callees, 2);
                    if self.rng.one_in(2) {
                        let e = self.gen_block(vars, callees, 2);
                        out.push_str(&format!(
                            "    if {} {{\n{t}    }} else {{\n{e}    }}\n",
                            gen_cond(&mut self.rng, vars)
                        ));
                    } else {
                        out.push_str(&format!("    if {} {{\n{t}    }}\n", gen_cond(&mut self.rng, vars)));
                    }
                }
                6 => {
                    // Bounded counter loop (always terminates: the counter
                    // starts at 0 and grows by exactly 1 per iteration).
                    let c = self.fresh();
                    let bound = 1 + self.rng.below(3);
                    let mut b = format!("        {c} = {c} + 1\n");
                    if self.rng.one_in(2) {
                        b.push_str(&format!("        print({c})\n"));
                    }
                    if self.rng.one_in(2) {
                        let w = self.fresh();
                        let mut readers = vars.clone();
                        readers.push(c.clone());
                        b.push_str(&format!(
                            "        let {w} = {}\n",
                            self.gen_expr_small(&readers)
                        ));
                    }
                    out.push_str(&format!(
                        "    let {c} = 0\n    while {c} < {bound} {{\n{b}    }}\n"
                    ));
                }
                _ => {
                    if !callees.is_empty() && self.rng.one_in(2) {
                        let (name, params) = &callees[self.rng.below(callees.len())];
                        let mut args = Vec::new();
                        for _ in 0..params.len() {
                            args.push(self.gen_expr_small(vars));
                        }
                        let v = self.fresh();
                        out.push_str(&format!("    let {v} = {name}({})\n", args.join(", ")));
                        vars.push(v);
                    } else {
                        out.push_str(&format!("    print({})\n", self.gen_expr_top(vars, callees)));
                    }
                }
            }
        }
        // Tail: early-return shape or plain value return.
        if self.rng.one_in(3) {
            out.push_str(&format!(
                "    if {} {{\n        return {}\n    }}\n    return {}\n",
                gen_cond(&mut self.rng, vars),
                self.gen_expr_top(vars, callees),
                self.gen_expr_top(vars, callees)
            ));
        } else {
            out.push_str(&format!("    return {}\n", self.gen_expr_top(vars, callees)));
        }
        out
    }

    fn gen_block(
        &mut self,
        vars: &[String],
        callees: &[(String, Vec<String>)],
        n: usize,
    ) -> String {
        let scope = vars.to_vec();
        let mut out = String::new();
        for _ in 0..n {
            if self.rng.one_in(2) || scope.is_empty() {
                out.push_str(&format!("        print({})\n", self.gen_expr_top(&scope, callees)));
            } else {
                let v = scope[self.rng.below(scope.len())].clone();
                out.push_str(&format!("        {v} = {}\n", self.gen_expr_top(&scope, callees)));
            }
        }
        out
    }

    fn gen_expr_top(&mut self, vars: &[String], callees: &[(String, Vec<String>)]) -> String {
        // Occasionally call an earlier helper inline. `callees` never
        // contains the enclosing function itself (callers pass
        // strictly-earlier helpers only), so the call graph stays a DAG
        // and generation always terminates.
        if !callees.is_empty() && self.rng.one_in(6) {
            let h = self.rng.below(callees.len());
            let (name, params) = callees[h].clone();
            let mut args = Vec::new();
            for ty in params {
                let e = self.gen_expr_small(vars);
                // Narrow params get mostly-in-range args (sometimes out
                // of range: boundary-error agreement is also coverage).
                args.push(if ty != "i64" && self.rng.below(4) == 0 {
                    match ty.as_str() {
                        "u8" => "1000".to_string(),
                        "i32" => "5000000000".to_string(),
                        _ => e,
                    }
                } else {
                    e
                });
            }
            return format!("{name}({})", args.join(", "));
        }
        gen_expr(&mut self.rng, vars, 2)
    }

    fn gen_expr_small(&mut self, vars: &[String]) -> String {
        gen_expr(&mut self.rng, vars, 1)
    }

    fn gen_program(&mut self) -> String {
        self.tmp = 0;
        let mut out = String::new();
        let n_helpers = self.rng.below(2);
        // Helper signatures first (names + annotations), so bodies can
        // call earlier ones; annotations are occasionally narrow.
        // Callees carry full param annotations (for narrow-arg sampling).
        let mut sigs: Vec<(String, Vec<String>, String)> = Vec::new();
        for h in 0..n_helpers {
            let name = format!("h{h}");
            let argc = self.rng.below(3);
            let mut params = Vec::new();
            for _ in 0..argc {
                params.push(if self.rng.one_in(8) {
                    ["u8", "i32"][self.rng.below(2)].to_string()
                } else {
                    "i64".to_string()
                });
            }
            let ret = if self.rng.one_in(10) { "u8" } else { "i64" }.to_string();
            sigs.push((name, params, ret));
        }
        for (idx, (name, params, ret)) in sigs.iter().enumerate() {
            // Bodies may call strictly-earlier helpers (DAG).
            let callees: Vec<(String, Vec<String>)> = sigs[..idx]
                .iter()
                .map(|(n, p, _)| (n.clone(), p.clone()))
                .collect();
            let plist: Vec<String> = params
                .iter()
                .enumerate()
                .map(|(i, t)| format!("a{i}: {t}"))
                .collect();
            let mut vars: Vec<String> =
                (0..params.len()).map(|i| format!("a{i}")).collect();
            // Narrow `-> u8` helpers return an in-range value (a small
            // modulo of the body expr); the S5 targeted gates already
            // cover narrow-return violations deterministically.
            let body = if ret != "i64" {
                format!(
                    "    return ({} % 200)\n",
                    self.gen_expr_top(&vars, &callees)
                )
            } else {
                { let b = 1 + self.rng.below(2); self.gen_body(&mut vars, &callees, b) }
            };
            out.push_str(&format!(
                "fn {name}({}) -> {ret} {{\n{body}}}\n",
                plist.join(", ")
            ));
        }
        // main calls any helper (or none).
        let callees: Vec<(String, Vec<String>)> = sigs
            .iter()
            .map(|(n, p, _)| (n.clone(), p.clone()))
            .collect();
        let mut vars: Vec<String> = Vec::new();
        let budget = 2 + self.rng.below(2);
        let body = self.gen_body(&mut vars, &callees, budget);
        out.push_str(&format!("fn main() -> i64 {{\n{body}}}\n"));
        out
    }
}

#[test]
fn jit_diff_random_arith() {
    let dir = scratch("diffrand");
    const N: usize = 300;
    // One deep-stack worker for the whole loop (a single 256 MiB
    // reservation, not one per program): generated programs are
    // shallow by construction, and a generator depth bug still
    // surfaces as a panic here rather than aborting mid-run.
    let summary = klang::with_deep_stack(|| {
        let mut gen = Gen {
            rng: Rng(0x5EED_2026),
            tmp: 0,
        };
        let mut skipped = 0usize;
        let mut ok_values = 0usize;
        let mut ok_errors = 0usize;
        let mut mismatches: Vec<String> = Vec::new();
        for i in 0..N {
            let src = gen.gen_program();
            let mir = match try_lower(&src) {
                Ok(m) => m,
                Err(e) => {
                    mismatches.push(format!("program {i}: {e}\nprogram:\n{src}"));
                    continue;
                }
            };
            let (iv, iout) = run_interp(&mir);
            match (run_jit(&mir), iv) {
            (JitVerdict::Unsupported(_), _) => skipped += 1,
            (JitVerdict::Ok(jv, jout), Ok(inv)) => {
                if jout != iout || jv != inv || exit_byte(jv) != exit_byte(inv) {
                    mismatches.push(format!(
                        "program {i}: value/stdout mismatch\njit:  Ok({jv}, {jout:?})\ninterp: Ok({inv}, {iout:?})\nprogram:\n{src}"
                    ));
                } else {
                    ok_values += 1;
                }
            }
            (JitVerdict::RuntimeErr((jcode, jmsg), jout), Err((icode, imsg))) => {
                if jout != iout || jcode != icode || jmsg != imsg {
                    mismatches.push(format!(
                        "program {i}: error mismatch\njit:  Err(({jcode}, {jmsg}), {jout:?})\ninterp: Err(({icode}, {imsg}), {iout:?})\nprogram:\n{src}"
                    ));
                } else {
                    ok_errors += 1;
                }
            }
            (j, v) => {
                let jdesc = match &j {
                    JitVerdict::Ok(x, o) => format!("Ok({x}, {o:?})"),
                    JitVerdict::RuntimeErr((c, m), o) => format!("Err(({c}, {m}), {o:?})"),
                    JitVerdict::Unsupported(m) => format!("Unsupported({m})"),
                };
                mismatches.push(format!(
                    "program {i}: verdict mismatch\njit: {jdesc}\ninterp: {v:?}\nprogram:\n{src}"
                ));
            }
        }
        }
        (skipped, ok_values, ok_errors, mismatches)
    });
    let (skipped, ok_values, ok_errors, mismatches) = summary;
    println!("differential: {N} programs, {ok_values} value-agree, {ok_errors} error-agree, {skipped} skipped-unsupported, {} mismatches", mismatches.len());
    assert!(
        mismatches.is_empty(),
        "\n{} differential mismatch(es):\n{}\n",
        mismatches.len(),
        mismatches.join("\n")
    );
    std::fs::remove_dir_all(&dir).ok();
}
