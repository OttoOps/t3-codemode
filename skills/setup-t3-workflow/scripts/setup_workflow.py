#!/usr/bin/env python3
"""Detection and config writing for the setup-t3-workflow interview.

`detect` prints one JSON object: the platform, the current slot settings, the
installed candidates for each slot, the T3 providers and models, and whether
t3cm runs. `write` merges slot settings from stdin into the user config file,
refuses a bare name that matches several skills, and prints `t3wf slots`.
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
                      "t3cm": t3cm()}, indent=1))


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


def cmd_write(a):
    slots = load_t3wf().slots
    try:
        new = json.load(sys.stdin)
    except ValueError as e:
        raise Fail(f"stdin is not JSON: {e}")
    if not isinstance(new, dict):
        raise Fail("stdin must be a JSON object of slot settings")
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
    for name, fn in (("detect", cmd_detect), ("write", cmd_write)):
        s = sub.add_parser(name)
        s.set_defaults(fn=fn)
        s.add_argument("--repo", help="the repository (default: the current directory's)")
        s.add_argument("--skills", help="skills listed in this session, comma or space separated")
    w = sub.choices["write"]
    w.add_argument("--unset", action="append", metavar="SLOT[.ROLE]",
                   help="drop a slot (or one model role) so its default applies")
    w.add_argument("--replace", action="store_true", help="start from an empty config, not the current file")
    a = p.parse_args(argv)
    try:
        a.fn(a)
    except Fail as e:
        print(f"setup_workflow: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
