# Changelog

All notable changes to this project are documented here.
## [0.3.0] - 2026-09-15


### ✨ Features

- Add install scripts for macOS, Linux and Windows


### 📝 Documentation

- Document downloading and unpacking the release binaries
- Resolve the latest version in the install snippets



## [0.2.0] - 2026-09-10


### ✨ Features

- Add branch switching with worktree awareness


### 👷 CI

- Cut releases with release-plz, driven by gitmoji
- Allow running the release build by hand, without publishing
- Fix the release-plz action reference


### 📌 Dependencies

- Pin cargo-deb to 3.x in the release workflow
- Build the glibc Linux binary on Ubuntu 22.04 to lower its floor


### 📦 Build & packaging

- Add Debian package to releases and name assets by platform
- Ship a static musl Linux build; drop Debian packaging


