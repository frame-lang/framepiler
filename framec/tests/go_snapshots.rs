//! RFC-0027 in-tree snapshot tests — go backend.
//!
//! Mirrors python_snapshots.rs against the go target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "go"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "go"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "go"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "go"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "go"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "go"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "go"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "go"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "go"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "go"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "go"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "go"));
}
