// Body closer for GDScript language — reuses Python FSM since both use
// # line comments, "..." and '...' strings, and """...""" triple-quoted strings.

use super::python::BodyCloserPy;
use super::{BodyCloser, CloseError};

pub struct BodyCloserGDScript;

impl BodyCloser for BodyCloserGDScript {
    fn close_byte(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<usize, CloseError> {
        // GDScript shares Python's comment and string syntax
        BodyCloserPy.close_byte(bytes, open_brace_index)
    }
}
