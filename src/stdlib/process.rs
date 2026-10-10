//! Klang OS-interop process boundary (STDLIB-OSIO-2).
//!
//! `process::run(cmd, args)` in the PRD maps onto Klang's existing flat
//! builtin `run_process` (see `hir::is_builtin` and
//! `runtime::exec_builtin`). Flat names are deliberate: Klang's `::`
//! call syntax only resolves through declared `mod` blocks
//! (`modules::rewrite_ctor` converts `m::f(args)` into a `Call` only
//! when `m` is a program module), so a namespaced `process::run(...)`
//! spelling would parse as an enum constructor and fail type checking.
//! Consistency with the existing builtins matters more than the PRD's
//! illustrative `::` shape.
//!
//! All spawning goes through the free function here so both the
//! interpreter (`runtime::exec_builtin`) and unit tests share one error
//! mapping. Execution uses `std::process::Command` with an argv array —
//! never a shell string — so arguments containing spaces or shell
//! metacharacters arrive at the child unmangled. There is deliberately
//! no `run_shell`-style API: the unsafe thing must not be the
//! easy/default thing (see the PRD security section).
//!
//! Design decision (per the PRD's explicit question): a non-zero exit is
//! a normal, catchable result — it populates `exit_code` in the
//! returned map and does NOT raise. Only a failure to spawn at all
//! (missing binary, not executable, OS refusal) is a diagnostic.

use crate::diagnostics::Diagnostic;

/// `E-PROCESS-NOT-FOUND`: command doesn't exist / isn't executable.
fn process_not_found_for(op: &str, cmd: &str, cause: &str) -> Diagnostic {
    Diagnostic::error(
        "E-PROCESS-NOT-FOUND",
        &format!("{op}({cmd}) failed: command not found or not executable"),
        "runtime",
        0,
        0,
        cause,
        &["check the command is installed and on PATH"],
        "process/not-found",
    )
}

/// `E-PROCESS-FAILED`: the process could not be spawned for a reason
/// other than not-found (OS refusal, resource exhaustion, …). The OS
/// error string is carried as the cause verbatim — never a shared
/// generic message (cf. AUDIT F14). Note: a process that runs and
/// exits non-zero is NOT this error; it is an `Ok` map with a non-zero
/// `exit_code` for the caller to check.
fn process_failed_for(op: &str, cmd: &str, cause: &str) -> Diagnostic {
    Diagnostic::error(
        "E-PROCESS-FAILED",
        &format!("{op}({cmd}) failed to spawn: {cause}"),
        "runtime",
        0,
        0,
        cause,
        &["check OS process limits and permissions"],
        "process/spawn",
    )
}

/// `E-PROCESS-NOT-FOUND`: command doesn't exist / isn't executable.
fn process_not_found(cmd: &str, cause: &str) -> Diagnostic {
    process_not_found_for("run_process", cmd, cause)
}

/// `E-PROCESS-FAILED`: the process could not be spawned for a reason
/// other than not-found (OS refusal, resource exhaustion, …). The OS
/// error string is carried as the cause verbatim — never a shared
/// generic message (cf. AUDIT F14). Note: a process that runs and
/// exits non-zero is NOT this error; it is an `Ok` map with a non-zero
/// `exit_code` for the caller to check.
fn process_failed(cmd: &str, cause: &str) -> Diagnostic {
    process_failed_for("run_process", cmd, cause)
}

/// Map a spawn-time `io::Error` to the specific `E-PROCESS-*`
/// diagnostic for `op` on `cmd`. `NotFound` and `PermissionDenied`
/// (binary missing / not executable) get `E-PROCESS-NOT-FOUND`;
/// everything else is `E-PROCESS-FAILED` with the real OS message.
/// Both are runtime diagnostics, so `try`/`catch` catches them
/// (`e["code"]` is the `E-PROCESS-*` code); `E-CANCELLED`/`exit()`
/// stay uncatchable as before.
pub fn map_spawn_error_for(op: &str, cmd: &str, err: &std::io::Error) -> Diagnostic {
    use std::io::ErrorKind;
    let cause = err.to_string();
    match err.kind() {
        ErrorKind::NotFound | ErrorKind::PermissionDenied => {
            process_not_found_for(op, cmd, &cause)
        }
        _ => process_failed_for(op, cmd, &cause),
    }
}

/// Map a spawn-time `io::Error` to the specific `E-PROCESS-*`
/// diagnostic for `cmd`. `NotFound` and `PermissionDenied` (binary
/// missing / not executable) get `E-PROCESS-NOT-FOUND`; everything
/// else is `E-PROCESS-FAILED` with the real OS message.
pub fn map_spawn_error(cmd: &str, err: &std::io::Error) -> Diagnostic {
    use std::io::ErrorKind;
    let cause = err.to_string();
    match err.kind() {
        ErrorKind::NotFound | ErrorKind::PermissionDenied => {
            process_not_found(cmd, &cause)
        }
        _ => process_failed(cmd, &cause),
    }
}

/// Current working directory as a (lossy) UTF-8 string. A process
/// without a reachable CWD (deleted out from under it) is `E-IO-FAILED`
/// with the real OS message.
pub fn cwd() -> Result<String, Diagnostic> {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| crate::stdlib::file::map_io_error("cwd", ".", &e))
}

/// Set the environment variable `name` to `value` for this process
/// (and its future children). Names containing `=` or NUL are
/// `E-ENV-INVALID` (`std::env::set_var` would panic on them); empty
/// names follow the platform (allowed on Unix). Always succeeds
/// otherwise — `set_var` itself is infallible.
pub fn set_env(name: &str, value: &str) -> Result<(), Diagnostic> {
    if name.contains('=') || name.contains('\0') || value.contains('\0') {
        return Err(crate::diagnostics::Diagnostic::error(
            "E-ENV-INVALID",
            &format!("set_env({name:?}) failed: names cannot contain `=` or NUL"),
            "runtime",
            0,
            0,
            "environment variable names cannot contain `=` or NUL bytes",
            &["pass a plain NAME without `=`"],
            "env/invalid",
        ));
    }
    std::env::set_var(name, value);
    Ok(())
}

/// Captured result of one synchronous run-and-wait execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// Captured stdout (lossy UTF-8).
    pub stdout: String,
    /// Captured stderr (lossy UTF-8).
    pub stderr: String,
    /// Process exit code (`-1` when terminated by signal / unknown).
    pub exit_code: i32,
}

/// Run `cmd` with argv `args` synchronously, capturing stdout, stderr,
/// and the exit code. Arguments are passed as an argv array via
/// `std::process::Command` — no shell is involved, so spaces and shell
/// metacharacters are literal.
pub fn run(cmd: &str, args: &[String]) -> Result<ProcessOutput, Diagnostic> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| map_spawn_error(cmd, &e))?;
    Ok(ProcessOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit_code: out.status.code().unwrap_or(-1),
    })
}

/// S6 `spawn(cmd, args)`: argv-array spawn without a shell, capturing
/// stdout/stderr and the exit code. Identical transport to [`run`]
/// (same `Command::output`, which reads both pipes on threads so
/// >1 MB outputs cannot deadlock); only the caller's view differs
/// (the interpreter renders `status` instead of `exit_code`).
/// No shell, no globbing, no interpolation, no timeout.
pub fn spawn_capture(cmd: &str, args: &[String]) -> Result<ProcessOutput, Diagnostic> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| map_spawn_error_for("spawn", cmd, &e))?;
    Ok(ProcessOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit_code: out.status.code().unwrap_or(-1),
    })
}

/// S6 `run(cmd, args)`: argv-array spawn without a shell, with
/// stdin/stdout/stderr inherited from the parent. Returns only the
/// exit code (`-1` when terminated by signal / unknown). A non-zero
/// exit is a normal `Ok` result; only a failure to start (missing
/// binary, permission denied, OS refusal) is `E-PROCESS-*`.
/// No shell, no globbing, no interpolation, no timeout, no capture.
pub fn run_inherit(cmd: &str, args: &[String]) -> Result<i32, Diagnostic> {
    use std::process::Stdio;
    let status = std::process::Command::new(cmd)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| map_spawn_error_for("run", cmd, &e))?;
    Ok(status.code().unwrap_or(-1))
}

/// S6 `get_env(name)`: value of the variable or `""` when unset.
/// Klang has no `null`/`Option`, so absence is `""` — the same
/// convention as the existing `env` builtin (never an error).
/// Reads the process environment only; it never lists or dumps it.
pub fn get_env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}
