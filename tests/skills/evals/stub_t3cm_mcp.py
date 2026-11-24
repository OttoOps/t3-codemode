#!/usr/bin/env python3
"""A stdio MCP server that stands in for t3cm's `execute` and `search` tools in a live eval sandbox.

It runs no code. Each `execute` call is answered by the first fixture rule whose `match` regular
expression finds the submitted code, as `{"result": ...}` or as an error carrying the rule's
`error`, and every call is appended to a log. A rule with `results` hands them out in turn, one per
call it answers.
"""
import json
import os
import re
import sys

DESCRIPTIONS = {
    "execute": "Run JavaScript against T3 Code and the thread-workflow layer `wf` (wf.pick, wf.pools, "
               "wf.send, wf.openChild, ...). Write an async arrow function; its return value comes back.",
    "search": "Search the t3cm API catalog (T3 tools, wf, mem) with an async arrow function over `catalog`.",
}


def rules():
    path = os.environ.get("T3CM_STUB_FIXTURE")
    if not path:
        return []
    with open(path) as f:
        return json.load(f)


def log(name, args):
    path = os.environ.get("T3CM_STUB_LOG")
    if path:
        with open(path, "a") as f:
            f.write(json.dumps({"tool": name, "input": args}) + "\n")


def earlier_calls(pattern):
    path = os.environ.get("T3CM_STUB_LOG")
    if not path or not os.path.exists(path):
        return 0
    with open(path) as f:
        calls = [json.loads(line) for line in f if line.strip()]
    return sum(1 for c in calls[:-1] if re.search(pattern, str(c["input"].get("code", "")), re.S))


def answer(code):
    for rule in rules():
        if re.search(rule["match"], code, re.S):
            if "error" in rule:
                return {"content": [{"type": "text", "text": rule["error"]}], "isError": True}
            result = rule.get("result")
            if "results" in rule:
                result = rule["results"][earlier_calls(rule["match"]) % len(rule["results"])]
            return {"content": [{"type": "text", "text": json.dumps({"result": result, "logs": []})}]}
    return {"content": [{"type": "text", "text": json.dumps({"result": None, "logs": []})}]}


def handle(msg):
    method = msg.get("method")
    if method == "initialize":
        return {"protocolVersion": msg.get("params", {}).get("protocolVersion", "2025-06-18"),
                "capabilities": {"tools": {}}, "serverInfo": {"name": "t3cm", "version": "0.0.0"}}
    if method == "tools/list":
        schema = {"type": "object", "properties": {"code": {"type": "string"}}, "required": ["code"]}
        return {"tools": [{"name": n, "description": d, "inputSchema": schema} for n, d in DESCRIPTIONS.items()]}
    if method == "tools/call":
        name = msg["params"]["name"]
        args = msg["params"].get("arguments") or {}
        log(name, args)
        if name == "search":
            return {"content": [{"type": "text", "text": json.dumps({"result": []})}]}
        return answer(str(args.get("code", "")))
    if method == "ping":
        return {}
    return None


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--cli":
        args = sys.argv[2:]
        if len(args) >= 1 and args[0] == "exec":
            src = sys.stdin.read() if len(args) < 2 or args[1] == "-" else open(args[1]).read()
            log("execute", {"code": src})
            res = answer(src)
            text = res["content"][0]["text"]
            if res.get("isError"):
                sys.stderr.write(text + "\n")
                sys.exit(1)
            sys.stdout.write(text + "\n")
            return
        if args[:2] == ["config", "get"]:
            with open(os.environ["T3CM_STUB_CONFIG"]) as f:
                value = json.load(f)
            for key in (args[2].split(".") if len(args) > 2 else []):
                value = value.get(key) if isinstance(value, dict) else None
            log("config", {"args": args})
            sys.stdout.write(json.dumps(value, indent=2) + "\n")
            return
        sys.stderr.write(f"stub t3cm: unsupported args {args}\n")
        sys.exit(1)
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
