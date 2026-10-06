//! Backblaze B2 (S3-compatible) storage backend gates.
//!
//! Everything here runs offline: SigV4 is checked against published AWS
//! test vectors, and the S3-compatible backend is exercised against an
//! in-process mock S3 server (std-only TCP listener) that stores objects
//! in memory. The only network test is opt-in (`KLANG_LIVE_TESTS=1` with
//! real B2 variables) and is skipped by default.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use klang::registry_storage::{
    backend_from_env, derive_region, parse_list_xml, sign_v4, B2Config,
    LocalDiskStorage, PutOutcome, S3Storage, Storage, StorageBackend,
};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "klang-b2-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Serializes tests that mutate the process environment.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(())).lock().unwrap()
}

// ---------------------------------------------------------------------------
// SigV4 against published AWS vectors (offline)
// ---------------------------------------------------------------------------

#[test]
fn sigv4_get_vanilla_vector() {
    // From the AWS Signature Version 4 test suite (`get-vanilla`):
    // https://docs.aws.amazon.com/general/latest/gr/signature-v4-test-suite.html
    let sig = sign_v4(
        "GET",
        "/",
        &[],
        &[
            ("host".to_string(), "example.amazonaws.com".to_string()),
            ("x-amz-date".to_string(), "20150830T123600Z".to_string()),
        ],
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "AKIDEXAMPLE",
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        "us-east-1",
        "service",
        "20150830T123600Z",
        "20150830",
    );
    assert_eq!(
        sig.canonical_request,
        "GET\n/\n\nhost:example.amazonaws.com\nx-amz-date:20150830T123600Z\n\nhost;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sig.authorization,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    );
}

#[test]
fn sigv4_s3_range_get_vector() {
    // S3 `GET Object` with a Range header, from the S3 SigV4 docs.
    let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let sig = sign_v4(
        "GET",
        "/test.txt",
        &[],
        &[
            ("host".to_string(), "examplebucket.s3.amazonaws.com".to_string()),
            ("range".to_string(), "bytes=0-9".to_string()),
            ("x-amz-content-sha256".to_string(), empty.to_string()),
            ("x-amz-date".to_string(), "20130524T000000Z".to_string()),
        ],
        empty,
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        "us-east-1",
        "s3",
        "20130524T000000Z",
        "20130524",
    );
    assert_eq!(sig.signed_headers, "host;range;x-amz-content-sha256;x-amz-date");
    assert!(
        sig.authorization.ends_with(
            "Signature=67fe34c8530db585abddc51067328adfedb6e42487d2566dc7d927d6e2722900"
        ),
        "unexpected auth: {}",
        sig.authorization
    );
    assert!(sig.authorization.contains(
        "Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request"
    ));
}

#[test]
fn sigv4_list_xml_parses_minimally() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>b</Name><Prefix>packages/</Prefix><KeyCount>2</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>false</IsTruncated><Contents><Key>packages/a/1.0.0.klangpkg</Key></Contents><Contents><Key>packages/a/1.0.0.sha256</Key></Contents></ListBucketResult>"#;
    let (keys, truncated, next) = parse_list_xml(xml);
    assert_eq!(keys, vec!["packages/a/1.0.0.klangpkg", "packages/a/1.0.0.sha256"]);
    assert!(!truncated);
    assert!(next.is_none());
}

// ---------------------------------------------------------------------------
// LocalDisk behind the trait
// ---------------------------------------------------------------------------

#[test]
fn local_disk_trait_roundtrip() {
    let root = tmp("local");
    let s = LocalDiskStorage::new(root);
    assert_eq!(s.get("a/b").unwrap(), None);
    assert!(!s.exists("a/b").unwrap());
    assert_eq!(s.put_if_absent("a/b", b"one").unwrap(), PutOutcome::Created);
    assert_eq!(s.get("a/b").unwrap(), Some(b"one".to_vec()));
    assert!(s.exists("a/b").unwrap());
    // Idempotent republish of identical bytes.
    assert_eq!(
        s.put_if_absent("a/b", b"one").unwrap(),
        PutOutcome::AlreadyExistsSame
    );
    // Conflict on different bytes.
    assert_eq!(
        s.put_if_absent("a/b", b"two").unwrap(),
        PutOutcome::AlreadyExistsDifferent
    );
    // Unconditional overwrite still works at the object layer.
    s.put("a/b", b"two").unwrap();
    assert_eq!(s.get("a/b").unwrap(), Some(b"two".to_vec()));
    s.put("a/c", b"x").unwrap();
    s.put("z", b"y").unwrap();
    assert_eq!(s.list("").unwrap(), vec!["a/b", "a/c", "z"]);
    assert_eq!(s.list("a/").unwrap(), vec!["a/b", "a/c"]);
    assert_eq!(s.list("nope/").unwrap(), Vec::<String>::new());
}

#[test]
fn local_disk_rejects_traversal() {
    let root = tmp("local-trav");
    let s = LocalDiskStorage::new(root);
    for bad in ["../evil", "/abs", "a/../../b", "a\\b", "", "a//b"] {
        assert!(s.get(bad).is_err(), "get accepts `{bad}`");
        assert!(s.put(bad, b"x").is_err(), "put accepts `{bad}`");
        assert!(s.put_if_absent(bad, b"x").is_err(), "pia accepts `{bad}`");
        assert!(s.exists(bad).is_err(), "exists accepts `{bad}`");
    }
}

// ---------------------------------------------------------------------------
// Mock S3 server (std-only): GET / PUT / HEAD / DELETE + ListObjectsV2
// ---------------------------------------------------------------------------

struct MockS3 {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    auth_headers: Mutex<Vec<String>>,
    requests: Mutex<u64>,
    /// Next N object GETs fail with 500 (retry test).
    fail_next_gets: Mutex<usize>,
}

impl MockS3 {
    fn delete(&self, key: &str) {
        self.objects.lock().unwrap().remove(key);
    }
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if b[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            m.insert(percent_decode(k), percent_decode(v));
        }
    }
    m
}

fn respond(stream: &mut std::net::TcpStream, status: u16, reason: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle_mock(mock: &MockS3, mut stream: std::net::TcpStream) {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(p) => p,
        None => return,
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.lines();
    let rl = lines.next().unwrap_or("").to_string();
    let mut parts = rl.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = buf[end + 4..].to_vec();
    while body.len() < len {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    body.truncate(len);
    *mock.requests.lock().unwrap() += 1;

    // Every S3 request must carry a SigV4 Authorization header.
    let auth = headers.get("authorization").cloned().unwrap_or_default();
    mock.auth_headers.lock().unwrap().push(auth.clone());
    if !auth.starts_with("AWS4-HMAC-SHA256 ") {
        respond(&mut stream, 403, "Forbidden", b"missing SigV4 auth");
        return;
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (percent_decode(p), parse_query(q)),
        None => (percent_decode(&target), HashMap::new()),
    };
    let bucket = "/test-bucket";
    let rest = match path.strip_prefix(bucket) {
        Some(r) => r.to_string(),
        None => {
            respond(&mut stream, 404, "Not Found", b"no bucket");
            return;
        }
    };
    // ListObjectsV2.
    if (rest.is_empty() || rest == "/") && query.get("list-type").map(|s| s.as_str()) == Some("2") {
        let prefix = query.get("prefix").cloned().unwrap_or_default();
        let max: usize = query
            .get("max-keys")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);
        let start: usize = query
            .get("continuation-token")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let objs = mock.objects.lock().unwrap();
        let mut keys: Vec<String> = objs.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        keys.sort();
        let page: Vec<String> = keys.iter().skip(start).take(max).cloned().collect();
        let truncated = start + page.len() < keys.len();
        let mut xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>test-bucket</Name><Prefix>{prefix}</Prefix><KeyCount>{}</KeyCount><MaxKeys>{max}</MaxKeys><IsTruncated>{}</IsTruncated>"#,
            page.len(),
            if truncated { "true" } else { "false" }
        );
        for k in &page {
            xml.push_str(&format!("<Contents><Key>{k}</Key></Contents>"));
        }
        if truncated {
            xml.push_str(&format!("<NextContinuationToken>{}</NextContinuationToken>", start + page.len()));
        }
        xml.push_str("</ListBucketResult>");
        respond(&mut stream, 200, "OK", xml.as_bytes());
        return;
    }
    let key = rest.trim_start_matches('/').to_string();
    if key.is_empty() || key.contains("//") {
        respond(&mut stream, 400, "Bad Request", b"bad key");
        return;
    }
    match method.as_str() {
        "GET" => {
            let mut fail = mock.fail_next_gets.lock().unwrap();
            if *fail > 0 {
                *fail -= 1;
                respond(&mut stream, 500, "Server Error", b"flaky");
                return;
            }
            match mock.objects.lock().unwrap().get(&key).cloned() {
                Some(b) => respond(&mut stream, 200, "OK", &b),
                None => respond(&mut stream, 404, "Not Found", b"no key"),
            }
        }
        "HEAD" => {
            let head = format!(
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                if mock.objects.lock().unwrap().contains_key(&key) {
                    "200 OK"
                } else {
                    "404 Not Found"
                },
                "",
                0
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.flush();
        }
        "PUT" => {
            mock.objects.lock().unwrap().insert(key, body);
            respond(&mut stream, 200, "OK", b"");
        }
        "DELETE" => {
            mock.objects.lock().unwrap().remove(&key);
            respond(&mut stream, 204, "No Content", b"");
        }
        _ => respond(&mut stream, 400, "Bad Request", b"bad method"),
    }
}

struct MockHandle {
    addr: String,
    mock: Arc<MockS3>,
}

fn start_mock() -> MockHandle {
    let listener = TcpListener::bind("127.0.0.1:0").expect("mock binds");
    let port = listener.local_addr().unwrap().port();
    let mock = Arc::new(MockS3 {
        objects: Mutex::new(HashMap::new()),
        auth_headers: Mutex::new(Vec::new()),
        requests: Mutex::new(0),
        fail_next_gets: Mutex::new(0),
    });
    let m = mock.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(s) = stream {
                handle_mock(&m, s);
            }
        }
    });
    MockHandle {
        addr: format!("127.0.0.1:{port}"),
        mock,
    }
}

fn s3_to_mock(h: &MockHandle) -> S3Storage {
    S3Storage::new(
        "TESTKEYID".to_string(),
        "TESTSECRET".to_string(),
        "test-bucket".to_string(),
        format!("http://{}", h.addr),
        "us-east-1".to_string(),
    )
    .expect("mock S3 config")
}

fn all_auth_signed(h: &MockHandle) {
    let seen = h.mock.auth_headers.lock().unwrap();
    assert!(!seen.is_empty(), "mock saw no requests");
    for a in seen.iter() {
        assert!(a.starts_with("AWS4-HMAC-SHA256 "), "unsigned request: {a}");
    }
    assert!(
        seen.iter().any(|a| a.contains("Credential=TESTKEYID/")),
        "key id never sent"
    );
}

// ---------------------------------------------------------------------------
// S3Storage against the mock (trait semantics)
// ---------------------------------------------------------------------------

#[test]
fn s3_trait_roundtrip() {
    let h = start_mock();
    let s = s3_to_mock(&h);
    assert_eq!(s.get("a/b").unwrap(), None);
    assert!(!s.exists("a/b").unwrap());
    assert_eq!(s.put_if_absent("a/b", b"one").unwrap(), PutOutcome::Created);
    assert_eq!(s.get("a/b").unwrap(), Some(b"one".to_vec()));
    assert!(s.exists("a/b").unwrap());
    assert_eq!(
        s.put_if_absent("a/b", b"one").unwrap(),
        PutOutcome::AlreadyExistsSame
    );
    assert_eq!(
        s.put_if_absent("a/b", b"two").unwrap(),
        PutOutcome::AlreadyExistsDifferent
    );
    s.put("a/c", b"x").unwrap();
    s.put("z", b"y").unwrap();
    assert_eq!(s.list("").unwrap(), vec!["a/b", "a/c", "z"]);
    assert_eq!(s.list("a/").unwrap(), vec!["a/b", "a/c"]);
    assert_eq!(s.list("nope/").unwrap(), Vec::<String>::new());
    for bad in ["../evil", "/abs", "a\\b", ""] {
        assert!(s.get(bad).is_err(), "get accepts `{bad}`");
    }
    all_auth_signed(&h);
}

#[test]
fn s3_retries_transient_never_4xx() {
    let h = start_mock();
    let s = s3_to_mock(&h);
    s.put("flaky", b"data").unwrap();
    *h.mock.fail_next_gets.lock().unwrap() = 2; // 500, 500, then 200
    let before = *h.mock.requests.lock().unwrap();
    assert_eq!(s.get("flaky").unwrap(), Some(b"data".to_vec()));
    let after = *h.mock.requests.lock().unwrap();
    assert_eq!(after - before, 3, "expected 2 retries then success");
    // A 404 is final: exactly one request, no retries.
    let before = *h.mock.requests.lock().unwrap();
    assert_eq!(s.get("missing-key").unwrap(), None);
    assert_eq!(*h.mock.requests.lock().unwrap() - before, 1, "4xx must not retry");
}

#[test]
fn s3_list_paginates() {
    let h = start_mock();
    let s = s3_to_mock(&h);
    // Force small pages through the mock's max-keys handling by listing
    // via many keys (mock honors max-keys; client pages internally).
    for i in 0..5 {
        s.put(&format!("p/{i:03}"), b"x").unwrap();
    }
    let keys = s.list("p/").unwrap();
    assert_eq!(keys.len(), 5);
    assert_eq!(keys[0], "p/000");
}

// ---------------------------------------------------------------------------
// Full registry over mock S3: publish / download / 409 / persistence
// ---------------------------------------------------------------------------

fn publish_archive(base: &str, name: &str, version: &str, body: &str) -> Vec<u8> {
    let dir = tmp(&format!("pkg-{name}-{version}"));
    std::fs::write(
        dir.join("klang.toml"),
        format!("name = \"{name}\"\nversion = \"{version}\"\nentry = \"main\"\n"),
    )
    .unwrap();
    std::fs::write(dir.join("lib.klang"), body).unwrap();
    let files = klang::registry::collect_package_files(&dir).expect("collects");
    let archive = klang::registry::pack_archive(&files).expect("packs");
    klang::registry::publish_pkg(base, TOKEN, name, version, &archive).expect("publishes");
    archive
}

fn serve_on_mock(data_tag: &str, h: &MockHandle) -> (String, S3Storage) {
    let s = s3_to_mock(h);
    let storage: Arc<dyn Storage> = Arc::new(s.clone());
    let base = klang::registry::spawn_ephemeral_with_storage(&tmp(data_tag), TOKEN, storage, "b2")
        .expect("registry starts on mock S3");
    (base, s)
}

#[test]
fn registry_on_mock_s3_publish_download_immutable() {
    let h = start_mock();
    let (base, s) = serve_on_mock("mock-reg", &h);

    let archive = publish_archive(&base, "calc", "1.0.0", "fn t(n: i32) -> i32 { return n }");
    let sha = klang::registry::sha256_hex(&archive);

    // Object layout: archive + checksum sidecar + per-package index.
    assert_eq!(
        s.get("packages/calc/1.0.0.klangpkg").unwrap(),
        Some(archive.clone())
    );
    assert_eq!(
        s.get("packages/calc/1.0.0.sha256").unwrap().map(|b| String::from_utf8(b).unwrap()),
        Some(sha.clone())
    );
    let doc = s.get("index/calc.json").unwrap().expect("index doc stored");
    let text = String::from_utf8(doc).unwrap();
    assert!(text.contains("\"1.0.0\"") && text.contains(&sha), "{text}");

    // Metadata + download round-trip with integrity.
    let meta = klang::registry::fetch_metadata(&base, "calc").expect("metadata");
    assert_eq!(meta.latest, "1.0.0");
    assert_eq!(meta.versions[0].1, sha);
    let bytes = klang::registry::download(&base, "calc", "1.0.0").expect("download");
    assert_eq!(bytes, archive);
    assert_eq!(klang::registry::sha256_hex(&bytes), sha);

    // Identical republish is idempotent; different bytes conflict.
    klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive).expect("idempotent");
    let dir = tmp("pkg-calc-diff");
    std::fs::write(
        dir.join("klang.toml"),
        "name = \"calc\"\nversion = \"1.0.0\"\nentry = \"main\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib.klang"), "fn t(n: i32) -> i32 { return n + 1 }").unwrap();
    let files = klang::registry::collect_package_files(&dir).expect("collects");
    let archive2 = klang::registry::pack_archive(&files).expect("packs");
    let err = klang::registry::publish_pkg(&base, TOKEN, "calc", "1.0.0", &archive2)
        .expect_err("conflicts");
    assert_eq!(err.kind, "conflict", "{err}");
    // Original bytes untouched.
    assert_eq!(s.get("packages/calc/1.0.0.klangpkg").unwrap(), Some(archive));

    all_auth_signed(&h);
}

#[test]
fn registry_on_mock_s3_restart_persists() {
    let h = start_mock();
    let (base, _) = serve_on_mock("mock-restart-1", &h);
    let archive = publish_archive(&base, "calc", "1.0.0", "fn t(n: i32) -> i32 { return n }");
    publish_archive(&base, "calc", "1.1.0", "fn t(n: i32) -> i32 { return n + 1 }");

    // "Restart": a brand-new server instance over the same bucket (fresh
    // in-memory index rebuilt from the object listing).
    let (base2, _) = serve_on_mock("mock-restart-2", &h);
    let meta = klang::registry::fetch_metadata(&base2, "calc").expect("metadata survives restart");
    assert_eq!(meta.versions.len(), 2, "{meta:?}");
    assert_eq!(meta.latest, "1.1.0");
    let bytes = klang::registry::download(&base2, "calc", "1.0.0").expect("download survives");
    assert_eq!(bytes, archive);
}

#[test]
fn registry_on_mock_s3_repairs_corrupt_index() {
    let h = start_mock();
    let (base, s) = serve_on_mock("mock-repair-1", &h);
    publish_archive(&base, "calc", "1.0.0", "fn t(n: i32) -> i32 { return n }");

    // Corrupt the per-package doc and drop the sidecar; a fresh instance
    // must rebuild both from the archives.
    s.put("index/calc.json", b"{corrupt").unwrap();
    h.mock.delete("packages/calc/1.0.0.sha256");

    let (base2, s2) = serve_on_mock("mock-repair-2", &h);
    let meta = klang::registry::fetch_metadata(&base2, "calc").expect("repaired metadata");
    assert_eq!(meta.versions.len(), 1);
    let doc = s2.get("index/calc.json").unwrap().expect("doc rewritten");
    assert!(String::from_utf8(doc).unwrap().contains("\"1.0.0\""));
    let side = s2
        .get("packages/calc/1.0.0.sha256")
        .unwrap()
        .expect("sidecar rewritten");
    assert_eq!(side.len(), 64);
}

#[test]
fn registry_on_mock_s3_health() {
    let h = start_mock();
    let (base, _) = serve_on_mock("mock-health", &h);
    // /health never touches storage: 200 even with an empty bucket.
    let res = ureq::get(&format!("{base}/health")).call().expect("health");
    assert_eq!(res.status().as_u16(), 200);
    // /health/storage probes the backend and names it without secrets.
    let mut res = ureq::get(&format!("{base}/health/storage")).call().expect("storage health");
    assert_eq!(res.status().as_u16(), 200);
    let body = res.body_mut().read_to_string().expect("body");
    assert!(body.contains("\"ok\":true"), "{body}");
    assert!(body.contains("\"b2\""), "{body}");
    assert!(!body.contains("TESTSECRET"), "secret leaked: {body}");
    assert!(!body.contains("test-bucket"), "bucket detail leaked: {body}");
}

// ---------------------------------------------------------------------------
// Backend selection / config (no network)
// ---------------------------------------------------------------------------

struct EnvGuard {
    saved: HashMap<String, Option<String>>,
}

impl EnvGuard {
    fn take(keys: &[&str]) -> Self {
        let mut saved = HashMap::new();
        for k in keys {
            saved.insert(k.to_string(), std::env::var(k).ok());
            std::env::remove_var(k);
        }
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}

const ENV_KEYS: &[&str] = &[
    "KLANG_STORAGE",
    "B2_KEY_ID",
    "B2_APP_KEY",
    "B2_BUCKET",
    "B2_ENDPOINT",
    "B2_REGION",
];

#[test]
fn config_unset_selects_local() {
    let _guard = env_lock();
    let _env = EnvGuard::take(ENV_KEYS);
    assert!(matches!(backend_from_env().unwrap(), StorageBackend::Local));
    std::env::set_var("KLANG_STORAGE", "disk");
    assert!(matches!(backend_from_env().unwrap(), StorageBackend::Local));
    std::env::set_var("KLANG_STORAGE", "");
    assert!(matches!(backend_from_env().unwrap(), StorageBackend::Local));
}

#[test]
fn config_missing_b2_vars_name_the_var_without_secrets() {
    let _guard = env_lock();
    let _env = EnvGuard::take(ENV_KEYS);
    std::env::set_var("KLANG_STORAGE", "b2");
    let secret = "SUPER-SECRET-APP-KEY-999";
    std::env::set_var("B2_APP_KEY", secret);
    std::env::set_var("B2_KEY_ID", "SOME-KEY-ID-123");
    // B2_BUCKET / B2_ENDPOINT missing: error names one of them...
    let err = backend_from_env().expect_err("missing bucket/endpoint");
    assert!(
        err.contains("B2_BUCKET") || err.contains("B2_ENDPOINT"),
        "vague error: {err}"
    );
    // ...and never echoes any supplied value.
    assert!(!err.contains(secret), "secret in error: {err}");
    assert!(!err.contains("SOME-KEY-ID-123"), "key id in error: {err}");

    for missing in ["B2_KEY_ID", "B2_APP_KEY", "B2_BUCKET", "B2_ENDPOINT"] {
        let _env2 = EnvGuard::take(ENV_KEYS);
        std::env::set_var("KLANG_STORAGE", "B2"); // case-insensitive
        std::env::set_var("B2_KEY_ID", "K");
        std::env::set_var("B2_APP_KEY", "A");
        std::env::set_var("B2_BUCKET", "buck");
        std::env::set_var("B2_ENDPOINT", "s3.us-west-004.backblazeb2.com");
        std::env::remove_var(missing);
        let err = backend_from_env().expect_err("must fail");
        assert!(err.contains(missing), "error `{err}` does not name `{missing}`");
    }
}

#[test]
fn config_b2_region_derives_from_endpoint() {
    let _guard = env_lock();
    let _env = EnvGuard::take(ENV_KEYS);
    assert_eq!(derive_region("s3.us-west-004.backblazeb2.com"), "us-west-004");
    assert_eq!(derive_region("https://s3.eu-central-003.backblazeb2.com/"), "eu-central-003");
    assert_eq!(derive_region("garbage"), "us-east-1");

    std::env::set_var("KLANG_STORAGE", "b2");
    std::env::set_var("B2_KEY_ID", "K");
    std::env::set_var("B2_APP_KEY", "A");
    std::env::set_var("B2_BUCKET", "buck");
    std::env::set_var("B2_ENDPOINT", "s3.us-west-004.backblazeb2.com");
    match backend_from_env().unwrap() {
        StorageBackend::B2(cfg) => assert_eq!(cfg.region, "us-west-004"),
        _ => panic!("expected b2"),
    }
    std::env::set_var("B2_REGION", "custom-1");
    match backend_from_env().unwrap() {
        StorageBackend::B2(cfg) => assert_eq!(cfg.region, "custom-1"),
        _ => panic!("expected b2"),
    }
    // Full circle: the config builds a usable client struct.
    let cfg = B2Config {
        key_id: "K".to_string(),
        app_key: "A".to_string(),
        bucket: "buck".to_string(),
        endpoint: "s3.us-west-004.backblazeb2.com".to_string(),
        region: String::new(),
    };
    assert!(S3Storage::from_b2(&cfg).is_ok());
}

// ---------------------------------------------------------------------------
// Opt-in live test (skipped unless KLANG_LIVE_TESTS=1 with real B2 vars)
// ---------------------------------------------------------------------------

#[test]
fn live_b2_roundtrip_opt_in() {
    if std::env::var("KLANG_LIVE_TESTS").as_deref() != Ok("1") {
        eprintln!("live_b2_roundtrip_opt_in: skipped (set KLANG_LIVE_TESTS=1 for a live run)");
        return;
    }
    let get = |n: &str| std::env::var(n).expect(&format!("live test needs {n}"));
    let cfg = B2Config {
        key_id: get("B2_KEY_ID"),
        app_key: get("B2_APP_KEY"),
        bucket: get("B2_BUCKET"),
        endpoint: get("B2_ENDPOINT"),
        region: std::env::var("B2_REGION").unwrap_or_default(),
    };
    if !cfg.region.is_empty() {
        // keep as-is
    }
    let s = S3Storage::from_b2(&cfg).expect("live client builds");
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let key = format!("test/live-{}-{}.txt", std::process::id(), nanos);
    let body = format!("klang live probe {nanos}").into_bytes();
    assert_eq!(s.put_if_absent(&key, &body).unwrap(), PutOutcome::Created);
    assert_eq!(s.get(&key).unwrap(), Some(body.clone()));
    assert!(s.exists(&key).unwrap());
    assert!(s.list("test/").unwrap().iter().any(|k| k == &key));
    s.delete_object(&key).expect("live cleanup");
    assert_eq!(s.get(&key).unwrap(), None, "cleanup must remove the probe");
}
