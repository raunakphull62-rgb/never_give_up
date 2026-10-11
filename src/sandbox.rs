//! X1 sandbox mode for `klang run --sandbox`.
//!
//! In sandbox mode file access is limited to the system temp directory,
//! and process spawning (`spawn`/`run`/`run_process`) plus network
//! (`http_get`/`http_post` and async variants) are denied with clear
//! `E-SANDBOX-*` codes. Plain `klang run` keeps normal behavior
//! (relaxed relative paths, spawn/network allowed).
//!
//! Threads and FFI have no runtime surface in this tree (no `thread_spawn`
//! / `extern` builtins yet), so there is nothing to deny for those
//! categories today; the codes below reserve the namespace for the
//! A-lane stages that will add them (see PRD 2.7).

use std::sync::atomic::{AtomicBool, Ordering};

use crate::diagnostics::Diagnostic;

/// Global sandbox flag. False by default (normal `klang run` + library
/// callers); the CLI sets true for `klang run --sandbox`, tests set it
/// explicitly around sandboxed runs.
static SANDBOX_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable (`true`) or disable (`false`) sandbox denials for the current
/// process. The CLI calls this once per `run`; library tests set/reset
/// around each sandboxed case (serialized by a mutex in the gate file).
pub fn set_sandbox(enabled: bool) {
    SANDBOX_ENABLED.store(enabled, Ordering::SeqCst);
}

/// True when sandbox denials are active (`klang run --sandbox` or an
/// explicit test setup).
pub fn is_sandbox() -> bool {
    SANDBOX_ENABLED.load(Ordering::SeqCst)
}

/// `E-SANDBOX-SPAWN`: process spawning denied in sandbox mode.
pub fn deny_spawn(op: &str) -> Diagnostic {
    Diagnostic::error(
        "E-SANDBOX-SPAWN",
        &format!("{op}() denied in sandbox mode: process spawning is not allowed with `klang run --sandbox`"),
        "runtime",
        0,
        0,
        "sandbox mode denies spawn/run: untrusted programs must not start OS processes",
        &["run without --sandbox to allow process spawning"],
        "sandbox/spawn",
    )
}

/// `E-SANDBOX-NETWORK`: network access denied in sandbox mode.
pub fn deny_network(op: &str) -> Diagnostic {
    Diagnostic::error(
        "E-SANDBOX-NETWORK",
        &format!("{op}() denied in sandbox mode: network access is not allowed with `klang run --sandbox`"),
        "runtime",
        0,
        0,
        "sandbox mode denies network: untrusted programs must not dial out",
        &["run without --sandbox to allow network access"],
        "sandbox/network",
    )
}

/// `E-SANDBOX-FILE`: file access outside the temp dir in sandbox mode.
/// Normal strict-guard failures outside sandbox stay `E-RUNTIME` (pinned
/// by existing gates); only sandbox runs use this code so the denial is
/// unambiguous in `klang run --sandbox` output.
pub fn deny_file(path: &str, reason: &str) -> Diagnostic {
    Diagnostic::error(
        "E-SANDBOX-FILE",
        &format!("sandbox denies file access to `{path}`: {reason}"),
        "runtime",
        0,
        0,
        "sandbox mode limits file access to the system temp directory",
        &["run without --sandbox for normal relative paths"],
        "sandbox/file",
    )
}
