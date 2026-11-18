import json
import os

from test_t3wf import T3wfTestCase

CLAUDE_CAPS = {"providers": [
    {"providerInstanceId": "claudeAgent", "driverKind": "claudeAgent", "constraints": [], "models": [
        {"id": "claude-opus-5-5", "options": [{"id": "effort", "values": ["low", "medium", "high", "max"]}]}]},
    {"providerInstanceId": "codex", "driverKind": "codex", "constraints": [], "models": [
        {"id": "gpt-6.2-sol", "options": [{"id": "effort", "values": ["low", "medium", "high"]}]}]},
]}
NO_CLAUDE_CAPS = {"providers": [
    {"providerInstanceId": "claudeAgent", "driverKind": "claudeAgent",
     "constraints": ["Provider instance is disabled."], "models": []},
    CLAUDE_CAPS["providers"][1],
]}


class SlotsTestCase(T3wfTestCase):
    def setUp(self):
        super().setUp()
        self.repo = self.env.tmp / "repo"
        self.repo.mkdir()
        self.claude = self.env.home / ".claude"
        self.config = self.env.config / "t3-workflow" / "config.json"

    def skill(self, base, name, inline=False, fm_name=None):
        d = base / name
        d.mkdir(parents=True, exist_ok=True)
        extra = "disable-model-invocation: true\n" if inline else ""
        (d / "SKILL.md").write_text(f"---\nname: {fm_name or name}\ndescription: test\n{extra}---\n\nBody.\n")
        return d / "SKILL.md"

    def plugin_skill(self, plugin, name, version="1.0.0", market="acme", **kw):
        root = self.claude / "plugins" / "cache" / market / plugin / version
        return root, self.skill(root / "skills", name, **kw)

    def install_plugins(self, *entries):
        data = {"version": 2, "plugins": {f"{p}@acme": [{"scope": "user", "installPath": str(root)}]
                                          for p, root in entries}}
        path = self.claude / "plugins" / "installed_plugins.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(data))

    def write_config(self, cfg):
        self.config.parent.mkdir(parents=True, exist_ok=True)
        self.config.write_text(json.dumps(cfg) if not isinstance(cfg, str) else cfg)

    def caps(self, caps):
        data = json.loads(self.env.fake.read_text())
        data["capabilities"] = caps
        self.env.fake.write_text(json.dumps(data))

    def agents_md(self, block):
        (self.repo / "AGENTS.md").write_text("# Repo\n\nIntro.\n\n## Agent workflow\n\n" + block +
                                             "\n## Other\n\n- Review: ignored-skill\n")

    def t3wf(self, *args, ok=True):
        return self.env.run(*args, "--repo", str(self.repo), ok=ok)

    def slots(self, *extra):
        return json.loads(self.t3wf("slots", "--json", *extra).stdout)


class Resolver(SlotsTestCase):
    def test_finds_a_namespaced_and_a_bare_sol_loop(self):
        root, plugin_path = self.plugin_skill("cbrip", "sol-loop")
        self.install_plugins(("cbrip", root))
        bare_path = self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        r = json.loads(self.t3wf("resolve", "sol-loop", "--json").stdout)
        found = {c["name"]: c["path"] for c in r["candidates"]}
        self.assertEqual(found, {"sol-loop": str(bare_path), "cbrip:sol-loop": str(plugin_path)})
        self.assertEqual(r["resolved"]["status"], "ok")
        self.assertEqual(r["resolved"]["name"], "sol-loop")

    def test_a_bare_name_pins_the_install_with_no_namespace(self):
        root, _ = self.plugin_skill("cbrip", "sol-loop")
        self.install_plugins(("cbrip", root))
        bare_path = self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        r = json.loads(self.t3wf("resolve", "sol-loop", "--json", "--skills", "cbrip:sol-loop").stdout)
        self.assertEqual([c["name"] for c in r["candidates"]], ["cbrip:sol-loop", "sol-loop"])
        self.assertEqual((r["resolved"]["status"], r["resolved"]["path"]), ("ok", str(bare_path)))

    def test_a_bare_name_that_only_matches_namespaces_is_ambiguous(self):
        cbrip, cbrip_path = self.plugin_skill("cbrip", "sol-loop")
        tools, _ = self.plugin_skill("tools", "sol-loop")
        self.install_plugins(("cbrip", cbrip), ("tools", tools))
        r = json.loads(self.t3wf("resolve", "sol-loop", "--json").stdout)["resolved"]
        self.assertEqual((r["status"], r["path"], r["others"]), ("ambiguous", str(cbrip_path), ["tools:sol-loop"]))

    def test_a_recorded_full_name_wins(self):
        root, plugin_path = self.plugin_skill("cbrip", "sol-loop")
        self.install_plugins(("cbrip", root))
        self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        r = json.loads(self.t3wf("resolve", "cbrip:sol-loop", "--json").stdout)["resolved"]
        self.assertEqual((r["status"], r["name"], r["path"]), ("ok", "cbrip:sol-loop", str(plugin_path)))

    def test_the_session_list_comes_first(self):
        root, plugin_path = self.plugin_skill("cbrip", "sol-loop")
        self.install_plugins(("cbrip", root))
        self.skill(self.claude / "skills", "sol-loop")
        r = json.loads(self.t3wf("resolve", "sol-loop", "--json", "--skills", "pr, cbrip:sol-loop").stdout)
        self.assertEqual([c["name"] for c in r["candidates"]], ["cbrip:sol-loop", "sol-loop"])
        self.assertEqual(r["candidates"][0]["where"], "session")
        self.assertEqual(r["candidates"][0]["path"], str(plugin_path))

    def test_a_skill_that_disables_model_invocation_is_inline(self):
        path = self.skill(self.claude / "skills", "unslop", inline=True)
        r = json.loads(self.t3wf("resolve", "unslop", "--json").stdout)["resolved"]
        self.assertEqual((r["invoke"], r["path"]), ("inline", str(path)))
        self.assertIn("[inline]", self.t3wf("resolve", "unslop").stdout)

    def test_every_install_location_is_searched(self):
        self.skill(self.claude / "skills", "a")
        self.skill(self.env.home / ".agents" / "skills", "b")
        self.skill(self.repo / ".claude" / "skills", "c")
        self.skill(self.env.home / ".codex" / "skills", "d")
        root, _ = self.plugin_skill("tools", "e")
        self.install_plugins(("tools", root))
        wheres = {n: self.t3wf("resolve", n).stdout for n in "abcde"}
        self.assertIn("(~/.claude/skills)", wheres["a"])
        self.assertIn("(~/.agents/skills)", wheres["b"])
        self.assertIn("(project .claude/skills)", wheres["c"])
        self.assertIn("(~/.codex/skills)", wheres["d"])
        self.assertIn("tools:e", wheres["e"])

    def test_a_symlinked_install_counts_once(self):
        self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        (self.claude / "skills").mkdir(parents=True)
        os.symlink(self.env.home / ".agents" / "skills" / "sol-loop", self.claude / "skills" / "sol-loop")
        r = json.loads(self.t3wf("resolve", "sol-loop", "--json").stdout)
        self.assertEqual(len(r["candidates"]), 1)
        self.assertEqual(r["resolved"]["status"], "ok")

    def test_without_installed_plugins_the_newest_cached_version_is_used(self):
        self.plugin_skill("cbrip", "sol-loop", version="1.9.0")
        _, newest = self.plugin_skill("cbrip", "sol-loop", version="1.10.0")
        r = json.loads(self.t3wf("resolve", "sol-loop", "--json").stdout)
        self.assertEqual([c["path"] for c in r["candidates"]], [str(newest)])

    def test_same_named_installs_are_all_listed_but_not_ambiguous(self):
        claude = self.skill(self.claude / "skills", "sol-loop")
        codex = self.skill(self.env.home / ".codex" / "skills", "sol-loop")
        r = json.loads(self.t3wf("resolve", "sol-loop", "--json").stdout)
        self.assertEqual([c["path"] for c in r["candidates"]], [str(claude), str(codex)])
        self.assertEqual((r["resolved"]["status"], r["resolved"]["path"]), ("ok", str(claude)))

    def test_a_bare_symlink_to_a_plugin_skill_keeps_the_plugin_name(self):
        root, plugin_path = self.plugin_skill("cbrip", "sol-loop")
        self.install_plugins(("cbrip", root))
        (self.claude / "skills").mkdir(parents=True)
        os.symlink(plugin_path.parent, self.claude / "skills" / "sol-loop")
        r = json.loads(self.t3wf("resolve", "cbrip:sol-loop", "--json").stdout)["resolved"]
        self.assertEqual((r["status"], r["path"]), ("ok", str(plugin_path)))

    def test_the_codex_plugin_cache_is_searched(self):
        root = self.env.home / ".codex" / "plugins" / "cache" / "acme" / "cbrip"
        self.skill(root / "1.0.0" / "skills", "sol-loop")
        newest = self.skill(root / "1.2.0" / "skills", "sol-loop")
        r = json.loads(self.t3wf("resolve", "cbrip:sol-loop", "--json").stdout)["resolved"]
        self.assertEqual((r["status"], r["path"]), ("ok", str(newest)))

    def test_a_missing_full_name_exits_1_and_lists_other_matches(self):
        self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        r = self.t3wf("resolve", "cbrip:sol-loop", ok=False)
        self.assertEqual(r.returncode, 1)
        self.assertIn("cbrip:sol-loop: not installed; other matches:", r.stdout)
        self.assertIn("sol-loop: ", r.stdout.splitlines()[1])

    def test_an_unknown_skill_exits_1(self):
        r = self.t3wf("resolve", "nothing-here", ok=False)
        self.assertEqual(r.returncode, 1)
        self.assertIn("not installed", r.stdout)


class Settings(SlotsTestCase):
    def test_no_config_yields_the_defaults(self):
        self.caps(CLAUDE_CAPS)
        eff = self.slots()
        self.assertIsNone(eff["config"])
        self.assertEqual(eff["problems"], [])
        self.assertIsNone(eff["review"]["skill"])
        self.assertIn("t3dotgg/tstack", eff["review"]["note"])
        self.assertEqual(eff["passes"]["skills"], [])
        self.assertEqual(eff["credentials"]["renewals"], [])
        self.assertEqual(eff["delivery"], {"slot": "delivery", "source": "default", "mode": "merge"})
        for role in ("ticket", "worker"):
            m = eff["models"][role]
            self.assertEqual(m["spec"], {"provider": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"})
            self.assertIsNone(m["note"])
        r = self.t3wf("doctor")
        self.assertIn("doctor: ok", r.stdout)

    def test_default_review_uses_a_found_sol_loop(self):
        self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        r = self.slots("--no-t3")["review"]
        self.assertEqual((r["source"], r["skill"]["name"]), ("default", "sol-loop"))

    def test_default_model_is_the_provider_default_without_claude(self):
        self.caps(NO_CLAUDE_CAPS)
        m = self.slots()["models"]["worker"]
        self.assertIsNone(m["spec"])
        self.assertEqual(m["note"], "T3 offers no Claude")

    def test_default_model_is_the_provider_default_when_t3_is_unreachable(self):
        m = self.slots()["models"]["ticket"]
        self.assertIsNone(m["spec"])
        self.assertIn("T3 unreachable", m["note"])

    def test_user_config_sets_every_slot(self):
        self.skill(self.claude / "skills", "unslop", inline=True)
        self.skill(self.claude / "skills", "pr")
        self.skill(self.claude / "skills", "aws-sso-login")
        self.skill(self.claude / "skills", "deploy")
        self.caps(CLAUDE_CAPS)
        self.write_config({"version": 1, "review": None, "passes": ["unslop", "pr"],
                           "credentials": [{"match": "ExpiredToken", "skill": "aws-sso-login"}],
                           "delivery": {"mode": "tag-deploy", "skill": "deploy"},
                           "models": {"ticket": {"model": "claude-opus-5-5", "effort": "max"},
                                      "worker": {"provider": "codex", "model": "gpt-6.2-sol", "effort": "high"}}})
        eff = self.slots()
        self.assertEqual(eff["config"], str(self.config))
        self.assertEqual((eff["review"]["skill"], eff["review"]["note"]), (None, "none configured"))
        self.assertEqual([(s["name"], s["invoke"]) for s in eff["passes"]["skills"]],
                         [("unslop", "inline"), ("pr", "skill")])
        self.assertEqual(eff["credentials"]["renewals"][0]["match"], "ExpiredToken")
        self.assertEqual(eff["delivery"]["skill"]["name"], "deploy")
        self.assertEqual(eff["models"]["worker"]["spec"]["model"], "gpt-6.2-sol")
        self.assertNotIn("problem", eff["models"]["worker"])
        self.assertIn("doctor: ok", self.t3wf("doctor").stdout)

    def test_the_repo_block_overrides_the_user_config(self):
        self.skill(self.claude / "skills", "unslop", inline=True)
        self.skill(self.claude / "skills", "deploy")
        self.write_config({"passes": ["unslop"], "review": "sol-loop", "delivery": {"mode": "merge"},
                           "models": {"worker": {"model": "claude-opus-5-5", "effort": "high"}}})
        self.agents_md("- Tracker: github-issues\n"
                       "- Review: none\n"
                       "- Passes: none\n"
                       "- Delivery: tag-deploy (see Deploying)\n"
                       "- Deploy skill: `deploy` (`content/v*` when only `training/` changed)\n"
                       "- Worker model: claude-opus-5-5, max effort\n")
        eff = self.slots("--no-t3")
        self.assertEqual((eff["review"]["skill"], eff["review"]["source"]), (None, "AGENTS.md"))
        self.assertEqual((eff["passes"]["skills"], eff["passes"]["source"]), ([], "AGENTS.md"))
        self.assertEqual((eff["delivery"]["mode"], eff["delivery"]["skill"]["name"], eff["delivery"]["source"]),
                         ("tag-deploy", "deploy", "AGENTS.md"))
        self.assertEqual(eff["models"]["worker"]["spec"], {"model": "claude-opus-5-5", "effort": "max"})
        self.assertEqual(eff["models"]["ticket"]["source"], "default")

    def test_the_repo_block_reads_passes_and_credential_renewal(self):
        self.agents_md("- Passes: `paul-voice`, `unslop`, then `pr`\n"
                       "- Credential renewal: `ExpiredToken|sso` -> `aws-sso-login`; `401` -> gh-login\n")
        eff = self.slots("--no-t3")
        self.assertEqual([s["ref"] for s in eff["passes"]["skills"]], ["paul-voice", "unslop", "pr"])
        self.assertEqual([(r["match"], r["skill"]["ref"]) for r in eff["credentials"]["renewals"]],
                         [("ExpiredToken|sso", "aws-sso-login"), ("401", "gh-login")])

    def test_a_repo_that_turns_on_tag_deploy_keeps_the_user_deploy_skill(self):
        self.skill(self.claude / "skills", "ship")
        self.write_config({"delivery": {"mode": "merge", "skill": "ship"}})
        self.agents_md("- Delivery: tag-deploy\n")
        d = self.slots("--no-t3")["delivery"]
        self.assertEqual((d["mode"], d["skill"]["name"]), ("tag-deploy", "ship"))
        self.assertIn("doctor: ok", self.t3wf("doctor", "--no-t3").stdout)

    def test_a_missing_skill_is_skipped_not_an_error(self):
        self.write_config({"passes": ["gone"]})
        r = self.t3wf("slots", "--no-t3")
        self.assertIn("SKIP gone: no longer resolves", r.stdout)


class Doctor(SlotsTestCase):
    def test_reports_a_configured_skill_that_has_been_removed(self):
        path = self.skill(self.claude / "skills", "unslop", inline=True)
        self.write_config({"passes": ["unslop"]})
        self.assertIn("doctor: ok", self.t3wf("doctor", "--no-t3").stdout)
        os.remove(path)
        os.rmdir(path.parent)
        r = self.t3wf("doctor", "--no-t3", ok=False)
        self.assertEqual(r.returncode, 1)
        self.assertIn("PROBLEM: passes[0]: skill unslop no longer resolves", r.stdout)

    def test_a_removed_full_name_names_what_is_installed(self):
        self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        self.write_config({"review": "cbrip:sol-loop"})
        r = self.t3wf("doctor", "--no-t3", ok=False)
        self.assertIn("review: skill cbrip:sol-loop no longer resolves; installed: sol-loop", r.stdout)

    def test_an_ambiguous_bare_name_is_a_warning(self):
        cbrip, _ = self.plugin_skill("cbrip", "sol-loop")
        tools, _ = self.plugin_skill("tools", "sol-loop")
        self.install_plugins(("cbrip", cbrip), ("tools", tools))
        self.write_config({"review": "sol-loop"})
        r = self.t3wf("doctor", "--no-t3")
        self.assertIn("warn: review: sol-loop matches cbrip:sol-loop and tools:sol-loop; record the full name",
                      r.stdout)

    def test_models_are_checked_through_t3(self):
        self.caps(CLAUDE_CAPS)
        self.write_config({"models": {"ticket": {"model": "no-such-model"},
                                      "worker": {"model": "claude-opus-5-5", "effort": "turbo"}}})
        r = self.t3wf("doctor", ok=False)
        self.assertIn("PROBLEM: model ticket: T3 offers no enabled provider with model no-such-model", r.stdout)
        self.assertIn("PROBLEM: model worker: effort turbo is not offered for claude-opus-5-5", r.stdout)

    def test_a_disabled_provider_does_not_count(self):
        self.caps(NO_CLAUDE_CAPS)
        self.write_config({"models": {"worker": {"model": "claude-opus-5-5"}}})
        self.assertIn("no enabled provider", self.t3wf("doctor", ok=False).stdout)

    def test_models_are_unverified_when_t3_is_unreachable(self):
        self.write_config({"models": {"worker": {"model": "claude-opus-5-5"}}})
        r = self.t3wf("doctor")
        self.assertIn("warn: model worker: unverified: T3 unreachable", r.stdout)

    def test_a_removed_deploy_skill_is_reported_in_merge_mode_too(self):
        self.write_config({"delivery": {"mode": "merge", "skill": "ship"}})
        r = self.t3wf("doctor", "--no-t3", ok=False)
        self.assertIn("PROBLEM: delivery: skill ship no longer resolves", r.stdout)
        self.assertIn("delivery: merge; deploy skill SKIP ship: no longer resolves [user]",
                      self.t3wf("slots", "--no-t3").stdout)

    def test_tag_deploy_needs_a_deploy_skill(self):
        self.agents_md("- Delivery: tag-deploy\n")
        r = self.t3wf("doctor", "--no-t3", ok=False)
        self.assertIn("PROBLEM: delivery: tag-deploy names no deploy skill", r.stdout)


class CheckConfig(SlotsTestCase):
    def test_no_config_is_fine(self):
        self.assertEqual(self.t3wf("check-config").stdout.strip(), "config ok")

    def test_a_bad_shape_is_named(self):
        self.write_config({"version": 1, "reviewer": "x", "passes": "unslop",
                           "credentials": [{"match": "(", "skill": "a"}],
                           "delivery": {"mode": "ship"}, "models": {"boss": {"model": "m"}}})
        r = self.t3wf("check-config", ok=False)
        self.assertEqual(r.returncode, 1)
        for text in ("unknown keys ['reviewer']", "passes: must be a list", "match: not a regular expression",
                     "'ship' is not one of merge, tag-deploy", "models.boss: unknown role"):
            self.assertIn(text, r.stdout)

    def test_invalid_json_is_named(self):
        self.write_config("{not json")
        r = self.t3wf("check-config", ok=False)
        self.assertIn("config.json", r.stdout)
        eff = self.slots("--no-t3")
        self.assertTrue(eff["problems"])
        self.assertEqual(eff["delivery"]["mode"], "merge")

    def test_a_bad_repo_block_is_named(self):
        self.agents_md("- Delivery: carrier-pigeon\n")
        r = self.t3wf("check-config", ok=False)
        self.assertIn("AGENTS.md delivery.mode: 'carrier-pigeon' is not one of merge, tag-deploy", r.stdout)
