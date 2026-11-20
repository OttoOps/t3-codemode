"""Pluggable slots for the T3 workflow roles.

Five slots name capabilities, not skills: an independent review before a PR,
ordered worker passes, credential renewal, delivery, and the models for ticket
and worker threads, each a model or a t3cm pool. Settings come from built-in
defaults, then the user config file, then the repository's `AGENTS.md` "Agent
workflow" block; the last one that sets a slot wins. A configured skill is
found by its bare name in any namespace.
"""
import json
import os
import re
import subprocess

CONFIG_VERSION = 1
SLOTS = ("review", "passes", "credentials", "delivery", "models")
TOP_KEYS = ("version",) + SLOTS
DELIVERY_MODES = ("merge", "tag-deploy")
MODEL_ROLES = ("ticket", "worker")
DEFAULT_REVIEW = "sol-loop"
DEFAULT_MODEL = {"provider": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"}
REVIEW_SUGGESTION = "sol-loop from https://github.com/t3dotgg/tstack"
SKILL_REF = re.compile(r"^(?:[A-Za-z0-9._/-]+:)?[A-Za-z0-9._-]+$")
POOL_NOTE = "t3cm picks the instance at launch with wf.pick"
EFFORT_WORD = re.compile(r"\b(low|medium|high|xhigh|max|ultrathink)\b", re.I)


class ConfigError(Exception):
    pass


# ---------------------------------------------------------------- locations

def home():
    return os.path.expanduser("~")


def claude_dir():
    return os.environ.get("CLAUDE_CONFIG_DIR") or os.path.join(home(), ".claude")


def codex_dir():
    return os.environ.get("CODEX_HOME") or os.path.join(home(), ".codex")


def config_path():
    if os.environ.get("T3WF_CONFIG"):
        return os.environ["T3WF_CONFIG"]
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(home(), ".config")
    return os.path.join(base, "t3-workflow", "config.json")


def git(root, *args):
    try:
        r = subprocess.run(["git", "-C", root, *args], capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout.strip() if r.returncode == 0 else None


def repo_root(start=None):
    start = os.path.abspath(start or os.getcwd())
    return git(start, "rev-parse", "--show-toplevel") or start


# ---------------------------------------------------------------- user config

def load_user_config(path=None):
    path = path or config_path()
    try:
        with open(path) as f:
            cfg = json.load(f)
    except FileNotFoundError:
        return {}
    except (OSError, ValueError) as e:
        raise ConfigError(f"{path}: {e}")
    if not isinstance(cfg, dict):
        raise ConfigError(f"{path}: the top level must be a JSON object")
    return cfg


def _skill_ref_problem(where, v):
    if not isinstance(v, str) or not SKILL_REF.match(v):
        return f"{where}: {v!r} is not a skill name (bare `name` or `namespace:name`)"
    return None


def validate_slots(cfg, where):
    problems = []
    if "review" in cfg and cfg["review"] is not None:
        problems.append(_skill_ref_problem(f"{where}.review", cfg["review"]))
    if "passes" in cfg:
        if not isinstance(cfg["passes"], list):
            problems.append(f"{where}.passes: must be a list of skill names")
        else:
            problems += [_skill_ref_problem(f"{where}.passes[{i}]", v) for i, v in enumerate(cfg["passes"])]
    if "credentials" in cfg:
        creds = cfg["credentials"]
        if not isinstance(creds, list):
            problems.append(f"{where}.credentials: must be a list of {{\"match\", \"skill\"}} objects")
        else:
            for i, c in enumerate(creds):
                w = f"{where}.credentials[{i}]"
                if not isinstance(c, dict) or set(c) != {"match", "skill"}:
                    problems.append(f"{w}: needs exactly the keys match and skill")
                    continue
                problems.append(_skill_ref_problem(f"{w}.skill", c["skill"]))
                try:
                    re.compile(c["match"])
                except (re.error, TypeError) as e:
                    problems.append(f"{w}.match: not a regular expression: {e}")
    if "delivery" in cfg:
        d = cfg["delivery"]
        if not isinstance(d, dict):
            problems.append(f"{where}.delivery: must be an object with mode (and skill for tag-deploy)")
        else:
            extra = set(d) - {"mode", "skill"}
            if extra:
                problems.append(f"{where}.delivery: unknown keys {sorted(extra)}")
            if "mode" in d and d["mode"] not in DELIVERY_MODES:
                problems.append(f"{where}.delivery.mode: {d['mode']!r} is not one of {', '.join(DELIVERY_MODES)}")
            if "skill" in d:
                problems.append(_skill_ref_problem(f"{where}.delivery.skill", d["skill"]))
    if "models" in cfg:
        m = cfg["models"]
        if not isinstance(m, dict):
            problems.append(f"{where}.models: must be an object keyed by ticket and worker")
        else:
            for role, spec in m.items():
                w = f"{where}.models.{role}"
                if role not in MODEL_ROLES:
                    problems.append(f"{w}: unknown role; roles are {', '.join(MODEL_ROLES)}")
                elif isinstance(spec, dict) and "pool" in spec:
                    if set(spec) != {"pool"}:
                        problems.append(f"{w}: a pool sets the model and effort, so pool stands alone")
                    elif not isinstance(spec["pool"], str) or not spec["pool"]:
                        problems.append(f"{w}.pool: must be a non-empty string")
                elif not isinstance(spec, dict) or set(spec) - {"model", "effort", "provider"}:
                    problems.append(f"{w}: must be an object with model, and optionally effort and provider, "
                                    f"or with pool alone")
                else:
                    for k, v in spec.items():
                        if not isinstance(v, str) or not v:
                            problems.append(f"{w}.{k}: must be a non-empty string")
    return [p for p in problems if p]


def validate_config(cfg, where="config"):
    problems = []
    extra = set(cfg) - set(TOP_KEYS)
    if extra:
        problems.append(f"{where}: unknown keys {sorted(extra)}; keys are {', '.join(TOP_KEYS)}")
    if cfg.get("version", CONFIG_VERSION) != CONFIG_VERSION:
        problems.append(f"{where}.version: {cfg['version']!r}, expected {CONFIG_VERSION}")
    return problems + validate_slots(cfg, where)


# ---------------------------------------------------------------- AGENTS.md

def _first_name(value):
    ticked = re.findall(r"`([^`]+)`", value)
    if ticked:
        return ticked[0].strip()
    words = value.split()
    return words[0].strip(".,;()") if words else ""


def _names(value):
    ticked = re.findall(r"`([^`]+)`", value)
    if ticked:
        return [t.strip() for t in ticked]
    return [w.strip(" .") for w in value.split(",") if w.strip(" .")]


def _is_none(value):
    return value.strip().strip("`").lower() in ("none", "no", "off", "-")


def agent_workflow_lines(text):
    lines, level = [], None
    for raw in text.splitlines():
        m = re.match(r"^(#{1,6})\s+(.*?)\s*#*\s*$", raw)
        if m:
            if level is not None and len(m.group(1)) <= level:
                break
            if level is None and m.group(2).strip().lower() == "agent workflow":
                level = len(m.group(1))
            continue
        if level is not None:
            b = re.match(r"^\s*[-*]\s+([^:]+?):\s*(.*)$", raw)
            if b:
                lines.append((b.group(1).strip().lower(), b.group(2).strip()))
    return lines


def parse_agent_workflow(text):
    """Slot settings from an AGENTS.md "Agent workflow" block; keys the block leaves out are absent."""
    cfg, problems = {}, []
    for key, value in agent_workflow_lines(text):
        if key in ("review", "independent review"):
            cfg["review"] = None if _is_none(value) else _first_name(value)
        elif key in ("passes", "worker passes"):
            cfg["passes"] = [] if _is_none(value) else _names(value)
        elif key in ("credential renewal", "credentials"):
            pairs = []
            if not _is_none(value):
                for part in value.split(";"):
                    sides = re.split(r"\s*(?:->|→)\s*", part.strip(), maxsplit=1)
                    if len(sides) != 2:
                        problems.append(f"AGENTS.md Credential renewal: {part.strip()!r} is not `match` -> `skill`")
                        continue
                    pairs.append({"match": sides[0].strip().strip("`"), "skill": _first_name(sides[1])})
            cfg["credentials"] = pairs
        elif key == "delivery":
            cfg.setdefault("delivery", {})["mode"] = _first_name(value)
        elif key == "deploy skill":
            cfg.setdefault("delivery", {})["skill"] = _first_name(value)
        elif key in ("ticket model", "worker model"):
            pool = re.match(r"^\s*(?:the\s+)?pool:?\s+(.*)$", value, re.I)
            if pool:
                cfg.setdefault("models", {})[key.split()[0]] = {"pool": _first_name(pool.group(1))}
                continue
            spec = {"model": _first_name(value)}
            rest = value.replace(spec["model"], "", 1)
            effort = EFFORT_WORD.search(rest)
            if effort:
                spec["effort"] = effort.group(1).lower()
            cfg.setdefault("models", {})[key.split()[0]] = spec
    problems += [p.replace("AGENTS.md.", "AGENTS.md ") for p in validate_slots(cfg, "AGENTS.md")]
    return cfg, problems


def repo_override(root):
    path = os.path.join(root, "AGENTS.md")
    try:
        with open(path) as f:
            return parse_agent_workflow(f.read())
    except OSError:
        return {}, []


# ---------------------------------------------------------------- skills

def frontmatter(path):
    try:
        with open(path) as f:
            text = f.read(8192)
    except OSError:
        return {}
    m = re.match(r"^---\s*\n(.*?)\n---\s*(?:\n|$)", text, re.S)
    if not m:
        return {}
    meta = {}
    for line in m.group(1).splitlines():
        kv = re.match(r"^([A-Za-z0-9_-]+):\s*(.*)$", line)
        if kv:
            meta[kv.group(1)] = kv.group(2).strip().strip("\"'")
    return meta


def is_inline(meta):
    return str(meta.get("disable-model-invocation", "")).lower() in ("true", "yes", "1")


def _skill_dirs(base):
    try:
        names = sorted(os.listdir(base))
    except OSError:
        return []
    return [os.path.join(base, n, "SKILL.md") for n in names
            if os.path.isfile(os.path.join(base, n, "SKILL.md"))]


def _version_key(v):
    return [int(x) if x.isdigit() else -1 for x in re.split(r"[.+-]", v)]


def newest_cached(cache):
    """(plugin name, skills dir) for the newest version of each plugin in a cache/<market>/<plugin>/<version> tree."""
    roots = []
    for market in sorted(os.listdir(cache)) if os.path.isdir(cache) else []:
        mdir = os.path.join(cache, market)
        for plugin in sorted(os.listdir(mdir)) if os.path.isdir(mdir) else []:
            pdir = os.path.join(mdir, plugin)
            versions = [v for v in os.listdir(pdir) if os.path.isdir(os.path.join(pdir, v))]
            if versions:
                roots.append((plugin, os.path.join(pdir, max(versions, key=_version_key), "skills")))
    return roots


def plugin_skill_roots(root):
    """(plugin name, skills dir) for each installed Claude plugin, then each cached Codex plugin."""
    return claude_plugin_roots(root) + newest_cached(os.path.join(codex_dir(), "plugins", "cache"))


def claude_plugin_roots(root):
    """installed_plugins.json, else the newest cached version."""
    plugins = os.path.join(claude_dir(), "plugins")
    roots = []
    try:
        with open(os.path.join(plugins, "installed_plugins.json")) as f:
            installed = json.load(f).get("plugins", {})
        for key, entries in installed.items():
            name = key.split("@", 1)[0]
            for e in entries if isinstance(entries, list) else [entries]:
                scoped = e.get("projectPath")
                if scoped and os.path.realpath(scoped) != os.path.realpath(root):
                    continue
                if e.get("installPath"):
                    roots.append((name, os.path.join(e["installPath"], "skills")))
        return roots
    except (OSError, ValueError, AttributeError):
        return newest_cached(os.path.join(plugins, "cache"))


def search_roots(root):
    """(where, namespace, skills dir) in search order after the session's own list."""
    roots = [("~/.claude/skills", None, os.path.join(claude_dir(), "skills")),
             ("~/.agents/skills", None, os.path.join(home(), ".agents", "skills")),
             ("project .claude/skills", None, os.path.join(root, ".claude", "skills")),
             ("~/.codex/skills", None, os.path.join(codex_dir(), "skills"))]
    roots += [("plugin cache", plugin, d) for plugin, d in plugin_skill_roots(root)]
    return roots


def installed_skills(root):
    """Every SKILL.md on disk, once each by real path and full name: {name, bare, path, inline, where}."""
    seen, found = set(), []
    for where, ns, base in search_roots(root):
        for path in _skill_dirs(base):
            meta = frontmatter(path)
            bare = meta.get("name") or os.path.basename(os.path.dirname(path))
            name = f"{ns}:{bare}" if ns else bare
            key = (os.path.realpath(path), name)
            if key in seen:
                continue
            seen.add(key)
            found.append({"name": name, "bare": bare,
                          "dir": os.path.basename(os.path.dirname(path)), "path": path,
                          "inline": is_inline(meta), "where": where})
    return found


def bare(ref):
    return ref.rsplit(":", 1)[-1]


class Resolver:
    """Finds skills by bare name in any namespace: the session's listed skills first, then the install locations.

    An exact full name wins; a skill installed with no namespace has its bare name as its full name.
    """

    def __init__(self, root, listed=()):
        self.root = root
        self.listed = [s.strip().lstrip("/") for s in listed if s.strip()]
        self._disk = None

    def disk(self):
        if self._disk is None:
            self._disk = installed_skills(self.root)
        return self._disk

    def candidates(self, ref):
        """Every match in search order; a listed skill takes the path of its first install on disk."""
        want = bare(ref)
        out, used = [], set()
        for name in dict.fromkeys(n for n in self.listed if bare(n) == want):
            on_disk = next((s for s in self.disk() if s["name"] == name), None)
            if on_disk:
                used.add(on_disk["path"])
            out.append(dict(on_disk or {"name": name, "bare": want, "path": None, "inline": False},
                            where="session"))
        out += [s for s in self.disk() if want in (s["bare"], s["dir"]) and s["path"] not in used]
        return out

    def resolve(self, ref):
        cands = self.candidates(ref)
        names = list(dict.fromkeys(c["name"] for c in cands))
        exact = [c for c in cands if c["name"] == ref]
        hit = exact or ([] if ":" in ref else cands)
        if not hit:
            return {"ref": ref, "status": "missing", "others": names}
        e = _entry(ref, hit[0])
        others = [n for n in names if n != e["name"]]
        if others and not exact:
            e.update(status="ambiguous", others=others)
        else:
            e["status"] = "ok"
        return e


def _entry(ref, c):
    return {"ref": ref, "name": c["name"], "path": c.get("path"), "where": c.get("where"),
            "invoke": "inline" if c.get("inline") else "skill"}


# ---------------------------------------------------------------- effective settings

def merged(root, user_cfg):
    """Each slot's value and the layer it came from; delivery and models merge field by field."""
    block, problems = repo_override(root)
    ls = [("user", user_cfg), ("AGENTS.md", block)]
    eff = {"review": (None, "default"), "passes": ([], "default"), "credentials": ([], "default"),
           "delivery": ({"mode": "merge"}, "default"),
           "models": {r: (None, "default") for r in MODEL_ROLES}}
    review_set = False
    for src, cfg in ls:
        for slot in ("review", "passes", "credentials"):
            if slot in cfg:
                eff[slot] = (cfg[slot], src)
                review_set = review_set or slot == "review"
        if isinstance(cfg.get("delivery"), dict):
            eff["delivery"] = (dict(eff["delivery"][0], **cfg["delivery"]), src)
        for role, spec in (cfg.get("models") or {}).items():
            if role in MODEL_ROLES and isinstance(spec, dict):
                prev = eff["models"][role][0] or {}
                same = spec.get("model") in (None, prev.get("model"))
                base = prev if same and "pool" not in spec and "pool" not in prev else {}
                eff["models"][role] = (dict(base, **spec), src)
    return eff, review_set, problems


def model_offers(caps, model, provider=None):
    """Enabled providers that offer `model`, from orchestrator_capabilities output, as {provider: model entry}."""
    offers = {}
    for p in (caps or {}).get("providers", []):
        if any("disabled" in str(c).lower() for c in p.get("constraints", [])):
            continue
        pid = p.get("providerInstanceId")
        if provider and provider not in (pid, p.get("driverKind")):
            continue
        for m in p.get("models", []):
            if m.get("id") == model:
                offers[pid] = m
    return offers


def effort_values(entry):
    for o in (entry or {}).get("options", []):
        if o.get("id") in ("effort", "thinking_level"):
            return [str(v) for v in o.get("values", [])]
    return None


def check_model(spec, caps):
    """None when T3 offers the model at that effort; else what is wrong."""
    offers = model_offers(caps, spec["model"], spec.get("provider"))
    on = f" on {spec['provider']}" if spec.get("provider") else ""
    if not offers:
        return f"T3 offers no enabled provider with model {spec['model']}{on}"
    if spec.get("effort"):
        allowed = [effort_values(m) for m in offers.values()]
        if all(a is not None and spec["effort"] not in a for a in allowed):
            return f"effort {spec['effort']} is not offered for {spec['model']}{on} ({', '.join(allowed[0])})"
    return None


def default_model(caps, why_unchecked="T3 not checked"):
    if caps is None:
        return None, f"{why_unchecked}, so Claude is unconfirmed"
    if model_offers(caps, DEFAULT_MODEL["model"], DEFAULT_MODEL["provider"]):
        return dict(DEFAULT_MODEL), None
    return None, "T3 offers no Claude"


class Settings:
    """The effective slot settings for one repository."""

    def __init__(self, root, listed=(), caps_fn=None, user_cfg=None, config_file=None):
        self.root = root
        self.config_file = config_file or config_path()
        self.config_exists = os.path.exists(self.config_file)
        self.config_problems = []
        if user_cfg is None:
            try:
                user_cfg = load_user_config(self.config_file)
            except ConfigError as e:
                self.config_problems.append(str(e))
                user_cfg = {}
        self.config_problems += validate_config(user_cfg, os.path.basename(self.config_file)) if user_cfg else []
        if self.config_problems:
            user_cfg = {}
        self.user_cfg = user_cfg
        self.resolver = Resolver(root, listed)
        self.caps_fn = caps_fn
        self.caps_error = None
        self._caps = {}
        self.eff, self.review_set, self.repo_problems = merged(root, user_cfg)

    def caps(self, search):
        if self.caps_fn is None:
            return None
        if search not in self._caps:
            try:
                self._caps[search] = self.caps_fn(search)
            except Exception as e:
                self.caps_error = str(e)
                self._caps[search] = None
        return self._caps[search]

    def review(self):
        ref, src = self.eff["review"]
        if not self.review_set:
            hit = self.resolver.resolve(DEFAULT_REVIEW)
            if hit["status"] == "missing":
                return {"slot": "review", "source": "default", "skill": None,
                        "note": f"no reviewer installed; suggest {REVIEW_SUGGESTION}"}
            return {"slot": "review", "source": "default", "skill": hit}
        if ref is None:
            return {"slot": "review", "source": src, "skill": None, "note": "none configured"}
        return {"slot": "review", "source": src, "skill": self.resolver.resolve(ref)}

    def passes(self):
        refs, src = self.eff["passes"]
        return {"slot": "passes", "source": src, "skills": [self.resolver.resolve(r) for r in refs]}

    def credentials(self):
        pairs, src = self.eff["credentials"]
        return {"slot": "credentials", "source": src,
                "renewals": [{"match": p["match"], "skill": self.resolver.resolve(p["skill"])} for p in pairs]}

    def delivery(self):
        d, src = self.eff["delivery"]
        out = {"slot": "delivery", "source": src, "mode": d.get("mode", "merge")}
        if d.get("skill"):
            out["skill"] = self.resolver.resolve(d["skill"])
        elif out["mode"] == "tag-deploy":
            out["skill"] = None
            out["problem"] = "tag-deploy names no deploy skill"
        return out

    def model(self, role):
        spec, src = self.eff["models"][role]
        if spec is None:
            caps = self.caps(DEFAULT_MODEL["model"])
            spec, note = default_model(caps, f"T3 unreachable ({self.caps_error})" if self.caps_error
                                       else "T3 not checked")
            return {"slot": f"model.{role}", "source": "default", "spec": spec, "note": note}
        out = {"slot": f"model.{role}", "source": src, "spec": spec}
        if "pool" in spec:
            out["note"] = POOL_NOTE
            return out
        if "model" not in spec:
            out["problem"] = "names no model"
            return out
        caps = self.caps(spec["model"])
        if caps is None:
            out["note"] = "unverified: " + (f"T3 unreachable ({self.caps_error})" if self.caps_error
                                            else "T3 not checked")
        else:
            problem = check_model(spec, caps)
            if problem:
                out["problem"] = problem
        return out

    def all(self):
        return {"repo": self.root, "config": self.config_file if self.config_exists else None,
                "review": self.review(), "passes": self.passes(), "credentials": self.credentials(),
                "delivery": self.delivery(), "models": {r: self.model(r) for r in MODEL_ROLES}}


# ---------------------------------------------------------------- reports

def skill_text(e):
    if e is None:
        return "none"
    if e["status"] == "missing":
        alt = f" (installed: {', '.join(e['others'])})" if e.get("others") else ""
        return f"SKIP {e['ref']}: no longer resolves{alt}"
    how = f"inline: read {e['path']} and apply it" if e["invoke"] == "inline" else "Skill tool"
    text = f"{e['name']} ({how})"
    if e["status"] == "ambiguous":
        text += f" [also: {', '.join(e['others'])}]"
    return text


def model_text(m):
    s = m["spec"]
    if s is None:
        text = "provider default"
    elif "pool" in s:
        text = f"pool {s['pool']}"
    else:
        text = s.get("model", "?") + (f" effort {s['effort']}" if s.get("effort") else "") + \
            (f" on {s['provider']}" if s.get("provider") else "")
    if m.get("problem"):
        text += f" - PROBLEM: {m['problem']}"
    elif m.get("note"):
        text += f" ({m['note']})"
    return text


def report_lines(eff):
    r = eff["review"]
    review = skill_text(r["skill"]) + (f" ({r['note']})" if r.get("note") else "")
    p = eff["passes"]
    passes = "; ".join(skill_text(s) for s in p["skills"]) or "none"
    c = eff["credentials"]
    creds = "; ".join(f"/{x['match']}/ -> {skill_text(x['skill'])}" for x in c["renewals"]) or "none"
    d = eff["delivery"]
    delivery = d["mode"]
    if d.get("skill"):
        delivery += (" via " if d["mode"] == "tag-deploy" else "; deploy skill ") + skill_text(d["skill"])
    elif d.get("problem"):
        delivery += " via ? - PROBLEM: " + d["problem"]
    lines = [f"repo: {eff['repo']}; config: {eff['config'] or 'none (defaults)'}",
             f"review: {review} [{r['source']}]",
             f"passes: {passes} [{p['source']}]",
             f"credentials: {creds} [{c['source']}]",
             f"delivery: {delivery} [{d['source']}]"]
    lines += [f"model {role}: {model_text(m)} [{m['source']}]" for role, m in eff["models"].items()]
    return lines


def configured_skills(eff):
    """(where, resolution) for every skill the settings name."""
    out = []
    if eff["review"]["skill"] and eff["review"]["source"] != "default":
        out.append(("review", eff["review"]["skill"]))
    out += [(f"passes[{i}]", s) for i, s in enumerate(eff["passes"]["skills"])]
    out += [(f"credentials /{x['match']}/", x["skill"]) for x in eff["credentials"]["renewals"]]
    if eff["delivery"].get("skill"):
        out.append(("delivery", eff["delivery"]["skill"]))
    return out


def doctor(settings):
    """(problems, warnings, eff) for the effective settings."""
    problems = list(settings.config_problems) + list(settings.repo_problems)
    warnings = []
    eff = settings.all()
    for where, e in configured_skills(eff):
        if e["status"] == "missing":
            alt = f"; installed: {', '.join(e['others'])}" if e.get("others") else ""
            problems.append(f"{where}: skill {e['ref']} no longer resolves{alt}")
        elif e["status"] == "ambiguous":
            warnings.append(f"{where}: {e['ref']} matches {e['name']} and {', '.join(e['others'])}; "
                            f"record the full name")
    if eff["delivery"].get("problem"):
        problems.append(f"delivery: {eff['delivery']['problem']}")
    for role, m in eff["models"].items():
        if m.get("problem"):
            problems.append(f"model {role}: {m['problem']}")
        elif m["source"] != "default" and m.get("note") and "pool" not in (m["spec"] or {}):
            warnings.append(f"model {role}: {m['note']}")
    if eff["review"]["skill"] is None and eff["review"]["source"] == "default":
        warnings.append(f"review: {eff['review']['note']}")
    return problems, warnings, eff
