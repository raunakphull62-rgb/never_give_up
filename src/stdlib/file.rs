//! Klang OS-interop file boundary (STDLIB-OSIO-1).
//!
//! `file::read/write/append/exists/remove` in the PRD map onto Klang's
//! existing flat builtins `read_file` / `write_file` / `append_file` /
//! `exists` / `remove_file` (see `hir::is_builtin` and
//! `runtime::exec_builtin`). Flat names are deliberate: Klang's `::` call syntax only resolves through declared
//! `mod` blocks (`modules::rewrite_ctor` converts `m::f(args)` into a
//! `Call` only when `m` is a program module), so a namespaced
//! `file::read(...)` spelling would parse as an enum constructor and
//! fail type checking. Consistency with the four existing builtins
//! matters more than the PRD's illustrative `::` shape.
//!
//! All I/O goes through the free functions here so both the interpreter
//! (`runtime::exec_builtin`) and unit tests share one error mapping.
//! Paths are taken literally: no globbing, no `~` expansion, no
//! environment-variable substitution (surprising path expansion is its
//! own bug class; see the PRD security section).
//!
//! Tests use real temp-dir files (unlike `net::MockTransport`, no mock
//! is needed — the filesystem is hermetic under `std::env::temp_dir()`).

use crate::diagnostics::Diagnostic;

/// `E-IO-NOT-FOUND`: file doesn't exist on read.
fn io_not_found(op: &str, path: &str, cause: &str) -> Diagnostic {
    Diagnostic::error(
        "E-IO-NOT-FOUND",
        &format!("{op}({path}) failed: file does not exist"),
        "runtime",
        0,
        0,
        cause,
        &["check the path exists first with exists()"],
        "io/not-found",
    )
}

/// `E-IO-PERMISSION`: permission denied on read/write/append.
fn io_permission(op: &str, path: &str, cause: &str) -> Diagnostic {
    Diagnostic::error(
        "E-IO-PERMISSION",
        &format!("{op}({path}) failed: permission denied"),
        "runtime",
        0,
        0,
        cause,
        &["check file ownership and mode bits"],
        "io/permission",
    )
}

/// `E-IO-FAILED`: other I/O failure (disk full, etc.). The OS error
/// string is carried as the cause verbatim — never a shared generic
/// message (cf. AUDIT F14, where one cause string was reused across
/// unrelated failure modes).
fn io_failed(op: &str, path: &str, cause: &str) -> Diagnostic {
    Diagnostic::error(
        "E-IO-FAILED",
        &format!("{op}({path}) failed: {cause}"),
        "runtime",
        0,
        0,
        cause,
        &["check disk space and filesystem health"],
        "io/failure",
    )
}

/// Map an `io::Error` to the specific `E-IO-*` diagnostic for `op` on
/// `path`. `NotFound` and `PermissionDenied` get their own codes;
/// everything else is `E-IO-FAILED` with the real OS message.
pub fn map_io_error(op: &str, path: &str, err: &std::io::Error) -> Diagnostic {
    use std::io::ErrorKind;
    let cause = err.to_string();
    match err.kind() {
        ErrorKind::NotFound => io_not_found(op, path, &cause),
        ErrorKind::PermissionDenied => io_permission(op, path, &cause),
        _ => io_failed(op, path, &cause),
    }
}

/// Read the whole file at `path` into a string.
pub fn read(path: &str) -> Result<String, Diagnostic> {
    std::fs::read_to_string(path).map_err(|e| map_io_error("read_file", path, &e))
}

/// Overwrite `path` with `content`. Returns the byte count written.
pub fn write(path: &str, content: &str) -> Result<usize, Diagnostic> {
    std::fs::write(path, content)
        .map(|()| content.len())
        .map_err(|e| map_io_error("write_file", path, &e))
}

/// Append `content` to `path` (creating it when absent). Returns the
/// byte count appended.
pub fn append(path: &str, content: &str) -> Result<usize, Diagnostic> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| map_io_error("append_file", path, &e))?;
    f.write_all(content.as_bytes())
        .map(|()| content.len())
        .map_err(|e| map_io_error("append_file", path, &e))
}

/// Check whether `path` exists. Never fails (mirrors `exists()`).
pub fn exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

/// Delete the file at `path`. Returns `()` on success; missing files map
/// to `E-IO-NOT-FOUND` and permission failures to `E-IO-PERMISSION` via
/// the shared [`map_io_error`] helper (same convention as
/// read/write/append). Directory removal is out of scope.
pub fn remove(path: &str) -> Result<(), Diagnostic> {
    std::fs::remove_file(path).map_err(|e| map_io_error("remove_file", path, &e))
}

/// List entry names (files and directories, not `.`/`..`) directly
/// inside `path`, sorted for determinism (raw `read_dir` order is
/// OS-dependent). Missing dirs map to `E-IO-NOT-FOUND`, permission
/// failures to `E-IO-PERMISSION` via [`map_io_error`].
pub fn list_dir(path: &str) -> Result<Vec<String>, Diagnostic> {
    let rd = std::fs::read_dir(path).map_err(|e| map_io_error("list_dir", path, &e))?;
    let mut out = Vec::new();
    for entry in rd {
        let entry = entry.map_err(|e| map_io_error("list_dir", path, &e))?;
        let name = entry.file_name();
        out.push(name.to_string_lossy().into_owned());
    }
    out.sort();
    Ok(out)
}

/// Create the single directory at `path` (parents must exist).
/// Creating an existing directory is `E-IO-FAILED` (use `make_dirs`
/// for the idempotent form); other failures map via [`map_io_error`].
pub fn make_dir(path: &str) -> Result<(), Diagnostic> {
    std::fs::create_dir(path).map_err(|e| map_io_error("make_dir", path, &e))
}

/// Create `path` and all missing parents (idempotent: existing dirs
/// are fine). Failures map via [`map_io_error`].
pub fn make_dirs(path: &str) -> Result<(), Diagnostic> {
    std::fs::create_dir_all(path).map_err(|e| map_io_error("make_dirs", path, &e))
}

/// True when `path` exists and is a directory. Never fails (mirrors
/// `exists()`).
pub fn is_dir(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

/// True when `path` exists and is a file. Never fails (mirrors
/// `exists()`).
pub fn is_file(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

/// Rename (move) `from` to `to`. Missing sources map to
/// `E-IO-NOT-FOUND`, permission failures to `E-IO-PERMISSION` via
/// [`map_io_error`].
pub fn rename(from: &str, to: &str) -> Result<(), Diagnostic> {
    std::fs::rename(from, to).map_err(|e| map_io_error("rename_file", from, &e))
}

/// Copy `from` to `to`, returning the byte count copied. Missing
/// sources map to `E-IO-NOT-FOUND` via [`map_io_error`].
pub fn copy(from: &str, to: &str) -> Result<u64, Diagnostic> {
    std::fs::copy(from, to).map_err(|e| map_io_error("copy_file", from, &e))
}

/// Size of the file at `path` in bytes. Missing files map to
/// `E-IO-NOT-FOUND` via [`map_io_error`]. Sizes above `i32::MAX` flow
/// as values but cannot participate in checked `i32` arithmetic
/// (same as oversized `int()` results).
pub fn file_size(path: &str) -> Result<u64, Diagnostic> {
    let meta = std::fs::metadata(path).map_err(|e| map_io_error("file_size", path, &e))?;
    Ok(meta.len())
}
