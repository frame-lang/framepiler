// Rust syntax skipper — Frame-generated state machine.
//
// Source: rust_skipper.frs (Frame specification)
// Generated: rust_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec compile -l rust -o /tmp framec/src/frame_c/compiler/native_region_scanner/rust_skipper.frs
//   cp /tmp/rust_skipper.rs framec/src/frame_c/compiler/native_region_scanner/rust_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("rust_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::rust::BodyCloserRust;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerRust;

/// Rust syntax skipper - handles //, nested /* */, strings, and raw strings r#"..."#
pub struct RustSkipper;

impl SyntaxSkipper for RustSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserRust)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = RustSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = i;
        fsm.end = end;
        fsm.do_skip_comment();
        if fsm.success != 0 {
            Some(fsm.result_pos)
        } else {
            None
        }
    }

    fn skip_string(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = RustSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = i;
        fsm.end = end;
        fsm.do_skip_string();
        if fsm.success != 0 {
            Some(fsm.result_pos)
        } else {
            None
        }
    }

    fn find_line_end(&self, bytes: &[u8], start: usize, end: usize) -> usize {
        let mut fsm = RustSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = RustSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = i;
        fsm.end = end;
        fsm.do_balanced_paren_end();
        if fsm.success != 0 {
            Some(fsm.result_pos)
        } else {
            None
        }
    }

    fn skip_nested_scope(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        // Rust closure with block body: `|args| { body }` (also
        // `move |args| { body }`). Trigger on `|`. We must be careful
        // not to swallow a boolean OR (`a || b`) — the closure shape
        // is `|` directly followed by 0+ args then a closing `|` and
        // a `{`. Boolean `||` is two adjacent `|` with no args
        // between, but the *next* token after `||` is an expression,
        // not a `{`. Logical `|` (bitwise) takes operands too, never
        // followed by `{` syntactically.
        //
        // Match strategy:
        //   `|` at position i, find matching `|` at depth 0
        //   (skipping nested `(...)` and `<...>` for type ascriptions),
        //   then whitespace, then `{`. Return matching `}` + 1.
        if bytes[i] != b'|' {
            return None;
        }
        let mut j = i + 1;
        let mut paren = 0i32;
        let mut angle = 0i32;
        while j < end {
            match bytes[j] {
                b'(' => paren += 1,
                b')' => paren -= 1,
                b'<' if paren == 0 => angle += 1,
                b'>' if paren == 0 && angle > 0 => angle -= 1,
                b'|' if paren == 0 && angle == 0 => break,
                b'\n' if paren == 0 && angle == 0 => return None,
                _ => {}
            }
            j += 1;
        }
        if j >= end || bytes[j] != b'|' {
            return None;
        }
        let mut k = j + 1;
        while k < end && matches!(bytes[k], b' ' | b'\t') {
            k += 1;
        }
        if k >= end || bytes[k] != b'{' {
            return None;
        }
        let mut closer = BodyCloserRust;
        closer.close_byte(bytes, k).ok().map(|c| c + 1)
    }
}

impl NativeRegionScanner for NativeRegionScannerRust {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&RustSkipper, bytes, open_brace_index)
    }
}
