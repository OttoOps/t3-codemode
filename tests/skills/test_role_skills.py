import json
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
    worker = (SKILLS / "t3-worker" / "procedure.md").read_text()
    reconcile = (ROOT / "src" / "reconcile.rs").read_text()

    def test_the_ticket_merges_through_wf_merge_and_reports_each_outcome(self):
        word = section(self.ticket, "### On the user's word")
        self.assertIn("wf.merge({ repo:", word)
        self.assertIn("head:", word)
        for status in ("merged", "queued", "waiting", "refused"):
            self.assertIn(f"- `{status}`:", word, status)
        self.assertIn("holds the approval as `waiting` until it finishes", " ".join(word.split()))
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

    def test_the_worker_reports_each_pr_step_with_its_head(self):
        worker = " ".join((SKILLS / "t3-worker" / "procedure.md").read_text().split())
        for event in ("opened", "updated", "ready"):
            self.assertIn(f'wf.pr({{ event: "{event}", repo: "<owner>/<name>", pr: <number>, head: "<', worker)
        self.assertIn('publishing: "<Publishing line>"', worker)
        self.assertIn("A merge approval holds only for the head it names", worker)
        self.assertNotIn('gate: "merge", body', worker)

    def test_the_ticket_hands_its_pr_format_to_the_worker_link(self):
        ticket = " ".join(self.ticket.split())
        self.assertIn("wf.openChild({ role: \"worker\", label, issue, parentIssue, part, pr })", ticket)
        self.assertIn("{ format, sections, passes, mergeOnly, method }", ticket)
        between = " ".join(section(self.ticket, "## 4. Between workers").split())
        self.assertIn("t3cm left this to the ticket: <why>", between)
        self.assertIn("### PR updated", section(self.ticket, "## 4. Between workers"))
        self.assertNotIn("misnamed", between)
        self.assertNotIn("sent under a gate of its own", between)

    def test_the_merge_wait_reads_what_the_reconcile_sends(self):
        wait = " ".join(section(self.ticket, "#### The merge wait").split())
        self.assertIn("does not poll GitHub", wait)
        self.assertIn('wf.gate({ open: "merge" })', wait)
        self.assertIn("`board.reconcileIdleMinutes` to `0`", wait)
        watch = section(self.ticket, "#### The merge watch")
        self.assertIn("`wf.cards()`", watch)
        self.assertIn("call `wf.merge` again", watch)
        self.assertIn("starts no new watch", watch)
        self.assertNotIn("has the PR URL", watch)
        for phrase in ("merged on GitHub at", "Run your post-merge steps.", "was closed on GitHub without merging"):
            self.assertIn(phrase, wait)
            sent = phrase.replace("GitHub", "{forge}")
            self.assertIn(sent, self.reconcile, f"src/reconcile.rs no longer sends {sent!r}")
        for answer in ("merge", "requeue", "ok", "hold"):
            self.assertIn(f'\\"{answer}\\"', self.reconcile, answer)
            self.assertIn(f'"{answer}"', wait, answer)

    def test_a_pending_check_on_an_approved_head_waits_in_wf_merge(self):
        word = " ".join(section(self.ticket, "### On the user's word").split())
        self.assertNotIn("end the turn; the worker reports again", word)
        self.assertNotIn('outcome `"pending"`', word)
        self.assertIn('without a second "merge"', word)
        watch = " ".join(section(self.ticket, "#### The merge watch").split())
        self.assertNotIn('"pending"', watch)
        self.assertIn("The merge watch serves three cases", watch)
        self.assertIn("Approval never carries over to a new head", watch)

    def test_the_board_merges_on_the_answer_in_code(self):
        word = " ".join(section(self.ticket, "### On the user's word").split())
        answer = (ROOT / "src" / "merge_answer.rs").read_text()
        self.assertIn("t3cm runs this section in code", word)
        self.assertIn("`[t3cm event from=t3cm]`", word)
        for phrase in ("Remaining delivery stages for this PR, in order:", "Hand off.", "act on that outcome as"):
            self.assertIn(phrase, answer, f"src/merge_answer.rs no longer sends {phrase!r}")
        self.assertIn("Run them from item 4", word)
        self.assertIn("act on it as item 2 says", word)
        wait = " ".join(section(self.ticket, "#### The merge wait").split())
        self.assertIn('it runs "merge", "requeue" and "ok" in code', wait)

    def test_the_merge_watch_retry_repeats_the_check_gate_when_reconcile_is_off(self):
        watch = " ".join(section(self.ticket, "#### The merge watch").split())
        m = re.search(r"query='([^']+)'", watch)
        self.assertIsNotNone(m, "merge-watch GraphQL query missing")
        query = m.group(1)
        depth = 0
        for ch in query:
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                self.assertGreaterEqual(depth, 0, f"unmatched closing brace in query: {query}")
        self.assertEqual(depth, 0, f"unbalanced braces in merge-watch query: {query}")
        self.assertIn("isDraft", query)
        self.assertIn("statusCheckRollup", query)
        self.assertIn('Before calling `wf.merge` again, repeat "On the user\'s word" item 1\'s check gate', watch)
        self.assertIn("including an optional check that re-ran while `mergeStateStatus` is `CLEAN` or `HAS_HOOKS`", watch)
        self.assertIn("do not call `wf.merge` and end the turn quietly", watch)

    def test_an_untracked_forge_ticket_starts_its_merge_watch_when_the_pr_opens(self):
        start = " ".join(section(self.ticket, "## 1. Start").split())
        self.assertIn("on such an untracked forge this thread starts its own merge watch when the PR or MR opens", start)
        self.assertIn("whether or not it has a parent", start)
        opened = " ".join(section(self.ticket, "### PR opened").split())
        self.assertIn("On an untracked forge (step 1 item 3), start the merge watch now", opened)
        updated = " ".join(section(self.ticket, "### PR updated").split())
        self.assertIn("On an untracked forge, update the merge watch's prompt with the new head", updated)
        wait = " ".join(section(self.ticket, "#### The merge wait").split())
        self.assertIn("The reconcile never reads a PR or MR on an untracked forge", wait)
        self.assertIn("whether or not it has a parent and whatever `board.reconcileIdleMinutes` says", wait)
        watch = " ".join(section(self.ticket, "#### The merge watch").split())
        self.assertIn("the untracked-forge case", watch)
        self.assertIn("from the moment the PR or MR opens, with or without a parent", watch)
        self.assertIn("`glab mr view <iid> --output json`", watch)
        untracked = watch.split('On an untracked forge (the prompt says "Forge: untracked")', 1)[1]
        self.assertIn("in place of the GitHub query", untracked)
        for case in ("**Merged:**", "**Closed without merging:**", "**Head moved away from the head the prompt names:**",
                     "**Still open at the prompt's head:**", "**The status command fails:**"):
            self.assertIn(case, untracked, case)
        self.assertIn("approval never carries over to a new head", untracked)
        self.assertIn("Keep the task", untracked)
        self.assertIn("keep the task until that succeeds", untracked)
        self.assertIn("start the merge watch again at its current head", untracked)
        self.assertIn("clear any merge gate open for it", untracked)
        self.assertIn("it deletes the watch before it goes on from \"On the user's word\" item 3, so items 3 and 4 run once", watch)
        self.assertIn('setting "Auth gate sent" back to "no"', untracked)
        self.assertIn("The approved chain holds only when the merged head is the head it approved", untracked)

    def test_the_worker_does_not_wait_on_t3s_watcher_for_an_untracked_merge(self):
        open_pr = " ".join(section(self.worker, "## 3. Open the PR and watch it").split())
        self.assertIn('`wf.send({ kind: "event", body })` (a `note` never wakes it', open_pr)
        self.assertIn("T3's pull request watcher does not wake you on a merge there", open_pr)

    def test_the_root_merge_watch_matches_reconcile_cards_by_approval_id(self):
        watch = " ".join(section(self.ticket, "#### The merge watch").split())
        self.assertIn('c.approval === "<approval>"', watch)
        self.assertIn("Ignore cards with no `approval` or a different `approval`", watch)
        self.assertNotIn("`<owner>/<name>#<number>` in lower case", watch)


class DeliveryStepsTests(unittest.TestCase):
    ticket = (SKILLS / "t3-ticket" / "SKILL.md").read_text()
    worker = (SKILLS / "t3-worker" / "procedure.md").read_text()
    setup_skill = (SKILLS / "setup-t3-workflow" / "SKILL.md").read_text()
    interview = (SKILLS / "setup-t3-workflow" / "references" / "interview.md").read_text()
    slots = (SKILLS / "t3-workflow" / "slots.md").read_text()
    board = (ROOT / "docs" / "src" / "board.md").read_text()
    adr = (ROOT / "docs" / "adr" / "0007-composable-delivery.md").read_text()

    def test_ticket_builds_gates_from_resolved_delivery_steps(self):
        ready = " ".join(section(self.ticket, "### Ready to merge").split())
        self.assertIn('wf.slot("delivery", { files })', ready)
        self.assertIn("`skipped`", ready)
        self.assertIn('gate: "merge"', ready)
        self.assertIn('wf.send({ kind: "gate", gate: "merge", body, options })', ready)
        self.assertIn('["merge, deploy and verify", "merge and deploy", "merge"]', ready)
        self.assertIn('mode: "merge"', ready)
        self.assertIn('mode: "tag-deploy"', ready)

        stages = " ".join(section(self.ticket, "### Delivery stages").split())
        self.assertIn('gate: "never"', stages)
        self.assertIn('gate: "chain"', stages)
        self.assertIn('gate: "always"', stages)
        self.assertIn('gate: "delivery:<id>"', stages)
        self.assertIn('"skip <id>"', stages)
        self.assertIn('kind: "skill"', stages)
        self.assertIn('kind: "manual"', stages)
        self.assertIn("watch: true", stages)
        self.assertIn("The deploy watch", stages)
        self.assertIn("8-option and 60-character-per-option limits", stages)
        self.assertIn("done and verify", stages)
        self.assertIn('state === "MERGED"', ready)
        self.assertIn("override delivery to merge-only", ready)
        self.assertIn("union of the original PR's `files` and all follow-up PRs' `files`", ready)
        handoff = " ".join(section(self.ticket, "### The handoff").split())
        self.assertIn('re-resolve `await wf.slot("delivery", { files: combinedFiles })`', handoff)
        self.assertIn("previously skipped by `paths` but now matches", handoff)
        self.assertIn("repeat any earlier completed stages invalidated by the new `main` revision", handoff)
        self.assertIn("Preserve an earlier completed stage only when its result still holds", handoff)
        self.assertIn('For a `post-handoff` stage (under `closeAfter: "all"`)', stages)

    def test_ticket_honors_scope_boundaries_and_close_after(self):
        stages = " ".join(section(self.ticket, "### Delivery stages").split())
        self.assertIn("`pr` scope", stages)
        self.assertIn("`ticket` scope", stages)
        self.assertIn("`post-handoff` scope", stages)
        self.assertIn("targeted fix worker on `main`", stages)
        self.assertIn('wf.send({ kind: "report", body })', stages)
        self.assertIn('`closeAfter`', stages)

        close = " ".join(section(self.ticket, "## 5. Close").split())
        self.assertIn("`post-handoff`", close)
        self.assertIn('`closeAfter` is `"pr"`', close)
        self.assertIn('`closeAfter` is `"ticket"`', close)
        self.assertIn('`closeAfter` is `"all"`', close)

    def test_worker_hands_off_after_its_last_pr_scoped_stage(self):
        text = " ".join(self.worker.split())
        self.assertIn("`pr`-scoped delivery stages finish", text)
        self.assertIn("never wait on `ticket`-scoped or `post-handoff` stages", text)
        self.assertIn("Merged #<number> at <sha>. Hand off.", text)
        self.assertIn("Delivered: <summary>. Hand off.", text)
        self.assertIn("Delivery stage <id> failed:", text)

    def test_setup_interview_covers_multi_step_delivery_and_repo_config(self):
        self.assertIn("t3cm setup write --repo-config", self.setup_skill)
        self.assertIn(".t3-workflow.json", self.setup_skill)
        self.assertIn("repoPath", self.setup_skill)
        self.assertIn("repoExists", self.setup_skill)
        self.assertIn("without `--asked`", self.setup_skill)

        delivery = " ".join(section(self.interview, "## Delivery").split())
        self.assertIn("t3cm setup write --repo-config", delivery)
        self.assertIn(".t3-workflow.json", delivery)
        self.assertIn("`closeAfter`", delivery)
        self.assertIn("`post-handoff`", delivery)
        self.assertIn('"steps"', delivery)

    def test_slots_and_board_docs_describe_composable_delivery(self):
        slots = " ".join(self.slots.split())
        self.assertIn(".t3-workflow.json", slots)
        self.assertIn('wf.slot("delivery", { files? })', slots)
        self.assertIn("t3cm setup write --repo-config", slots)
        self.assertIn("`closeAfter`", slots)
        self.assertIn('gate: "delivery:<id>"', slots)

        board = " ".join(self.board.split())
        self.assertIn("Delivery gates and chains", board)
        self.assertIn('wf.slot("delivery", { files })', board)
        self.assertIn('gate: "merge"', board)
        self.assertIn('gate: "delivery:<id>"', board)
        self.assertIn('["merge, deploy and verify", "merge and deploy", "merge"]', board)
        self.assertIn("0007-composable-delivery.md", board)

    def test_composable_delivery_adr_exists_and_is_accepted(self):
        fm = frontmatter(self.adr)
        self.assertEqual(fm.get("status", "").strip(), "accepted")
        self.assertIn("delivery.steps", self.adr)
        self.assertIn(".t3-workflow.json", self.adr)
        self.assertIn('gate: "delivery:<id>"', self.adr)
        self.assertIn("closeAfter", self.adr)
        for forbidden in TEAM_SKILLS:
            self.assertNotIn(forbidden, self.adr, f"ADR mentions {forbidden}")


class RemoteBoardSetupTests(unittest.TestCase):
    setup_skill = (SKILLS / "setup-t3-workflow" / "SKILL.md").read_text()
    interview = (SKILLS / "setup-t3-workflow" / "references" / "interview.md").read_text()
    steps = json.loads((SKILLS / "setup-t3-workflow" / "steps.json").read_text())["steps"]
    board = (ROOT / "docs" / "src" / "board.md").read_text()

    def remote(self):
        return " ".join(section(self.interview, "## Remote board").split())

    def test_the_remote_step_is_an_interview_step_newer_than_0_7_0(self):
        step = next(s for s in self.steps if s["id"] == "board:remote")
        self.assertEqual((step["kind"], step["slot"]), ("interview", "remote"))
        self.assertNotIn("legacy", step)
        self.assertGreater(tuple(int(n) for n in step["since"].split(".")), (0, 7, 0))

    def test_setup_asks_about_the_remote_board_last_and_records_it(self):
        text = " ".join(self.setup_skill.split())
        self.assertIn("model ticket, model worker, flow, remote board.", text)
        self.assertIn("--asked review,passes,credentials,delivery,models,flow,remote", text)
        self.assertIn("records only `--asked flow` and `--asked remote`", text)

    def test_the_remote_interview_offers_off_tailscale_and_a_proxy(self):
        text = self.remote()
        self.assertIn("**off** (the default)", text)
        self.assertIn("**Tailscale Serve**", text)
        for proxy in ("Caddy", "nginx", "cloudflared"):
            self.assertIn(proxy, text)
        self.assertIn("""t3cm config set board.remote '{"mode": "pairing", "origin": "<origin>"}'""", text)

    def test_tailscale_runs_expose_and_explains_pairing_is_not_needed(self):
        text = self.remote()
        self.assertIn("t3cm board expose --tailscale", text)
        self.assertIn("--allow-user <login>", text)
        self.assertIn("--require-pairing", text)
        self.assertIn("`Tailscale-User-Login` header", text)
        self.assertIn("it is not needed here", text)
        self.assertIn('t3cm board pair --label "<device>"', text)
        self.assertIn("QR code", text)

    def test_the_operator_step_runs_through_the_sudo_flow(self):
        text = self.remote()
        self.assertIn("sudo tailscale set --operator=$USER", text)
        self.assertIn("run the command with `sudo -n` first", text)
        self.assertIn("then run the same expose again", text)
        self.assertIn('`request_secret` with `delivery: "file"`', text)
        self.assertIn('sudo -S -p \'\' tailscale set --operator="$USER" < "$file"', text)
        self.assertIn("`tailscale.exe`, and the operator step does not apply", text)

    def test_every_change_restarts_and_is_checked(self):
        text = self.remote()
        self.assertIn("`restartHint`", text)
        self.assertIn("restart-only", text)
        self.assertIn("`remote.enabled` is `true`", text)
        self.assertIn("`remote.origin`", text)
        self.assertIn('curl -sS -H \'Accept: text/html\' "<origin>/"', text)
        self.assertIn("`Sign in to t3 board - t3 board`", text)
        self.assertIn("`t3cm status` shows `remote.enabled: false`", text)

    def test_turning_it_off_reuses_the_expose_port(self):
        off = " ".join(section(self.interview, "### Turn it off").split())
        self.assertIn("t3cm board unexpose", off)
        self.assertIn("same `--port` the expose used", off)
        self.assertIn("""t3cm config set board.remote '{"mode": "off"}'""", off)

    def test_the_remote_interview_links_the_book(self):
        text = self.remote()
        self.assertIn("https://ottoops.com/t3-codemode/board.html#remote-access", text)
        self.assertIn("https://ottoops.com/t3-codemode/config.html#remote-board", text)
        self.assertIn("https://ottoops.com/t3-codemode/board.html#reverse-proxy-examples", text)
        heads = {anchor(h) for h in re.findall(r"^#+ (.+)$", self.board, re.M)}
        self.assertTrue({"remote-access", "reverse-proxy-examples"} <= heads)


class FlowSetupTests(unittest.TestCase):
    setup_skill = (SKILLS / "setup-t3-workflow" / "SKILL.md").read_text()
    interview = (SKILLS / "setup-t3-workflow" / "references" / "interview.md").read_text()
    steps = json.loads((SKILLS / "setup-t3-workflow" / "steps.json").read_text())["steps"]
    config_doc = (ROOT / "docs" / "src" / "config.md").read_text()
    board_doc = (ROOT / "docs" / "src" / "board.md").read_text()

    def flow(self):
        return " ".join(section(self.interview, "## Flow").split())

    def test_the_flow_step_is_an_interview_step_newer_than_0_10_0(self):
        step = next(s for s in self.steps if s["id"] == "board:flow")
        self.assertEqual((step["kind"], step["slot"]), ("interview", "flow"))
        self.assertNotIn("legacy", step)
        self.assertGreater(tuple(int(n) for n in step["since"].split(".")), (0, 10, 0))

    def test_setup_includes_flow_in_description_and_completion(self):
        text = " ".join(self.setup_skill.split())
        self.assertIn("project flow", text)
        self.assertIn("any flow change has saved and reloaded", text)

    def test_the_flow_interview_covers_every_flow_key_one_at_a_time(self):
        text = self.flow()
        for token in (
            "t3cm config get flow",
            "t3cm config set flow.<key> <value>",
            "`flow.launch`",
            "`flow.maxTickets`",
            "`flow.repos`",
            "`flow.includeRepos`",
            "`flow.excludeRepos`",
            "`flow.optIn.label`",
            "`flow.optIn.milestone`",
            "`flow.serializeFileConflicts`",
            "t3cm config set flow",
            "t3cm config unset flow",
            "t3cm config unset flow.optIn",
            "Reloaded the running service",
            "warn: not reloaded",
            "kept the config it was running",
            "--asked flow",
        ):
            self.assertIn(token, text, token)

    def test_the_flow_interview_links_the_book(self):
        text = self.flow()
        self.assertIn("https://ottoops.com/t3-codemode/config.html#flow", text)
        self.assertIn("https://ottoops.com/t3-codemode/board.html#the-flow", text)
        config_heads = {anchor(h) for h in re.findall(r"^#+ (.+)$", self.config_doc, re.M)}
        board_heads = {anchor(h) for h in re.findall(r"^#+ (.+)$", self.board_doc, re.M)}
        self.assertIn("flow", config_heads)
        self.assertIn("the-flow", board_heads)


class NonGithubTrackerTests(unittest.TestCase):
    ticket = (SKILLS / "t3-ticket" / "SKILL.md").read_text()
    worker = (SKILLS / "t3-worker" / "procedure.md").read_text()
    workflow = (SKILLS / "t3-workflow" / "SKILL.md").read_text()
    codemode = (SKILLS / "t3-workflow" / "codemode.md").read_text()

    def test_ticket_step_1_resolves_explicit_ref_before_repo_default(self):
        start = " ".join(section(self.ticket, "## 1. Start").split())
        for token in ("`github-issues`", "`jira`", "`gitlab-issues`", "`none`",
                      "explicit ticket ref always wins over the repo default",
                      "`533` or `#533`", 'wf.init({ role: "ticket" })', "GitHub `origin`"):
            self.assertIn(token, start)

    def test_ticket_step_2_reads_non_github_without_gh_and_keeps_github_writes(self):
        read_split = " ".join(section(self.ticket, "## 2. Read the work and split it").split())
        for token in ("gh issue view <n>", "skip `gh issue` commands", 'gate: "ticket"',
                      "with tracker `github-issues`:", "skip `gh issue comment` and `gh issue edit`", "Refs PROJ-123"):
            self.assertIn(token, read_split)

    def test_ticket_step_3_passes_non_github_slugs_scope_and_tracker_to_worker(self):
        launch = " ".join(self.ticket.split("## 3. Launch a worker", 1)[1].split("## 4. Between workers", 1)[0].split())
        for token in ("<owner>/<repo>#<n>", "KEY-N", "<host>/KEY-N", "<host>/<scope>#<n>",
                      "Tracker:", "Scope:", "`wf.join` or `wf.init`"):
            self.assertIn(token, launch)

    def test_ticket_pr_handoff_and_close_scope_gh_issue_writes_to_github(self):
        opened = " ".join(section(self.ticket, "### PR opened").split())
        for token in ("`Refs #<n>`", "`Refs PROJ-123`", "`Refs <host>/PROJ-123`", "`Refs <host>/<scope>#<n>`"):
            self.assertIn(token, opened)

        handoff = " ".join(section(self.ticket, "### The handoff").split())
        for token in ("With tracker `github-issues`:", "skip `gh issue` comments and closes",
                      'When `closeAfter` is `"pr"` and tracker is `github-issues`'):
            self.assertIn(token, handoff)

        close = " ".join(section(self.ticket, "## 5. Close").split())
        for token in ("With tracker `github-issues`:", "skip `gh issue` comments, label edits, and closes"):
            self.assertIn(token, close)

    def test_worker_uses_refs_for_non_github_and_keeps_github_trailer(self):
        link_proto = " ".join(section(self.worker, "## The link protocol").split())
        self.assertIn("On a non-GitHub git host, check the MR/PR state", link_proto)

        orient = " ".join(section(self.worker, "## 1. Orient").split())
        self.assertIn("Skip `gh issue` commands", orient)
        self.assertIn("Tracker:", orient)

        build = " ".join(section(self.worker, "## 2. Build").split())
        for token in ('--trailer "Github-Issue:#<number>"', "omit the `Github-Issue` trailer", "`Refs PROJ-123`"):
            self.assertIn(token, build)

        open_pr = " ".join(section(self.worker, "## 3. Open the PR and watch it").split())
        for token in ("`Refs #<number>`", "`Refs PROJ-123`", "`Refs <host>/PROJ-123`",
                      "`Refs <host>/<scope>#<n>`", "non-GitHub git host"):
            self.assertIn(token, open_pr)

    def test_workflow_docs_describe_non_github_issue_slugs_with_valid_js(self):
        self.assertNotRegex(self.workflow, r"wf\.openChild\(\{[^}]*\?")
        for text in (" ".join(self.workflow.split()), " ".join(self.codemode.split())):
            for slug in ("<owner>/<repo>#<n>", "KEY-N", "<host>/KEY-N", "<host>/<scope>#<n>"):
                self.assertIn(slug, text)


if __name__ == "__main__":
    unittest.main()
