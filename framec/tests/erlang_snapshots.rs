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
