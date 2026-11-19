import json
import re
import sys
from datetime import datetime, timedelta, timezone

NOW = datetime(2026, 11, 19, 0, 26, 30, tzinfo=timezone.utc)
ZONE = timedelta(hours=-7)
TITLES = {
    "t3-codemode#2": "Release builds for Linux, macOS and Windows",
    "t3-codemode#17": "Publish T3 workflow skills with a setup skill",
    "t3-codemode#55": "Add filters, live search and saved contexts to board",
    "t3-codemode#56": "Merge via t3cm and reconcile with GitHub",
    "t3-codemode#59": "Board drops reply buttons for natural gate lines",
    "t3-codemode#67": "Make the T3 board readable at a glance",
    "web#646": "Launch page banner field",
    "web#647": "Build a search index from page sections",
    "web#648": "Check pages at build time",
}


def iso(clock: str | None) -> str | None:
    m = re.match(r"(\d\d):(\d\d) MST$", clock or "")
    if not m:
        return None
    day = (NOW + ZONE).replace(hour=int(m[1]), minute=int(m[2]), second=0) - ZONE
    if day > NOW + timedelta(minutes=1):
        day -= timedelta(days=1)
    return day.strftime("%Y-%m-%dT%H:%M:%SZ")


def node(n: dict) -> None:
    if n.get("label") in TITLES:
        n["title"] = f"{n['label']}: {TITLES[n['label']]}"
    n["upAt"] = iso(n.get("lastUp"))
    if n.get("last"):
        n["last"]["iso"] = iso(n["last"].get("at"))
    for w in n.get("workers", []):
        node(w)


path = sys.argv[1]
s = json.load(open(path))
for t in s["tree"]:
    node(t)
for t in s["settled"]["tickets"]:
    node(t)
    t["settledIso"] = iso(t.get("settledAt"))
labels = {t["label"]: t for t in s["tree"] + s["settled"]["tickets"]}
for f in s["feed"]:
    m = re.match(r"#(\d+)\b", f["label"])
    owner = next((t for l, t in labels.items() if m and l.endswith("#" + m[1])), labels.get(f["label"]))
    f["ticketLabel"] = owner["label"] if owner else None
    f["ticketTitle"] = owner.get("title") if owner else None
s["service"]["watcher"]["lastPassAt"] = iso(s["service"]["watcher"].get("lastPass"))
open(path, "w").write(json.dumps(s, indent=1, ensure_ascii=False) + "\n")
