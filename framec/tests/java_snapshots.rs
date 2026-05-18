//! RFC-0027 in-tree snapshot tests — java backend.
//!
//! Mirrors python_snapshots.rs against the java target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "java"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "java"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "java"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "java"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "java"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "java"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "java"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "java"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "java"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "java"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "java"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "java"));
}
