// Body closer for Swift language — Frame-generated state machine.
//
// Source: swift.frs (Frame specification)
// Generated: swift.gen.rs (via framec --target rust)
// This file: glue module wiring generated FSM to BodyCloser trait
//
// To regenerate:
//   ./target/release/framec framec/src/frame_c/compiler/body_closer/swift.frs -l rust > framec/src/frame_c/compiler/body_closer/swift.gen.rs

#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("swift.gen.rs");

use super::{BodyCloser, CloseError, CloseErrorKind};

pub struct BodyCloserSwift;

impl BodyCloser for BodyCloserSwift {
    fn close_byte(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<usize, CloseError> {
        let mut fsm = SwiftBodyCloserFsm::new();
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
