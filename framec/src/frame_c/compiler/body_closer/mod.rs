#[derive(Debug)]
pub enum CloseErrorKind {
    Unimplemented,
    UnterminatedString,
    UnterminatedComment,
    UnterminatedRawString,
    UnmatchedBraces,
}

#[derive(Debug)]
pub struct CloseError {
    pub kind: CloseErrorKind,
    pub message: String,
}

impl CloseError {
    pub fn unimplemented() -> Self {
        CloseError { kind: CloseErrorKind::Unimplemented, message: "Body closer not yet implemented".to_string() }
    }
}

pub trait BodyCloser {
    fn close_byte(&mut self, bytes: &[u8], open_brace_index: usize) -> Result<usize, CloseError>;
}

pub mod python;
pub mod typescript;
pub mod csharp;
pub mod c;
pub mod cpp;
pub mod java;
pub mod rust;
pub mod go;
pub mod javascript;
pub mod php;
pub mod kotlin;
pub mod swift;
pub mod ruby;
pub mod erlang;
pub mod lua;
pub mod dart;
pub mod gdscript;

use crate::frame_c::visitors::TargetLanguage;

/// Single dispatch point for language-specific body closers.
/// Given the full byte slice, the position of the opening `{`, and the target language,
/// returns the absolute position of the matching closing `}`.
pub fn close_body(bytes: &[u8], open: usize, lang: TargetLanguage) -> Result<usize, CloseError> {
    match lang {
        TargetLanguage::Python3 => python::BodyCloserPy.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::TypeScript => typescript::BodyCloserTs.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::CSharp => csharp::BodyCloserCs.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::C => c::BodyCloserC.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Cpp => cpp::BodyCloserCpp.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Java => java::BodyCloserJava.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Rust => rust::BodyCloserRust.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Go => go::BodyCloserGo.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::JavaScript => javascript::BodyCloserJs.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Php => php::BodyCloserPhp.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Kotlin => kotlin::BodyCloserKotlin.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Swift => swift::BodyCloserSwift.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Ruby => ruby::BodyCloserRuby.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Erlang => erlang::BodyCloserErlang.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Lua => lua::BodyCloserLua.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Dart => dart::BodyCloserDart.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::GDScript => gdscript::BodyCloserGDScript.close_byte(&bytes[open..], 0).map(|c| open + c),
        TargetLanguage::Graphviz => Err(CloseError { kind: CloseErrorKind::Unimplemented, message: "Graphviz does not use body closers".into() }),
    }
}
