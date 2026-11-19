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

fn fake_archive(dir: &Path, reported: &str) -> (String, Vec<u8>) {
    let target = TARGET.expect("a release target");
    let v = Version::parse(NEW).unwrap();
    let stem = archive_stem(&v, target);
    let inner = dir.join("pack").join(&stem);
    std::fs::create_dir_all(&inner).unwrap();
    let script = inner.join("t3cm");
    std::fs::write(&script, format!("#!/bin/sh\necho 't3cm {reported}'\n")).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let name = archive_name(&v, target);
    let out = dir.join(&name);
    let ok = Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(dir.join("pack")).arg(&stem).status().unwrap();
    assert!(ok.success());
    (name, std::fs::read(out).unwrap())
}

async fn setup(reported: &str, corrupt: bool) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let (name, archive) = fake_archive(dir.path(), reported);
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
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let bin = bin_dir.join("t3cm");
    std::fs::copy(env!("CARGO_BIN_EXE_t3cm"), &bin).unwrap();
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
    assert_eq!(files, ["t3cm"], "no staged or backup binary is left behind");
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
