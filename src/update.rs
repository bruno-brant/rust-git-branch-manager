//! Finding out whether a newer release exists, and installing it.
//!
//! The check deliberately uses `git ls-remote` rather than an HTTP client: the
//! tool already requires git, and borrowing it means no TLS stack, no extra
//! dependency in the static musl build, and the user's existing proxy and
//! credential configuration is honoured for free.
//!
//! Nothing here ever writes to stdout. That stream carries one thing only — the
//! worktree path for `cd "$(git-branch-manager)"` — so update chatter would
//! break shell wrappers.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

pub const REPO: &str = "bruno-brant/rust-git-branch-manager";

/// The version this binary was built as, without the leading `v`.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// How long a check is trusted before asking the network again.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Where releases live. `GBM_BASE_URL` overrides it, which is what the tests
/// point at a local server (the same override `install.sh` offers).
pub fn base_url() -> String {
    std::env::var("GBM_BASE_URL").unwrap_or_else(|_| format!("https://github.com/{REPO}"))
}

/// Result of a check, as the UI needs to think about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Running the newest release, or new enough.
    Current,
    /// A newer release exists.
    Available(String),
}

/// True when `candidate` is a later release than `current`. Both may carry a
/// leading `v`. Anything that does not parse as `x.y.z` loses — a tag we cannot
/// read is never a reason to nag someone to upgrade.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().strip_prefix('v').unwrap_or(v.trim());
    // Stop at any pre-release or build suffix: 1.2.3-rc1 counts as 1.2.3 here,
    // and the equality that follows keeps it from displacing a real 1.2.3.
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Pick the highest `vX.Y.Z` out of `git ls-remote --tags` output.
pub fn newest_tag(ls_remote_output: &str) -> Option<String> {
    ls_remote_output
        .lines()
        .filter_map(|line| line.split("refs/tags/").nth(1))
        .filter_map(|tag| parse_version(tag).map(|v| (v, tag.trim().to_string())))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, tag)| tag)
}

/// Ask the remote what the newest released version is.
pub fn latest_version() -> Result<String> {
    let out = Command::new("git")
        .args([
            // Don't let a dead network hold the worker thread forever.
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=10",
            "ls-remote",
            "--tags",
            "--refs",
            &base_url(),
        ])
        .output()
        .context("failed to run `git ls-remote`")?;

    if !out.status.success() {
        bail!(
            "`git ls-remote` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    newest_tag(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| anyhow!("no version tags found"))
}

// --- the daily cache -------------------------------------------------------

/// File the last check is remembered in, so running the tool twenty times an
/// hour does not mean twenty round trips.
pub fn cache_path() -> Option<PathBuf> {
    let dir = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
    }?;
    Some(dir.join("git-branch-manager").join("last-update-check"))
}

/// A cached result still inside its TTL, if there is one.
pub fn read_cache(path: &Path, now: SystemTime) -> Option<String> {
    let body = fs::read_to_string(path).ok()?;
    let (stamp, version) = body.split_once('\n')?;
    let stamp = Duration::from_secs(stamp.trim().parse().ok()?);
    let age = now.duration_since(UNIX_EPOCH).ok()?.checked_sub(stamp)?;
    (age < CACHE_TTL).then(|| version.trim().to_string())
}

pub fn write_cache(path: &Path, version: &str, now: SystemTime) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let stamp = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    fs::write(path, format!("{stamp}\n{version}\n"))?;
    Ok(())
}

/// The whole check: honour the opt-out, use the cache when it is fresh, and
/// otherwise ask the network and remember the answer.
///
/// Errors are the caller's to swallow — being offline is not a malfunction.
pub fn check() -> Result<Status> {
    if std::env::var_os("GBM_NO_UPDATE_CHECK").is_some() {
        return Ok(Status::Current);
    }
    let cache = cache_path();
    let now = SystemTime::now();

    let latest = match cache.as_deref().and_then(|p| read_cache(p, now)) {
        Some(cached) => cached,
        None => {
            let fetched = latest_version()?;
            if let Some(path) = cache.as_deref() {
                // A cache we cannot write is not worth failing a check over.
                let _ = write_cache(path, &fetched, now);
            }
            fetched
        }
    };

    Ok(if is_newer(&latest, CURRENT) {
        Status::Available(latest)
    } else {
        Status::Current
    })
}

// --- installing it ---------------------------------------------------------

/// The release asset for the platform this binary was built for.
///
/// Linux always takes the static build: it runs anywhere, and an update is
/// exactly the moment to stop depending on the host's glibc.
pub fn asset_name(version: &str) -> Result<String> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x86_64-static",
        // One universal binary covers both Macs.
        ("macos", "x86_64" | "aarch64") => "macos-universal",
        ("windows", "x86_64") => "windows-x86_64",
        (os, arch) => bail!(
            "no prebuilt binary for {os} {arch} — update with:\n  \
             cargo install --git https://github.com/{REPO}"
        ),
    };
    let ext = if platform.starts_with("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    Ok(format!("git-branch-manager-{version}-{platform}.{ext}"))
}

/// The expected hash for `asset` out of a `SHA256SUMS` file. The `*` form is
/// what `sha256sum` writes for binary mode.
pub fn expected_hash(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        (name == asset).then(|| hash.trim().to_lowercase())
    })
}

pub fn sha256_of(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn curl(url: &str, dest: &Path) -> Result<()> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(dest)
        .arg(url)
        .output()
        .context("curl is required to download an update, and could not be run")?;
    if !out.status.success() {
        bail!(
            "download failed: {url}{}",
            match String::from_utf8_lossy(&out.stderr).trim() {
                "" => String::new(),
                why => format!(" ({why})"),
            }
        );
    }
    Ok(())
}

fn extract(archive: &Path, into: &Path) -> Result<()> {
    // bsdtar (macOS, Windows 10+) reads zip; -xzf is the gzip path everywhere.
    let args: &[&str] = if archive.extension().is_some_and(|e| e == "zip") {
        &["-xf"]
    } else {
        &["-xzf"]
    };
    let out = Command::new("tar")
        .args(args)
        .arg(archive)
        .arg("-C")
        .arg(into)
        .output()
        .context("tar is required to unpack an update, and could not be run")?;
    if !out.status.success() {
        bail!(
            "could not unpack {}: {}",
            archive.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// Put `new` in place of `target`, which may be the running program.
///
/// On Unix a running executable can be renamed over: the process holds its
/// inode, so the swap is atomic and the old image stays valid until exit.
/// Windows refuses to replace a running image but allows renaming it, so the
/// old one is moved aside and cleaned up on the next start.
pub fn replace_binary(target: &Path, new: &Path) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", target.display()))?;

    // Stage beside the target: rename cannot cross filesystems, and the
    // download lives in a temp dir that often is one.
    let staged = dir.join(".git-branch-manager.update");
    fs::copy(new, &staged).with_context(|| {
        format!(
            "cannot write to {} — this install may need elevated rights, \
             or was put there by a package manager",
            dir.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    }

    if cfg!(windows) {
        let aside = target.with_extension("exe.old");
        let _ = fs::remove_file(&aside);
        fs::rename(target, &aside)?;
    }

    if let Err(e) = fs::rename(&staged, target) {
        let _ = fs::remove_file(&staged);
        return Err(e).with_context(|| format!("could not replace {}", target.display()));
    }
    Ok(())
}

/// Remove the displaced binary a previous Windows update left behind. A no-op
/// everywhere else, and never an error worth surfacing.
pub fn clean_after_update() {
    if !cfg!(windows) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(exe.with_extension("exe.old"));
    }
}

/// Download `version`, check it against the published sums, and swap it in.
/// `progress` is called with each step so the UI can say what is happening.
pub fn install(version: &str, progress: &dyn Fn(&str)) -> Result<()> {
    let target = std::env::current_exe().context("cannot locate the running binary")?;
    install_to(&target, version, progress)
}

/// The body of [`install`], against an explicit destination so it can be tested
/// without overwriting the test runner.
pub fn install_to(target: &Path, version: &str, progress: &dyn Fn(&str)) -> Result<()> {
    if target.components().any(|c| c.as_os_str() == ".cargo") {
        bail!(
            "this copy was installed by cargo — update it with:\n  \
             cargo install --git https://github.com/{REPO}"
        );
    }

    let asset = asset_name(version)?;
    let base = base_url();
    let tmp = TempDir::new()?;

    progress("downloading");
    let archive = tmp.path.join(&asset);
    curl(
        &format!("{base}/releases/download/{version}/{asset}"),
        &archive,
    )?;

    progress("verifying");
    let sums_path = tmp.path.join("SHA256SUMS");
    curl(
        &format!("{base}/releases/download/{version}/SHA256SUMS"),
        &sums_path,
    )?;
    let sums = fs::read_to_string(&sums_path)?;
    let expected = expected_hash(&sums, &asset)
        .ok_or_else(|| anyhow!("SHA256SUMS has no entry for {asset}"))?;
    let actual = sha256_of(&archive)?;
    if expected != actual {
        bail!("checksum mismatch for {asset}\n  expected {expected}\n  actual   {actual}");
    }

    progress("installing");
    extract(&archive, &tmp.path)?;
    let binary = find_binary(&tmp.path)
        .ok_or_else(|| anyhow!("the downloaded archive did not contain git-branch-manager"))?;
    replace_binary(target, &binary)
}

/// The unpacked archive holds `<name>-<version>-<platform>/git-branch-manager`.
fn find_binary(root: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "git-branch-manager.exe"
    } else {
        "git-branch-manager"
    };
    let direct = root.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    for entry in fs::read_dir(root).ok()?.flatten() {
        let candidate = entry.path().join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// A temp directory that removes itself. Small enough not to be worth a crate.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gbm-update-{stamp}-{}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Serialises the tests that set process-wide environment variables. Cargo runs
/// tests as parallel threads, so without this one test's `remove_var` can land
/// in the middle of another's download and send it at the real GitHub.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions_are_recognised() {
        assert!(is_newer("v0.6.0", "0.5.0"));
        assert!(is_newer("0.5.1", "0.5.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("v0.10.0", "0.9.0"), "10 is not less than 9");
    }

    #[test]
    fn same_or_older_versions_are_not_an_update() {
        assert!(!is_newer("v0.5.0", "0.5.0"));
        assert!(!is_newer("v0.4.9", "0.5.0"));
        // A dev build ahead of any release must not be told to downgrade.
        assert!(!is_newer("v0.5.0", "0.6.0-dev"));
    }

    #[test]
    fn unparseable_tags_never_prompt_an_update() {
        for junk in ["nightly", "v", "v1.2.3.4", "release-2026"] {
            assert!(!is_newer(junk, "0.5.0"), "{junk} should not count");
        }
    }

    #[test]
    fn the_newest_tag_wins_whatever_the_order() {
        let out = "\
aaa\trefs/tags/v0.1.0
bbb\trefs/tags/v0.10.0
ccc\trefs/tags/v0.9.0
ddd\trefs/tags/not-a-version
";
        assert_eq!(newest_tag(out).as_deref(), Some("v0.10.0"));
    }

    #[test]
    fn no_tags_means_no_answer() {
        assert_eq!(newest_tag(""), None);
        assert_eq!(newest_tag("aaa\trefs/heads/main"), None);
    }

    #[test]
    fn asset_names_match_what_the_release_workflow_publishes() {
        let name = asset_name("v0.5.0").unwrap();
        assert!(name.starts_with("git-branch-manager-v0.5.0-"), "{name}");
        let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => "git-branch-manager-v0.5.0-linux-x86_64-static.tar.gz",
            ("macos", _) => "git-branch-manager-v0.5.0-macos-universal.tar.gz",
            ("windows", _) => "git-branch-manager-v0.5.0-windows-x86_64.zip",
            _ => return,
        };
        assert_eq!(name, expected);
    }

    #[test]
    fn hashes_are_read_out_of_a_sums_file() {
        let sums = "\
abc123  git-branch-manager-v0.5.0-linux-x86_64.tar.gz
DEF456 *git-branch-manager-v0.5.0-windows-x86_64.zip
";
        assert_eq!(
            expected_hash(sums, "git-branch-manager-v0.5.0-linux-x86_64.tar.gz").as_deref(),
            Some("abc123")
        );
        // Binary-mode entries and upper-case digests are the same thing.
        assert_eq!(
            expected_hash(sums, "git-branch-manager-v0.5.0-windows-x86_64.zip").as_deref(),
            Some("def456")
        );
        assert_eq!(expected_hash(sums, "something-else.tar.gz"), None);
    }

    #[test]
    fn a_fresh_cache_is_used_and_a_stale_one_is_not() {
        let dir = std::env::temp_dir().join(format!("gbm-cache-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("last-update-check");
        let now = SystemTime::now();

        write_cache(&path, "v9.9.9", now).unwrap();
        assert_eq!(read_cache(&path, now).as_deref(), Some("v9.9.9"));

        let tomorrow = now + CACHE_TTL + Duration::from_secs(60);
        assert_eq!(
            read_cache(&path, tomorrow),
            None,
            "a day-old check is stale"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_cache_is_simply_a_miss() {
        assert_eq!(
            read_cache(Path::new("/nonexistent/gbm/cache"), SystemTime::now()),
            None
        );
    }

    #[test]
    fn the_opt_out_short_circuits_the_check() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("GBM_NO_UPDATE_CHECK", "1");
        let status = check().unwrap();
        std::env::remove_var("GBM_NO_UPDATE_CHECK");
        assert_eq!(status, Status::Current);
    }

    #[test]
    fn replacing_a_binary_swaps_the_contents_in_place() {
        let dir = std::env::temp_dir().join(format!("gbm-replace-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("git-branch-manager");
        let new = dir.join("downloaded");
        fs::write(&target, b"old").unwrap();
        fs::write(&new, b"new").unwrap();

        replace_binary(&target, &new).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(
            !dir.join(".git-branch-manager.update").exists(),
            "the staging file must not be left behind"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the replacement must stay executable");
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replacing_into_an_unwritable_place_explains_itself() {
        let err = replace_binary(
            Path::new("/nonexistent-dir/git-branch-manager"),
            Path::new("/etc/hostname"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("cannot write to"), "{err}");
    }
}
