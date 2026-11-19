#!/usr/bin/env python3
"""A stdio MCP server that stands in for T3 Code's tools in a live eval sandbox.

It lists the T3 tools a workflow thread sees, answers each call from a fixture
file, and appends every call to a log, so an eval can assert what a run asked
T3 to do without a real T3 server.
"""
import json
import os
import sys

TOOLS = [
    "t3_thread_launch", "t3_thread_send", "t3_thread_read", "t3_thread_list",
    "t3_thread_organize", "t3_thread_configure", "t3_thread_wait", "t3_worktree_status",
    "t3_worktree_list", "t3_project_list", "orchestrator_capabilities", "delegate_task",
    "link_pull_request", "watch_pull_request", "unwatch_pull_request",
    "list_thread_pull_requests", "schedule_task", "list_scheduled_tasks",
    "delete_scheduled_task",
]
DESCRIPTIONS = {
    "t3_thread_launch": "Launch a new top-level T3 Code thread, optionally in a new worktree.",
    "t3_worktree_status": "Report this thread's worktree: attached or not, path, branch, project root.",
    "watch_pull_request": "Watch an open PR for this thread; T3 wakes you on checks, comments and conflicts.",
}


def fixtures():
    path = os.environ.get("T3_STUB_FIXTURE")
    if not path:
        return {}
    with open(path) as f:
        return json.load(f)


def log(name, args):
    path = os.environ.get("T3_STUB_LOG")
    if path:
        with open(path, "a") as f:
            f.write(json.dumps({"tool": name, "input": args}) + "\n")


def tool(name):
    return {"name": name, "description": DESCRIPTIONS.get(name, f"T3 Code tool {name}."),
            "inputSchema": {"type": "object", "additionalProperties": True}}


def handle(msg):
    method = msg.get("method")
    if method == "initialize":
        return {"protocolVersion": msg.get("params", {}).get("protocolVersion", "2025-06-18"),
                "capabilities": {"tools": {}}, "serverInfo": {"name": "t3-code", "version": "0.0.0"}}
    if method == "tools/list":
        return {"tools": [tool(n) for n in TOOLS]}
    if method == "tools/call":
        name = msg["params"]["name"]
        args = msg["params"].get("arguments") or {}
        log(name, args)
        reply = fixtures().get(name, {"ok": True})
        return {"content": [{"type": "text", "text": json.dumps(reply)}]}
    if method == "ping":
        return {}
    return None


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        if "id" not in msg:
            continue
        result = handle(msg)
        if result is None:
            out = {"jsonrpc": "2.0", "id": msg["id"],
                   "error": {"code": -32601, "message": f"no method {msg.get('method')}"}}
        else:
            out = {"jsonrpc": "2.0", "id": msg["id"], "result": result}
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
