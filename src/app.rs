//! Application state and the logic that drives it: navigation, selection, and the
//! partitioning of a delete request into safe / force / worktree buckets.

use anyhow::Result;
use git2::Repository;

use crate::git::{self, BranchInfo};

/// What the UI is currently showing / waiting on.
pub enum Mode {
    /// Normal list navigation.
    Browsing,
    /// A confirmation modal is up because some selected branches need force-delete
    /// and/or have worktrees that must be removed first.
    Confirm(PendingDelete),
}

/// The set of branches a delete action will act on, already partitioned and
/// awaiting user confirmation for the unsafe parts.
pub struct PendingDelete {
    /// Indices needing force delete (`-D`): not fully merged.
    pub force: Vec<usize>,
    /// Indices that are checked out in a worktree (worktree removed, then branch deleted).
    pub worktree: Vec<usize>,
}

pub struct App {
    repo: Repository,
    pub branches: Vec<BranchInfo>,
    pub selected: Vec<bool>,
    pub cursor: usize,
    /// First visible row index — the scroll offset that drives paging.
    pub offset: usize,
    pub mode: Mode,
    /// Transient one-line message shown in the status bar (result of the last action).
    pub status: String,
    pub should_quit: bool,
}

impl App {
    pub fn new(repo: Repository) -> Result<Self> {
        let branches = git::list_branches(&repo)?;
        let selected = vec![false; branches.len()];
        Ok(Self {
            repo,
            branches,
            selected,
            cursor: 0,
            offset: 0,
            mode: Mode::Browsing,
            status: String::new(),
            should_quit: false,
        })
    }

    /// Re-read branches from disk and reset selection, keeping the cursor in range.
    pub fn refresh(&mut self) -> Result<()> {
        self.branches = git::list_branches(&self.repo)?;
        self.selected = vec![false; self.branches.len()];
        if self.cursor >= self.branches.len() {
            self.cursor = self.branches.len().saturating_sub(1);
        }
        self.clamp_offset_to_cursor(usize::MAX);
        Ok(())
    }

    // --- Navigation -------------------------------------------------------

    pub fn move_down(&mut self, n: usize) {
        if self.branches.is_empty() {
            return;
        }
        self.cursor = (self.cursor + n).min(self.branches.len() - 1);
    }

    pub fn move_up(&mut self, n: usize) {
        self.cursor = self.cursor.saturating_sub(n);
    }

    /// Keep `offset` such that the cursor stays within the visible window of
    /// `view_height` rows. Called by the renderer once it knows the area height.
    pub fn clamp_offset_to_cursor(&mut self, view_height: usize) {
        if view_height == 0 {
            return;
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + view_height {
            self.offset = self.cursor + 1 - view_height;
        }
        // Don't leave blank space at the bottom when the list shrank.
        let max_offset = self.branches.len().saturating_sub(view_height);
        if self.offset > max_offset {
            self.offset = max_offset;
        }
    }

    // --- Selection --------------------------------------------------------

    /// Toggle selection on the branch under the cursor. HEAD/blocked branches
    /// cannot be selected.
    pub fn toggle_selection(&mut self) {
        if let Some(b) = self.branches.get(self.cursor) {
            if b.is_blocked() {
                self.status = format!("'{}' is the current branch and can't be deleted", b.name);
                return;
            }
            self.selected[self.cursor] = !self.selected[self.cursor];
        }
    }

    /// The indices the next delete will act on: explicitly selected branches, or —
    /// if nothing is selected — the branch under the cursor (when deletable).
    fn target_indices(&self) -> Vec<usize> {
        let explicit: Vec<usize> = self
            .selected
            .iter()
            .enumerate()
            .filter(|(_, &s)| s)
            .map(|(i, _)| i)
            .collect();
        if !explicit.is_empty() {
            return explicit;
        }
        match self.branches.get(self.cursor) {
            Some(b) if !b.is_blocked() => vec![self.cursor],
            _ => Vec::new(),
        }
    }

    // --- Delete flow ------------------------------------------------------

    /// Triggered by Enter. Deletes everything safe immediately; if anything needs
    /// force or worktree removal, stash it in a `PendingDelete` and switch to the
    /// confirmation modal. Returns without prompting if nothing is left over.
    pub fn request_delete(&mut self) -> Result<()> {
        let targets = self.target_indices();
        if targets.is_empty() {
            self.status = "nothing to delete (select branches with Space)".into();
            return Ok(());
        }

        let mut safe = Vec::new();
        let mut force = Vec::new();
        let mut worktree = Vec::new();
        let mut blocked = 0;

        for i in targets {
            let b = &self.branches[i];
            if b.is_blocked() {
                blocked += 1;
            } else if b.worktree.is_some() {
                worktree.push(i);
            } else if b.is_merged {
                safe.push(i);
            } else {
                force.push(i);
            }
        }

        // Delete the safe ones right away.
        let mut deleted = 0;
        let mut errors = Vec::new();
        for &i in &safe {
            let name = self.branches[i].name.clone();
            match git::delete_branch(&self.repo, &name) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }

        if force.is_empty() && worktree.is_empty() {
            self.finish_delete(deleted, blocked, errors)?;
        } else {
            // Remaining unsafe work needs confirmation.
            if deleted > 0 {
                self.status = format!("deleted {deleted} merged branch(es); confirming the rest…");
            }
            self.mode = Mode::Confirm(PendingDelete { force, worktree });
        }
        Ok(())
    }

    /// User answered `y` in the confirmation modal: force-delete the unmerged ones
    /// and remove worktrees (then delete those branches).
    pub fn confirm_delete(&mut self) -> Result<()> {
        let pending = match std::mem::replace(&mut self.mode, Mode::Browsing) {
            Mode::Confirm(p) => p,
            other => {
                self.mode = other;
                return Ok(());
            }
        };

        let mut deleted = 0;
        let mut errors = Vec::new();

        for &i in &pending.force {
            let name = self.branches[i].name.clone();
            match git::delete_branch(&self.repo, &name) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        for &i in &pending.worktree {
            let name = self.branches[i].name.clone();
            let path = self.branches[i].worktree.clone().unwrap();
            match git::remove_worktree_and_branch(&self.repo, &name, &path) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }

        self.finish_delete(deleted, 0, errors)
    }

    /// User answered `n`/Esc in the modal.
    pub fn cancel_confirm(&mut self) {
        self.mode = Mode::Browsing;
        self.status = "cancelled".into();
    }

    fn finish_delete(&mut self, deleted: usize, blocked: usize, errors: Vec<String>) -> Result<()> {
        self.refresh()?;
        let mut parts = vec![format!("deleted {deleted} branch(es)")];
        if blocked > 0 {
            parts.push(format!("{blocked} skipped (current branch)"));
        }
        if !errors.is_empty() {
            parts.push(format!("{} error(s): {}", errors.len(), errors.join("; ")));
        }
        self.status = parts.join(" — ");
        Ok(())
    }
}
