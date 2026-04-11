// Suppress clippy lints that are cosmetic/stylistic across the codebase.
// The framepiler's codegen uses match statements extensively for
// per-language dispatch — converting these to if-let chains would
// reduce readability. Similarly, explicit returns and format! calls
// are used for clarity in code generation contexts.
#![allow(clippy::single_match)]
#![allow(clippy::needless_return)]
#![allow(clippy::single_char_add_str)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::useless_format)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::useless_conversion)]
#![allow(clippy::redundant_field_names)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::new_without_default)]  // Generated FSM structs use new() not Default
#![allow(clippy::manual_strip)]
#![allow(clippy::match_like_matches_macro)]
#![allow(clippy::len_zero)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::map_clone)]
#![allow(clippy::option_as_ref_deref)]
#![allow(clippy::implicit_saturating_sub)]
#![allow(clippy::empty_line_after_doc_comments)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::format_in_format_args)]
#![allow(clippy::to_string_in_format_args)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::clone_on_copy)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::unnecessary_map_or)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::iter_kv_map)]
#![allow(clippy::manual_contains)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::bool_comparison)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unreachable_patterns)]
#![allow(dead_code)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(clippy::manual_pattern_char_comparison)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::unnecessary_to_owned)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::suspicious_else_formatting)]
#![allow(clippy::possible_missing_else)]
#![allow(clippy::unwrap_or_default)]
#![allow(clippy::for_kv_map)]
#![allow(clippy::cloned_ref_to_slice_refs)]
#![allow(clippy::duplicated_attributes)]

pub mod frame_c;
use crate::driver::{Exe, TargetLanguage};
use crate::frame_c::*;
use std::convert::TryFrom;
use wasm_bindgen::prelude::*;

/// WASM entry point for the online framepiler.
#[wasm_bindgen]
pub fn run(frame_code: &str, format: &str) -> String {
    let _exe = Exe::new();
    match TargetLanguage::try_from(format) {
        Ok(target) => {
            if frame_code.contains("@target ") {
                let result = crate::frame_c::compiler::compile_module(frame_code, target);
                match result {
                    Ok(code) => code,
                    Err(run_error) => run_error.error,
                }
            } else {
                "Error: Frame files must specify @@target language.".to_string()
            }
        }
        Err(err) => err,
    }
}
