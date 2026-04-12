// Go syntax skipper for native region scanning.
//
// Go has the same comment/string syntax as Java (// and /* */, double-quoted
// strings with backslash escapes) plus backtick raw strings.
// We reuse the Java FSM for comments and standard strings, and add
// backtick raw string handling.

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

// Reuse Java syntax skipper FSM — Go has identical comment/string syntax
include!("java_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::go::BodyCloserGo;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerGo;

/// Go syntax skipper - handles //, /* */, double-quoted strings, and backtick raw strings
pub struct GoSkipper;

impl SyntaxSkipper for GoSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserGo)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        // Go uses the same comment syntax as Java: // and /* */
        let mut fsm = JavaSyntaxSkipperFsm::new();
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
        // Handle backtick raw strings first (Go-specific)
        if i < end && bytes[i] == b'`' {
            let mut j = i + 1;
            while j < end && bytes[j] != b'`' {
                j += 1;
            }
            if j < end {
                return Some(j + 1);
            }
            return Some(end);
        }
        // Double-quoted strings — same as Java
        let mut fsm = JavaSyntaxSkipperFsm::new();
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
        let mut fsm = JavaSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = JavaSyntaxSkipperFsm::new();
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
}

impl NativeRegionScanner for NativeRegionScannerGo {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&GoSkipper, bytes, open_brace_index)
    }
}
