//! RFC-0027 in-tree snapshot tests — erlang backend.
//!
//! Mirrors python_snapshots.rs against the erlang target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "erlang"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "erlang"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "erlang"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "erlang"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "erlang"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "erlang"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "erlang"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "erlang"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "erlang"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "erlang"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "erlang"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "erlang"));
}
