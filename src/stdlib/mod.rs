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

/// Value utilities: `ord` / `chr` / `slice` / `sort` (SYS-DATA-1).
/// Wired into the interpreter as the flat builtins
/// `ord(s)` / `chr(n)` / `slice(v, lo, hi)` / `sort(list)`.
pub mod seq;

/// B1 stdlib gap fill: regex complements, uuid, path absolute, file
/// mtime, sprintf, seeded RNG, char-based string method helpers.
/// Wired as the flat builtins `regex_find_all` / `regex_replace` /
/// `uuid_v4` / `path_absolute` / `file_mtime` / `sprintf` /
/// `random_seed` / `random_seeded_int` / `random_seeded_float` plus the
/// string methods `pad_start` / `pad_end` / `repeat` / `lines`.
pub mod b1;

/// A1 sockets: TCP, UDP, DNS with opaque handle values.
/// Wired as the flat builtins `tcp_listen` / `listener_port` /
/// `tcp_accept` / `tcp_accept_timeout` / `tcp_connect` / `conn_read` /
/// `conn_read_line` / `conn_read_hex` / `conn_write` / `conn_write_hex` /
/// `conn_close` / `conn_set_timeout` / `conn_peer` / `udp_bind` /
/// `udp_send_to` / `udp_recv_from` / `dns_lookup` / `is_null`.
pub mod a1_net;

/// A2 threads, channels, signals with opaque handle values.
/// Wired as the flat builtins `thread_spawn` / `thread_join` /
/// `thread_id` / `chan` / `send` / `recv` / `recv_timeout` / `try_recv` /
/// `close` / `signal_watch` / `signal_pending`.
pub mod a2_threads;

/// A3 HTTP server + fuller client with opaque server handles.
/// Wired as the flat builtins `http_serve` / `server_port` /
/// `http_stop` / `http_request` / `http_json` / `http_text` /
/// `http_redirect` / `route_match` / `json_decode` / `json_stringify`.
pub mod a3_http;

/// L1 datetime: UTC epoch-millisecond civil math via `chrono`.
/// Wired as the flat builtins `dt_now` / `dt_parse_iso` / `dt_format` /
/// `dt_add` / `dt_diff_ms` / `dt_parts` / `dt_parts_offset` /
/// `dt_from_parts` / `dt_local_offset_minutes` / `dt_is_leap` /
/// `dt_days_in_month`.
pub mod l1_datetime;

/// L1 compression: gzip/zlib strings (hex) + files + tar.
/// Wired as the flat builtins `gzip_compress` / `gzip_decompress` /
/// `gzip_file` / `gunzip_file` / `zlib_compress` / `zlib_decompress` /
/// `zlib_file` / `unzlib_file` / `tar_create` / `tar_extract` /
/// `tar_extract_limit`.
pub mod l1_compress;

/// v2 stdlib error placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdlibError {
    /// Machine-readable reason.
    pub message: String,
}
