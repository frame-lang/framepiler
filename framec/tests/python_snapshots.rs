//! RFC-0027 in-tree snapshot tests — Python backend.
//!
//! Snapshots the framec-emitted Python code for the canonical
//! 3-fixture corpus. Changes to the Python backend produce
//! reviewable `.snap` diffs in PRs.
//!
//! Re-bless workflow when an intentional codegen change is made:
//!   cargo install cargo-insta   # one-time
//!   cargo test --test python_snapshots
//!   cargo insta review
//!   git add tests/snapshots/ && git commit
//!
//! Adding a backend: copy this file to e.g. `java_snapshots.rs`
//! and change the target string in each call. Phase 2 of RFC-0027
//! rolls this out to the remaining 16 backends.

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "python_3"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "python_3"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "python_3"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "python_3"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "python_3"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "python_3"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "python_3"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "python_3"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "python_3"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "python_3"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "python_3"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "python_3"));
}
