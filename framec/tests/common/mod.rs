//! Shared test helpers for RFC-0027 in-tree snapshot tests.
//!
//! Cargo convention: files under `tests/common/` are treated as
//! module sources rather than separate integration test binaries
//! (cf. https://doc.rust-lang.org/cargo/reference/cargo-targets.html#integration-tests),
//! so this `mod.rs` can be `mod common;`-imported by each
//! per-backend snapshot test file.

#![allow(dead_code)]

use framec::frame_c::compiler::compile_module;
use framec::frame_c::compiler::TargetLanguage;
use framec::frame_c::utils::RunError;
use std::convert::TryFrom;
use std::path::PathBuf;

/// Load a fixture from `tests/fixtures/<name>.frm` and compile it
/// for the given target language. Returns the generated target
/// code as a String, suitable for `insta::assert_snapshot!`.
///
/// Panics with a useful message if the fixture file is missing or
/// if framec returns an error (snapshot tests assume the fixture
/// itself is valid Frame; a compile error means the fixture has a
/// bug, not the snapshot).
pub fn compile_fixture(fixture_name: &str, target: &str) -> String {
    let lang = TargetLanguage::try_from(target)
        .unwrap_or_else(|e| panic!("unknown target language '{}': {}", target, e));
    let fixture_path = fixture_path(fixture_name);
    let source = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("read fixture {}: {}", fixture_path.display(), e));
    match compile_module(&source, lang) {
        Ok(code) => code,
        Err(RunError { error, .. }) => panic!(
            "framec failed to compile fixture {} for target {}:\n{}",
            fixture_name, target, error
        ),
    }
}

/// Absolute path to a fixture file under `tests/fixtures/`.
fn fixture_path(fixture_name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(format!("{}.frm", fixture_name));
    p
}
