//! KLANG-FOUNDATION-3 Part 2C — stdio JSON-RPC language server gates.
//!
//! v1 scope: diagnostics-as-you-type over the exact `klang check`
//! pipeline, with REAL line/character positions from byte spans.
//! Hover/go-to-definition are planned v2 (loud `MethodNotFound`, never
//! a hang). Whole-file re-check per settled change with a 100 ms
//! debounce; `examples/eval.klang` re-checks at ~39 ms p95, far under
//! the ~200 ms budget that would force the Salsa path instead.
//!
//! BUG-A discipline: the silent failure is a WRONG position (the old
//! stub hardcoded `line: 0`) or a swallowed diagnostic, so positions
//! are asserted exactly — including UTF-16 character units and clamped
//! out-of-range spans — and the end-to-end tests drive the real binary
//! over raw framed JSON-RPC, asserting the published line/column of an
//! intentional type error.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use klang::diagnostics::{Diagnostic, Span};

// ---------------------------------------------------------------------------
// Unit: byte offsets -> LSP positions (the stub hardcoded line 0)
// ---------------------------------------------------------------------------

#[test]
fn positions_first_line() {
    assert_eq!(klang::lsp::byte_offset_to_position("abc", 0), (0, 0));
    assert_eq!(klang::lsp::byte_offset_to_position("abc", 1), (0, 1));
    assert_eq!(klang::lsp::byte_offset_to_position("abc", 3), (0, 3));
}

#[test]
fn positions_multiline_exact() {
    // Offsets 0,1='a','b' 2='\n' 3..5='c','d','e' 6='\n' 7='f'.
    let t = "ab\ncde\nf";
    assert_eq!(klang::lsp::byte_offset_to_position(t, 0), (0, 0));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 2), (0, 2));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 3), (1, 0));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 5), (1, 2));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 7), (2, 0));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 8), (2, 1));
}

#[test]
fn positions_crlf_counts_one_break() {
    // `\r\n` is one line break; the `\r` is not a character of either line.
    let t = "ab\r\ncde";
    assert_eq!(klang::lsp::byte_offset_to_position(t, 2), (0, 2));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 4), (1, 0));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 5), (1, 1));
}

#[test]
fn positions_unicode_utf16_units() {
    // `é` is 1 UTF-16 unit; `𝄞` (U+1D11E) is a surrogate pair = 2 units.
    // A `chars().count()` implementation would report 1 for both and
    // drift editors on astral-plane text.
    let t = "aé𝄞b";
    assert_eq!(klang::lsp::byte_offset_to_position(t, 1), (0, 1));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 3), (0, 2));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 7), (0, 4));
    assert_eq!(klang::lsp::byte_offset_to_position(t, 8), (0, 5));
}

#[test]
fn positions_clamp_and_snap() {
    // Past-end clamps to end-of-document (never panics); mid-char
    // offsets snap back to the containing boundary (never split a scalar).
    assert_eq!(klang::lsp::byte_offset_to_position("ab", 99), (0, 2));
    assert_eq!(klang::lsp::byte_offset_to_position("", 5), (0, 0));
    // "aé": bytes 0='a', 1..3='é'. Offset 2 is mid-char -> snaps to 1.
    assert_eq!(klang::lsp::byte_offset_to_position("aé", 2), (0, 1));
}

// ---------------------------------------------------------------------------
// Unit: diagnostic rendering (message content = klang check format)
// ---------------------------------------------------------------------------

fn sample_diag() -> Diagnostic {
    Diagnostic {
        code: "E-TYPE".to_string(),
        severity: "error".to_string(),
        message: "mismatch".to_string(),
        primary_span: Span {
            file: "f.klang".to_string(),
            start: 3,
            end: 5,
        },
        cause: String::new(),
        expected: None,
        found: None,
        fixes: Vec::new(),
        rule: String::new(),
        related: Vec::new(),
    }
}

#[test]
fn diagnostic_maps_real_range_not_zero() {
    // "ab\ncde": bytes 3..5 = "cd" on line 1, characters 0..2.
    let j = klang::lsp::diagnostic_to_lsp_json(&sample_diag(), "ab\ncde");
    assert!(j.contains("\"line\":1"), "{j}");
    assert!(j.contains("\"start\":{\"line\":1,\"character\":0}"), "{j}");
    assert!(j.contains("\"end\":{\"line\":1,\"character\":2}"), "{j}");
    // The stub this replaced always emitted line 0: this exact shape
    // would have failed there.
    assert!(!j.contains("\"line\":0"), "{j}");
}

#[test]
fn diagnostic_carries_code_severity_source_and_check_json() {
    let j = klang::lsp::diagnostic_to_lsp_json(&sample_diag(), "ab\ncde");
    assert!(j.contains("\"severity\":1"), "{j}");
    assert!(j.contains("\"code\":\"E-TYPE\""), "{j}");
    assert!(j.contains("\"source\":\"klang\""), "{j}");
    assert!(j.contains("\"message\":\"mismatch\""), "{j}");
    // `data` IS the `klang check` diagnostic JSON verbatim.
    assert!(j.contains("\"data\":{"), "{j}");
    assert!(j.contains("\"primary_span\""), "{j}");
}

#[test]
fn severity_mapping_never_silent() {
    assert_eq!(klang::lsp::severity_to_lsp("error"), 1);
    assert_eq!(klang::lsp::severity_to_lsp("warning"), 2);
    assert_eq!(klang::lsp::severity_to_lsp("info"), 3);
    assert_eq!(klang::lsp::severity_to_lsp("hint"), 4);
    assert_eq!(klang::lsp::severity_to_lsp("weird"), 1);
}

// ---------------------------------------------------------------------------
// Unit: URIs and document checking
// ---------------------------------------------------------------------------

#[test]
fn uri_mapping() {
    assert_eq!(
        klang::lsp::uri_to_path("file:///tmp/x.klang"),
        Some("/tmp/x.klang".to_string())
    );
    assert_eq!(klang::lsp::uri_to_path("untitled:1"), None);
    assert_eq!(klang::lsp::uri_to_path("file://"), None);
    assert_eq!(klang::lsp::uri_to_path("file:///tmp/a%20b.klang"), Some("/tmp/a b.klang".to_string()));
    assert_eq!(klang::lsp::uri_to_path("file:///tmp/%ZZ.klang"), None);
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "klang-lsp-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn check_document_clean_and_dirty() {
    // On-disk file: the BUFFER is what gets checked (disk holds stale
    // clean text; the dirty buffer's type error must still surface).
    let dir = tmpdir("basic");
    let path = dir.join("main.klang");
    std::fs::write(&path, "fn main() -> i32 {\n    return 1\n}\n").unwrap();
    let path_s = path.to_string_lossy().to_string();
    assert!(klang::lsp::check_document(
        Some(&path_s),
        "fn main() -> i32 {\n    return 1\n}\n"
    )
    .is_empty());
    let diags = klang::lsp::check_document(
        Some(&path_s),
        "fn main() -> i32 {\n    return \"hi\"\n}\n",
    );
    assert!(diags.iter().any(|d| d.code == "E-TYPE"), "{diags:?}");
    // Untitled (no path): standalone check still reports real spans.
    let diags = klang::lsp::check_document(None, "fn main() -> i32 {\n    return \"hi\"\n}\n");
    assert!(diags.iter().any(|d| d.code == "E-TYPE"), "{diags:?}");
    let d = diags.iter().find(|d| d.code == "E-TYPE").unwrap();
    let (line, _) = klang::lsp::byte_offset_to_position(
        "fn main() -> i32 {\n    return \"hi\"\n}\n",
        d.primary_span.start,
    );
    assert_eq!(line, 1, "type error is on the second line: {d:?}");
}

#[test]
fn check_document_uses_buffer_imports_from_disk() {
    // The buffer's own `import` resolves against saved files on disk:
    // loader + checker identical to `klang check`.
    let dir = tmpdir("imports");
    std::fs::write(dir.join("lib.klang"), "fn helper(n: i32) -> i32 {\n    return n + 1\n}\n").unwrap();
    let main = dir.join("main.klang");
    std::fs::write(&main, "fn main() -> i32 {\n    return 0\n}\n").unwrap();
    let main_s = main.to_string_lossy().to_string();
    let diags = klang::lsp::check_document(
        Some(&main_s),
        "import \"lib.klang\"\nfn main() -> i32 {\n    return helper(41)\n}\n",
    );
    assert!(diags.is_empty(), "{diags:?}");
    // And a mistyped call through the import fails loudly.
    let diags = klang::lsp::check_document(
        Some(&main_s),
        "import \"lib.klang\"\nfn main() -> i32 {\n    return helper(\"x\")\n}\n",
    );
    assert!(diags.iter().any(|d| d.code == "E-TYPE"), "{diags:?}");
}

// ---------------------------------------------------------------------------
// End-to-end: the real binary over raw framed JSON-RPC
// ---------------------------------------------------------------------------

struct Session {
    child: Child,
    stdin: ChildStdin,
    inbox: mpsc::Receiver<String>,
}

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// Blocking framed read (runs on the reader thread; the main test
/// thread receives with a timeout so a hung server fails, never hangs).
fn read_framed_blocking(reader: &mut BufReader<std::process::ChildStdout>) -> Option<String> {
    let mut content_len: Option<usize> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t.strip_prefix("Content-Length:") {
            content_len = v.trim().parse().ok();
        }
    }
    let n = content_len?;
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

impl Session {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_klang"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("klang lsp spawns");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(body) = read_framed_blocking(&mut reader) {
                if tx.send(body).is_err() {
                    break;
                }
            }
        });
        Self { child, stdin, inbox: rx }
    }

    fn send(&mut self, body: &str) {
        let bytes = frame(body);
        self.stdin.write_all(&bytes).expect("write frame");
        self.stdin.flush().expect("flush");
    }

    fn recv(&mut self) -> String {
        self.inbox
            .recv_timeout(Duration::from_secs(15))
            .expect("lsp server answered within 15s")
    }

    fn stop(mut self) {
        self.send("{\"jsonrpc\":\"2.0\",\"id\":999,\"method\":\"shutdown\"}");
        let resp = self.recv();
        assert!(resp.contains("\"id\":999"), "{resp}");
        self.send("{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}");
        let status = self.child.wait_timeout().expect("wait");
        let _ = status;
    }
}

trait WaitTimeout {
    fn wait_timeout(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl WaitTimeout for Child {
    fn wait_timeout(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.try_wait()? {
                Some(s) => return Ok(Some(s)),
                None if Instant::now() >= deadline => {
                    self.kill()?;
                    return Ok(self.try_wait()?);
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

/// First `"line":N` inside the FIRST publishDiagnostics body for `uri`.
fn publish_line(body: &str) -> Option<u32> {
    let marker = "\"method\":\"textDocument/publishDiagnostics\"";
    let at = body.find(marker)?;
    let after = &body[at..];
    let l = after.find("\"line\":")?;
    after[l + 7..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

fn publish_count(body: &str) -> usize {
    // Counts diagnostics by `"code":"E-` occurrences inside the
    // diagnostics array (server `code` fields always carry E-* codes).
    body.match_indices("\"code\":\"E-").count()
}

#[test]
fn e2e_initialize_reports_sync_capability() {
    let mut s = Session::start();
    s.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}");
    let resp = s.recv();
    assert!(resp.contains("\"id\":1"), "{resp}");
    assert!(resp.contains("\"textDocumentSync\":1"), "{resp}");
    assert!(resp.contains("klang"), "{resp}");
    s.send("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}");
    s.stop();
}

#[test]
fn e2e_type_error_publishes_real_line_and_column() {
    // THE Part 2C proof: an intentional type error on line 2 publishes
    // its REAL line (1, 0-based) via raw JSON-RPC — not line 0.
    let dir = tmpdir("e2e");
    let path = dir.join("main.klang");
    std::fs::write(&path, "fn main() -> i32 {\n    return 1\n}\n").unwrap();
    let uri = format!("file://{}", path.to_string_lossy());
    let mut s = Session::start();
    s.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}");
    let _ = s.recv();
    s.send("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}");
    let bad = "fn main() -> i32 {\\n    return \\\"hi\\\"\\n}\\n";
    s.send(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\",\"languageId\":\"klang\",\"version\":1,\"text\":\"{bad}\"}}}}}}"
    ));
    let mut got_error = false;
    for _ in 0..5 {
        let body = s.recv();
        if body.contains("publishDiagnostics") && body.contains(&uri) {
            let n = publish_count(&body);
            if n > 0 {
                assert!(body.contains("E-TYPE"), "{body}");
                assert_eq!(publish_line(&body), Some(1), "error is on line 2 (0-based 1): {body}");
                // Real column too: the error is NOT at line start.
                assert!(!body.contains("\"start\":{\"line\":1,\"character\":0}"), "{body}");
                got_error = true;
                break;
            }
        }
    }
    assert!(got_error, "expected a publishDiagnostics with the type error");
    // Fix it: the next publish for the URI carries zero diagnostics.
    let good = "fn main() -> i32 {\\n    return 1\\n}\\n";
    s.send(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\",\"version\":2}},\"contentChanges\":[{{\"text\":\"{good}\"}}]}}}}"
    ));
    let mut got_clean = false;
    for _ in 0..5 {
        let body = s.recv();
        if body.contains("publishDiagnostics") && body.contains(&uri) && publish_count(&body) == 0 {
            got_clean = true;
            break;
        }
    }
    assert!(got_clean, "expected an empty publishDiagnostics after the fix");
    s.stop();
}

#[test]
fn e2e_hover_and_definition_are_loud_v2_not_hangs() {
    let mut s = Session::start();
    s.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}");
    let _ = s.recv();
    s.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"textDocument/hover\",\"params\":{\"textDocument\":{\"uri\":\"file:///x.klang\"},\"position\":{\"line\":0,\"character\":0}}}");
    let resp = s.recv();
    assert!(resp.contains("-32601"), "{resp}");
    assert!(resp.contains("v2"), "{resp}");
    s.send("{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"textDocument/definition\",\"params\":{\"textDocument\":{\"uri\":\"file:///x.klang\"},\"position\":{\"line\":0,\"character\":0}}}");
    let resp = s.recv();
    assert!(resp.contains("-32601"), "{resp}");
    s.stop();
}

#[test]
fn e2e_close_publishes_empty_and_garbage_never_kills_server() {
    let mut s = Session::start();
    s.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}");
    let _ = s.recv();
    // Malformed JSON and unknown methods must not kill the server.
    s.send("this is not json");
    s.send("{\"jsonrpc\":\"2.0\",\"method\":\"unknown/notification\",\"params\":{}}");
    s.send("{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"unknown/method\",\"params\":{}}");
    let resp = s.recv();
    assert!(resp.contains("-32601"), "{resp}");
    // Open then close: close publishes an empty diagnostic set.
    let uri = "file:///tmp/never-saved-lsp-close.klang";
    s.send(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\",\"languageId\":\"klang\",\"version\":1,\"text\":\"fn main() -> i32 {{\\n    return 1\\n}}\\n\"}}}}}}"
    ));
    // Drain the publish for the open (clean file: zero diagnostics).
    for _ in 0..5 {
        let body = s.recv();
        if body.contains("publishDiagnostics") && body.contains(uri) {
            break;
        }
    }
    s.send(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didClose\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\"}}}}}}"
    ));
    let mut got_close = false;
    for _ in 0..5 {
        let body = s.recv();
        if body.contains("publishDiagnostics") && body.contains(uri) && publish_count(&body) == 0 {
            got_close = true;
            break;
        }
    }
    assert!(got_close, "didClose must publish an empty set");
    s.stop();
}
