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
