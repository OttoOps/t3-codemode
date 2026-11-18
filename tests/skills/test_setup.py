import http.server
import json
import platform
import shutil
import socket
import subprocess
import sys
import threading

from test_slots import CLAUDE_CAPS, NO_CLAUDE_CAPS, SlotsTestCase
from test_t3wf import ROOT

SETUP = ROOT / "skills" / "setup-t3-workflow" / "scripts" / "setup_workflow.py"


class Health(http.server.BaseHTTPRequestHandler):
    body = b""

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(self.body)

    def log_message(self, *args):
        pass


class SetupTestCase(SlotsTestCase):
    def setUp(self):
        super().setUp()
        self.bin = self.env.tmp / "bin"
        self.bin.mkdir()
        (self.bin / "git").symlink_to(shutil.which("git"))
        self.closed = socket.socket()
        self.closed.bind(("127.0.0.1", 0))
        self.port = self.closed.getsockname()[1]

    def tearDown(self):
        self.closed.close()
        super().tearDown()

    def serve_health(self, body):
        Health.body = body if isinstance(body, bytes) else json.dumps(body).encode()
        server = http.server.HTTPServer(("127.0.0.1", 0), Health)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        self.port = server.server_address[1]

    def setup(self, *args, stdin=None, ok=True, **extra):
        env = self.env.environ(PATH=str(self.bin), T3CM_PORT=str(self.port))
        env.update(extra)
        r = subprocess.run([sys.executable, str(SETUP), *args, "--repo", str(self.repo)], input=stdin,
                           text=True, capture_output=True, cwd=self.env.tmp, env=env, timeout=120)
        if ok and r.returncode != 0:
            raise AssertionError(f"setup {' '.join(args)} exited {r.returncode}: {r.stderr}")
        return r

    def detect(self, *args):
        return json.loads(self.setup("detect", *args).stdout)

    def write(self, cfg, *args, ok=True):
        return self.setup("write", *args, stdin=json.dumps(cfg), ok=ok)

    def both_sol_loops(self):
        root, plugin_path = self.plugin_skill("cbrip", "sol-loop")
        self.install_plugins(("cbrip", root))
        return plugin_path, self.skill(self.env.home / ".agents" / "skills", "sol-loop")

    def saved(self):
        return json.loads(self.config.read_text())


class Detect(SetupTestCase):
    def test_offers_a_namespaced_and_a_bare_sol_loop(self):
        plugin_path, bare_path = self.both_sol_loops()
        review = self.detect()["slots"]["review"]
        self.assertEqual({c["name"]: c["path"] for c in review["found"]["sol-loop"]},
                         {"sol-loop": str(bare_path), "cbrip:sol-loop": str(plugin_path)})
        self.assertNotIn("suggest", review)

    def test_suggests_tstacks_sol_loop_when_none_is_installed(self):
        review = self.detect()["slots"]["review"]
        self.assertEqual((review["found"], review["missing"]), ({}, ["sol-loop"]))
        self.assertIn("https://github.com/t3dotgg/tstack", review["suggest"])

    def test_a_slash_only_pass_is_marked_inline(self):
        self.skill(self.claude / "skills", "unslop", inline=True)
        self.skill(self.claude / "skills", "pr")
        passes = self.detect()["slots"]["passes"]
        self.assertEqual(passes["found"]["unslop"][0]["invoke"], "inline")
        self.assertEqual(passes["found"]["pr"][0]["invoke"], "skill")

    def test_voice_skills_listed_in_the_session_are_pass_candidates(self):
        root, _ = self.plugin_skill("writers", "ann-voice")
        self.install_plugins(("writers", root))
        passes = self.detect("--skills", "writers:ann-voice, pr")["slots"]["passes"]
        self.assertEqual([c["name"] for c in passes["found"]["writers:ann-voice"]], ["writers:ann-voice"])

    def test_a_missing_full_name_is_missing_with_what_is_installed_instead(self):
        bare = self.skill(self.env.home / ".agents" / "skills", "sol-loop")
        self.write_config({"review": "cbrip:sol-loop"})
        review = self.detect()["slots"]["review"]
        self.assertNotIn("cbrip:sol-loop", review["found"])
        self.assertIn("cbrip:sol-loop", review["missing"])
        self.assertEqual(review["installedInstead"]["cbrip:sol-loop"][0]["path"], str(bare))
        self.assertEqual(review["found"]["sol-loop"][0]["path"], str(bare))

    def test_a_name_that_looks_like_an_option_is_still_resolved(self):
        self.write_config({"review": "-review"})
        self.assertIn("-review", self.detect()["slots"]["review"]["missing"])

    def test_repo_problems_are_kept_apart_from_the_user_config(self):
        self.write_config({"passes": []})
        self.agents_md("- Delivery: carrier-pigeon\n")
        config = self.detect()["config"]
        self.assertEqual(config["problems"], [])
        self.assertTrue(any("carrier-pigeon" in p for p in config["repoProblems"]))

    def test_configured_skills_are_searched_too(self):
        self.skill(self.claude / "skills", "ship")
        self.write_config({"delivery": {"mode": "tag-deploy", "skill": "ship"}})
        delivery = self.detect()["slots"]["delivery"]
        self.assertEqual(delivery["configured"], ["ship"])
        self.assertIn("ship", delivery["found"])
        self.assertEqual(delivery["missing"], ["deploy"])

    def test_lists_enabled_providers_and_their_models(self):
        caps = {"providers": NO_CLAUDE_CAPS["providers"] + [
            {"providerInstanceId": "gemini", "displayName": "Gemini", "constraints": [], "models": [
                {"id": "g-1", "options": [{"id": "thinking_level", "values": ["low", "high"], "default": "low"}]},
                {"id": "g-2"}]}]}
        self.caps(caps)
        p = self.detect()["providers"]
        self.assertEqual(p["disabled"], ["claudeAgent"])
        self.assertEqual([e["provider"] for e in p["enabled"]], ["codex", "gemini"])
        self.assertEqual(p["enabled"][0]["models"], ["gpt-6.2-sol (effort low|medium|high)"])
        self.assertEqual(p["enabled"][1]["models"], ["g-1 (effort low|high; default low)", "g-2"])

    def test_names_the_error_when_t3_does_not_answer(self):
        self.assertIn("T3 unreachable", self.detect()["providers"]["error"])

    def test_summary_shows_the_current_settings(self):
        self.caps(CLAUDE_CAPS)
        d = self.detect()
        self.assertEqual((d["config"]["exists"], d["config"]["path"]), (False, str(self.config)))
        self.assertIn("review: none (no reviewer installed; suggest sol-loop from https://github.com/t3dotgg/tstack)"
                      " [default]", d["summary"])

    def test_platform_reports_systemd_user_units(self):
        plat = self.detect()["platform"]
        self.assertEqual(plat["system"], platform.system().lower())
        self.assertEqual(plat["systemdUser"], plat["system"] == "linux")


class T3cm(SetupTestCase):
    def test_missing_and_stopped(self):
        t = self.detect()["t3cm"]
        self.assertEqual((t["binary"], t["running"], t["connected"]), (None, False, False))
        self.assertEqual(t["url"], f"http://127.0.0.1:{self.port}/")

    def test_running_and_connected(self):
        (self.bin / "t3cm").write_text("#!/bin/sh\n")
        (self.bin / "t3cm").chmod(0o755)
        self.serve_health({"version": "9.9.9", "t3": {"connected": True}})
        t = self.detect()["t3cm"]
        self.assertEqual((t["binary"], t["running"], t["connected"], t["version"]),
                         (str(self.bin / "t3cm"), True, True, "9.9.9"))

    def test_another_service_on_the_port_is_not_t3cm(self):
        for body in ([], {}, {"t3": {"connected": "false"}}):
            self.serve_health(body)
            t = self.detect()["t3cm"]
            self.assertEqual((t["running"], t["error"]), (False, "/healthz did not answer like t3cm"), body)

    def test_the_probe_skips_the_proxy(self):
        self.serve_health({"t3": {"connected": True}})
        t = json.loads(self.setup("detect", http_proxy="http://127.0.0.1:9", HTTP_PROXY="http://127.0.0.1:9",
                                  no_proxy="", NO_PROXY="").stdout)["t3cm"]
        self.assertTrue(t["running"])

    def test_a_bad_port_setting_is_reported(self):
        r = self.setup("detect", T3CM_PORT="abc")
        t = json.loads(r.stdout)["t3cm"]
        self.assertEqual((t["running"], t["error"]), (False, "T3CM_PORT='abc' is not a port number"))


class Write(SetupTestCase):
    def test_records_the_namespaced_choice(self):
        plugin_path, _ = self.both_sol_loops()
        r = self.write({"review": "cbrip:sol-loop"})
        self.assertEqual(self.saved(), {"review": "cbrip:sol-loop", "version": 1})
        self.assertIn(f"wrote {self.config}", r.stdout)
        self.assertIn("review: cbrip:sol-loop (Skill tool) [user]", r.stdout)

    def test_records_the_bare_choice(self):
        _, bare_path = self.both_sol_loops()
        self.write({"review": "sol-loop"}, "--skills", "cbrip:sol-loop")
        r = self.slots("--no-t3", "--skills", "cbrip:sol-loop")["review"]["skill"]
        self.assertEqual((r["status"], r["path"]), ("ok", str(bare_path)))

    def test_refuses_a_bare_name_that_matches_several_skills(self):
        cbrip, _ = self.plugin_skill("cbrip", "sol-loop")
        tools, _ = self.plugin_skill("tools", "sol-loop")
        self.install_plugins(("cbrip", cbrip), ("tools", tools))
        self.write_config({"review": None})
        r = self.write({"review": "sol-loop"}, ok=False)
        self.assertEqual(r.returncode, 1)
        self.assertIn("review: sol-loop matches cbrip:sol-loop and tools:sol-loop; record one full name", r.stderr)
        self.assertEqual(self.saved(), {"review": None})

    def test_a_kept_ambiguous_name_warns_without_blocking_another_slot(self):
        cbrip, _ = self.plugin_skill("cbrip", "sol-loop")
        tools, _ = self.plugin_skill("tools", "sol-loop")
        self.install_plugins(("cbrip", cbrip), ("tools", tools))
        self.write_config({"review": "sol-loop"})
        r = self.write({"models": {"worker": {"model": "claude-opus-5-5"}}})
        self.assertIn("warn: review: sol-loop matches cbrip:sol-loop and tools:sol-loop", r.stdout)
        self.assertEqual(self.saved()["models"], {"worker": {"model": "claude-opus-5-5"}})

    def test_a_slash_only_skill_is_recorded_and_runs_inline(self):
        path = self.skill(self.claude / "skills", "unslop", inline=True)
        r = self.write({"passes": ["unslop"]})
        self.assertEqual(self.saved()["passes"], ["unslop"])
        self.assertIn(f"passes: unslop (inline: read {path} and apply it) [user]", r.stdout)

    def test_merges_into_the_current_config(self):
        self.write_config({"version": 1, "passes": ["pr"], "delivery": {"mode": "merge"},
                           "models": {"ticket": {"model": "claude-opus-5-5", "effort": "max"}}})
        self.write({"models": {"worker": {"provider": "codex", "model": "gpt-6.2-sol", "effort": "high"}}},
                   "--unset", "passes")
        self.assertEqual(self.saved(), {"version": 1, "delivery": {"mode": "merge"},
                                        "models": {"ticket": {"model": "claude-opus-5-5", "effort": "max"},
                                                   "worker": {"provider": "codex", "model": "gpt-6.2-sol",
                                                              "effort": "high"}}})
        self.write({}, "--unset", "models.ticket")
        self.assertEqual(list(self.saved()["models"]), ["worker"])

    def test_unset_rejects_a_target_that_is_not_a_slot_or_model_role(self):
        self.write_config({"version": 1, "passes": ["pr"]})
        r = self.write({}, "--unset", "passes.worker", ok=False)
        self.assertIn("--unset passes.worker: name a slot", r.stderr)
        self.assertEqual(self.saved()["passes"], ["pr"])

    def test_a_kept_pass_is_not_blamed_for_a_review_that_is_unset(self):
        cbrip, _ = self.plugin_skill("cbrip", "sol-loop")
        tools, _ = self.plugin_skill("tools", "sol-loop")
        self.install_plugins(("cbrip", cbrip), ("tools", tools))
        self.write_config({"passes": ["sol-loop"]})
        r = self.write({"review": "sol-loop"}, "--unset", "review")
        self.assertIn("warn: passes[0]: sol-loop matches", r.stdout)

    def test_a_slot_set_and_unset_in_one_write_is_dropped(self):
        self.write({"passes": True}, "--unset", "passes")
        self.assertNotIn("passes", self.saved())

    def test_tag_deploy_needs_a_deploy_skill(self):
        r = self.write({"delivery": {"mode": "tag-deploy"}}, ok=False)
        self.assertIn("delivery: tag-deploy needs a deploy skill", r.stderr)
        self.write({"delivery": {"mode": "tag-deploy", "skill": "ship"}})
        self.assertEqual(self.saved()["delivery"], {"mode": "tag-deploy", "skill": "ship"})

    def test_tag_deploy_that_is_also_unset_is_dropped(self):
        self.write({"delivery": {"mode": "tag-deploy"}}, "--unset", "delivery")
        self.assertNotIn("delivery", self.saved())

    def test_a_model_role_needs_a_model(self):
        self.write_config({"models": {"worker": {"model": "claude-opus-5-5"}}})
        r = self.write({"models": {"worker": {"effort": "high"}}}, ok=False)
        self.assertIn("models.worker: names no model", r.stderr)

    def test_another_config_version_is_left_alone(self):
        self.write_config({"version": 2, "review": "sol-loop"})
        r = self.write({"passes": []}, ok=False)
        self.assertIn("is config version 2; pass --replace", r.stderr)
        self.assertEqual(self.saved()["version"], 2)

    def test_a_symlinked_config_is_written_through(self):
        target = self.env.tmp / "dotfiles" / "t3wf.json"
        target.parent.mkdir()
        target.write_text(json.dumps({"passes": ["pr"]}))
        self.config.parent.mkdir(parents=True)
        self.config.symlink_to(target)
        self.write({"review": None})
        self.assertTrue(self.config.is_symlink())
        self.assertEqual(json.loads(target.read_text()), {"version": 1, "passes": ["pr"], "review": None})

    def test_a_config_dir_that_cannot_be_made_is_not_written(self):
        self.config.parent.parent.mkdir(parents=True, exist_ok=True)
        self.config.parent.write_text("")
        r = self.write({"passes": []}, "--replace", ok=False)
        self.assertIn("not written", r.stderr)

    def test_a_failed_save_leaves_no_staged_file(self):
        self.config.mkdir(parents=True)
        (self.config / "occupied").write_text("")
        r = self.write({"passes": []}, "--replace", ok=False)
        self.assertIn("not written", r.stderr)
        self.assertEqual(sorted(p.name for p in self.config.parent.iterdir()), ["config.json"])

    def test_review_none_turns_review_off(self):
        self.both_sol_loops()
        r = self.write({"review": None})
        self.assertIsNone(self.saved()["review"])
        self.assertIn("review: none (none configured) [user]", r.stdout)

    def test_a_bad_setting_is_not_written(self):
        r = self.write({"delivery": {"mode": "ship"}}, ok=False)
        self.assertEqual(r.returncode, 1)
        self.assertIn("'ship' is not one of merge, tag-deploy", r.stderr)
        self.assertFalse(self.config.exists())

    def test_a_skill_not_yet_installed_is_written_with_a_warning(self):
        r = self.write({"review": "sol-loop"})
        self.assertEqual(self.saved()["review"], "sol-loop")
        self.assertIn("warn: review: sol-loop is not installed yet", r.stdout)

    def test_a_broken_config_needs_replace(self):
        self.write_config("{not json")
        r = self.write({"passes": []}, ok=False)
        self.assertIn("--replace", r.stderr)
        self.write({"passes": []}, "--replace")
        self.assertEqual(self.saved(), {"passes": [], "version": 1})
