//! Integration smoke test for the git layer against a prebuilt throwaway repo
//! (created by the verification step). Confirms merged/unmerged/worktree classification.
//!
//! Run with: GBM_TEST_REPO=<path> cargo test --test git_logic -- --nocapture

#[path = "../src/git.rs"]
mod git;

#[test]
fn classifies_branches() {
    let repo_path = match std::env::var("GBM_TEST_REPO") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("GBM_TEST_REPO not set; skipping");
            return;
        }
    };
    let repo = git2::Repository::open(&repo_path).expect("open test repo");
    let branches = git::list_branches(&repo).expect("list branches");

    for b in &branches {
        println!(
            "{:20} head={:5} merged={:5} worktree={:?}",
            b.name, b.is_head, b.is_merged, b.worktree
        );
    }

    let find = |n: &str| branches.iter().find(|b| b.name == n).expect(n);

    // main is HEAD and merged into itself.
    assert!(find("main").is_head);

    // branched off main with no new commits -> merged.
    assert!(find("feature-merged").is_merged, "feature-merged should be merged");

    // merged back into main via --no-ff -> merged.
    assert!(find("feature-merged-2").is_merged, "feature-merged-2 should be merged");

    // has a commit not on main -> NOT merged (needs force).
    assert!(!find("feature-unmerged").is_merged, "feature-unmerged should be unmerged");

    // checked out in a linked worktree -> worktree path present.
    assert!(find("feature-worktree").worktree.is_some(), "feature-worktree should have a worktree");
}
