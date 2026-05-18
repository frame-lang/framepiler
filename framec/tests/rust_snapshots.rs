//! RFC-0027 in-tree snapshot tests — rust backend.
//!
//! Mirrors python_snapshots.rs against the rust target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "rust"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "rust"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "rust"));
}

#[test]
fn state_args() {
    insta::assert_snapshot!(compile_fixture("04_state_args", "rust"));
}

#[test]
fn pushpop() {
    insta::assert_snapshot!(compile_fixture("05_pushpop", "rust"));
}

#[test]
fn selfcall() {
    insta::assert_snapshot!(compile_fixture("06_selfcall", "rust"));
}

#[test]
fn forward() {
    insta::assert_snapshot!(compile_fixture("07_forward", "rust"));
}

#[test]
fn lifecycle() {
    insta::assert_snapshot!(compile_fixture("08_lifecycle", "rust"));
}

#[test]
fn return_explicit() {
    insta::assert_snapshot!(compile_fixture("09_return_explicit", "rust"));
}

#[test]
fn actions() {
    insta::assert_snapshot!(compile_fixture("10_actions", "rust"));
}

#[test]
fn consts() {
    insta::assert_snapshot!(compile_fixture("11_consts", "rust"));
}

#[test]
fn no_persist() {
    insta::assert_snapshot!(compile_fixture("12_no_persist", "rust"));
}
