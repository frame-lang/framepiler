//! Language-specific backend implementations
//!
//! Each backend implements the LanguageBackend trait to generate code
//! for a specific target language.

pub mod c;
pub mod cpp;
pub mod csharp;
pub mod dart;
pub mod erlang;
pub mod gdscript;
pub mod go;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod lua;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod swift;
pub mod typescript;
