// Body closer for Go language.
//
// Go has the same brace/comment/string syntax as Java PLUS
// backtick raw strings (`...`). This FSM is generated from go.frs
// and handles all Go-specific string delimiters.

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("go.gen.rs");

use super::{BodyCloser, CloseError, CloseErrorKind};

pub struct BodyCloserGo;

impl BodyCloser for BodyCloserGo {
    fn close_byte(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<usize, CloseError> {
        let mut fsm = GoBodyCloserFsm::new();
        fsm.bytes = bytes.to_vec();
        fsm.pos = open_brace_index + 1;
        fsm.depth = 1;
        fsm.scan();
        match fsm.error_kind {
            0 => Ok(fsm.result_pos),
            1 => Err(CloseError {
                kind: CloseErrorKind::UnterminatedString,
                message: fsm.error_msg,
            }),
            2 => Err(CloseError {
                kind: CloseErrorKind::UnterminatedComment,
                message: fsm.error_msg,
            }),
            _ => Err(CloseError {
                kind: CloseErrorKind::UnmatchedBraces,
                message: fsm.error_msg,
            }),
        }
    }
}
