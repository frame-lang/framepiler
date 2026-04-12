// Erlang syntax skipper — Frame-generated state machine.
//
// Source: erlang_skipper.frs (Frame specification)
// Generated: erlang_skipper.gen.rs (via framec)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec erlang_skipper.frs > erlang_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

include!("erlang_skipper.gen.rs");
include!("erlang_scope_scanner.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::erlang::BodyCloserErlang;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerErlang;

/// Erlang syntax skipper - handles % comments and "..." strings
pub struct ErlangSkipper;

impl SyntaxSkipper for ErlangSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserErlang)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = ErlangSyntaxSkipperFsm::new();
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
        let mut fsm = ErlangSyntaxSkipperFsm::new();
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
        let mut fsm = ErlangSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = ErlangSyntaxSkipperFsm::new();
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
        // Only trigger on `f` (start of `fun` keyword)
        if i + 3 > end || bytes[i] != b'f' || bytes[i + 1] != b'u' || bytes[i + 2] != b'n' {
            return None;
        }
        let mut fsm = ErlangScopeScannerFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = i;
        fsm.end = end;
        fsm.do_scan();
        if fsm.success != 0 {
            Some(fsm.result_pos)
        } else {
            None
        }
    }
}

impl NativeRegionScanner for NativeRegionScannerErlang {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&ErlangSkipper, bytes, open_brace_index)
    }
}
