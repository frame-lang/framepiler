//! RFC-0027 in-tree snapshot tests — swift backend.
//!
//! Mirrors python_snapshots.rs against the swift target.
//! Re-bless workflow + corpus discipline documented in
//! CONTRIBUTING.md § "Snapshot tests (RFC-0027)".

mod common;

use common::compile_fixture;

#[test]
fn linear_fsm() {
    insta::assert_snapshot!(compile_fixture("01_linear_fsm", "swift"));
}

#[test]
fn hsm() {
    insta::assert_snapshot!(compile_fixture("02_hsm", "swift"));
}

#[test]
fn persist() {
    insta::assert_snapshot!(compile_fixture("03_persist", "swift"));
}
