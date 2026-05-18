//! Frame-statement lexing in native-aware mode.
//!
//! When the lexer is inside a handler/action/operation body it must recognize
//! the small set of Frame-specific statements that can appear there: `->`,
//! `=>`, `(args) ->` (exit-args transition), `push$`, `pop$`, `@@:` /
//! `@@:self`, and `$`-sigil references. This module groups those routines
//! together; the entry point is [`Lexer::try_sol_frame_statement`], called
//! by `advance_native` at each statement-start position.

use super::{LexError, Lexer, Span, Spanned, Token};

impl<'a> Lexer<'a> {
    // ========================================================================
    // SOL Frame Statement Detection (Native-Aware Mode)
    // ========================================================================

    pub(super) fn try_sol_frame_statement(
        &mut self,
        end: usize,
    ) -> Result<Option<Vec<Spanned>>, LexError> {
        let pos = self.cursor;

        if pos >= end {
            return Ok(None);
        }

        let b = self.source[pos];

        // Check for backtick prefix (V4 embedded Frame statement syntax)
        let mut check_pos = pos;
        if b == b'`' {
            check_pos += 1;
            while check_pos < end
                && (self.source[check_pos] == b' ' || self.source[check_pos] == b'\t')
            {
                check_pos += 1;
            }
            if check_pos >= end {
                return Ok(None);
            }
        }

        let cb = self.source[check_pos];

        // ---- Transition: -> ----
        if cb == b'-' && check_pos + 1 < end && self.source[check_pos + 1] == b'>' {
            return self.lex_sol_transition(check_pos, end);
        }

        // ---- Forward: => ----
        if cb == b'=' && check_pos + 1 < end && self.source[check_pos + 1] == b'>' {
            return self.lex_sol_forward(check_pos, end);
        }

        // ---- Transition with exit args: (exit_args) -> ... ----
        if cb == b'(' {
            if let Some(result) = self.try_lex_exit_args_transition(check_pos, end)? {
                return Ok(Some(result));
            }
        }

        // ---- push$ ----
        if cb == b'p'
            && check_pos + 4 < end
            && self.source[check_pos + 1] == b'u'
            && self.source[check_pos + 2] == b's'
            && self.source[check_pos + 3] == b'h'
            && self.source[check_pos + 4] == b'$'
        {
            self.cursor = check_pos + 5;
            let tokens = vec![Spanned {
                token: Token::PushState,
                span: Span::new(check_pos, self.cursor),
            }];
            self.skip_to_newline(end);
            return Ok(Some(tokens));
        }

        // ---- pop$ (standalone) ----
        if cb == b'p'
            && check_pos + 3 < end
            && self.source[check_pos + 1] == b'o'
            && self.source[check_pos + 2] == b'p'
            && self.source[check_pos + 3] == b'$'
        {
            self.cursor = check_pos + 4;
            let tokens = vec![Spanned {
                token: Token::PopState,
                span: Span::new(check_pos, self.cursor),
            }];
            self.skip_to_newline(end);
            return Ok(Some(tokens));
        }

        // ---- return <expr> (Frame return sugar) ----
        if cb == b'r' && check_pos + 6 <= end && &self.source[check_pos..check_pos + 6] == b"return"
        {
            let after_return = check_pos + 6;
            if after_return < end
                && (self.source[after_return] == b' ' || self.source[after_return] == b'\t')
            {
                self.cursor = after_return;
                let mut tokens = vec![Spanned {
                    token: Token::Return,
                    span: Span::new(check_pos, self.cursor),
                }];
                // Capture the expression after return as native code
                let line_end = self.skipper.find_line_end(self.source, self.cursor, end);
                if self.cursor < line_end {
                    let expr =
                        String::from_utf8_lossy(&self.source[self.cursor..line_end]).to_string();
                    tokens.push(Spanned {
                        token: Token::NativeCode(expr),
                        span: Span::new(self.cursor, line_end),
                    });
                }
                self.cursor = line_end;
                if self.cursor < end && self.source[self.cursor] == b'\n' {
                    self.cursor += 1;
                }
                return Ok(Some(tokens));
            }
        }

        Ok(None)
    }

    /// Peek past `->` (+ optional whitespace, enter args, label, `=> `) to confirm
    /// a Frame transition target (`$State`, `pop$`) follows.  Returns `true` when
    /// the `->` is almost certainly a Frame transition, `false` when it's more
    /// likely a native operator (C++ `ptr->member`, Rust closure `|x| -> T`, etc.).
    ///
    /// `after_arrow` is the byte position immediately after `>` in `->`.
    pub(super) fn peek_frame_transition_target(&self, after_arrow: usize, end: usize) -> bool {
        let mut k = after_arrow;
        // Skip whitespace
        while k < end && (self.source[k] == b' ' || self.source[k] == b'\t') {
            k += 1;
        }
        if k >= end {
            return false;
        }
        // -> => $State (transition forward)
        if k + 1 < end && self.source[k] == b'=' && self.source[k + 1] == b'>' {
            k += 2;
            while k < end && (self.source[k] == b' ' || self.source[k] == b'\t') {
                k += 1;
            }
            return k < end && self.source[k] == b'$';
        }
        // -> pop$
        if k + 3 < end
            && self.source[k] == b'p'
            && self.source[k + 1] == b'o'
            && self.source[k + 2] == b'p'
            && self.source[k + 3] == b'$'
        {
            return true;
        }
        // -> (args) ...  — skip balanced parens, then look for $
        if self.source[k] == b'(' {
            if let Some(k2) = self.skipper.balanced_paren_end(self.source, k, end) {
                k = k2;
                while k < end && (self.source[k] == b' ' || self.source[k] == b'\t') {
                    k += 1;
                }
                // After (args), skip optional "label"
                if k < end && (self.source[k] == b'"' || self.source[k] == b'\'') {
                    let quote = self.source[k];
                    k += 1;
                    while k < end && self.source[k] != quote {
                        if self.source[k] == b'\\' && k + 1 < end {
                            k += 2;
                        } else {
                            k += 1;
                        }
                    }
                    if k < end {
                        k += 1;
                    }
                    while k < end && (self.source[k] == b' ' || self.source[k] == b'\t') {
                        k += 1;
                    }
                }
                return k < end && self.source[k] == b'$';
            }
            return false;
        }
        // -> "label" $State — skip label, check for $
        if self.source[k] == b'"' || self.source[k] == b'\'' {
            let quote = self.source[k];
            k += 1;
            while k < end && self.source[k] != quote {
                if self.source[k] == b'\\' && k + 1 < end {
                    k += 2;
                } else {
                    k += 1;
                }
            }
            if k < end {
                k += 1;
            }
            while k < end && (self.source[k] == b' ' || self.source[k] == b'\t') {
                k += 1;
            }
            return k < end && self.source[k] == b'$';
        }
        // -> $State (simple)
        self.source[k] == b'$'
    }

    /// Lex a transition statement: -> $State, -> (args) $State, -> pop$, -> => $State
    pub(super) fn lex_sol_transition(
        &mut self,
        arrow_pos: usize,
        end: usize,
    ) -> Result<Option<Vec<Spanned>>, LexError> {
        self.cursor = arrow_pos + 2; // Skip ->
        let mut tokens = vec![Spanned {
            token: Token::Arrow,
            span: Span::new(arrow_pos, self.cursor),
        }];

        self.skip_inline_whitespace();

        // Check for -> => $State (transition forward)
        if self.cursor + 1 < end
            && self.source[self.cursor] == b'='
            && self.source[self.cursor + 1] == b'>'
        {
            let fa_start = self.cursor;
            self.cursor += 2;
            tokens.push(Spanned {
                token: Token::FatArrow,
                span: Span::new(fa_start, self.cursor),
            });
            self.skip_inline_whitespace();
        }

        // Check for enter args: (args)
        if self.cursor < end && self.source[self.cursor] == b'(' {
            if let Some(paren_end) = self
                .skipper
                .balanced_paren_end(self.source, self.cursor, end)
            {
                let args_text =
                    String::from_utf8_lossy(&self.source[self.cursor..paren_end]).to_string();
                tokens.push(Spanned {
                    token: Token::NativeCode(args_text),
                    span: Span::new(self.cursor, paren_end),
                });
                self.cursor = paren_end;
                self.skip_inline_whitespace();
            }
        }

        // Check for label: "label" before $State
        if self.cursor < end
            && (self.source[self.cursor] == b'"' || self.source[self.cursor] == b'\'')
        {
            let quote = self.source[self.cursor];
            let str_start = self.cursor;
            self.cursor += 1; // Skip opening quote
            let content_start = self.cursor;
            while self.cursor < end && self.source[self.cursor] != quote {
                if self.source[self.cursor] == b'\\' && self.cursor + 1 < end {
                    self.cursor += 2; // Skip escaped char
                } else {
                    self.cursor += 1;
                }
            }
            let content =
                String::from_utf8_lossy(&self.source[content_start..self.cursor]).to_string();
            if self.cursor < end {
                self.cursor += 1; // Skip closing quote
            }
            tokens.push(Spanned {
                token: Token::StringLit(content),
                span: Span::new(str_start, self.cursor),
            });
            self.skip_inline_whitespace();
        }

        // Check for pop$ after ->
        if self.cursor + 3 < end
            && self.source[self.cursor] == b'p'
            && self.source[self.cursor + 1] == b'o'
            && self.source[self.cursor + 2] == b'p'
            && self.source[self.cursor + 3] == b'$'
        {
            let pop_start = self.cursor;
            self.cursor += 4;
            tokens.push(Spanned {
                token: Token::PopState,
                span: Span::new(pop_start, self.cursor),
            });
        }
        // State ref: $StateName
        else if self.cursor < end && self.source[self.cursor] == b'$' {
            let sr_start = self.cursor;
            self.cursor += 1; // Skip $
            if self.cursor < end && self.source[self.cursor] == b'^' {
                // $^ parent ref
                self.cursor += 1;
                tokens.push(Spanned {
                    token: Token::ParentRef,
                    span: Span::new(sr_start, self.cursor),
                });
            } else {
                let name = self.scan_identifier();
                tokens.push(Spanned {
                    token: Token::StateRef(name),
                    span: Span::new(sr_start, self.cursor),
                });
            }

            // Check for state args: $State(args) — skip empty parens $State()
            // Emit args BEFORE the StateRef to match parser's expected pattern:
            // Arrow → NativeCode(args) → StateRef
            if self.cursor < end && self.source[self.cursor] == b'(' {
                let paren_start = self.cursor;
                if let Some(paren_end) =
                    self.skipper
                        .balanced_paren_end(self.source, self.cursor, end)
                {
                    let args_text =
                        String::from_utf8_lossy(&self.source[paren_start..paren_end]).to_string();
                    // Only emit args token if there's actual content (not just "()")
                    let inner = args_text
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .trim();
                    if !inner.is_empty() {
                        // Insert args BEFORE the StateRef token we just pushed
                        if let Some(state_ref) = tokens.pop() {
                            tokens.push(Spanned {
                                token: Token::NativeCode(args_text),
                                span: Span::new(paren_start, paren_end),
                            });
                            tokens.push(state_ref);
                        }
                    }
                    self.cursor = paren_end;
                }
            }
        }

        // Skip rest of line — comments/semicolons after Frame statements are noise
        self.skip_to_newline(end);
        Ok(Some(tokens))
    }

    /// Lex a forward statement at SOL: => $State or => $^
    pub(super) fn lex_sol_forward(
        &mut self,
        fa_pos: usize,
        end: usize,
    ) -> Result<Option<Vec<Spanned>>, LexError> {
        self.cursor = fa_pos + 2; // Skip =>
        let mut tokens = vec![Spanned {
            token: Token::FatArrow,
            span: Span::new(fa_pos, self.cursor),
        }];

        self.skip_inline_whitespace();

        if self.cursor < end && self.source[self.cursor] == b'$' {
            let sr_start = self.cursor;
            self.cursor += 1;
            if self.cursor < end && self.source[self.cursor] == b'^' {
                self.cursor += 1;
                tokens.push(Spanned {
                    token: Token::ParentRef,
                    span: Span::new(sr_start, self.cursor),
                });
            } else {
                let name = self.scan_identifier();
                tokens.push(Spanned {
                    token: Token::StateRef(name),
                    span: Span::new(sr_start, self.cursor),
                });
            }
        }

        // Skip rest of line — comments/semicolons after Frame statements are noise
        self.skip_to_newline(end);
        Ok(Some(tokens))
    }

    /// Try to lex transition with exit args: (exit_args) -> (enter_args) $State
    pub(super) fn try_lex_exit_args_transition(
        &mut self,
        paren_pos: usize,
        end: usize,
    ) -> Result<Option<Vec<Spanned>>, LexError> {
        if let Some(paren_end) = self.skipper.balanced_paren_end(self.source, paren_pos, end) {
            let mut k = paren_end;
            while k < end && (self.source[k] == b' ' || self.source[k] == b'\t') {
                k += 1;
            }
            if k + 1 < end && self.source[k] == b'-' && self.source[k + 1] == b'>' {
                // This is (exit_args) -> ...
                let exit_args =
                    String::from_utf8_lossy(&self.source[paren_pos..paren_end]).to_string();
                let mut tokens = vec![Spanned {
                    token: Token::NativeCode(exit_args),
                    span: Span::new(paren_pos, paren_end),
                }];

                let arrow_start = k;
                self.cursor = k + 2;
                tokens.push(Spanned {
                    token: Token::Arrow,
                    span: Span::new(arrow_start, self.cursor),
                });

                self.skip_inline_whitespace();

                // Optional enter args
                if self.cursor < end && self.source[self.cursor] == b'(' {
                    if let Some(pe2) =
                        self.skipper
                            .balanced_paren_end(self.source, self.cursor, end)
                    {
                        let enter_args =
                            String::from_utf8_lossy(&self.source[self.cursor..pe2]).to_string();
                        tokens.push(Spanned {
                            token: Token::NativeCode(enter_args),
                            span: Span::new(self.cursor, pe2),
                        });
                        self.cursor = pe2;
                        self.skip_inline_whitespace();
                    }
                }

                // State ref or pop$
                if self.cursor < end && self.source[self.cursor] == b'$' {
                    let sr_start = self.cursor;
                    self.cursor += 1;
                    let name = self.scan_identifier();
                    tokens.push(Spanned {
                        token: Token::StateRef(name),
                        span: Span::new(sr_start, self.cursor),
                    });
                } else if self.cursor + 3 < end
                    && self.source[self.cursor] == b'p'
                    && self.source[self.cursor + 1] == b'o'
                    && self.source[self.cursor + 2] == b'p'
                    && self.source[self.cursor + 3] == b'$'
                {
                    let pop_start = self.cursor;
                    self.cursor += 4;
                    tokens.push(Spanned {
                        token: Token::PopState,
                        span: Span::new(pop_start, self.cursor),
                    });
                }

                self.skip_to_newline(end);
                return Ok(Some(tokens));
            }
        }
        Ok(None)
    }

    // ========================================================================
    // Context Construct Lexing (Native-Aware Mode)
    // ========================================================================

    pub(super) fn lex_context_construct(&mut self, end: usize) -> Result<(), LexError> {
        let start = self.cursor;
        self.cursor += 2; // Skip "@@"

        // RFC-0013 attribute form: `@@[name]` or `@@[name(args)]`.
        // Inside a system body (e.g. before an interface method),
        // this is the C#/Java/Kotlin annotation shape. Parse the
        // bracketed name + optional args slice and emit a single
        // Token::Attribute. Module-scope `@@[persist]` etc. are
        // handled earlier by the segmenter (PragmaKind::Persist).
        if self.cursor < end && self.source[self.cursor] == b'[' {
            self.cursor += 1; // skip [
                              // Attribute name: alphanumeric / underscore / hyphen.
            let name_start = self.cursor;
            while self.cursor < end {
                let c = self.source[self.cursor];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                    self.cursor += 1;
                } else {
                    break;
                }
            }
            let name = String::from_utf8_lossy(&self.source[name_start..self.cursor]).to_string();
            // Optional args: `(...)` with paren depth tracking so
            // nested calls / list literals don't end the args early.
            let mut args: Option<String> = None;
            if self.cursor < end && self.source[self.cursor] == b'(' {
                let args_start = self.cursor + 1;
                self.cursor += 1;
                let mut depth: i32 = 1;
                while self.cursor < end && depth > 0 {
                    match self.source[self.cursor] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    self.cursor += 1;
                }
                if depth == 0 {
                    let inner = &self.source[args_start..self.cursor - 1];
                    args = Some(String::from_utf8_lossy(inner).to_string());
                }
            }
            // Expect closing `]`.
            if self.cursor < end && self.source[self.cursor] == b']' {
                self.cursor += 1;
            }
            self.emit(Token::Attribute { name, args }, start, self.cursor);
            return Ok(());
        }

        if self.cursor < end && self.source[self.cursor] == b':' {
            self.cursor += 1; // Skip ":"
            if self.cursor + 5 < end && &self.source[self.cursor..self.cursor + 6] == b"return" {
                self.cursor += 6;
                self.emit(Token::ContextReturn, start, self.cursor);
            } else if self.cursor + 4 < end
                && &self.source[self.cursor..self.cursor + 5] == b"event"
            {
                self.cursor += 5;
                self.emit(Token::ContextEvent, start, self.cursor);
            } else if self.cursor + 3 < end && &self.source[self.cursor..self.cursor + 4] == b"data"
            {
                self.cursor += 4;
                let key = self.scan_dot_key(end);
                self.emit(Token::ContextData(key), start, self.cursor);
            } else if self.cursor + 5 < end
                && &self.source[self.cursor..self.cursor + 6] == b"params"
            {
                self.cursor += 6;
                let key = self.scan_dot_key(end);
                self.emit(Token::ContextParams(key), start, self.cursor);
            } else {
                // Unknown @@: variant — emit as native
                let text = String::from_utf8_lossy(&self.source[start..self.cursor]).to_string();
                self.emit(Token::NativeCode(text), start, self.cursor);
            }
        } else {
            // Just "@@" without . or : — emit as native
            let text = String::from_utf8_lossy(&self.source[start..self.cursor]).to_string();
            self.emit(Token::NativeCode(text), start, self.cursor);
        }

        Ok(())
    }

    // ========================================================================
    // Dollar-sign Lexing (Structural Mode)
    // ========================================================================

    pub(super) fn lex_dollar(&mut self, start: usize) -> Result<(), LexError> {
        self.cursor += 1; // Skip $

        if self.cursor >= self.end {
            return Err(LexError::InvalidFrameConstruct {
                text: "$".to_string(),
                span: Span::new(start, self.cursor),
            });
        }

        let next = self.source[self.cursor];

        match next {
            // $> — enter handler
            b'>' => {
                self.cursor += 1;
                self.emit(Token::EnterHandler, start, self.cursor);
            }
            // $^ — parent ref
            b'^' => {
                self.cursor += 1;
                self.emit(Token::ParentRef, start, self.cursor);
            }
            // $.varName — state variable ref
            b'.' => {
                self.cursor += 1;
                let name = self.scan_identifier();
                self.emit(Token::StateVarRef(name), start, self.cursor);
            }
            // $StateName — state reference
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let name = self.scan_identifier();
                self.emit(Token::StateRef(name), start, self.cursor);
            }
            _ => {
                return Err(LexError::InvalidFrameConstruct {
                    text: format!("${}", next as char),
                    span: Span::new(start, self.cursor + 1),
                });
            }
        }

        Ok(())
    }
}
