//! Domain-section parser.
//!
//! Parses Frame's `domain:` section — a list of fields, each in the canonical
//! `name [: type] = init` form. Type and initializer strings are opaque to
//! the parser (Frame's transpiler treats them as native-language source); the
//! parser only frames them with byte spans.

use super::{ParseError, Parser};
use crate::frame_c::compiler::frame_ast::*;
use crate::frame_c::compiler::lexer::Token;

impl<'a> Parser<'a> {
    /// Parse the domain section.
    ///
    /// Each field uses canonical Frame syntax: `name [: type] = init`.
    /// Type and init are opaque strings — Frame doesn't interpret them.
    /// Multi-line init via `= ( ... )` wrapper is supported.
    pub(super) fn parse_domain(&mut self) -> Result<Vec<DomainVar>, ParseError> {
        let mut vars = Vec::new();
        let src = self.lexer.source();
        let mut pos = self.lexer.cursor();
        let lang = self.lexer.lang();
        // Domain block walks bytes manually rather than tokenizing
        // through the lexer, so pending-comment capture lives here too.
        // Comments accumulate in `pending_doc` while we step over
        // comment-only lines (`# foo`); when we successfully parse a
        // field, we drain them onto its `leading_comments`.
        let mut pending_doc: Vec<String> = Vec::new();
        // RFC-0013 wave 2 phase 2: pending attributes (from `@@[name(args?)]`
        // lines preceding a field). Drain into the field's `attributes`
        // when we successfully parse one.
        let mut pending_attrs: Vec<crate::frame_c::compiler::frame_ast::Attribute> = Vec::new();

        // Skip initial whitespace/newlines after `domain:`
        while pos < src.len()
            && (src[pos] == b' ' || src[pos] == b'\t' || src[pos] == b'\n' || src[pos] == b'\r')
        {
            pos += 1;
        }

        while pos < src.len() {
            // Skip blank lines and whitespace
            while pos < src.len() && (src[pos] == b'\n' || src[pos] == b'\r') {
                pos += 1;
            }
            if pos >= src.len() {
                break;
            }

            // Find indentation level and start of content
            let line_start = pos;
            while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
                pos += 1;
            }
            if pos >= src.len() {
                break;
            }

            // Check if this line starts a new section or closes the system block.
            // Section keywords at the same or lower indent level end the domain.
            // Also check for `}` which closes the @@system block.
            if src[pos] == b'}' {
                // Reset cursor to before the `}` so the system parser sees it
                self.lexer.set_cursor(line_start);
                break;
            }

            // RFC-0013 wave 2 phase 2: attribute `@@[name(args?)]`
            // attaches to the next field. Accepted on its own line or
            // immediately preceding the field on the same line. Multiple
            // attributes (separated by whitespace) accumulate.
            while pos + 2 < src.len()
                && src[pos] == b'@'
                && src[pos + 1] == b'@'
                && src[pos + 2] == b'['
            {
                let attr_start = pos;
                pos += 3; // skip @@[
                let name_start = pos;
                while pos < src.len() {
                    let c = src[pos];
                    if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                        pos += 1;
                    } else {
                        break;
                    }
                }
                let name = std::str::from_utf8(&src[name_start..pos])
                    .unwrap_or("")
                    .to_string();
                let mut args: Option<String> = None;
                if pos < src.len() && src[pos] == b'(' {
                    let args_inner_start = pos + 1;
                    pos += 1;
                    let mut depth: i32 = 1;
                    while pos < src.len() && depth > 0 {
                        match src[pos] {
                            b'(' => depth += 1,
                            b')' => depth -= 1,
                            _ => {}
                        }
                        pos += 1;
                    }
                    if depth == 0 {
                        args = Some(
                            std::str::from_utf8(&src[args_inner_start..pos - 1])
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                }
                if pos < src.len() && src[pos] == b']' {
                    pos += 1;
                }
                pending_attrs.push(crate::frame_c::compiler::frame_ast::Attribute {
                    name,
                    args,
                    span: Span::new(attr_start, pos),
                });
                // Skip same-line whitespace; if we hit newline, attribute
                // was on its own line — outer loop iterates fresh and any
                // following attribute or field is parsed normally. If we
                // hit non-whitespace, the field starts here on the same
                // line and falls through to the word scan below.
                while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
                    pos += 1;
                }
                if pos < src.len() && (src[pos] == b'\n' || src[pos] == b'\r') {
                    break;
                }
                // Otherwise loop to allow back-to-back same-line attrs
                // like `@@[a] @@[b] field = ...`.
            }
            // If we just consumed an attribute and are now at a newline,
            // restart the outer loop so newline-skipping re-runs.
            if !pending_attrs.is_empty()
                && pos < src.len()
                && (src[pos] == b'\n' || src[pos] == b'\r')
            {
                continue;
            }

            // Peek at the word to see if it's a section keyword
            let word_start = pos;
            while pos < src.len() && (src[pos].is_ascii_alphanumeric() || src[pos] == b'_') {
                pos += 1;
            }
            let word = std::str::from_utf8(&src[word_start..pos]).unwrap_or("");

            // Check for section keywords followed by `:` (with optional whitespace)
            let mut check_pos = pos;
            while check_pos < src.len() && (src[check_pos] == b' ' || src[check_pos] == b'\t') {
                check_pos += 1;
            }
            let followed_by_colon = check_pos < src.len() && src[check_pos] == b':';

            if followed_by_colon
                && matches!(
                    word,
                    "interface" | "machine" | "actions" | "operations" | "domain"
                )
            {
                // This is a new section — stop parsing domain
                self.lexer.set_cursor(line_start);
                break;
            }

            // Parse canonical domain field: [const] name [: type] = init
            let field_start = word_start;

            // 0. Check for `const` modifier
            let (is_const, name) = if word == "const" {
                while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
                    pos += 1;
                }
                let name_start = pos;
                while pos < src.len() && (src[pos].is_ascii_alphanumeric() || src[pos] == b'_') {
                    pos += 1;
                }
                (
                    true,
                    std::str::from_utf8(&src[name_start..pos])
                        .unwrap_or("")
                        .to_string(),
                )
            } else {
                (false, word.to_string())
            };

            if name.is_empty() {
                // Comment-only or unparseable line. Capture the
                // comment text (if any) into `pending_doc` so the
                // next real field declaration can claim it as a
                // leading-comment trivia. The capture handles the
                // common `# comment` and `// comment` forms — Frame
                // source for a target language already uses that
                // language's comment leader (Oceans Model), so
                // codegen emits the captured text verbatim.
                let line_text_start = word_start;
                while pos < src.len() && src[pos] != b'\n' {
                    pos += 1;
                }
                let raw = std::str::from_utf8(&src[line_text_start..pos])
                    .unwrap_or("")
                    .trim_end_matches('\r')
                    .trim_end();
                if !raw.is_empty() {
                    pending_doc.push(raw.to_string());
                }
                continue;
            }

            // Skip whitespace after name
            while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
                pos += 1;
            }

            // 2. Optional type: if ':' follows
            let var_type = if pos < src.len() && src[pos] == b':' {
                pos += 1; // consume ':'
                          // Skip whitespace after ':'
                while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
                    pos += 1;
                }
                // Scan type slot until top-level '=' (bracket-aware for generics)
                let type_start = pos;
                let mut bracket_depth: i32 = 0;
                while pos < src.len() && src[pos] != b'\n' {
                    match src[pos] {
                        b'<' | b'(' | b'[' | b'{' => {
                            bracket_depth += 1;
                            pos += 1;
                        }
                        b'>' | b')' | b']' | b'}' => {
                            bracket_depth -= 1;
                            pos += 1;
                        }
                        b'=' if bracket_depth == 0 => break,
                        _ => {
                            pos += 1;
                        }
                    }
                }
                let type_text = std::str::from_utf8(&src[type_start..pos])
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if type_text.is_empty() {
                    Type::Unknown
                } else {
                    Type::Custom(type_text)
                }
            } else {
                Type::Unknown
            };

            // Skip whitespace before '='
            while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
                pos += 1;
            }

            // 3. Optional '=' followed by init expression
            if pos >= src.len() || src[pos] == b'\n' || src[pos] != b'=' {
                // No '=' — field with no initializer (e.g., `count : int`)
                while pos < src.len() && src[pos] != b'\n' {
                    pos += 1;
                }
                vars.push(DomainVar {
                    name,
                    var_type,
                    initializer_text: None,
                    is_const,
                    leading_comments: std::mem::take(&mut pending_doc),
                    attributes: std::mem::take(&mut pending_attrs),
                    span: Span::new(field_start, pos),
                });
                continue;
            }
            pos += 1; // consume '='

            // 4. Capture init expression (opaque)
            // Skip whitespace after '='
            while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
                pos += 1;
            }

            // Check for multi-line wrapper: '(' on this line with no matching ')' on same line
            let init_text = if pos < src.len() && src[pos] == b'(' {
                // Check if ')' is on the same line
                let paren_pos = pos;
                let mut check = pos + 1;
                let mut depth = 1i32;
                let mut same_line = false;
                while check < src.len() && src[check] != b'\n' {
                    match src[check] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                same_line = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                    check += 1;
                }

                if same_line {
                    // Single-line — capture to EOL
                    let init_start = pos;
                    while pos < src.len() && src[pos] != b'\n' {
                        pos += 1;
                    }
                    std::str::from_utf8(&src[init_start..pos])
                        .unwrap_or("")
                        .trim_end()
                        .to_string()
                } else {
                    // Multi-line wrapper: scan from after '(' to matching ')'
                    pos = paren_pos + 1; // after opening '('
                    let wrapper_content_start = pos;
                    let mut depth = 1i32;
                    while pos < src.len() && depth > 0 {
                        match src[pos] {
                            b'(' | b'[' | b'{' => depth += 1,
                            b')' | b']' | b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break; // pos is at the closing ')'
                                }
                            }
                            b'"' => {
                                // Skip string literal
                                pos += 1;
                                while pos < src.len() && src[pos] != b'"' {
                                    if src[pos] == b'\\' && pos + 1 < src.len() {
                                        pos += 1;
                                    }
                                    pos += 1;
                                }
                            }
                            b'\'' => {
                                // Skip char/string literal
                                pos += 1;
                                while pos < src.len() && src[pos] != b'\'' {
                                    if src[pos] == b'\\' && pos + 1 < src.len() {
                                        pos += 1;
                                    }
                                    pos += 1;
                                }
                            }
                            _ => {}
                        }
                        pos += 1;
                    }
                    if depth != 0 {
                        return Err(ParseError {
                            message: format!(
                                "domain field '{}': unterminated multi-line initializer '('",
                                name
                            ),
                            span: Span::new(paren_pos, pos),
                        });
                    }
                    // pos is now past the closing ')' — capture content between outer parens
                    let wrapper_content_end = pos - 1; // before ')'
                    let init =
                        std::str::from_utf8(&src[wrapper_content_start..wrapper_content_end])
                            .unwrap_or("")
                            .trim()
                            .to_string();
                    // Skip past any trailing whitespace on the closing ')' line
                    while pos < src.len() && src[pos] != b'\n' {
                        if src[pos] != b' ' && src[pos] != b'\t' {
                            return Err(ParseError {
                                message: format!(
                                    "domain field '{}': unexpected tokens after closing ')'",
                                    name
                                ),
                                span: Span::new(pos, pos + 1),
                            });
                        }
                        pos += 1;
                    }
                    init
                }
            } else {
                // Single-line init — capture to EOL
                let init_start = pos;
                while pos < src.len() && src[pos] != b'\n' {
                    pos += 1;
                }
                std::str::from_utf8(&src[init_start..pos])
                    .unwrap_or("")
                    .trim_end()
                    .to_string()
            };

            let init_opt = if init_text.is_empty() {
                None
            } else {
                Some(init_text)
            };

            vars.push(DomainVar {
                name,
                var_type,
                initializer_text: init_opt,
                is_const,
                leading_comments: std::mem::take(&mut pending_doc),
                attributes: std::mem::take(&mut pending_attrs),
                span: Span::new(field_start, pos),
            });
        }

        self.lexer.set_cursor(pos);
        // Drain any remaining tokens the lexer may have buffered for the domain section
        loop {
            let tok = self.peek()?;
            match tok {
                Token::Interface
                | Token::Machine
                | Token::Actions
                | Token::Operations
                | Token::Eof
                | Token::Domain => break,
                Token::RBrace => break,
                _ => {
                    self.advance()?;
                }
            }
        }
        Ok(vars)
    }
}
