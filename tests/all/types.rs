use crate::common;

use serde_json::json;
use t3_codemode::catalog::Catalog;
use t3_codemode::types::{sanitize_identifier, schema_to_ts};

#[test]
fn every_fixture_tool_gets_a_signature() {
    let tools = common::fixture_tools();
    assert_eq!(tools.len(), 74);
    let cat = Catalog::build(&tools);
    assert_eq!(cat.t3_names.len(), 74);
    for e in cat.entries.iter().filter(|e| e.kind == "t3") {
        assert!(e.signature.contains(&format!("t3.{}(input", e.name)), "{}", e.signature);
        assert!(e.signature.contains("): Promise<"), "{}", e.signature);
        assert!(!e.signature.contains("*/*"), "{}", e.signature);
    }
    let decls = cat.declarations();
    let raw = serde_json::to_string(&tools).unwrap().len();
    let per_tool: usize = cat.entries.iter().filter(|e| e.kind == "t3").map(|e| e.signature.len()).sum();
    println!(
        "TS declarations: {} chars (~{} tokens); t3 signatures {} chars; raw tool JSON {} chars (~{} tokens)",
        decls.len(),
        decls.len() / 4,
        per_tool,
        raw,
        raw / 4
    );
    assert!(decls.len() < raw);
    assert!(decls.starts_with("declare const t3: {\n"));
}

#[test]
fn the_send_signature_reads_naturally() {
    let tools = common::fixture_tools();
    let cat = Catalog::build(&tools);
    let send = cat.entries.iter().find(|e| e.name == "t3_thread_send").unwrap();
    assert!(send.signature.contains("threadId: string;"));
    assert!(send.signature.contains("mode?: \"auto\" | \"queue\" | \"steer\" | \"restart\";"));
    assert!(send.signature.contains("link?: string | null;"));
    assert!(send.signature.starts_with("/** Send a message to any thread."));
}

#[test]
fn schema_conversion_covers_the_common_keywords() {
    let s = json!({
        "type": "object",
        "$defs": {"Node": {"type": "object", "properties": {"next": {"$ref": "#/$defs/Node"}, "v": {"type": "integer"}}, "required": ["v"]}},
        "properties": {
            "id": {"type": "string", "description": "the id */ here"},
            "kind": {"enum": ["a", "b"]},
            "c": {"const": 3},
            "maybe": {"anyOf": [{"type": "string"}, {"type": "null"}]},
            "both": {"allOf": [{"type": "object", "properties": {"x": {"type": "number"}}}, {"type": "object", "properties": {"y": {"type": "number"}}}]},
            "list": {"type": "array", "items": {"type": ["string", "number"]}},
            "pair": {"type": "array", "items": [{"type": "string"}, {"type": "boolean"}]},
            "node": {"$ref": "#/$defs/Node"},
            "bag": {"type": "object", "additionalProperties": {"type": "boolean"}},
            "any": {},
            "nul": {"type": "string", "nullable": true},
            "weird-key": {"type": "string"}
        },
        "required": ["id", "kind"]
    });
    let ts = schema_to_ts(&s, 0);
    for want in [
        "/** the id *\\/ here */",
        "id: string;",
        "kind: \"a\" | \"b\";",
        "c?: 3;",
        "maybe?: string | null;",
        "x?: number;",
        "} & {",
        "list?: (string | number)[];",
        "pair?: [string, boolean];",
        "next?: unknown;",
        "v: number;",
        "bag?: Record<string, boolean>;",
        "any?: unknown;",
        "nul?: string | null;",
        "\"weird-key\"?: string;",
    ] {
        assert!(ts.contains(want), "missing {want:?} in:\n{ts}");
    }
}

#[test]
fn the_depth_guard_stops_runaway_nesting() {
    let mut s = json!({"type": "string"});
    for _ in 0..40 {
        s = json!({"type": "object", "properties": {"n": s}});
    }
    let ts = schema_to_ts(&s, 0);
    assert!(ts.contains("unknown"));
    assert!(ts.len() < 2000);
}

#[test]
fn identifiers_are_sanitized() {
    assert_eq!(sanitize_identifier("my-tool"), "my_tool");
    assert_eq!(sanitize_identifier("delete"), "delete_");
    assert_eq!(sanitize_identifier("1st"), "_1st");
    assert_eq!(sanitize_identifier("t3_thread_list"), "t3_thread_list");
}
