//! MCP fake-harness gates (Phase 4, Part A).
//!
//! A scripted "fake harness" drives the MCP surface the way a real
//! harness would — `klang_check` for diagnostics, `klang_scope_plan`
//! for the edit scope, `klang_run` / `klang_fmt` for execution and
//! formatting — entirely through JSON-RPC request strings, proving the
//! verification mechanism survived the connector removal intact:
//! identical diagnostic shape, identical scope verdicts to the
//! `repair_gates.rs` suite, end-to-end oracle loop.

use klang::mcp::{Json, handle_request, parse_json};

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

fn result_of(resp: &Json) -> &Json {
    resp.get("result").expect("result field")
}

/// The tool's payload: `result.content[0].text` parsed as JSON.
fn text_payload(resp: &Json) -> Json {
    let content = result_of(resp).get("content").and_then(Json::as_arr).expect("content array");
    let text = content[0].get("text").and_then(Json::as_str).expect("text block");
    parse_json(text).expect("payload parses")
}


fn as_bool(v: &Json) -> Option<bool> {
    match v {
        Json::Bool(b) => Some(*b),
        _ => None,
    }
}

fn as_int(v: &Json) -> Option<i64> {
    match v {
        Json::Int(n) => Some(*n),
        _ => None,
    }
}

fn src_arg(source: &str) -> Json {
    Json::Obj(vec![("source".to_string(), Json::Str(source.to_string()))])
}

fn check_diags(source: &str) -> Vec<klang::diagnostics::Diagnostic> {
    let mut p = klang::parser::Parser::new(source);
    match p.parse_program() {
        Err(d) => vec![d],
        Ok(prog) => match klang::hir::TypedHIR::check(prog) {
            Ok(_) => vec![],
            Err(ds) => ds,
        },
    }
}

#[test]
fn handshake_and_tool_list() {
    let params = Json::Obj(vec![
        ("protocolVersion".to_string(), Json::Str("2024-11-05".to_string())),
        ("capabilities".to_string(), Json::Obj(vec![])),
        (
            "clientInfo".to_string(),
            Json::Obj(vec![("name".to_string(), Json::Str("fake-harness".to_string()))]),
        ),
    ]);
    let resp = parse_json(&handle_request(&req("1", "initialize", params)).expect("resp")).expect("json");
    let result = result_of(&resp);
    assert_eq!(
        result.get("protocolVersion").and_then(Json::as_str),
        Some("2024-11-05")
    );
    assert_eq!(
        result.get("serverInfo").and_then(|s| s.get("name")).and_then(Json::as_str),
        Some("klang")
    );
    // notifications/initialized gets silence.
    assert!(handle_request("{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}").is_none());

    let resp = parse_json(&handle_request(&req("2", "tools/list", Json::Obj(vec![]))).expect("resp"))
        .expect("json");
    let tools = result_of(&resp).get("tools").and_then(Json::as_arr).expect("tools");
    let names: Vec<&str> = tools.iter().filter_map(|t| t.get("name")).filter_map(Json::as_str).collect();
    assert_eq!(names, vec!["klang_check", "klang_run", "klang_fmt", "klang_scope_plan"]);
    for t in tools {
        assert!(t.get("description").and_then(Json::as_str).is_some(), "{t:?}");
        assert!(t.get("inputSchema").is_some(), "{t:?}");
    }
}

#[test]
fn check_carries_identical_diagnostic_shape() {
    // The DoD-critical property: MCP diagnostics are the exact same
    // shape as Diagnostic::to_json(), so harness prompt work stays valid.
    for src in [
        "fn main() -> i32 { return nope }",
        "fn main() -> i32 { return 42 }",
        "enum Opt { Some(x: i32), None }\nfn pick(v: Opt) -> i32 { return match v { Opt::Some(n) => n } }",
    ] {
        let resp = call("c", "klang_check", src_arg(src));
        let payload = text_payload(&resp);
        let got = payload.get("diagnostics").and_then(Json::as_arr).expect("diagnostics");
        let want: Vec<Json> = check_diags(src)
            .iter()
            .map(|d| parse_json(&d.to_json()).expect("to_json parses"))
            .collect();
        assert_eq!(got, &want, "shape must match to_json() for {src:?}");
    }
}

#[test]
fn scope_plan_matches_planner_verdicts() {
    // Same verdicts as repair_gates, reached through the MCP round-trip:
    // klang_check output fed verbatim into klang_scope_plan.
    let cases: Vec<(&str, &str, Vec<&str>)> = vec![
        // (source, expected scope, expected functions)
        (
            "enum Opt { Some(x: i32), None }\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n }\n}\n",
            "function",
            vec!["pick"],
        ),
        (
            "fn add(a: i32, b: i32) -> i32 { return a + b }\nfn main() -> i32 { return add(1) }\n",
            "function",
            vec!["main"],
        ),
    ];
    for (src, want_scope, want_fns) in cases {
        let check_resp = call("c", "klang_check", src_arg(src));
        let check_payload = text_payload(&check_resp);
        let diags = check_payload.get("diagnostics").expect("diagnostics").clone();
        assert!(!diags.as_arr().expect("array").is_empty(), "must fail check: {src:?}");
        let args = Json::Obj(vec![
            ("source".to_string(), Json::Str(src.to_string())),
            ("diagnostics".to_string(), diags),
        ]);
        let resp = call("s", "klang_scope_plan", args);
        let payload = text_payload(&resp);
        assert_eq!(
            payload.get("scope").and_then(Json::as_str),
            Some(want_scope),
            "{src:?}"
        );
        let fns: Vec<&str> = payload
            .get("functions")
            .and_then(Json::as_arr)
            .expect("functions")
            .iter()
            .filter_map(Json::as_str)
            .collect();
        assert_eq!(fns, want_fns, "{src:?}");
        // And it equals the direct planner call on the same input.
        let direct = klang::repair::plan_scope(&check_diags(src), src, false);
        assert_eq!(
            direct,
            klang::repair::scope::RepairScope::Functions(
                want_fns.iter().map(|s| s.to_string()).collect()
            )
        );
    }
}

#[test]
fn scope_plan_declaration_level_goes_file_with_reason() {
    // Declaration-level diagnostics always repair at file scope (F5),
    // with the reason naming the rule.
    let src = "mod m { fn secret() -> i32 { return 1 } }\nfn main() -> i32 { return m::secret() }\n";
    let check_resp = call("c", "klang_check", src_arg(src));
    let diags = text_payload(&check_resp).get("diagnostics").expect("diags").clone();
    let args = Json::Obj(vec![
        ("source".to_string(), Json::Str(src.to_string())),
        ("diagnostics".to_string(), diags),
    ]);
    let payload = text_payload(&call("s", "klang_scope_plan", args));
    assert_eq!(payload.get("scope").and_then(Json::as_str), Some("file"));
    let reason = payload.get("reason").and_then(Json::as_str).expect("reason");
    assert!(reason.contains("modules/visibility"), "{reason}");
}

#[test]
fn run_tool_executes_and_reports() {
    let resp = call(
        "r",
        "klang_run",
        Json::Obj(vec![(
            "source".to_string(),
            Json::Str("fn main() -> i32 { print(40 + 2) return 42 }".to_string()),
        )]),
    );
    assert_eq!(result_of(&resp).get("isError").and_then(as_bool), Some(false));
    let payload = text_payload(&resp);
    assert_eq!(payload.get("ok").and_then(as_bool), Some(true));
    let out: Vec<&str> = payload
        .get("stdout")
        .and_then(Json::as_arr)
        .expect("stdout")
        .iter()
        .filter_map(Json::as_str)
        .collect();
    assert_eq!(out, vec!["42"]);
    assert_eq!(payload.get("return_value").and_then(as_int), Some(42));
}

#[test]
fn run_tool_checks_first_and_surfaces_runtime_errors() {
    // Check failure: no execution, diagnostics back, isError set.
    let resp = call("r", "klang_run", src_arg("fn main() -> i32 { return nope }"));
    let payload = text_payload(&resp);
    assert_eq!(payload.get("ok").and_then(as_bool), Some(false));
    assert_eq!(payload.get("stage").and_then(Json::as_str), Some("check"));
    assert!(!payload.get("diagnostics").and_then(Json::as_arr).expect("diags").is_empty());
    // Runtime failure: stage + verbatim diagnostic object.
    let resp = call("r", "klang_run", src_arg("fn main() -> i32 { return 1 / 0 }"));
    let payload = text_payload(&resp);
    assert_eq!(payload.get("stage").and_then(Json::as_str), Some("run"));
    assert_eq!(
        payload.get("error").and_then(|e| e.get("code")).and_then(Json::as_str),
        Some("E-RUNTIME")
    );
}

#[test]
fn fmt_tool_round_trips_and_reports_parse_errors() {
    let resp = call("f", "klang_fmt", src_arg("fn main() -> i32 { return 1+2 }"));
    let payload = text_payload(&resp);
    assert_eq!(payload.get("ok").and_then(as_bool), Some(true));
    let formatted = payload.get("formatted").and_then(Json::as_str).expect("formatted");
    assert!(formatted.contains("return (1 + 2)"), "{formatted}");
    // Re-checks clean (formatter output is valid input).
    assert!(check_diags(formatted).is_empty());

    let resp = call("f", "klang_fmt", src_arg("fn main() -> i32 { return "));
    let payload = text_payload(&resp);
    assert_eq!(payload.get("ok").and_then(as_bool), Some(false));
    assert_eq!(
        payload.get("diagnostic").and_then(|e| e.get("code")).and_then(Json::as_str),
        Some("E-PARSE")
    );
}

#[test]
fn protocol_errors_are_json_rpc_shaped() {
    let err = |resp: Json| resp.get("error").expect("error").clone();
    // Unknown tool.
    let resp = call("e", "nope", Json::Obj(vec![]));
    assert_eq!(err(resp).get("code").and_then(as_int), Some(-32602));
    // Missing required argument.
    let resp = call("e", "klang_check", Json::Obj(vec![]));
    assert_eq!(err(resp).get("code").and_then(as_int), Some(-32602));
    // Malformed diagnostics array for scope planning.
    let bad = Json::Obj(vec![
        ("source".to_string(), Json::Str("fn main() -> i32 { return 1 }".to_string())),
        ("diagnostics".to_string(), Json::Arr(vec![Json::Obj(vec![]) ])),
    ]);
    let resp = call("e", "klang_scope_plan", bad);
    assert!(err(resp).get("message").and_then(Json::as_str).expect("msg").contains("diagnostics[0]"));
    // Unknown method.
    let resp = parse_json(&handle_request(&req("e", "bogus/method", Json::Obj(vec![]))).expect("resp")).expect("json");
    assert_eq!(err(resp).get("code").and_then(as_int), Some(-32601));
    // Garbage line.
    let resp = parse_json(&handle_request("definitely not json").expect("resp")).expect("json");
    assert_eq!(err(resp).get("code").and_then(as_int), Some(-32700));
}

#[test]
fn fake_harness_oracle_loop_converges_through_mcp() {
    // The PRD's integration proof, scripted: broken -> check -> scope ->
    // harness applies the known fix -> check clean -> run correct.
    // Mirrors benchmark S1-arity through the MCP surface.
    let broken = "fn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\n\nfn main() -> i32 {\n    return add(1)\n}\n";
    let fixed = "fn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\n\nfn main() -> i32 {\n    return add(1, 2)\n}\n";

    // 1. check: exactly one E-ARITY diagnostic.
    let diags_payload = text_payload(&call("h1", "klang_check", src_arg(broken)));
    let diags = diags_payload.get("diagnostics").and_then(Json::as_arr).expect("diags");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].get("code").and_then(Json::as_str), Some("E-ARITY"));

    // 2. scope: function(main), fed with the check output verbatim.
    let scope_payload = text_payload(&call(
        "h2",
        "klang_scope_plan",
        Json::Obj(vec![
            ("source".to_string(), Json::Str(broken.to_string())),
            ("diagnostics".to_string(), Json::Arr(diags.clone())),
        ]),
    ));
    assert_eq!(scope_payload.get("scope").and_then(Json::as_str), Some("function"));
    let fns: Vec<&str> = scope_payload
        .get("functions")
        .and_then(Json::as_arr)
        .expect("functions")
        .iter()
        .filter_map(Json::as_str)
        .collect();
    assert_eq!(fns, vec!["main"]);

    // 3. harness "edits" (oracle fix), re-check is clean, run gives 3.
    let clean = text_payload(&call("h3", "klang_check", src_arg(fixed)));
    assert!(clean.get("diagnostics").and_then(Json::as_arr).expect("diags").is_empty());
    let run = text_payload(&call("h4", "klang_run", src_arg(fixed)));
    assert_eq!(run.get("return_value").and_then(as_int), Some(3));
}
