//! End-to-end cover for the self-update path, against a fake release served
//! from the local filesystem.
//!
//! `GBM_BASE_URL` points the code at that fake release — the same override
//! `install.sh` takes — so the real download, checksum and binary-swap code
//! runs without touching the network or the published releases.
//!
//! Everything lives in one test function on purpose: it manipulates process
//! environment variables, which are global, and cargo runs tests in parallel
//! threads. One test means no race.
//!
//! Skipped on Windows, where the published asset is a zip: building one here
//! would need a zip writer, and `tar` is the only archiver we can rely on.

#![cfg(not(windows))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use git_branch_manager::update::{self, Status};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gbm-update-flow-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run(dir: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("could not run {program}: {e}"));
    assert!(
        out.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Build `releases/download/<version>/` holding the platform asset and a
/// SHA256SUMS, exactly as the release workflow publishes them.
fn fake_release(root: &Path, version: &str, binary_contents: &str, good_sums: bool) -> String {
    let asset = update::asset_name(version).expect("this platform has a published asset");
    let stage = root.join("stage");
    let payload = stage.join(asset.trim_end_matches(".tar.gz"));
    fs::create_dir_all(&payload).unwrap();
    fs::write(payload.join("git-branch-manager"), binary_contents).unwrap();

    let dest = root.join("releases").join("download").join(version);
    fs::create_dir_all(&dest).unwrap();

    let rel_dir = payload.file_name().unwrap().to_str().unwrap().to_string();
    run(
        &stage,
        "tar",
        &[
            "-czf",
            dest.join(&asset).to_str().unwrap(),
            rel_dir.as_str(),
        ],
    );

    let digest = if good_sums {
        update::sha256_of(&dest.join(&asset)).unwrap()
    } else {
        "0".repeat(64)
    };
    fs::write(dest.join("SHA256SUMS"), format!("{digest}  {asset}\n")).unwrap();
    asset
}

#[test]
fn check_prompt_and_install_against_a_fake_release() {
    let cache = TempDir::new("cache");
    std::env::set_var("XDG_CACHE_HOME", &cache.path);
    std::env::remove_var("GBM_NO_UPDATE_CHECK");

    // --- the check: a repo whose newest tag is far ahead of this build -------
    let repo = TempDir::new("repo");
    run(&repo.path, "git", &["init", "-q", "--bare", "origin.git"]);
    let work = repo.path.join("work");
    fs::create_dir_all(&work).unwrap();
    run(&work, "git", &["init", "-q", "-b", "main"]);
    run(&work, "git", &["config", "user.email", "t@t"]);
    run(&work, "git", &["config", "user.name", "t"]);
    fs::write(work.join("f"), "x").unwrap();
    run(&work, "git", &["add", "."]);
    run(&work, "git", &["commit", "-qm", "init"]);
    for tag in ["v0.1.0", "v9.9.9", "v0.2.0"] {
        run(&work, "git", &["tag", tag]);
    }
    let origin = repo.path.join("origin.git");
    run(
        &work,
        "git",
        &["push", "-q", origin.to_str().unwrap(), "main", "--tags"],
    );

    std::env::set_var("GBM_BASE_URL", &origin);
    assert_eq!(
        update::check().unwrap(),
        Status::Available("v9.9.9".into()),
        "the newest tag should be offered, not the last one pushed"
    );

    // The answer is cached, so a second check does not need the remote at all.
    std::env::set_var("GBM_BASE_URL", "/definitely/not/a/repo");
    assert_eq!(
        update::check().unwrap(),
        Status::Available("v9.9.9".into()),
        "the cached answer should be reused within the TTL"
    );

    // And the opt-out silences it entirely.
    std::env::set_var("GBM_NO_UPDATE_CHECK", "1");
    assert_eq!(update::check().unwrap(), Status::Current);
    std::env::remove_var("GBM_NO_UPDATE_CHECK");

    // --- installing it -------------------------------------------------------
    let release = TempDir::new("release");
    fake_release(&release.path, "v9.9.9", "NEW BINARY", true);
    std::env::set_var("GBM_BASE_URL", format!("file://{}", release.path.display()));

    let installed = TempDir::new("bin");
    let target = installed.path.join("git-branch-manager");
    fs::write(&target, "OLD BINARY").unwrap();

    // Drive the real composed function, not a re-implementation of it.
    let steps = std::sync::Mutex::new(Vec::<String>::new());
    update::install_to(&target, "v9.9.9", &|step: &str| {
        steps.lock().unwrap().push(step.to_string())
    })
    .expect("the fake release should install cleanly");

    assert_eq!(
        *steps.lock().unwrap(),
        ["downloading", "verifying", "installing"],
        "the UI is driven by these steps, so their order is part of the behaviour"
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "NEW BINARY",
        "the binary on disk should be the downloaded one"
    );
    assert!(
        !installed.path.join(".git-branch-manager.update").exists(),
        "no staging file should survive a successful install"
    );

    // --- a tampered download must be refused ---------------------------------
    let bad = TempDir::new("bad-release");
    fake_release(&bad.path, "v9.9.9", "EVIL BINARY", false);
    std::env::set_var("GBM_BASE_URL", format!("file://{}", bad.path.display()));

    let err = update::install_to(&target, "v9.9.9", &|_: &str| {})
        .expect_err("a bad checksum must not install")
        .to_string();
    assert!(err.contains("checksum mismatch"), "{err}");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "NEW BINARY",
        "a refused update must leave the installed binary alone"
    );

    // --- a cargo-managed install is left to cargo ----------------------------
    let cargo_dir = installed.path.join(".cargo").join("bin");
    fs::create_dir_all(&cargo_dir).unwrap();
    let cargo_target = cargo_dir.join("git-branch-manager");
    fs::write(&cargo_target, "CARGO BINARY").unwrap();
    let err = update::install_to(&cargo_target, "v9.9.9", &|_: &str| {})
        .expect_err("cargo installs should be refused, not overwritten")
        .to_string();
    assert!(err.contains("cargo install"), "{err}");
    assert_eq!(fs::read_to_string(&cargo_target).unwrap(), "CARGO BINARY");

    std::env::remove_var("GBM_BASE_URL");
    std::env::remove_var("XDG_CACHE_HOME");
}
