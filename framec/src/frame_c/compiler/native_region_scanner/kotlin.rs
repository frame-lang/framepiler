// Kotlin syntax skipper — Frame-generated state machine.
//
// Source: kotlin_skipper.frs (Frame specification)
// Generated: kotlin_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec framec/src/frame_c/compiler/native_region_scanner/kotlin_skipper.frs -l rust > framec/src/frame_c/compiler/native_region_scanner/kotlin_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("kotlin_skipper.gen.rs");

use super::*;
use super::unified::*;
use crate::frame_c::compiler::body_closer::kotlin::BodyCloserKotlin;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerKotlin;

/// Kotlin syntax skipper - handles //, /* */, strings, and raw strings """..."""
pub struct KotlinSkipper;

impl SyntaxSkipper for KotlinSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserKotlin)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = KotlinSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = i;
        fsm.end = end;
        fsm.do_skip_comment();
        if fsm.success != 0 { Some(fsm.result_pos) } else { None }
    }

    fn skip_string(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = KotlinSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = i;
        fsm.end = end;
        fsm.do_skip_string();
        if fsm.success != 0 { Some(fsm.result_pos) } else { None }
    }

    fn find_line_end(&self, bytes: &[u8], start: usize, end: usize) -> usize {
        let mut fsm = KotlinSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = KotlinSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = i;
        fsm.end = end;
        fsm.do_balanced_paren_end();
        if fsm.success != 0 { Some(fsm.result_pos) } else { None }
    }

}

impl NativeRegionScanner for NativeRegionScannerKotlin {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&KotlinSkipper, bytes, open_brace_index)
    }
}
