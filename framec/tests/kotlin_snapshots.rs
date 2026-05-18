//! RFC-0027 in-tree snapshot tests — kotlin backend.
//!
//! Mirrors python_snapshots.rs against the kotlin target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "kotlin"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "kotlin"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "kotlin"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "kotlin"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "kotlin"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "kotlin"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "kotlin"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "kotlin"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "kotlin"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "kotlin"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "kotlin"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "kotlin"));
}
