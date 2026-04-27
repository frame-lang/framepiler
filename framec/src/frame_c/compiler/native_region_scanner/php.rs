// PHP syntax skipper — Frame-generated state machine.
//
// Source: php_skipper.frs (Frame specification)
// Generated: php_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec compile -l rust -o /tmp framec/src/frame_c/compiler/native_region_scanner/php_skipper.frs
//   cp /tmp/php_skipper.rs framec/src/frame_c/compiler/native_region_scanner/php_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("php_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::php::BodyCloserPhp;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerPhp;

/// PHP syntax skipper - handles //, #, /* */, and string literals
pub struct PhpSkipper;

impl SyntaxSkipper for PhpSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserPhp)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = PhpSyntaxSkipperFsm::new();
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
        let mut fsm = PhpSyntaxSkipperFsm::new();
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
        let mut fsm = PhpSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = PhpSyntaxSkipperFsm::new();
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
        // PHP closure: `function(args) [use(...)] { body }`. Trigger
        // on `f` and require `function` keyword start at a token
        // boundary. We don't try to detect the arrow form
        // `fn(args) => expr` because its body is always an
        // expression and can't contain Frame statements.
        if i + 8 >= end || &bytes[i..i + 8] != b"function" {
            return None;
        }
        if i > 0 {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                return None;
            }
        }
        let mut j = i + 8;
        while j < end && matches!(bytes[j], b' ' | b'\t') {
            j += 1;
        }
        if j >= end || bytes[j] != b'(' {
            return None;
        }
        let after_args = self.balanced_paren_end(bytes, j, end)?;
        let mut k = after_args;
        // Optional `use(...)` clause before the body brace.
        while k < end && matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
            k += 1;
        }
        if k + 3 < end && &bytes[k..k + 3] == b"use" {
            let mut m = k + 3;
            while m < end && matches!(bytes[m], b' ' | b'\t') {
                m += 1;
            }
            if m < end && bytes[m] == b'(' {
                k = self.balanced_paren_end(bytes, m, end)?;
                while k < end && matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
                    k += 1;
                }
            }
        }
        // Optional `: ReturnType` (PHP 7+)
        if k < end && bytes[k] == b':' {
            k += 1;
            while k < end && bytes[k] != b'{' && bytes[k] != b'\n' {
                k += 1;
            }
        }
        if k >= end || bytes[k] != b'{' {
            return None;
        }
        let mut closer = BodyCloserPhp;
        closer.close_byte(bytes, k).ok().map(|c| c + 1)
    }
}

impl NativeRegionScanner for NativeRegionScannerPhp {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&PhpSkipper, bytes, open_brace_index)
    }
}
