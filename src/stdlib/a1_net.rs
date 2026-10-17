//! A1 sockets: TCP, UDP, DNS (Wave 2 Stage A1).
//!
//! Flat builtins (matching `read_file` / `http_get` convention):
//! `tcp_listen`, `listener_port`, `tcp_accept`, `tcp_accept_timeout`,
//! `tcp_connect`, `conn_read`, `conn_read_line`, `conn_read_hex`,
//! `conn_write`, `conn_write_hex`, `conn_close`, `conn_set_timeout`,
//! `conn_peer`, `udp_bind`, `udp_send_to`, `udp_recv_from`,
//! `dns_lookup`, `is_null`.
//!
//! Strings are UTF-8. There is no bytes type: `conn_read` uses lossy
//! UTF-8 (like `http_get` bodies) and `conn_read_hex` / `conn_write_hex`
//! carry exact bytes as lowercase hex. General hex helpers live in the
//! pure-Klang `hex` package (`hex_encode` / `hex_decode`); no native
//! duplicates are added here (a same-named native would trip `W-SHADOW`
//! and break the package parity self-test).
//!
//! Handles (`Listener`, `Conn`, `UdpSock`) are opaque `Value` variants
//! holding `Arc<Mutex<...>>` state: they cannot be forged from ints,
//! they compare by identity, they render as `<listener>` / `<conn>` /
//! `<udpsock>`, and they close on drop (program exit closes whatever the
//! program forgot). `conn_close` accepts any handle kind (listener, conn
//! or udpsock) and is idempotent. Operations on a locally-closed handle
//! are `E-NET-CLOSED`; peer close surfaces as `""` (`conn_read`,
//! `conn_read_hex`) or null (`conn_read_line`, `tcp_accept_timeout`).
//!
//! Null has no literal: null values only arise from `tcp_accept_timeout`
//! (timeout) and `conn_read_line` (peer close). Test them with the
//! `is_null(x)` builtin (`1` for null, else `0`); `str(null)` is `"null"`,
//! `print(null)` prints `null`, null is falsy and `==` only to null.
//!
//! All functions deny with `E-SANDBOX-NETWORK` under
//! `klang run --sandbox`. Blocking calls only (no threads in A1).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::diagnostics::Diagnostic;
use crate::runtime::Value;

// ---------------------------------------------------------------------------
// Limits.
// ---------------------------------------------------------------------------

/// Max bytes one `conn_read` / `conn_read_hex` / `udp_recv_from` call
/// buffers (10 MB). Larger `max_bytes` is `E-NET-IO`, never an OOM.
pub const MAX_READ_BYTES: i64 = 10 * 1024 * 1024;
/// Max UDP datagram the recv path accepts (UDP cannot exceed 64 KiB).
pub const MAX_UDP_BYTES: i64 = 65535;
/// Characters of a hex payload shown verbatim in diagnostics past this.
const MAX_SHOWN_HEX: usize = 200;

// ---------------------------------------------------------------------------
// Handle state (opaque, shared by all `Value` clones of one handle).
// ---------------------------------------------------------------------------

/// Listening socket. `listener` is `None` once closed.
#[derive(Debug)]
pub struct ListenerState {
    pub listener: Option<TcpListener>,
    pub closed: bool,
}

/// Connected stream plus line-read leftover buffer. `stream` is `None`
/// once closed. `peer` is the `ip:port` string captured at accept/connect.
/// `requests` counts served requests (HTTP keep-alive cap; survives the
/// worker→pool→worker handoffs).
#[derive(Debug)]
pub struct ConnState {
    pub stream: Option<TcpStream>,
    pub buf: Vec<u8>,
    pub closed: bool,
    pub peer: String,
    pub requests: u64,
}

/// UDP socket. `socket` is `None` once closed.
#[derive(Debug)]
pub struct UdpState {
    pub socket: Option<UdpSocket>,
    pub closed: bool,
}

pub type ListenerHandle = Arc<Mutex<ListenerState>>;
pub type ConnHandle = Arc<Mutex<ConnState>>;
pub type UdpHandle = Arc<Mutex<UdpState>>;

// ---------------------------------------------------------------------------
// Error constructors (every message names the offending value).
// ---------------------------------------------------------------------------

fn net_err(code: &str, message: String, cause: String, rule: &str) -> Diagnostic {
    Diagnostic::error(code, &message, "runtime", 0, 0, &cause, &["check the host, port and timeout, then retry"], rule)
}

fn refused(op: &str, target: &str, cause: &str) -> Diagnostic {
    net_err(
        "E-NET-REFUSED",
        format!("{op}({target}) failed: connection refused"),
        cause.to_string(),
        "net/refused",
    )
}

fn timeout(op: &str, target: &str, cause: &str) -> Diagnostic {
    net_err(
        "E-NET-TIMEOUT",
        format!("{op}({target}) failed: timed out"),
        cause.to_string(),
        "net/timeout",
    )
}

fn addr_invalid(op: &str, target: &str, detail: &str) -> Diagnostic {
    net_err(
        "E-NET-ADDR-INVALID",
        format!("{op}({target}) failed: invalid address: {detail}"),
        detail.to_string(),
        "net/addr-invalid",
    )
}

fn addr_in_use(op: &str, target: &str, cause: &str) -> Diagnostic {
    net_err(
        "E-NET-ADDR-IN-USE",
        format!("{op}({target}) failed: address already in use"),
        cause.to_string(),
        "net/addr-in-use",
    )
}

fn closed(op: &str, kind: &str) -> Diagnostic {
    net_err(
        "E-NET-CLOSED",
        format!("{op} failed: {kind} is already closed"),
        format!("{kind} handle was closed"),
        "net/closed",
    )
}

fn io_err(op: &str, target: &str, cause: &str) -> Diagnostic {
    net_err(
        "E-NET-IO",
        format!("{op}({target}) failed: {cause}"),
        cause.to_string(),
        "net/io",
    )
}

fn dns_err(op: &str, host: &str, cause: &str) -> Diagnostic {
    net_err(
        "E-NET-DNS",
        format!("{op}({host}) failed: DNS lookup failed: {cause}"),
        cause.to_string(),
        "net/dns",
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

// ---------------------------------------------------------------------------
// Small validators.
// ---------------------------------------------------------------------------

fn check_port(op: &str, target: &str, port: i64) -> Result<u16, Diagnostic> {
    if port < 0 || port > 65535 {
        return Err(addr_invalid(op, target, &format!("port {port} out of range 0..65535")));
    }
    Ok(port as u16)
}

fn check_max_bytes(op: &str, max_bytes: i64, cap: i64) -> Result<usize, Diagnostic> {
    if max_bytes < 0 {
        return Err(io_err(op, &format!("{max_bytes}"), &format!("max_bytes {max_bytes} is negative")));
    }
    if max_bytes > cap {
        return Err(io_err(
            op,
            &format!("{max_bytes}"),
            &format!("max_bytes {max_bytes} exceeds limit {cap}"),
        ));
    }
    Ok(max_bytes as usize)
}

fn check_timeout_ms(op: &str, ms: i64) -> Result<u64, Diagnostic> {
    if ms < 0 {
        return Err(io_err(op, &format!("{ms}"), &format!("timeout {ms}ms is negative")));
    }
    // Guard against absurd durations overflowing `Duration`.
    if ms > 24 * 3600 * 1000 {
        return Err(io_err(op, &format!("{ms}"), &format!("timeout {ms}ms exceeds 24h")));
    }
    Ok(ms as u64)
}

fn is_timeout_err(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock
}

fn map_bind_err(op: &str, target: &str, e: &std::io::Error) -> Diagnostic {
    match e.kind() {
        std::io::ErrorKind::AddrInUse => addr_in_use(op, target, &e.to_string()),
        std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::InvalidInput => {
            addr_invalid(op, target, &e.to_string())
        }
        _ => {
            let s = e.to_string();
            if s.contains("already in use") {
                addr_in_use(op, target, &s)
            } else if s.contains("invalid") || s.contains("cannot assign") {
                addr_invalid(op, target, &s)
            } else {
                io_err(op, target, &s)
            }
        }
    }
}

fn map_connect_err(op: &str, target: &str, e: &std::io::Error) -> Diagnostic {
    match e.kind() {
        std::io::ErrorKind::ConnectionRefused => refused(op, target, &e.to_string()),
        std::io::ErrorKind::TimedOut => timeout(op, target, &e.to_string()),
        std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::InvalidInput => {
            addr_invalid(op, target, &e.to_string())
        }
        _ => {
            let s = e.to_string();
            if s.contains("refused") {
                refused(op, target, &s)
            } else if s.contains("timed out") || s.contains("timeout") {
                timeout(op, target, &s)
            } else {
                io_err(op, target, &s)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hex helpers (internal; the public hex API is the `hex` package).
// ---------------------------------------------------------------------------

/// Lowercase hex of `bytes`.
pub fn hex_encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Decode lowercase/uppercase hex to bytes. Odd length or bad digits are
/// `E-NET-IO` naming (a redacted prefix of) the offending text.
pub fn hex_decode_str(op: &str, hex: &str) -> Result<Vec<u8>, Diagnostic> {
    if hex.len() % 2 != 0 {
        return Err(io_err(op, &redact_hex(hex), "odd hex length"));
    }
    let b = hex.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = hex_val(b[i]);
        let lo = hex_val(b[i + 1]);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h << 4) | l),
            _ => {
                return Err(io_err(op, &redact_hex(hex), "bad hex digit"));
            }
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
// Handle extractors (wrong-type errors name the clear type).
// ---------------------------------------------------------------------------

fn as_listener(op: &str, v: &Value) -> Result<ListenerHandle, Diagnostic> {
    match v {
        Value::Listener(h) => Ok(h.clone()),
        other => Err(crate::runtime::type_err(op, "a Listener", other)),
    }
}

fn as_conn(op: &str, v: &Value) -> Result<ConnHandle, Diagnostic> {
    match v {
        Value::Conn(h) => Ok(h.clone()),
        other => Err(crate::runtime::type_err(op, "a Conn", other)),
    }
}

fn as_udp(op: &str, v: &Value) -> Result<UdpHandle, Diagnostic> {
    match v {
        Value::UdpSock(h) => Ok(h.clone()),
        other => Err(crate::runtime::type_err(op, "a UdpSock", other)),
    }
}

// ---------------------------------------------------------------------------
// TCP.
// ---------------------------------------------------------------------------

/// `tcp_listen(host, port) -> Listener` (port 0 picks a free port).
pub fn tcp_listen(host: &str, port: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "tcp_listen";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let target = format!("{host}:{port}");
    if host.is_empty() || host.contains('\0') {
        return Err(addr_invalid(OP, &target, &format!("host {host:?} is empty or contains NUL")));
    }
    let p = check_port(OP, &target, port)?;
    let listener = TcpListener::bind(format!("{host}:{p}")).map_err(|e| map_bind_err(OP, &target, &e))?;
    Ok(Value::Listener(Arc::new(Mutex::new(ListenerState {
        listener: Some(listener),
        closed: false,
    }))))
}

/// `listener_port(l) -> Int`.
pub fn listener_port(v: &Value) -> Result<i64, Diagnostic> {
    const OP: &str = "listener_port";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_listener(OP, v)?;
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.listener.is_none() {
        return Err(closed(OP, "Listener"));
    }
    let addr = st.listener.as_ref().expect("checked").local_addr().map_err(|e| io_err(OP, "listener", &e.to_string()))?;
    Ok(addr.port() as i64)
}

/// `tcp_accept(l) -> Conn` (blocking).
pub fn tcp_accept(v: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "tcp_accept";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_listener(OP, v)?;
    // Hold the lock across the blocking accept: A1 is single-threaded
    // (no Klang threads yet), so no other holder can interleave.
    let (stream, addr) = {
        let st = h.lock().unwrap_or_else(|e| e.into_inner());
        if st.closed || st.listener.is_none() {
            return Err(closed(OP, "Listener"));
        }
        st.listener.as_ref().expect("checked").accept().map_err(|e| io_err(OP, "listener", &e.to_string()))?
    };
    Ok(make_conn(stream, addr))
}

/// `tcp_accept_timeout(l, ms) -> Conn` or null (timeout is `Null`, not an error).
pub fn tcp_accept_timeout(v: &Value, ms: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "tcp_accept_timeout";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let wait = check_timeout_ms(OP, ms)?;
    let h = as_listener(OP, v)?;
    {
        let st = h.lock().unwrap_or_else(|e| e.into_inner());
        if st.closed || st.listener.is_none() {
            return Err(closed(OP, "Listener"));
        }
        st.listener.as_ref().expect("checked").set_nonblocking(true).map_err(|e| io_err(OP, "listener", &e.to_string()))?;
    }
    let deadline = Instant::now() + Duration::from_millis(wait);
    let outcome: Result<Option<(TcpStream, SocketAddr)>, Diagnostic> = loop {
        let attempt = {
            let st = h.lock().unwrap_or_else(|e| e.into_inner());
            if st.closed || st.listener.is_none() {
                break Err(closed(OP, "Listener"));
            }
            st.listener.as_ref().expect("checked").accept()
        };
        match attempt {
            Ok((s, a)) => break Ok(Some((s, a))),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break Ok(None);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => break Err(io_err(OP, "listener", &e.to_string())),
        }
    };
    {
        let st = h.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = st.listener.as_ref() {
            let _ = l.set_nonblocking(false);
        }
    }
    match outcome? {
        None => Ok(Value::Null),
        Some((stream, addr)) => {
            let _ = stream.set_nonblocking(false);
            Ok(make_conn(stream, addr))
        }
    }
}

/// Build a `Value::Conn` from an accepted/connect socket (internal helper).
fn make_conn(stream: TcpStream, addr: SocketAddr) -> Value {
    Value::Conn(Arc::new(Mutex::new(ConnState {
        stream: Some(stream),
        buf: Vec::new(),
        closed: false,
        peer: addr.to_string(),
        requests: 0,
    })))
}

/// Blocking accept for the HTTP server acceptor. Unlike `tcp_accept` it
/// does NOT hold the listener mutex across the accept (the listener state
/// is duplicated under the lock and accepted outside it), so `server_port`
/// and `conn_close` never block behind a waiting acceptor. On Linux
/// `close()` does not wake a blocked accept — [`listener_wake`] is the
/// wake path.
pub fn listener_accept_blocking(v: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "listener_accept_blocking";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_listener(OP, v)?;
    let dup = {
        let st = h.lock().unwrap_or_else(|e| e.into_inner());
        if st.closed || st.listener.is_none() {
            return Err(closed(OP, "Listener"));
        }
        st.listener
            .as_ref()
            .expect("checked")
            .try_clone()
            .map_err(|e| io_err(OP, "listener", &e.to_string()))?
    };
    let (stream, addr) = dup.accept().map_err(|e| io_err(OP, "listener", &e.to_string()))?;
    Ok(make_conn(stream, addr))
}

/// One non-blocking accept attempt (drain helper for graceful shutdown):
/// `Ok(Null)` when the backlog is empty. Restores blocking mode so later
/// accepts block again.
pub fn listener_accept_nonblocking(v: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "listener_accept_nonblocking";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_listener(OP, v)?;
    let attempt = {
        let st = h.lock().unwrap_or_else(|e| e.into_inner());
        if st.closed || st.listener.is_none() {
            return Err(closed(OP, "Listener"));
        }
        let l = st.listener.as_ref().expect("checked");
        let _ = l.set_nonblocking(true);
        let r = l.accept();
        let _ = l.set_nonblocking(false);
        r
    };
    match attempt {
        Ok((stream, addr)) => Ok(make_conn(stream, addr)),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Value::Null),
        Err(e) => Err(io_err(OP, "listener", &e.to_string())),
    }
}

/// Wake a thread blocked in [`listener_accept_blocking`] by connecting to
/// the listener's own address (a `close()` does not wake a blocked accept
/// on Linux). The wake socket is dropped immediately, so the accepted end
/// reads instant EOF and can never stall a worker. Bounded 200 ms;
/// best-effort (a full backlog unblocks via the job queue draining, and
/// shutdown proceeds regardless).
pub fn listener_wake(v: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "http_stop";
    let h = as_listener(OP, v)?;
    let addr = {
        let st = h.lock().unwrap_or_else(|e| e.into_inner());
        if st.closed || st.listener.is_none() {
            return Ok(()); // nothing to wake (already closed)
        }
        st.listener
            .as_ref()
            .expect("checked")
            .local_addr()
            .map_err(|e| io_err(OP, "listener", &e.to_string()))?
    };
    let mut targets = vec![addr];
    if addr.ip().is_unspecified() {
        targets = vec![
            SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), addr.port()),
            SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), addr.port()),
        ];
    }
    for a in targets {
        // Drop the socket right away: the handshake-completed connection
        // still lands in the accept queue, but with instant EOF.
        if TcpStream::connect_timeout(&a, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
    }
    Ok(())
}

/// `tcp_connect(host, port, timeout_ms) -> Conn`.
pub fn tcp_connect(host: &str, port: i64, timeout_ms: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "tcp_connect";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let target = format!("{host}:{port}");
    if host.is_empty() || host.contains('\0') {
        return Err(addr_invalid(OP, &target, &format!("host {host:?} is empty or contains NUL")));
    }
    let p = check_port(OP, &target, port)?;
    let wait = check_timeout_ms(OP, timeout_ms)?;
    let addrs: Vec<SocketAddr> = (host, p).to_socket_addrs().map(|it| it.collect()).map_err(|e| dns_err(OP, host, &e.to_string()))?;
    if addrs.is_empty() {
        return Err(dns_err(OP, host, "no addresses returned"));
    }
    let mut last: Option<Diagnostic> = None;
    for addr in &addrs {
        let res = if wait == 0 {
            TcpStream::connect(addr).map_err(|e| map_connect_err(OP, &target, &e))
        } else {
            TcpStream::connect_timeout(addr, Duration::from_millis(wait)).map_err(|e| map_connect_err(OP, &target, &e))
        };
        match res {
            Ok(stream) => {
                return Ok(make_conn(stream, *addr));
            }
            Err(d) => {
                last = Some(d);
            }
        }
    }
    Err(last.unwrap_or_else(|| io_err(OP, &target, "connection failed")))
}

// ---------------------------------------------------------------------------
// Conn data path (shared buffered core).
// ---------------------------------------------------------------------------

/// Drain up to `want` bytes from `st.buf` (already-read leftovers).
fn take_from_buf(st: &mut ConnState, want: usize) -> Vec<u8> {
    let n = want.min(st.buf.len());
    st.buf.drain(..n).collect()
}

/// Core read: buffered leftovers first, then one socket round up to `max_n`.
/// Returns `(bytes, eof)` where `eof` means the peer closed and no bytes
/// remain. Timeout and local-close are errors, never `eof`. A single
/// syscall only: short reads return early (callers loop for exact sizes).
fn read_bytes(st: &mut ConnState, op: &str, max_n: usize) -> Result<(Vec<u8>, bool), Diagnostic> {
    if st.closed || st.stream.is_none() {
        return Err(closed(op, "Conn"));
    }
    if max_n == 0 {
        return Ok((Vec::new(), false));
    }
    // Buffered leftovers win without touching the socket (keeps
    // `conn_read` / `conn_read_line` coherent and never blocks when
    // bytes are already available).
    if !st.buf.is_empty() {
        return Ok((take_from_buf(st, max_n), false));
    }
    let mut chunk = vec![0u8; max_n.min(65536).max(1)];
    let peer = st.peer.clone();
    let n = match st.stream.as_mut() {
        None => return Err(closed(op, "Conn")),
        Some(s) => s.read(&mut chunk).map_err(|e| {
            if is_timeout_err(&e) {
                timeout(op, &peer, &e.to_string())
            } else {
                io_err(op, &peer, &e.to_string())
            }
        })?,
    };
    if n == 0 {
        return Ok((Vec::new(), true));
    }
    chunk.truncate(n);
    Ok((chunk, false))
}

/// `conn_read(c, max_bytes) -> Str` (lossy UTF-8; `""` at peer close).
pub fn conn_read(v: &Value, max_bytes: i64) -> Result<String, Diagnostic> {
    const OP: &str = "conn_read";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let max_n = check_max_bytes(OP, max_bytes, MAX_READ_BYTES)?;
    let h = as_conn(OP, v)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    let (bytes, _eof) = read_bytes(&mut st, OP, max_n)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// `conn_read_hex(c, max_bytes) -> Str` (lowercase hex; `""` at peer close).
pub fn conn_read_hex(v: &Value, max_bytes: i64) -> Result<String, Diagnostic> {
    const OP: &str = "conn_read_hex";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let max_n = check_max_bytes(OP, max_bytes, MAX_READ_BYTES)?;
    let h = as_conn(OP, v)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    let (bytes, _eof) = read_bytes(&mut st, OP, max_n)?;
    Ok(hex_encode_bytes(&bytes))
}

/// `conn_read_line(c) -> Str` without the newline, or null at peer close.
/// `"\n"` yields `""`; `"a\r\n"` yields `"a"`. A final partial line
/// without a newline is returned once; the next call is null.
pub fn conn_read_line(v: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "conn_read_line";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_conn(OP, v)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.stream.is_none() {
        return Err(closed(OP, "Conn"));
    }
    loop {
        if let Some(pos) = st.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = st.buf.drain(..=pos).collect();
            let mut s = line;
            s.pop();
            if s.last() == Some(&b'\r') {
                s.pop();
            }
            return Ok(Value::Str(String::from_utf8_lossy(&s).into_owned()));
        }
        let mut chunk = vec![0u8; 4096];
        let peer = st.peer.clone();
        let n = match st.stream.as_mut() {
            None => return Err(closed(OP, "Conn")),
            Some(s) => s.read(&mut chunk).map_err(|e| {
                if is_timeout_err(&e) {
                    timeout(OP, &peer, &e.to_string())
                } else {
                    io_err(OP, &peer, &e.to_string())
                }
            })?,
        };
        if n == 0 {
            if st.buf.is_empty() {
                return Ok(Value::Null);
            }
            let rest: Vec<u8> = std::mem::take(&mut st.buf);
            let mut s = rest;
            if s.last() == Some(&b'\n') {
                s.pop();
            }
            if s.last() == Some(&b'\r') {
                s.pop();
            }
            return Ok(Value::Str(String::from_utf8_lossy(&s).into_owned()));
        }
        st.buf.extend_from_slice(&chunk[..n]);
        if st.buf.len() > (MAX_READ_BYTES as usize) + 65536 {
            return Err(io_err(OP, &peer, "line exceeds 10 MB limit"));
        }
    }
}

/// `conn_write(c, s) -> Int` (bytes written).
pub fn conn_write(v: &Value, s: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "conn_write";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_conn(OP, v)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.stream.is_none() {
        return Err(closed(OP, "Conn"));
    }
    let peer = st.peer.clone();
    let stream = st.stream.as_mut().expect("checked");
    stream.write_all(s.as_bytes()).map_err(|e| {
        if is_timeout_err(&e) {
            timeout(OP, &peer, &e.to_string())
        } else {
            io_err(OP, &peer, &e.to_string())
        }
    })?;
    Ok(s.len() as i64)
}

/// `conn_write_hex(c, hex) -> Int` (bytes written).
pub fn conn_write_hex(v: &Value, hex: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "conn_write_hex";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let bytes = hex_decode_str(OP, hex)?;
    let h = as_conn(OP, v)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.stream.is_none() {
        return Err(closed(OP, "Conn"));
    }
    let peer = st.peer.clone();
    let stream = st.stream.as_mut().expect("checked");
    if !bytes.is_empty() {
        stream.write_all(&bytes).map_err(|e| {
            if is_timeout_err(&e) {
                timeout(OP, &peer, &e.to_string())
            } else {
                io_err(OP, &peer, &e.to_string())
            }
        })?;
    }
    Ok(bytes.len() as i64)
}

/// `conn_close(c)` over any handle kind (Conn, Listener, UdpSock).
/// Idempotent; returns `()` (the interpreter renders `1`).
pub fn conn_close(v: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "conn_close";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    match v {
        Value::Conn(h) => {
            let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
            if !st.closed {
                if let Some(s) = st.stream.take() {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
                st.buf.clear();
                st.closed = true;
            }
            Ok(())
        }
        Value::Listener(h) => {
            let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
            st.listener.take();
            st.closed = true;
            Ok(())
        }
        Value::UdpSock(h) => {
            let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
            st.socket.take();
            st.closed = true;
            Ok(())
        }
        other => Err(crate::runtime::type_err(OP, "a Conn, Listener or UdpSock", other)),
    }
}

/// `conn_set_timeout(c, ms)`: read/write deadlines for later calls.
/// `0` clears (blocking); negative is `E-NET-IO`.
pub fn conn_set_timeout(v: &Value, ms: i64) -> Result<(), Diagnostic> {
    const OP: &str = "conn_set_timeout";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let wait = check_timeout_ms(OP, ms)?;
    let h = as_conn(OP, v)?;
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.stream.is_none() {
        return Err(closed(OP, "Conn"));
    }
    let dur = if wait == 0 { None } else { Some(Duration::from_millis(wait)) };
    let s = st.stream.as_ref().expect("checked");
    s.set_read_timeout(dur).map_err(|e| io_err(OP, &st.peer, &e.to_string()))?;
    s.set_write_timeout(dur).map_err(|e| io_err(OP, &st.peer, &e.to_string()))?;
    Ok(())
}

/// Served-request count for the HTTP keep-alive cap (internal helper).
pub fn conn_requests(v: &Value) -> Result<u64, Diagnostic> {
    let h = match v {
        Value::Conn(h) => h.clone(),
        other => return Err(crate::runtime::type_err("conn_requests", "a Conn", other)),
    };
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    Ok(st.requests)
}

/// Bump the served-request count; returns the new value (internal helper).
pub fn conn_bump_requests(v: &Value) -> Result<u64, Diagnostic> {
    let h = match v {
        Value::Conn(h) => h.clone(),
        other => return Err(crate::runtime::type_err("conn_bump_requests", "a Conn", other)),
    };
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    st.requests += 1;
    Ok(st.requests)
}

/// What a non-consuming readiness probe found (internal helper, HTTP idle
/// poller + worker fast path). Uses `TcpStream::peek`, so nothing is
/// consumed and no buffered partial line is touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnPoll {
    /// Bytes are readable (a request, complete or partial, has arrived).
    Data,
    /// Peer closed (EOF).
    Eof,
    /// Nothing readable within `timeout_ms`.
    Timeout,
}

/// Peek the socket with a short read timeout; does NOT consume bytes and
/// does not disturb buffered data. Only the READ timeout is touched (the
/// write timeout is left alone, so a caller mid-write is unaffected).
pub fn conn_poll_readable(v: &Value, timeout_ms: i64) -> Result<ConnPoll, Diagnostic> {
    const OP: &str = "conn_poll_readable";
    let h = match v {
        Value::Conn(h) => h.clone(),
        other => return Err(crate::runtime::type_err(OP, "a Conn", other)),
    };
    let wait = check_timeout_ms(OP, timeout_ms)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.stream.is_none() {
        return Err(closed(OP, "Conn"));
    }
    // A leftover from an earlier chunked read counts as readable: the
    // next (or partial next) request is already in memory — parking this
    // conn would orphan it (the kernel queue can be empty meanwhile).
    if !st.buf.is_empty() {
        return Ok(ConnPoll::Data);
    }
    let dur = if wait == 0 { None } else { Some(Duration::from_millis(wait)) };
    let peer = st.peer.clone();
    let s = st.stream.as_mut().expect("checked");
    let res = {
        s.set_read_timeout(dur).map_err(|e| io_err(OP, &peer, &e.to_string()))?;
        let mut b = [0u8; 1];
        s.peek(&mut b)
    };
    match res {
        Ok(0) => Ok(ConnPoll::Eof),
        Ok(_) => Ok(ConnPoll::Data),
        Err(e) if is_timeout_err(&e) => Ok(ConnPoll::Timeout),
        Err(e) => Err(io_err(OP, &peer, &e.to_string())),
    }
}

/// `conn_peer(c) -> Str` (`"ip:port"`).
pub fn conn_peer(v: &Value) -> Result<String, Diagnostic> {
    const OP: &str = "conn_peer";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_conn(OP, v)?;
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.stream.is_none() {
        return Err(closed(OP, "Conn"));
    }
    Ok(st.peer.clone())
}

// ---------------------------------------------------------------------------
// UDP.
// ---------------------------------------------------------------------------

/// `udp_bind(host, port) -> Sock` (port 0 picks a free port).
pub fn udp_bind(host: &str, port: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "udp_bind";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let target = format!("{host}:{port}");
    if host.is_empty() || host.contains('\0') {
        return Err(addr_invalid(OP, &target, &format!("host {host:?} is empty or contains NUL")));
    }
    let p = check_port(OP, &target, port)?;
    let sock = UdpSocket::bind(format!("{host}:{p}")).map_err(|e| map_bind_err(OP, &target, &e))?;
    Ok(Value::UdpSock(Arc::new(Mutex::new(UdpState { socket: Some(sock), closed: false }))))
}

/// `udp_send_to(sock, host, port, s) -> Int` (bytes sent).
pub fn udp_send_to(sock: &Value, host: &str, port: i64, s: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "udp_send_to";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let target = format!("{host}:{port}");
    if host.is_empty() || host.contains('\0') {
        return Err(addr_invalid(OP, &target, &format!("host {host:?} is empty or contains NUL")));
    }
    let p = check_port(OP, &target, port)?;
    let h = as_udp(OP, sock)?;
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.socket.is_none() {
        return Err(closed(OP, "UdpSock"));
    }
    let addrs: Vec<SocketAddr> =
        (host, p).to_socket_addrs().map(|it| it.collect()).map_err(|e| dns_err(OP, host, &e.to_string()))?;
    let addr = addrs.into_iter().next().ok_or_else(|| dns_err(OP, host, "no addresses returned"))?;
    let n = st.socket.as_ref().expect("checked").send_to(s.as_bytes(), addr).map_err(|e| io_err(OP, &target, &e.to_string()))?;
    Ok(n as i64)
}

/// `udp_recv_from(sock, max_bytes, timeout_ms) -> {data, host, port}`.
/// Timeout is `E-NET-TIMEOUT`, never silent.
pub fn udp_recv_from(sock: &Value, max_bytes: i64, timeout_ms: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "udp_recv_from";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let max_n = check_max_bytes(OP, max_bytes, MAX_UDP_BYTES)?;
    let wait = check_timeout_ms(OP, timeout_ms)?;
    let h = as_udp(OP, sock)?;
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed || st.socket.is_none() {
        return Err(closed(OP, "UdpSock"));
    }
    let socket = st.socket.as_ref().expect("checked");
    let dur = if wait == 0 { None } else { Some(Duration::from_millis(wait)) };
    socket.set_read_timeout(dur).map_err(|e| io_err(OP, "socket", &e.to_string()))?;
    let mut buf = vec![0u8; max_n.max(1)];
    match socket.recv_from(&mut buf) {
        Ok((n, addr)) => {
            let data = String::from_utf8_lossy(&buf[..n]).into_owned();
            Ok(Value::Map(vec![
                ("data".to_string(), Value::Str(data)),
                ("host".to_string(), Value::Str(addr.ip().to_string())),
                ("port".to_string(), Value::Int(addr.port() as i64)),
            ]))
        }
        Err(e) if is_timeout_err(&e) => Err(timeout(OP, "socket", &e.to_string())),
        Err(e) => Err(io_err(OP, "socket", &e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// DNS.
// ---------------------------------------------------------------------------

/// `dns_lookup(host) -> array of ip strings` (sorted, deduped).
pub fn dns_lookup(host: &str) -> Result<Vec<String>, Diagnostic> {
    const OP: &str = "dns_lookup";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    if host.is_empty() || host.contains('\0') {
        return Err(dns_err(OP, host, &format!("host {host:?} is empty or contains NUL")));
    }
    let addrs: Vec<SocketAddr> =
        (host, 0).to_socket_addrs().map(|it| it.collect()).map_err(|e| dns_err(OP, host, &e.to_string()))?;
    if addrs.is_empty() {
        return Err(dns_err(OP, host, "no addresses returned"));
    }
    let mut ips: Vec<String> = addrs.into_iter().map(|a| a.ip().to_string()).collect();
    ips.sort();
    ips.dedup();
    Ok(ips)
}
