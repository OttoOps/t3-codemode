#!/usr/bin/env python3
"""t3wf protocol 2: the t3-workflow link protocol, run outside the model.

One registry per machine holds every parent and child link, every message body,
and each thread's code-mode variables. A watcher (`t3wf tick`, run every few
minutes by a systemd user timer, or lazily by any t3wf command) reads each
child's live state from T3 and does the timekeeping: nudges, escalations and
delivery. Threads never schedule timers or check in. A message to a parent is
one command; routine progress is stored as a note and reaches the parent as a
short digest at its next wake, never as a wake of its own. Big bodies travel as
a capped excerpt plus a handle (`t3wf show <handle>`).
"""
import argparse
import contextlib
import datetime as dt
import fcntl
import io
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
SKILL_DIR = os.path.dirname(HERE)
SKILLS_DIR = os.path.dirname(SKILL_DIR)
sys.path.insert(0, HERE)

PROTOCOL = 2
ROLE_SKILL = {"overseer": "t3-overseer", "ticket": "t3-ticket", "worker": "t3-worker"}
UP_KINDS = ("note", "event", "gate", "blocker", "report", "handoff", "closed")
CLOSING = ("handoff", "closed")
PARAMS = ("down", "miss", "stall")
THREAD_ENV = "T3CODE_THREAD_ID"
ACTIVE = ("preparing", "queued", "starting", "running")
ENDED_BADLY = ("failed", "interrupted", "cancelled")
UNIT = "t3wf-tick"
LEGACY_UNIT = "cbrip-t3wf-tick"


class Fail(Exception):
    pass


# ---------------------------------------------------------------- time

_NOW = None


def now():
    if _NOW:
        return _NOW
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0)


def iso(t):
    return t.strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_ts(s):
    if not s:
        return None
    s = re.sub(r"\.\d+Z$", "Z", s)
    return dt.datetime.strptime(s, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=dt.timezone.utc)


def minutes_since(s):
    t = parse_ts(s)
    return None if t is None else (now() - t).total_seconds() / 60.0


def clock(s):
    t = parse_ts(s)
    if t is None:
        return "-"
    try:
        tz = os.environ.get("T3WF_TZ")
        if tz:
            from zoneinfo import ZoneInfo
            local = t.astimezone(ZoneInfo(tz))
        else:
            local = t.astimezone()
        return local.strftime("%H:%M %Z")
    except Exception:
        return t.strftime("%H:%MZ")


# ---------------------------------------------------------------- config

def read_json(path):
    with open(path) as f:
        return json.load(f)


def config():
    return read_json(os.path.join(SKILL_DIR, "defaults.json"))["v2"]


def role_config(role):
    if role not in ROLE_SKILL:
        raise Fail(f"unknown role {role!r}; roles are {', '.join(ROLE_SKILL)}")
    path = os.path.join(SKILLS_DIR, ROLE_SKILL[role], "t3-role.json")
    cfg = read_json(path)
    if cfg.get("role") != role:
        raise Fail(f"{path} declares role {cfg.get('role')!r}, expected {role!r}")
    return cfg


def link_params(parent_role, child_role):
    base = dict(config()["link"])
    children = role_config(parent_role).get("children", {})
    if child_role not in children:
        raise Fail(f"role {parent_role!r} opens no links to {child_role!r} (t3-role.json 'children')")
    base.update({k: v for k, v in children[child_role].items() if k in PARAMS})
    problems = validate_params(base)
    if problems:
        raise Fail(f"{parent_role} -> {child_role} link numbers are invalid: " + "; ".join(problems))
    return base


def link_policy(ln):
    """Which kinds wake the parent, and how long a body may be, for this link: defaults, then the parent role's override."""
    cfg = config()
    wake, cap = list(cfg["wake"]), cfg["bodyCap"]
    with contextlib.suppress(OSError, ValueError, Fail):
        child = role_config(ln.get("parentRole") or "").get("children", {}).get(ln["role"], {})
        wake = child.get("wake", wake)
        cap = child.get("bodyCap", cap)
    return wake, cap


def validate_params(p):
    problems = [f"{k} must be a positive integer, got {p.get(k)!r}" for k in PARAMS
                if not isinstance(p.get(k), int) or p[k] < 1]
    if not problems and p["stall"] <= p["down"]:
        problems.append(f"stall ({p['stall']}) must exceed down ({p['down']})")
    return problems


def check_config():
    problems = []
    try:
        c = config()
        if c.get("protocol") != PROTOCOL:
            problems.append(f"defaults.json v2.protocol {c.get('protocol')} != {PROTOCOL}")
        problems += [f"defaults: {x}" for x in validate_params(dict(c["link"]))]
        bad = set(c["wake"]) - set(UP_KINDS) - {"escalate", "reply"}
        if bad:
            problems.append(f"defaults: unknown wake kinds {sorted(bad)}")
        if "note" in c["wake"]:
            problems.append("defaults: a note never wakes its parent; take it out of wake")
    except (OSError, KeyError, ValueError) as e:
        return [f"defaults.json: {e}"]
    for role in ROLE_SKILL:
        try:
            cfg = role_config(role)
            for child, over in cfg.get("children", {}).items():
                link_params(role, child)
                bad = set(over.get("wake", [])) - set(UP_KINDS) - {"escalate", "reply"}
                if bad:
                    problems.append(f"{role} -> {child}: unknown wake kinds {sorted(bad)}")
                if "bodyCap" in over and (not isinstance(over["bodyCap"], int) or over["bodyCap"] < 200):
                    problems.append(f"{role} -> {child}: bodyCap must be an integer of at least 200")
        except (OSError, ValueError, Fail) as e:
            problems.append(f"{role}: {e}")
    return problems


# ---------------------------------------------------------------- store

def state_root():
    if os.environ.get("T3WF_STATE_DIR"):
        return os.path.realpath(os.environ["T3WF_STATE_DIR"])
    base = os.environ.get("XDG_STATE_HOME") or os.path.join(os.path.expanduser("~"), ".local", "state")
    return os.path.realpath(os.path.join(base, "t3-workflow"))


def store_dir():
    return os.path.join(state_root(), "v2")


def legacy_store_dir():
    return os.path.join(os.path.expanduser("~"), ".claude", "cbrip", "t3-workflow", "v2")


def safe(s):
    return re.sub(r"[^A-Za-z0-9._-]", "_", s)


def atomic_write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".t3wf-")
    with os.fdopen(fd, "w") as f:
        f.write(text)
    os.replace(tmp, path)


class Store:
    """The machine's link registry. Hold it with `with Store() as st:`; it saves on a clean exit."""

    def __init__(self):
        self.dir = store_dir()
        self.path = os.path.join(self.dir, "links.json")
        self.lockf = None
        self.data = None

    def __enter__(self):
        os.makedirs(self.dir, exist_ok=True)
        self.lockf = open(os.path.join(self.dir, "lock"), "w")
        fcntl.flock(self.lockf, fcntl.LOCK_EX)
        if os.path.exists(self.path):
            self.data = read_json(self.path)
        else:
            self.data = {"version": 2, "seq": 0, "nodes": {}, "pending": {}, "links": {}, "msgs": {}, "outbox": []}
        return self

    def __exit__(self, exc_type, *rest):
        try:
            if exc_type is None:
                atomic_write(self.path, json.dumps(self.data, indent=1, sort_keys=True))
        finally:
            fcntl.flock(self.lockf, fcntl.LOCK_UN)
            self.lockf.close()

    @property
    def links(self):
        return self.data["links"]

    def node(self, tid):
        return self.data["nodes"].get(tid)

    def parent_link(self, tid):
        return self.links.get(tid)

    def children(self, tid, open_only=True):
        return [ln for ln in self.links.values() if ln["parent"] == tid and not (open_only and ln["closed"])]

    def find_child(self, parent, ref):
        for ln in self.children(parent, open_only=False):
            if ref in (ln["child"], ln["label"]):
                return ln
        raise Fail(f"{ref!r} is not a child link of this thread; `t3wf board` lists them")

    def new_msg(self, frm, to, kind, body):
        self.data["seq"] += 1
        h = f"m{self.data['seq']}"
        atomic_write(os.path.join(self.dir, "msgs", h + ".txt"), body)
        self.data["msgs"][h] = {"from": frm, "to": to, "kind": kind, "at": iso(now()),
                                "chars": len(body), "head": " ".join(body.split())[:160]}
        msgs = self.data["msgs"]
        if len(msgs) > 2000:
            for old in sorted(msgs, key=lambda k: int(k[1:]))[:len(msgs) - 2000]:
                msgs.pop(old)
                with contextlib.suppress(OSError):
                    os.remove(os.path.join(self.dir, "msgs", old + ".txt"))
        return h

    def body(self, handle):
        path = os.path.join(self.dir, "msgs", safe(handle) + ".txt")
        if not os.path.exists(path):
            raise Fail(f"no message {handle!r} (only the last 2000 are kept)")
        with open(path) as f:
            return f.read()


def heartbeat_path():
    return os.path.join(store_dir(), "heartbeat")


def heartbeat_age():
    try:
        with open(heartbeat_path()) as f:
            return minutes_since(f.read().strip())
    except OSError:
        return None


# ---------------------------------------------------------------- T3 access

class FakeT3:
    """A file-backed stand-in for T3, for tests: {"threads": {id: thread}, "sent": [...]}."""

    def __init__(self, path):
        self.path = path

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        pass

    def _load(self):
        return read_json(self.path)

    def send_message(self, thread_id, text):
        return self.call("send_message", thread_id=thread_id, text=text)

    def call(self, name, **args):
        d = self._load()
        if name == "t3_thread_read":
            th = d["threads"].get(args["threadId"])
            if th is None:
                raise Fail(f"thread {args['threadId']} not found")
            return {"thread": th, "items": th.get("_items", [])}
        if name == "t3_thread_send":
            if d.get("sendFails"):
                raise Fail("T3 unreachable (fake)")
            d.setdefault("sent", []).append(args)
            atomic_write(self.path, json.dumps(d, indent=1))
            return {"ok": True}
        if name == "send_message":
            return self.call("t3_thread_send", threadId=args["thread_id"], message=args["text"], mode="queue")
        if name == "t3_thread_list":
            return {"threads": list(d["threads"].values())}
        raise Fail(f"fake T3 has no tool {name}")

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        return lambda **args: self.call(name, **args)


def t3_session(label="t3wf"):
    fake = os.environ.get("T3WF_FAKE_T3")
    if fake:
        return FakeT3(fake)
    from t3api import T3
    return T3(label=label)


def t3_try(fn):
    try:
        return fn(), None
    except Exception as e:
        return None, str(e)


def deliver(t3, store, to, text, handle):
    t3.send_message(to, text)


# ---------------------------------------------------------------- message text

def header(kind, frm_label, frm_id, handle, gate=None):
    h = f"[t3wf2 {kind} from={shlex.quote(frm_label)} id={frm_id} msg={handle}"
    if gate:
        h += f" gate={gate}"
    return h + "]"


def capped(body, handle, cap=None):
    cap = cap or config()["bodyCap"]
    if len(body) <= cap:
        return body
    return body[:cap].rstrip() + f"\n... ({len(body) - cap} more chars: `t3wf show {handle}`)"


def digest_for(store, parent, exclude=(), consumed=None):
    lines = []
    for ln in store.children(parent, open_only=False):
        unread = [h for h in ln.get("unread", []) if h not in exclude and h in store.data["msgs"]]
        if not unread:
            continue
        last = store.data["msgs"][unread[-1]]
        more = f" (+{len(unread) - 1} earlier)" if len(unread) > 1 else ""
        lines.append(f"- {ln['label']} {clock(last['at'])} {last['head'][:120]} [{unread[-1]}]{more}")
        if consumed is not None:
            consumed.append((ln, set(unread)))
    cap = config()["digestLines"]
    if len(lines) > cap:
        lines = lines[:cap] + [f"- ... {len(lines) - cap} more threads: `t3wf inbox`"]
    return ("\nNotes since your last wake (`t3wf show <msg>` for one in full):\n" + "\n".join(lines)) if lines else ""


def flush_outbox(t3, store, only_to=None, force=False):
    """Deliver queued wake messages, one combined message per idle parent."""
    sent, kept, errors = [], [], []
    by_to = {}
    for item in store.data["outbox"]:
        by_to.setdefault(item["to"], []).append(item)
    for to, items in by_to.items():
        if only_to and to != only_to:
            kept += items
            continue
        if not force:
            st, err = t3_try(lambda: t3.t3_thread_read(threadId=to, limit=1, maxCharsPerItem=1)["thread"])
            if st and st.get("status") in ACTIVE and (minutes_since(items[0]["queued"]) or 0) < 30:
                kept += items
                continue
        text = "\n\n".join(i["text"] for i in items)
        handles = [i["handle"] for i in items]
        consumed = []
        text += digest_for(store, to, exclude=handles, consumed=consumed)
        _, err = t3_try(lambda: deliver(t3, store, to, text, "+".join(handles)))
        if err:
            errors.append(f"{to}: {err}")
            kept += items
        else:
            for ln, done in consumed:
                ln["unread"] = [h for h in ln.get("unread", []) if h not in done]
            sent.append((to, handles))
    store.data["outbox"] = kept
    return sent, errors


def watcher_running():
    age = heartbeat_age()
    return age is not None and age <= 3 * config()["poll"]


def send_wake(store, to, text, handle, t3=None):
    """Send now when the target is idle or the watcher is off; otherwise queue it for one combined delivery."""
    store.data["outbox"].append({"to": to, "text": text, "handle": handle, "queued": iso(now())})
    own = t3 is None
    if own:
        t3 = t3_session()
    try:
        with (t3 if own else contextlib.nullcontext(t3)) as s:
            sent, errors = flush_outbox(s, store, only_to=to, force=not watcher_running())
    except Exception as e:
        return f"queued {handle}: T3 is unreachable ({e}); the watcher retries"
    if sent:
        return f"delivered {handle}"
    if errors:
        return f"queued {handle}: {errors[0]}; the watcher retries"
    return f"queued {handle}: the recipient is mid-turn; the watcher delivers it, merged with anything else queued, when the turn ends"


# ---------------------------------------------------------------- commands

def out(lines):
    print("\n".join(lines) if isinstance(lines, list) else lines)


def cmd_whoami(a):
    if not a.self:
        raise Fail(f"${THREAD_ENV} is unset: run t3wf from a T3 thread's shell, or pass --self")
    out(f"{a.self}")


def cmd_init(a):
    with Store() as st:
        st.data["nodes"][a.self] = {"role": a.role, "at": iso(now())}
        root = st.parent_link(a.self) is None or st.parent_link(a.self)["closed"]
    msg = [f"thread {a.self} is a {a.role}" + (" with no parent: escalations come to you as ACTION NEEDED lines." if root else ".")]
    msg.append(watcher_line())
    out(msg)


def watcher_line():
    age = heartbeat_age()
    if age is not None and age <= 3 * config()["poll"]:
        return f"Link watcher: running (last pass {age:.0f} min ago)."
    if systemctl_bin() and os.path.exists(unit_path("timer")):
        return f"Link watcher: installed but its last pass was {'never' if age is None else f'{age:.0f} min ago'}; `systemctl --user status {UNIT}.timer`."
    return ("Link watcher: not installed; t3wf commands run its pass lazily. "
            "**ACTION NEEDED:** install the link watcher, here: `t3wf daemon install`")


def join_line(parent, label, role, params):
    return (f"t3wf join --parent {parent} --label {shlex.quote(label)} --role {role} "
            f"--down {params['down']} --miss {params['miss']} --stall {params['stall']} --protocol {PROTOCOL}")


def cmd_open_child(a):
    with Store() as st:
        node = st.node(a.self)
        if node is None:
            raise Fail("run `t3wf init` (no parent) or your brief's `t3wf join` line first")
        params = link_params(node["role"], a.role)
        st.data["pending"][f"{a.self}|{a.label}"] = {"parent": a.self, "label": a.label, "role": a.role,
                                                    "params": params, "at": iso(now()), "reported": False}
    out(["Put this block, verbatim, at the end of the new thread's launch brief:", "",
         "## t3-workflow link",
         "This thread is a child on a t3-workflow link. Before anything else, invoke the t3-workflow skill and run:",
         join_line(a.self, a.label, a.role, params), "",
         "Nothing to run after the launch: the child binds the link when it joins."])


def cmd_join(a):
    if a.protocol != PROTOCOL:
        raise Fail(f"this t3wf speaks protocol {PROTOCOL} only; protocol {a.protocol} is no longer supported")
    params = {k: getattr(a, k) for k in PARAMS}
    problems = validate_params(params)
    if problems:
        raise Fail("the brief's link numbers are invalid: " + "; ".join(problems))
    link_url = None
    if not os.environ.get("T3WF_OFFLINE"):
        def read_link():
            with t3_session() as t3:
                return t3.t3_thread_read(threadId=a.self, limit=1, maxCharsPerItem=1)["thread"].get("link")
        link_url, _ = t3_try(read_link)
    with Store() as st:
        st.data["nodes"][a.self] = {"role": a.role, "at": iso(now())}
        parent_node = st.node(a.parent) or {}
        st.data["pending"].pop(f"{a.parent}|{a.label}", None)
        old = st.links.get(a.self) or {}
        same = old.get("parent") == a.parent
        st.links[a.self] = {
            "child": a.self, "parent": a.parent, "label": a.label, "role": a.role,
            "parentRole": a.parent_role or parent_node.get("role"), "params": params,
            "link": link_url or old.get("link"), "opened": old.get("opened") if same else iso(now()),
            "lastUp": iso(now()), "lastSeen": None, "lastNudge": None, "nudges": 0, "escalated": False,
            "reported": old.get("reported", []) if same else [], "gate": old.get("gate") if same else None,
            "gateSince": old.get("gateSince") if same else None, "closed": False, "closedAt": None,
            "unread": old.get("unread", []) if same else []}
    out([f"Joined {a.parent} as {a.label}. The link watcher nudges this thread after {params['down']} idle minutes "
         f"with no gate open; there are no timers or check-ins to keep.",
         "Send routine progress as `--kind note` (it never wakes the parent); `event`, `gate`, `blocker` and `handoff` wake it."])


def read_body(body):
    return sys.stdin.read().strip() if body in (None, "-") else body


def cmd_send(a):
    body = read_body(a.body)
    if not body:
        raise Fail("empty body")
    with Store() as st:
        ln = st.parent_link(a.self)
        if ln is None:
            raise Fail("this thread has no parent link; it reports to the user directly")
        if ln["closed"]:
            raise Fail("the link to your parent is closed; nothing to send on it")
        if a.kind == "gate":
            ln["gate"] = re.sub(r"\s+", "_", a.gate or "user")
            ln["gateSince"] = iso(now())
        handle = st.new_msg(a.self, ln["parent"], a.kind, body)
        ln["lastUp"] = iso(now())
        ln["nudges"] = 0
        ln["escalated"] = False
        wake_kinds, cap = link_policy(ln)
        wake = a.wake if a.wake is not None else a.kind in wake_kinds
        if a.kind in CLOSING:
            ln["closed"], ln["closedAt"], ln["closedHow"] = True, iso(now()), a.kind
            ln["gate"] = None
            wake = True
        if not wake:
            ln.setdefault("unread", []).append(handle)
            result = f"noted {handle}: no wake; {ln['parent']} sees it in its next digest"
        else:
            text = header(a.kind, ln["label"], a.self, handle, ln.get("gate")) + "\n" + capped(body, handle, cap)
            result = send_wake(st, ln["parent"], text, handle)
    out(result + (". The link is closed." if a.kind in CLOSING else "."))


def cmd_reply(a):
    body = read_body(a.body)
    with Store() as st:
        ln = st.find_child(a.self, a.to)
        if ln["closed"] and not a.reopen:
            raise Fail(f"the link to {ln['label']} is closed; pass --reopen to send anyway (it starts a new turn there)")
        handle = st.new_msg(a.self, ln["child"], "reply", body)
        text = header("reply", "parent", a.self, handle) + "\n" + capped(body, handle)
        result = send_wake(st, ln["child"], text, handle)
    out(result + ".")


def cmd_gate(a):
    with Store() as st:
        ln = st.parent_link(a.self)
        if ln is None or ln["closed"]:
            raise Fail("this thread has no open parent link to hold a gate on")
        ln["gate"] = None if a.clear else re.sub(r"\s+", "_", a.open)
        ln["nudges"], ln["escalated"] = 0, False
        ln["gateSince"] = None if a.clear else iso(now())
    out("Gate cleared: the watcher nudges this thread again when it sits idle." if a.clear
        else f"Gate {ln['gate']} open: the watcher will not nudge this thread until `t3wf gate --clear`.")


def cmd_close(a):
    with Store() as st:
        ln = st.find_child(a.self, a.link)
        ln["closed"], ln["closedAt"], ln["closedHow"] = True, iso(now()), "closed-by-parent"
    out(f"The link to {ln['label']} is closed; the watcher stops watching it.")


def cmd_inbox(a):
    with Store() as st:
        lines = []
        for ln in st.children(a.self, open_only=False):
            for h in ln.get("unread", []):
                m = st.data["msgs"].get(h)
                if m:
                    lines.append(f"{h} {clock(m['at'])} {ln['label']} {m['kind']}: {m['head']}")
            ln["unread"] = []
    out(lines or "No unread notes.")


def live_states(ids):
    if not ids or os.environ.get("T3WF_OFFLINE"):
        return {}
    res = {}
    try:
        with t3_session() as t3:
            for i in ids:
                th, _ = t3_try(lambda: t3.t3_thread_read(threadId=i, limit=1, maxCharsPerItem=1)["thread"])
                if th:
                    res[i] = th
    except Exception:
        pass
    return res


def cmd_board(a):
    with Store() as st:
        par = st.parent_link(a.self)
        kids = st.children(a.self, open_only=not a.all)
        msgs = st.data["msgs"]
    live = live_states([k["child"] for k in kids if not k["closed"]])
    lines = []
    if par:
        lines.append(f"parent: {par['parent']} as {par['label']}" + (" (closed)" if par["closed"] else "")
                     + (f", gate {par['gate']}" if par.get("gate") else ""))
    if not kids:
        lines.append("No child links" + ("." if a.all else " open."))
    for k in kids:
        th = live.get(k["child"], {})
        status = th.get("status", "?") if not k["closed"] else k.get("closedHow", "closed")
        if th.get("pendingRequestCount"):
            status += "+asks"
        if th.get("pendingBackgroundTasks"):
            status += "+bg"
        last = msgs.get(next((h for h in sorted((h for h, m in msgs.items() if m["from"] == k["child"]),
                                               key=lambda h: int(h[1:]), reverse=True)), None) or "", {})
        lines.append(f"{k['label']} [{k['role']}] {status}"
                     + (f" gate={k['gate']}" if k.get("gate") else "")
                     + f" up {clock(k['lastUp'])}"
                     + (f" nudged x{k['nudges']}" if k.get("nudges") else "")
                     + (f" unread {len(k.get('unread', []))}" if k.get("unread") else "")
                     + (f" | {last.get('kind')}: {last.get('head', '')[:90]}" if last else "")
                     + f" | {k.get('link') or k['child']}")
    out(lines)


def cmd_peek(a):
    with Store() as st:
        try:
            tid = st.find_child(a.self, a.ref)["child"] if a.self else a.ref
        except Fail:
            tid = a.ref
    with t3_session() as t3:
        th = t3.t3_thread_read(threadId=tid, limit=1, maxCharsPerItem=1)["thread"]
        r = t3.t3_thread_read(threadId=tid, limit=a.messages, maxCharsPerItem=a.chars,
                              beforePosition=max(int(th.get("itemCount") or 0), 1) + 1)
    lines = [f"{th.get('title')} | {th.get('status')} | updated {clock(th.get('updatedAt'))} | {th.get('link')}"]
    extras = []
    if th.get("pendingRequestCount"):
        extras.append(f"{th['pendingRequestCount']} pending request(s)")
    if th.get("pendingBackgroundTasks"):
        extras.append(f"{len(th['pendingBackgroundTasks'])} background task(s)")
    for pr in th.get("pullRequests") or []:
        extras.append(f"PR {pr.get('url') or pr.get('number')} {pr.get('state', '')}".strip())
    if extras:
        lines.append("; ".join(extras))
    for it in r.get("items", []):
        text = it.get("text") or it.get("content") or ""
        if isinstance(text, (list, dict)):
            text = json.dumps(text)[:a.chars]
        lines.append(f"{it.get('role') or it.get('type', 'item')}: {' '.join(str(text).split())[:a.chars]}")
    out(lines)


def cmd_show(a):
    with Store() as st:
        m = st.data["msgs"].get(a.handle)
        body = st.body(a.handle)
    if m:
        print(f"{a.handle} {m['kind']} {clock(m['at'])} from {m['from']}")
    print(body)


def cmd_status(a):
    with Store() as st:
        d = {"node": st.node(a.self), "parent": st.parent_link(a.self),
             "children": st.children(a.self, open_only=False),
             "pending": [p for p in st.data["pending"].values() if p["parent"] == a.self]}
    print(json.dumps(d, indent=1))


def cmd_sweep(a):
    with Store() as st:
        open_kids = st.children(a.self)
        par = st.parent_link(a.self)
        pend = [p["label"] for p in st.data["pending"].values() if p["parent"] == a.self]
        for p in list(st.data["pending"]):
            if st.data["pending"][p]["parent"] == a.self:
                st.data["pending"].pop(p)
    lines = []
    if open_kids:
        lines.append("Links still open: " + ", ".join(f"{k['label']} ({k.get('link') or k['child']})" for k in open_kids)
                     + ". Close each with its handoff or `t3wf close --link <id>` before you finish.")
    else:
        lines.append("Every child link is closed.")
    if pend:
        lines.append("Dropped never-joined links: " + ", ".join(pend) + ".")
    if par and not par["closed"]:
        lines.append("Your own parent link is open: finish with `t3wf send --kind handoff` (or `--kind closed`).")
    out(lines)


def cmd_adopt(a):
    with Store() as st:
        moved = []
        for ln in st.children(a.from_):
            ln["parent"] = a.self
            moved.append(ln["label"])
        for key in [k for k, p in st.data["pending"].items() if p["parent"] == a.from_]:
            p = st.data["pending"].pop(key)
            p["parent"] = a.self
            st.data["pending"][f"{a.self}|{p['label']}"] = p
        st.data["outbox"] = [dict(i, to=a.self) if i["to"] == a.from_ else i for i in st.data["outbox"]]
    out(("Adopted: " + ", ".join(moved) + ". Their messages now come here; they need no word of it."
         if moved else f"{a.from_} has no open child links."))


def cmd_check_config(a):
    problems = check_config()
    out(problems or "config ok")
    if problems:
        sys.exit(1)


# ---------------------------------------------------------------- the watcher

def nudge_text(ln, why, handle):
    return (header("nudge", "watcher", "t3wf", handle) + "\n" + why +
            f" Run your role's resume step, then tell {ln['parent']} what changed with `t3wf send` "
            "(`--kind note` for routine progress). Waiting on a person: `t3wf gate --open <what>`. "
            "Done: `t3wf send --kind handoff`. Stopped for good: `t3wf send --kind closed`. "
            f"Nudge {ln['nudges']} of {ln['params']['miss']}.")


def report_once(st, ln, key, kind, text, actions):
    if key in ln.setdefault("reported", []):
        return
    ln["reported"].append(key)
    handle = st.new_msg("t3wf", ln["parent"], kind, text)
    st.data["outbox"].append({"to": ln["parent"], "text": header(kind, ln["label"], ln["child"], handle) + "\n" + text,
                              "handle": handle, "queued": iso(now())})
    actions.append(f"{kind} to {ln['parent']} about {ln['label']}: {key}")


def tick(t3, st, dry=False):
    actions = []
    for ln in list(st.links.values()):
        if ln["closed"]:
            continue
        p = ln["params"]
        th, err = t3_try(lambda: t3.t3_thread_read(threadId=ln["child"], limit=1, maxCharsPerItem=1)["thread"])
        if th is None:
            if err and "not found" in err.lower():
                ln["closed"], ln["closedAt"], ln["closedHow"] = True, iso(now()), "thread-gone"
                h = st.new_msg("t3wf", ln["parent"], "note", f"{ln['label']}'s thread is gone; its link is closed.")
                ln.setdefault("unread", []).append(h)
                actions.append(f"closed {ln['label']}: thread gone")
            continue
        if th.get("archived"):
            ln["closed"], ln["closedAt"], ln["closedHow"] = True, iso(now()), "archived"
            actions.append(f"closed {ln['label']}: archived")
            continue
        if not ln.get("link") and th.get("link"):
            ln["link"] = th["link"]
        updated = th.get("updatedAt")
        ln["lastSeen"] = updated
        status = th.get("status")
        busy = (status in ACTIVE or th.get("blockedOnThreadWait") or th.get("pendingBackgroundTasks"))
        idle = minutes_since(updated) or 0
        if th.get("pendingRequestCount") or status == "waiting":
            if idle >= p["down"]:
                report_once(st, ln, f"asks@{th.get('latestRunId')}", "report",
                            f"{ln['label']} is waiting on an answer to a question or approval in its own thread "
                            f"for {idle:.0f} min: {ln.get('link') or ln['child']}", actions)
            continue
        if busy:
            up_quiet = minutes_since(ln["lastUp"]) or 0
            if not ln.get("gate") and up_quiet >= p["stall"]:
                report_once(st, ln, f"stall@{ln['lastUp']}", "report",
                            f"{ln['label']} has been busy but sent nothing for {up_quiet:.0f} min. "
                            f"`t3wf peek {shlex.quote(ln['label'])}` shows where it is: {ln.get('link') or ln['child']}", actions)
            continue
        if ln.get("gate"):
            continue
        bad_end = status in ENDED_BADLY and ln.get("badRun") != th.get("latestRunId")
        since_nudge = minutes_since(ln["lastNudge"]) if ln.get("lastNudge") else None
        due = bad_end or (idle >= p["down"] and (since_nudge is None or since_nudge >= p["down"]))
        if not due:
            continue
        if ln["nudges"] >= p["miss"]:
            if not ln["escalated"]:
                ln["escalated"] = True
                body = (f"{ln['label']} has sat idle for {idle:.0f} min through {ln['nudges']} nudges, with no gate open. "
                        f"Look at it (`t3wf peek {shlex.quote(ln['label'])}`), then reply, relaunch it, or "
                        f"`t3wf close --link {ln['child']}`. Its thread: {ln.get('link') or ln['child']}")
                handle = st.new_msg("t3wf", ln["parent"], "escalate", body)
                text = header("escalate", ln["label"], ln["child"], handle) + "\n" + body
                st.data["outbox"].append({"to": ln["parent"], "text": text, "handle": handle, "queued": iso(now())})
                actions.append(f"escalate {ln['label']} to {ln['parent']}")
            continue
        ln["nudges"] += 1
        ln["lastNudge"] = iso(now())
        if bad_end:
            ln["badRun"] = th.get("latestRunId")
        why = (f"Your last run ended {status}." if bad_end else
               f"This thread has been idle {idle:.0f} min with its link to {ln['label']}'s parent open and no gate.")
        handle = st.new_msg("t3wf", ln["child"], "nudge", why)
        text = nudge_text(ln, why, handle)
        actions.append(f"nudge {ln['label']} ({ln['nudges']}/{p['miss']})")
        if not dry:
            _, err = t3_try(lambda: deliver(t3, st, ln["child"], text, handle))
            if err:
                st.data["outbox"].append({"to": ln["child"], "text": text, "handle": handle, "queued": iso(now())})
    for key, pend in st.data["pending"].items():
        age = minutes_since(pend["at"]) or 0
        if not pend.get("reported") and age >= pend["params"]["down"]:
            pend["reported"] = True
            body = (f"{pend['label']} was opened {age:.0f} min ago and never joined. Check its launch; "
                    "`t3wf sweep` drops it.")
            handle = st.new_msg("t3wf", pend["parent"], "report", body)
            st.data["outbox"].append({"to": pend["parent"], "queued": iso(now()), "handle": handle,
                                      "text": header("report", pend["label"], "-", handle) + "\n" + body})
            actions.append(f"report never-joined {pend['label']}")
    if not dry:
        sent, errors = flush_outbox(t3, st)
        actions += [f"delivered {'+'.join(h)} to {to}" for to, h in sent]
        actions += [f"delivery failed: {e}" for e in errors]
        atomic_write(heartbeat_path(), iso(now()))
    return actions


def cmd_tick(a):
    with Store() as st:
        if not any(not ln["closed"] for ln in st.links.values()) and not st.data["outbox"] and not st.data["pending"]:
            if not a.dry_run:
                atomic_write(heartbeat_path(), iso(now()))
            out("idle: no open links")
            return
        with t3_session("t3wf-watcher") as t3:
            actions = tick(t3, st, dry=a.dry_run)
    out(actions or "no action")


def lazy_tick():
    if os.environ.get("T3WF_NO_LAZY"):
        return
    age = heartbeat_age()
    if age is not None and age < 5 * config()["poll"]:
        return
    with contextlib.suppress(Exception):
        with Store() as st:
            if any(not ln["closed"] for ln in st.links.values()) or st.data["outbox"]:
                with t3_session("t3wf-watcher") as t3:
                    tick(t3, st)


# ---------------------------------------------------------------- daemon (systemd user timer)

def unit_path(kind, unit=UNIT):
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(os.path.expanduser("~"), ".config")
    return os.path.join(base, "systemd", "user", f"{unit}.{kind}")


def shim_path():
    return os.path.join(store_dir(), "bin", "t3wf-tick")


SHIM = """#!/bin/sh
export T3WF_STATE_DIR=%(state)s
exec python3 %(script)s tick
"""


def systemctl_bin():
    return os.environ.get("T3WF_SYSTEMCTL") or shutil.which("systemctl")


def systemctl(*args, check=True):
    subprocess.run([systemctl_bin(), "--user", *args], check=check)


def remove_timer(unit):
    systemctl("disable", "--now", f"{unit}.timer", check=False)
    for k in ("timer", "service"):
        with contextlib.suppress(OSError):
            os.remove(unit_path(k, unit))
    systemctl("daemon-reload", check=False)


def install_timer():
    atomic_write(shim_path(), SHIM % {"script": shlex.quote(os.path.abspath(__file__)),
                                      "state": shlex.quote(state_root())})
    os.chmod(shim_path(), 0o755)
    poll = config()["poll"]
    atomic_write(unit_path("service"),
                 "[Unit]\nDescription=t3-workflow link watcher pass\n\n[Service]\nType=oneshot\n"
                 f"ExecStart={shim_path()}\nTimeoutStartSec=120\nNice=10\n")
    atomic_write(unit_path("timer"),
                 "[Unit]\nDescription=t3-workflow link watcher\n\n[Timer]\n"
                 f"OnBootSec=2min\nOnUnitActiveSec={poll}min\nAccuracySec=30s\n\n[Install]\nWantedBy=timers.target\n")
    systemctl("daemon-reload")
    systemctl("enable", "--now", f"{UNIT}.timer")
    return poll


def cmd_daemon(a):
    if a.action == "status":
        out(watcher_line())
        return
    if not systemctl_bin():
        raise Fail("no systemctl here: t3wf commands run the watcher's pass lazily instead. "
                   "On macOS, a launchd agent running `t3wf tick` every 2 minutes does the same job.")
    if a.action == "uninstall":
        remove_timer(UNIT)
        out("Link watcher removed.")
        return
    poll = install_timer()
    out(f"Link watcher installed: {UNIT}.timer runs `t3wf tick` every {poll} min. "
        f"Logs: `journalctl --user -u {UNIT}.service`.")


# ---------------------------------------------------------------- migration from the cbrip plugin

def migrate_store():
    """Move the registry the cbrip plugin kept under ~/.claude to this skill's state folder, once.

    The old path becomes a symlink to the new one, so a thread still running the
    plugin's t3wf reads and writes the same registry, and the new lock is the old
    lock file's inode, so a plugin command already queued on it stays serialized.
    links.json is copied last: its presence marks a finished migration. Every check
    runs under a lock beside the old folder, so no command sees the move half done.
    """
    old, new = legacy_store_dir(), store_dir()
    if not os.path.isdir(os.path.dirname(old)):
        return None
    with open(os.path.join(os.path.dirname(old), ".t3wf-migrate.lock"), "a") as ml:
        fcntl.flock(ml, fcntl.LOCK_EX)
        if (os.path.islink(old) or not os.path.isfile(os.path.join(old, "links.json"))
                or os.path.realpath(old) == new or os.path.exists(os.path.join(new, "links.json"))):
            return None
        return move_store(old, new)


def move_store(old, new):
    with open(os.path.join(old, "lock"), "a") as lk:
        fcntl.flock(lk, fcntl.LOCK_EX)
        shutil.copytree(old, new, dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns("lock", "links.json", ".t3wf-*"))
        tmp = os.path.join(new, f".t3wf-lock-{os.getpid()}")
        with contextlib.suppress(OSError):
            with contextlib.suppress(FileNotFoundError):
                os.remove(tmp)
            os.link(os.path.join(old, "lock"), tmp)
            os.replace(tmp, os.path.join(new, "lock"))
        tmp = os.path.join(new, f".t3wf-links-{os.getpid()}")
        shutil.copy2(os.path.join(old, "links.json"), tmp)
        os.replace(tmp, os.path.join(new, "links.json"))
        aside = f"{old}.migrated"
        try:
            os.rename(old, aside)
            try:
                os.symlink(new, old)
            except OSError:
                os.rename(aside, old)
                raise
        except OSError:
            os.remove(os.path.join(new, "links.json"))
            raise
    shutil.rmtree(aside, ignore_errors=True)
    return f"moved the link registry from {old} to {new}; the old path is now a symlink to it"


def registries_shared():
    old = legacy_store_dir()
    return not os.path.isfile(os.path.join(old, "links.json")) or os.path.realpath(old) == os.path.realpath(store_dir())


def migrate_timer():
    """Swap the cbrip plugin's watcher timer for this one, once both read the same registry."""
    if not os.path.exists(unit_path("timer", LEGACY_UNIT)) or not systemctl_bin() or not registries_shared():
        return None
    install_timer()
    remove_timer(LEGACY_UNIT)
    return f"replaced {LEGACY_UNIT}.timer with {UNIT}.timer"


def migrate():
    """Run both migration steps; False when the registry could not move, so no command may start a new one."""
    for step in (migrate_store, migrate_timer):
        try:
            done = step()
        except (OSError, subprocess.CalledProcessError) as e:
            print(f"t3wf: {step.__name__} failed: {e}", file=sys.stderr)
            if step is migrate_store:
                print(f"t3wf: the registry is still at {legacy_store_dir()}; fix the error and run t3wf again",
                      file=sys.stderr)
                return False
            continue
        if done:
            print(f"t3wf: {done}.", file=sys.stderr)
    return True


# ---------------------------------------------------------------- code mode

def vars_path(self_id):
    return os.path.join(store_dir(), "vars", safe(self_id or "anon") + ".json")


class LazyT3:
    def __init__(self):
        self.s = None

    def _get(self):
        if self.s is None:
            self.s = t3_session("t3wf-exec").__enter__()
        return self.s

    def call(self, name, **args):
        return self._get().call(name, **args)

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        return lambda **args: self._get().call(name, **args)

    def close(self):
        if self.s is not None:
            self.s.__exit__(None, None, None)


def cmd_exec(a):
    src = sys.stdin.read() if a.file in (None, "-") else open(a.file).read()
    vp = vars_path(a.self)
    try:
        saved = read_json(vp)
    except (OSError, ValueError):
        saved = {}
    t3 = LazyT3()
    buf = io.StringIO()

    def show(obj, chars=2000):
        s = obj if isinstance(obj, str) else json.dumps(obj, ensure_ascii=False, default=str)
        print(s if len(s) <= chars else s[:chars] + f" ... ({len(s) - chars} more chars)")

    def registry():
        with Store() as st:
            return json.loads(json.dumps(st.data))

    g = {"__name__": "__t3wf_exec__", "t3": t3, "vars": saved, "self_id": a.self, "show": show,
         "registry": registry, "json": json, "re": re}
    failed = False
    with contextlib.redirect_stdout(buf):
        try:
            exec(compile(src, "<t3wf exec>", "exec"), g)
        except Exception:
            failed = True
            tb = traceback.format_exc().strip().splitlines()
            print("\n".join(tb[-6:]))
        finally:
            t3.close()
    try:
        atomic_write(vp, json.dumps(g["vars"], default=str))
    except (TypeError, ValueError) as e:
        print(f"vars not saved: {e}", file=buf)
    text = buf.getvalue()
    if len(text) > a.max_out:
        text = text[:a.max_out] + f"\n... output cut at {a.max_out} of {len(text)} chars; print less, or keep it in `vars`."
    sys.stdout.write(text)
    if failed:
        sys.exit(1)


# ---------------------------------------------------------------- cli

def build_parser():
    p = argparse.ArgumentParser(prog="t3wf", description="t3-workflow link protocol 2.")
    p.add_argument("--now", help=argparse.SUPPRESS)
    p.add_argument("--json", action="store_true", help=argparse.SUPPRESS)
    sub = p.add_subparsers(dest="cmd", required=True)

    def cmd(name, fn, help_, self_req=True):
        s = sub.add_parser(name, help=help_)
        s.add_argument("--self", default=os.environ.get(THREAD_ENV), help=f"this thread's id (default ${THREAD_ENV})")
        s.set_defaults(fn=fn, self_req=self_req)
        return s

    cmd("whoami", cmd_whoami, "print this thread's id", self_req=False)
    s = cmd("init", cmd_init, "start this thread as a root (no parent)")
    s.add_argument("--role", required=True, choices=list(ROLE_SKILL))
    s = cmd("open-child", cmd_open_child, "prepare a link block for a child's launch brief")
    s.add_argument("--role", required=True, choices=list(ROLE_SKILL))
    s.add_argument("--label", required=True)
    s = cmd("join", cmd_join, "join the link from a launch brief")
    s.add_argument("--parent", required=True)
    s.add_argument("--parent-role", choices=list(ROLE_SKILL))
    s.add_argument("--label", required=True)
    s.add_argument("--role", required=True, choices=list(ROLE_SKILL))
    for k in PARAMS:
        s.add_argument(f"--{k}", type=int, default=config()["link"][k])
    s.add_argument("--protocol", type=int, default=PROTOCOL)
    s = cmd("send", cmd_send, "send to this thread's parent")
    s.add_argument("--kind", required=True, choices=UP_KINDS)
    s.add_argument("--body", help="text, or - (default) for stdin")
    s.add_argument("--gate", help="with --kind gate: the gate's name (default user)")
    w = s.add_mutually_exclusive_group()
    w.add_argument("--wake", dest="wake", action="store_true", default=None)
    w.add_argument("--no-wake", dest="wake", action="store_false")
    s = cmd("reply", cmd_reply, "send to a child")
    s.add_argument("--to", required=True, help="the child's thread id or label")
    s.add_argument("--body")
    s.add_argument("--reopen", action="store_true")
    s = cmd("gate", cmd_gate, "open or clear a wait on a person")
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--open")
    g.add_argument("--clear", action="store_true")
    s = cmd("close", cmd_close, "close a child link without a handoff")
    s.add_argument("--link", required=True)
    cmd("inbox", cmd_inbox, "unread notes from children, then mark them read")
    s = cmd("board", cmd_board, "one line per child link, with live status")
    s.add_argument("--all", action="store_true", help="include closed links")
    s = cmd("peek", cmd_peek, "compact live view of one thread")
    s.add_argument("ref", help="a child label, or any thread id")
    s.add_argument("--messages", type=int, default=1)
    s.add_argument("--chars", type=int, default=500)
    s = cmd("show", cmd_show, "a stored message in full", self_req=False)
    s.add_argument("handle")
    cmd("status", cmd_status, "this thread's registry entries as JSON")
    cmd("sweep", cmd_sweep, "before finishing: name links still open")
    s = cmd("adopt", cmd_adopt, "take over a retiring thread's child links")
    s.add_argument("--from", dest="from_", required=True)
    s = cmd("tick", cmd_tick, "one watcher pass over every open link", self_req=False)
    s.add_argument("--dry-run", action="store_true")
    s = cmd("daemon", cmd_daemon, "install, remove or check the watcher timer", self_req=False)
    s.add_argument("action", choices=("install", "uninstall", "status"))
    s = cmd("exec", cmd_exec, "code mode: run Python with `t3`, `vars` and `show` bound; only its output returns",
            self_req=False)
    s.add_argument("file", nargs="?", default="-")
    s.add_argument("--max-out", type=int, default=6000)
    cmd("check-config", cmd_check_config, "validate defaults.json and every t3-role.json", self_req=False)
    return p


def main(argv=None):
    global _NOW
    argv = list(sys.argv[1:] if argv is None else argv)
    args = build_parser().parse_args(argv)
    if args.now:
        _NOW = parse_ts(args.now)
    if args.self_req and not args.self:
        print(f"t3wf: no thread id: ${THREAD_ENV} is unset; pass --self", file=sys.stderr)
        return 2
    if not migrate():
        return 2
    try:
        args.fn(args)
    except Fail as e:
        print(f"t3wf: {e}", file=sys.stderr)
        return 2
    if args.cmd not in ("tick", "daemon", "exec", "check-config", "show"):
        lazy_tick()
    return 0


if __name__ == "__main__":
    sys.exit(main())
