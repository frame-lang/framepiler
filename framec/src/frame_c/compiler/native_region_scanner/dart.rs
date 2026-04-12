// Dart syntax skipper — reuses TypeScript syntax skipper.
//
// Dart uses the same comment styles (// and /* */), string delimiters
// (" and '), and brace-delimited blocks as TypeScript.

use super::typescript::TypeScriptSkipper;
use super::unified::*;
use super::*;
use crate::frame_c::compiler::body_closer::dart::BodyCloserDart;
use crate::frame_c::compiler::body_closer::BodyCloser;

pub struct NativeRegionScannerDart;

/// Dart syntax skipper - delegates to TypeScript skipper (same syntax rules)
pub struct DartSkipper;

impl SyntaxSkipper for DartSkipper {
    fn body_closer(&self) -> Box<dyn BodyCloser> {
        Box::new(BodyCloserDart)
    }

    fn skip_comment(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        TypeScriptSkipper.skip_comment(bytes, i, end)
    }

    fn skip_string(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        TypeScriptSkipper.skip_string(bytes, i, end)
    }

    fn find_line_end(&self, bytes: &[u8], start: usize, end: usize) -> usize {
        TypeScriptSkipper.find_line_end(bytes, start, end)
    }

    fn balanced_paren_end(&self, bytes: &[u8], i: usize, end: usize) -> Option<usize> {
        TypeScriptSkipper.balanced_paren_end(bytes, i, end)
    }
}

impl NativeRegionScanner for NativeRegionScannerDart {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        super::unified::scan_native_regions(&DartSkipper, bytes, open_brace_index)
    }
}
