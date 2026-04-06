// Body closer for Dart language — reuses TypeScript body closer.
//
// Dart uses the same comment styles (// and /* */) and string delimiters
// (" and ') as TypeScript, so the TypeScript FSM works correctly.

use super::{BodyCloser, CloseError};
use super::typescript::BodyCloserTs;

pub struct BodyCloserDart;

impl BodyCloser for BodyCloserDart {
    fn close_byte(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<usize, CloseError> {
        BodyCloserTs.close_byte(bytes, open_brace_index)
    }
}
