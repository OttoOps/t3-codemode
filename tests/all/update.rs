#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use axum::Router;
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use sha2::{Digest, Sha256};
use t3_codemode::release::Version;
use t3_codemode::upgrade::{TARGET, archive_name, archive_stem};

use crate::skills_install::{copy_tree, npx_install, repo_skills, skills_dir};

const NEW: &str = "9.9.9";

#[derive(Clone)]
struct Release {
    archive: Vec<u8>,
    name: String,
    sums: String,
}

async fn latest() -> Response {
    (StatusCode::FOUND, [(header::LOCATION, format!("/releases/tag/v{NEW}"))]).into_response()
}

async fn download(State(r): State<Release>, UrlPath((tag, file)): UrlPath<(String, String)>) -> Response {
    if tag != format!("v{NEW}") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if file == r.name {
        r.archive.into_response()
    } else if file == "SHA256SUMS" {
        r.sums.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

fn hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn which(tool: &str) -> PathBuf {
    let out = Command::new("sh").args(["-c", &format!("command -v {tool}")]).output().unwrap();
    PathBuf::from(String::from_utf8(out.stdout).unwrap().trim())
}

struct Env {
    dir: tempfile::TempDir,
    bin: PathBuf,
    url: String,
}

impl Env {
    fn path(&self) -> PathBuf {
        self.dir.path().join("path")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn t3cm(&self, args: &[&str]) -> Output {
        Command::new(&self.bin)
            .args(args)
            .env_clear()
            .env("HOME", self.dir.path())
            .env("PATH", self.path())
            .env("T3CM_RELEASES_URL", &self.url)
            .env("T3CM_STATE_DIR", self.state())
            .env("T3CM_CONFIG", self.dir.path().join("none.json"))
            .output()
            .unwrap()
    }
}

fn fake_archive(dir: &Path, script: &str, unit: Option<&str>) -> (String, Vec<u8>) {
    let target = TARGET.expect("a release target");
    let v = Version::parse(NEW).unwrap();
    let stem = archive_stem(&v, target);
    let inner = dir.join("pack").join(&stem);
    std::fs::create_dir_all(&inner).unwrap();
    let bin = inner.join("t3cm");
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    if let Some(unit) = unit {
        std::fs::write(inner.join("t3-codemode.service"), unit).unwrap();
    }
    let name = archive_name(&v, target);
    let out = dir.join(&name);
    let ok = Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(dir.join("pack")).arg(&stem).status().unwrap();
    assert!(ok.success());
    (name, std::fs::read(out).unwrap())
}

fn reports(version: &str) -> String {
    format!("#!/bin/sh\necho 't3cm {version}'\n")
}

async fn setup(reported: &str, corrupt: bool) -> Env {
    serve_release(&reports(reported), None, corrupt, "bin").await
}

async fn serve_release(script: &str, unit: Option<&str>, corrupt: bool, bin_dir: &str) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let (name, archive) = fake_archive(dir.path(), script, unit);
    let sum = if corrupt { "0".repeat(64) } else { hex(&archive) };
    let sums = format!("{sum}  {name}\n{}  other.zip\n", "f".repeat(64));
    let app = Router::new()
        .route("/releases/latest", get(latest))
        .route("/releases/download/{tag}/{file}", get(download))
        .with_state(Release { archive, name, sums });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/releases", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let path = dir.path().join("path");
    std::fs::create_dir_all(&path).unwrap();
    for tool in ["tar", "gzip"] {
        std::os::unix::fs::symlink(which(tool), path.join(tool)).unwrap();
    }
    let bin_dir = dir.path().join(bin_dir);
    std::fs::create_dir_all(&bin_dir).unwrap();
    let bin = bin_dir.join("t3cm");
    t3_codemode::fake::copy_executable(Path::new(env!("CARGO_BIN_EXE_t3cm")), &bin);
    Env { dir, bin, url }
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn leftovers(bin: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(bin.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn check_reports_the_newer_release_and_caches_it() {
    let env = setup(NEW, false).await;
    let (out, cached) = tokio::task::spawn_blocking(move || {
        let o = env.t3cm(&["update", "--check"]);
        (o, std::fs::read_to_string(env.state().join("update-check.json")).unwrap_or_default())
    })
    .await
    .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains(&format!("{} -> {NEW}", env!("CARGO_PKG_VERSION"))), "{t}");
    assert!(t.contains(&format!("/releases/tag/v{NEW}")), "{t}");
    assert!(cached.contains(NEW), "{cached}");
}

#[tokio::test(flavor = "multi_thread")]
async fn update_replaces_the_binary_in_place() {
    let env = setup(NEW, false).await;
    let (out, after, files) = tokio::task::spawn_blocking(move || {
        let out = env.t3cm(&["update", "--no-restart", "--port", "9"]);
        let after = Command::new(&env.bin).arg("--version").output().unwrap();
        (out, String::from_utf8_lossy(&after.stdout).trim().to_string(), leftovers(&env.bin))
    })
    .await
    .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("Checksum OK") && t.contains("skipped the provenance check"), "{t}");
    assert!(t.contains(&format!("Installed t3cm {NEW}")), "{t}");
    assert_eq!(after, format!("t3cm {NEW}"));
    assert_eq!(files, ["t3cm", "t3cm.old"], "the old binary is kept until a service has run the new one");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_checksum_mismatch_leaves_the_binary_alone() {
    let env = setup(NEW, true).await;
    let before = std::fs::read(&env.bin).unwrap();
    let (out, after, files) = tokio::task::spawn_blocking(move || {
        let out = env.t3cm(&["update", "--no-restart"]);
        (out, std::fs::read(&env.bin).unwrap(), leftovers(&env.bin))
    })
    .await
    .unwrap();
    assert!(!out.status.success());
    assert!(text(&out).contains("checksum mismatch"), "{}", text(&out));
    assert!(after == before);
    assert_eq!(files, ["t3cm"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_binary_that_misreports_its_version_is_refused() {
    let env = setup("0.0.1", false).await;
    let before = std::fs::read(&env.bin).unwrap();
    let (out, after) = tokio::task::spawn_blocking(move || {
        let out = env.t3cm(&["update", "--no-restart"]);
        (out, std::fs::read(&env.bin).unwrap())
    })
    .await
    .unwrap();
    assert!(!out.status.success());
    assert!(text(&out).contains("not t3cm 9.9.9"), "{}", text(&out));
    assert!(after == before);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unpublished_version_fails_cleanly() {
    let env = setup(NEW, false).await;
    let out = tokio::task::spawn_blocking(move || env.t3cm(&["update", "--no-restart", "--version", "v8.0.0"]))
        .await
        .unwrap();
    assert!(!out.status.success());
    assert!(text(&out).contains("404"), "{}", text(&out));
}

const SKILLS_STUB: &str = r#"#!/bin/sh
case "$1" in
  --version) echo 't3cm 9.9.9' ;;
  skills) echo "$*" > "$HOME/skills-call"; exit "${STUB_EXIT:-0}" ;;
esac
"#;

fn install_skills(env: &Env) {
    let out = env.t3cm(&["skills", "install"]);
    assert!(out.status.success(), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread")]
async fn update_brings_managed_skills_to_the_new_release() {
    let env = serve_release(SKILLS_STUB, None, false, "bin").await;
    let (out, call) = tokio::task::spawn_blocking(move || {
        install_skills(&env);
        let out = env.t3cm(&["update", "--no-restart", "--port", "9"]);
        (out, std::fs::read_to_string(env.dir.path().join("skills-call")).unwrap_or_default())
    })
    .await
    .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(call.trim(), "skills update", "the new binary syncs the skills it carries");
}

#[tokio::test(flavor = "multi_thread")]
async fn update_leaves_skills_alone_when_t3cm_did_not_install_them() {
    let env = serve_release(SKILLS_STUB, None, false, "bin").await;
    let (out, called) = tokio::task::spawn_blocking(move || {
        let out = env.t3cm(&["update", "--no-restart", "--port", "9"]);
        (out, env.dir.path().join("skills-call").exists())
    })
    .await
    .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert!(!called, "a user who declined the workflow gets no skills from an update");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_skills_sync_fails_the_update_but_keeps_the_new_binary() {
    let env = serve_release(&SKILLS_STUB.replace("${STUB_EXIT:-0}", "1"), None, false, "bin").await;
    let (out, after) = tokio::task::spawn_blocking(move || {
        install_skills(&env);
        let out = env.t3cm(&["update", "--no-restart", "--port", "9"]);
        (out, Command::new(&env.bin).arg("--version").output().unwrap())
    })
    .await
    .unwrap();
    let t = text(&out);
    assert!(!out.status.success(), "{t}");
    assert!(t.contains("skills update failed") && t.contains("run: t3cm skills update"), "{t}");
    assert_eq!(String::from_utf8_lossy(&after.stdout).trim(), format!("t3cm {NEW}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_up_to_date_update_still_repairs_the_skills() {
    let env = setup(NEW, false).await;
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let (out, back) = tokio::task::spawn_blocking(move || {
        install_skills(&env);
        let file = env.dir.path().join(".agents/skills/t3-worker/procedure.md");
        std::fs::remove_file(&file).unwrap();
        let out = env.t3cm(&["update", "--no-restart", "--version", &current]);
        (out, file.is_file())
    })
    .await
    .unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("is up to date") && t.contains("synced the skills"), "{t}");
    assert!(back, "the missing skill file is written again");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_up_to_date_update_waits_for_a_running_update() {
    let env = setup(NEW, false).await;
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let (blocked, back_while_held, after, back) = tokio::task::spawn_blocking(move || {
        install_skills(&env);
        let file = env.dir.path().join(".agents/skills/t3-worker/procedure.md");
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir_all(env.state()).unwrap();
        let lock = std::fs::File::create(env.state().join("update.lock")).unwrap();
        lock.lock().unwrap();
        let blocked = env.t3cm(&["update", "--no-restart", "--version", &current]);
        let back_while_held = file.is_file();
        lock.unlock().unwrap();
        drop(lock);
        let after = env.t3cm(&["update", "--no-restart", "--version", &current]);
        (blocked, back_while_held, after, file.is_file())
    })
    .await
    .unwrap();
    let t = text(&blocked);
    assert!(!blocked.status.success(), "{t}");
    assert!(t.contains("another t3cm update is running"), "{t}");
    assert!(!back_while_held, "no skill file is written while another update holds the lock");
    assert!(after.status.success(), "{}", text(&after));
    assert!(back, "the sync runs once the lock is free");
}

#[cfg(target_os = "linux")]
mod service {
    use super::*;
    use std::time::{Duration, Instant};

    const SHIPPED_UNIT: &str = include_str!("../../deploy/t3-codemode.service");

    const STUB: &str = r#"#!/bin/sh
case "$1" in
  --version) echo 't3cm 9.9.9' ;;
  serve) exec python3 -c '
import http.server, json, sys
port = int(sys.argv[sys.argv.index("--port") + 1])
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = json.dumps({"version": "9.9.9"}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
' "$@" ;;
esac
"#;

    const CRASHES: &str = "#!/bin/sh\ncase \"$1\" in\n  --version) echo 't3cm 9.9.9' ;;\n  serve) exit 1 ;;\nesac\n";

    struct Service {
        env: Env,
        port: u16,
    }

    impl Service {
        fn fake(&self) -> PathBuf {
            self.env.dir.path().join("systemd")
        }

        fn pid(&self) -> Option<i32> {
            std::fs::read_to_string(self.fake().join("pid")).ok()?.trim().parse().ok()
        }

        fn restart(&self) {
            let out = Command::new(self.env.path().join("systemctl")).args(["--user", "restart"]).output().unwrap();
            assert!(out.status.success(), "{}", text(&out));
        }

        fn serves(&self, version: &str) -> bool {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                if healthz(self.port).as_deref() == Some(version) {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            false
        }

        fn update(&self) -> Output {
            self.env.t3cm(&["update", "--port", &self.port.to_string()])
        }
    }

    impl Drop for Service {
        fn drop(&mut self) {
            if let Some(pid) = self.pid() {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }

    fn healthz(port: u16) -> Option<String> {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        write!(s, "GET /healthz HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").ok()?;
        let mut body = String::new();
        s.read_to_string(&mut body).ok()?;
        let json = body.split("\r\n\r\n").nth(1)?;
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        v["version"].as_str().map(str::to_string)
    }

    async fn installed(script: &str, unit: Option<&str>) -> Service {
        let env = serve_release(script, unit, false, ".local/bin").await;
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let fake = env.dir.path().join("systemd");
        std::fs::create_dir_all(&fake).unwrap();
        let units = env.dir.path().join(".config/systemd/user");
        std::fs::create_dir_all(&units).unwrap();
        std::fs::write(units.join("t3-codemode.service"), SHIPPED_UNIT).unwrap();
        let home = env.dir.path().display().to_string();
        let f = fake.display().to_string();
        t3_codemode::fake::write_executable(
            &fake.join("service"),
            &format!(
                "#!/bin/sh\nexport T3CODE_HOME='{home}' T3CM_T3_BIN='{home}/no-t3' T3CM_CONFIG='{home}/none.json' T3CM_NO_UPDATE_CHECK=1\n\
                 exec '{home}/.local/bin/t3cm' serve --no-watcher --port {port} --state-dir '{f}/state'\n"
            ),
        );
        t3_codemode::fake::write_executable(
            &env.path().join("systemctl"),
            &format!(
                r#"#!/bin/sh
PATH=/usr/bin:/bin
S='{f}'
alive() {{ p=$(cat "$S/pid" 2>/dev/null) && [ -d "/proc/$p" ] && [ "$(cut -d' ' -f3 "/proc/$p/stat")" != Z ]; }}
echo "$*" >> "$S/calls"
case "$2" in
  cat) exit 0 ;;
  is-active) alive ;;
  show) if alive && [ ! -e "$S/hide-pid" ]; then cat "$S/pid"; else echo 0; fi ;;
  restart)
    if alive; then kill "$(cat "$S/pid")"; while alive; do sleep 0.05; done; fi
    setsid "$S/service" >> "$S/log" 2>&1 < /dev/null &
    echo $! > "$S/pid" ;;
  *) exit 0 ;;
esac
"#
            ),
        );
        let svc = Service { env, port };
        svc.restart();
        assert!(svc.serves(env!("CARGO_PKG_VERSION")), "the fake service never served the current t3cm");
        svc
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_restarts_the_service_onto_the_new_release() {
        let changed = SHIPPED_UNIT.replace("Nice=5", "Nice=6");
        let svc = installed(STUB, Some(&changed)).await;
        let (out, svc) = tokio::task::spawn_blocking(move || (svc.update(), svc)).await.unwrap();
        let t = text(&out);
        assert!(out.status.success(), "{t}");
        assert!(t.contains(&format!("t3cm updated: {} -> {NEW}", env!("CARGO_PKG_VERSION"))), "{t}");
        assert_eq!(healthz(svc.port).as_deref(), Some(NEW));
        assert_eq!(leftovers(&svc.env.bin), ["t3cm"], "the backup goes once the new release serves");
        let home = svc.env.dir.path();
        let local = std::fs::read_to_string(home.join(".config/systemd/user/t3-codemode.service")).unwrap();
        assert_eq!(local, SHIPPED_UNIT, "the updater never writes the unit");
        let saved = std::fs::read_to_string(svc.env.state().join("t3-codemode.service")).unwrap();
        assert_eq!(saved, changed);
        assert!(t.contains("This release changes t3-codemode.service"), "{t}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_release_that_will_not_serve_is_rolled_back() {
        let svc = installed(CRASHES, None).await;
        let before = std::fs::read(&svc.env.bin).unwrap();
        let (out, svc) = tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let out = svc.update();
            assert!(started.elapsed() < Duration::from_secs(30), "the rollback waited out the health timeout");
            (out, svc)
        })
        .await
        .unwrap();
        let t = text(&out);
        assert!(!out.status.success(), "{t}");
        assert!(t.contains("the service stopped before it reported 9.9.9"), "{t}");
        assert!(t.contains(&format!("Rolled back to {}", env!("CARGO_PKG_VERSION"))), "{t}");
        assert!(svc.serves(env!("CARGO_PKG_VERSION")));
        assert!(std::fs::read(&svc.env.bin).unwrap() == before);
        assert_eq!(leftovers(&svc.env.bin), ["t3cm"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_is_refused_while_the_service_serves_another_version() {
        let svc = installed(STUB, None).await;
        let other = svc.fake().join("service");
        t3_codemode::fake::write_executable(
            &other,
            &format!("#!/bin/sh\nexec python3 -c '{}' --port {}\n", OLD_SERVER, svc.port),
        );
        svc.restart();
        assert!(svc.serves("0.0.1"));
        let before = std::fs::read(&svc.env.bin).unwrap();
        let (out, svc) = tokio::task::spawn_blocking(move || (svc.update(), svc)).await.unwrap();
        let t = text(&out);
        assert!(!out.status.success(), "{t}");
        assert!(t.contains("a t3cm 0.0.1 answers"), "{t}");
        assert!(std::fs::read(&svc.env.bin).unwrap() == before);
        assert_eq!(leftovers(&svc.env.bin), ["t3cm"]);
        let calls = std::fs::read_to_string(svc.fake().join("calls")).unwrap();
        assert_eq!(calls.matches("restart").count(), 2, "{calls}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_is_refused_when_the_service_binary_is_unknown() {
        let svc = installed(STUB, None).await;
        std::fs::write(svc.fake().join("hide-pid"), "").unwrap();
        let before = std::fs::read(&svc.env.bin).unwrap();
        let (out, svc) = tokio::task::spawn_blocking(move || {
            (svc.env.t3cm(&["update", "--no-restart", "--port", &svc.port.to_string()]), svc)
        })
        .await
        .unwrap();
        let t = text(&out);
        assert!(!out.status.success(), "{t}");
        assert!(t.contains("could not tell which binary the running service runs"), "{t}");
        assert!(std::fs::read(&svc.env.bin).unwrap() == before);
    }

    const OLD_SERVER: &str = r#"
import http.server, sys
port = int(sys.argv[sys.argv.index("--port") + 1])
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = b"{\"version\": \"0.0.1\"}"
        self.send_response(200)
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)
    def log_message(self, *a):
        pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#;
}

async fn up_to_date_update(prepare: fn(&Path)) -> Output {
    let env = setup(NEW, false).await;
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    tokio::task::spawn_blocking(move || {
        prepare(env.dir.path());
        env.t3cm(&["update", "--no-restart", "--version", &current])
    })
    .await
    .unwrap()
}

fn assert_take_over_hint_follows_up_to_date(out: &Output) {
    let t = text(out);
    assert!(out.status.success(), "{t}");
    let up = t.find("is up to date").unwrap_or_else(|| panic!("no up-to-date line: {t}"));
    let hint = t.find("t3cm skills install --take-over").unwrap_or_else(|| panic!("no take-over hint: {t}"));
    assert!(up < hint, "the hint follows the up-to-date line: {t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_up_to_date_update_offers_to_take_over_npx_skills() {
    let out = up_to_date_update(|home| {
        npx_install(home, false);
    })
    .await;
    assert_take_over_hint_follows_up_to_date(&out);
    assert!(text(&out).contains("came from npx skills"), "{}", text(&out));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_up_to_date_update_offers_to_take_over_hand_copied_skills() {
    let out = up_to_date_update(|home| copy_tree(&repo_skills(), &skills_dir(home))).await;
    assert_take_over_hint_follows_up_to_date(&out);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_up_to_date_update_without_skills_offers_no_take_over() {
    let out = up_to_date_update(|_| {}).await;
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("is up to date") && !t.contains("--take-over"), "{t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_up_to_date_update_survives_an_unreadable_skill_lock() {
    let out = up_to_date_update(|home| {
        std::fs::create_dir_all(home.join(".agents")).unwrap();
        std::fs::write(home.join(".agents").join(".skill-lock.json"), "{").unwrap();
    })
    .await;
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("is up to date") && t.contains(".skill-lock.json is not valid JSON"), "{t}");
}

const OLDER: &str = "0.6.0";

async fn serve_older_latest() -> String {
    let app = Router::new().route(
        "/releases/latest",
        get(|| async { (StatusCode::FOUND, [(header::LOCATION, format!("/releases/tag/v{OLDER}"))]).into_response() }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/releases", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

fn assert_installed_is_newer() {
    assert!(Version::parse(OLDER).unwrap() < Version::current(), "OLDER must stay below the crate version");
}

#[tokio::test(flavor = "multi_thread")]
async fn check_never_names_an_older_release_as_latest() {
    assert_installed_is_newer();
    let mut env = setup(NEW, false).await;
    env.url = serve_older_latest().await;
    let out = tokio::task::spawn_blocking(move || env.t3cm(&["update", "--check"])).await.unwrap();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(!t.contains(&format!("latest release: {OLDER}")), "{t}");
    assert!(t.contains(&format!("{} is up to date", env!("CARGO_PKG_VERSION"))), "{t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn status_never_reports_an_older_release_as_latest() {
    assert_installed_is_newer();
    let url = serve_older_latest().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let left_by_older_install = format!(r#"{{"latest":"{OLDER}","checkedAt":"{}"}}"#, jiff::Timestamp::now());
    std::fs::write(state.join("update-check.json"), left_by_older_install).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
    let home = dir.path().to_path_buf();
    let base = move |args: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_t3cm"));
        c.args(args)
            .env_clear()
            .env("HOME", &home)
            .env("T3CODE_HOME", &home)
            .env("T3CM_T3_BIN", home.join("no-t3"))
            .env("T3CM_TAILSCALE_BIN", home.join("no-tailscale"))
            .env("T3CM_CONFIG", home.join("none.json"))
            .env("T3CM_RELEASES_URL", &url)
            .env("T3CM_STATE_DIR", &state)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        c
    };
    let (shown, cached) = tokio::task::spawn_blocking(move || {
        let state = dir.path().join("state");
        let mut svc = base(&["serve", "--no-watcher", "--port", &port, "--state-dir"]).arg(&state).spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut shown = Vec::new();
        loop {
            let out = base(&["status", "--port", &port]).output().unwrap();
            if out.status.success() {
                let s: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
                let update = s["update"].clone();
                shown.push(update.clone());
                if !update.is_null() {
                    break;
                }
            }
            if std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let _ = svc.kill();
        let _ = svc.wait();
        (shown, std::fs::read_to_string(state.join("update-check.json")).unwrap_or_default())
    })
    .await
    .unwrap();
    let current = env!("CARGO_PKG_VERSION");
    assert!(shown.last().is_some_and(|u| !u.is_null()), "the service never ran its check: {shown:?}");
    for u in shown.iter().filter(|u| !u.is_null()) {
        assert_eq!(u["latest"], current, "{shown:?}");
        assert_eq!(u["available"], false, "{shown:?}");
    }
    assert!(cached.contains(&format!(r#""checkedBy":"{current}""#)), "{cached}");
}
