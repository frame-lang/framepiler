// C# syntax skipper — Frame-generated state machine.
//
// Source: csharp_skipper.frs (Frame specification)
// Generated: csharp_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec compile -l rust -o /tmp framec/src/frame_c/compiler/native_region_scanner/csharp_skipper.frs
//   cp /tmp/csharp_skipper.rs framec/src/frame_c/compiler/native_region_scanner/csharp_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("csharp_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::csharp::BodyCloserCs;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerCs;

/// C# syntax skipper - handles //, /* */, preprocessor #, strings, verbatim @"", interpolated $""
pub struct CSharpSkipper;

impl SyntaxSkipper for CSharpSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserCs)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = CSharpSyntaxSkipperFsm::new();
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
        let mut fsm = CSharpSyntaxSkipperFsm::new();
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
        let mut fsm = CSharpSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = CSharpSyntaxSkipperFsm::new();
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
        scan_dollar_string_regions(bytes, i, end, b'$')
    }

    fn skip_nested_scope(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        // C# lambda with statement body: `(args) => { body }` or
        // `args => { body }`. Same shape as TS/JS arrow, distinct from
        // Frame `=> $State` (forward).
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
        let mut closer = BodyCloserCs;
        closer.close_byte(bytes, j).ok().map(|c| c + 1)
    }
}

impl NativeRegionScanner for NativeRegionScannerCs {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&CSharpSkipper, bytes, open_brace_index)
    }
}
