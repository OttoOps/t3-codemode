#!/usr/bin/env python3
"""PreToolUse hook for a live eval sandbox: refuses, before it runs, any tool call that names the real
home's agent folders, a local server port, or a network client, so a run cannot reach them by path."""
import json
import re
import sys

NETWORK = r"\b(curl|wget|ncat|telnet|ssh|scp|rsync)\b|/usr/bin/gh\b"


def strings(value):
    if isinstance(value, dict):
        return [s for v in value.values() for s in strings(v)]
    if isinstance(value, list):
        return [s for v in value for s in strings(v)]
    return [str(value)]


def blocked(tool_input, real_home):
    homes = [re.escape(f"{real_home}/{d}") for d in (".claude", ".agents", ".codex", ".t3")]
    pattern = "|".join([*homes, r"(?:127\.0\.0\.1|localhost|0\.0\.0\.0):\d+", NETWORK])
    return re.search(pattern, "\n".join(strings(tool_input)))


def main():
    event = json.load(sys.stdin)
    hit = blocked(event.get("tool_input", {}), sys.argv[1])
    if hit:
        sys.stderr.write(f"blocked by the eval sandbox: {hit.group(0)!r} is outside the sandbox\n")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
