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
        // Erlang character literal: `$<char>` is the integer value
        // of the byte after `$`. Without this fast path, the unified
        // scanner walks past `$` without recognising it as the start
        // of a literal, then the very next byte (e.g. `"`, `'`, `(`)
        // gets misidentified as a string/atom start by the FSM and
        // the rest of the file is swallowed up to the next matching
        // delimiter — which is how the prolog of state_var_parser
        // (and any handler body using `$<punct>`) corrupted the
        // segmenter.
        //
        // Recognise `$<char>` here so the cursor advances by two
        // bytes (the `$` plus the literal char). `$\<escape>` skips
        // three bytes (`$`, `\`, escape char). Anything else falls
        // through to the FSM's regular string/atom handling.
        if i < end && bytes[i] == b'$' && i + 1 < end {
            // `$.` is a Frame state-var marker handled later in the
            // unified scanner — don't consume it here.
            if bytes[i + 1] != b'.' {
                if bytes[i + 1] == b'\\' && i + 2 < end {
                    return Some(i + 3);
                }
                return Some(i + 2);
            }
        }

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
