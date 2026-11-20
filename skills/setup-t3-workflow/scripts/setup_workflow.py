#!/usr/bin/env python3
"""Detection and config writing for the setup-t3-workflow interview.

`detect` prints one JSON object: the platform, the current slot settings, the
installed candidates for each slot, the T3 providers and models, and whether
t3cm runs. `write` merges slot settings from stdin into the user config file,
refuses a bare name that matches several skills, and prints `t3wf slots`.
`rule` installs the always-loaded filed-issue rule for Claude Code and Codex.
`write` and `rule` record the setup steps they cover, from steps.json, in the
config's `steps`, and the release that wrote it in `writtenBy`.
"""
import argparse
import contextlib
import http.client
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

SUGGESTED = {"review": ["sol-loop"], "passes": ["unslop", "pr"],
             "credentials": ["aws-sso-login"], "delivery": ["deploy"]}
REVIEW_SOURCE = "https://github.com/t3dotgg/tstack"
EFFORT_OPTIONS = ("effort", "reasoningEffort", "thinking_level")
T3CM_PORT = 47731
RULE_FILE = "t3-workflow-filed-issues.md"
RULE_START = "<!-- t3-workflow:filed-issues:start -->"
RULE_END = "<!-- t3-workflow:filed-issues:end -->"
RULE_STEP = "rule:filed-issues"
STAMP_KEY = "t3-codemode-release"
HERE = os.path.dirname(os.path.abspath(__file__))


class Fail(Exception):
    pass


def t3wf_script():
    if os.environ.get("T3WF_SCRIPT"):
        return os.environ["T3WF_SCRIPT"]
    here = os.path.dirname(os.path.abspath(__file__))
    for base in dict.fromkeys((here, os.path.dirname(os.path.realpath(__file__)))):
        path = os.path.join(base, "..", "..", "t3-workflow", "scripts", "t3wf.py")
        if os.path.isfile(path):
            return os.path.normpath(path)
    raise Fail("the t3-workflow skill is not installed next to setup-t3-workflow; install both")


def load_t3wf():
    sys.path.insert(0, os.path.dirname(t3wf_script()))
    import t3wf
    return t3wf


def run_t3wf(*args, ok=(0,)):
    r = subprocess.run([sys.executable, t3wf_script(), *args], capture_output=True, text=True, timeout=120)
    if r.returncode not in ok:
        raise Fail(f"t3wf {' '.join(args)} exited {r.returncode}: {r.stderr.strip()}")
    return r


def common(a):
    return ([f"--repo={a.repo}"] if a.repo else []) + ([f"--skills={a.skills}"] if a.skills else [])


def listed(a):
    return [n.strip().lstrip("/") for n in (a.skills or "").replace(",", " ").split() if n.strip()]


# ---------------------------------------------------------------- steps

def catalog():
    path = os.path.join(HERE, "..", "steps.json")
    try:
        with open(path) as f:
            return json.load(f)["steps"]
    except (OSError, ValueError, KeyError, TypeError) as e:
        raise Fail(f"setup-t3-workflow has no readable steps.json ({e}); update it")


def release():
    try:
        with open(os.path.join(HERE, "..", "SKILL.md")) as f:
            lines = f.read().splitlines()
    except OSError:
        return None
    if not lines or lines[0].strip() != "---":
        return None
    for line in lines[1:]:
        if line.strip() == "---":
            return None
        key, _, value = line.strip().partition(":")
        if key == STAMP_KEY:
            return value.strip().strip('"\'') or None
    return None


def done_steps(cfg, steps):
    if isinstance(cfg.get("steps"), list):
        return list(cfg["steps"])
    return [s["id"] for s in steps if s.get("legacy")]


def record(cfg, done, steps):
    known = [s["id"] for s in steps if s["id"] in done]
    cfg["steps"] = known + [i for i in dict.fromkeys(done) if i not in known]
    if release():
        cfg["writtenBy"] = release()
    else:
        cfg.pop("writtenBy", None)
    return cfg


def rule_done(plan):
    states = [p["status"] for p in plan]
    return "current" in states and all(s in ("current", "no agent") for s in states)


def steps_status(slots):
    steps = catalog()
    path = slots.config_path()
    out = {"config": path, "writtenBy": None, "recorded": [], "missing": []}
    try:
        cfg = slots.load_user_config(path)
    except slots.ConfigError:
        return out
    exists = os.path.exists(path)
    if exists:
        out["recorded"] = done_steps(cfg, steps)
        out["writtenBy"] = cfg.get("writtenBy")
    try:
        rule_missing = any(p["status"] in ("missing", "stale") for p in rule_plan())
    except Fail:
        rule_missing = True
    for s in steps:
        if s["kind"] == "interview" and s["id"] not in out["recorded"]:
            out["missing"].append({"id": s["id"], "kind": s["kind"], "slot": s.get("slot")})
        elif s["kind"] == "auto" and rule_missing:
            out["missing"].append({"id": s["id"], "kind": s["kind"]})
    return out


# ---------------------------------------------------------------- detect

def resolve(a, name):
    data = json.loads(run_t3wf("resolve", "--json", *common(a), "--", name, ok=(0, 1)).stdout)
    out = {}
    for c in data["candidates"]:
        out.setdefault(c["name"], {"name": c["name"], "path": c.get("path"), "where": c.get("where"),
                                   "invoke": "inline" if c.get("inline") else "skill"})
    return data["resolved"]["status"] != "missing", list(out.values())


def configured(eff):
    names = {"review": [], "passes": [], "credentials": [], "delivery": []}
    if eff["review"].get("skill") and eff["review"]["source"] != "default":
        names["review"].append(eff["review"]["skill"]["ref"])
    names["passes"] += [s["ref"] for s in eff["passes"]["skills"]]
    names["credentials"] += [r["skill"]["ref"] for r in eff["credentials"]["renewals"]]
    if eff["delivery"].get("skill"):
        names["delivery"].append(eff["delivery"]["skill"]["ref"])
    return names


def candidates(a, eff):
    voices = [n for n in listed(a) if "voice" in n.rsplit(":", 1)[-1]]
    have = configured(eff)
    out = {}
    for slot, suggested in SUGGESTED.items():
        names = dict.fromkeys(have[slot] + suggested + (voices if slot == "passes" else []))
        found = {n: resolve(a, n) for n in names}
        out[slot] = {"configured": have[slot], "found": {n: c for n, (ok, c) in found.items() if ok},
                     "missing": [n for n, (ok, _) in found.items() if not ok]}
        others = {n: c for n, (ok, c) in found.items() if not ok and c}
        if others:
            out[slot]["installedInstead"] = others
    if not out["review"]["found"].get("sol-loop"):
        out["review"]["suggest"] = f"sol-loop from {REVIEW_SOURCE}"
    return out


def model_line(model):
    for o in model.get("options", []):
        if o.get("id") in EFFORT_OPTIONS:
            default = f"; default {o['default']}" if o.get("default") else ""
            return f"{model.get('id')} (effort {'|'.join(map(str, o.get('values', [])))}{default})"
    return model.get("id")


def providers():
    t3wf = load_t3wf()
    try:
        with t3wf.t3_session("setup-t3-workflow") as t3:
            caps = t3.call("orchestrator_capabilities")
    except Exception as e:
        return {"error": str(e) or type(e).__name__}
    enabled, disabled = [], []
    for p in caps.get("providers", []):
        if any("disabled" in str(c).lower() for c in p.get("constraints", [])):
            disabled.append(p.get("providerInstanceId"))
            continue
        entry = {"provider": p.get("providerInstanceId"), "name": p.get("displayName"),
                 "models": [model_line(m) for m in p.get("models", [])]}
        if p.get("omittedModels"):
            entry["omittedModels"] = p["omittedModels"]
        enabled.append(entry)
    return {"enabled": enabled, "disabled": disabled}


def systemd_user():
    try:
        r = subprocess.run([os.environ.get("T3WF_SYSTEMCTL", "systemctl"), "--user", "show-environment"],
                           capture_output=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return False
    return r.returncode == 0


def platform_info():
    system = platform.system().lower()
    user_units = system == "linux" and systemd_user()
    return {"system": system, "wsl": "microsoft" in platform.release().lower(),
            "systemdUser": user_units, "t3cmSupported": user_units}


def t3cm():
    local = os.path.join(os.path.expanduser("~"), ".local", "bin", "t3cm")
    out = {"binary": shutil.which("t3cm") or (local if os.path.isfile(local) else None),
           "running": False, "connected": False}
    port = os.environ.get("T3CM_PORT") or str(T3CM_PORT)
    if not port.isdigit():
        out["error"] = f"T3CM_PORT={port!r} is not a port number"
        return out
    out["url"] = f"http://127.0.0.1:{port}/"
    try:
        direct = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with direct.open(f"http://127.0.0.1:{port}/healthz", timeout=3) as r:
            health = json.loads(r.read().decode())
    except (OSError, ValueError, http.client.HTTPException) as e:
        out["error"] = str(e)
        return out
    if not isinstance(health, dict) or not isinstance(health.get("t3"), dict) \
            or not isinstance(health["t3"].get("connected"), bool):
        out["error"] = "/healthz did not answer like t3cm"
        return out
    out.update(running=True, version=health.get("version"),
               connected=health["t3"]["connected"])
    return out


def user_config_problems(slots):
    path = slots.config_path()
    try:
        cfg = slots.load_user_config(path)
    except slots.ConfigError as e:
        return [str(e)]
    return slots.validate_config(cfg, os.path.basename(path)) if cfg else []


def cmd_detect(a):
    slots = load_t3wf().slots
    eff = json.loads(run_t3wf("slots", "--json", *common(a)).stdout)
    mine = user_config_problems(slots)
    print(json.dumps({"platform": platform_info(),
                      "config": {"path": slots.config_path(), "exists": eff["config"] is not None,
                                 "problems": mine,
                                 "repoProblems": [p for p in eff["problems"] if p not in mine]},
                      "summary": slots.report_lines(eff),
                      "slots": candidates(a, eff),
                      "providers": providers(),
                      "rule": rule_status(),
                      "steps": steps_status(slots),
                      "t3cm": t3cm()}, indent=1))


# ---------------------------------------------------------------- rule

def rule_text():
    path = os.path.join(os.path.dirname(t3wf_script()), "..", "rules", "report-filed-issues.md")
    try:
        with open(path) as f:
            return f.read()
    except OSError as e:
        raise Fail(f"the t3-workflow skill has no filed-issue rule ({e}); update it")


def codex_block(text):
    return f"{RULE_START}\n{text.rstrip()}\n{RULE_END}\n"


def read_or_empty(path):
    try:
        with open(path) as f:
            return f.read()
    except FileNotFoundError:
        return ""
    except (OSError, UnicodeDecodeError) as e:
        raise Fail(f"cannot read {path}: {e}")


def spliced(old, block):
    start, end = old.find(RULE_START), old.find(RULE_END)
    if start != -1 and end > start:
        rest = old[end + len(RULE_END):].lstrip("\n")
        return old[:start] + block + ("\n" + rest if rest else "")
    return (old.rstrip("\n") + "\n\n" if old.strip() else "") + block


def rule_plan():
    slots = load_t3wf().slots
    text = rule_text()
    codex = slots.codex_dir()
    override = os.path.join(codex, "AGENTS.override.md")
    plan = []
    for agent, home_dir, path in (
            ("claude", slots.claude_dir(), os.path.join(slots.claude_dir(), "rules", RULE_FILE)),
            ("codex", codex, override if os.path.isfile(override) else os.path.join(codex, "AGENTS.md"))):
        if not os.path.isdir(home_dir):
            plan.append({"agent": agent, "path": path, "status": "no agent", "home": home_dir})
            continue
        path = os.path.realpath(path)
        old = read_or_empty(path)
        start, end = old.find(RULE_START), old.find(RULE_END)
        if agent == "codex" and ((start == -1) != (end == -1) or end < start):
            raise Fail(f"{path} has a {RULE_START} or {RULE_END} marker without its pair; fix or remove it, "
                       "then run the command again")
        new = text if agent == "claude" else spliced(old, codex_block(text))
        ours = old if agent == "claude" else RULE_START in old
        status = "current" if old == new else "stale" if ours else "missing"
        plan.append({"agent": agent, "path": path, "status": status, "new": new})
    return plan


def rule_status():
    try:
        return {p["agent"]: {"status": p["status"], "path": p["path"]} for p in rule_plan()}
    except Fail as e:
        return {"error": str(e)}


def save(path, text):
    tmp = None
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".t3wf.", suffix=".md")
        with os.fdopen(fd, "w") as f:
            f.write(text)
        os.replace(tmp, path)
    except OSError as e:
        if tmp:
            with contextlib.suppress(OSError):
                os.remove(tmp)
        raise Fail(f"not written: {path}: {e}")


def record_rule():
    slots = load_t3wf().slots
    path = os.path.realpath(slots.config_path())
    if not os.path.exists(path):
        return
    try:
        cfg = slots.load_user_config(path)
    except slots.ConfigError:
        return
    if cfg.get("version", slots.CONFIG_VERSION) != slots.CONFIG_VERSION or slots.validate_config(cfg):
        return
    steps = catalog()
    done = done_steps(cfg, steps)
    if RULE_STEP in done:
        return
    save_config(path, record(cfg, done + [RULE_STEP], steps))
    print(f"recorded {RULE_STEP} in {path}")


def cmd_rule(a):
    plan = rule_plan()
    if all(p["status"] == "no agent" for p in plan):
        raise Fail("found neither a Claude Code nor a Codex config folder; install one of them first")
    for p in plan:
        if p["status"] == "no agent":
            print(f"{p['agent']}: skipped, {p['home']} does not exist")
        elif p["status"] == "current":
            print(f"{p['agent']}: {p['path']} is current")
        else:
            save(p["path"], p["new"])
            print(f"{p['agent']}: wrote {p['path']}")
    record_rule()


# ---------------------------------------------------------------- write

def skill_refs(cfg):
    refs = []
    if isinstance(cfg.get("review"), str):
        refs.append(("review", cfg["review"]))
    refs += [(f"passes[{i}]", r) for i, r in enumerate(cfg.get("passes") or [])]
    refs += [(f"credentials[{i}]", c.get("skill")) for i, c in enumerate(cfg.get("credentials") or [])
             if isinstance(c, dict)]
    if isinstance(cfg.get("delivery"), dict) and cfg["delivery"].get("skill"):
        refs.append(("delivery", cfg["delivery"]["skill"]))
    return refs


def merge(old, new, unset):
    cfg = dict(old)
    for key, value in new.items():
        if key == "models" and isinstance(value, dict):
            base = cfg["models"] if isinstance(cfg.get("models"), dict) else {}
            cfg["models"] = {r: s for r, s in dict(base, **value).items() if s is not None}
        else:
            cfg[key] = value
    slots = load_t3wf().slots
    for key in unset:
        slot, _, role = key.partition(".")
        if slot not in slots.SLOTS or (role and (slot, role) not in (("models", r) for r in slots.MODEL_ROLES)):
            raise Fail(f"--unset {key}: name a slot ({', '.join(slots.SLOTS)}) or models.ticket or models.worker")
        if role:
            if isinstance(cfg.get("models"), dict):
                cfg["models"] = {r: v for r, v in cfg["models"].items() if r != role}
        else:
            cfg.pop(slot, None)
    cfg.pop("version", None)
    return {"version": load_t3wf().slots.CONFIG_VERSION, **cfg}


def save_config(path, cfg):
    tmp = None
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".config.", suffix=".json")
        with os.fdopen(fd, "w") as f:
            json.dump(cfg, f, indent=2)
            f.write("\n")
        os.replace(tmp, path)
    except OSError as e:
        if tmp:
            with contextlib.suppress(OSError):
                os.remove(tmp)
        raise Fail(f"not written: {e}")


def asked_slots(a, new, slots):
    names = [n.strip() for v in a.asked or [] for n in v.split(",") if n.strip()]
    bad = [n for n in names if n not in slots.SLOTS]
    if bad:
        raise Fail(f"--asked {', '.join(bad)}: name a slot ({', '.join(slots.SLOTS)})")
    unset = [u.partition(".")[0] for u in a.unset or []]
    return set(names) | {k for k in new if k in slots.SLOTS} | set(unset)


def cmd_write(a):
    slots = load_t3wf().slots
    try:
        new = json.load(sys.stdin)
    except ValueError as e:
        raise Fail(f"stdin is not JSON: {e}")
    if not isinstance(new, dict):
        raise Fail("stdin must be a JSON object of slot settings")
    kept = [k for k in slots.RECORD_KEYS if k in new]
    if kept:
        raise Fail(f"stdin sets {', '.join(kept)}; setup records those itself")
    path = os.path.realpath(slots.config_path())
    try:
        old = {} if a.replace else slots.load_user_config(path)
    except slots.ConfigError as e:
        raise Fail(f"{e}; fix it, or pass --replace to overwrite it")
    if old.get("version", slots.CONFIG_VERSION) != slots.CONFIG_VERSION:
        raise Fail(f"{path} is config version {old['version']!r}; pass --replace to start a version "
                   f"{slots.CONFIG_VERSION} file")
    cfg = merge(old, new, a.unset or [])
    problems = slots.validate_config(cfg, "new config")
    d = cfg.get("delivery") if "delivery" in new else None
    if isinstance(d, dict) and d.get("mode") == "tag-deploy" and not d.get("skill"):
        problems.append("delivery: tag-deploy needs a deploy skill")
    if isinstance(cfg.get("models"), dict):
        problems += [f"models.{r}: names no model" for r, spec in cfg["models"].items()
                     if isinstance(spec, dict) and "model" not in spec and "pool" not in spec]
    if problems:
        raise Fail("not written:\n" + "\n".join(problems))
    resolver = slots.Resolver(slots.repo_root(a.repo), listed(a))
    changed = set(skill_refs({k: cfg[k] for k in new if k in cfg}))
    warnings = []
    for where, ref in skill_refs(cfg):
        hit = resolver.resolve(ref)
        if hit["status"] == "ambiguous":
            text = f"{where}: {ref} matches {hit['name']} and {', '.join(hit['others'])}; record one full name"
            (problems if (where, ref) in changed else warnings).append(text)
        elif hit["status"] == "missing":
            warnings.append(f"{where}: {ref} is not installed yet; the roles skip it until it is")
    if problems:
        raise Fail("not written:\n" + "\n".join(problems))
    steps = catalog()
    done = done_steps(old, steps) if os.path.exists(path) and not a.replace else []
    by_slot = {s.get("slot"): s["id"] for s in steps if s["kind"] == "interview"}
    done += [by_slot[n] for n in sorted(asked_slots(a, new, slots)) if n in by_slot]
    try:
        if rule_done(rule_plan()):
            done.append(RULE_STEP)
    except Fail:
        pass
    save_config(path, record(cfg, done, steps))
    print(f"wrote {path}")
    for w in warnings:
        print(f"warn: {w}")
    try:
        print(run_t3wf("slots", *common(a)).stdout, end="")
    except (Fail, subprocess.TimeoutExpired) as e:
        print(f"warn: saved, but the slot summary failed ({e}); run t3wf slots")


def main(argv=None):
    p = argparse.ArgumentParser(prog="setup_workflow.py", description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    for name, fn in (("detect", cmd_detect), ("write", cmd_write), ("rule", cmd_rule)):
        s = sub.add_parser(name)
        s.set_defaults(fn=fn)
        s.add_argument("--repo", help="the repository (default: the current directory's)")
        s.add_argument("--skills", help="skills listed in this session, comma or space separated")
    w = sub.choices["write"]
    w.add_argument("--unset", action="append", metavar="SLOT[.ROLE]",
                   help="drop a slot (or one model role) so its default applies")
    w.add_argument("--replace", action="store_true", help="start from an empty config, not the current file")
    w.add_argument("--asked", action="append", metavar="SLOT[,SLOT]",
                   help="a slot the user was asked about, kept ones too, recorded as a done setup step")
    a = p.parse_args(argv)
    try:
        a.fn(a)
    except Fail as e:
        print(f"setup_workflow: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
