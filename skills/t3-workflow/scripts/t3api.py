"""A stdlib client for the running T3 Code server's /mcp endpoint, for scripts outside the model.

T3 serves its orchestration tools over Streamable HTTP MCP. A process run as the
machine's owner mints a short-lived `mcp-client` bearer session with the `t3` CLI
(`t3 auth session issue`), the way `t3 thread send` does, uses it, and revokes it
on close. The token never leaves this process.
"""
import glob
import json
import os
import subprocess
import urllib.error
import urllib.request

PROTOCOL_VERSION = "2025-06-18"
SCOPES = ("orchestration:read", "orchestration:operate")


class T3Error(Exception):
    pass


def t3_home():
    return os.environ.get("T3CODE_HOME") or os.path.join(os.path.expanduser("~"), ".t3")


def runtime_state():
    path = os.path.join(t3_home(), "userdata", "server-runtime.json")
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        raise T3Error(f"T3 Code is not running here: no readable {path}")


def find_t3_binary(state=None):
    """The t3 CLI that matches the running server: $T3WF_T3_BIN, the server's own binary, then PATH."""
    env = os.environ.get("T3WF_T3_BIN")
    if env:
        return env
    state = state or {}
    pid = state.get("pid")
    if pid:
        exe = f"/proc/{pid}/exe"
        try:
            real = os.path.realpath(exe)
            if os.path.basename(real) == "t3" and os.access(real, os.X_OK):
                return real
        except OSError:
            pass
    for pattern in ("wsl-runtime/sha256-*/t3", "runtime/versions/*/t3"):
        found = sorted(glob.glob(os.path.join(t3_home(), pattern)), key=os.path.getmtime, reverse=True)
        if found:
            return found[0]
    return "t3"


def _parse_body(content_type, raw):
    if "text/event-stream" in (content_type or ""):
        last = None
        for line in raw.splitlines():
            if line.startswith("data:"):
                last = line[5:].strip()
        if last is None:
            raise T3Error("empty event stream from /mcp")
        return json.loads(last)
    return json.loads(raw) if raw.strip() else None


class T3:
    """One MCP session against the local T3 server. Use as a context manager."""

    def __init__(self, ttl="15m", label="t3wf"):
        self.ttl = ttl
        self.label = label
        self.token = None
        self.session_id = None
        self.mcp_session = None
        self.origin = None
        self.binary = None
        self._next_id = 0

    def __enter__(self):
        self.open()
        return self

    def __exit__(self, *exc):
        self.close()

    def open(self):
        state = runtime_state()
        self.origin = os.environ.get("T3WF_T3_ORIGIN") or state.get("origin") or f"http://127.0.0.1:{state.get('port', 3773)}"
        self.binary = find_t3_binary(state)
        cmd = [self.binary, "auth", "session", "issue", "--subject", "mcp-client", "--ttl", self.ttl,
               "--label", self.label, "--json"]
        for s in SCOPES:
            cmd += ["--scope", s]
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        except (OSError, subprocess.TimeoutExpired) as e:
            raise T3Error(f"could not run {self.binary}: {e}")
        if r.returncode != 0:
            raise T3Error(f"`t3 auth session issue` failed: {r.stderr.strip()[-300:]}")
        issued = json.loads(r.stdout[r.stdout.index("{"):])
        self.token, self.session_id = issued["token"], issued["sessionId"]
        res = self._post({"jsonrpc": "2.0", "id": self._id(), "method": "initialize",
                          "params": {"protocolVersion": PROTOCOL_VERSION, "capabilities": {},
                                     "clientInfo": {"name": self.label, "version": "2"}}})
        if not res or "result" not in res:
            raise T3Error(f"MCP initialize failed: {res}")
        self._post({"jsonrpc": "2.0", "method": "notifications/initialized"})
        return self

    def close(self):
        if self.session_id and self.binary:
            try:
                subprocess.run([self.binary, "auth", "session", "revoke", self.session_id],
                               capture_output=True, text=True, timeout=60)
            except (OSError, subprocess.TimeoutExpired):
                pass
        self.token = self.session_id = None

    def _id(self):
        self._next_id += 1
        return self._next_id

    def _post(self, body):
        headers = {"Authorization": f"Bearer {self.token}", "Content-Type": "application/json",
                   "Accept": "application/json, text/event-stream", "MCP-Protocol-Version": PROTOCOL_VERSION}
        if self.mcp_session:
            headers["Mcp-Session-Id"] = self.mcp_session
        req = urllib.request.Request(self.origin.rstrip("/") + "/mcp", data=json.dumps(body).encode(), headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                sid = r.headers.get("Mcp-Session-Id")
                if sid:
                    self.mcp_session = sid
                return _parse_body(r.headers.get("Content-Type"), r.read().decode())
        except urllib.error.HTTPError as e:
            raise T3Error(f"/mcp HTTP {e.code}: {e.read().decode()[:300]}")
        except urllib.error.URLError as e:
            raise T3Error(f"/mcp unreachable at {self.origin}: {e.reason}")

    def tools(self):
        res = self._post({"jsonrpc": "2.0", "id": self._id(), "method": "tools/list"})
        return res["result"]["tools"]

    def call(self, name, **args):
        """Call one T3 tool; return its structured result, or its text parsed as JSON when it is JSON."""
        res = self._post({"jsonrpc": "2.0", "id": self._id(), "method": "tools/call",
                          "params": {"name": name, "arguments": args}})
        if res is None:
            raise T3Error(f"{name}: no response")
        if "error" in res:
            raise T3Error(f"{name}: {res['error'].get('message', res['error'])}")
        result = res["result"]
        if result.get("isError"):
            raise T3Error(f"{name}: " + " ".join(c.get("text", "") for c in result.get("content", []))[:500])
        if "structuredContent" in result:
            return result["structuredContent"]
        text = "".join(c.get("text", "") for c in result.get("content", []) if c.get("type") == "text")
        try:
            return json.loads(text)
        except ValueError:
            return text

    def send_message(self, thread_id, text):
        """Queue a message as the thread's next turn through `t3 thread send`.

        Sessions from `t3 auth session issue` carry an approval-required runtime
        ceiling, so T3 refuses their t3_thread_send to a full-access thread. The
        CLI's own session runs at the owner's full-access ceiling.
        """
        binary = self.binary or find_t3_binary(runtime_state())
        try:
            r = subprocess.run([binary, "thread", "send", thread_id, "-"], input=text,
                               capture_output=True, text=True, timeout=120)
        except (OSError, subprocess.TimeoutExpired) as e:
            raise T3Error(f"could not run {binary}: {e}")
        if r.returncode != 0:
            raise T3Error(f"`t3 thread send` failed: {(r.stderr or r.stdout).strip()[-300:]}")

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        return lambda **args: self.call(name, **args)
