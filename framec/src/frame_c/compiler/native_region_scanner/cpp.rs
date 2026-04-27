// C++ syntax skipper — Frame-generated state machine.
//
// Source: cpp_skipper.frs (Frame specification)
// Generated: cpp_skipper.gen.rs (via framec compile -l rust)
// This file: glue module wiring generated FSM to SyntaxSkipper trait
//
// To regenerate:
//   ./target/release/framec compile -l rust -o /tmp framec/src/frame_c/compiler/native_region_scanner/cpp_skipper.frs
//   cp /tmp/cpp_skipper.rs framec/src/frame_c/compiler/native_region_scanner/cpp_skipper.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("cpp_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::cpp::BodyCloserCpp;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerCpp;

/// C++ syntax skipper - handles //, /* */, strings, and raw strings R"delim(...)delim"
pub struct CppSkipper;

impl SyntaxSkipper for CppSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserCpp)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = CppSyntaxSkipperFsm::new();
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
        let mut fsm = CppSyntaxSkipperFsm::new();
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
        let mut fsm = CppSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = CppSyntaxSkipperFsm::new();
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
        // C++ lambda: `[capture](args) [mutable] [-> ret] { body }`.
        // Trigger on `[` followed (after capture-list close) by `(`.
        // We must distinguish lambda capture from array indexing
        // (`vec[0]`) and from attribute syntax (`[[nodiscard]]`):
        //
        //   `[<capture>](<args>) [...] {` → lambda  (skip body)
        //   `vec[0]`                       → indexing  (no `(` after `]`)
        //   `[[attr]]`                     → attribute  (`[[`)
        //
        // The lambda check: match `[`, find matching `]` at depth 0,
        // require `(` immediately after, find matching `)`, allow
        // optional `mutable`, optional `-> ret_type`, then `{`.
        if bytes[i] != b'[' || (i + 1 < end && bytes[i + 1] == b'[') {
            return None;
        }
        let mut j = i + 1;
        let mut depth = 1i32;
        while j < end && depth > 0 {
            match bytes[j] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        if depth != 0 || j >= end {
            return None;
        }
        // Skip whitespace, expect `(`
        while j < end && matches!(bytes[j], b' ' | b'\t') {
            j += 1;
        }
        if j >= end || bytes[j] != b'(' {
            return None;
        }
        // Find matching `)` for the args list — reuse the skipper's
        // own paren matcher so string/comment handling is consistent.
        let after_args = self.balanced_paren_end(bytes, j, end)?;
        let mut k = after_args;
        // Skip whitespace and optional `mutable`, optional `-> type`
        while k < end {
            while k < end && matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
                k += 1;
            }
            if k + 7 < end && &bytes[k..k + 7] == b"mutable" {
                k += 7;
                continue;
            }
            if k + 1 < end && bytes[k] == b'-' && bytes[k + 1] == b'>' {
                // `-> ret_type` — consume up to the next `{`
                k += 2;
                while k < end && bytes[k] != b'{' {
                    k += 1;
                }
                continue;
            }
            break;
        }
        if k >= end || bytes[k] != b'{' {
            return None;
        }
        let mut closer = BodyCloserCpp;
        closer.close_byte(bytes, k).ok().map(|c| c + 1)
    }
}

impl NativeRegionScanner for NativeRegionScannerCpp {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&CppSkipper, bytes, open_brace_index)
    }
}
