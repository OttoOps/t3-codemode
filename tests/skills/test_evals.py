import contextlib
import io
import json
import os
import pathlib
import re
import shlex
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

EVALS = pathlib.Path(__file__).resolve().parent / "evals"
sys.path.insert(0, str(EVALS))
import live  # noqa: E402

ROLES_AND_SETUP = ["t3-overseer", "t3-ticket", "t3-worker", "setup-t3-workflow"]
BRIEF_SKILLS = ["t3-ticket", "t3-worker"]
TRIGGER_KEYS = {"query", "should_trigger", "form", "env"}
CHECK_EXTRAS = {"input", "none", "contains"}
SETUP_KEYS = {"issues", "t3", "t3cm", "config", "home_skills", "repo_files", "link", "branch", "origin", "forges"}


class EvalSets(unittest.TestCase):
    def test_every_skill_has_a_trigger_set_and_every_role_a_behavior_set(self):
        self.assertEqual(sorted(p.name for p in EVALS.iterdir() if p.is_dir() and p.name != "__pycache__"),
                         live.skill_names())
        for skill in live.skill_names():
            self.assertTrue((EVALS / skill / "triggers.json").is_file(), skill)
        for skill in ROLES_AND_SETUP:
            self.assertTrue((EVALS / skill / "evals.json").is_file(), skill)

    def test_trigger_sets_have_the_skill_creator_shape(self):
        for skill in live.skill_names():
            items = live.load(skill, "triggers")
            self.assertIsInstance(items, list)
            for item in items:
                self.assertLessEqual(set(item), TRIGGER_KEYS, (skill, item))
                self.assertIsInstance(item["query"], str)
                self.assertTrue(item["query"].strip(), skill)
                self.assertIsInstance(item["should_trigger"], bool)
                self.assertIn(item.get("env", "t3"), ("t3", "plain"))
                if item["should_trigger"]:
                    self.assertIn(item.get("form"), ("plain", "brief"), (skill, item["query"][:60]))
            queries = [(i["query"], i.get("env", "t3")) for i in items]
            self.assertEqual(len(queries), len(set(queries)), f"{skill} repeats a query")

    def test_every_trigger_set_has_both_kinds_and_a_plain_language_trigger(self):
        for skill in live.skill_names():
            items = live.load(skill, "triggers")
            self.assertTrue(any(i["should_trigger"] for i in items), skill)
            self.assertTrue(any(not i["should_trigger"] for i in items), skill)
            self.assertTrue(any(i["should_trigger"] and i["form"] == "plain" for i in items), skill)

    def test_no_prompt_starts_with_a_slash(self):
        for skill in live.skill_names():
            for item in live.load(skill, "triggers"):
                self.assertFalse(item["query"].lstrip().startswith("/"), (skill, item["query"][:60]))
        for skill in ROLES_AND_SETUP:
            for ev in live.load(skill, "evals")["evals"]:
                self.assertFalse(ev["prompt"].lstrip().startswith("/"), (skill, ev["name"]))

    def test_ticket_and_worker_trigger_from_a_launch_brief(self):
        for skill in BRIEF_SKILLS:
            briefs = [i for i in live.load(skill, "triggers") if i["should_trigger"] and i["form"] == "brief"]
            self.assertTrue(briefs, skill)
            self.assertTrue(any("## t3-codemode link" in i["query"] for i in briefs), skill)
            self.assertTrue(any(f"the {skill} skill" not in i["query"] for i in briefs),
                            f"{skill} needs a brief that does not name the skill")

    def test_near_misses_cover_the_other_roles_and_sessions_outside_t3(self):
        ticket = live.load("t3-ticket", "triggers")
        self.assertTrue(any(i.get("env") == "plain" and not i["should_trigger"] for i in ticket))
        negatives = " ".join(i["query"] for i in ticket if not i["should_trigger"])
        for word in ("status", "label", "comment", "in parallel", "## Procedure"):
            self.assertIn(word, negatives)

    def test_behavior_sets_have_the_skill_creator_shape_and_checks(self):
        for skill in ROLES_AND_SETUP:
            spec = live.load(skill, "evals")
            self.assertEqual(spec["skill_name"], skill)
            ids = [e["id"] for e in spec["evals"]]
            self.assertTrue(ids, skill)
            self.assertEqual(len(ids), len(set(ids)), skill)
            for ev in spec["evals"]:
                self.assertIsInstance(ev["id"], int)
                self.assertRegex(ev["name"], r"^[a-z0-9-]+$")
                self.assertTrue(ev["prompt"].strip() and ev["expected_output"].strip())
                self.assertEqual(ev["files"], [])
                self.assertLessEqual(set(ev["setup"]), SETUP_KEYS, ev["name"])
                self.assertEqual(len(ev["expectations"]), len(ev["checks"]), ev["name"])
                for text, check in zip(ev["expectations"], ev["checks"]):
                    self.assertIsInstance(text, str)
                    kinds = [k for k in live.CHECK_KINDS if k in check]
                    self.assertEqual(len(kinds), 1, check)
                    self.assertLessEqual(set(check) - {kinds[0]}, CHECK_EXTRAS, check)
                    for key in ("input", "contains", *(k for k in ("bash", "text", "gh", "glab", "repo") if k in check)):
                        if key in check:
                            re.compile(check[key])
                if "{link_block}" in ev["prompt"]:
                    self.assertIn("link", ev["setup"], ev["name"])
                for rule in ev["setup"].get("t3cm", []):
                    self.assertEqual(len(set(rule) & {"result", "results", "error"}), 1, rule)
                    self.assertLessEqual(set(rule), {"match", "result", "results", "error"}, rule)
                    re.compile(rule["match"])

    def test_a_pool_slot_is_covered_for_workers_tickets_and_setup(self):
        for skill, role in (("t3-ticket", "worker"), ("t3-overseer", "ticket")):
            evs = [e for e in live.load(skill, "evals")["evals"]
                   if "pool" in e["setup"].get("config", {}).get("models", {}).get(role, {})]
            with_t3cm = [e for e in evs if "t3cm" in e["setup"]]
            self.assertTrue(any(c.get("t3") == "t3_thread_launch" and "claude-b" in c.get("input", "")
                                for e in with_t3cm for c in e["checks"]), skill)
            self.assertTrue(any(c.get("tool") == "execute" and "wf\\.pick" in c.get("input", "")
                                for e in with_t3cm for c in e["checks"]), skill)
        self.assertTrue([e for e in live.load("t3-ticket", "evals")["evals"]
                         if "pool" in e["setup"].get("config", {}).get("models", {}).get("worker", {})
                         and "t3cm" not in e["setup"]], "no eval launches a pool slot through t3cm exec -")
        setup = live.load("setup-t3-workflow", "evals")["evals"]
        self.assertTrue(any(c.get("tool") == "execute" and "wf\\.pools" in c.get("input", "")
                            for e in setup for c in e["checks"]))
        worker = live.load("t3-worker", "evals")["evals"]
        self.assertTrue(any(c.get("tool") == "*" and "wf\\.renewal" in c.get("input", "")
                            for e in worker for c in e["checks"]), "no worker eval exercises wf.renewal")

    def test_a_tracked_gitlab_host_is_covered_for_tickets_and_workers(self):
        for skill in BRIEF_SKILLS:
            evs = [e for e in live.load(skill, "evals")["evals"]
                   if "://gitlab." in e["setup"].get("origin", "")
                   and e["setup"]["origin"].split("/")[2] in e["setup"].get("forges", {})]
            self.assertTrue(evs, f"{skill} has no eval on a GitLab host listed under forges")
            self.assertTrue(any("glab" in c for e in evs for c in e["checks"]), skill)
        worker = [e for e in live.load("t3-worker", "evals")["evals"] if "forges" in e["setup"]]
        self.assertTrue(any(c.get("tool") == "*" and "wf\\.pr" in c.get("input", "") for e in worker for c in e["checks"]))

    def test_the_worker_runs_a_slash_only_pass(self):
        evs = [e for e in live.load("t3-worker", "evals")["evals"]
               if any(s.get("inline") and n in e["setup"].get("config", {}).get("passes", [])
                      for n, s in e["setup"].get("home_skills", {}).items())]
        self.assertTrue(evs, "no worker eval configures a slash-only pass")
        checks = evs[0]["checks"]
        self.assertIn({"tool": "Skill", "input": "unslop", "none": True}, checks)
        read = next(c["input"] for c in checks if c.get("tool") == "*" and "unslop" in c.get("input", ""))
        for given, reads in [({"file_path": "/box/skills/unslop/SKILL.md"}, True),
                             ({"command": 'git commit -m "x"\ncat /box/skills/unslop/SKILL.md'}, True),
                             ({"command": "sed -n 1,40p /box/skills/unslop/SKILL.md"}, True),
                             ({"command": "ls /box/skills/unslop/SKILL.md"}, False)]:
            self.assertEqual(live.matches(read, given), reads, given)


class SandboxHarness(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = pathlib.Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def box(self, **kw):
        return live.Sandbox(self.tmp / "box", **kw)

    def run_in(self, box, *cmd, stdin=None):
        return subprocess.run(list(cmd), cwd=box.repo, env=box.environ(), input=stdin, text=True,
                              capture_output=True, timeout=60)

    def test_the_environment_stays_inside_the_sandbox(self):
        box = self.box()
        env = box.environ()
        for key in ("HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "T3CODE_HOME", "GH_FAKE_DIR"):
            self.assertTrue(env[key].startswith(str(box.root)), key)
        self.assertTrue(env["PATH"].startswith(str(box.bin) + ":"))
        self.assertEqual(env["PATH"], f"{box.bin}:/usr/bin:/bin")
        for tool in box.bin.iterdir():
            self.assertIn(tool.name, ("python3", "claude", "gh", "glab", "t3cm", "systemctl"))
        self.assertEqual(env["T3CODE_THREAD_ID"], live.SELF)
        self.assertEqual(env["GIT_ALLOW_PROTOCOL"], "file")
        self.assertFalse({"CLAUDE_CONFIG_DIR", "CODEX_HOME", "GH_TOKEN", "GITHUB_TOKEN"} & set(env))
        self.assertEqual(live.repo_changes(box.root), [], "a fresh sandbox starts with a clean repo")

    def test_with_skill_installs_every_skill_and_without_installs_none(self):
        with_box = live.Sandbox(self.tmp / "with", skills=live.skill_names())
        without = live.Sandbox(self.tmp / "without")
        installed = sorted(p.parent.name for p in (with_box.home / ".claude" / "skills").glob("*/SKILL.md"))
        self.assertEqual(installed, live.skill_names())
        self.assertEqual(list((without.home / ".claude" / "skills").iterdir()), [])
        settings = json.loads((without.home / ".claude" / "settings.json").read_text())
        self.assertIn("mcp__t3-code", settings["permissions"]["allow"])
        self.assertIn("WebFetch", settings["permissions"]["deny"])
        hook = settings["hooks"]["PreToolUse"][0]
        self.assertEqual(hook["matcher"], "*")
        self.assertIn("guard_hook.py", hook["hooks"][0]["command"])

    def test_gh_answers_from_fixtures_and_logs(self):
        box = self.box(setup={"issues": {"42": {"number": 42, "title": "Quiet flag"}}})
        r = self.run_in(box, "gh", "issue", "view", "42", "-R", "acme/widgets", "--json", "title", "-q", ".title")
        self.assertEqual(r.stdout.strip(), "Quiet flag")
        self.assertNotEqual(self.run_in(box, "gh", "issue", "view", "7").returncode, 0)
        body = box.root / "body.md"
        body.write_text("Refs #42\n")
        r = self.run_in(box, "gh", "pr", "create", "--base", "main", "--title", "feat: x", "--body-file", str(body))
        self.assertIn("/pull/", r.stdout)
        log = live.read_jsonl(box.gh_log)
        self.assertEqual(log[-1]["body"], "Refs #42\n")
        self.assertEqual(self.run_in(box, "gh", "api", "user", "-q", ".login").stdout.strip(), "eval-user")

    def test_glab_answers_from_fixtures_and_logs(self):
        box = self.box(setup={"issues": {"42": {"iid": 42, "title": "Quiet flag"}}})
        r = self.run_in(box, "glab", "issue", "view", "42", "--output", "json")
        self.assertEqual(json.loads(r.stdout)["title"], "Quiet flag")
        self.assertNotEqual(self.run_in(box, "glab", "issue", "view", "7").returncode, 0)
        r = self.run_in(box, "glab", "mr", "create", "--source-branch", "feat/x", "--target-branch", "main",
                        "--title", "feat: x", "--description", "Refs #42\n", "--yes")
        self.assertIn("/-/merge_requests/", r.stdout)
        self.assertEqual(live.read_jsonl(box.glab_log)[-1]["body"], "Refs #42\n")
        r = self.run_in(box, "glab", "mr", "list", "--source-branch", "feat/x", "--all", "--output", "json")
        self.assertEqual(json.loads(r.stdout), [])
        r = self.run_in(box, "glab", "api", "--hostname", "gitlab.acme.example", "projects/acme%2Fwidgets/merge_requests/7")
        self.assertEqual(json.loads(r.stdout)["state"], "opened")
        self.assertEqual(self.run_in(box, "glab", "api", "user", "-q", ".username").stdout.strip(), "eval-user")
        self.assertEqual(live.read_jsonl(box.gh_log), [], "glab calls never reach the fake gh")

    def test_a_gitlab_origin_pushes_to_its_own_bare_repo_and_lists_its_forge(self):
        origin = "https://gitlab.acme.example/acme/widgets.git"
        forges = {"gitlab.acme.example": {"kind": "gitlab", "cli": "glab"}}
        box = self.box(setup={"origin": origin, "forges": forges, "branch": "feat/x"})
        self.assertEqual(self.run_in(box, "git", "config", "remote.origin.url").stdout.strip(), origin)
        self.assertEqual(self.run_in(box, "git", "push", "-q", "origin", "feat/x").returncode, 0)
        bare = box.root / "gitlab.acme.example" / "acme" / "widgets.git"
        heads = subprocess.run(["git", "--git-dir", str(bare), "branch"], capture_output=True, text=True).stdout
        self.assertIn("feat/x", heads)
        r = self.run_in(box, "t3cm", "config", "get", "forges")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout), forges)
        plain = live.Sandbox(self.tmp / "plain")
        self.assertEqual(json.loads(self.run_in(plain, "t3cm", "config", "get", "forges").stdout), {})

    def test_pushes_land_in_the_local_bare_repo(self):
        box = self.box(setup={"branch": "feat/x"})
        (box.repo / "x.txt").write_text("x\n")
        self.run_in(box, "git", "add", "x.txt")
        self.run_in(box, "git", "commit", "-qm", "feat: x")
        r = self.run_in(box, "git", "push", "-q", "origin", "feat/x")
        self.assertEqual(r.returncode, 0, r.stderr)
        bare = box.root / "github.com" / "acme" / "widgets.git"
        heads = subprocess.run(["git", "--git-dir", str(bare), "branch"], capture_output=True, text=True).stdout
        self.assertIn("feat/x", heads)
        self.assertEqual(self.run_in(box, "git", "remote", "get-url", "origin").stdout.strip(), str(bare))
        self.assertEqual(self.run_in(box, "git", "config", "remote.origin.url").stdout.strip(), live.REPO_URL)
        self.assertEqual(self.run_in(box, "git", "fetch", "-q", "origin").returncode, 0)
        r = self.run_in(box, "git", "ls-remote", "https://github.com/acme/other.git")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not allowed", r.stderr)

    def test_the_link_block_is_a_worker_join_line(self):
        box = self.box(setup={"link": "#42 PR 1/2"})
        self.assertTrue(box.link_block.startswith("## t3-codemode link"))
        self.assertIn(f'wf.join({{ parent: "{live.PARENT}", label: "#42 PR 1/2", role: "worker"', box.link_block)
        self.assertIn(box.link_block, box.prompt("brief\n\n{link_block}\n"))
        r = self.run_in(box, "t3cm", "exec", "-", stdin=box.link_block.splitlines()[-1])
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout)["result"]["thread"], live.SELF)

    def test_an_inline_pass_resolves_to_its_sandbox_skill_file(self):
        unslop = {"inline": True, "description": "Strip slop.", "body": "1. Delete robust."}
        box = self.box(setup={"config": {"version": 1, "passes": ["unslop"]}, "home_skills": {"unslop": unslop}})
        r = self.run_in(box, "t3cm", "exec", "-", stdin='async () => wf.slot("passes")')
        self.assertEqual(r.returncode, 0, r.stderr)
        path = box.home / ".claude" / "skills" / "unslop" / "SKILL.md"
        self.assertEqual(json.loads(r.stdout)["result"]["skills"][0]["how"], f"inline: read {path} and apply it")

    def test_the_stub_t3_server_lists_answers_and_logs(self):
        box = self.box(setup={"t3": {"t3_worktree_status": {"attached": True, "worktreePath": "{repo}"}}})
        calls = [{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}},
                 {"jsonrpc": "2.0", "method": "notifications/initialized"},
                 {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
                 {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                  "params": {"name": "t3_worktree_status", "arguments": {}}},
                 {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                  "params": {"name": "t3_thread_launch", "arguments": {"title": "x"}}}]
        server = json.loads((box.repo / ".mcp.json").read_text())["mcpServers"]["t3-code"]
        r = subprocess.run([server["command"], *server["args"]], input="\n".join(map(json.dumps, calls)) + "\n",
                           env={**os.environ, **server["env"]}, capture_output=True, text=True, timeout=30)
        out = [json.loads(line) for line in r.stdout.splitlines()]
        self.assertEqual([o["id"] for o in out], [1, 2, 3, 4])
        self.assertIn("t3_thread_launch", [t["name"] for t in out[1]["result"]["tools"]])
        status = json.loads(out[2]["result"]["content"][0]["text"])
        self.assertEqual(status, {"attached": True, "worktreePath": str(box.repo)})
        self.assertEqual([c["tool"] for c in live.read_jsonl(box.t3_log)], ["t3_worktree_status", "t3_thread_launch"])

    def test_the_stub_t3cm_server_answers_by_rule_and_logs(self):
        box = self.box(setup={"t3cm": [{"match": r"wf\.pick\(", "results": [{"instanceId": "claude-a"},
                                                                         {"instanceId": "claude-b"}]},
                                       {"match": r"wf\.pools\(", "error": "pool opus: T3 did not answer"}]})
        servers = json.loads((box.repo / ".mcp.json").read_text())["mcpServers"]
        self.assertEqual(sorted(servers), ["t3-code", "t3cm"])
        settings = json.loads((box.home / ".claude" / "settings.json").read_text())
        self.assertIn("mcp__t3cm", settings["permissions"]["allow"])
        code = lambda i, c: {"jsonrpc": "2.0", "id": i, "method": "tools/call",
                             "params": {"name": "execute", "arguments": {"code": c}}}
        calls = [{"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
                 code(2, 'async () => wf.pick({ pool: "opus" })'),
                 code(3, "async () => wf.pools()"),
                 code(4, 'async () => wf.openChild({ role: "worker", label: "x" })'),
                 code(5, "async () => 1"),
                 code(6, 'async () => wf.pick({ pool: "opus" })'),
                 code(7, 'async () => wf.pick({ pool: "opus" })')]
        server = servers["t3cm"]
        r = subprocess.run([server["command"], *server["args"]], input="\n".join(map(json.dumps, calls)) + "\n",
                           env={**os.environ, **server["env"]}, capture_output=True, text=True, timeout=30)
        out = [json.loads(line)["result"] for line in r.stdout.splitlines()]
        self.assertEqual(sorted(t["name"] for t in out[0]["tools"]), ["execute", "search"])
        text = lambda o: json.loads(o["content"][0]["text"])
        self.assertEqual([text(out[i])["result"]["instanceId"] for i in (1, 5, 6)],
                         ["claude-a", "claude-b", "claude-a"])
        self.assertTrue(out[2]["isError"])
        self.assertIn("t3-codemode link", text(out[3])["result"]["brief"])
        self.assertIsNone(text(out[4])["result"])
        self.assertEqual(len(live.read_jsonl(box.t3cm_log)), 6)

    def test_without_t3cm_the_sandbox_has_no_t3cm_server(self):
        box = self.box()
        self.assertEqual(list(json.loads((box.repo / ".mcp.json").read_text())["mcpServers"]), ["t3-code"])
        settings = json.loads((box.home / ".claude" / "settings.json").read_text())
        self.assertNotIn("mcp__t3cm", settings["permissions"]["allow"])


class Checks(unittest.TestCase):
    STREAM = [
        {"type": "system", "subtype": "init"},
        {"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "Bash", "input": {"command": "t3cm exec - <<'EOF'\nasync () => wf.init({ role: 'ticket' })\nEOF"}},
            {"type": "tool_use", "name": "mcp__t3-code__t3_worktree_status", "input": {}},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "/x/skills/unslop/SKILL.md"}}]}},
        {"type": "result", "result": "**ACTION NEEDED:** approve this PR split", "total_cost_usd": 0.5,
         "num_turns": 3},
    ]

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self._tmp.name)
        (self.root / "repo").mkdir()
        (self.root / "repo" / "out.md").write_text("Unslopped: 2 rules applied.\n")
        (self.root / "gh.log").write_text(json.dumps({"argv": ["pr", "create", "--title", "t"],
                                                      "body": "Unslopped: 2 rules applied."}) + "\n")
        (self.root / "glab.log").write_text(json.dumps({"argv": ["mr", "create", "--title", "t"],
                                                        "body": "Refs #42"}) + "\n")
        (self.root / "t3-calls.jsonl").write_text(json.dumps({"tool": "t3_worktree_status", "input": {}}) + "\n")
        self.facts = live.transcript_facts(json.dumps(e) for e in self.STREAM)

    def tearDown(self):
        self._tmp.cleanup()

    def passes(self, check):
        return live.check_passes(check, self.facts, self.root)

    def test_transcript_facts(self):
        self.assertEqual(len(self.facts["uses"]), 3)
        self.assertEqual((self.facts["cost"], self.facts["turns"]), (0.5, 3))

    def test_each_check_kind(self):
        self.assertTrue(self.passes({"bash": r"t3cm exec -.*wf\.init.*ticket"}))
        self.assertTrue(self.passes({"tool": "t3_worktree_status"}))
        self.assertTrue(self.passes({"tool": "*", "input": r"unslop/SKILL\.md"}))
        self.assertTrue(self.passes({"text": "approve this PR split"}))
        self.assertTrue(self.passes({"gh": "pr create", "input": "Unslopped"}))
        self.assertTrue(self.passes({"glab": "mr create", "input": "Refs #42"}))
        self.assertFalse(self.passes({"glab": "pr create"}))
        self.assertTrue(self.passes({"t3": "t3_worktree_status"}))
        self.assertTrue(self.passes({"file": "out.md", "contains": "Unslopped"}))
        self.assertFalse(self.passes({"gh": "pr create", "input": "robust"}))
        self.assertFalse(self.passes({"file": "missing.md", "contains": "x"}))
        self.assertTrue(self.passes({"file": "out.md"}))
        self.assertTrue(self.passes({"file": "missing.md", "none": True}))

    def test_the_repo_check_sees_files_changed_by_any_tool(self):
        repo = self.root / "repo"
        with self.assertRaises(FileNotFoundError, msg="no saved base must not pass as clean"):
            self.passes({"repo": r"\.py$", "none": True})
        (self.root / "repo-base.json").write_text(json.dumps(live.repo_snapshot(repo)))
        (repo / ".git").mkdir()
        (repo / ".git" / "index").write_text("x")
        self.assertTrue(self.passes({"repo": ".", "none": True}))
        (repo / "out.md").write_text("changed\n")
        self.assertFalse(self.passes({"repo": ".", "none": True}))
        self.assertTrue(self.passes({"repo": r"^out\.md$"}))
        (repo / "out.md").write_text("Unslopped: 2 rules applied.\n")
        (repo / "src").mkdir()
        (repo / "src" / "new.py").write_text("")
        self.assertEqual(live.repo_changes(self.root), ["src/new.py"])
        (repo / "src" / "new.py").unlink()
        (repo / "out.md").unlink()
        self.assertEqual(live.repo_changes(self.root), ["out.md"])
        (repo / "out.md").write_text("Unslopped: 2 rules applied.\n")
        self.assertEqual(live.repo_changes(self.root), [])
        (repo / "out.md").chmod(0o755)
        self.assertEqual(live.repo_changes(self.root), ["out.md"])


    def test_rescore_reads_saved_transcripts_without_a_model(self):
        ev = live.load("setup-t3-workflow", "evals")["evals"][0]
        with_skill = [{"type": "assistant", "message": {"content": [
                          {"type": "tool_use", "name": "Bash",
                           "input": {"command": "t3cm setup detect --skills a"}}]}},
                      {"type": "result", "result": "review: none\nmodel worker: default\nWhich reviewer?",
                       "total_cost_usd": 0.25, "num_turns": 4}]
        without = [{"type": "result", "result": "Which reviewer?", "total_cost_usd": 0.1, "num_turns": 2}]
        for mode, stream in (("with_skill", with_skill), ("without_skill", without)):
            root = self.root / "out" / "setup-t3-workflow" / ev["name"] / mode
            (root / "repo").mkdir(parents=True)
            (root / "repo" / "README.md").write_text("# widgets\n")
            (root / "repo-base.json").write_text(json.dumps(live.repo_snapshot(root / "repo")))
            (root / "transcript.jsonl").write_text("\n".join(map(json.dumps, stream)) + "\n")
        out = self.root / "out"
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = live.main(["behavior", "--skill", "setup-t3-workflow", "--eval", str(ev["id"]),
                              "--rescore", str(out)])
        self.assertEqual(code, 0)
        report = json.loads((out / "rescore.json").read_text())
        self.assertEqual(report["cost_usd"], 0.35)
        self.assertTrue(report["evals"][0]["pass"])
        (out / "setup-t3-workflow" / ev["name"] / "without_skill" / "transcript.jsonl").write_text("")
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = live.main(["behavior", "--skill", "setup-t3-workflow", "--eval", str(ev["id"]),
                              "--rescore", str(out)])
        self.assertEqual(code, 1, "a control run that never finished must not count as a difference")

    def test_the_guard_hook_refuses_the_real_home_local_servers_and_network_clients(self):
        hook = live.HERE / "guard_hook.py"
        real = os.path.join(live.REAL_HOME, ".claude", "settings.json")
        cases = [({"command": f"cat {real}"}, 2),
                 ({"command": "curl -s http://127.0.0.1:3773/mcp"}, 2),
                 ({"command": "wget https://example.com"}, 2),
                 ({"file_path": str(self.root / "home" / ".claude" / "x")}, 0),
                 ({"command": "git add x\ncat >> widgets.py <<'EOF'\nx\nEOF"}, 0),
                 ({"url": "https://github.com/acme/widgets/pull/101"}, 0)]
        for tool_input, code in cases:
            r = subprocess.run([sys.executable, str(hook), live.REAL_HOME], capture_output=True, text=True,
                               input=json.dumps({"tool_name": "Bash", "tool_input": tool_input}))
            self.assertEqual(r.returncode, code, tool_input)
        facts = {"uses": [{"name": "Bash", "input": cases[0][0]}, {"name": "Read", "input": cases[3][0]}]}
        self.assertEqual(live.blocked_calls(facts), facts["uses"][:1])

    def test_skill_creator_must_be_named(self):
        with mock.patch.dict(os.environ, {}, clear=True), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                live.skill_creator_dir(None)
        sc = self.root / "sc"
        (sc / "scripts").mkdir(parents=True)
        (sc / "scripts" / "run_eval.py").write_text("")
        self.assertEqual(live.skill_creator_dir(str(sc)), sc.resolve())

    def test_the_pool_checks_match_a_pick_and_its_launch(self):
        ev = next(e for e in live.load("t3-overseer", "evals")["evals"] if e["name"] == "tickets-from-a-pool-in-thread")
        pick = next(c for c in ev["checks"] if c.get("tool") == "execute" and "pick" in c["input"])
        launches = [c for c in ev["checks"] if c.get("t3") == "t3_thread_launch" and not c.get("none")]
        self.assertEqual(len(launches), 4)
        facts = {"uses": [{"name": "mcp__t3cm__execute", "input": {"code": 'async () => wf.pick({ pool: "opus" })'}}]}
        self.assertTrue(live.check_passes(pick, facts, self.root))

        def sel(inst, option="effort"):
            return {"instanceId": inst, "model": "claude-opus-5-5", "options": [{"id": option, "value": "high"}]}

        def launch(title, n, s):
            return {"tool": "t3_thread_launch", "input": {"title": f"widgets#{title}: x", "modelSelection": s,
                                                          "message": f"Work https://github.com/acme/widgets/issues/{n}"}}

        def logged(*calls):
            (self.root / "t3-calls.jsonl").write_text("".join(json.dumps(c) + "\n" for c in calls))
            return [live.check_passes(c, facts, self.root) for c in launches]

        self.assertEqual(logged(launch(42, 42, sel("claude-a")), launch(43, 43, sel("claude-b"))), [True] * 4)
        self.assertFalse(all(logged(launch(42, 42, sel("claude-a")), launch(43, 43, sel("claude-a")))),
                         "one pick reused for both tickets")
        self.assertFalse(all(logged(launch(42, 43, sel("claude-a")), launch(42, 42, sel("claude-b")))),
                         "a launch whose title and issue URL disagree")
        self.assertFalse(any(logged(launch(42, 42, sel("claude-a", "temperature")),
                                    launch(43, 43, {"instanceId": "claude-b"}))))

    def test_none_inverts_a_check(self):
        self.assertTrue(self.passes({"tool": "Edit", "none": True}))
        self.assertTrue(self.passes({"t3": "t3_thread_launch", "none": True}))
        self.assertFalse(self.passes({"tool": "Read", "none": True}))
        self.assertFalse(self.passes({"tool": "Skill", "input": "unslop", "none": False}))


if __name__ == "__main__":
    unittest.main()
