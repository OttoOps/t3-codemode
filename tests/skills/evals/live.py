#!/usr/bin/env python3
"""Live evals for the skills in skills/: trigger evals and with/without-skill behavior evals.

Every run happens in a throwaway sandbox: its own HOME, XDG dirs and T3 home, a
stub T3 MCP server, a fake gh on PATH, and a git repo whose pushes land in a
local bare repo. Nothing reads or writes the real ~/.claude, ~/.agents or
~/.codex, a T3 server, or GitHub. Auth comes from ANTHROPIC_API_KEY or
CLAUDE_CODE_OAUTH_TOKEN in the environment.

    python3 tests/skills/evals/live.py triggers [--skill NAME ...]
    python3 tests/skills/evals/live.py behavior [--skill NAME ...] [--eval ID ...]
"""
import argparse
import concurrent.futures
import hashlib
import json
import multiprocessing
import os
import pathlib
import re
import shlex
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import guard_hook  # noqa: E402
ROOT = HERE.parents[2]
SKILLS = ROOT / "skills"
T3WF = SKILLS / "t3-workflow" / "scripts" / "t3wf.py"
DEFAULT_MODEL = "claude-opus-5-5"
AUTH_VARS = ("ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN")
REPO_URL = "https://github.com/acme/widgets.git"
SELF = "thread-eval"
PARENT = "thread-ticket"
T0 = "2026-01-05T12:00:00Z"
RUNTIME_NOTE = ("<runtime_info>You are running in T3 Code through the Claude Code harness. "
                "No need to mention this otherwise.</runtime_info>\n")
ALLOW = ["Bash", "Read", "Edit", "Write", "Glob", "Grep", "Skill", "ToolSearch", "mcp__t3-code"]
DENY = ["WebFetch", "WebSearch", "AskUserQuestion"]
CHECK_KINDS = ("tool", "bash", "text", "file", "gh", "t3", "repo")
LINK_HEADING = "## t3-workflow link"
BRIEF_TOOLS = 8
REAL_HOME = os.path.expanduser("~")


def skill_names():
    return sorted(p.parent.name for p in SKILLS.glob("*/SKILL.md"))


def load(skill, kind):
    return json.loads((HERE / skill / f"{kind}.json").read_text())


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Sandbox:
    """One throwaway machine for one run."""

    def __init__(self, root, skills=(), t3=True, setup=None):
        self.root = pathlib.Path(root)
        self.setup = setup or {}
        self.home = self.root / "home"
        self.repo = self.root / "repo"
        self.bin = self.root / "bin"
        self.fake_t3 = self.root / "fake-t3.json"
        self.t3_log = self.root / "t3-calls.jsonl"
        self.gh_log = self.root / "gh.log"
        self.t3 = t3
        for d in (self.home / ".claude" / "skills", self.bin, self.root / "config", self.root / "state",
                  self.root / "t3home"):
            d.mkdir(parents=True, exist_ok=True)
        self._settings()
        self._bin()
        self._repo()
        self._t3()
        for name in skills:
            shutil.copytree(SKILLS / name, self.home / ".claude" / "skills" / name)
        for name, spec in self.setup.get("home_skills", {}).items():
            self._home_skill(name, spec)
        if "config" in self.setup:
            path = self.root / "config" / "t3-workflow" / "config.json"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(self.setup["config"]))
        self.link_block = self._link() if self.setup.get("link") else ""

    def _settings(self):
        guard = f"{shlex.quote(sys.executable)} {shlex.quote(str(HERE / 'guard_hook.py'))} {shlex.quote(REAL_HOME)}"
        settings = {"permissions": {"allow": ALLOW, "deny": DENY}, "enableAllProjectMcpServers": True,
                    "hooks": {"PreToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": guard}]}]}}
        (self.home / ".claude" / "settings.json").write_text(json.dumps(settings))
        if self.t3:
            (self.home / ".claude" / "CLAUDE.md").write_text(RUNTIME_NOTE)

    def _bin(self):
        (self.bin / "python3").symlink_to(sys.executable)
        if shutil.which("claude"):
            (self.bin / "claude").symlink_to(shutil.which("claude"))
        gh = self.bin / "gh"
        gh.write_text(f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(HERE / 'fake_gh.py'))} \"$@\"\n")
        sysctl = self.bin / "systemctl"
        sysctl.write_text("#!/bin/sh\nexit 1\n")
        for p in (gh, sysctl):
            p.chmod(0o755)
        fixtures = {"issues": self.setup.get("issues", {})}
        (self.root / "gh.json").write_text(json.dumps(fixtures))

    def _git(self, *args, cwd=None):
        subprocess.run(["git", *args], cwd=cwd or self.repo, check=True, capture_output=True,
                       env={**self.environ(), "GIT_CONFIG_NOSYSTEM": "1"})

    def _repo(self):
        remote = self.root / "github.com" / "acme" / "widgets.git"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(remote)], check=True,
                       capture_output=True, env={**self.environ(), "GIT_CONFIG_NOSYSTEM": "1"})
        self.repo.mkdir()
        self._git("init", "-q", "-b", "main")
        self._git("config", "user.name", "Eval User")
        self._git("config", "user.email", "eval-user@example.invalid")
        self._git("remote", "add", "origin", REPO_URL)
        self._git("config", f"url.{remote}.insteadOf", REPO_URL)
        files = {"README.md": "# widgets\n\nA small widget library.\n",
                 "widgets.py": "def widget(name):\n    return f\"widget {name}\"\n",
                 **self.setup.get("repo_files", {})}
        for rel, text in files.items():
            path = self.repo / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self._git("add", *files)
        self._git("commit", "-q", "-m", "chore: initial commit")
        self._git("push", "-q", "origin", "main")
        if self.setup.get("branch"):
            self._git("switch", "-q", "-c", self.setup["branch"])
        if self.t3:
            server = {"command": sys.executable, "args": [str(HERE / "stub_t3_mcp.py")],
                      "env": {"T3_STUB_LOG": str(self.t3_log), "T3_STUB_FIXTURE": str(self.root / "t3.json")}}
            (self.repo / ".mcp.json").write_text(json.dumps({"mcpServers": {"t3-code": server}}))
            (self.repo / ".git" / "info" / "exclude").write_text(".mcp.json\n")
        (self.root / "repo-base.json").write_text(json.dumps(repo_snapshot(self.repo)))

    def _t3(self):
        replies = {"t3_worktree_status": {"attached": False, "projectWorkspaceRoot": str(self.repo),
                                          "branch": "main", "defaultStartFromOrigin": True},
                   "t3_project_list": {"projects": [{"projectId": "proj-widgets", "title": "widgets",
                                                     "workspaceRoot": str(self.repo)}]},
                   "t3_thread_launch": {"threadId": "thread-launched", "link": "t3://thread-launched"}}
        replies.update(self.setup.get("t3", {}))
        text = json.dumps(replies).replace("{repo}", str(self.repo))
        (self.root / "t3.json").write_text(text)
        threads = {t: {"threadId": t, "status": "idle", "updatedAt": T0, "link": f"t3://{t}"}
                   for t in (SELF, PARENT)}
        self.fake_t3.write_text(json.dumps({"threads": threads}))

    def _home_skill(self, name, spec):
        d = self.home / ".claude" / "skills" / name
        d.mkdir(parents=True)
        extra = "disable-model-invocation: true\n" if spec.get("inline") else ""
        (d / "SKILL.md").write_text(f"---\nname: {name}\ndescription: {spec['description']}\n{extra}---\n\n"
                                    f"{spec['body']}\n")

    def _link(self):
        env = self.environ()
        run = lambda *a: subprocess.run([sys.executable, str(T3WF), *a, "--self", PARENT], env=env,
                                        cwd=self.root, check=True, capture_output=True, text=True).stdout
        run("init", "--role", "ticket")
        printed = run("open-child", "--role", "worker", "--label", self.setup["link"])
        return printed[printed.index(LINK_HEADING):].split("\n\n", 1)[0].strip()

    def environ(self):
        keep = {k: os.environ[k] for k in ("LANG", "LC_ALL", "TERM", *AUTH_VARS) if k in os.environ}
        return {**keep, "HOME": str(self.home), "PATH": f"{self.bin}:/usr/bin:/bin",
                "XDG_CONFIG_HOME": str(self.root / "config"), "XDG_STATE_HOME": str(self.root / "state"),
                "T3CODE_HOME": str(self.root / "t3home"), "T3CODE_THREAD_ID": SELF,
                "T3WF_FAKE_T3": str(self.fake_t3), "T3WF_NO_LAZY": "1",
                "T3WF_SYSTEMCTL": str(self.bin / "systemctl"), "T3CM_PORT": str(free_port()),
                "GH_FAKE_DIR": str(self.root), "GIT_TERMINAL_PROMPT": "0", "GIT_ALLOW_PROTOCOL": "file"}

    def prompt(self, text):
        return text.replace("{link_block}", self.link_block).replace("{repo}", str(self.repo))


def claude_bin():
    return shutil.which("claude") or sys.exit("live evals need the claude CLI on PATH")


def require_auth():
    if not any(os.environ.get(k) for k in AUTH_VARS):
        sys.exit("live evals need ANTHROPIC_API_KEY or CLAUDE_CODE_OAUTH_TOKEN; see tests/skills/evals/README.md")


def skill_creator_dir(arg):
    path = arg or os.environ.get("SKILL_CREATOR_DIR")
    if not path or not (pathlib.Path(path) / "scripts" / "run_eval.py").is_file():
        sys.exit("pass --skill-creator <dir> or set SKILL_CREATOR_DIR to a skill-creator skill folder")
    return pathlib.Path(path).resolve()


def skill_called(use, clean_name):
    if use["name"] == "Skill":
        return clean_name in use["input"].get("skill", "")
    return use["name"] == "Read" and clean_name in use["input"].get("file_path", "")


def run_until_skill(query, skill_name, description, timeout, project, model, max_tools=BRIEF_TOOLS):
    """Like run_single_query, but a launch brief may call other tools first: its link block makes the
    thread invoke t3-workflow before its role skill. Triggered when the skill is called within the
    first max_tools tool calls."""
    clean_name = f"{skill_name}-skill-{uuid.uuid4().hex[:8]}"
    commands = pathlib.Path(project) / ".claude" / "commands"
    commands.mkdir(parents=True, exist_ok=True)
    command = commands / f"{clean_name}.md"
    indented = "\n  ".join(description.split("\n"))
    command.write_text(f"---\ndescription: |\n  {indented}\n---\n\n# {skill_name}\n\n"
                       f"This skill handles: {description}\n")
    proc = subprocess.Popen(["claude", "-p", query, "--output-format", "stream-json", "--verbose", "--model", model,
                             "--max-turns", str(max_tools + 2)], cwd=project, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    timer, seen = threading.Timer(timeout, proc.kill), 0
    timer.start()
    try:
        for line in proc.stdout:
            facts = transcript_facts([line])
            for use in facts["uses"]:
                if skill_called(use, clean_name):
                    return True
                seen += 1
                if seen >= max_tools:
                    return False
            if '"type":"result"' in line.replace(" ", ""):
                return False if facts["completed"] else None
        return None
    finally:
        timer.cancel()
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        command.unlink(missing_ok=True)


def preflight(root, model):
    """One plain run in a sandbox first: an auth or CLI failure would otherwise read as every query not firing."""
    box = Sandbox(root, t3=False)
    r = subprocess.run([str(box.bin / "claude"), "-p", "Reply with the word ready.", "--model", model,
                        "--output-format", "stream-json", "--verbose"], cwd=box.repo, env=box.environ(),
                       stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=120)
    if not transcript_facts(r.stdout.splitlines())["completed"]:
        sys.exit(f"claude did not complete a plain run in the sandbox: {r.stderr.strip()[-500:]}")


def one_trigger(job):
    """One query run in its own sandbox, through skill-creator's run_single_query."""
    from scripts.run_eval import run_single_query
    fd = os.open(os.devnull, os.O_RDONLY)
    os.dup2(fd, 0)
    os.environ.clear()
    os.environ.update(job["env"])
    detect = run_until_skill if LINK_HEADING in job["query"] else run_single_query
    start = time.time()
    try:
        fired = detect(job["query"], job["name"], job["description"], job["timeout"], job["project"], job["model"])
        return None if fired is False and time.time() - start >= job["timeout"] else fired
    except Exception as e:
        print(f"query failed: {e}", file=sys.stderr)
        return None


def run_triggers(args):
    require_auth()
    if args.description and len(args.skill or []) != 1:
        sys.exit("--description needs exactly one --skill")
    sys.path.insert(0, str(skill_creator_dir(args.skill_creator)))
    from scripts.utils import parse_skill_md
    out = pathlib.Path(args.out or tempfile.mkdtemp(prefix="t3-trigger-evals-")).resolve()
    report, start = {"model": args.model, "runs_per_query": args.runs, "skills": {}}, time.time()
    claude_bin()
    jobs = []
    for skill in args.skill or skill_names():
        name, description, _ = parse_skill_md(SKILLS / skill)
        if args.description:
            description = args.description
        report["skills"][skill] = {"description": description, "results": []}
        for i, item in enumerate(load(skill, "triggers")):
            for run in range(args.runs):
                box = Sandbox(out / skill / f"q{i:02d}-r{run}", skills=[s for s in skill_names() if s != skill],
                              t3=item.get("env", "t3") == "t3")
                env = box.environ()
                jobs.append({"skill": skill, "index": i, "item": item, "query": item["query"], "name": name,
                             "description": description, "timeout": args.timeout, "project": str(box.repo),
                             "model": args.model, "env": env})
    preflight(out / "preflight", args.model)
    ctx = multiprocessing.get_context("fork")
    with concurrent.futures.ProcessPoolExecutor(max_workers=args.workers, mp_context=ctx) as pool:
        fired = list(pool.map(one_trigger, jobs))
    for skill, entry in report["skills"].items():
        mine = [(j, f) for j, f in zip(jobs, fired) if j["skill"] == skill]
        for i in sorted({j["index"] for j, _ in mine}):
            runs = [f for j, f in mine if j["index"] == i]
            item = next(j["item"] for j, _ in mine if j["index"] == i)
            errors = runs.count(None)
            rate = sum(bool(f) for f in runs) / len(runs)
            ok = not errors and (rate >= 0.5 if item["should_trigger"] else rate < 0.5)
            entry["results"].append({"query": item["query"], "should_trigger": item["should_trigger"],
                                     "form": item.get("form"), "env": item.get("env", "t3"),
                                     "trigger_rate": rate, "triggers": sum(bool(f) for f in runs),
                                     "runs": len(runs), "errors": errors, "pass": ok})
        entry["passed"] = sum(r["pass"] for r in entry["results"])
        entry["total"] = len(entry["results"])
        print(f"{skill}: {entry['passed']}/{entry['total']} passed", file=sys.stderr)
        for r in entry["results"]:
            if not r["pass"]:
                print(f"  FAIL rate={r['triggers']}/{r['runs']} expected={r['should_trigger']}: "
                      f"{r['query'][:90]!r}", file=sys.stderr)
    report["seconds"] = round(time.time() - start)
    report["claude_runs"] = len(jobs)
    (out / "triggers.json").write_text(json.dumps(report, indent=1))
    summary = {k: f"{v['passed']}/{v['total']}" for k, v in report["skills"].items()}
    print(json.dumps({**summary, "claude_runs": len(jobs), "seconds": report["seconds"], "out": str(out)}, indent=1))
    return 0 if all(v["passed"] == v["total"] for v in report["skills"].values()) else 1


def transcript_facts(lines):
    uses, text, cost, turns, completed = [], "", None, None, False
    for line in lines:
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if e.get("type") == "assistant":
            for c in e.get("message", {}).get("content", []):
                if c.get("type") == "tool_use":
                    uses.append({"name": c["name"], "input": c.get("input", {})})
        elif e.get("type") == "result":
            text, cost, turns = e.get("result") or "", e.get("total_cost_usd"), e.get("num_turns")
            completed = e.get("subtype") == "error_max_turns" or not e.get("is_error")
    return {"uses": uses, "text": text, "cost": cost, "turns": turns, "completed": completed}


def repo_snapshot(repo):
    snap = {}
    for path in sorted(pathlib.Path(repo).rglob("*")):
        rel = path.relative_to(repo).as_posix()
        if rel == ".git" or rel.startswith(".git/") or (path.is_dir() and not path.is_symlink()):
            continue
        if path.is_symlink():
            data = b"link:" + os.readlink(path).encode()
        else:
            data = path.read_bytes() if path.is_file() else b"special"
        mode = oct(path.lstat().st_mode & 0o111)
        snap[rel] = f"{mode}:{hashlib.sha256(data).hexdigest()}"
    return snap


def repo_changes(box_root):
    base_file = box_root / "repo-base.json"
    if not base_file.exists():
        raise FileNotFoundError(f"{base_file} is missing, so the repo check cannot tell what changed")
    base = json.loads(base_file.read_text())
    now = repo_snapshot(box_root / "repo")
    return sorted(p for p in base.keys() | now.keys() if base.get(p) != now.get(p))


def read_jsonl(path):
    p = pathlib.Path(path)
    return [json.loads(line) for line in p.read_text().splitlines() if line.strip()] if p.exists() else []


def matches(pattern, value):
    return re.search(pattern, value if isinstance(value, str) else json.dumps(value), re.I | re.S) is not None


def count_hits(check, facts, box_root):
    kind = next(k for k in CHECK_KINDS if k in check)
    want = check.get("input")
    if kind == "tool":
        name = check["tool"]
        named = [u for u in facts["uses"] if name in ("*", u["name"]) or u["name"].endswith("__" + name)]
        return sum(1 for u in named if want is None or matches(want, u["input"]))
    if kind == "bash":
        return sum(1 for u in facts["uses"] if u["name"] == "Bash"
                   and matches(check["bash"], u["input"].get("command", "")))
    if kind == "text":
        return int(matches(check["text"], facts["text"]))
    if kind == "gh":
        return sum(1 for c in read_jsonl(box_root / "gh.log")
                   if matches(check["gh"], " ".join(c["argv"])) and (want is None or matches(want, c.get("body") or "")))
    if kind == "t3":
        return sum(1 for c in read_jsonl(box_root / "t3-calls.jsonl")
                   if c["tool"] == check["t3"] and (want is None or matches(want, c["input"])))
    if kind == "repo":
        return sum(1 for p in repo_changes(box_root) if matches(check["repo"], p))
    path = box_root / "repo" / check["file"]
    if "contains" not in check:
        return int(path.exists())
    return int(path.exists() and matches(check["contains"], path.read_text()))


def check_passes(check, facts, box_root):
    hits = count_hits(check, facts, box_root)
    return hits == 0 if check.get("none") else hits > 0


def behave(ev, skill, mode, out, args):
    root = out / skill / ev["name"] / mode
    t = time.time()
    if args.rescore:
        stream = (root / "transcript.jsonl").read_text()
    else:
        box = Sandbox(root, skills=skill_names() if mode == "with_skill" else [], setup=ev.get("setup"))
        cmd = [str(box.bin / "claude"), "-p", box.prompt(ev["prompt"]), "--model", args.model, "--output-format",
               "stream-json", "--verbose", "--max-turns", str(ev.get("max_turns", args.max_turns))]
        try:
            stream = subprocess.run(cmd, cwd=box.repo, env=box.environ(), stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=ev.get("timeout", args.timeout)).stdout
        except subprocess.TimeoutExpired as e:
            stream = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
        (root / "transcript.jsonl").write_text(stream)
    facts = transcript_facts(stream.splitlines())
    return {"checks": [check_passes(c, facts, root) for c in ev["checks"]], "cost_usd": facts["cost"],
            "turns": facts["turns"], "seconds": None if args.rescore else round(time.time() - t),
            "completed": facts["completed"], "blocked": blocked_calls(facts), "sandbox": str(root)}


def blocked_calls(facts):
    return [u for u in facts["uses"] if guard_hook.blocked(u["input"], REAL_HOME)]


def run_behavior(args):
    if args.rescore:
        out = pathlib.Path(args.rescore).resolve()
    else:
        require_auth()
        claude_bin()
        out = pathlib.Path(args.out or tempfile.mkdtemp(prefix="t3-behavior-evals-")).resolve()
    report, start = {"model": args.model, "evals": []}, time.time()
    for skill in args.skill or [s for s in skill_names() if (HERE / s / "evals.json").exists()]:
        for ev in load(skill, "evals")["evals"]:
            if args.eval and ev["id"] not in args.eval:
                continue
            runs = {mode: behave(ev, skill, mode, out, args) for mode in ("with_skill", "without_skill")}
            w, wo = runs["with_skill"]["checks"], runs["without_skill"]["checks"]
            valid = all(r["completed"] for r in runs.values())
            row = {"skill": skill, "id": ev["id"], "name": ev["name"], "runs": runs, "valid": valid,
                   "pass": valid and all(w) and not all(wo),
                   "expectations": [{"text": x, "with_skill": a, "without_skill": b}
                                    for x, a, b in zip(ev["expectations"], w, wo)]}
            report["evals"].append(row)
            print(f"{skill}/{ev['name']}: {'PASS' if row['pass'] else 'FAIL'}{'' if valid else ' (invalid run)'} "
                  f"with {sum(w)}/{len(w)}, without {sum(wo)}/{len(wo)}", file=sys.stderr)
    report["seconds"] = sum(r["runs"][m]["seconds"] or 0 for r in report["evals"] for m in r["runs"]) \
        if args.rescore else round(time.time() - start)
    report["cost_usd"] = round(sum((r["runs"][m]["cost_usd"] or 0) for r in report["evals"] for m in r["runs"]), 4)
    (out / ("rescore.json" if args.rescore else "behavior.json")).write_text(json.dumps(report, indent=1))
    print(json.dumps({"passed": sum(r["pass"] for r in report["evals"]), "total": len(report["evals"]),
                      "cost_usd": report["cost_usd"], "seconds": report["seconds"], "out": str(out)}, indent=1))
    return 0 if all(r["pass"] for r in report["evals"]) else 1


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("triggers", "behavior"):
        s = sub.add_parser(name)
        s.add_argument("--skill", action="append", choices=skill_names())
        s.add_argument("--model", default=DEFAULT_MODEL)
        s.add_argument("--out")
    t = sub.choices["triggers"]
    t.add_argument("--runs", type=int, default=3)
    t.add_argument("--workers", type=int, default=6)
    t.add_argument("--timeout", type=int, default=90)
    t.add_argument("--description", help="test this description instead of the SKILL.md one")
    t.add_argument("--skill-creator")
    b = sub.choices["behavior"]
    b.add_argument("--eval", action="append", type=int)
    b.add_argument("--max-turns", type=int, default=40)
    b.add_argument("--timeout", type=int, default=900)
    b.add_argument("--rescore", metavar="OUT", help="score the transcripts a past run left in OUT again, no model calls")
    args = p.parse_args(argv)
    return run_triggers(args) if args.cmd == "triggers" else run_behavior(args)


if __name__ == "__main__":
    sys.exit(main())
