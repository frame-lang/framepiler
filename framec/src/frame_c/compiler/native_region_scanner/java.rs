// Java syntax skipper — Frame-generated state machine.
//
// Source: java_skipper.frs (Frame specification)
// Generated: java_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec compile -l rust -o /tmp framec/src/frame_c/compiler/native_region_scanner/java_skipper.frs
//   cp /tmp/java_skipper.rs framec/src/frame_c/compiler/native_region_scanner/java_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("java_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::java::BodyCloserJava;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerJava;

/// Java syntax skipper - handles //, /* */, strings, and text blocks
pub struct JavaSkipper;

impl SyntaxSkipper for JavaSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserJava)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
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

    fn skip_nested_scope(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        // Detect Java block-bodied lambda preamble.
        //
        //   (args) -> { body }
        //   args -> { body }
        //   () -> { body }
        //
        // Trigger: `->` token. Distinguished from Frame's `-> $State`
        // (which the unified scanner handles via match_frame_statement)
        // by the byte that follows whitespace after the arrow:
        //   `-> {` → Java lambda body  (skip)
        //   `-> $` → Frame transition  (let match_frame_statement run)
        //
        // Expression-bodied lambdas (`(x, y) -> x + y`) don't terminate
        // in `{` and therefore can't contain statement-level Frame
        // markers; we leave them as native text, which is correct.
        if i + 2 >= end || bytes[i] != b'-' || bytes[i + 1] != b'>' {
            return None;
        }
        let mut j = i + 2;
        while j < end && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if j >= end || bytes[j] != b'{' {
            return None;
        }
        let mut closer = BodyCloserJava;
        closer.close_byte(bytes, j).ok().map(|c| c + 1)
    }
}

impl NativeRegionScanner for NativeRegionScannerJava {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&JavaSkipper, bytes, open_brace_index)
    }
}
