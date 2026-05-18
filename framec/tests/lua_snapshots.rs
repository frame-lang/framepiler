//! RFC-0027 in-tree snapshot tests — lua backend.
//!
//! Mirrors python_snapshots.rs against the lua target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "lua"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "lua"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "lua"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "lua"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "lua"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "lua"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "lua"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "lua"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "lua"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "lua"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "lua"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "lua"));
}
