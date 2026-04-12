// Body closer for Erlang language — Frame-generated state machine.
//
// Source: erlang.frs (Frame specification)
// Generated: erlang.gen.rs (via framec)
// This file: glue module wiring generated FSM to BodyCloser trait

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("erlang.gen.rs");

use super::{BodyCloser, CloseError, CloseErrorKind};

pub struct BodyCloserErlang;

impl BodyCloser for BodyCloserErlang {
    fn close_byte(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<usize, CloseError> {
        let mut fsm = ErlangBodyCloserFsm::new();
        fsm.bytes = bytes.to_vec();
        fsm.pos = open_brace_index + 1;
        fsm.end = bytes.len();
        fsm.depth = 1;
        fsm.scan();
        if fsm.success != 0 {
            Ok(fsm.pos)
        } else {
            match fsm.error_kind {
                1 => Err(CloseError {
                    kind: CloseErrorKind::UnterminatedString,
                    message: String::new(),
                }),
                2 => Err(CloseError {
                    kind: CloseErrorKind::UnterminatedComment,
                    message: String::new(),
                }),
                _ => Err(CloseError {
                    kind: CloseErrorKind::UnmatchedBraces,
                    message: String::new(),
                }),
            }
        }
    }
}
