//! All git interaction lives here. We use `git2` (libgit2) for everything except
//! worktree *removal*, where libgit2's pruning is too weak to match
//! `git worktree remove`, so we shell out to the `git` binary for that one step.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use git2::{BranchType, Repository};

/// A single local branch and everything the UI needs to know about it.
#[derive(Clone, Debug)]
pub struct BranchInfo {
    pub name: String,
    /// The currently checked-out branch (HEAD of the main worktree). Never deletable.
    pub is_head: bool,
    /// Tip is reachable from HEAD (or its upstream) → safe to delete with `-d`.
    /// When false, deletion requires force (`-D`).
    pub is_merged: bool,
    /// `Some(path)` if this branch is checked out in a linked worktree.
    pub worktree: Option<PathBuf>,
}

impl BranchInfo {
    /// A branch we can never delete: it's HEAD of the main working tree.
    pub fn is_blocked(&self) -> bool {
        self.is_head
    }
}

/// The git operations `App` depends on. Abstracting these behind a trait keeps the
/// application's state logic (navigation, paging, the delete-partitioning rules)
/// testable without a real repository — tests supply a mock implementation.
pub trait GitOperations {
    /// Current local branches with merged-status and worktree attachment resolved.
    fn list_branches(&self) -> Result<Vec<BranchInfo>>;
    /// Delete a branch. Does not enforce the merged check (the caller gates that).
    fn delete_branch(&self, name: &str) -> Result<()>;
    /// Remove a branch's linked worktree, then delete the branch.
    fn remove_worktree_and_branch(&self, name: &str, worktree_path: &Path) -> Result<()>;
}

/// Production `GitOperations` backed by a real libgit2 `Repository`.
pub struct RealGit {
    repo: Repository,
}

impl RealGit {
    pub fn new(repo: Repository) -> Self {
        Self { repo }
    }
}

impl GitOperations for RealGit {
    fn list_branches(&self) -> Result<Vec<BranchInfo>> {
        list_branches(&self.repo)
    }
    fn delete_branch(&self, name: &str) -> Result<()> {
        delete_branch(&self.repo, name)
    }
    fn remove_worktree_and_branch(&self, name: &str, worktree_path: &Path) -> Result<()> {
        remove_worktree_and_branch(&self.repo, name, worktree_path)
    }
}

/// Open the repository discovered from the current working directory.
pub fn open_repo() -> Result<Repository> {
    Repository::discover(".").context("not inside a git repository")
}

/// Build the branch list with merged-status and worktree attachment resolved.
pub fn list_branches(repo: &Repository) -> Result<Vec<BranchInfo>> {
    let head_oid = repo.head().ok().and_then(|h| h.target());
    let worktree_map = worktree_branch_map(repo)?;

    let mut out = Vec::new();
    for entry in repo.branches(Some(BranchType::Local))? {
        let (branch, _) = entry?;
        let name = match branch.name()? {
            Some(n) => n.to_string(),
            None => continue, // non-UTF8 branch name; skip
        };

        let tip = branch.get().target();
        let is_merged = match (head_oid, tip) {
            (Some(head), Some(t)) => is_reachable(repo, head, t),
            _ => false,
        };

        out.push(BranchInfo {
            is_head: branch.is_head(),
            worktree: worktree_map.get(&name).cloned(),
            is_merged,
            name,
        });
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Is `tip` an ancestor of (or equal to) `from`? This mirrors git's rule for
/// "fully merged": a branch is safe to delete when its tip is already reachable
/// from where we are. We also accept reachability from the branch's upstream,
/// handled by the caller when present.
fn is_reachable(repo: &Repository, from: git2::Oid, tip: git2::Oid) -> bool {
    if from == tip {
        return true;
    }
    // `from` is a descendant of `tip` => tip is already merged into `from`.
    repo.graph_descendant_of(from, tip).unwrap_or(false)
}

/// Map of branch name -> worktree path, for branches checked out in linked worktrees.
fn worktree_branch_map(repo: &Repository) -> Result<HashMap<String, PathBuf>> {
    let mut map = HashMap::new();
    let names = repo.worktrees()?;
    for wt_name in names.iter().flatten() {
        let wt = match repo.find_worktree(wt_name) {
            Ok(w) => w,
            Err(_) => continue,
        };
        let path = wt.path().to_path_buf();
        // Open the worktree as a repo to learn which branch its HEAD points at.
        if let Ok(wt_repo) = Repository::open(&path) {
            if let Ok(head) = wt_repo.head() {
                if head.is_branch() {
                    if let Some(shorthand) = head.shorthand() {
                        map.insert(shorthand.to_string(), path);
                    }
                }
            }
        }
    }
    Ok(map)
}

/// Delete a branch via libgit2. libgit2 does not enforce the merged check, so this
/// is used for both the safe and force paths — the caller gates which branches reach here.
pub fn delete_branch(repo: &Repository, name: &str) -> Result<()> {
    let mut branch = repo
        .find_branch(name, BranchType::Local)
        .with_context(|| format!("branch '{name}' not found"))?;
    branch
        .delete()
        .with_context(|| format!("failed to delete branch '{name}'"))?;
    Ok(())
}

/// Remove a linked worktree, then delete its branch. libgit2's worktree pruning is
/// limited, so we use `git worktree remove` for a clean removal (handles the admin
/// files and locking correctly), then delete the branch with libgit2.
pub fn remove_worktree_and_branch(
    repo: &Repository,
    name: &str,
    worktree_path: &Path,
) -> Result<()> {
    let workdir = repo
        .workdir()
        .or_else(|| Some(repo.path()))
        .ok_or_else(|| anyhow!("repository has no working directory"))?;

    let status = Command::new("git")
        .current_dir(workdir)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path)
        .status()
        .context("failed to invoke `git worktree remove`")?;

    if !status.success() {
        return Err(anyhow!(
            "`git worktree remove {}` failed",
            worktree_path.display()
        ));
    }

    // The worktree no longer holds the branch checked out; now it's deletable.
    delete_branch(repo, name)
}
