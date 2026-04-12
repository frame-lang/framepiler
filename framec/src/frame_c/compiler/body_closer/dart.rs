// Body closer for Dart language — reuses TypeScript body closer.
//
// Dart uses the same comment styles (// and /* */) and string delimiters
// (" and ') as TypeScript, so the TypeScript FSM works correctly.

use super::typescript::BodyCloserTs;
use super::{BodyCloser, CloseError};

pub struct BodyCloserDart;

impl BodyCloser for BodyCloserDart {
    fn close_byte(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<usize, CloseError> {
        BodyCloserTs.close_byte(bytes, open_brace_index)
    }
}
