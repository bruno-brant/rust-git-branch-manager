# git-branch-manager

A keyboard-driven terminal UI (TUI) for browsing and bulk-deleting local git
branches. It understands the difference between a safe delete (`git branch -d`)
and a force delete (`git branch -D`), asks for confirmation before anything
destructive, and handles branches checked out in worktrees by removing the
worktree first.

## Features

- Browse all local branches in a scrolling, paged list that never overflows the
  terminal — works the same with 5 branches or 500.
- Multi-select branches and delete them in a batch.
- Merged branches are deleted immediately; **not-fully-merged** branches are
  collected into a single confirmation prompt before force-deletion.
- Branches checked out in a **worktree** are detected and marked; deleting one
  removes the worktree first, then the branch.
- The current branch (`HEAD`) is shown but protected from deletion.

## Keybindings

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Move cursor |
| `PgUp` / `PgDn` | Page up / down |
| `Home` / `End` | Jump to first / last branch |
| `Space` | Toggle selection |
| `Enter` | Delete selected (or the branch under the cursor) |
| `r` | Refresh the branch list |
| `q` / `Esc` | Quit |

In the confirmation prompt: `y` to proceed, `n` / `Esc` to cancel.

## Install

### From a release

Download the archive for your platform from the
[Releases](../../releases) page, extract it, and put the binary on your `PATH`.

### From source

```sh
cargo install --path .
```

Requires a Rust toolchain and a C compiler/linker for the `git2` (libgit2)
native build:

- **Linux/macOS**: a system C toolchain (`build-essential` / Xcode CLT).
- **Windows (MSVC)**: the "Desktop development with C++" workload (provides
  `cl.exe`, `link.exe`, and the Windows SDK).
- **Windows (GNU)**: a MinGW-w64 toolchain on `PATH` (e.g. `scoop install mingw`).

## Usage

Run it from inside any git repository:

```sh
git-branch-manager
```

## Development

```sh
cargo build
cargo test          # unit tests (app logic) + integration tests (real git repos)
cargo fmt --all
cargo clippy --all-targets
```

The application logic in [`src/app.rs`](src/app.rs) is decoupled from git via the
`GitOperations` trait, so it is unit-tested against an in-memory mock. The git
layer in [`src/git.rs`](src/git.rs) is covered by integration tests that build
throwaway repositories with `git2`.
