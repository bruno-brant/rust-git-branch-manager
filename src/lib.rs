//! Library crate for git-branch-manager. The binary (`main.rs`) and the
//! integration tests both depend on these modules, so the code is compiled once
//! and shared — no duplicate compilation, no spurious dead-code warnings.

pub mod app;
pub mod git;
pub mod ui;
