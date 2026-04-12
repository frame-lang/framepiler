//! Output block transformation — two-stage lexer/parser.
//!
//! Transforms generated Frame output from brace-delimited blocks to
//! target language syntax (Lua: if/then/end, Erlang: case/of/end).
//!
//! Architecture:
//!   Stage 1: OutputBlockLexer tokenizes text (respects strings/comments)
//!   Stage 2: OutputBlockParser consumes tokens, emits transformed text
//!
//! Both stages are Frame state machines (.frs → .gen.rs).

#[allow(unreachable_patterns)]
#[allow(unused_mut)]
#[allow(dead_code)]
#[allow(non_snake_case)]
#[allow(unused_variables)]
mod _output_block_lexer {
    include!("output_block_lexer.gen.rs");
}

#[allow(unreachable_patterns)]
#[allow(unused_mut)]
#[allow(dead_code)]
#[allow(non_snake_case)]
#[allow(unused_variables)]
mod _output_block_parser {
    include!("output_block_parser.gen.rs");
}

use _output_block_lexer::OutputBlockLexerFsm;
use _output_block_parser::OutputBlockParserFsm;

/// Block transformation mode
#[derive(Clone, Copy)]
pub enum BlockTransformMode {
    /// Lua: if/then/elseif/else/end, while/do/end
    Lua = 1,
    /// Erlang: case/of/true->/false->/end (future)
    Erlang = 2,
}

/// Transform generated output from Frame brace blocks to target language syntax.
///
/// Uses two Frame state machines:
/// 1. OutputBlockLexer: tokenizes text, skipping strings/comments
/// 2. OutputBlockParser: consumes tokens, emits transformed text
pub fn transform_blocks(text: &str, mode: BlockTransformMode) -> String {
    if text.is_empty() {
        return String::new();
    }

    let bytes = text.as_bytes();

    // Configure lexer for the target language
    let (comment_char, comment_double) = match mode {
        BlockTransformMode::Lua => (b'-', true),     // -- comments
        BlockTransformMode::Erlang => (b'%', false), // % comments
    };

    // Stage 1: Lex
    let mut lexer = OutputBlockLexerFsm::new();
    lexer.bytes = bytes.to_vec();
    lexer.end = bytes.len();
    lexer.comment_char = comment_char;
    lexer.comment_double = comment_double;
    lexer.do_lex();

    // Stage 2: Parse
    let mut parser = OutputBlockParserFsm::new();
    parser.bytes = bytes.to_vec();
    parser.mode = mode as usize;
    parser.token_kinds = lexer.token_kinds;
    parser.token_starts = lexer.token_starts;
    parser.token_ends = lexer.token_ends;
    parser.do_parse();

    parser.result
}
