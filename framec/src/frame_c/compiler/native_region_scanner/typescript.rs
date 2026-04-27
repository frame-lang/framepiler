// TypeScript syntax skipper — Frame-generated state machine.
//
// Source: typescript_skipper.frs (Frame specification)
// Generated: typescript_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec compile -l rust -o /tmp framec/src/frame_c/compiler/native_region_scanner/typescript_skipper.frs
//   cp /tmp/typescript_skipper.rs framec/src/frame_c/compiler/native_region_scanner/typescript_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("typescript_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::typescript::BodyCloserTs;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerTs;

/// TypeScript syntax skipper - handles //, /* */, strings, and template literals
pub struct TypeScriptSkipper;

impl SyntaxSkipper for TypeScriptSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserTs)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = TypeScriptSyntaxSkipperFsm::new();
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
        let mut fsm = TypeScriptSyntaxSkipperFsm::new();
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
        let mut fsm = TypeScriptSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = TypeScriptSyntaxSkipperFsm::new();
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
        // TS arrow lambda with block body: `(args) => { body }` /
        // `args => { body }` / `() => { body }`. Trigger on `=>`.
        // Distinguish from Frame `=> $State` (forward) by what follows
        // whitespace after the arrow:
        //   `=> {` → TS arrow body  (skip)
        //   `=> $` → Frame forward  (let unified scanner handle)
        // Expression-bodied arrows (`=> expr`) carry no Frame markers
        // we care about, so falling through is fine.
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
        let mut closer = BodyCloserTs;
        closer.close_byte(bytes, j).ok().map(|c| c + 1)
    }
}

impl NativeRegionScanner for NativeRegionScannerTs {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&TypeScriptSkipper, bytes, open_brace_index)
    }
}
