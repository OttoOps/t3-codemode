#!/usr/bin/env python3
"""A stand-in for the glab CLI in a live eval sandbox: answers from fixtures, logs every call, reaches no network."""
import json
import os
import re
import sys

from fake_gh import emit, option, positional

HOST = "gitlab.acme.example"
PATH = "acme/widgets"
MR_URL = f"https://{HOST}/{PATH}/-/merge_requests/7"
VALUE_FLAGS = ("-R", "--repo", "--hostname", "-X", "--method", "-F", "--output", "-d", "--description",
               "-t", "--title", "-s", "--source-branch", "-b", "--target-branch", "-m", "--message",
               "--per-page", "-P", "--state", "-l", "--label")


def words(args):
    out, skip = [], False
    for a in args:
        if skip:
            skip = False
        elif a in VALUE_FLAGS:
            skip = True
        elif not a.startswith("-"):
            out.append(a)
    return out


def merge_request(fixtures):
    return {"iid": 7, "web_url": MR_URL, "state": "opened", "draft": False, "sha": "0" * 40,
            "source_branch": "feat/x", "target_branch": "main", "detailed_merge_status": "mergeable",
            "description": "", **fixtures.get("mr", {})}


def issue(fixtures, ref):
    return fixtures.get("issues", {}).get(ref.lstrip("#").rsplit("/", 1)[-1])


def api(path, fixtures):
    path = path.split("?", 1)[0]
    if path == "user":
        return {"username": "eval-user"}
    if m := re.fullmatch(r"projects/[^/]+/issues/(\d+)", path):
        return issue(fixtures, m.group(1))
    if re.fullmatch(r"projects/[^/]+/merge_requests/\d+", path):
        return merge_request(fixtures)
    if re.fullmatch(r"projects/[^/]+/merge_requests(/\d+/(pipelines|diffs|notes))?", path):
        return []
    if re.fullmatch(r"projects/[^/]+", path):
        return {"path_with_namespace": PATH, "web_url": f"https://{HOST}/{PATH}", "merge_trains_enabled": False}
    return {}


def answer(args, fixtures):
    w = words(args)
    head = " ".join(w[:2])
    as_json = option(args, "--output", "-F") == "json"
    if head == "auth status":
        sys.stderr.write(f"{HOST}\n  Logged in to {HOST} as eval-user\n")
    elif head == "issue view" and len(w) > 2:
        found = issue(fixtures, w[2])
        if found is None:
            sys.stderr.write(f"ERROR: 404 Not Found: issue {w[2]}\n")
            return 1
        print(json.dumps(found) if as_json else f"title:\t{found.get('title', '')}\n--\n{found.get('description', '')}")
    elif head == "repo view":
        emit({"path_with_namespace": PATH, "web_url": f"https://{HOST}/{PATH}"}, args)
    elif head == "mr create":
        print(MR_URL)
    elif head == "mr view":
        print(json.dumps(merge_request(fixtures)) if as_json else f"title:\t\nstate:\topen\nurl:\t{MR_URL}")
    elif head == "mr list":
        print("[]" if as_json else "No open merge requests match your search.")
    elif head in ("mr update", "mr note", "issue note"):
        print(MR_URL)
    elif w[:1] == ["api"] and len(w) > 1:
        found = api(w[1], fixtures)
        if found is None:
            sys.stderr.write("glab: 404 Not Found (HTTP 404)\n")
            return 1
        emit(found, args)
    return 0


def main():
    args = sys.argv[1:]
    base = os.environ["GH_FAKE_DIR"]
    with open(os.path.join(base, "glab.json")) as f:
        fixtures = json.load(f)
    body = option(args, "--description", "-d")
    with open(os.path.join(base, "glab.log"), "a") as f:
        f.write(json.dumps({"argv": args, "body": body}) + "\n")
    return answer(args, fixtures)


if __name__ == "__main__":
    sys.exit(main())
