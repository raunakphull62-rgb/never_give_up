//! Klang v2 standard library (Phase 6).
//!
//! `verify`/`tune` entry points live in [`verify`]; `net::echo_get` arrives
//! in Phase 10. Only [`verify`]/[`tune`](verify::tune) mint Harmonic proofs.

/// `verify`/`tune` runtime entry points.
pub mod verify;

/// Network boundary: `net::echo_get` behind a mockable transport.
pub mod net;

/// OS-interop file boundary: `read`/`write`/`append`/`exists`/`remove`
/// with `E-IO-*` diagnostics (STDLIB-OSIO-1, remove in follow-up). Wired
/// into the interpreter as the flat builtins
/// `read_file`/`write_file`/`append_file`/`exists`/`remove_file`.
pub mod file;

/// OS-interop process boundary: `run(cmd, args)` with `E-PROCESS-*`
/// diagnostics (STDLIB-OSIO-2). Wired into the interpreter as the flat
/// builtin `run_process(cmd, args)` returning a map with
/// `stdout`/`stderr`/`exit_code`.
pub mod process;

/// OS-interop regex boundary: `is_match`/`find` with
/// `E-REGEX-INVALID-PATTERN` diagnostics (STDLIB-OSIO-3). Wired into
/// the interpreter as the flat builtins
/// `regex_is_match(pattern, text)` / `regex_find(pattern, text)`.
pub mod regex;

/// OS-interop time boundary: `sleep`/`now`/`elapsed` with
/// `E-TIME-INVALID` diagnostics (STDLIB-TIME-1). Wired into the
/// interpreter as the flat builtins
/// `time_sleep(seconds)` / `time_now()` / `time_elapsed(since)`.
pub mod time;

/// OS-interop HTTP boundary: real synchronous `GET`/`POST` via
/// `ureq` with `E-NET-*` diagnostics (STDLIB-NET-1), plus simulated-async
/// twins on a shared bounded pool (FOUNDATION-3 Part 2A). Wired into the
/// interpreter as the flat builtins
/// `http_get(url)` / `http_post(url, body, headers)` /
/// `http_get_async(url)` / `http_post_async(url, body, headers)`.
/// `net.rs`'s `MockTransport` is the v2 echo test double and stays untouched.
pub mod http;

/// OS-interop stdin boundary: `read_line` / `parse_int` / `parse_float`
/// with `E-PARSE-INT` / `E-PARSE-FLOAT` diagnostics (STDLIB-IO-1). Wired
/// into the interpreter as the flat builtins
/// `read_line()` / `parse_int(s)` / `parse_float(s)`.
pub mod io;

/// v2 stdlib error placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdlibError {
    /// Machine-readable reason.
    pub message: String,
}
