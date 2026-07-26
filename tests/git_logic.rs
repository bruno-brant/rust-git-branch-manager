//! Integration tests for the git layer. Each test builds its own throwaway
//! repository with git2, so there is no external setup — they exercise the real
//! merged-detection and worktree-mapping logic end to end.
//!
//! These link libgit2, so they require a working build toolchain. Run with:
//!   cargo test --test git_logic

use git_branch_manager::git;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use git2::{Repository, Signature};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A unique temp directory that cleans itself up on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("gbm-test-{pid}-{n}"));
        if path.exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Commit all current index contents and return the new commit id. Creates an
/// initial commit (no parent) or a child of HEAD.
fn commit_all(repo: &Repository, msg: &str) -> git2::Oid {
    let sig = Signature::now("test", "test@example.com").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();

    let parents = match repo.head() {
        Ok(h) => vec![repo.find_commit(h.target().unwrap()).unwrap()],
        Err(_) => vec![],
    };
    let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)
        .unwrap()
}

fn write_file(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).unwrap();
}

/// Build a repo on `main` with one commit, returning the repo and its working dir.
fn init_repo(dir: &Path) -> Repository {
    let repo = Repository::init(dir).unwrap();
    // Ensure the default branch is named `main` for predictability.
    repo.set_head("refs/heads/main").ok();
    write_file(dir, "a.txt", "hello");
    commit_all(&repo, "initial");
    repo
}

#[test]
fn merged_branch_is_detected_as_merged() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);

    // Branch off HEAD with no new commits -> tip == main tip -> merged.
    let head = repo.head().unwrap().target().unwrap();
    repo.branch("feature-merged", &repo.find_commit(head).unwrap(), false)
        .unwrap();

    let branches = git::list_branches(&repo).unwrap();
    let f = branches
        .iter()
        .find(|b| b.name == "feature-merged")
        .unwrap();
    assert!(
        f.is_merged,
        "a branch with no commits beyond HEAD is merged"
    );
    assert!(!f.is_head);
    assert!(f.worktree.is_none());
}

#[test]
fn unmerged_branch_is_detected_as_unmerged() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);

    // Create a branch, check it out, add a commit not reachable from main.
    let head = repo.head().unwrap().target().unwrap();
    repo.branch("feature-unmerged", &repo.find_commit(head).unwrap(), false)
        .unwrap();
    repo.set_head("refs/heads/feature-unmerged").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    write_file(&tmp.path, "b.txt", "world");
    commit_all(&repo, "unmerged work");

    // Move back to main so HEAD is main again.
    repo.set_head("refs/heads/main").unwrap();
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();

    let branches = git::list_branches(&repo).unwrap();
    let f = branches
        .iter()
        .find(|b| b.name == "feature-unmerged")
        .unwrap();
    assert!(
        !f.is_merged,
        "a branch with a commit not on HEAD is unmerged"
    );
}

#[test]
fn head_branch_is_flagged() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);

    let branches = git::list_branches(&repo).unwrap();
    let main = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main.is_head);
    assert!(main.is_blocked());
}

#[test]
fn worktree_branch_is_mapped_to_its_path() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);

    let head = repo.head().unwrap().target().unwrap();
    repo.branch("feature-worktree", &repo.find_commit(head).unwrap(), false)
        .unwrap();

    // Add a linked worktree checking out that branch.
    let wt_path = tmp.path.parent().unwrap().join(format!(
        "gbm-test-wt-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&wt_path);
    let mut opts = git2::WorktreeAddOptions::new();
    let reference = repo.find_reference("refs/heads/feature-worktree").unwrap();
    opts.reference(Some(&reference));
    repo.worktree("feature-worktree", &wt_path, Some(&opts))
        .unwrap();

    let branches = git::list_branches(&repo).unwrap();
    let f = branches
        .iter()
        .find(|b| b.name == "feature-worktree")
        .unwrap();
    assert!(
        f.worktree.is_some(),
        "branch checked out in a worktree should carry its path"
    );

    // Cleanup the worktree dir we created outside the TempDir.
    let _ = std::fs::remove_dir_all(&wt_path);
}

#[test]
fn switch_branch_moves_head_and_updates_the_working_tree() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);

    // A branch whose tip changes a.txt and adds b.txt.
    let head = repo.head().unwrap().target().unwrap();
    repo.branch("feature", &repo.find_commit(head).unwrap(), false)
        .unwrap();
    git::switch_branch(&repo, "feature").unwrap();
    write_file(&tmp.path, "a.txt", "changed on feature");
    write_file(&tmp.path, "b.txt", "only on feature");
    commit_all(&repo, "feature work");

    assert_eq!(repo.head().unwrap().shorthand(), Some("feature"));

    // Back to main: the working tree must follow HEAD.
    git::switch_branch(&repo, "main").unwrap();

    assert_eq!(repo.head().unwrap().shorthand(), Some("main"));
    assert_eq!(
        std::fs::read_to_string(tmp.path.join("a.txt")).unwrap(),
        "hello",
        "a.txt should be restored to main's content"
    );
    assert!(
        !tmp.path.join("b.txt").exists(),
        "a file that only exists on feature should be gone"
    );

    let branches = git::list_branches(&repo).unwrap();
    assert!(branches.iter().find(|b| b.name == "main").unwrap().is_head);
    assert!(
        !branches
            .iter()
            .find(|b| b.name == "feature")
            .unwrap()
            .is_head
    );
}

#[test]
fn switch_branch_refuses_to_clobber_uncommitted_changes() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);

    // `feature` rewrites a.txt.
    let head = repo.head().unwrap().target().unwrap();
    repo.branch("feature", &repo.find_commit(head).unwrap(), false)
        .unwrap();
    git::switch_branch(&repo, "feature").unwrap();
    write_file(&tmp.path, "a.txt", "changed on feature");
    commit_all(&repo, "feature work");
    git::switch_branch(&repo, "main").unwrap();

    // Uncommitted edit to the same file the switch would have to overwrite.
    write_file(&tmp.path, "a.txt", "work in progress");

    let err = git::switch_branch(&repo, "feature").unwrap_err();
    assert!(
        err.to_string().contains("feature"),
        "error should name the branch: {err}"
    );
    assert_eq!(
        repo.head().unwrap().shorthand(),
        Some("main"),
        "a refused checkout must leave HEAD where it was"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path.join("a.txt")).unwrap(),
        "work in progress",
        "local work must survive a refused switch"
    );
}

#[test]
fn switch_to_a_missing_branch_errors() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);
    assert!(git::switch_branch(&repo, "no-such-branch").is_err());
    assert_eq!(repo.head().unwrap().shorthand(), Some("main"));
}

#[test]
fn delete_branch_removes_it() {
    let tmp = TempDir::new();
    let repo = init_repo(&tmp.path);
    let head = repo.head().unwrap().target().unwrap();
    repo.branch("to-delete", &repo.find_commit(head).unwrap(), false)
        .unwrap();

    assert!(git::list_branches(&repo)
        .unwrap()
        .iter()
        .any(|b| b.name == "to-delete"));

    git::delete_branch(&repo, "to-delete").unwrap();

    assert!(!git::list_branches(&repo)
        .unwrap()
        .iter()
        .any(|b| b.name == "to-delete"));
}
