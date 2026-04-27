// JavaScript syntax skipper — Frame-generated state machine.
//
// Source: javascript_skipper.frs (Frame specification)
// Generated: javascript_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec compile -l rust -o /tmp framec/src/frame_c/compiler/native_region_scanner/javascript_skipper.frs
//   cp /tmp/javascript_skipper.rs framec/src/frame_c/compiler/native_region_scanner/javascript_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("javascript_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::javascript::BodyCloserJs;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerJs;

/// JavaScript syntax skipper - handles //, /* */, strings, and template literals
pub struct JavaScriptSkipper;

impl SyntaxSkipper for JavaScriptSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserJs)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = JavaScriptSyntaxSkipperFsm::new();
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
        let mut fsm = JavaScriptSyntaxSkipperFsm::new();
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
        let mut fsm = JavaScriptSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = JavaScriptSyntaxSkipperFsm::new();
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
        scan_template_literal_regions(bytes, i, end)
    }

    fn skip_nested_scope(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        // JS arrow lambda with block body: same shape as TS.
        // `=> {` → arrow body; `=> $` → Frame forward.
        if i + 2 >= end || bytes[i] != b'=' || bytes[i + 1] != b'>' {
            return None;
        }
        let mut j = i + 2;
        while j < end && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if j >= end || bytes[j] != b'{' {
            return None;
        }
        let mut closer = BodyCloserJs;
        closer.close_byte(bytes, j).ok().map(|c| c + 1)
    }
}

impl NativeRegionScanner for NativeRegionScannerJs {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&JavaScriptSkipper, bytes, open_brace_index)
    }
}
