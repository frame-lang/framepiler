//! Scanner traits and factory functions
//!
//! This module eliminates the 6x scanner duplication by providing a unified
//! trait and factory function for language-specific scanners.

use crate::frame_c::compiler::native_region_scanner::{
    c::NativeRegionScannerC, cpp::NativeRegionScannerCpp, csharp::NativeRegionScannerCs,
    java::NativeRegionScannerJava, python::NativeRegionScannerPy, rust::NativeRegionScannerRust,
    typescript::NativeRegionScannerTs, NativeRegionScanner, ScanError, ScanResult,
};
use crate::frame_c::visitors::TargetLanguage;

/// Trait for region scanning (wrapper around NativeRegionScanner)
pub trait RegionScannerTrait: Send + Sync {
    /// Scan native code for Frame regions
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError>;
}

// Implement trait for all scanner types
impl RegionScannerTrait for NativeRegionScannerPy {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        NativeRegionScanner::scan(self, bytes, open_brace_index)
    }
}

impl RegionScannerTrait for NativeRegionScannerTs {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        NativeRegionScanner::scan(self, bytes, open_brace_index)
    }
}

impl RegionScannerTrait for NativeRegionScannerRust {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        NativeRegionScanner::scan(self, bytes, open_brace_index)
    }
}

impl RegionScannerTrait for NativeRegionScannerCs {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        NativeRegionScanner::scan(self, bytes, open_brace_index)
    }
}

impl RegionScannerTrait for NativeRegionScannerC {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        NativeRegionScanner::scan(self, bytes, open_brace_index)
    }
}

impl RegionScannerTrait for NativeRegionScannerCpp {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        NativeRegionScanner::scan(self, bytes, open_brace_index)
    }
}

impl RegionScannerTrait for NativeRegionScannerJava {
    fn scan(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<ScanResult, ScanError> {
        NativeRegionScanner::scan(self, bytes, open_brace_index)
    }
}

/// Get the appropriate region scanner for a target language
///
/// This factory function replaces the duplicated match statements that were
/// scattered throughout mod.rs.
pub fn get_region_scanner(target: TargetLanguage) -> Box<dyn RegionScannerTrait> {
    match target {
        TargetLanguage::Python3 => Box::new(NativeRegionScannerPy),
        TargetLanguage::TypeScript => Box::new(NativeRegionScannerTs),
        TargetLanguage::Rust => Box::new(NativeRegionScannerRust),
        TargetLanguage::CSharp => Box::new(NativeRegionScannerCs),
        TargetLanguage::C => Box::new(NativeRegionScannerC),
        TargetLanguage::Cpp => Box::new(NativeRegionScannerCpp),
        TargetLanguage::Java => Box::new(NativeRegionScannerJava),
        // Fallback for unsupported languages
        _ => Box::new(NativeRegionScannerPy), // Python as default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_scanner_python() {
        let _scanner = get_region_scanner(TargetLanguage::Python3);
    }

    #[test]
    fn test_get_scanner_typescript() {
        let scanner = get_region_scanner(TargetLanguage::TypeScript);
        let _ = scanner;
    }

    #[test]
    fn test_get_scanner_all_languages() {
        let languages = vec![
            TargetLanguage::Python3,
            TargetLanguage::TypeScript,
            TargetLanguage::Rust,
            TargetLanguage::CSharp,
            TargetLanguage::C,
            TargetLanguage::Cpp,
            TargetLanguage::Java,
        ];

        for lang in languages {
            let scanner = get_region_scanner(lang);
            let _ = scanner;
        }
    }
}
