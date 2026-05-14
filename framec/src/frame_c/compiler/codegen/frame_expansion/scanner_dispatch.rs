//! Per-target scanner factory and the `@@:system.state` lowering.
//!
//! Three small entry points, all stateless:
//!
//! - `get_native_scanner(lang)` — returns a boxed
//!   `NativeRegionScanner` impl for the target language. Frame's
//!   scanner pipeline needs a per-target skipper so it can step
//!   past string literals, comments, raw strings, and other
//!   language-specific lexical regions while looking for Frame
//!   sigils.
//! - `expand_system_state(lang)` — emits the literal target-side
//!   expression for `@@:system.state` (the current state name
//!   accessor). One-liner per backend.
//! - `expand_system_state_in_code(code, lang)` — runs the two
//!   stateless rewrites on operation-body code:
//!     - `@@:system.state` → the per-backend compartment accessor
//!     - `@@:(<expr>)` → `return <expr>;` (or `<expr>` on Erlang)
//!
//! The pipeline calls `get_native_scanner` from three places
//! (state_dispatch, system_codegen, frame_validator) so it stays
//! `pub(crate)` and is re-exported from the parent module.

use crate::frame_c::compiler::native_region_scanner::{
    c::NativeRegionScannerC, cpp::NativeRegionScannerCpp, csharp::NativeRegionScannerCs,
    dart::NativeRegionScannerDart, erlang::NativeRegionScannerErlang,
    gdscript::NativeRegionScannerGDScript, go::NativeRegionScannerGo,
    java::NativeRegionScannerJava, javascript::NativeRegionScannerJs,
    kotlin::NativeRegionScannerKotlin, lua::NativeRegionScannerLua, php::NativeRegionScannerPhp,
    python::NativeRegionScannerPy, ruby::NativeRegionScannerRuby, rust::NativeRegionScannerRust,
    swift::NativeRegionScannerSwift, typescript::NativeRegionScannerTs, NativeRegionScanner,
};
use crate::frame_c::visitors::TargetLanguage;

/// Get the native region scanner for the target language.
pub(crate) fn get_native_scanner(lang: TargetLanguage) -> Box<dyn NativeRegionScanner> {
    match lang {
        TargetLanguage::Python3 => Box::new(NativeRegionScannerPy),
        TargetLanguage::TypeScript => Box::new(NativeRegionScannerTs),
        TargetLanguage::JavaScript => Box::new(NativeRegionScannerJs),
        TargetLanguage::Rust => Box::new(NativeRegionScannerRust),
        TargetLanguage::CSharp => Box::new(NativeRegionScannerCs),
        TargetLanguage::C => Box::new(NativeRegionScannerC),
        TargetLanguage::Cpp => Box::new(NativeRegionScannerCpp),
        TargetLanguage::Java => Box::new(NativeRegionScannerJava),
        TargetLanguage::Kotlin => Box::new(NativeRegionScannerKotlin),
        TargetLanguage::Swift => Box::new(NativeRegionScannerSwift),
        TargetLanguage::Go => Box::new(NativeRegionScannerGo),
        TargetLanguage::Php => Box::new(NativeRegionScannerPhp),
        TargetLanguage::Ruby => Box::new(NativeRegionScannerRuby),
        TargetLanguage::Erlang => Box::new(NativeRegionScannerErlang),
        TargetLanguage::Lua => Box::new(NativeRegionScannerLua),
        TargetLanguage::Dart => Box::new(NativeRegionScannerDart),
        TargetLanguage::GDScript => Box::new(NativeRegionScannerGDScript),
        // Graphviz is an output-only target (emitted from the SystemGraph IR,
        // not from native code). The validator still scans for Frame tokens
        // (e.g. @@:self.method()) during the graphviz compile path; those
        // tokens are target-language-agnostic, so any skipper works. Use the
        // Python scanner as a neutral default.
        TargetLanguage::Graphviz => Box::new(NativeRegionScannerPy),
    }
}

/// Expand `@@:system.state` to the target-language compartment state accessor.
/// Used by both handler body expansion and operation body expansion.
pub(crate) fn expand_system_state(lang: TargetLanguage) -> String {
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => {
            "self.__compartment.state".to_string()
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
            "this.__compartment.state".to_string()
        }
        TargetLanguage::Rust => super::super::rust_system::rust_system_state(),
        TargetLanguage::C => "self->__compartment->state".to_string(),
        TargetLanguage::Cpp => "__compartment->state".to_string(),
        TargetLanguage::Java | TargetLanguage::Kotlin | TargetLanguage::CSharp => {
            "__compartment.state".to_string()
        }
        TargetLanguage::Swift => "__compartment.state".to_string(),
        TargetLanguage::Go => "s.__compartment.state".to_string(),
        TargetLanguage::Php => "$this->__compartment->state".to_string(),
        TargetLanguage::Ruby => "@__compartment.state".to_string(),
        TargetLanguage::Lua => "self.__compartment.state".to_string(),
        TargetLanguage::Erlang => "\"\"".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    }
}

/// Expand `@@:system.state` occurrences in operation body code.
/// Operations are native code but `@@:system.state` is a read-only accessor
/// that's safe in non-static operations.
pub(crate) fn expand_system_state_in_code(code: &str, lang: TargetLanguage) -> String {
    let mut result = code.to_string();

    // Expand @@:system.state → compartment accessor
    if result.contains("@@:system.state") {
        result = result.replace("@@:system.state", &expand_system_state(lang));
    }

    // Expand @@:(expr) → return expr
    // In operation bodies, @@:(expr) means "return this value" (no context stack).
    // This handles patterns like @@:(@@:system.state) where the inner was already expanded.
    while let Some(start) = result.find("@@:(") {
        let after = start + 4; // position after "@@:("
        let bytes = result.as_bytes();
        let mut depth = 1i32;
        let mut j = after;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                j += 1;
            }
        }
        if depth == 0 {
            let expr = &result[after..j];
            let expansion = match lang {
                // Erlang: last expression IS the return value
                TargetLanguage::Erlang => expr.to_string(),
                // No-semicolon languages
                TargetLanguage::Python3
                | TargetLanguage::GDScript
                | TargetLanguage::Ruby
                | TargetLanguage::Kotlin
                | TargetLanguage::Swift
                | TargetLanguage::Lua
                | TargetLanguage::Go => format!("return {}", expr),
                // Semicolon languages
                _ => format!("return {};", expr),
            };
            result = format!("{}{}{}", &result[..start], expansion, &result[j + 1..]);
        } else {
            break; // unmatched paren — bail
        }
    }

    result
}
