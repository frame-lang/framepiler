//! Cross-backend helpers used by both `generate_interface_wrappers`
//! and `generate_persistence_methods`.
//!
//! Two functions:
//!
//! - `is_dynamic_target` — classify backends by typed/dynamic
//!   semantics. Dynamic backends (Python, JS, Ruby, Lua, PHP,
//!   GDScript, Erlang) have no `void` to honor — every function
//!   returns *something* — so interface-method wrappers always expose
//!   the `FrameContext._return` slot to the caller. Typed backends
//!   condition the wrapper on the source's declared return type so
//!   the wrapper doesn't try to return a value from a `void` method.
//!   See `docs/frame_runtime.md § "Return values across target
//!   languages"`.
//!
//! - `frame_return_default` — per-(language, type) zero-value
//!   literal. Used by interface wrappers to seed `_return` so a
//!   handler that doesn't write `@@:return` still produces a valid
//!   type-default at the wrapper boundary — rather than null/None,
//!   which would crash typed langs on unboxing (Java/C#) or violate
//!   the typed-return contract on dynamic langs (Python returning
//!   `None` when an `: int` was promised).
//!
//!   Frame source uses canonical type names (`int`, `str`, `bool`,
//!   `float`, etc.); each backend maps to its own type system and
//!   each language's zero-value convention. Unknown / void types
//!   fall back to the language's null literal.
//!
//! Rust, Go, C, C++, Swift, and Erlang are not consumers of
//! `frame_return_default` — they wire context init through their own
//! per-backend paths.

use crate::frame_c::visitors::TargetLanguage;

pub(super) fn is_dynamic_target(lang: TargetLanguage) -> bool {
    matches!(
        lang,
        TargetLanguage::Python3
            | TargetLanguage::JavaScript
            | TargetLanguage::Ruby
            | TargetLanguage::Lua
            | TargetLanguage::Php
            | TargetLanguage::GDScript
            | TargetLanguage::Erlang
    )
}

pub(super) fn frame_return_default(lang: TargetLanguage, type_str: &str) -> String {
    let t = type_str.trim();
    // Common int/string/bool patterns each lang accepts.
    let is_int = matches!(
        t,
        "int" | "Int" | "i32" | "i64" | "long" | "Long" | "Integer"
    );
    let is_str = matches!(t, "str" | "string" | "String");
    let is_bool = matches!(t, "bool" | "boolean" | "Boolean");
    let is_float = matches!(t, "float" | "Float" | "double" | "Double" | "f32" | "f64");

    match lang {
        TargetLanguage::Python3 => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "False".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "None".to_string()
            }
        }
        TargetLanguage::JavaScript | TargetLanguage::TypeScript => {
            if is_int || is_float {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else {
                "null".to_string()
            }
        }
        TargetLanguage::Ruby => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "nil".to_string()
            }
        }
        TargetLanguage::Lua => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "nil".to_string()
            }
        }
        TargetLanguage::Php => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "null".to_string()
            }
        }
        TargetLanguage::Java => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "null".to_string()
            }
        }
        TargetLanguage::CSharp => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "null".to_string()
            }
        }
        TargetLanguage::Kotlin => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "null".to_string()
            }
        }
        TargetLanguage::Dart => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "null".to_string()
            }
        }
        TargetLanguage::GDScript => {
            if is_int {
                "0".to_string()
            } else if is_str {
                "\"\"".to_string()
            } else if is_bool {
                "false".to_string()
            } else if is_float {
                "0.0".to_string()
            } else {
                "null".to_string()
            }
        }
        // Other langs (Rust, Go, C, C++, Swift, Erlang) handle
        // defaults via their own context-init paths; this helper
        // is currently called only by the wrappers above. Return
        // a generic null marker for safety — those backends don't
        // wire it through.
        _ => "null".to_string(),
    }
}
