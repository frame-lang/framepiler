//! RFC-0027 in-tree snapshot tests — csharp backend.
//!
//! Mirrors python_snapshots.rs against the csharp target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "csharp"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "csharp"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "csharp"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "csharp"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "csharp"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "csharp"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "csharp"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "csharp"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "csharp"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "csharp"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "csharp"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "csharp"));
}
