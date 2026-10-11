//! SECAUDIT-1 — file-path sandbox adversarial gates.
//!
//! F1: plain mode allows every path (OS errors are `E-IO-*`); only
//! `klang run --sandbox` keeps the strict guard (`E-SANDBOX-FILE`).
//! The `..` escapes below run in plain library mode, so a missing target
//! is `E-IO-NOT-FOUND`. Embedded NUL bytes stay a loud `E-IO-FAILED`.
//! Symlink read-through is the F14-documented residual (canonicalization
//! would break legitimately symlinked temp dirs, e.g. macOS
//! `/tmp → /private/tmp`): pinned here deliberately so any future
//! change shows as an intentional diff, not a silent drift.

use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

fn tmp_path(name: &str) -> String {
    let dir = std::env::temp_dir().join("klang-sandbox");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().replace('\\', "/")
}

#[test]
fn sandbox_parent_escape_rejected() {
    // F1: plain mode allows `..` (OS decides). These targets do not exist,
    // so the result is E-IO-NOT-FOUND. Sandbox denial is E-SANDBOX-FILE
    // (see f1_gates / x1_sandbox_gates).
    let src = "fn main() -> i32 { print(read_file(\"../../../etc/passwd\")) return 0 }";
    let err = run_src(src, "main").expect_err("escape must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
    // A buried `..` (not just a leading one) behaves identically.
    let src = "fn main() -> i32 { print(read_file(\"sub/../../etc/passwd\")) return 0 }";
    let err = run_src(src, "main").expect_err("escape must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn sandbox_nul_byte_is_loud_io_failure() {
    // `\u0000` smuggles a real NUL into the path (len proves the byte
    // is there). The lexical guard passes (under tmp); the OS layer
    // rejects loudly — specific code, real cause, never a panic.
    let path = tmp_path("nul_holder.txt");
    let src = format!(
        "fn main() -> i32 {{ let p = \"/tmp/a\\u0000b\" print(len(p)) print(read_file(p)) return 0 }}"
    );
    let _ = std::fs::remove_file(&path);
    let err = run_src(&src, "main").expect_err("NUL path must fail");
    assert_eq!(err.code, "E-IO-FAILED", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("NUL"),
        "real OS cause, not generic: {}",
        err.to_json()
    );
}

#[test]
#[cfg(unix)]
fn sandbox_symlink_read_follows_link() {
    // F14 residual, pinned deliberately: a symlink inside the sandbox
    // is followed (no canonicalization — which would break symlinked
    // temp dirs on macOS). Planting the link already needs fs access.
    let target = tmp_path("link_target.txt");
    std::fs::write(&target, "through-the-link").unwrap();
    let link = tmp_path("read_link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let src = format!("fn main() -> i32 {{ print(read_file(\"{link}\")) return 42 }}");
    let (v, out) = run_src(&src, "main").expect("link read runs");
    assert_eq!(out, vec!["through-the-link".to_string()]);
    assert_eq!(v, 42);
    std::fs::remove_file(&link).ok();
    std::fs::remove_file(&target).ok();
}

#[test]
#[cfg(unix)]
fn sandbox_remove_file_removes_link_not_target() {
    // `remove_file` on a symlink unlinks the link; the target survives.
    let target = tmp_path("unlink_target.txt");
    std::fs::write(&target, "target-survives").unwrap();
    let link = tmp_path("unlink_link");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let src = format!("fn main() -> i32 {{ return remove_file(\"{link}\") }}");
    let (v, _) = run_src(&src, "main").expect("unlink runs");
    assert_eq!(v, 1);
    assert!(
        !std::path::Path::new(&link).exists(),
        "link itself is gone"
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "target-survives"
    );
    std::fs::remove_file(&target).ok();
}
