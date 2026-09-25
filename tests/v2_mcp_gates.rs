//! Phase 11 — v2 MCP gates: `klang_v2_check` matches the CLI shape.

use klang::mcp::{handle_request, parse_json, Json};

fn req(id: &str, method: &str, params: Json) -> String {
    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        ("id".to_string(), Json::Str(id.to_string())),
        ("method".to_string(), Json::Str(method.to_string())),
        ("params".to_string(), params),
    ])
    .render()
}

fn call(id: &str, tool: &str, source: &str) -> Json {
    let args = Json::Obj(vec![("source".to_string(), Json::Str(source.to_string()))]);
    let params = Json::Obj(vec![
        ("name".to_string(), Json::Str(tool.to_string())),
        ("arguments".to_string(), args),
    ]);
    let resp = handle_request(&req(id, "tools/call", params)).expect("response");
    parse_json(&resp).expect("json")
}

fn result_of(resp: &Json) -> &Json {
    resp.get("result").expect("result")
}

fn text_payload(resp: &Json) -> Json {
    let text = result_of(resp)
        .get("content")
        .and_then(Json::as_arr)
        .and_then(|c| c.first())
        .and_then(|b| b.get("text"))
        .and_then(Json::as_str)
        .expect("text payload");
    parse_json(text).expect("payload parses")
}

#[test]
fn v2_mcp_lists_v2_check() {
    let resp =
        parse_json(&handle_request(&req("1", "tools/list", Json::Obj(vec![]))).expect("resp"))
            .expect("json");
    let tools = result_of(&resp)
        .get("tools")
        .and_then(Json::as_arr)
        .expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name"))
        .filter_map(Json::as_str)
        .collect();
    assert!(
        names.contains(&"klang_v2_check"),
        "v2 check listed: {names:?}"
    );
    // v1 tools stay listed.
    for v1 in ["klang_check", "klang_run", "klang_fmt", "klang_scope_plan"] {
        assert!(names.contains(&v1), "v1 tool kept: {names:?}");
    }
}

#[test]
fn v2_mcp_check_matches_cli_shape() {
    let bad = "flow(score i32) { return 1 }";
    let resp = call("c", "klang_v2_check", bad);
    let payload = text_payload(&resp);
    let got = payload
        .get("diagnostics")
        .and_then(Json::as_arr)
        .expect("diagnostics");
    assert_eq!(got.len(), 1);
    // The CLI prints `Diagnostic::to_json()`; the tool embeds the re-parsed
    // same object — assert byte-level equivalence of the shape.
    let want = klang::mcp::v2_check_source(bad);
    assert_eq!(want.len(), 1);
    assert_eq!(want[0].code, "E-PARSE-FLOW");
    let want_json = parse_json(&want[0].to_json()).expect("parses");
    assert_eq!(&got[0], &want_json);
    // Clean input is an empty array on both surfaces.
    let good = "flow(score: i32, dep=threshold: i32) -> str { score >= threshold }";
    let resp = call("d", "klang_v2_check", good);
    let payload = text_payload(&resp);
    assert_eq!(
        payload
            .get("diagnostics")
            .and_then(Json::as_arr)
            .map(Vec::len),
        Some(0)
    );
}

#[test]
fn v2_mcp_consumes_v2_diagnostics() {
    // A v2 diagnostic round-trips through the MCP JSON layer, so harnesses
    // can feed v2 check output back without parsing human text.
    let diags = klang::mcp::v2_check_source("echo fn fetch(");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "E-PARSE-ECHO");
    let back = parse_json(&diags[0].to_json()).expect("parses");
    assert_eq!(
        back.get("code").and_then(Json::as_str),
        Some("E-PARSE-ECHO")
    );
}
