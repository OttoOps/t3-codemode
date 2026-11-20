import json
import sys

src, dst, view = sys.argv[1], sys.argv[2], sys.argv[3]
state = json.load(open(src))


def entry(seq, repo, number, title, thread, at, local, dismissed=None):
    return {
        "id": f"n{seq}",
        "url": f"https://github.com/{repo}/issues/{number}",
        "repo": repo,
        "number": number,
        "label": f"{repo.split('/')[1]}#{number}",
        "title": title,
        "note": None,
        "thread": thread,
        "threadLink": f"https://t3.example/threads/{thread}",
        "threadTitle": thread.replace("-", " "),
        "at": at,
        "atLocal": local,
        "dismissed": dismissed is not None,
        "dismissedAt": dismissed,
        "dismissedAtLocal": "17:20 MST" if dismissed else None,
        "restoredAt": None,
    }


new = [
    entry(4, "acme/widgets", 212, "Grader rejects a rubric with an empty weight", "widgets-grader-refactor", "2026-11-19T00:24:10Z", "17:24 MST"),
    entry(3, "acme/gadgets", 88, "Retry the export when the bucket answers 503", "gadgets-export-hardening", "2026-11-19T00:05:40Z", "17:05 MST"),
    entry(2, "acme/widgets", 209, "Docs link in the CLI help points at the old site", "widgets-cli-cleanup", "2026-11-18T22:41:00Z", "15:41 MST"),
]
gone = [entry(1, "acme/gadgets", 85, "Flaky timezone test on the nightly runner", "gadgets-ci-triage", "2026-11-18T20:12:00Z", "13:12 MST", "2026-11-19T00:20:00Z")]
if view == "empty":
    new, gone = [], []
state["newIssues"] = new
state["dismissedIssues"] = gone
state["counts"]["newIssues"] = len(new)
json.dump(state, open(dst, "w"))
