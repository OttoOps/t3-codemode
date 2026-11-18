use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value, json};

use crate::types::{doc_block, is_identifier, property_key, sanitize_identifier, signature};
use crate::upstream::{T3_NAMESPACE, ToolDef, qualified};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub kind: &'static str,
    pub server: String,
    pub description: String,
    pub signature: String,
    pub input_schema: Value,
}

pub struct Catalog {
    pub entries: Vec<Entry>,
    pub t3_names: BTreeSet<String>,
    pub upstream_names: BTreeMap<String, BTreeSet<String>>,
    namespaces: Vec<String>,
    pub json: String,
}

const WAKE_RESULT: &str = r#"{ handle: string; delivery: "noted" | "sent" | "queued" | "board"; detail?: string; card?: string; closed?: boolean }"#;
const CARD: &str = r#"{ id: string; kind: string; label: string | null; ticket: string | null; link: string | null; text: string; handle: string | null; options: string[]; at: string; atLocal: string; resolved?: string; resolution?: string | null }"#;

fn builtin(name: &str, kind: &'static str, description: &str, sig: &str, input: Value) -> Entry {
    Entry {
        name: name.into(),
        kind,
        server: kind.into(),
        description: description.into(),
        signature: format!("{}{sig}", doc_block(description, "")),
        input_schema: input,
    }
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

pub fn builtins() -> Vec<Entry> {
    let kinds = json!(["note", "event", "gate", "blocker", "report", "handoff", "closed"]);
    let link = obj(
        json!({"parent": {"type": "string"}, "label": {"type": "string"}, "role": {"type": "string"},
               "down": {"type": "integer"}, "miss": {"type": "integer"}, "stall": {"type": "integer"}}),
        &["parent", "label", "role"],
    );
    vec![
        builtin(
            "mem.get",
            "mem",
            "Read a JSON value from this session's namespace; null when absent.",
            "mem.get(key: string): Promise<unknown | null>;",
            obj(json!({"key": {"type": "string"}}), &["key"]),
        ),
        builtin(
            "mem.set",
            "mem",
            "Store a JSON value (up to 1 MB) under key. Survives restarts.",
            "mem.set(key: string, value: unknown): Promise<null>;",
            obj(json!({"key": {"type": "string"}, "value": {}}), &["key", "value"]),
        ),
        builtin(
            "mem.del",
            "mem",
            "Delete a key; true when it existed.",
            "mem.del(key: string): Promise<boolean>;",
            obj(json!({"key": {"type": "string"}}), &["key"]),
        ),
        builtin(
            "mem.list",
            "mem",
            "Keys in the namespace, sorted, optionally starting with prefix.",
            "mem.list(prefix?: string): Promise<string[]>;",
            obj(json!({"prefix": {"type": "string"}}), &[]),
        ),
        builtin(
            "mem.ns",
            "mem",
            "The same get/set/del/list API on another namespace, shared across threads (for example \"work:widgets#41\"). Synchronous.",
            "mem.ns(name: string): { get; set; del; list };",
            obj(json!({"name": {"type": "string"}}), &["name"]),
        ),
        builtin(
            "vars",
            "mem",
            "A plain object restored before each execute and saved after it, per session (default: your thread). Assign fields; keep it JSON-serializable.",
            "vars: Record<string, unknown>;",
            json!({}),
        ),
        builtin(
            "wf.init",
            "wf",
            "Start this thread with a role: overseer, ticket or worker. A role the overseer role lists as a child (ticket by default) with no open link joins the board (parent \"board\"), labeled from its T3 title or `label`, so its gates become board cards; root: true keeps it a standalone root. Other roles start as roots.",
            "wf.init(input: { role: string; label?: string; root?: boolean }): Promise<{ thread: string; role: string; root: boolean; parent?: string; label?: string; watcher: string }>;",
            obj(
                json!({"role": {"type": "string"}, "label": {"type": "string"}, "root": {"type": "boolean"}}),
                &["role"],
            ),
        ),
        builtin(
            "wf.openChild",
            "wf",
            "Prepare a link to a child thread you are about to launch. Paste the returned brief, verbatim, at the end of the child's launch brief; the child binds the link with wf.join.",
            "wf.openChild(input: { role: string; label: string }): Promise<{ brief: string; params: { down: number; miss: number; stall: number } }>;",
            obj(json!({"role": {"type": "string"}, "label": {"type": "string"}}), &["role", "label"]),
        ),
        builtin(
            "wf.join",
            "wf",
            "Join the link a parent opened (the call is in your launch brief). The watcher then nudges you after `down` idle minutes with no gate open.",
            "wf.join(input: { parent: string; label: string; role: string; down?: number; miss?: number; stall?: number }): Promise<{ parent: string; label: string; params: { down: number; miss: number; stall: number } }>;",
            link,
        ),
        builtin(
            "wf.send",
            "wf",
            "Send to your parent. note: stored, never wakes, reaches the parent as a digest line at its next wake. event/gate/blocker/report wake it per link policy (overseer links turn events into notes). gate also opens a gate (no nudges until cleared). handoff/closed close the link. wake overrides the policy. With parent \"board\": note/event go to the board's feed, gate/blocker/report become a decision card (delivery \"board\", card id), handoff/closed mark the ticket done.",
            &format!(
                "wf.send(input: {{ kind: \"note\" | \"event\" | \"gate\" | \"blocker\" | \"report\" | \"handoff\" | \"closed\"; body: string; gate?: string; wake?: boolean }}): Promise<{WAKE_RESULT}>;"
            ),
            obj(
                json!({"kind": {"enum": kinds}, "body": {"type": "string"}, "gate": {"type": "string"}, "wake": {"type": "boolean"}}),
                &["kind", "body"],
            ),
        ),
        builtin(
            "wf.reply",
            "wf",
            "Send to one of your children (by label or thread id). A closed link needs reopen: true.",
            &format!("wf.reply(input: {{ to: string; body: string; reopen?: boolean }}): Promise<{WAKE_RESULT}>;"),
            obj(
                json!({"to": {"type": "string"}, "body": {"type": "string"}, "reopen": {"type": "boolean"}}),
                &["to", "body"],
            ),
        ),
        builtin(
            "wf.gate",
            "wf",
            "Open a gate while you wait on a person ({open: \"merge\"}); the watcher does not nudge you until {clear: true}.",
            "wf.gate(input: { open: string } | { clear: true }): Promise<{ gate: string | null }>;",
            obj(json!({"open": {"type": "string"}, "clear": {"type": "boolean"}}), &[]),
        ),
        builtin(
            "wf.close",
            "wf",
            "Close a child link without a handoff (by label or thread id); the watcher stops watching it.",
            "wf.close(input: { child: string }): Promise<{ closed: string }>;",
            obj(json!({"child": {"type": "string"}}), &["child"]),
        ),
        builtin(
            "wf.inbox",
            "wf",
            "Unread notes from your children, oldest first; marks them read.",
            "wf.inbox(): Promise<{ handle: string; at: string; label: string; kind: string; head: string }[]>;",
            obj(json!({}), &[]),
        ),
        builtin(
            "wf.show",
            "wf",
            "One stored message in full, by handle (\"m12\").",
            "wf.show(handle: string): Promise<{ handle: string; kind: string; from: string; to: string; at: string; body: string }>;",
            obj(json!({"handle": {"type": "string"}}), &["handle"]),
        ),
        builtin(
            "wf.board",
            "wf",
            "Your parent link and one object per child link, with live T3 status. all: include closed links. parent: show that node's tree instead, each child with its workers (parent: \"board\" is the whole t3-codemode board, no caller needed).",
            "wf.board(input?: { all?: boolean; parent?: string }): Promise<{ parent: { thread: string; label: string; closed: boolean; gate: string | null } | null; children: { label: string; role: string; thread: string; link: string | null; status: string; asks: number; background: number; gate: string | null; lastUp: string; nudges: number; unread: number; last: { handle: string; kind: string; head: string } | null }[] }>;",
            obj(json!({"all": {"type": "boolean"}, "parent": {"type": "string"}}), &[]),
        ),
        builtin(
            "wf.adopt",
            "wf",
            "Take over a retiring thread's open child links, pending links and queued messages.",
            "wf.adopt(input: { from: string }): Promise<{ adopted: string[]; pending: string[] }>;",
            obj(json!({"from": {"type": "string"}}), &["from"]),
        ),
        builtin(
            "wf.sweep",
            "wf",
            "Before finishing: open child links, never-joined links (dropped now), and whether your own parent link is still open.",
            "wf.sweep(): Promise<{ open: string[]; dropped: string[]; parentOpen: boolean }>;",
            obj(json!({}), &[]),
        ),
        builtin(
            "wf.launchTicket",
            "wf",
            "Launch a t3-ticket thread for a GitHub issue that reports to the t3-codemode board (the root overseer). The T3 project is the one whose workspace origin remote is the issue's repo, unless project (id or title) is given. An unmapped repo or failed launch throws and leaves an error card on the board.",
            "wf.launchTicket(input: { issue: string; project?: string; title?: string; model?: string; effort?: string }): Promise<{ ok: true; label: string; thread: string; link: string | null; projectId: string; title: string }>;",
            obj(
                json!({"issue": {"type": "string", "description": "https://github.com/<owner>/<repo>/issues/<n> or <owner>/<repo>#<n>"},
                       "project": {"type": "string"}, "title": {"type": "string"}, "model": {"type": "string"}, "effort": {"type": "string"}}),
                &["issue"],
            ),
        ),
        builtin(
            "wf.cards",
            "wf",
            "The board's open decision cards, oldest first: gates, blockers, reports and escalations from ticket threads, with the reply options parsed from their text. all: include resolved cards, each with resolved (local time) and resolution.",
            &format!("wf.cards(input?: {{ all?: boolean }}): Promise<{CARD}[]>;"),
            obj(json!({"all": {"type": "boolean"}}), &[]),
        ),
        builtin(
            "wf.answer",
            "wf",
            "Answer a board card for the user: sends the ticket a [t3cm reply] 'The user says \"<text>\" about <id>: <card text>' and resolves the card. Same as a button on the board page.",
            &format!("wf.answer(id: string, text: string): Promise<{WAKE_RESULT}>;"),
            obj(json!({"id": {"type": "string"}, "text": {"type": "string"}}), &["id", "text"]),
        ),
        builtin(
            "wf.feed",
            "wf",
            "The board's activity feed: the latest notes and events from ticket and worker threads, newest first.",
            "wf.feed(input?: { limit?: number }): Promise<{ handle: string; at: string; iso: string; label: string; kind: string; head: string }[]>;",
            obj(json!({"limit": {"type": "integer"}}), &[]),
        ),
        builtin(
            "wf.waiting.add",
            "wf",
            "Add an item to your waiting list (decisions the user owes). Ids cycle a1..a9, b1..d9, skipping open ones.",
            "wf.waiting.add(input: { text: string; from?: string; link?: string }): Promise<{ id: string }>;",
            obj(json!({"text": {"type": "string"}, "from": {"type": "string"}, "link": {"type": "string"}}), &["text"]),
        ),
        builtin(
            "wf.waiting.resolve",
            "wf",
            "Resolve an open waiting-list item.",
            "wf.waiting.resolve(id: string, note?: string): Promise<{ id: string; text: string; resolved: true }>;",
            obj(json!({"id": {"type": "string"}, "note": {"type": "string"}}), &["id"]),
        ),
        builtin(
            "wf.waiting.list",
            "wf",
            "Your open waiting-list items, oldest first.",
            "wf.waiting.list(): Promise<{ id: string; text: string; from: string | null; link: string | null; at: string }[]>;",
            obj(json!({}), &[]),
        ),
    ]
}

const CALL_SIGNATURE: &str = "call(name: string, input?: object): Promise<unknown>;";

fn display_name(ns: &str, tool: &str) -> String {
    if is_identifier(tool) { format!("{ns}.{tool}") } else { format!("{ns}[{}]", property_key(tool)) }
}

fn push_member(s: &mut String, signature: &str, display: &str, member: &str) {
    for line in signature.lines() {
        s.push_str("  ");
        match line.strip_prefix(display) {
            Some(rest) => {
                s.push_str(member);
                s.push_str(rest);
            }
            None => s.push_str(line),
        }
        s.push('\n');
    }
}

pub fn aliases(tools: &[&str]) -> Vec<(String, Option<String>)> {
    let exact: BTreeSet<&str> = tools.iter().copied().collect();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let sanitized: Vec<String> = tools.iter().map(|t| sanitize_identifier(t)).collect();
    for (t, a) in tools.iter().zip(&sanitized) {
        if a != t {
            *counts.entry(a.clone()).or_default() += 1;
        }
    }
    tools
        .iter()
        .zip(sanitized)
        .map(|(t, a)| {
            let free = a != *t && counts[&a] == 1 && !exact.contains(a.as_str()) && a != "call";
            (t.to_string(), free.then_some(a))
        })
        .collect()
}

impl Catalog {
    pub fn build(t3_tools: &[Value]) -> Catalog {
        Catalog::build_with(t3_tools, &[])
    }

    pub fn build_with(t3_tools: &[Value], upstreams: &[(String, Vec<ToolDef>)]) -> Catalog {
        let upstream_count: usize = upstreams.iter().map(|(_, t)| t.len()).sum();
        let mut entries = Vec::with_capacity(t3_tools.len() + upstream_count + 24);
        let mut t3_names = BTreeSet::new();
        for tool in t3_tools {
            let Some(name) = tool["name"].as_str() else { continue };
            let description = tool["description"].as_str().unwrap_or_default().to_string();
            let input = tool.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"}));
            let display = format!("t3.{}", sanitize_identifier(name));
            let sig = signature(&display, &description, &input, tool.get("outputSchema"));
            t3_names.insert(name.to_string());
            entries.push(Entry {
                name: name.into(),
                kind: "t3",
                server: T3_NAMESPACE.into(),
                description,
                signature: sig,
                input_schema: input,
            });
        }
        let mut upstream_names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut namespaces = Vec::new();
        for (ns, tools) in upstreams {
            if ns == T3_NAMESPACE || upstream_names.contains_key(ns) {
                continue;
            }
            namespaces.push(ns.clone());
            let names = upstream_names.entry(ns.clone()).or_default();
            for tool in tools {
                if !names.insert(tool.name.clone()) {
                    continue;
                }
                let display = display_name(ns, &tool.name);
                let mut sig = signature(&display, &tool.description, &tool.input_schema, tool.output_schema.as_ref());
                if tool.name == "call" {
                    sig.insert_str(doc_block(&tool.description, "").len() + display.len() + 1, "\"call\", ");
                }
                entries.push(Entry {
                    name: qualified(ns, &tool.name),
                    kind: "mcp",
                    server: ns.clone(),
                    description: tool.description.clone(),
                    signature: sig,
                    input_schema: tool.input_schema.clone(),
                });
            }
        }
        entries.extend(builtins());
        let json = serde_json::to_string(&entries).expect("catalog serializes");
        Catalog { entries, t3_names, upstream_names, namespaces, json }
    }

    pub fn tools_of(&self, ns: &str) -> Vec<&str> {
        let prefix_len = ns.len() + 1;
        self.entries.iter().filter(|e| e.kind == "mcp" && e.server == ns).map(|e| &e.name[prefix_len..]).collect()
    }

    pub fn declarations(&self) -> String {
        let mut s = String::from("declare const t3: {\n");
        for e in self.entries.iter().filter(|e| e.kind == "t3") {
            for line in e.signature.lines() {
                s.push_str("  ");
                s.push_str(line.strip_prefix("t3.").unwrap_or(line));
                s.push('\n');
            }
        }
        s.push_str(&format!("  {CALL_SIGNATURE}\n}};\n"));
        for ns in &self.namespaces {
            s.push_str(&format!("declare const {ns}: {{\n"));
            for e in self.entries.iter().filter(|e| e.kind == "mcp" && e.server == *ns) {
                let tool = &e.name[ns.len() + 1..];
                if tool == "call" {
                    continue;
                }
                push_member(&mut s, &e.signature, &display_name(ns, tool), &property_key(tool));
            }
            s.push_str(&format!("  {CALL_SIGNATURE}\n}};\n"));
        }
        s.push_str(&format!("declare const tools: {{\n  {CALL_SIGNATURE}\n}};\n"));
        for e in self.entries.iter().filter(|e| e.kind != "t3" && e.kind != "mcp") {
            s.push_str(&e.signature);
            s.push('\n');
        }
        s
    }
}
