//! Wave1 S2b — MCP `klang_check` follows imports.
//!
//! Calls the MCP tool entry point the way `mcp_gates.rs` does
//! (JSON-RPC `tools/call` via `handle_request`). Each test uses its own
//! unique temp dir (pid + atomic counter). Never runs program code.

use klang::mcp::{handle_request, parse_json, Json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static CTR: AtomicU64 = AtomicU64::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("klang-s2-mcp-{}-{}-{}", tag, std::process::id(), c));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &PathBuf, rel: &str, content: &str) -> String {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, content).unwrap();
    p.to_string_lossy().to_string()
}

fn req(id: &str, method: &str, params: Json) -> String {
    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        ("id".to_string(), Json::Str(id.to_string())),
        ("method".to_string(), Json::Str(method.to_string())),
        ("params".to_string(), params),
    ])
    .render()
}

fn call(id: &str, tool: &str, args: Json) -> Json {
    let params = Json::Obj(vec![
        ("name".to_string(), Json::Str(tool.to_string())),
        ("arguments".to_string(), args),
    ]);
    let resp = handle_request(&req(id, "tools/call", params)).expect("response");
    parse_json(&resp).expect("valid json response")
}

fn text_payload(resp: &Json) -> Json {
    let content = resp.get("result").expect("result").get("content").and_then(Json::as_arr).expect("content");
    let text = content[0].get("text").and_then(Json::as_str).expect("text");
    parse_json(text).expect("payload parses")
}

fn check_args(source: &str, file: Option<&str>) -> Json {
    let mut pairs = vec![("source".to_string(), Json::Str(source.to_string()))];
    if let Some(f) = file {
        pairs.push(("file".to_string(), Json::Str(f.to_string())));
    }
    Json::Obj(pairs)
}

fn diags_of(payload: &Json) -> Vec<Json> {
    payload.get("diagnostics").and_then(Json::as_arr).expect("diagnostics").clone()
}

fn diag_file(d: &Json) -> String {
    d.get("primary_span").and_then(|s| s.get("file")).and_then(Json::as_str).unwrap_or("").to_string()
}

#[test]
fn s2_mcp_single_file_clean_and_error() {
    // No `file`: single-buffer check, backward compatible.
    let clean = "fn main() -> i32 { return 42 }\n";
    let payload = text_payload(&call("c1", "klang_check", check_args(clean, None)));
    assert!(diags_of(&payload).is_empty(), "{payload:?}");

    let bad = "fn main() -> i32 { return nope_mcp + 1 }\n";
    let payload = text_payload(&call("c2", "klang_check", check_args(bad, None)));
    let ds = diags_of(&payload);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].get("code").and_then(Json::as_str), Some("E-UNDEFINED"));
}

#[test]
fn s2_mcp_local_import_clean() {
    let dir = tmpdir("local-clean");
    let lib_src = "fn libdouble(n: i32) -> i32 { return n * 2 }\n";
    write(&dir, "lib.klang", lib_src);
    let entry_src = "import \"lib.klang\"\nfn main() -> i32 { return libdouble(21) }\n";
    let entry_path = write(&dir, "main.klang", entry_src);
    // Must follow the local import: clean means clean (would be E-UNDEFINED
    // for libdouble without import following).
    let payload = text_payload(&call("c", "klang_check", check_args(entry_src, Some(&entry_path))));
    assert!(diags_of(&payload).is_empty(), "{payload:?}");
    let _ = dir;
}

#[test]
fn s2_mcp_error_inside_imported_file_names_that_file() {
    let dir = tmpdir("local-err");
    let lib_src = "fn libbadmcp() -> i32 { return \"hi\" - 1 }\n";
    let lib_path = write(&dir, "lib.klang", lib_src);
    let entry_src = "import \"lib.klang\"\nfn main() -> i32 { return libbadmcp() }\n";
    let entry_path = write(&dir, "main.klang", entry_src);
    let payload = text_payload(&call("c", "klang_check", check_args(entry_src, Some(&entry_path))));
    let ds = diags_of(&payload);
    assert!(!ds.is_empty(), "{payload:?}");
    assert!(ds.iter().any(|d| d.get("code").and_then(Json::as_str) == Some("E-TYPE")), "{payload:?}");
    let d = ds.iter().find(|d| d.get("code").and_then(Json::as_str) == Some("E-TYPE")).unwrap();
    assert_eq!(diag_file(d), lib_path, "{d:?}");
    // Real span, not 0,0.
    let start = match d.get("primary_span").and_then(|s| s.get("start")) { Some(Json::Int(n)) => *n, _ => 0 };
    let end = match d.get("primary_span").and_then(|s| s.get("end")) { Some(Json::Int(n)) => *n, _ => 0 };
    assert!((start, end) != (0, 0), "{d:?}");
    assert!(end > start, "{d:?}");
    let text = std::fs::read_to_string(&lib_path).unwrap();
    assert!(text[(start as usize)..(end as usize)].contains("-"), "{d:?}");
}

#[test]
fn s2_mcp_unresolved_import_has_real_span() {
    let dir = tmpdir("unresolved");
    let entry_src = "import \"nosuchmcp.klang\"\nfn main() -> i32 { return 0 }\n";
    let entry_path = write(&dir, "main.klang", entry_src);
    let payload = text_payload(&call("c", "klang_check", check_args(entry_src, Some(&entry_path))));
    let ds = diags_of(&payload);
    assert_eq!(ds.len(), 1, "{payload:?}");
    assert_eq!(ds[0].get("code").and_then(Json::as_str), Some("E-IO-NOT-FOUND"));
    assert_eq!(diag_file(&ds[0]), entry_path, "{:?}", ds[0]);
    let start = match ds[0].get("primary_span").and_then(|s| s.get("start")) { Some(Json::Int(n)) => *n, _ => 0 };
    let end = match ds[0].get("primary_span").and_then(|s| s.get("end")) { Some(Json::Int(n)) => *n, _ => 0 };
    assert!((start, end) != (0, 0), "{:?}", ds[0]);
    let text = std::fs::read_to_string(&entry_path).unwrap();
    assert!(text[(start as usize)..(end as usize)].contains("nosuchmcp.klang"), "{:?}", ds[0]);
}

#[test]
fn s2_mcp_installed_package_import() {
    // Hand-made vendor fixture (no network): klang.toml + lock + .klang_pkgs.
    let dir = tmpdir("pkg");
    let sha = "b".repeat(64);
    write(&dir, "klang.toml", "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nmcplib = \"registry:mcplib@1.0.0\"\n");
    write(&dir, "klang.lock", &format!("package mcplib 1.0.0 {sha}\n"));
    write(&dir, ".klang_pkgs/mcplib/1.0.0/lib.klang", "fn mcpdouble(n: i32) -> i32 { return n * 2 }\n");
    let entry_src = "import \"mcplib\"\nfn main() -> i32 { return mcpdouble(21) }\n";
    let entry_path = write(&dir, "main.klang", entry_src);
    let payload = text_payload(&call("c", "klang_check", check_args(entry_src, Some(&entry_path))));
    assert!(diags_of(&payload).is_empty(), "{payload:?}");

    // And an error inside the installed package names the vendor file.
    write(&dir, ".klang_pkgs/mcplib/1.0.0/lib.klang", "fn mcpdouble(n: i32) -> i32 { return \"hi\" - 1 }\n");
    let payload = text_payload(&call("c2", "klang_check", check_args(entry_src, Some(&entry_path))));
    let ds = diags_of(&payload);
    assert!(ds.iter().any(|d| d.get("code").and_then(Json::as_str) == Some("E-TYPE")), "{payload:?}");
    let d = ds.iter().find(|d| d.get("code").and_then(Json::as_str) == Some("E-TYPE")).unwrap();
    assert!(diag_file(d).contains(".klang_pkgs/mcplib/1.0.0/"), "{d:?}");
}
