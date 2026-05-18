//! RFC-0027 in-tree snapshot tests — dart backend.
//!
//! Mirrors python_snapshots.rs against the dart target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "dart"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "dart"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "dart"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "dart"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "dart"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "dart"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "dart"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "dart"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "dart"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "dart"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "dart"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "dart"));
}
