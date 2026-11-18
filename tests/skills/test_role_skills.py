import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKILLS = ROOT / "skills"
ROLE_FILES = [SKILLS / "t3-overseer" / "SKILL.md", SKILLS / "t3-ticket" / "SKILL.md",
              SKILLS / "t3-worker" / "SKILL.md", SKILLS / "t3-worker" / "procedure.md"]
TEAM_SKILLS = ["sol-loop", "unslop", "aws-sso-login", "veracity", "narration-author", "audio-pipeline",
               "scene-sync", "paul-voice", "cbrip:"]
SLOTS_LINK = "../t3-workflow/slots.md#reading-slots-in-a-role"


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
        self.assertIn("`model worker` line", (SKILLS / "t3-ticket" / "SKILL.md").read_text())
        self.assertIn("`model ticket`", (SKILLS / "t3-overseer" / "SKILL.md").read_text())

    def test_each_role_reads_the_slots(self):
        for path in (SKILLS / "t3-overseer" / "SKILL.md", SKILLS / "t3-ticket" / "SKILL.md",
                     SKILLS / "t3-worker" / "procedure.md"):
            text = path.read_text()
            self.assertIn("t3wf slots", text, path)
            self.assertIn(SLOTS_LINK, text, path)

    def test_relative_links_resolve(self):
        for path in [*ROLE_FILES, *(SKILLS / "t3-workflow").glob("*.md")]:
            for target in re.findall(r"\]\(([^)\s]+)\)", path.read_text()):
                if re.match(r"[a-z]+:", target):
                    continue
                file, _, frag = target.partition("#")
                dest = (path.parent / file).resolve() if file else path
                self.assertTrue(dest.exists(), f"{path.relative_to(ROOT)} -> {target}")
                if frag and dest.suffix == ".md":
                    heads = {anchor(h) for h in re.findall(r"^#+ (.+)$", dest.read_text(), re.M)}
                    self.assertIn(frag, heads, f"{path.relative_to(ROOT)} -> {target}")

    def test_no_skill_is_slash_only(self):
        for path in SKILLS.glob("*/SKILL.md"):
            self.assertNotIn("disable-model-invocation", frontmatter(path.read_text()), path)


if __name__ == "__main__":
    unittest.main()
