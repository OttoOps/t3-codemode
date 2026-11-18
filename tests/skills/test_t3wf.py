import importlib.util
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[2]
T3WF = ROOT / "skills" / "t3-workflow" / "scripts" / "t3wf.py"

PARENT = "thread-parent"
CHILD = "thread-child"
T0 = "2026-01-05T12:00:00Z"


class Env:
    def __init__(self, tmp):
        self.tmp = pathlib.Path(tmp)
        self.home = self.tmp / "home"
        self.state = self.tmp / "state"
        self.config = self.tmp / "config"
        self.fake = self.tmp / "fake-t3.json"
        self.sysctl_log = self.tmp / "systemctl.log"
        self.sysctl = self.tmp / "systemctl"
        self.home.mkdir()
        self.fail_enable = self.tmp / "fail-enable"
        self.sysctl.write_text(
            "#!/bin/sh\n"
            f'echo "$@" >> {shlex.quote(str(self.sysctl_log))}\n'
            f'if [ "$2" = enable ] && [ -e {shlex.quote(str(self.fail_enable))} ]; then exit 1; fi\n')
        self.sysctl.chmod(0o755)
        self.threads({
            PARENT: {"threadId": PARENT, "status": "idle", "updatedAt": T0, "link": "t3://parent"},
            CHILD: {"threadId": CHILD, "status": "idle", "updatedAt": T0, "link": "t3://child"},
        })

    def threads(self, threads):
        data = json.loads(self.fake.read_text()) if self.fake.exists() else {}
        data["threads"] = threads
        self.fake.write_text(json.dumps(data))

    def sent(self):
        return json.loads(self.fake.read_text()).get("sent", [])

    def environ(self, **extra):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("T3WF_", "T3CODE_", "XDG_"))}
        env.update(HOME=str(self.home), XDG_STATE_HOME=str(self.state),
                   XDG_CONFIG_HOME=str(self.config), T3WF_FAKE_T3=str(self.fake),
                   T3WF_NO_LAZY="1", T3WF_SYSTEMCTL=str(self.sysctl), T3WF_TZ="UTC")
        env.update(extra)
        return env

    def run(self, *args, stdin=None, ok=True, **extra):
        r = subprocess.run([sys.executable, str(T3WF), *args], input=stdin, text=True, cwd=self.tmp,
                           capture_output=True, env=self.environ(**extra), timeout=60)
        if ok and r.returncode != 0:
            raise AssertionError(f"t3wf {' '.join(args)} exited {r.returncode}: {r.stderr}")
        return r

    def registry(self, store=None):
        store = store or self.state / "t3-workflow" / "v2"
        return json.loads((store / "links.json").read_text())

    def link(self, parent=PARENT, child=CHILD, label="w1", **extra):
        self.run("init", "--role", "ticket", "--self", parent, **extra)
        block = self.run("open-child", "--role", "worker", "--label", label, "--self", parent, **extra).stdout
        join = next(line for line in block.splitlines() if line.startswith("t3wf join "))
        return self.run(*shlex.split(join)[1:], "--self", child, **extra)


class T3wfTestCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.env = Env(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()


class CoreCommands(T3wfTestCase):
    def test_check_config_passes(self):
        self.assertEqual(self.env.run("check-config").stdout.strip(), "config ok")

    def test_join_binds_the_link_from_the_open_child_block(self):
        out = self.env.link().stdout
        self.assertIn(f"Joined {PARENT} as w1", out)
        reg = self.env.registry()
        ln = reg["links"][CHILD]
        self.assertEqual((ln["parent"], ln["role"], ln["parentRole"]), (PARENT, "worker", "ticket"))
        self.assertEqual(ln["params"], {"down": 45, "miss": 2, "stall": 240})
        self.assertEqual(ln["link"], "t3://child")
        self.assertFalse(ln["closed"])
        self.assertEqual(reg["pending"], {})

    def test_open_child_block_names_the_unnamespaced_skill(self):
        self.env.run("init", "--role", "ticket", "--self", PARENT)
        block = self.env.run("open-child", "--role", "worker", "--label", "w1", "--self", PARENT).stdout
        self.assertIn("invoke the t3-workflow skill", block)
        self.assertNotIn("cbrip", block)

    def test_protocol_1_is_refused(self):
        r = self.env.run("join", "--parent", PARENT, "--label", "w1", "--role", "worker",
                         "--protocol", "1", "--self", CHILD, ok=False)
        self.assertEqual(r.returncode, 2)
        self.assertIn("protocol 1 is no longer supported", r.stderr)
        r = self.env.run("fired", "--self", CHILD, ok=False)
        self.assertEqual(r.returncode, 2)
        self.assertFalse((ROOT / "skills" / "t3-workflow" / "scripts" / "t3wf_v1.py").exists())

    def test_note_is_stored_without_a_wake_and_reaches_the_inbox(self):
        self.env.link()
        out = self.env.run("send", "--kind", "note", "--body", "tests green", "--self", CHILD).stdout
        self.assertIn("noted m1: no wake", out)
        self.assertEqual(self.env.sent(), [])
        inbox = self.env.run("inbox", "--self", PARENT).stdout
        self.assertIn("w1 note: tests green", inbox)
        self.assertEqual(self.env.run("inbox", "--self", PARENT).stdout.strip(), "No unread notes.")

    def test_event_wakes_the_parent_with_a_header_and_the_digest(self):
        self.env.link()
        self.env.run("send", "--kind", "note", "--body", "built", "--self", CHILD)
        out = self.env.run("send", "--kind", "event", "--body", "PR opened", "--self", CHILD).stdout
        self.assertIn("delivered m2", out)
        [msg] = self.env.sent()
        self.assertEqual(msg["threadId"], PARENT)
        self.assertTrue(msg["message"].startswith(f"[t3wf2 event from=w1 id={CHILD} msg=m2]\nPR opened"))
        self.assertIn("Notes since your last wake", msg["message"])
        self.assertIn("built [m1]", msg["message"])

    def test_long_body_is_capped_and_show_prints_it_whole(self):
        self.env.link()
        body = "x" * 2000
        self.env.run("send", "--kind", "event", "--body", "-", "--self", CHILD, stdin=body)
        [msg] = self.env.sent()
        self.assertIn("500 more chars: `t3wf show m1`", msg["message"])
        shown = self.env.run("show", "m1").stdout
        self.assertIn(body, shown)

    def test_gate_opens_and_clears(self):
        self.env.link()
        self.env.run("send", "--kind", "gate", "--gate", "merge", "--body", "reply merge", "--self", CHILD)
        self.assertEqual(self.env.registry()["links"][CHILD]["gate"], "merge")
        self.assertIn("gate=merge", self.env.sent()[0]["message"].splitlines()[0])
        self.env.run("gate", "--clear", "--self", CHILD)
        self.assertIsNone(self.env.registry()["links"][CHILD]["gate"])

    def test_reply_reaches_the_child_by_label(self):
        self.env.link()
        self.env.run("reply", "--to", "w1", "--body", "Merged #4 at abc. Hand off.", "--self", PARENT)
        [msg] = self.env.sent()
        self.assertEqual(msg["threadId"], CHILD)
        self.assertTrue(msg["message"].startswith(f"[t3wf2 reply from=parent id={PARENT}"))

    def test_handoff_closes_the_link(self):
        self.env.link()
        out = self.env.run("send", "--kind", "handoff", "--body", "done", "--self", CHILD).stdout
        self.assertIn("The link is closed", out)
        self.assertTrue(self.env.registry()["links"][CHILD]["closed"])
        r = self.env.run("send", "--kind", "note", "--body", "late", "--self", CHILD, ok=False)
        self.assertIn("the link to your parent is closed", r.stderr)
        self.assertIn("Every child link is closed.", self.env.run("sweep", "--self", PARENT).stdout)

    def test_board_and_sweep_name_open_links(self):
        self.env.link()
        board = self.env.run("board", "--self", PARENT).stdout
        self.assertIn("w1 [worker] idle", board)
        self.assertIn("t3://child", board)
        self.assertIn("Links still open: w1", self.env.run("sweep", "--self", PARENT).stdout)

    def test_close_and_adopt(self):
        self.env.link()
        self.env.run("init", "--role", "ticket", "--self", "thread-new")
        out = self.env.run("adopt", "--from", PARENT, "--self", "thread-new").stdout
        self.assertIn("Adopted: w1", out)
        self.assertEqual(self.env.registry()["links"][CHILD]["parent"], "thread-new")
        self.env.run("close", "--link", CHILD, "--self", "thread-new")
        self.assertEqual(self.env.registry()["links"][CHILD]["closedHow"], "closed-by-parent")

    def test_tick_nudges_an_idle_child_then_escalates(self):
        self.env.link()
        later = "2026-01-05T13:00:00Z"
        out = self.env.run("--now", later, "tick").stdout
        self.assertIn("nudge w1 (1/2)", out)
        self.assertTrue(self.env.sent()[-1]["message"].startswith("[t3wf2 nudge from=watcher"))
        self.assertEqual(self.env.sent()[-1]["threadId"], CHILD)
        self.assertIn("no action", self.env.run("--now", "2026-01-05T13:10:00Z", "tick").stdout)
        self.assertIn("nudge w1 (2/2)", self.env.run("--now", "2026-01-05T13:50:00Z", "tick").stdout)
        out = self.env.run("--now", "2026-01-05T14:40:00Z", "tick").stdout
        self.assertIn("escalate w1", out)
        esc = self.env.sent()[-1]
        self.assertEqual(esc["threadId"], PARENT)
        self.assertTrue(esc["message"].startswith(f"[t3wf2 escalate from=w1 id={CHILD}"))

    def test_tick_leaves_a_gated_child_alone(self):
        self.env.link()
        self.env.run("gate", "--open", "review", "--self", CHILD)
        self.assertIn("no action", self.env.run("--now", "2026-01-05T15:00:00Z", "tick").stdout)

    def test_daemon_install_writes_the_t3wf_tick_units(self):
        self.env.run("daemon", "install")
        units = self.env.config / "systemd" / "user"
        self.assertIn("ExecStart=", (units / "t3wf-tick.service").read_text())
        self.assertIn("OnUnitActiveSec=2min", (units / "t3wf-tick.timer").read_text())
        shim = (self.env.state / "t3-workflow" / "v2" / "bin" / "t3wf-tick").read_text()
        self.assertIn(str(T3WF), shim)
        self.assertIn(f"export T3WF_STATE_DIR={self.env.state / 't3-workflow'}", shim)
        self.assertIn("--user enable --now t3wf-tick.timer", self.env.sysctl_log.read_text())


class Migration(T3wfTestCase):
    def legacy_root(self):
        return self.env.home / ".claude" / "cbrip" / "t3-workflow"

    def make_legacy_state(self):
        legacy = {"T3WF_STATE_DIR": str(self.legacy_root())}
        self.env.link(**legacy)
        self.env.run("send", "--kind", "note", "--body", "built at d4db97c", "--self", CHILD, **legacy)
        self.env.run("send", "--kind", "gate", "--gate", "merge", "--body", "reply merge", "--self", CHILD,
                     **legacy)
        units = self.env.config / "systemd" / "user"
        units.mkdir(parents=True)
        (units / "cbrip-t3wf-tick.timer").write_text("[Timer]\n")
        (units / "cbrip-t3wf-tick.service").write_text("[Service]\n")
        (self.legacy_root() / "v2" / "bin").mkdir()
        (self.legacy_root() / "v2" / "bin" / "t3wf-tick").write_text("#!/bin/sh\n")
        return self.env.registry(self.legacy_root() / "v2")

    def test_open_links_migrate_intact_to_the_new_state_folder(self):
        before = self.make_legacy_state()
        self.assertFalse(before["links"][CHILD]["closed"])

        r = self.env.run("board", "--self", PARENT)
        self.assertIn("moved the link registry", r.stderr)
        self.assertIn("replaced cbrip-t3wf-tick.timer with t3wf-tick.timer", r.stderr)

        new = self.env.state / "t3-workflow" / "v2"
        self.assertEqual(self.env.registry(new), before)
        self.assertIn("w1 [worker]", r.stdout)
        self.assertIn("gate=merge", r.stdout)
        self.assertIn("built at d4db97c", self.env.run("show", "m1").stdout)

        old = self.legacy_root() / "v2"
        self.assertTrue(old.is_symlink())
        self.assertEqual(old.resolve(), new.resolve())
        self.assertIn(str(T3WF), (new / "bin" / "t3wf-tick").read_text())

        units = self.env.config / "systemd" / "user"
        self.assertFalse((units / "cbrip-t3wf-tick.timer").exists())
        self.assertFalse((units / "cbrip-t3wf-tick.service").exists())
        self.assertTrue((units / "t3wf-tick.timer").exists())
        log = self.env.sysctl_log.read_text()
        self.assertIn("--user disable --now cbrip-t3wf-tick.timer", log)
        self.assertIn("--user enable --now t3wf-tick.timer", log)

        self.env.run("gate", "--clear", "--self", CHILD)
        self.env.run("send", "--kind", "handoff", "--body", "done", "--self", CHILD)
        self.assertTrue(self.env.registry(new)["links"][CHILD]["closed"])

    def test_the_new_lock_is_the_old_lock_file(self):
        self.make_legacy_state()
        inode = (self.legacy_root() / "v2" / "lock").stat().st_ino
        self.env.run("board", "--self", PARENT)
        self.assertEqual((self.env.state / "t3-workflow" / "v2" / "lock").stat().st_ino, inode)

    def test_a_failed_timer_swap_keeps_the_old_timer_and_retries(self):
        self.make_legacy_state()
        units = self.env.config / "systemd" / "user"
        self.env.fail_enable.touch()
        r = self.env.run("board", "--self", PARENT)
        self.assertIn("migrate_timer failed", r.stderr)
        self.assertTrue((units / "cbrip-t3wf-tick.timer").exists())
        self.assertNotIn("disable", self.env.sysctl_log.read_text())
        self.env.fail_enable.unlink()
        r = self.env.run("board", "--self", PARENT)
        self.assertIn("replaced cbrip-t3wf-tick.timer with t3wf-tick.timer", r.stderr)
        self.assertFalse((units / "cbrip-t3wf-tick.timer").exists())

    def test_a_thread_on_the_old_path_shares_the_migrated_registry(self):
        self.make_legacy_state()
        self.env.run("board", "--self", PARENT)
        self.env.run("send", "--kind", "note", "--body", "via old path", "--self", CHILD,
                     T3WF_STATE_DIR=str(self.legacy_root()))
        inbox = self.env.run("inbox", "--self", PARENT).stdout
        self.assertIn("via old path", inbox)

    def test_a_failed_rename_stops_the_command_and_retries_later(self):
        before = self.make_legacy_state()
        blocker = self.legacy_root() / "v2.migrated" / "stale"
        blocker.parent.mkdir()
        blocker.write_text("x")
        r = self.env.run("board", "--self", PARENT, ok=False)
        self.assertEqual(r.returncode, 2)
        self.assertFalse((self.env.state / "t3-workflow" / "v2" / "links.json").exists())
        blocker.unlink()
        blocker.parent.rmdir()
        self.assertIn("moved the link registry", self.env.run("board", "--self", PARENT).stderr)
        self.assertEqual(self.env.registry(), before)

    def test_a_failed_symlink_rolls_back_so_the_next_run_retries(self):
        before = self.make_legacy_state()
        spec = importlib.util.spec_from_file_location("t3wf_under_test", T3WF)
        t3wf = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(t3wf)
        with mock.patch.dict(os.environ, self.env.environ(), clear=True), \
                mock.patch("os.symlink", side_effect=OSError("injected")):
            with self.assertRaises(OSError):
                t3wf.migrate_store()
        old = self.legacy_root() / "v2"
        self.assertFalse(old.is_symlink())
        self.assertEqual(self.env.registry(old), before)
        self.assertFalse((self.env.state / "t3-workflow" / "v2" / "links.json").exists())
        self.assertIn("moved the link registry", self.env.run("board", "--self", PARENT).stderr)
        self.assertEqual(self.env.registry(), before)

    def test_a_relative_state_dir_gets_an_absolute_symlink(self):
        before = self.make_legacy_state()
        (self.env.tmp / "nested" / "dir").mkdir(parents=True)
        (self.env.tmp / "alias").symlink_to(self.env.tmp / "nested" / "dir")
        self.env.run("board", "--self", PARENT, T3WF_STATE_DIR="alias/../rel-state")
        new = self.env.tmp / "nested" / "rel-state" / "v2"
        self.assertEqual((self.legacy_root() / "v2").resolve(), new.resolve())
        self.assertEqual(self.env.registry(self.legacy_root() / "v2"), before)

    def test_migration_runs_once(self):
        self.make_legacy_state()
        self.env.run("board", "--self", PARENT)
        r = self.env.run("board", "--self", PARENT)
        self.assertNotIn("moved", r.stderr)
        self.assertNotIn("replaced", r.stderr)

    def test_an_existing_registry_is_never_overwritten(self):
        self.env.run("init", "--role", "overseer", "--self", "thread-other")
        before = self.make_legacy_state()
        r = self.env.run("board", "--self", "thread-other")
        self.assertNotIn("moved", r.stderr)
        old = self.legacy_root() / "v2"
        self.assertFalse(old.is_symlink())
        self.assertEqual(self.env.registry(old), before)
        self.assertNotIn(CHILD, self.env.registry()["links"])
        self.assertNotIn("replaced", r.stderr)
        self.assertTrue((self.env.config / "systemd" / "user" / "cbrip-t3wf-tick.timer").exists())

    def test_a_failed_copy_stops_the_command_and_retries_later(self):
        before = self.make_legacy_state()
        body = self.legacy_root() / "v2" / "msgs" / "m1.txt"
        body.chmod(0)
        try:
            r = self.env.run("board", "--self", PARENT, ok=False)
        finally:
            body.chmod(0o644)
        self.assertEqual(r.returncode, 2)
        self.assertIn("the registry is still at", r.stderr)
        self.assertFalse((self.env.state / "t3-workflow" / "v2" / "links.json").exists())
        self.assertFalse((self.legacy_root() / "v2").is_symlink())
        r = self.env.run("board", "--self", PARENT)
        self.assertIn("moved the link registry", r.stderr)
        self.assertEqual(self.env.registry(), before)

    def test_no_legacy_state_means_no_migration(self):
        r = self.env.run("init", "--role", "ticket", "--self", PARENT)
        self.assertEqual(r.stderr, "")
        self.assertFalse((self.env.home / ".claude").exists())
        self.assertFalse(self.env.sysctl_log.exists())


if __name__ == "__main__":
    unittest.main()
