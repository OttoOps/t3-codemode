import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKILLS = ROOT / "skills"
ROLE_FILES = [SKILLS / "t3-overseer" / "SKILL.md", SKILLS / "t3-ticket" / "SKILL.md",
              SKILLS / "t3-worker" / "SKILL.md", SKILLS / "t3-worker" / "procedure.md"]
TEAM_SKILLS = ["sol-loop", "unslop", "aws-sso-login", "veracity", "narration-author", "audio-pipeline",
               "scene-sync", "paul-voice", "cbrip:"]
SLOTS_LINK = "../t3-workflow/slots.md#looking-up-slots-in-a-role"


def frontmatter(text):
    m = re.match(r"---\n(.*?)\n---\n", text, re.S)
    return dict(line.split(":", 1) for line in m.group(1).splitlines() if ":" in line) if m else {}


def anchor(heading):
    return re.sub(r"[^a-z0-9 -]", "", heading.strip().lower()).replace(" ", "-")


class RoleSkillTests(unittest.TestCase):
    def test_roles_name_no_team_skill(self):
        for path in ROLE_FILES:
            text = path.read_text()
            for name in TEAM_SKILLS:
                self.assertNotIn(name, text, f"{path.relative_to(ROOT)} hard-codes {name}")

    def test_the_worker_model_comes_from_the_slot_not_the_procedure(self):
        fm = frontmatter((SKILLS / "t3-worker" / "procedure.md").read_text())
        self.assertNotIn("model", fm)
        self.assertNotIn("effort", fm)
        self.assertIn('wf.slot("models")', (SKILLS / "t3-ticket" / "SKILL.md").read_text())
        self.assertIn('wf.slot("models"', (SKILLS / "t3-overseer" / "SKILL.md").read_text())

    def test_each_role_looks_up_its_slots_at_point_of_use(self):
        overseer = (SKILLS / "t3-overseer" / "SKILL.md").read_text()
        self.assertIn('wf.slot("models"', overseer)
        self.assertIn(SLOTS_LINK, overseer)

        ticket = (SKILLS / "t3-ticket" / "SKILL.md").read_text()
        self.assertIn('wf.slot("delivery")', ticket)
        self.assertIn('wf.slot("models")', ticket)
        self.assertIn("wf.renewal(", ticket)
        self.assertIn(SLOTS_LINK, ticket)

        worker = (SKILLS / "t3-worker" / "procedure.md").read_text()
        self.assertIn('wf.slot("review")', worker)
        self.assertIn('wf.slot("passes")', worker)
        self.assertIn("wf.renewal(", worker)
        self.assertIn(SLOTS_LINK, worker)

    def test_no_skill_file_is_python_or_mentions_t3wf_or_skills_flag(self):
        for path in sorted(SKILLS.rglob("*")):
            if not path.is_file():
                continue
            rel = path.relative_to(ROOT)
            self.assertNotEqual(path.suffix, ".py", f"Python file remains under skills/: {rel}")
            self.assertNotEqual(path.name, "t3-role.json", f"retired role config remains: {rel}")
            self.assertNotEqual(path.name, "defaults.json", f"retired defaults.json remains: {rel}")
            text = path.read_text()
            if path.name != "steps.json":
                self.assertNotIn("t3wf", text, f"{rel} still mentions t3wf")
            self.assertNotIn("python3", text, f"{rel} still invokes python3")
            self.assertNotIn("--skills", text, f"{rel} still passes --skills")

    def test_every_wf_board_example_uses_children(self):
        codemode = (SKILLS / "t3-workflow" / "codemode.md").read_text()
        self.assertIn("(await wf.board()).children.map(", codemode)
        for path in [ROOT / "README.md", *(SKILLS / "t3-workflow").glob("*.md"), *(ROOT / "docs" / "src").glob("*.md")]:
            text = path.read_text()
            self.assertNotIn("(await wf.board()).map(", text, f"{path.relative_to(ROOT)} maps over wf.board() directly")

    def test_relative_links_resolve(self):
        for path in [*ROLE_FILES, *(SKILLS / "t3-workflow").glob("*.md"),
                     *(SKILLS / "setup-t3-workflow").rglob("*.md")]:
            for target in re.findall(r"\]\(([^)\s]+)\)", path.read_text()):
                if re.match(r"[a-z]+:", target):
                    continue
                file, _, frag = target.partition("#")
                dest = (path.parent / file).resolve() if file else path
                self.assertTrue(dest.exists(), f"{path.relative_to(ROOT)} -> {target}")
                if frag and dest.suffix == ".md":
                    heads = {anchor(h) for h in re.findall(r"^#+ (.+)$", dest.read_text(), re.M)}
                    self.assertIn(frag, heads, f"{path.relative_to(ROOT)} -> {target}")

    def test_the_router_names_every_skill(self):
        text = (SKILLS / "t3-workflow" / "SKILL.md").read_text()
        table = text.split("## Which skill", 1)[1].split("\n## ", 1)[0]
        go_to = " ".join(row.rstrip(" |").rsplit("|", 1)[-1] for row in table.splitlines()[3:] if row.startswith("|"))
        for skill in SKILLS.glob("*/SKILL.md"):
            name = skill.parent.name
            if name != "t3-workflow":
                self.assertIn(f"`{name}`", go_to, f"t3-workflow's Which skill table does not route to {name}")

    def test_no_skill_is_slash_only(self):
        for path in SKILLS.glob("*/SKILL.md"):
            self.assertNotIn("disable-model-invocation", frontmatter(path.read_text()), path)


def section(text, heading):
    start = text.index(heading)
    level = heading.split(" ", 1)[0]
    nxt = re.search(rf"^#{{1,{len(level)}}} ", text[start + len(heading):], re.M)
    return text[start:start + len(heading) + nxt.start()] if nxt else text[start:]


class MergeRouteTests(unittest.TestCase):
    ticket = (SKILLS / "t3-ticket" / "SKILL.md").read_text()
    reconcile = (ROOT / "src" / "reconcile.rs").read_text()

    def test_the_ticket_merges_through_wf_merge_and_reports_each_outcome(self):
        word = section(self.ticket, "### On the user's word")
        self.assertIn("wf.merge({ repo:", word)
        self.assertIn("head:", word)
        for status in ("merged", "queued", "waiting", "refused"):
            self.assertIn(f"- `{status}`:", word, status)
        self.assertIn("A pending check is waited out", word)
        self.assertNotIn("**Without t3cm**", word)
        self.assertNotIn("--match-head-commit", word)

    def test_the_merge_watch_stays_for_root_and_reconcile_off_cases(self):
        word = section(self.ticket, "### On the user's word")
        self.assertIn("#### The merge watch", word)
        self.assertIn("Each merge-watch run", word)

    def test_no_skill_merges_through_t3s_merge_button_or_pr_action(self):
        rules = section(self.ticket, "## Hard rules")
        self.assertIn("Never merge through `gh pr merge` or T3's own merge button or its PR", rules)
        self.assertIn("merge queue", rules)
        codemode = (SKILLS / "t3-workflow" / "codemode.md").read_text()
        self.assertIn("never with `gh pr merge` or T3's\nown merge button or PR action", codemode)
        worker = (SKILLS / "t3-worker" / "procedure.md").read_text()
        self.assertIn("Never merge the PR yourself", worker)

    def test_every_merge_action_line_names_the_approved_head(self):
        lines = []
        for path in [*ROLE_FILES, *(SKILLS / "t3-workflow").glob("*.md")]:
            text = " ".join(path.read_text().split())
            lines += [(path, m) for m in re.findall(r'reply \\?"merge(?: and deploy)?\\?" to (?:merge|queue) #?[^,]+', text)]
        self.assertGreaterEqual(len(lines), 6)
        for path, line in lines:
            self.assertRegex(line, r"at head (<sha>|[0-9a-f]{7,40})", f"{path.relative_to(ROOT)}: {line}")

    def test_the_worker_merge_gate_names_its_head(self):
        worker = (SKILLS / "t3-worker" / "procedure.md").read_text()
        self.assertIn('ready to\n  merge: #<number>, head <sha>, checks passed"', worker)
        self.assertIn("Keep `head <sha>`", worker)

    def test_the_merge_wait_reads_what_the_reconcile_sends(self):
        wait = " ".join(section(self.ticket, "#### The merge wait").split())
        self.assertIn("does not poll GitHub", wait)
        self.assertIn('wf.gate({ open: "merge" })', wait)
        self.assertIn("`board.reconcileIdleMinutes` to `0`", wait)
        watch = section(self.ticket, "#### The merge watch")
        self.assertIn("`wf.cards()`", watch)
        self.assertIn("call `wf.merge` again", watch)
        self.assertIn("starts no\n  new watch", watch)
        self.assertIn("`<owner>/<name>#<number>` in lower case", watch)
        self.assertNotIn("has the PR URL", watch)
        for phrase in ("merged on GitHub at", "Run your post-merge steps.", "was closed on GitHub without merging"):
            self.assertIn(phrase, wait)
            self.assertIn(phrase, self.reconcile, f"src/reconcile.rs no longer sends {phrase!r}")
        for answer in ("merge", "requeue", "ok", "hold"):
            self.assertIn(f'\\"{answer}\\"', self.reconcile, answer)
            self.assertIn(f'"{answer}"', wait, answer)


if __name__ == "__main__":
    unittest.main()
