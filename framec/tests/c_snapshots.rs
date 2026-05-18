//! RFC-0027 in-tree snapshot tests — c backend.
//!
//! Mirrors python_snapshots.rs against the c target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "c"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "c"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "c"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "c"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "c"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "c"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "c"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "c"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "c"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "c"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "c"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "c"));
}
