//! L1 compression: gzip/zlib strings (hex) + files + tar (Wave 2 Stage L1).
//!
//! No bytes type exists in Klang (strings are UTF-8, like A1 `conn_read`),
//! so compressed bytes cross the boundary as lowercase hex for strings and
//! as files for binary data:
//!
//! Flat builtins (see `hir::is_builtin` and `runtime::exec_builtin`):
//! `gzip_compress`, `gzip_decompress`, `gzip_file`, `gunzip_file`,
//! `zlib_compress`, `zlib_decompress`, `zlib_file`, `unzlib_file`,
//! `tar_create`, `tar_extract`, `tar_extract_limit`.
//!
//! Pure-Rust crates only: `flate2` with the `miniz_oxide` backend and
//! `tar`. String forms are pure computation (allowed in
//! `klang run --sandbox`); file forms go through the runtime's
//! temp-dir guard (`E-SANDBOX-FILE` outside the temp dir in sandbox).
//!
//! Errors (never a panic, never silent):
//! `E-COMPRESS-CORRUPT` (bad hex, invalid/deflated data, non-UTF-8 string
//! output, corrupt tar), `E-COMPRESS-TOO-LARGE` (over the input/output
//! cap), `E-COMPRESS-UNSAFE-PATH` (tar entry with `..`, absolute path,
//! symlink or hardlink). File IO failures (missing source, permission)
//! reuse `E-IO-*` via `stdlib::file::map_io_error`, like `read_file`.

use std::io::{Read, Write};

use crate::diagnostics::Diagnostic;

// ---------------------------------------------------------------------------
// Limits.
// ---------------------------------------------------------------------------

/// Max bytes one string-compress call accepts (32 MB; the 10 MB gate fits).
pub const MAX_STRING_INPUT_BYTES: usize = 32 * 1024 * 1024;
/// Max bytes one string-decompress call emits (64 MB; the 10 MB gate fits).
pub const MAX_STRING_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// Default max bytes for file decompress/tar extraction (256 MB per queue).
pub const MAX_FILE_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
/// Max bytes one file-compress call reads (256 MB, same default).
pub const MAX_FILE_INPUT_BYTES: u64 = 256 * 1024 * 1024;
/// Characters of a hex payload shown verbatim in diagnostics past this.
const MAX_SHOWN_HEX: usize = 200;

// ---------------------------------------------------------------------------
// Error constructors (every message names the offending value).
// ---------------------------------------------------------------------------

fn compress_err(code: &str, message: String, cause: String, rule: &str) -> Diagnostic {
    Diagnostic::error(
        code,
        &message,
        "runtime",
        0,
        0,
        &cause,
        &["check the input bytes, paths and size limits, then retry"],
        rule,
    )
}

fn corrupt(op: &str, shown: &str, detail: &str) -> Diagnostic {
    compress_err(
        "E-COMPRESS-CORRUPT",
        format!("{op}({shown}) failed: corrupt compressed data: {detail}"),
        detail.to_string(),
        "compress/corrupt",
    )
}

fn too_large(op: &str, shown: &str, detail: &str) -> Diagnostic {
    compress_err(
        "E-COMPRESS-TOO-LARGE",
        format!("{op}({shown}) failed: output too large: {detail}"),
        detail.to_string(),
        "compress/too-large",
    )
}

fn unsafe_path(op: &str, entry: &str) -> Diagnostic {
    compress_err(
        "E-COMPRESS-UNSAFE-PATH",
        format!("{op} failed: unsafe tar entry {entry:?} (absolute paths, `..` and links are rejected)"),
        format!("tar entry {entry:?} escapes the destination"),
        "compress/unsafe-path",
    )
}

fn redact_hex(hex: &str) -> String {
    if hex.len() <= MAX_SHOWN_HEX {
        return format!("{hex:?}");
    }
    let head = &hex[..MAX_SHOWN_HEX];
    let rest = hex.len() - MAX_SHOWN_HEX;
    format!("{head:?}... [{rest} chars redacted]")
}

fn redact_str(s: &str) -> String {
    if s.len() <= MAX_SHOWN_HEX {
        return format!("{s:?}");
    }
    let head = &s[..MAX_SHOWN_HEX];
    let rest = s.len() - MAX_SHOWN_HEX;
    format!("{head:?}... [{rest} chars redacted]")
}

// ---------------------------------------------------------------------------
// Hex helpers (string forms carry exact bytes as lowercase hex, like A1
// `conn_read_hex`; the public hex API stays the pure-Klang `hex` package).
// ---------------------------------------------------------------------------

/// Lowercase hex of `bytes`.
pub fn hex_encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Decode hex to bytes. Odd length or bad digits are
/// `E-COMPRESS-CORRUPT` naming (a redacted prefix of) the offending text.
pub fn hex_decode_str(op: &str, hex: &str) -> Result<Vec<u8>, Diagnostic> {
    if hex.len() % 2 != 0 {
        return Err(corrupt(op, &redact_hex(hex), "odd hex length"));
    }
    let b = hex.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = hex_val(b[i]);
        let lo = hex_val(b[i + 1]);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h << 4) | l),
            _ => return Err(corrupt(op, &redact_hex(hex), "bad hex digit")),
        }
        i += 2;
    }
    Ok(out)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// String forms (pure; allowed in sandbox).
// ---------------------------------------------------------------------------

/// `gzip_compress(s) -> hex Str`. Input is the UTF-8 bytes of `s`.
pub fn gzip_compress(s: &str) -> Result<String, Diagnostic> {
    const OP: &str = "gzip_compress";
    if s.len() > MAX_STRING_INPUT_BYTES {
        return Err(too_large(
            OP,
            &redact_str(s),
            &format!(
                "input {} bytes exceeds limit {}",
                s.len(),
                MAX_STRING_INPUT_BYTES
            ),
        ));
    }
    let mut enc = flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    );
    enc.write_all(s.as_bytes())
        .map_err(|e| corrupt(OP, &redact_str(s), &e.to_string()))?;
    let bytes = enc
        .finish()
        .map_err(|e| corrupt(OP, &redact_str(s), &e.to_string()))?;
    Ok(hex_encode_bytes(&bytes))
}

/// `gzip_decompress(hex) -> Str`. Bad hex, invalid gzip, over-limit output
/// or non-UTF-8 output is `E-COMPRESS-CORRUPT`/`E-COMPRESS-TOO-LARGE`.
pub fn gzip_decompress(hex: &str) -> Result<String, Diagnostic> {
    const OP: &str = "gzip_decompress";
    let bytes = hex_decode_str(OP, hex)?;
    let dec = flate2::read::GzDecoder::new(&bytes[..]);
    let mut out = Vec::new();
    // `take` caps a zip bomb before it can OOM: one byte past the limit
    // is enough to report TOO-LARGE loudly.
    let mut capped = dec.take((MAX_STRING_OUTPUT_BYTES as u64) + 1);
    capped
        .read_to_end(&mut out)
        .map_err(|e| corrupt(OP, &redact_hex(hex), &e.to_string()))?;
    if out.len() > MAX_STRING_OUTPUT_BYTES {
        return Err(too_large(
            OP,
            &redact_hex(hex),
            &format!(
                "output exceeds limit {} bytes",
                MAX_STRING_OUTPUT_BYTES
            ),
        ));
    }
    String::from_utf8(out)
        .map_err(|e| corrupt(OP, &redact_hex(hex), &format!("output is not UTF-8: {e}")))
}

/// `zlib_compress(s) -> hex Str`.
pub fn zlib_compress(s: &str) -> Result<String, Diagnostic> {
    const OP: &str = "zlib_compress";
    if s.len() > MAX_STRING_INPUT_BYTES {
        return Err(too_large(
            OP,
            &redact_str(s),
            &format!(
                "input {} bytes exceeds limit {}",
                s.len(),
                MAX_STRING_INPUT_BYTES
            ),
        ));
    }
    let mut enc = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    );
    enc.write_all(s.as_bytes())
        .map_err(|e| corrupt(OP, &redact_str(s), &e.to_string()))?;
    let bytes = enc
        .finish()
        .map_err(|e| corrupt(OP, &redact_str(s), &e.to_string()))?;
    Ok(hex_encode_bytes(&bytes))
}

/// `zlib_decompress(hex) -> Str`.
pub fn zlib_decompress(hex: &str) -> Result<String, Diagnostic> {
    const OP: &str = "zlib_decompress";
    let bytes = hex_decode_str(OP, hex)?;
    let dec = flate2::read::ZlibDecoder::new(&bytes[..]);
    let mut out = Vec::new();
    let mut capped = dec.take((MAX_STRING_OUTPUT_BYTES as u64) + 1);
    capped
        .read_to_end(&mut out)
        .map_err(|e| corrupt(OP, &redact_hex(hex), &e.to_string()))?;
    if out.len() > MAX_STRING_OUTPUT_BYTES {
        return Err(too_large(
            OP,
            &redact_hex(hex),
            &format!(
                "output exceeds limit {} bytes",
                MAX_STRING_OUTPUT_BYTES
            ),
        ));
    }
    String::from_utf8(out)
        .map_err(|e| corrupt(OP, &redact_hex(hex), &format!("output is not UTF-8: {e}")))
}

// ---------------------------------------------------------------------------
// File forms (runtime guards paths with the temp-dir rule in sandbox).
// ---------------------------------------------------------------------------

fn read_input_file(op: &str, path: &str) -> Result<Vec<u8>, Diagnostic> {
    let bytes =
        std::fs::read(path).map_err(|e| crate::stdlib::file::map_io_error(op, path, &e))?;
    if bytes.len() as u64 > MAX_FILE_INPUT_BYTES {
        return Err(too_large(
            op,
            &format!("{path:?}"),
            &format!(
                "input {} bytes exceeds limit {MAX_FILE_INPUT_BYTES}",
                bytes.len()
            ),
        ));
    }
    Ok(bytes)
}

fn write_output_file(op: &str, path: &str, bytes: &[u8]) -> Result<i64, Diagnostic> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| crate::stdlib::file::map_io_error(op, path, &e))?;
        }
    }
    std::fs::write(path, bytes)
        .map_err(|e| crate::stdlib::file::map_io_error(op, path, &e))?;
    Ok(bytes.len() as i64)
}

fn decompress_capped(
    op: &str,
    shown: &str,
    reader: impl Read,
) -> Result<Vec<u8>, Diagnostic> {
    let mut out = Vec::new();
    let mut capped = reader.take(MAX_FILE_OUTPUT_BYTES + 1);
    capped
        .read_to_end(&mut out)
        .map_err(|e| corrupt(op, shown, &e.to_string()))?;
    if out.len() as u64 > MAX_FILE_OUTPUT_BYTES {
        return Err(too_large(
            op,
            shown,
            &format!("output exceeds limit {MAX_FILE_OUTPUT_BYTES} bytes"),
        ));
    }
    Ok(out)
}

/// `gzip_file(src, dst) -> Int` (output bytes). Binary-safe (any bytes).
pub fn gzip_file(src: &str, dst: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "gzip_file";
    let input = read_input_file(OP, src)?;
    let mut enc = flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    );
    enc.write_all(&input)
        .map_err(|e| corrupt(OP, &format!("{src:?}"), &e.to_string()))?;
    let bytes = enc
        .finish()
        .map_err(|e| corrupt(OP, &format!("{src:?}"), &e.to_string()))?;
    write_output_file(OP, dst, &bytes)
}

/// `gunzip_file(src, dst) -> Int` (output bytes).
pub fn gunzip_file(src: &str, dst: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "gunzip_file";
    let input = read_input_file(OP, src)?;
    let dec = flate2::read::GzDecoder::new(&input[..]);
    let out = decompress_capped(OP, &format!("{src:?}"), dec)?;
    write_output_file(OP, dst, &out)
}

/// `zlib_file(src, dst) -> Int` (output bytes).
pub fn zlib_file(src: &str, dst: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "zlib_file";
    let input = read_input_file(OP, src)?;
    let mut enc = flate2::write::ZlibEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    );
    enc.write_all(&input)
        .map_err(|e| corrupt(OP, &format!("{src:?}"), &e.to_string()))?;
    let bytes = enc
        .finish()
        .map_err(|e| corrupt(OP, &format!("{src:?}"), &e.to_string()))?;
    write_output_file(OP, dst, &bytes)
}

/// `unzlib_file(src, dst) -> Int` (output bytes; `zlib_file` inverse).
pub fn unzlib_file(src: &str, dst: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "unzlib_file";
    let input = read_input_file(OP, src)?;
    let dec = flate2::read::ZlibDecoder::new(&input[..]);
    let out = decompress_capped(OP, &format!("{src:?}"), dec)?;
    write_output_file(OP, dst, &out)
}

// ---------------------------------------------------------------------------
// Tar (plain, uncompressed).
// ---------------------------------------------------------------------------

/// `tar_create(src_dir, out_path) -> Int` (`1`).
/// Archives `src_dir` recursively with relative entry names.
pub fn tar_create(src_dir: &str, out_path: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "tar_create";
    let meta =
        std::fs::metadata(src_dir).map_err(|e| crate::stdlib::file::map_io_error(OP, src_dir, &e))?;
    if !meta.is_dir() {
        let e = std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{src_dir:?} is not a directory"),
        );
        return Err(crate::stdlib::file::map_io_error(OP, src_dir, &e));
    }
    if let Some(parent) = std::path::Path::new(out_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| crate::stdlib::file::map_io_error(OP, out_path, &e))?;
        }
    }
    let file = std::fs::File::create(out_path)
        .map_err(|e| crate::stdlib::file::map_io_error(OP, out_path, &e))?;
    let mut builder = tar::Builder::new(file);
    builder
        .append_dir_all(".", src_dir)
        .map_err(|e| corrupt(OP, &format!("{src_dir:?}"), &e.to_string()))?;
    builder
        .into_inner()
        .map_err(|e| corrupt(OP, &format!("{out_path:?}"), &e.to_string()))?;
    Ok(1)
}

/// `tar_extract(tar_path, dest_dir) -> Int` (`1`) with the default
/// 256 MB output cap.
pub fn tar_extract(tar_path: &str, dest_dir: &str) -> Result<i64, Diagnostic> {
    tar_extract_capped(tar_path, dest_dir, MAX_FILE_OUTPUT_BYTES)
}

/// `tar_extract_limit(tar_path, dest_dir, max_bytes) -> Int` (`1`).
/// `max_bytes` `0`..=1 GiB; negative or huge is `E-COMPRESS-TOO-LARGE`.
pub fn tar_extract_limit(
    tar_path: &str,
    dest_dir: &str,
    max_bytes: i64,
) -> Result<i64, Diagnostic> {
    const OP: &str = "tar_extract_limit";
    if max_bytes < 0 || max_bytes > 1024 * 1024 * 1024 {
        return Err(too_large(
            OP,
            &format!("{tar_path:?}"),
            &format!("limit {max_bytes} out of range 0..1073741824"),
        ));
    }
    tar_extract_capped(tar_path, dest_dir, max_bytes as u64)
}

fn tar_extract_capped(
    tar_path: &str,
    dest_dir: &str,
    max_bytes: u64,
) -> Result<i64, Diagnostic> {
    const OP: &str = "tar_extract";
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| crate::stdlib::file::map_io_error(OP, dest_dir, &e))?;
    let file = std::fs::File::open(tar_path)
        .map_err(|e| crate::stdlib::file::map_io_error(OP, tar_path, &e))?;
    let mut archive = tar::Archive::new(file);
    let entries = archive
        .entries()
        .map_err(|e| corrupt(OP, &format!("{tar_path:?}"), &e.to_string()))?;
    let dest = std::path::Path::new(dest_dir);
    let mut total: u64 = 0;
    for entry in entries {
        let mut entry =
            entry.map_err(|e| corrupt(OP, &format!("{tar_path:?}"), &e.to_string()))?;
        let entry_path = entry
            .path()
            .map_err(|e| corrupt(OP, &format!("{tar_path:?}"), &e.to_string()))?
            .into_owned();
        let entry_shown = entry_path.to_string_lossy().into_owned();
        // Path-traversal protection: reject absolute paths, `..`, and
        // links (symlinks/hardlinks can escape on unpack).
        if entry_path.is_absolute() {
            return Err(unsafe_path(OP, &entry_shown));
        }
        if entry_path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(unsafe_path(OP, &entry_shown));
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(unsafe_path(OP, &entry_shown));
        }
        // Output-size guard (pre-check from the header, never unpack past
        // the cap; the header could lie, so `unpack_in` below is the only
        // writer and `total` also guards the sum).
        if kind.is_file() {
            let size = entry.header().size().unwrap_or(0);
            total = total.saturating_add(size);
            if total > max_bytes {
                return Err(too_large(
                    OP,
                    &format!("{tar_path:?}"),
                    &format!("output exceeds limit {max_bytes} bytes"),
                ));
            }
        }
        entry
            .unpack_in(dest)
            .map_err(|e| {
                let s = e.to_string();
                if s.contains("absolute") || s.contains("parent") || s.contains("escape") {
                    unsafe_path(OP, &entry_shown)
                } else {
                    corrupt(OP, &format!("{tar_path:?}"), &s)
                }
            })?;
    }
    Ok(1)
}
