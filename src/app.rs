//! Application state and the logic that drives it: navigation, selection, and the
//! partitioning of a delete request into safe / force / worktree buckets.

use anyhow::Result;

use crate::git::{BranchInfo, GitOperations};

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

pub struct App<G: GitOperations> {
    git: G,
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

impl<G: GitOperations> App<G> {
    pub fn new(git: G) -> Result<Self> {
        let branches = git.list_branches()?;
        let selected = vec![false; branches.len()];
        Ok(Self {
            git,
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
        self.branches = self.git.list_branches()?;
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
            match self.git.delete_branch(&name) {
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
            match self.git.delete_branch(&name) {
                Ok(()) => deleted += 1,
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        for &i in &pending.worktree {
            let name = self.branches[i].name.clone();
            let path = self.branches[i].worktree.clone().unwrap();
            match self.git.remove_worktree_and_branch(&name, &path) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    /// In-memory `GitOperations` mock. Holds a mutable branch list so that a delete
    /// followed by `refresh()` reflects the new state, exactly like the real repo.
    /// Records every delete/worktree-removal call for assertions, and can be told to
    /// fail deletion of specific branches to exercise error reporting.
    struct MockGit {
        branches: RefCell<Vec<BranchInfo>>,
        deleted: RefCell<Vec<String>>,
        worktrees_removed: RefCell<Vec<String>>,
        fail_on: Vec<String>,
    }

    impl MockGit {
        fn new(branches: Vec<BranchInfo>) -> Self {
            Self {
                branches: RefCell::new(branches),
                deleted: RefCell::new(Vec::new()),
                worktrees_removed: RefCell::new(Vec::new()),
                fail_on: Vec::new(),
            }
        }

        fn failing_on(mut self, names: &[&str]) -> Self {
            self.fail_on = names.iter().map(|s| s.to_string()).collect();
            self
        }

        fn remove_branch(&self, name: &str) {
            self.branches.borrow_mut().retain(|b| b.name != name);
        }
    }

    impl GitOperations for MockGit {
        fn list_branches(&self) -> Result<Vec<BranchInfo>> {
            Ok(self.branches.borrow().clone())
        }
        fn delete_branch(&self, name: &str) -> Result<()> {
            if self.fail_on.iter().any(|n| n == name) {
                return Err(anyhow!("simulated failure"));
            }
            self.deleted.borrow_mut().push(name.to_string());
            self.remove_branch(name);
            Ok(())
        }
        fn remove_worktree_and_branch(&self, name: &str, _path: &Path) -> Result<()> {
            if self.fail_on.iter().any(|n| n == name) {
                return Err(anyhow!("simulated failure"));
            }
            self.worktrees_removed.borrow_mut().push(name.to_string());
            self.deleted.borrow_mut().push(name.to_string());
            self.remove_branch(name);
            Ok(())
        }
    }

    // --- Builders ---------------------------------------------------------

    fn branch(name: &str) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            is_head: false,
            is_merged: true,
            worktree: None,
        }
    }
    fn head(name: &str) -> BranchInfo {
        BranchInfo {
            is_head: true,
            ..branch(name)
        }
    }
    fn unmerged(name: &str) -> BranchInfo {
        BranchInfo {
            is_merged: false,
            ..branch(name)
        }
    }
    fn worktree(name: &str, path: &str) -> BranchInfo {
        BranchInfo {
            worktree: Some(PathBuf::from(path)),
            ..branch(name)
        }
    }

    fn app_with(branches: Vec<BranchInfo>) -> App<MockGit> {
        App::new(MockGit::new(branches)).unwrap()
    }

    // --- Navigation -------------------------------------------------------

    #[test]
    fn move_down_and_up_clamp_at_bounds() {
        let mut app = app_with(vec![branch("a"), branch("b"), branch("c")]);
        assert_eq!(app.cursor, 0);
        app.move_up(1); // already at top
        assert_eq!(app.cursor, 0);
        app.move_down(10); // past the end
        assert_eq!(app.cursor, 2);
        app.move_up(1);
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn navigation_on_empty_list_is_safe() {
        let mut app = app_with(vec![]);
        app.move_down(5);
        app.move_up(5);
        assert_eq!(app.cursor, 0);
    }

    // --- Paging / offset --------------------------------------------------

    #[test]
    fn offset_follows_cursor_below_the_viewport() {
        // 10 branches, viewport of 3 rows.
        let mut app = app_with((0..10).map(|i| branch(&format!("b{i}"))).collect());
        app.cursor = 5;
        app.clamp_offset_to_cursor(3);
        // cursor 5 must be visible in [offset, offset+3): offset = 5+1-3 = 3
        assert_eq!(app.offset, 3);
        assert!(app.cursor >= app.offset && app.cursor < app.offset + 3);
    }

    #[test]
    fn offset_follows_cursor_above_the_viewport() {
        let mut app = app_with((0..10).map(|i| branch(&format!("b{i}"))).collect());
        app.offset = 5;
        app.cursor = 2;
        app.clamp_offset_to_cursor(3);
        assert_eq!(app.offset, 2);
    }

    #[test]
    fn offset_never_leaves_blank_space_at_bottom() {
        let mut app = app_with((0..5).map(|i| branch(&format!("b{i}"))).collect());
        app.offset = 99; // nonsense large offset
        app.cursor = 4;
        app.clamp_offset_to_cursor(3);
        // max_offset = 5 - 3 = 2
        assert_eq!(app.offset, 2);
    }

    #[test]
    fn zero_height_viewport_does_not_panic() {
        let mut app = app_with(vec![branch("a")]);
        app.clamp_offset_to_cursor(0); // must be a no-op, no divide/underflow
        assert_eq!(app.offset, 0);
    }

    // --- Selection --------------------------------------------------------

    #[test]
    fn space_toggles_selection() {
        let mut app = app_with(vec![branch("a"), branch("b")]);
        app.cursor = 1;
        app.toggle_selection();
        assert!(app.selected[1]);
        app.toggle_selection();
        assert!(!app.selected[1]);
    }

    #[test]
    fn head_branch_cannot_be_selected() {
        let mut app = app_with(vec![head("main"), branch("a")]);
        app.cursor = 0;
        app.toggle_selection();
        assert!(!app.selected[0], "HEAD must not become selected");
        assert!(app.status.contains("current branch"));
    }

    // --- Delete partitioning ---------------------------------------------

    #[test]
    fn enter_with_no_selection_deletes_branch_under_cursor() {
        let mut app = app_with(vec![branch("a"), branch("b")]);
        app.cursor = 1;
        app.request_delete().unwrap();
        assert_eq!(*app.git.deleted.borrow(), vec!["b"]);
        assert!(matches!(app.mode, Mode::Browsing));
    }

    #[test]
    fn enter_on_head_with_no_selection_deletes_nothing() {
        let mut app = app_with(vec![head("main")]);
        app.cursor = 0;
        app.request_delete().unwrap();
        assert!(app.git.deleted.borrow().is_empty());
        assert!(app.status.contains("nothing to delete"));
    }

    #[test]
    fn merged_branches_delete_immediately_without_confirm() {
        let mut app = app_with(vec![branch("a"), branch("b"), branch("c")]);
        app.selected = vec![true, false, true];
        app.request_delete().unwrap();
        assert!(
            matches!(app.mode, Mode::Browsing),
            "no confirm needed for merged"
        );
        assert_eq!(*app.git.deleted.borrow(), vec!["a", "c"]);
        // refresh ran: deleted branches gone from the list.
        assert_eq!(app.branches.len(), 1);
        assert_eq!(app.branches[0].name, "b");
    }

    #[test]
    fn unmerged_branch_requires_confirmation() {
        let mut app = app_with(vec![branch("a"), unmerged("danger")]);
        app.selected = vec![true, true];
        app.request_delete().unwrap();
        // merged "a" already deleted; unmerged "danger" parked in confirm.
        assert_eq!(*app.git.deleted.borrow(), vec!["a"]);
        match &app.mode {
            Mode::Confirm(p) => {
                assert_eq!(p.force.len(), 1);
                assert!(p.worktree.is_empty());
            }
            _ => panic!("expected a Confirm modal"),
        }
    }

    #[test]
    fn confirm_yes_force_deletes_the_unmerged() {
        let mut app = app_with(vec![unmerged("danger")]);
        app.selected = vec![true];
        app.request_delete().unwrap();
        assert!(matches!(app.mode, Mode::Confirm(_)));
        app.confirm_delete().unwrap();
        assert_eq!(*app.git.deleted.borrow(), vec!["danger"]);
        assert!(matches!(app.mode, Mode::Browsing));
        assert!(app.branches.is_empty());
    }

    #[test]
    fn confirm_no_cancels_and_keeps_branch() {
        let mut app = app_with(vec![unmerged("danger")]);
        app.selected = vec![true];
        app.request_delete().unwrap();
        app.cancel_confirm();
        assert!(app.git.deleted.borrow().is_empty());
        assert!(matches!(app.mode, Mode::Browsing));
        assert!(app.status.contains("cancelled"));
    }

    #[test]
    fn worktree_branch_routes_to_confirm_and_removes_worktree_on_yes() {
        let mut app = app_with(vec![worktree("wt", "/tmp/wt")]);
        app.selected = vec![true];
        app.request_delete().unwrap();
        match &app.mode {
            Mode::Confirm(p) => {
                assert_eq!(p.worktree.len(), 1);
                assert!(p.force.is_empty());
            }
            _ => panic!("expected confirm for worktree branch"),
        }
        app.confirm_delete().unwrap();
        assert_eq!(*app.git.worktrees_removed.borrow(), vec!["wt"]);
        assert_eq!(*app.git.deleted.borrow(), vec!["wt"]);
    }

    #[test]
    fn mixed_batch_partitions_into_safe_force_and_worktree() {
        let mut app = app_with(vec![
            head("main"),
            branch("merged"),
            unmerged("force-me"),
            worktree("wt", "/tmp/wt"),
        ]);
        // select everything (HEAD selection is ignored by request_delete anyway)
        app.selected = vec![true, true, true, true];
        app.request_delete().unwrap();
        // safe one deleted now
        assert_eq!(*app.git.deleted.borrow(), vec!["merged"]);
        match &app.mode {
            Mode::Confirm(p) => {
                assert_eq!(p.force.len(), 1);
                assert_eq!(p.worktree.len(), 1);
            }
            _ => panic!("expected confirm"),
        }
    }

    #[test]
    fn delete_errors_are_reported_in_status() {
        let git = MockGit::new(vec![branch("a"), branch("b")]).failing_on(&["b"]);
        let mut app = App::new(git).unwrap();
        app.selected = vec![true, true];
        app.request_delete().unwrap();
        // "a" deleted, "b" failed.
        assert_eq!(*app.git.deleted.borrow(), vec!["a"]);
        assert!(app.status.contains("error"), "status: {}", app.status);
    }

    #[test]
    fn refresh_clears_selection_and_clamps_cursor() {
        let mut app = app_with(vec![branch("a"), branch("b"), branch("c")]);
        app.cursor = 2;
        app.selected = vec![true, true, true];
        // shrink the underlying repo to a single branch, then refresh.
        *app.git.branches.borrow_mut() = vec![branch("only")];
        app.refresh().unwrap();
        assert_eq!(app.branches.len(), 1);
        assert_eq!(app.cursor, 0);
        assert!(app.selected.iter().all(|&s| !s));
    }
}
