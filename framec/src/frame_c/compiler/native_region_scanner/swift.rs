// Swift syntax skipper — Frame-generated state machine.
//
// Source: swift_skipper.frs (Frame specification)
// Generated: swift_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec framec/src/frame_c/compiler/native_region_scanner/swift_skipper.frs -l rust > framec/src/frame_c/compiler/native_region_scanner/swift_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("swift_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::swift::BodyCloserSwift;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerSwift;

/// Swift syntax skipper - handles //, /* */ (nestable), strings, and multi-line strings """..."""
pub struct SwiftSkipper;

impl SyntaxSkipper for SwiftSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserSwift)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = SwiftSyntaxSkipperFsm::new();
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
        let mut fsm = SwiftSyntaxSkipperFsm::new();
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
        let mut fsm = SwiftSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = SwiftSyntaxSkipperFsm::new();
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

    fn string_interp_regions(
        &self,
        bytes: &[u8],
        i: usize,
        end: usize,
    ) -> Option<(usize, Vec<InterpRegion>)> {
        scan_paren_string_regions(bytes, i, end)
    }

    fn skip_nested_scope(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        // Swift closure: `{ args in body }`. Trigger on `{` and look
        // for ` in ` keyword at depth 0 before any newline/`{`/`}`.
        // Mirror Kotlin's first-char gating to avoid flagging
        // control-flow bodies (`if cond { ... }`) — first non-ws byte
        // must be an identifier start or `(` (tuple destructuring).
        //
        // We do NOT detect Swift's positional-shorthand form
        // (`{ $0 + $1 }`) because it has no `in` keyword and is
        // ambiguous with regular control-flow bodies in this scanner.
        // A Frame statement inside such a closure would slip past;
        // documented as a known limitation.
        if bytes[i] != b'{' {
            return None;
        }
        let mut j = i + 1;
        while j < end && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if j >= end {
            return None;
        }
        let first = bytes[j];
        if !(first.is_ascii_alphabetic() || first == b'_' || first == b'(') {
            return None;
        }
        let mut paren = 0i32;
        while j + 3 < end {
            if let Some(after) = self.skip_string(bytes, j, end) {
                j = after;
                continue;
            }
            if let Some(after) = self.skip_comment(bytes, j, end) {
                j = after;
                continue;
            }
            let b = bytes[j];
            match b {
                b'(' => paren += 1,
                b')' => paren -= 1,
                b'i' if paren == 0
                    && bytes[j + 1] == b'n'
                    && (j + 2 >= end
                        || !bytes[j + 2].is_ascii_alphanumeric() && bytes[j + 2] != b'_')
                    && (j == 0
                        || !bytes[j - 1].is_ascii_alphanumeric() && bytes[j - 1] != b'_') =>
                {
                    let mut closer = BodyCloserSwift;
                    return closer.close_byte(bytes, i).ok().map(|c| c + 1);
                }
                b'{' | b'}' if paren == 0 => return None,
                b';' | b'\n' if paren == 0 => return None,
                _ => {}
            }
            j += 1;
        }
        None
    }
}

impl NativeRegionScanner for NativeRegionScannerSwift {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&SwiftSkipper, bytes, open_brace_index)
    }
}
