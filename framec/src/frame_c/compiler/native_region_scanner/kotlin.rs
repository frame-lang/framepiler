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

use super::unified::*;
use super::*;
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
        if fsm.success != 0 {
            Some(fsm.result_pos)
        } else {
            None
        }
    }

    fn skip_string(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = KotlinSyntaxSkipperFsm::new();
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
        scan_dollar_string_regions(bytes, i, end, 0)
    }

    fn skip_nested_scope(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        // Kotlin lambda: `{ args -> body }` or `{ -> body }`. Trigger
        // on `{` and confirm by finding `->` at depth 0 inside the
        // brace block before any other Frame statement marker. We
        // also accept `fun(args) ... { body }` (anonymous fun).
        //
        // Plain blocks (`{ x = 1 }`) and control-flow bodies are
        // distinguishable because they lack the `->` between `{` and
        // their first statement. We require the arrow to land BEFORE
        // the first newline / semicolon / Frame marker, which is the
        // syntactic shape of Kotlin lambda parameter lists.
        if bytes[i] == b'{' {
            // Scan ahead for `->` at depth 0 within the same brace
            // block. To avoid flagging a control-flow body like
            // `if (cond) { -> $X }` as a lambda preamble, require
            // the first non-whitespace char to be an identifier
            // start (the lambda's arg name) or `(` (destructuring
            // tuple). Frame transitions in if-bodies start with `-`
            // and so are excluded.
            let mut j = i + 1;
            while j < end && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
                j += 1;
            }
            if j >= end {
                return None;
            }
            let first = bytes[j];
            if !(first.is_ascii_alphabetic() || first == b'_' || first == b'(') {
                return None;
            }
            // Track depth of inner parens (Kotlin destructuring args:
            // `{ (a, b) -> ... }`).
            let mut paren = 0i32;
            while j + 1 < end {
                if let Some(after) = self.skip_string(bytes, j, end) {
                    j = after;
                    continue;
                }
                if let Some(after) = self.skip_comment(bytes, j, end) {
                    j = after;
                    continue;
                }
                let b = bytes[j];
                match b {
                    b'(' => paren += 1,
                    b')' => paren -= 1,
                    b'-' if paren == 0 && bytes[j + 1] == b'>' => {
                        let mut closer = BodyCloserKotlin;
                        return closer.close_byte(bytes, i).ok().map(|c| c + 1);
                    }
                    b'{' | b'}' if paren == 0 => return None,
                    b';' | b'\n' if paren == 0 => return None,
                    _ => {}
                }
                j += 1;
            }
            return None;
        }
        // Anonymous `fun(args) ... { body }` form.
        if i + 3 < end && &bytes[i..i + 3] == b"fun" {
            if i > 0 {
                let prev = bytes[i - 1];
                if prev.is_ascii_alphanumeric() || prev == b'_' {
                    return None;
                }
            }
            let mut j = i + 3;
            while j < end && matches!(bytes[j], b' ' | b'\t') {
                j += 1;
            }
            if j >= end || bytes[j] != b'(' {
                return None;
            }
            let after_args = self.balanced_paren_end(bytes, j, end)?;
            let mut k = after_args;
            // Optional `: ReturnType`
            while k < end && matches!(bytes[k], b' ' | b'\t' | b'\n' | b'\r') {
                k += 1;
            }
            if k < end && bytes[k] == b':' {
                k += 1;
                while k < end && bytes[k] != b'{' && bytes[k] != b'\n' {
                    k += 1;
                }
            }
            if k >= end || bytes[k] != b'{' {
                return None;
            }
            let mut closer = BodyCloserKotlin;
            return closer.close_byte(bytes, k).ok().map(|c| c + 1);
        }
        None
    }
}

impl NativeRegionScanner for NativeRegionScannerKotlin {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&KotlinSkipper, bytes, open_brace_index)
    }
}
