"""Score a Dynamic layout judging round (rubric L8 to L13).

Usage: python3 layout-score.py <key.json> <judge>=<scores.json> [...]

Prints a Markdown table: the candidate's mean per criterion with each
judge's score, the reference median, and whether the candidate holds.
"""

import json
import statistics
import sys

CRITERIA = ["L8", "L9", "L10", "L11", "L12", "L13"]
BAR_WITHOUT_MEDIAN = 4.0


def main() -> None:
    key = json.load(open(sys.argv[1]))
    judges = []
    for arg in sys.argv[2:]:
        name, path = arg.split("=", 1)
        judges.append((name, json.load(open(path))["scores"]))
    names = [n for n, _ in judges]
    per = {}
    for name, scores in judges:
        for letter, crit in scores.items():
            subject = key[name][letter]
            for c in CRITERIA:
                v = crit.get(c, ["n/a"])[0]
                per.setdefault(subject, {}).setdefault(c, {})[name] = v
    def mean(subject: str, c: str) -> float | None:
        vals = [v for v in per[subject][c].values() if isinstance(v, (int, float))]
        return statistics.mean(vals) if vals else None
    refs = sorted(s for s in per if s != "candidate")
    print("| Criterion | Candidate (" + "/".join(names) + ") | " + " | ".join(refs) + " | Bar | Holds |")
    print("| --- | --- | " + " | ".join("---" for _ in refs) + " | --- | --- |")
    ok = True
    for c in CRITERIA:
        cm = mean("candidate", c)
        rm = [mean(r, c) for r in refs]
        scored = [m for m in rm if m is not None]
        bar = statistics.median(scored) if len(scored) >= 2 else BAR_WITHOUT_MEDIAN
        holds = cm is not None and cm >= bar - 1e-9
        ok &= holds
        each = "/".join(str(per["candidate"][c].get(n, "n/a")) for n in names)
        fmt = lambda m: "n/a" if m is None else f"{m:.2f}"
        print(f"| {c} | {fmt(cm)} ({each}) | " + " | ".join(fmt(m) for m in rm) + f" | {bar:.2f} | {'yes' if holds else 'no'} |")
    print()
    print("Result: " + ("met" if ok else "not met"))


if __name__ == "__main__":
    main()
