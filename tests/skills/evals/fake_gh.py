#!/usr/bin/env python3
"""A stand-in for the gh CLI in a live eval sandbox: answers from fixtures, logs every call, reaches no network."""
import json
import os
import sys

REPO = {"nameWithOwner": "acme/widgets", "url": "https://github.com/acme/widgets",
        "defaultBranchRef": {"name": "main"}}
PR_URL = "https://github.com/acme/widgets/pull/101"


def option(args, *names):
    for i, a in enumerate(args):
        for n in names:
            if a == n and i + 1 < len(args):
                return args[i + 1]
            if a.startswith(n + "="):
                return a.split("=", 1)[1]
    return None


def positional(args):
    out, skip = [], False
    for a in args:
        if skip:
            skip = False
        elif a.startswith("-"):
            skip = "=" not in a and a not in ("--web", "--draft", "--fill")
        else:
            out.append(a)
    return out


def select(data, path):
    for part in [p for p in path.replace("[", ".[").split(".") if p]:
        if part.startswith("[") and part.endswith("]"):
            data = data[int(part[1:-1])]
        else:
            data = data.get(part) if isinstance(data, dict) else None
    return data


def emit(data, args):
    path = option(args, "-q", "--jq")
    if path and all(c.isalnum() or c in "._[]" for c in path):
        data = select(data, path)
    print(data if isinstance(data, str) else json.dumps(data))


def answer(args, fixtures):
    words = positional(args)
    head = " ".join(words[:2])
    if head == "issue view" and len(words) > 2:
        issue = fixtures.get("issues", {}).get(words[2].lstrip("#").rsplit("/", 1)[-1])
        if issue is None:
            sys.stderr.write(f"GraphQL: Could not resolve to an issue with the number of {words[2]}.\n")
            return 1
        emit(issue, args)
    elif head == "repo view":
        emit(REPO, args)
    elif head == "api user":
        emit({"login": "eval-user"}, args)
    elif head == "auth status":
        print("Logged in to github.com account eval-user")
    elif head == "pr create":
        print(PR_URL)
    elif head == "pr view":
        emit(fixtures.get("pr", {"number": 101, "url": PR_URL, "state": "OPEN", "statusCheckRollup": []}), args)
    elif head in ("pr list", "issue list", "run list"):
        emit([], args)
    elif head in ("issue comment", "issue edit", "pr edit", "pr comment"):
        print(fixtures.get("issues", {}).get("url", REPO["url"]))
    return 0


def main():
    args = sys.argv[1:]
    base = os.environ["GH_FAKE_DIR"]
    with open(os.path.join(base, "gh.json")) as f:
        fixtures = json.load(f)
    body_file = option(args, "--body-file", "-F")
    body = option(args, "--body", "-b")
    if body_file and os.path.isfile(body_file):
        with open(body_file) as f:
            body = f.read()
    elif body_file == "-":
        body = sys.stdin.read()
    with open(os.path.join(base, "gh.log"), "a") as f:
        f.write(json.dumps({"argv": args, "body": body}) + "\n")
    return answer(args, fixtures)


if __name__ == "__main__":
    sys.exit(main())
