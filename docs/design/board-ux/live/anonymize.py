import hashlib
import json
import re
import sys
from typing import Any

LINK = re.compile(r"https://app\.t3\.codes/threads/[^\s\")]+")
THREAD = re.compile(
    r"\b(mcp:launch:)?[0-9a-f]{64}\b"
    r"|\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b"
)
SERVICE = {
    "version": "0.3.0",
    "t3": {"connected": True, "lastError": None},
    "tokenExpiresAt": "2026-12-17 12:38 MST",
    "watcher": {"lastPass": "17:26 MST", "alive": True},
    "openLinks": "desktop",
}


def fake(match: re.Match[str]) -> str:
    return "thread-" + hashlib.sha256(match.group(0).encode()).hexdigest()[:12]


def walk(node: Any, subs: list[tuple[str, str]]) -> Any:
    if isinstance(node, dict):
        return {k: walk(v, subs) for k, v in node.items()}
    if isinstance(node, list):
        return [walk(v, subs) for v in node]
    if not isinstance(node, str):
        return node
    for pattern, repl in subs:
        node = re.sub(pattern, repl, node)
    node = LINK.sub(lambda m: "https://t3.example/threads/" + fake(m), node)
    return THREAD.sub(fake, node)


def strings(node: Any) -> list[str]:
    if isinstance(node, dict):
        return [s for k, v in node.items() for s in [k, *strings(v)]]
    if isinstance(node, list):
        return [s for v in node for s in strings(v)]
    return [node] if isinstance(node, str) else []


def main() -> None:
    if len(sys.argv) != 4:
        sys.exit("usage: anonymize.py <raw-state.json> <private-subs.json> <out.json>")
    with open(sys.argv[1]) as f:
        state = json.load(f)
    with open(sys.argv[2]) as f:
        private = json.load(f)
    state.pop("service", None)
    out = walk(state, [tuple(p) for p in private["subs"]])
    out["service"] = SERVICE
    found = strings(out)
    left = sorted({w for w in private["forbid"] if any(w in s for s in found)})
    if left:
        sys.exit("unscrubbed: " + ", ".join(left))
    with open(sys.argv[3], "w") as f:
        json.dump(out, f, indent=1)


if __name__ == "__main__":
    main()
