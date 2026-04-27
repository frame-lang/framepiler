// Lua syntax skipper — Frame-generated state machine.
//
// Source: lua_skipper.frs (Frame specification)
// Generated: lua_skipper.gen.rs (via framec compile -l rust)

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("lua_skipper.gen.rs");

use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::lua::BodyCloserLua;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerLua;

/// Lua syntax skipper - handles -- comments, --[[ ]] block comments, strings, long strings
pub struct LuaSkipper;

impl SyntaxSkipper for LuaSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserLua)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = LuaSyntaxSkipperFsm::new();
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
        let mut fsm = LuaSyntaxSkipperFsm::new();
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
        let mut fsm = LuaSyntaxSkipperFsm::new();
        fsm.bytes = bytes[..end].to_vec();
        fsm.pos = start;
        fsm.end = end;
        fsm.do_find_line_end();
        fsm.result_pos
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        let mut fsm = LuaSyntaxSkipperFsm::new();
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
        // Lua function literal: `function(args) ... end`. Body
        // terminator is the `end` keyword, not a brace, so we walk
        // forward respecting nested constructs that also use `end`
        // (`do ... end`, `if ... end`, `for ... end`, `while ... end`,
        // `function ... end`, `repeat ... until`).
        //
        // Trigger on `f` and require `function(`.
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
        // Walk balancing block-opener / `end` pairs. We start at depth
        // 1 (the function body itself); openers add 1, `end` subtracts
        // 1 until we hit 0.
        let mut k = after_args;
        let mut depth = 1i32;
        while k < end {
            // Skip strings and comments first so their content can't
            // confuse the keyword search.
            if let Some(after) = self.skip_string(bytes, k, end) {
                k = after;
                continue;
            }
            if let Some(after) = self.skip_comment(bytes, k, end) {
                k = after;
                continue;
            }
            // Identifier/keyword chunk
            let b = bytes[k];
            if b.is_ascii_alphabetic() || b == b'_' {
                let start = k;
                while k < end && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'_') {
                    k += 1;
                }
                let kw = &bytes[start..k];
                let token_boundary = start == 0 || {
                    let p = bytes[start - 1];
                    !(p.is_ascii_alphanumeric() || p == b'_')
                };
                if token_boundary {
                    match kw {
                        b"function" | b"do" | b"if" | b"for" | b"while" | b"repeat" => {
                            // Only outer `function` and inner block openers
                            // increase depth. `if`/`for`/`while`/`function`
                            // all close with `end`; `repeat` closes with
                            // `until`, which we treat the same way (decrement).
                            depth += 1;
                        }
                        b"end" | b"until" => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(k);
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            k += 1;
        }
        None
    }
}

impl NativeRegionScanner for NativeRegionScannerLua {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&LuaSkipper, bytes, open_brace_index)
    }
}
