//! RFC-0027 in-tree snapshot tests — javascript backend.
//!
//! Mirrors python_snapshots.rs against the javascript target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "javascript"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "javascript"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "javascript"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "javascript"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "javascript"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "javascript"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "javascript"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "javascript"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "javascript"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "javascript"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "javascript"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "javascript"));
}
