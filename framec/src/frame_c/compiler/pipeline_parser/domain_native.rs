//! Native domain field declaration parser.
//!
//! Frame's domain section accepts native target-language field
//! declarations. Historically Frame stored each line as opaque text and
//! reached into it with heuristic helpers (`extract_var_name_from_native`,
//! `.find('=')` cascades) when codegen needed to know the field name,
//! type, or initializer separately. That approach broke for new
//! languages, multi-token types, and any field whose initializer
//! references a constructor parameter.
//!
//! This module replaces all of that with proper per-shape tokenizers.
//! The user keeps writing native syntax — Frame just parses it into
//! structured `(name, type_text, init_text)` tuples.
//!
//! ## Shapes
//!
//! Surveying every domain block in the test corpus showed that
//! native field declarations across all 17 target languages reduce to
//! exactly four distinct shapes:
//!
//! | Shape           | Form                              | Languages                                                       |
//! |-----------------|-----------------------------------|-----------------------------------------------------------------|
//! | `TypeFirst`     | `[modifiers] type name [= init]`  | C, C++, Java, C#, Kotlin, Swift, Dart, Rust (and var-bare Kotlin/Dart) |
//! | `AnnotatedName` | `[var] name: type [= init]`       | Python, TypeScript, JavaScript, Lua, PHP, Ruby, Rust, Swift, GDScript    |
//! | `GoStyle`       | `name type [= init]`              | Go                                                              |
//! | `BareName`      | `name [= init]`                   | Erlang and any dynamic-typed language with no annotation         |
//!
//! Some languages span multiple shapes (Kotlin can use TypeFirst for
//! Java-style declarations or AnnotatedName for Kotlin-native syntax).
//! The dispatcher tries shapes in a per-language priority order until
//! one succeeds.
//!
//! ## What this parser is NOT
//!
//! - It is not a target-language tokenizer in general. It only handles
//!   the subset that appears in single-line domain field declarations:
//!   identifiers, type tokens, brackets/angles for generics, optional
//!   init expressions captured verbatim.
//! - It does not interpret the init expression. The init expression is
//!   captured as a raw string from the first top-level `=` to end of
//!   line, then handed to codegen which emits it verbatim where the
//!   constructor params are in scope.
//! - It does not validate types. `Type::Custom(text)` is opaque to
//!   Frame, same as interface method param types.

use crate::frame_c::compiler::frame_ast::Type;
use crate::frame_c::visitors::TargetLanguage;

/// A successfully-parsed domain field declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDomainField {
    /// Field identifier.
    pub name: String,
    /// Field type as the user wrote it. May be `Type::Unknown` for shapes
    /// that don't carry an explicit type (BareName).
    pub var_type: Type,
    /// Initializer expression as raw target-language text, captured from
    /// the first top-level `=` to the end of the line. `None` if the
    /// field had no initializer (e.g. `int last_output` in C).
    pub init_text: Option<String>,
}

/// Reasons a shape parser can fail to recognize a line. The dispatcher
/// uses these to decide whether to try the next shape in the list or
/// surface an error to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainParseError {
    /// The line is empty or whitespace-only after the leading indent.
    Empty,
    /// The shape did not recognize the line — try the next shape.
    ShapeMismatch(String),
    /// The shape recognized the line but it was malformed in a way the
    /// user almost certainly intended to be that shape (e.g. an
    /// AnnotatedName with `name :` followed by no type). Stop trying
    /// shapes and surface this error.
    Malformed(String),
}

// ============================================================================
// Single-line tokenizer for native declaration lines
// ============================================================================

/// Token kinds the domain field tokenizer recognizes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok<'a> {
    /// `[a-zA-Z_][a-zA-Z0-9_]*`
    Ident(&'a str),
    /// `0`, `1.5`, `0xff`, etc. — captured as one token, not interpreted.
    Number(&'a str),
    /// String literal including the surrounding quotes, escape-aware.
    /// Stored as the slice from the opening quote to the closing quote inclusive.
    StringLit(&'a str),
    /// Single punctuation char or multi-char operator that's syntactically
    /// significant for our shape parsers.
    Punct(&'a str),
}

/// Position-tagged token. Position is byte offset into the original line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PosTok<'a> {
    tok: Tok<'a>,
    start: usize,
    end: usize,
}

/// Tokenize a single-line native field declaration. Returns a flat list
/// of tokens or an error if a string literal is unterminated. Whitespace
/// between tokens is dropped. Comments (`//`, `#`, `--`, `%`) end the
/// tokenization at the comment marker — useful when a line carries a
/// trailing comment.
fn tokenize(line: &str) -> Result<Vec<PosTok<'_>>, DomainParseError> {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut tokens = Vec::new();

    while i < n {
        let b = bytes[i];

        // Whitespace
        if b == b' ' || b == b'\t' || b == b'\r' {
            i += 1;
            continue;
        }

        // End of line — shouldn't appear in a single-line input but be safe
        if b == b'\n' {
            break;
        }

        // Comments end the line
        if b == b'#' {
            break; // Python, Ruby, Erlang(%), shell-style
        }
        if b == b'%' {
            break; // Erlang
        }
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            break; // C-style line comment
        }
        if b == b'-' && i + 1 < n && bytes[i + 1] == b'-' {
            break; // Lua line comment
        }

        // Identifier
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let text = &line[start..i];
            tokens.push(PosTok { tok: Tok::Ident(text), start, end: i });
            continue;
        }

        // Number — leading digit or `.digit`. Captured verbatim, not parsed.
        if b.is_ascii_digit() {
            let start = i;
            i += 1;
            // Hex / binary / octal prefix
            if i < n && (bytes[i] == b'x' || bytes[i] == b'X' || bytes[i] == b'b' || bytes[i] == b'B' || bytes[i] == b'o' || bytes[i] == b'O') {
                i += 1;
            }
            while i < n
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'_'
                    || bytes[i] == b'.')
            {
                i += 1;
            }
            let text = &line[start..i];
            tokens.push(PosTok { tok: Tok::Number(text), start, end: i });
            continue;
        }

        // String literal — handles `"..."`, `'...'`, with `\` escapes.
        // We capture the entire literal including its delimiters as a
        // single opaque blob; we never interpret its contents.
        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
            i += 1;
            while i < n && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i >= n {
                return Err(DomainParseError::Malformed(format!(
                    "unterminated string literal starting at byte {}",
                    start
                )));
            }
            i += 1; // consume closing quote
            let text = &line[start..i];
            tokens.push(PosTok { tok: Tok::StringLit(text), start, end: i });
            continue;
        }

        // Multi-char operator: `::` (path separator), `->` (return type),
        // `==` (equality, must be distinguished from assignment `=`).
        if i + 1 < n {
            let two = &line[i..i + 2];
            if matches!(two, "::" | "->" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "<<" | ">>") {
                tokens.push(PosTok { tok: Tok::Punct(two), start: i, end: i + 2 });
                i += 2;
                continue;
            }
        }

        // Single-char punctuation
        match b {
            b'=' | b':' | b',' | b';' | b'(' | b')' | b'[' | b']'
            | b'{' | b'}' | b'<' | b'>' | b'&' | b'*' | b'?' | b'!' | b'.'
            | b'+' | b'-' | b'/' | b'@' | b'|' | b'^' | b'~' => {
                let text = &line[i..i + 1];
                tokens.push(PosTok { tok: Tok::Punct(text), start: i, end: i + 1 });
                i += 1;
                continue;
            }
            _ => {
                // Unknown byte — skip it. We're tolerant by design;
                // shape parsers will fail to recognize the line and the
                // dispatcher will surface a clear error.
                i += 1;
            }
        }
    }

    Ok(tokens)
}

/// Find the index of the first top-level `=` token (not inside `<>`,
/// `()`, `[]`, or `{}`). Returns `None` if no top-level `=` exists.
/// Skips `==` and similar multi-char operators because they're emitted
/// as separate tokens (e.g. `Punct("==")`).
fn find_top_level_eq(toks: &[PosTok<'_>]) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, t) in toks.iter().enumerate() {
        match &t.tok {
            Tok::Punct(p) => match *p {
                "<" | "(" | "[" | "{" => depth += 1,
                ">" | ")" | "]" | "}" => depth -= 1,
                "=" if depth == 0 => return Some(i),
                _ => {}
            },
            _ => {}
        }
    }
    None
}

/// Slice `line` from the byte position right after the `=` token to the
/// end of the line, trimmed of leading and trailing whitespace.
fn init_text_from(line: &str, toks: &[PosTok<'_>], eq_idx: usize) -> Option<String> {
    let after_eq = toks[eq_idx].end;
    let init_slice = line[after_eq..].trim();
    if init_slice.is_empty() {
        None
    } else {
        Some(init_slice.to_string())
    }
}

/// Render a contiguous run of tokens (start_idx..end_idx, exclusive) back
/// to source text by reading the original line bytes. Preserves the
/// user's original spacing inside the run.
fn render_tokens(line: &str, toks: &[PosTok<'_>], start_idx: usize, end_idx: usize) -> String {
    if start_idx >= end_idx {
        return String::new();
    }
    let from = toks[start_idx].start;
    let to = toks[end_idx - 1].end;
    line[from..to].trim().to_string()
}

// ============================================================================
// Shape parsers
// ============================================================================

/// `TypeFirst`: `[modifiers] type [generics/ptr/ref] name [= init]`
///
/// Recognized by: at least 2 identifier-shaped tokens before the `=`
/// (or end of line), with the LAST identifier being the field name and
/// everything before it being the type-segment (modifiers + type).
///
/// Examples:
///   int last_output = -1
///   final int x
///   unsigned long long count
///   std::vector<int> items
///   const char* name = "hi"
///
/// Explicitly rejects:
/// - Lines with a top-level `:` in the head — those are AnnotatedName.
///   Without this rejection, `var name: Int = 5` would parse as TypeFirst
///   with name=`Int` and type=`var name :`, which is exactly wrong.
fn try_type_first(line: &str) -> Result<ParsedDomainField, DomainParseError> {
    let toks = tokenize(line)?;
    if toks.is_empty() {
        return Err(DomainParseError::Empty);
    }

    // Find optional `=` separator
    let eq_idx = find_top_level_eq(&toks);
    let head_end = eq_idx.unwrap_or(toks.len());

    // Reject AnnotatedName forms: a `:` punctuation in the head means
    // the user wrote `name: type`, not `type name`. Try AnnotatedName.
    for i in 0..head_end {
        if matches!(&toks[i].tok, Tok::Punct(":")) {
            return Err(DomainParseError::ShapeMismatch(
                "TypeFirst: ':' in head — line is AnnotatedName, not TypeFirst".to_string(),
            ));
        }
    }

    // The head must contain at least 2 identifier-class tokens.
    // Walk the head and remember the LAST identifier — that's the name.
    let mut last_ident_idx: Option<usize> = None;
    for i in 0..head_end {
        if matches!(&toks[i].tok, Tok::Ident(_)) {
            last_ident_idx = Some(i);
        }
    }

    let name_idx = match last_ident_idx {
        Some(i) => i,
        None => {
            return Err(DomainParseError::ShapeMismatch(
                "TypeFirst: no identifier found in declaration head".to_string(),
            ));
        }
    };

    // Need at least one token before the name to count as a type segment.
    if name_idx == 0 {
        return Err(DomainParseError::ShapeMismatch(
            "TypeFirst: only one identifier in head; not a type-first declaration".to_string(),
        ));
    }

    // Extract the name
    let name = match &toks[name_idx].tok {
        Tok::Ident(s) => s.to_string(),
        _ => unreachable!("name_idx points to an Ident by construction"),
    };

    // The type segment is everything from token 0 to name_idx (exclusive).
    // Render it back to source text to preserve modifiers, pointers,
    // angle brackets, namespaces, etc. as the user wrote them.
    let type_text = render_tokens(line, &toks, 0, name_idx);
    if type_text.is_empty() {
        return Err(DomainParseError::Malformed(
            "TypeFirst: name found but no type segment before it".to_string(),
        ));
    }

    let init_text = eq_idx.and_then(|i| init_text_from(line, &toks, i));

    Ok(ParsedDomainField {
        name,
        var_type: Type::Custom(type_text),
        init_text,
    })
}

/// `AnnotatedName`: `[var] name : type [= init]`
///
/// Recognized by: an identifier (optionally preceded by a `var`/`let`
/// keyword) followed immediately by `:`, then a type segment up to `=`
/// or end of line.
///
/// Examples:
///   last_output: int = -1
///   connection: Option<String> = None
///   var current_output: int = 0     (GDScript)
///   let mut name: String = "x"      (Rust)
fn try_annotated_name(line: &str) -> Result<ParsedDomainField, DomainParseError> {
    let toks = tokenize(line)?;
    if toks.is_empty() {
        return Err(DomainParseError::Empty);
    }

    // Optionally consume a leading `var` or `let` (and optional `mut`).
    let mut cursor = 0;
    if let Some(t) = toks.get(cursor) {
        if matches!(&t.tok, Tok::Ident("var") | Tok::Ident("let")) {
            cursor += 1;
            // Optional `mut` after `let`
            if let Some(Tok::Ident("mut")) = toks.get(cursor).map(|t| &t.tok) {
                cursor += 1;
            }
        }
    }

    // Expect identifier (the name)
    let name = match toks.get(cursor) {
        Some(PosTok { tok: Tok::Ident(s), .. }) => s.to_string(),
        _ => {
            return Err(DomainParseError::ShapeMismatch(
                "AnnotatedName: expected identifier as field name".to_string(),
            ));
        }
    };
    cursor += 1;

    // Expect `:` immediately after the name
    match toks.get(cursor) {
        Some(PosTok { tok: Tok::Punct(":"), .. }) => {
            cursor += 1;
        }
        _ => {
            return Err(DomainParseError::ShapeMismatch(
                "AnnotatedName: expected ':' after field name".to_string(),
            ));
        }
    }

    // Find the optional `=` separator from this point onward.
    let remaining = &toks[cursor..];
    let eq_in_remaining = find_top_level_eq(remaining);
    let type_end_in_remaining = eq_in_remaining.unwrap_or(remaining.len());

    if type_end_in_remaining == 0 {
        return Err(DomainParseError::Malformed(
            "AnnotatedName: ':' must be followed by a type".to_string(),
        ));
    }

    // Render the type segment
    let type_text = render_tokens(line, &toks, cursor, cursor + type_end_in_remaining);
    if type_text.is_empty() {
        return Err(DomainParseError::Malformed(
            "AnnotatedName: empty type segment after ':'".to_string(),
        ));
    }

    // Init expression, if any
    let init_text = eq_in_remaining
        .map(|local_idx| cursor + local_idx)
        .and_then(|abs_idx| init_text_from(line, &toks, abs_idx));

    Ok(ParsedDomainField {
        name,
        var_type: Type::Custom(type_text),
        init_text,
    })
}

/// `GoStyle`: `name type [= init]`
///
/// Recognized by: exactly one identifier as the FIRST token, followed
/// by something that's not `:` or `=` (the type segment), optionally
/// followed by `= init`.
///
/// Examples:
///   last_output int
///   connection *Foo = nil
///   m map[string]int
fn try_go_style(line: &str) -> Result<ParsedDomainField, DomainParseError> {
    let toks = tokenize(line)?;
    if toks.is_empty() {
        return Err(DomainParseError::Empty);
    }

    // First token must be a bare identifier.
    let name = match toks.first() {
        Some(PosTok { tok: Tok::Ident(s), .. }) => s.to_string(),
        _ => {
            return Err(DomainParseError::ShapeMismatch(
                "GoStyle: expected identifier as first token".to_string(),
            ));
        }
    };

    // Second token must NOT be `:` (that would be AnnotatedName) and
    // must NOT be `=` (that would be BareName with init).
    match toks.get(1) {
        None => {
            return Err(DomainParseError::ShapeMismatch(
                "GoStyle: only one token (would match BareName)".to_string(),
            ));
        }
        Some(PosTok { tok: Tok::Punct(":"), .. }) => {
            return Err(DomainParseError::ShapeMismatch(
                "GoStyle: ':' after name (would match AnnotatedName)".to_string(),
            ));
        }
        Some(PosTok { tok: Tok::Punct("="), .. }) => {
            return Err(DomainParseError::ShapeMismatch(
                "GoStyle: '=' after name (would match BareName)".to_string(),
            ));
        }
        _ => {} // OK, type segment starts here
    }

    // Find optional `=` separator from token 1 onward
    let remaining = &toks[1..];
    let eq_in_remaining = find_top_level_eq(remaining);
    let type_end_in_remaining = eq_in_remaining.unwrap_or(remaining.len());

    if type_end_in_remaining == 0 {
        return Err(DomainParseError::Malformed(
            "GoStyle: name with no type".to_string(),
        ));
    }

    let type_text = render_tokens(line, &toks, 1, 1 + type_end_in_remaining);

    let init_text = eq_in_remaining
        .map(|local_idx| 1 + local_idx)
        .and_then(|abs_idx| init_text_from(line, &toks, abs_idx));

    Ok(ParsedDomainField {
        name,
        var_type: Type::Custom(type_text),
        init_text,
    })
}

/// `BareName`: `name [= init]`
///
/// Recognized by: a single identifier (optionally preceded by `var`/`let`)
/// followed by either nothing or `=` and an init expression. No type.
///
/// Examples:
///   last_output = 0
///   count = 5
///   pending
fn try_bare_name(line: &str) -> Result<ParsedDomainField, DomainParseError> {
    let toks = tokenize(line)?;
    if toks.is_empty() {
        return Err(DomainParseError::Empty);
    }

    // Optionally consume a leading `var` or `let`.
    let mut cursor = 0;
    if let Some(t) = toks.get(cursor) {
        if matches!(&t.tok, Tok::Ident("var") | Tok::Ident("let")) {
            cursor += 1;
        }
    }

    // Expect identifier (the name)
    let name = match toks.get(cursor) {
        Some(PosTok { tok: Tok::Ident(s), .. }) => s.to_string(),
        _ => {
            return Err(DomainParseError::ShapeMismatch(
                "BareName: expected identifier as field name".to_string(),
            ));
        }
    };
    cursor += 1;

    // Next token must be either nothing or `=`. Anything else means this
    // isn't a BareName declaration.
    match toks.get(cursor) {
        None => Ok(ParsedDomainField {
            name,
            var_type: Type::Unknown,
            init_text: None,
        }),
        Some(PosTok { tok: Tok::Punct("="), .. }) => {
            let init_text = init_text_from(line, &toks, cursor);
            Ok(ParsedDomainField {
                name,
                var_type: Type::Unknown,
                init_text,
            })
        }
        Some(other) => Err(DomainParseError::ShapeMismatch(format!(
            "BareName: unexpected token after name: {:?}",
            other.tok
        ))),
    }
}

// ============================================================================
// Per-language dispatcher
// ============================================================================

/// Parse a single domain field declaration line for the given target
/// language. Tries the language's preferred shapes in priority order
/// until one succeeds, or returns the most specific error if all fail.
///
/// **Dispatch policy:** every language tries `AnnotatedName` first
/// (since `name: type = init` is the most user-friendly form and is
/// trivially recognized by the presence of `:`). If that fails, the
/// language's native shape is tried, then `BareName` as a final
/// fallback. This means **users can write the Frame-typed form
/// (`name: type = init`) in any source file regardless of target**, and
/// they can also use the target's native form. Both work.
///
/// The line is the raw text of one domain field declaration with leading
/// indent already stripped. It does NOT include the surrounding `domain:`
/// keyword or the closing brace.
pub fn parse_domain_field(
    line: &str,
    lang: TargetLanguage,
) -> Result<ParsedDomainField, DomainParseError> {
    // Determine the per-language priority list. AnnotatedName is tried
    // first universally because it's the most user-friendly and is
    // unambiguous (requires `:`). The language-native shape is tried
    // second. BareName is the final fallback for any language whose
    // dynamic-typed form is just `name [= init]`.
    let shapes: &[fn(&str) -> Result<ParsedDomainField, DomainParseError>] = match lang {
        // C-family languages: AnnotatedName first (Frame-typed form is
        // welcome here), then native TypeFirst, then BareName.
        TargetLanguage::C
        | TargetLanguage::Cpp
        | TargetLanguage::Java
        | TargetLanguage::CSharp
        | TargetLanguage::Kotlin
        | TargetLanguage::Dart
        | TargetLanguage::Swift
        | TargetLanguage::Rust => &[try_annotated_name, try_type_first, try_bare_name],

        // Python-family / dynamic-typed: AnnotatedName is native; BareName
        // for untyped fields.
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Lua
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::GDScript => &[try_annotated_name, try_bare_name],

        // Go: AnnotatedName first (Frame-typed form welcome), then
        // native GoStyle, then BareName.
        TargetLanguage::Go => &[try_annotated_name, try_go_style, try_bare_name],

        // Erlang: AnnotatedName (Frame-typed form welcome — type is
        // captured but discarded by the dynamic Erlang codegen), then
        // BareName for native bare assignments.
        TargetLanguage::Erlang => &[try_annotated_name, try_bare_name],

        // Graphviz never has domain blocks but be safe.
        TargetLanguage::Graphviz => &[try_annotated_name, try_bare_name],
    };

    let mut last_err: Option<DomainParseError> = None;
    for shape in shapes {
        match shape(line) {
            Ok(field) => return Ok(field),
            Err(DomainParseError::Malformed(msg)) => {
                // Malformed errors are definitive — the user clearly
                // intended this shape but got the syntax wrong. Surface
                // immediately rather than trying the next shape.
                return Err(DomainParseError::Malformed(msg));
            }
            Err(e @ DomainParseError::ShapeMismatch(_)) => {
                last_err = Some(e);
            }
            Err(e @ DomainParseError::Empty) => return Err(e),
        }
    }

    Err(last_err.unwrap_or_else(|| {
        DomainParseError::ShapeMismatch(
            "no shape parser matched the line for this language".to_string(),
        )
    }))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ----- Tokenizer tests -----

    #[test]
    fn tokenizes_simple_ident() {
        let toks = tokenize("foo").unwrap();
        assert_eq!(toks.len(), 1);
        assert!(matches!(toks[0].tok, Tok::Ident("foo")));
    }

    #[test]
    fn tokenizes_int_decl() {
        let toks = tokenize("int count = 0").unwrap();
        assert_eq!(toks.len(), 4);
        assert!(matches!(toks[0].tok, Tok::Ident("int")));
        assert!(matches!(toks[1].tok, Tok::Ident("count")));
        assert!(matches!(toks[2].tok, Tok::Punct("=")));
        assert!(matches!(toks[3].tok, Tok::Number("0")));
    }

    #[test]
    fn tokenizes_string_with_escape() {
        let toks = tokenize(r#"name = "hi \"world\"""#).unwrap();
        assert_eq!(toks.len(), 3);
        assert!(matches!(toks[0].tok, Tok::Ident("name")));
        assert!(matches!(toks[1].tok, Tok::Punct("=")));
        match &toks[2].tok {
            Tok::StringLit(s) => assert_eq!(*s, r#""hi \"world\"""#),
            _ => panic!("expected StringLit"),
        }
    }

    #[test]
    fn tokenizes_generic_type() {
        let toks = tokenize("Map<String, Integer> m = make()").unwrap();
        // Map < String , Integer > m = make ( )
        assert_eq!(toks.len(), 11);
        assert!(matches!(toks[0].tok, Tok::Ident("Map")));
        assert!(matches!(toks[1].tok, Tok::Punct("<")));
        assert!(matches!(toks[2].tok, Tok::Ident("String")));
        assert!(matches!(toks[3].tok, Tok::Punct(",")));
        assert!(matches!(toks[4].tok, Tok::Ident("Integer")));
        assert!(matches!(toks[5].tok, Tok::Punct(">")));
        assert!(matches!(toks[6].tok, Tok::Ident("m")));
        assert!(matches!(toks[7].tok, Tok::Punct("=")));
    }

    #[test]
    fn find_top_level_eq_skips_inside_brackets() {
        let toks = tokenize("Map<int = 5> m = init").unwrap();
        // The `=` inside <...> should not count as the top-level `=`.
        let idx = find_top_level_eq(&toks).unwrap();
        // The top-level `=` is the second one (after `m`).
        match &toks[idx].tok {
            Tok::Punct("=") => {}
            _ => panic!("expected Punct(\"=\")"),
        }
        // Token at idx-1 should be the field name `m`.
        assert!(matches!(toks[idx - 1].tok, Tok::Ident("m")));
    }

    #[test]
    fn find_top_level_eq_skips_double_eq() {
        let toks = tokenize("bool ok = (a == b)").unwrap();
        let idx = find_top_level_eq(&toks).unwrap();
        // The `==` inside (...) doesn't count, but it's also not at depth 0
        // — and it's tokenized as Punct("==") so it can't be confused with `=`.
        // The match must be the standalone `=` after `ok`.
        assert!(matches!(toks[idx - 1].tok, Tok::Ident("ok")));
    }

    // ----- TypeFirst tests -----

    #[test]
    fn type_first_simple() {
        let f = try_type_first("int count = 0").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, Some("0".to_string()));
    }

    #[test]
    fn type_first_no_init() {
        let f = try_type_first("int count").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, None);
    }

    #[test]
    fn type_first_multi_token_type() {
        let f = try_type_first("unsigned long long count = 0").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("unsigned long long".to_string()));
        assert_eq!(f.init_text, Some("0".to_string()));
    }

    #[test]
    fn type_first_with_modifiers() {
        let f = try_type_first("final int x = 5").unwrap();
        assert_eq!(f.name, "x");
        assert_eq!(f.var_type, Type::Custom("final int".to_string()));
        assert_eq!(f.init_text, Some("5".to_string()));
    }

    #[test]
    fn type_first_with_pointer() {
        let f = try_type_first("char* name = \"hi\"").unwrap();
        assert_eq!(f.name, "name");
        assert_eq!(f.var_type, Type::Custom("char*".to_string()));
        assert_eq!(f.init_text, Some("\"hi\"".to_string()));
    }

    #[test]
    fn type_first_with_generic() {
        let f = try_type_first("Map<String, Integer> m = make()").unwrap();
        assert_eq!(f.name, "m");
        assert_eq!(f.var_type, Type::Custom("Map<String, Integer>".to_string()));
        assert_eq!(f.init_text, Some("make()".to_string()));
    }

    #[test]
    fn type_first_init_with_eq_inside() {
        let f = try_type_first("bool ok = (a == b)").unwrap();
        assert_eq!(f.name, "ok");
        assert_eq!(f.var_type, Type::Custom("bool".to_string()));
        assert_eq!(f.init_text, Some("(a == b)".to_string()));
    }

    #[test]
    fn type_first_string_with_eq_inside() {
        let f = try_type_first("String s = \"a=b\"").unwrap();
        assert_eq!(f.name, "s");
        assert_eq!(f.init_text, Some("\"a=b\"".to_string()));
    }

    #[test]
    fn type_first_rejects_single_ident() {
        // Only one identifier — not type-first
        assert!(matches!(
            try_type_first("count"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    #[test]
    fn type_first_rejects_annotated_form() {
        // `name : type` form — explicitly rejected by TypeFirst because
        // the `:` in the head signals AnnotatedName. The dispatcher then
        // falls through to AnnotatedName for any language that lists
        // both shapes.
        assert!(matches!(
            try_type_first("count: int"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    #[test]
    fn type_first_rejects_var_annotated_form() {
        // Kotlin's `var name: Int = 5` form must be rejected so the
        // dispatcher falls through to AnnotatedName.
        assert!(matches!(
            try_type_first("var name: Int = 5"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    // ----- AnnotatedName tests -----

    #[test]
    fn annotated_name_simple() {
        let f = try_annotated_name("count: int = 5").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, Some("5".to_string()));
    }

    #[test]
    fn annotated_name_no_init() {
        let f = try_annotated_name("count: int").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, None);
    }

    #[test]
    fn annotated_name_with_var_keyword() {
        let f = try_annotated_name("var current_output: int = 0").unwrap();
        assert_eq!(f.name, "current_output");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, Some("0".to_string()));
    }

    #[test]
    fn annotated_name_with_let_mut() {
        let f = try_annotated_name("let mut name: String = \"x\"").unwrap();
        assert_eq!(f.name, "name");
        assert_eq!(f.var_type, Type::Custom("String".to_string()));
        assert_eq!(f.init_text, Some("\"x\"".to_string()));
    }

    #[test]
    fn annotated_name_with_generic_type() {
        let f = try_annotated_name("connection: Option<String> = None").unwrap();
        assert_eq!(f.name, "connection");
        assert_eq!(f.var_type, Type::Custom("Option<String>".to_string()));
        assert_eq!(f.init_text, Some("None".to_string()));
    }

    #[test]
    fn annotated_name_rejects_no_colon() {
        assert!(matches!(
            try_annotated_name("int count"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    #[test]
    fn annotated_name_rejects_bare() {
        assert!(matches!(
            try_annotated_name("count = 5"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    // ----- GoStyle tests -----

    #[test]
    fn go_style_simple() {
        let f = try_go_style("count int = 0").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, Some("0".to_string()));
    }

    #[test]
    fn go_style_no_init() {
        let f = try_go_style("count int").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, None);
    }

    #[test]
    fn go_style_pointer_type() {
        let f = try_go_style("conn *Foo = nil").unwrap();
        assert_eq!(f.name, "conn");
        assert_eq!(f.var_type, Type::Custom("*Foo".to_string()));
        assert_eq!(f.init_text, Some("nil".to_string()));
    }

    #[test]
    fn go_style_map_type() {
        let f = try_go_style("m map[string]int").unwrap();
        assert_eq!(f.name, "m");
        assert_eq!(f.var_type, Type::Custom("map[string]int".to_string()));
    }

    #[test]
    fn go_style_rejects_annotated() {
        assert!(matches!(
            try_go_style("count: int"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    #[test]
    fn go_style_rejects_bare_with_init() {
        assert!(matches!(
            try_go_style("count = 5"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    // ----- BareName tests -----

    #[test]
    fn bare_name_simple() {
        let f = try_bare_name("count = 0").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Unknown);
        assert_eq!(f.init_text, Some("0".to_string()));
    }

    #[test]
    fn bare_name_no_init() {
        let f = try_bare_name("count").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Unknown);
        assert_eq!(f.init_text, None);
    }

    #[test]
    fn bare_name_with_var() {
        let f = try_bare_name("var count = 5").unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.init_text, Some("5".to_string()));
    }

    #[test]
    fn bare_name_string_init() {
        let f = try_bare_name("name = \"hello\"").unwrap();
        assert_eq!(f.name, "name");
        assert_eq!(f.init_text, Some("\"hello\"".to_string()));
    }

    #[test]
    fn bare_name_rejects_typed() {
        assert!(matches!(
            try_bare_name("count: int = 5"),
            Err(DomainParseError::ShapeMismatch(_))
        ));
    }

    // ----- Dispatcher tests -----

    #[test]
    fn dispatch_python_annotated() {
        let f = parse_domain_field("count: int = 5", TargetLanguage::Python3).unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
    }

    #[test]
    fn dispatch_python_bare() {
        let f = parse_domain_field("count = 5", TargetLanguage::Python3).unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Unknown);
    }

    #[test]
    fn dispatch_c_type_first() {
        let f = parse_domain_field("int count = 0", TargetLanguage::C).unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
    }

    #[test]
    fn dispatch_c_pointer() {
        let f = parse_domain_field("char* last", TargetLanguage::C).unwrap();
        assert_eq!(f.name, "last");
        assert_eq!(f.var_type, Type::Custom("char*".to_string()));
        assert_eq!(f.init_text, None);
    }

    #[test]
    fn dispatch_go() {
        let f = parse_domain_field("count int = 0", TargetLanguage::Go).unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, Some("0".to_string()));
    }

    #[test]
    fn dispatch_erlang_bare() {
        let f = parse_domain_field("count = 5", TargetLanguage::Erlang).unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Unknown);
        assert_eq!(f.init_text, Some("5".to_string()));
    }

    #[test]
    fn dispatch_swift_annotated() {
        let f = parse_domain_field("balance: Int = 0", TargetLanguage::Swift).unwrap();
        assert_eq!(f.name, "balance");
        assert_eq!(f.var_type, Type::Custom("Int".to_string()));
    }

    #[test]
    fn dispatch_rust_with_generic() {
        let f = parse_domain_field(
            "connection: Option<String> = None",
            TargetLanguage::Rust,
        )
        .unwrap();
        assert_eq!(f.name, "connection");
        assert_eq!(f.var_type, Type::Custom("Option<String>".to_string()));
    }

    #[test]
    fn dispatch_java_with_string_init() {
        let f = parse_domain_field("String last_data = \"\"", TargetLanguage::Java).unwrap();
        assert_eq!(f.name, "last_data");
        assert_eq!(f.var_type, Type::Custom("String".to_string()));
        assert_eq!(f.init_text, Some("\"\"".to_string()));
    }

    #[test]
    fn dispatch_gdscript_var_prefixed() {
        let f = parse_domain_field("var current_output: int = 0", TargetLanguage::GDScript).unwrap();
        assert_eq!(f.name, "current_output");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
    }

    #[test]
    fn dispatch_gdscript_no_var() {
        let f = parse_domain_field("current_output: int = 0", TargetLanguage::GDScript).unwrap();
        assert_eq!(f.name, "current_output");
    }

    #[test]
    fn dispatch_kotlin_typed_kotlin_native() {
        // Kotlin's `var name: Int = 5` form
        let f = parse_domain_field("var name: Int = 5", TargetLanguage::Kotlin).unwrap();
        assert_eq!(f.name, "name");
        assert_eq!(f.var_type, Type::Custom("Int".to_string()));
    }

    #[test]
    fn dispatch_kotlin_java_compatible() {
        // Java-compatible TypeFirst form
        let f = parse_domain_field("int count = 0", TargetLanguage::Kotlin).unwrap();
        assert_eq!(f.name, "count");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
    }

    #[test]
    fn dispatch_referencing_constructor_param() {
        // The init expression references a constructor param. Frame
        // doesn't care — it captures the text verbatim and codegen
        // emits it where the param is in scope.
        let f = parse_domain_field("int balance = initial", TargetLanguage::C).unwrap();
        assert_eq!(f.name, "balance");
        assert_eq!(f.var_type, Type::Custom("int".to_string()));
        assert_eq!(f.init_text, Some("initial".to_string()));
    }

    #[test]
    fn dispatch_init_calls_function() {
        let f = parse_domain_field(
            "auto items = make_vector(1, 2, 3)",
            TargetLanguage::Cpp,
        )
        .unwrap();
        assert_eq!(f.name, "items");
        assert_eq!(f.var_type, Type::Custom("auto".to_string()));
        assert_eq!(f.init_text, Some("make_vector(1, 2, 3)".to_string()));
    }
}
