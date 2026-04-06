// GDScript syntax skipper — reuses Python skipper since both use
// # line comments, "..." and '...' strings, and """...""" triple-quoted strings.

use super::*;
use super::unified::*;
use super::python::PythonSkipper;
use crate::frame_c::compiler::body_closer::gdscript::BodyCloserGDScript;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerGDScript;

/// GDScript syntax skipper — delegates to Python skipper for comment/string handling,
/// but uses GDScript's own body closer.
pub struct GDScriptSkipper;

impl SyntaxSkipper for GDScriptSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserGDScript)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        PythonSkipper.skip_comment(bytes, i, end)
    }

    fn skip_string(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        PythonSkipper.skip_string(bytes, i, end)
    }

    fn find_line_end(&self, bytes: &[u8], start: usize, end: usize) -> usize {
        PythonSkipper.find_line_end(bytes, start, end)
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        PythonSkipper.balanced_paren_end(bytes, i, end)
    }
}

impl NativeRegionScanner for NativeRegionScannerGDScript {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&GDScriptSkipper, bytes, open_brace_index)
    }
}
