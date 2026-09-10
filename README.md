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

| Platform | Asset | Requires |
| --- | --- | --- |
| Linux (x86-64) | `git-branch-manager-<version>-linux-x86_64.tar.gz` | glibc 2.34+ |
| Linux (x86-64, static) | `git-branch-manager-<version>-linux-x86_64-static.tar.gz` | nothing |
| Windows (x86-64) | `git-branch-manager-<version>-windows-x86_64.zip` | — |
| macOS (Intel + Apple Silicon) | `git-branch-manager-<version>-macos-universal.tar.gz` | — |

### Which Linux build?

Both contain the same program; they differ only in how they link the C library.

- **`linux-x86_64`** is dynamically linked against glibc. glibc is backward
  compatible but not forward compatible, so a binary runs on the glibc it was
  built against and anything newer — never on anything older, where it fails at
  startup with `version 'GLIBC_x.yz' not found`. This one is built on Ubuntu
  22.04 to keep that floor low: it needs **glibc 2.34 or newer**, which covers
  Debian 12+, Ubuntu 22.04+, and RHEL 9+.
- **`linux-x86_64-static`** is statically linked against musl. It has no
  external dependencies at all — one file, runs on any x86-64 Linux regardless
  of age or distribution. Slightly larger, and that's the only cost.

If you don't want to think about it, take the static one.

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

### Commit messages

Commit subjects start with a [gitmoji](https://gitmoji.dev). The emoji is not
decoration — [`cliff.toml`](cliff.toml) maps it to a
[conventional commit](https://www.conventionalcommits.org) type, which decides
both the changelog section a commit lands in and how the version is bumped:

| Emoji | Means | Version effect |
| --- | --- | --- |
| 💥 | Breaking change | major (minor while 0.x) |
| ✨ | New feature | minor |
| 🐛 🚑️ 🩹 🔒️ | Bug fix | patch |
| ⚡️ | Performance | patch |
| 📝 ♻️ ✅ 📦 ⬆️ ⬇️ 📌 👷 💚 | Docs, refactor, tests, build, deps, CI | patch |

The `:shortcode:` spelling (`:sparkles:`) works too. See `cliff.toml` for the
full mapping; anything unrecognised is left out of the changelog.

### Releasing

Releases are cut by merging a pull request, not by tagging by hand:

1. Every push to `main` runs [`release-plz.yml`](.github/workflows/release-plz.yml),
   which opens or updates a **release PR** bumping the version in `Cargo.toml`
   and rewriting `CHANGELOG.md` from the commits since the last tag.
2. Review that PR. The proposed version comes from the commit history — edit it
   in the PR if you disagree.
3. Merge it. The next run of the same workflow pushes the `vX.Y.Z` tag, which
   triggers [`release.yml`](.github/workflows/release.yml) to build every
   platform and publish the GitHub Release with the binaries attached.

The version is derived from the raw commit subjects, so it is worth knowing what
each emoji does. Measured behaviour:

| Commit | While `0.x` | From `1.0` on |
| --- | --- | --- |
| `💥` | `0.1.1` → `0.2.0` | `1.0.0` → `2.0.0` |
| `✨` | `0.1.1` → `0.2.0` | `1.0.0` → `1.1.0` |
| anything else | `0.1.1` → `0.1.2` | `1.0.0` → `1.0.1` |

Below `1.0`, release-plz will not promote the crate to `1.0.0` on its own — that
stays a deliberate decision, made by editing the version in the release PR.

#### Testing the build without releasing

`release.yml` can be run by hand — **Actions → Release → Run workflow** — which
builds every platform and attaches the archives to that run, publishing nothing.
The publish job is skipped for anything but a tag push. Use it after changing
the release workflow, since normal CI never exercises it; the archives are named
after the branch (`git-branch-manager-main-linux-x86_64.tar.gz`) so they cannot
be mistaken for release assets.

To confirm the glibc floor on the Linux build, download that run's
`linux-x86_64` archive and check the highest version it demands:

```sh
objdump -T git-branch-manager | grep -o 'GLIBC_[0-9.]*' | sort -V | tail -1
# GLIBC_2.34
```

This needs a `RELEASE_PLZ_TOKEN` repository secret: a fine-grained personal
access token scoped to this repository with **Contents: read & write** and
**Pull requests: read & write**. It cannot be the default `GITHUB_TOKEN` —
GitHub does not start workflow runs for events raised by that token, so the tag
would never trigger the release build and the release would be published with
no binaries.

Version numbers follow semver, where the "public API" of a binary is what users
and their scripts depend on: the keybindings, and the stdout contract that makes
`cd "$(git-branch-manager)"` work. Changing either is a breaking change.
