# git-branch-manager

A keyboard-driven terminal UI (TUI) for browsing, switching to, and bulk-deleting
local git branches. It understands the difference between a safe delete
(`git branch -d`) and a force delete (`git branch -D`), asks for confirmation
before anything destructive, and handles branches checked out in worktrees by
removing the worktree first.

## Features

- Browse all local branches in a scrolling, paged list that never overflows the
  terminal — works the same with 5 branches or 500.
- **Switch** to the branch under the cursor. The checkout is a safe one: it is
  refused rather than overwriting uncommitted work.
- Switching to a branch that lives in a **worktree** exits and prints that
  worktree's path, so a shell wrapper can `cd` into it (see [Usage](#usage)).
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
| `s` | Switch to the branch under the cursor |
| `Enter` | Delete selected (or the branch under the cursor) |
| `r` | Refresh the branch list |
| `q` / `Esc` | Quit |

In the confirmation prompt: `y` to proceed, `n` / `Esc` to cancel.

## Install

### From a release

Download the asset for your platform from the [Releases](../../releases) page,
extract it, and put the binary on your `PATH`. Assets are named by platform:

| Platform | Asset |
| --- | --- |
| Linux (x86-64) | `git-branch-manager-<version>-linux-x86_64.tar.gz` |
| Windows (x86-64) | `git-branch-manager-<version>-windows-x86_64.zip` |
| macOS (Intel + Apple Silicon) | `git-branch-manager-<version>-macos-universal.tar.gz` |

### Debian / Ubuntu

Each release also ships a `.deb` for x86-64. Download it from the
[Releases](../../releases) page and install with `apt`, which pulls in any
missing system libraries:

```sh
sudo apt install ./git-branch-manager_<version>_amd64.deb
```

`sudo dpkg -i` works too, followed by `sudo apt-get -f install` if anything is
missing. The package installs the binary to `/usr/bin/git-branch-manager`.

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

### Switching branches

`s` checks out the branch under the cursor in the current working tree.

Git allows a branch to be checked out in only one working tree, so if the branch
is already held by a linked worktree, switching to it means going *there*
instead. A process cannot change its parent shell's directory, so the tool exits
and prints the worktree path on stdout. Wrap it in a shell function to make that
a real `cd`:

```sh
gbm() {
  local dir
  dir=$(git-branch-manager) && [ -n "$dir" ] && cd "$dir"
}
```

The UI itself is drawn on stderr, and stdout stays empty in every other case, so
the wrapper still shows the TUI normally and stays put unless a worktree was
picked.

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
