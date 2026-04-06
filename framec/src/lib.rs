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
