#![allow(unreachable_patterns)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(unused_variables)]

include!("lua.gen.rs");

use super::{BodyCloser, CloseError, CloseErrorKind};

pub struct BodyCloserLua;

impl BodyCloser for BodyCloserLua {
    fn close_byte(&mut self, bytes: &[u8], open: usize) -> Result<usize, CloseError> {
        let mut fsm = LuaBodyCloserFsm::new();
        fsm.bytes = bytes.to_vec();
        fsm.pos = open + 1;
        fsm.depth = 1;
        fsm.scan();
        if fsm.result >= 0 {
            Ok(fsm.result as usize)
        } else {
            Err(CloseError {
                kind: CloseErrorKind::UnmatchedBraces,
                message: "Unterminated Lua block".into(),
            })
        }
    }
}
