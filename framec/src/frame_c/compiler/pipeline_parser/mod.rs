//! Frame Parser (Stage 2 of the V4 Pipeline)
//!
//! Recursive descent parser that consumes tokens from the Lexer and builds
//! a complete `SystemAst`. The parser controls the Lexer's mode switching:
//! - Structural mode for section headers, method signatures, state blocks
//! - Native-aware mode for handler/action/operation bodies
//!
//! After parsing, the AST contains every Frame statement and every native code
//! chunk — no further source scanning is needed.

pub mod call_args;

use crate::frame_c::compiler::frame_ast::*;
use crate::frame_c::compiler::lexer::{LexError, Lexer, Spanned, Token};
use crate::frame_c::visitors::TargetLanguage;

// ============================================================================
// Parse Error
// ============================================================================

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Parse error at {}-{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for ParseError {}

impl From<LexError> for ParseError {
    fn from(e: LexError) -> Self {
        let span = match &e {
            LexError::UnexpectedByte { span, .. } => span.clone(),
            LexError::UnterminatedString { span } => span.clone(),
            LexError::UnterminatedComment { span } => span.clone(),
            LexError::InvalidFrameConstruct { span, .. } => span.clone(),
        };
        ParseError {
            message: e.to_string(),
            span,
        }
    }
}

// ============================================================================
// Parser
// ============================================================================

pub struct Parser<'a> {
    lexer: Lexer<'a>,
}

impl<'a> Parser<'a> {
    /// Create a new Parser wrapping a Lexer.
    pub fn new(lexer: Lexer<'a>) -> Self {
        Parser { lexer }
    }

    /// Parse the system body into a SystemAst.
    /// `name` is the system name (extracted by the Segmenter).
    pub fn parse_system(&mut self, name: String) -> Result<SystemAst, ParseError> {
        let start = self.lexer.cursor();
        let mut system = SystemAst::new(name, Span::new(start, start));

        // Parse sections until Eof
        loop {
            let tok = self.peek()?;
            match tok {
                Token::Eof => break,

                Token::Interface => {
                    self.advance()?;
                    self.expect_section_colon()?;
                    system.section_order.push(SystemSectionKind::Interface);
                    let methods = self.parse_interface_methods()?;
                    system.interface = methods;
                }

                Token::Machine => {
                    self.advance()?;
                    self.expect_section_colon()?;
                    system.section_order.push(SystemSectionKind::Machine);
                    let machine = self.parse_machine()?;
                    system.machine = Some(machine);
                }

                Token::Actions => {
                    self.advance()?;
                    self.expect_section_colon()?;
                    system.section_order.push(SystemSectionKind::Actions);
                    let actions = self.parse_actions()?;
                    system.actions = actions;
                }

                Token::Operations => {
                    self.advance()?;
                    self.expect_section_colon()?;
                    system.section_order.push(SystemSectionKind::Operations);
                    let operations = self.parse_operations()?;
                    system.operations = operations;
                }

                Token::Domain => {
                    self.advance()?;
                    self.expect_section_colon()?;
                    system.section_order.push(SystemSectionKind::Domain);
                    let domain = self.parse_domain()?;
                    system.domain = domain;
                }

                _ => {
                    let spanned = self.advance()?;
                    return Err(ParseError {
                        message: format!("Expected section keyword, found {:?}", spanned.token),
                        span: spanned.span,
                    });
                }
            }
        }

        system.span = Span::new(start, self.lexer.cursor());
        Ok(system)
    }

    // ========================================================================
    // Interface Section
    // ========================================================================

    fn parse_interface_methods(&mut self) -> Result<Vec<InterfaceMethod>, ParseError> {
        let mut methods = Vec::new();

        loop {
            let tok = self.peek()?;
            match tok {
                // Next section or end of system
                Token::Machine
                | Token::Actions
                | Token::Operations
                | Token::Domain
                | Token::Eof => break,
                // Another interface keyword (duplicate section)
                Token::Interface => break,
                // Method name
                Token::Ident(_) => {
                    let method = self.parse_interface_method()?;
                    methods.push(method);
                }
                _ => {
                    let spanned = self.advance()?;
                    return Err(ParseError {
                        message: format!(
                            "Expected method name in interface, found {:?}",
                            spanned.token
                        ),
                        span: spanned.span,
                    });
                }
            }
        }

        Ok(methods)
    }

    fn parse_interface_method(&mut self) -> Result<InterfaceMethod, ParseError> {
        // Drain section-level comments captured by the lexer's
        // `skip_whitespace_and_comments` since the previous token —
        // these are the user's docstrings preceding this method
        // declaration. Codegen will emit them before the per-target
        // wrapper. Drain *before* the modifier-loop's `peek()` so we
        // don't pick up comments from a later method's preamble.
        let leading_comments = self.lexer.take_pending_comments();
        // Check for `static` and `async` modifiers (in any order)
        let mut is_static = false;
        let mut is_async = false;
        loop {
            if let Token::Ident(ref name) = self.peek()? {
                if name == "static" && !is_static {
                    is_static = true;
                    self.advance()?;
                    continue;
                }
                if name == "async" && !is_async {
                    is_async = true;
                    self.advance()?;
                    continue;
                }
            }
            break;
        }

        let name_tok = self.expect_ident()?;
        let name = name_tok.0;
        let start = name_tok.1.start;

        // Parse parameter list: (params)
        let params = if self.check(&Token::LParen)? {
            self.advance()?; // (
            let params = self.parse_method_params()?;
            self.expect_token(&Token::RParen)?; // )
            params
        } else {
            vec![]
        };

        // Optional return type: : Type
        let return_type = if self.check(&Token::Colon)? {
            self.advance()?; // :
            Some(self.parse_type()?)
        } else {
            None
        };

        // Optional return init: = expr (can be multi-token like "self.x + a")
        let return_init = if self.check(&Token::Equals)? {
            self.advance()?; // =
                             // Scan source bytes from cursor to end of line to capture the full expression
            let src = self.lexer.source();
            let init_start = self.lexer.cursor();
            let mut pos = init_start;
            while pos < src.len() && src[pos] != b'\n' && src[pos] != b'#' {
                pos += 1;
            }
            let init_text = std::str::from_utf8(&src[init_start..pos])
                .unwrap_or("")
                .trim()
                .to_string();
            // Advance lexer cursor past this expression (skip comment if present)
            if pos < src.len() && src[pos] == b'#' {
                while pos < src.len() && src[pos] != b'\n' {
                    pos += 1;
                }
            }
            self.lexer.set_cursor(pos);
            if init_text.is_empty() {
                None
            } else {
                Some(strip_context_return_wrapper(&init_text))
            }
        } else {
            None
        };

        Ok(InterfaceMethod {
            name,
            params,
            return_type,
            return_init,
            is_async,
            is_static,
            leading_comments,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_method_params(&mut self) -> Result<Vec<MethodParam>, ParseError> {
        let mut params = Vec::new();

        loop {
            if self.check(&Token::RParen)? {
                break;
            }

            let (name, span) = self.expect_ident()?;
            let param_type = if self.check(&Token::Colon)? {
                self.advance()?;
                self.parse_type()?
            } else {
                Type::Unknown
            };

            let default = if self.check(&Token::Equals)? {
                self.advance()?;
                let tok = self.advance()?;
                match tok.token {
                    Token::StringLit(s) => Some(s),
                    Token::IntLit(i) => Some(i.to_string()),
                    Token::FloatLit(f) => Some(f.to_string()),
                    Token::Ident(s) => Some(s),
                    Token::BoolLit(b) => Some(b.to_string()),
                    _ => None,
                }
            } else {
                None
            };

            params.push(MethodParam {
                name,
                param_type,
                default,
                span,
            });

            if self.check(&Token::Comma)? {
                self.advance()?; // ,
            }
        }

        Ok(params)
    }

    // ========================================================================
    // Machine Section
    // ========================================================================

    fn parse_machine(&mut self) -> Result<MachineAst, ParseError> {
        let start = self.lexer.cursor();
        let mut states = Vec::new();

        loop {
            let tok = self.peek()?;
            match tok {
                // State declaration: $StateName
                Token::StateRef(_) => {
                    let state = self.parse_state()?;
                    states.push(state);
                }
                // Next section or end
                Token::Interface
                | Token::Actions
                | Token::Operations
                | Token::Domain
                | Token::Eof => break,
                Token::Machine => break,
                _ => {
                    let spanned = self.advance()?;
                    return Err(ParseError {
                        message: format!(
                            "Expected state declaration in machine, found {:?}",
                            spanned.token
                        ),
                        span: spanned.span,
                    });
                }
            }
        }

        Ok(MachineAst {
            states,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_state(&mut self) -> Result<StateAst, ParseError> {
        // Drain section-level comments captured before the `$State`
        // token — same trivia plumbing as `parse_interface_method` /
        // `parse_action` / `parse_operation`. Codegen emits these
        // before the per-target state-dispatch function definition.
        let leading_comments = self.lexer.take_pending_comments();
        let spanned = self.advance()?;
        let state_name = match spanned.token {
            Token::StateRef(name) => name,
            _ => return Err(self.unexpected(&spanned, "state name ($StateName)")),
        };
        let start = spanned.span.start;

        // Optional parent: => $ParentName
        let parent = if self.check(&Token::FatArrow)? {
            self.advance()?; // =>
            let parent_tok = self.advance()?;
            match parent_tok.token {
                Token::StateRef(name) => Some(name),
                _ => return Err(self.unexpected(&parent_tok, "parent state ($ParentName)")),
            }
        } else {
            None
        };

        // Optional state params: (params)
        let params = if self.check(&Token::LParen)? {
            self.advance()?;
            let params = self.parse_state_params()?;
            self.expect_token(&Token::RParen)?;
            params
        } else {
            vec![]
        };

        // State body: { ... }
        let brace_tok = self.expect_token(&Token::LBrace)?;
        let body_start = brace_tok.span.start;
        let body_close = self
            .lexer
            .find_close_brace(body_start)
            .ok_or_else(|| ParseError {
                message: format!("Unmatched '{{' for state {}", state_name),
                span: brace_tok.span.clone(),
            })?;

        let mut state = StateAst::new(state_name, Span::new(start, body_close + 1));
        state.parent = parent;
        state.params = params;
        state.leading_comments = leading_comments;
        state.body_span = Span::new(body_start + 1, body_close);

        // Parse state body contents
        self.parse_state_body(&mut state, body_close)?;

        // Skip past closing brace
        self.lexer.set_cursor(body_close + 1);

        Ok(state)
    }

    fn parse_state_body(
        &mut self,
        state: &mut StateAst,
        body_close: usize,
    ) -> Result<(), ParseError> {
        loop {
            let tok = self.peek()?;
            match tok {
                Token::RBrace | Token::Eof => break,

                // State variable declaration: $.varName
                Token::StateVarRef(_) => {
                    // Check if this is a state var declaration ($.name: type = init)
                    // by looking ahead for : or =
                    let sv = self.parse_state_var_decl()?;
                    state.state_vars.push(sv);
                }

                // Enter handler: $>
                Token::EnterHandler => {
                    let handler = self.parse_enter_handler(body_close)?;
                    state.enter = Some(handler);
                }

                // Exit handler: <$
                Token::ExitHandler => {
                    let handler = self.parse_exit_handler(body_close)?;
                    state.exit = Some(handler);
                }

                // Event handler: identifier(params) { body }
                Token::Ident(_) => {
                    let handler = self.parse_event_handler(body_close)?;
                    state.handlers.push(handler);
                }

                // Default forward: => $^
                Token::FatArrow => {
                    self.advance()?;
                    if self.check(&Token::ParentRef)? {
                        self.advance()?;
                        state.default_forward = true;
                    }
                }

                _ => {
                    // Skip unknown tokens in state body
                    self.advance()?;
                }
            }
        }
        Ok(())
    }

    fn parse_state_var_decl(&mut self) -> Result<StateVarAst, ParseError> {
        let spanned = self.advance()?;
        let name = match spanned.token {
            Token::StateVarRef(n) => n,
            _ => return Err(self.unexpected(&spanned, "state variable ($.name)")),
        };
        let start = spanned.span.start;

        let var_type = if self.check(&Token::Colon)? {
            self.advance()?;
            self.parse_type()?
        } else {
            Type::Unknown
        };

        let init = if self.check(&Token::Equals)? {
            self.advance()?; // consume `=`
            Some(self.parse_simple_expression()?)
        } else {
            None
        };

        Ok(StateVarAst {
            name,
            var_type,
            init,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_state_params(&mut self) -> Result<Vec<StateParam>, ParseError> {
        let mut params = Vec::new();
        loop {
            if self.check(&Token::RParen)? {
                break;
            }
            let (name, span) = self.expect_ident()?;
            let param_type = if self.check(&Token::Colon)? {
                self.advance()?;
                self.parse_type()?
            } else {
                Type::Unknown
            };
            params.push(StateParam {
                name,
                param_type,
                span,
            });
            if self.check(&Token::Comma)? {
                self.advance()?;
            }
        }
        Ok(params)
    }

    // ========================================================================
    // Handler Parsing
    // ========================================================================

    fn parse_enter_handler(&mut self, _state_close: usize) -> Result<EnterHandler, ParseError> {
        let start_tok = self.advance()?; // $>
        let start = start_tok.span.start;

        // Optional params
        let params = if self.check(&Token::LParen)? {
            self.advance()?;
            let p = self.parse_event_params()?;
            self.expect_token(&Token::RParen)?;
            p
        } else {
            vec![]
        };

        // Body: { ... }
        let body = self.parse_body_block()?;

        Ok(EnterHandler {
            params,
            body,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_exit_handler(&mut self, _state_close: usize) -> Result<ExitHandler, ParseError> {
        let start_tok = self.advance()?; // <$
        let start = start_tok.span.start;

        // Optional params
        let params = if self.check(&Token::LParen)? {
            self.advance()?;
            let p = self.parse_event_params()?;
            self.expect_token(&Token::RParen)?;
            p
        } else {
            vec![]
        };

        // Body: { ... }
        let body = self.parse_body_block()?;

        Ok(ExitHandler {
            params,
            body,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_event_handler(&mut self, _state_close: usize) -> Result<HandlerAst, ParseError> {
        let (event_name, name_span) = self.expect_ident()?;
        let start = name_span.start;

        // Optional params
        let params = if self.check(&Token::LParen)? {
            self.advance()?;
            let p = self.parse_event_params()?;
            self.expect_token(&Token::RParen)?;
            p
        } else {
            vec![]
        };

        // Optional return type
        let return_type = if self.check(&Token::Colon)? {
            self.advance()?;
            Some(self.parse_type()?)
        } else {
            None
        };

        // Optional return init: = @@:(expr) or = expr
        let return_init = if self.check(&Token::Equals)? {
            self.advance()?; // =
                             // Scan source bytes from cursor to '{' to capture the expression
            let src = self.lexer.source();
            let init_start = self.lexer.cursor();
            let mut pos = init_start;
            while pos < src.len() && src[pos] != b'{' && src[pos] != b'\n' {
                pos += 1;
            }
            let init_text = std::str::from_utf8(&src[init_start..pos])
                .unwrap_or("")
                .trim()
                .to_string();
            // Advance lexer cursor past this expression
            self.lexer.set_cursor(pos);
            if init_text.is_empty() {
                None
            } else {
                Some(strip_context_return_wrapper(&init_text))
            }
        } else {
            None
        };

        // Body: { ... }
        let body = self.parse_body_block()?;

        Ok(HandlerAst {
            event: event_name,
            params,
            return_type,
            return_init,
            body,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_event_params(&mut self) -> Result<Vec<EventParam>, ParseError> {
        let mut params = Vec::new();
        loop {
            if self.check(&Token::RParen)? {
                break;
            }
            let (name, span) = self.expect_ident()?;
            let param_type = if self.check(&Token::Colon)? {
                self.advance()?;
                self.parse_type()?
            } else {
                Type::Unknown
            };
            // Optional default value: `= expr`
            // Scan raw bytes to end of param (stop at `,` or `)`)
            let default_value = if self.check(&Token::Equals)? {
                self.advance()?; // =
                let src = self.lexer.source();
                let init_start = self.lexer.cursor();
                let mut pos = init_start;
                let mut depth = 0i32;
                while pos < src.len() {
                    match src[pos] {
                        b'(' | b'[' | b'{' => {
                            depth += 1;
                            pos += 1;
                        }
                        b')' | b']' | b'}' => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                            pos += 1;
                        }
                        b',' if depth == 0 => break,
                        b'\n' => break,
                        _ => pos += 1,
                    }
                }
                let init_text = std::str::from_utf8(&src[init_start..pos])
                    .unwrap_or("")
                    .trim()
                    .to_string();
                self.lexer.set_cursor(pos);
                if init_text.is_empty() {
                    None
                } else {
                    Some(init_text)
                }
            } else {
                None
            };
            params.push(EventParam {
                name,
                param_type,
                default_value,
                span,
            });
            if self.check(&Token::Comma)? {
                self.advance()?;
            }
        }
        Ok(params)
    }

    // ========================================================================
    // Body Block Parsing (Mode Switching)
    // ========================================================================

    /// Parse a body block: `{ ... }`. Switches lexer to native-aware mode,
    /// collects all tokens into statements, then switches back.
    fn parse_body_block(&mut self) -> Result<HandlerBody, ParseError> {
        let brace_tok = self.expect_token(&Token::LBrace)?;
        let open_pos = brace_tok.span.start;

        let close_pos = self
            .lexer
            .find_close_brace(open_pos)
            .ok_or_else(|| ParseError {
                message: "Unmatched '{' in handler body".to_string(),
                span: brace_tok.span.clone(),
            })?;

        // body_span includes braces — codegen's splice_handler_body_from_span() expects this
        let body_span = Span::new(open_pos, close_pos + 1);

        // Switch to native-aware mode (Lexer operates INSIDE braces)
        self.lexer.enter_native_mode(close_pos);

        // Collect native tokens into statements
        let mut statements = Vec::new();
        loop {
            let tok = self.lexer.next_token().map_err(ParseError::from)?;
            match tok.token {
                Token::Eof => break,

                Token::NativeCode(code) if !code.trim().is_empty() => {
                    statements.push(Statement::NativeCode(code));
                }

                Token::Arrow => {
                    // Transition: -> [label?] $State or -> pop$ or -> => $State
                    let next = self.lexer.next_token().map_err(ParseError::from)?;
                    match next.token {
                        Token::StateRef(target) => {
                            statements.push(Statement::Transition(TransitionAst {
                                target,
                                args: vec![],
                                label: None,
                                span: Span::new(tok.span.start, next.span.end),
                                indent: 0,
                                exit_args: None,
                                enter_args: None,
                                state_args: None,
                                is_pop: false,
                                is_forward: false,
                            }));
                        }
                        Token::StringLit(label_text) => {
                            // -> "label" $State — transition with label
                            let target_tok = self.lexer.next_token().map_err(ParseError::from)?;
                            if let Token::StateRef(target) = target_tok.token {
                                statements.push(Statement::Transition(TransitionAst {
                                    target,
                                    args: vec![],
                                    label: Some(label_text),
                                    span: Span::new(tok.span.start, target_tok.span.end),
                                    indent: 0,
                                    exit_args: None,
                                    enter_args: None,
                                    state_args: None,
                                    is_pop: false,
                                    is_forward: false,
                                }));
                            }
                        }
                        Token::FatArrow => {
                            // -> => $State (transition forward)
                            let target_tok = self.lexer.next_token().map_err(ParseError::from)?;
                            if let Token::StateRef(target) = target_tok.token {
                                statements.push(Statement::Transition(TransitionAst {
                                    target,
                                    args: vec![],
                                    label: None,
                                    span: Span::new(tok.span.start, target_tok.span.end),
                                    indent: 0,
                                    exit_args: None,
                                    enter_args: None,
                                    state_args: None,
                                    is_pop: false,
                                    is_forward: true,
                                }));
                            }
                        }
                        Token::PopState => {
                            // -> pop$ is a transition, not a standalone stack pop
                            statements.push(Statement::Transition(TransitionAst {
                                target: "pop$".to_string(),
                                args: vec![],
                                label: None,
                                span: Span::new(tok.span.start, next.span.end),
                                indent: 0,
                                exit_args: None,
                                enter_args: None,
                                state_args: None,
                                is_pop: true,
                                is_forward: false,
                            }));
                        }
                        Token::NativeCode(args) => {
                            // -> (args) $State or -> (args) "label" $State
                            let after_args = self.lexer.next_token().map_err(ParseError::from)?;
                            match after_args.token {
                                Token::StateRef(target) => {
                                    // -> (args) $State — enter args, no label
                                    statements.push(Statement::Transition(TransitionAst {
                                        target,
                                        args: vec![Expression::NativeExpr(args)],
                                        label: None,
                                        span: Span::new(tok.span.start, after_args.span.end),
                                        indent: 0,
                                        exit_args: None,
                                        enter_args: None,
                                        state_args: None,
                                        is_pop: false,
                                        is_forward: false,
                                    }));
                                }
                                Token::PopState => {
                                    // -> (enter_args) pop$ — pop with fresh enter args
                                    statements.push(Statement::Transition(TransitionAst {
                                        target: "pop$".to_string(),
                                        args: vec![],
                                        label: None,
                                        span: Span::new(tok.span.start, after_args.span.end),
                                        indent: 0,
                                        exit_args: None,
                                        enter_args: Some(args),
                                        state_args: None,
                                        is_pop: true,
                                        is_forward: false,
                                    }));
                                }
                                Token::StringLit(label_text) => {
                                    // -> (args) "label" $State — enter args + label
                                    let target_tok =
                                        self.lexer.next_token().map_err(ParseError::from)?;
                                    if let Token::StateRef(target) = target_tok.token {
                                        statements.push(Statement::Transition(TransitionAst {
                                            target,
                                            args: vec![Expression::NativeExpr(args)],
                                            label: Some(label_text),
                                            span: Span::new(tok.span.start, target_tok.span.end),
                                            indent: 0,
                                            exit_args: None,
                                            enter_args: None,
                                            state_args: None,
                                            is_pop: false,
                                            is_forward: false,
                                        }));
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }

                Token::FatArrow => {
                    // Forward: => $^ or => $State
                    let next = self.lexer.next_token().map_err(ParseError::from)?;
                    match next.token {
                        Token::ParentRef | Token::StateRef(_) => {
                            let event = match &next.token {
                                Token::ParentRef => "$^".to_string(),
                                Token::StateRef(n) => n.clone(),
                                _ => unreachable!(),
                            };
                            statements.push(Statement::Forward(ForwardAst {
                                event,
                                args: vec![],
                                span: Span::new(tok.span.start, next.span.end),
                                indent: 0,
                            }));
                        }
                        _ => {}
                    }
                }

                Token::PushState => {
                    statements.push(Statement::StackPush(StackPushAst {
                        span: tok.span,
                        indent: 0,
                    }));
                }

                Token::PopState => {
                    statements.push(Statement::StackPop(StackPopAst {
                        span: tok.span,
                        indent: 0,
                    }));
                }

                Token::Return => {
                    // return <expr>
                    let next = self.lexer.next_token().map_err(ParseError::from)?;
                    let value = match next.token {
                        Token::NativeCode(code) => {
                            Some(Expression::NativeExpr(code.trim().to_string()))
                        }
                        Token::Eof => None,
                        _ => None,
                    };
                    statements.push(Statement::Return(ReturnAst {
                        value,
                        span: tok.span,
                    }));
                }

                Token::StateVarRef(name) => {
                    // State variable reference (mid-line in native code)
                    // The codegen will handle this via NativeCode chunks
                    // For now, store as NativeCode with the Frame syntax
                    statements.push(Statement::NativeCode(format!("$.{}", name)));
                }

                Token::ContextReturn => {
                    statements.push(Statement::NativeCode("@@:return".to_string()));
                }

                Token::ContextEvent => {
                    statements.push(Statement::NativeCode("@@:event".to_string()));
                }

                Token::ContextData(key) => {
                    statements.push(Statement::NativeCode(format!("@@:data[{}]", key)));
                }

                Token::ContextParams(key) => {
                    statements.push(Statement::NativeCode(format!("@@:params[{}]", key)));
                }

                _ => {
                    // Unknown token in native mode — skip
                }
            }
        }

        // Switch back to structural mode and skip past closing brace
        self.lexer.enter_structural_mode();
        self.lexer.set_cursor(close_pos + 1);

        Ok(HandlerBody {
            statements,
            span: body_span,
        })
    }

    // ========================================================================
    // Actions Section
    // ========================================================================

    fn parse_actions(&mut self) -> Result<Vec<ActionAst>, ParseError> {
        let mut actions = Vec::new();
        loop {
            let tok = self.peek()?;
            match tok {
                Token::Ident(_) => {
                    let action = self.parse_action()?;
                    actions.push(action);
                }
                Token::Interface
                | Token::Machine
                | Token::Operations
                | Token::Domain
                | Token::Eof
                | Token::Actions => break,
                _ => {
                    self.advance()?; // skip
                }
            }
        }
        Ok(actions)
    }

    fn parse_action(&mut self) -> Result<ActionAst, ParseError> {
        // Drain section-level comments captured since the last
        // significant token — same trivia plumbing as
        // `parse_interface_method` / `parse_operation`. The captured
        // text is the user's docstring preceding this action.
        let leading_comments = self.lexer.take_pending_comments();
        // Check for `static` and `async` modifiers (in any order)
        let mut is_static = false;
        let mut is_async = false;
        loop {
            if let Token::Ident(ref name) = self.peek()? {
                if name == "static" && !is_static {
                    is_static = true;
                    self.advance()?;
                    continue;
                }
                if name == "async" && !is_async {
                    is_async = true;
                    self.advance()?;
                    continue;
                }
            }
            break;
        }

        let (name, name_span) = self.expect_ident()?;
        let start = name_span.start;

        let params = if self.check(&Token::LParen)? {
            self.advance()?;
            let p = self.parse_action_params()?;
            self.expect_token(&Token::RParen)?;
            p
        } else {
            vec![]
        };

        // Optional return type: : Type
        let return_type = if self.check(&Token::Colon)? {
            self.advance()?;
            self.parse_type()?
        } else {
            Type::Unknown
        };

        let body = self.parse_body_block()?;

        // E401: Validate no forbidden Frame syntax in action body
        self.validate_no_forbidden_frame_syntax(&body.statements, "action", &name)?;

        let code = self.extract_span_content(&body.span);

        Ok(ActionAst {
            name,
            params,
            return_type,
            body: ActionBody {
                span: body.span,
                code: Some(code),
            },
            is_async,
            is_static,
            leading_comments,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_action_params(&mut self) -> Result<Vec<ActionParam>, ParseError> {
        let mut params = Vec::new();
        loop {
            if self.check(&Token::RParen)? {
                break;
            }
            let (name, span) = self.expect_ident()?;
            let param_type = if self.check(&Token::Colon)? {
                self.advance()?;
                self.parse_type()?
            } else {
                Type::Unknown
            };
            params.push(ActionParam {
                name,
                param_type,
                default: None,
                span,
            });
            if self.check(&Token::Comma)? {
                self.advance()?;
            }
        }
        Ok(params)
    }

    // ========================================================================
    // Operations Section
    // ========================================================================

    fn parse_operations(&mut self) -> Result<Vec<OperationAst>, ParseError> {
        let mut ops = Vec::new();
        loop {
            let tok = self.peek()?;
            match tok {
                Token::Ident(_) => {
                    let op = self.parse_operation()?;
                    ops.push(op);
                }
                Token::Interface
                | Token::Machine
                | Token::Actions
                | Token::Domain
                | Token::Eof
                | Token::Operations => break,
                _ => {
                    self.advance()?;
                }
            }
        }
        Ok(ops)
    }

    fn parse_operation(&mut self) -> Result<OperationAst, ParseError> {
        // Drain section-level comments captured since the last
        // significant token — same trivia plumbing as
        // `parse_interface_method` / `parse_action`.
        let leading_comments = self.lexer.take_pending_comments();
        // Check for `static` and `async` modifiers (in any order)
        let mut is_static = false;
        let mut is_async = false;
        loop {
            if let Token::Ident(ref name) = self.peek()? {
                if name == "static" && !is_static {
                    is_static = true;
                    self.advance()?;
                    continue;
                }
                if name == "async" && !is_async {
                    is_async = true;
                    self.advance()?;
                    continue;
                }
            }
            break;
        }

        let (name, name_span) = self.expect_ident()?;
        let start = name_span.start;

        let params = if self.check(&Token::LParen)? {
            self.advance()?;
            let p = self.parse_operation_params()?;
            self.expect_token(&Token::RParen)?;
            p
        } else {
            vec![]
        };

        // Return type: : Type
        let return_type = if self.check(&Token::Colon)? {
            self.advance()?;
            self.parse_type()?
        } else {
            Type::Unknown
        };

        let body = self.parse_body_block()?;

        // E401: Validate no forbidden Frame syntax in operation body
        self.validate_no_forbidden_frame_syntax(&body.statements, "operation", &name)?;

        let code = self.extract_span_content(&body.span);

        Ok(OperationAst {
            name,
            params,
            return_type,
            body: OperationBody {
                span: body.span,
                code: Some(code),
            },
            is_static,
            is_async,
            leading_comments,
            span: Span::new(start, self.lexer.cursor()),
        })
    }

    fn parse_operation_params(&mut self) -> Result<Vec<OperationParam>, ParseError> {
        let mut params = Vec::new();
        loop {
            if self.check(&Token::RParen)? {
                break;
            }
            let (name, span) = self.expect_ident()?;
            let param_type = if self.check(&Token::Colon)? {
                self.advance()?;
                self.parse_type()?
            } else {
                Type::Unknown
            };
            params.push(OperationParam {
                name,
                param_type,
                default: None,
                span,
            });
            if self.check(&Token::Comma)? {
                self.advance()?;
            }
        }
        Ok(params)
    }

    // ========================================================================
    // Domain Section
    // ========================================================================

    /// Parse the domain section.
    ///
    /// Each field uses canonical Frame syntax: `name [: type] = init`.
    /// Type and init are opaque strings — Frame doesn't interpret them.
    /// Multi-line init via `= ( ... )` wrapper is supported.
    fn parse_domain(&mut self) -> Result<Vec<DomainVar>, ParseError> {
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

    // ========================================================================
    // Type Parsing
    // ========================================================================

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        // Scan raw source bytes for the type expression, since native types can be
        // arbitrarily complex: Vec<String>, HashMap<K,V>, string[], &str, *int, etc.
        let src = self.lexer.source();
        let start = self.lexer.cursor();
        let mut pos = start;
        let mut angle_depth = 0;
        let mut bracket_depth = 0;

        // Skip leading whitespace
        while pos < src.len() && (src[pos] == b' ' || src[pos] == b'\t') {
            pos += 1;
        }
        let type_start = pos;

        while pos < src.len() {
            let b = src[pos];
            match b {
                b'<' => {
                    angle_depth += 1;
                    pos += 1;
                }
                b'>' => {
                    angle_depth -= 1;
                    pos += 1;
                }
                b'[' => {
                    bracket_depth += 1;
                    pos += 1;
                }
                b']' => {
                    bracket_depth -= 1;
                    pos += 1;
                }
                // Stop at delimiters (only when not inside <> or [])
                b'\n' | b'{' if angle_depth == 0 && bracket_depth == 0 => break,
                b'=' | b')' | b',' if angle_depth == 0 && bracket_depth == 0 => break,
                // Stop at comment start — # begins a line comment in most languages
                b'#' if angle_depth == 0 && bracket_depth == 0 => break,
                // Type-valid characters: letters, digits, _, &, *, |, space, :, .
                _ => pos += 1,
            }
        }

        let type_text = std::str::from_utf8(&src[type_start..pos])
            .unwrap_or("")
            .trim()
            .to_string();
        self.lexer.set_cursor(pos);

        if type_text.is_empty() {
            return Ok(Type::Unknown);
        }

        // Frame has no type system — pass all types through verbatim
        Ok(Type::Custom(type_text))
    }

    // ========================================================================
    // Expression Parsing (simplified)
    // ========================================================================

    fn parse_simple_expression(&mut self) -> Result<Expression, ParseError> {
        let tok = self.advance()?;
        match tok.token {
            Token::IntLit(i) => Ok(Expression::Literal(Literal::Int(i))),
            Token::FloatLit(f) => Ok(Expression::Literal(Literal::Float(f))),
            Token::StringLit(s) => Ok(Expression::Literal(Literal::String(s))),
            Token::BoolLit(b) => Ok(Expression::Literal(Literal::Bool(b))),
            Token::Ident(name) => {
                match name.as_str() {
                    "None" | "null" | "nullptr" | "nil" => Ok(Expression::NativeExpr(name)),
                    _ => {
                        // Check for path expression like Vec::new() or String::new()
                        // :: is lexed as two Colon tokens
                        if self.check(&Token::Colon)? {
                            self.advance()?; // consume first :
                            if self.check(&Token::Colon)? {
                                self.advance()?; // consume second :
                                let mut path = name.clone();
                                path.push_str("::");
                                // Consume the method/type name
                                if let Token::Ident(_) = self.peek()? {
                                    let next = self.advance()?;
                                    if let Token::Ident(method) = next.token {
                                        path.push_str(&method);
                                    }
                                }
                                // Consume () if present
                                if self.check(&Token::LParen)? {
                                    self.advance()?; // (
                                    path.push('(');
                                    if self.check(&Token::RParen)? {
                                        self.advance()?; // )
                                        path.push(')');
                                    }
                                }
                                return Ok(Expression::NativeExpr(path));
                            }
                            // Single colon — this is a type annotation, not ::
                            // We already consumed one colon. This shouldn't happen
                            // for initializer expressions, but handle gracefully.
                        }
                        Ok(Expression::Var(name))
                    }
                }
            }
            Token::LBracket => {
                // Collect everything until matching RBracket
                let mut content = String::from("[");
                let mut depth = 1;
                while depth > 0 {
                    let next = self.advance()?;
                    match &next.token {
                        Token::LBracket => {
                            depth += 1;
                            content.push('[');
                        }
                        Token::RBracket => {
                            depth -= 1;
                            if depth > 0 {
                                content.push(']');
                            }
                        }
                        Token::Eof => break,
                        _ => {
                            let src = self.lexer.source();
                            let s = next.span.start.min(src.len());
                            let e = next.span.end.min(src.len());
                            content.push_str(std::str::from_utf8(&src[s..e]).unwrap_or(""));
                        }
                    }
                }
                content.push(']');
                Ok(Expression::NativeExpr(content))
            }
            Token::LBrace => {
                // Collect everything until matching RBrace (for dict literals etc.)
                let mut content = String::from("{");
                let mut depth = 1;
                while depth > 0 {
                    let next = self.advance()?;
                    match &next.token {
                        Token::LBrace => {
                            depth += 1;
                            content.push('{');
                        }
                        Token::RBrace => {
                            depth -= 1;
                            if depth > 0 {
                                content.push('}');
                            }
                        }
                        Token::Eof => break,
                        _ => {
                            let src = self.lexer.source();
                            let s = next.span.start.min(src.len());
                            let e = next.span.end.min(src.len());
                            content.push_str(std::str::from_utf8(&src[s..e]).unwrap_or(""));
                        }
                    }
                }
                content.push('}');
                Ok(Expression::NativeExpr(content))
            }
            _ => {
                // Fallback: extract the actual source text for this token
                let src = self.lexer.source();
                let s = tok.span.start.min(src.len());
                let e = tok.span.end.min(src.len());
                let text = std::str::from_utf8(&src[s..e]).unwrap_or("?");
                Ok(Expression::NativeExpr(text.to_string()))
            }
        }
    }

    // ========================================================================
    // Token Helpers
    // ========================================================================

    fn peek(&mut self) -> Result<&Token, ParseError> {
        self.lexer.peek().map_err(ParseError::from)
    }

    fn advance(&mut self) -> Result<Spanned, ParseError> {
        self.lexer.next_token().map_err(ParseError::from)
    }

    fn check(&mut self, expected: &Token) -> Result<bool, ParseError> {
        let tok = self.peek()?;
        Ok(std::mem::discriminant(tok) == std::mem::discriminant(expected))
    }

    fn expect_token(&mut self, expected: &Token) -> Result<Spanned, ParseError> {
        let tok = self.advance()?;
        if std::mem::discriminant(&tok.token) == std::mem::discriminant(expected) {
            Ok(tok)
        } else {
            Err(ParseError {
                message: format!("Expected {:?}, found {:?}", expected, tok.token),
                span: tok.span,
            })
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span), ParseError> {
        let tok = self.advance()?;
        match tok.token {
            Token::Ident(name) => Ok((name, tok.span)),
            _ => Err(self.unexpected(&tok, "identifier")),
        }
    }

    fn expect_section_colon(&mut self) -> Result<(), ParseError> {
        let tok = self.advance()?;
        match tok.token {
            Token::SectionColon => Ok(()),
            Token::Colon => Ok(()), // Accept regular colon too
            _ => Err(self.unexpected(&tok, "':'")),
        }
    }

    fn unexpected(&self, tok: &Spanned, expected: &str) -> ParseError {
        ParseError {
            message: format!("Expected {}, found {:?}", expected, tok.token),
            span: tok.span.clone(),
        }
    }

    /// Extract content from a body span (between braces, trimmed).
    fn extract_span_content(&self, span: &Span) -> String {
        let source = self.lexer.source();
        let end = span.end.min(source.len());
        let start = span.start.min(end);
        let text = std::str::from_utf8(&source[start..end]).unwrap_or("");
        text.trim_matches('\n').to_string()
    }

    /// E401: Validate that action/operation bodies don't contain forbidden Frame syntax.
    ///
    /// Forbidden in actions/operations:
    /// - `-> $State` (transitions)
    /// - `-> => $State` (transition with forwarding)
    /// - `-> pop$` (pop transition)
    /// - `=> $^` (dispatch to parent)
    /// - `push$` / `pop$` (state stack operations)
    /// - `$.varName` (state variable access)
    ///
    /// Allowed:
    /// - `@@.param`, `@@:return`, `@@:event`, `@@:data[key]`, `@@:params[key]` (context access)
    /// - `return` (native return statement, not Frame sugar)
    fn validate_no_forbidden_frame_syntax(
        &self,
        statements: &[Statement],
        context_kind: &str, // "action" or "operation"
        context_name: &str, // the action/operation name
    ) -> Result<(), ParseError> {
        for stmt in statements {
            match stmt {
                Statement::Transition(t) => {
                    return Err(ParseError {
                        message: format!(
                            "E401: Transition '-> ${}' is not allowed in {} '{}'. \
                             Transitions are only allowed in event handlers.",
                            t.target, context_kind, context_name
                        ),
                        span: t.span.clone(),
                    });
                }
                Statement::Forward(f) => {
                    return Err(ParseError {
                        message: format!(
                            "E401: Dispatch '=> {}' is not allowed in {} '{}'. \
                             Dispatch is only allowed in event handlers.",
                            f.event, context_kind, context_name
                        ),
                        span: f.span.clone(),
                    });
                }
                Statement::StackPush(s) => {
                    return Err(ParseError {
                        message: format!(
                            "E401: 'push$' is not allowed in {} '{}'. \
                             State stack operations are only allowed in event handlers.",
                            context_kind, context_name
                        ),
                        span: s.span.clone(),
                    });
                }
                Statement::StackPop(s) => {
                    return Err(ParseError {
                        message: format!(
                            "E401: 'pop$' is not allowed in {} '{}'. \
                             State stack operations are only allowed in event handlers.",
                            context_kind, context_name
                        ),
                        span: s.span.clone(),
                    });
                }
                Statement::NativeCode(code) if code.starts_with("$.") => {
                    return Err(ParseError {
                        message: format!(
                            "E401: State variable access '{}' is not allowed in {} '{}'. \
                             State variables are only accessible in event handlers.",
                            code, context_kind, context_name
                        ),
                        span: Span::new(0, 0), // No precise span available for NativeCode
                    });
                }
                // Allowed: Return (native in actions/operations), NativeCode, @@:* context access
                _ => {}
            }
        }
        Ok(())
    }
}

// ============================================================================
// Convenience Function
// ============================================================================

/// Parse a system body from source bytes.
/// `name` is the system name, `body_span` is the span inside the system braces.
/// Strip @@:() wrapper from return init expression.
/// "@@:(42)" → "42", "@@:(foo(bar(1)))" → "foo(bar(1))", "42" → "42"
fn strip_context_return_wrapper(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("@@:(") && trimmed.ends_with(')') {
        trimmed[4..trimmed.len() - 1].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn parse_system(
    source: &[u8],
    name: String,
    body_span: Span,
    lang: TargetLanguage,
) -> Result<SystemAst, ParseError> {
    let lexer = Lexer::new(source, body_span, lang);
    let mut parser = Parser::new(lexer);
    parser.parse_system(name)
}

/// Parse a system's header parameter list — the contents between `(` and `)`
/// in `@@system Name(...) { ... }`.
///
/// The header parameter list is captured by the segmenter as a span pointing
/// at the bytes between the parens (exclusive). It's NOT part of the system
/// body, so it doesn't go through the body lexer. Parameters can be:
///
///   - `name`                       — untyped domain param
///   - `name: type`                 — typed domain param
///   - `name: type = default`       — typed domain param with default
///   - `$(name)`                    — untyped start state param
///   - `$(name): type`              — typed start state param
///   - `$(name): type = default`    — typed start state param with default
///
/// Multiple params are comma-separated. Whitespace around tokens is ignored.
/// `$>(name)` (start enter param) is NOT yet supported and produces an error
/// directing the user to file a follow-up.
pub fn parse_system_header_params(
    source: &[u8],
    span: Span,
) -> Result<Vec<SystemParam>, ParseError> {
    let mut params = Vec::new();
    let mut i = span.start;
    let end = span.end;

    let is_ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let is_ident_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

    let skip_ws = |i: &mut usize| {
        while *i < end && (source[*i] == b' ' || source[*i] == b'\t') {
            *i += 1;
        }
    };

    loop {
        skip_ws(&mut i);
        if i >= end {
            break;
        }

        // Each iteration parses one parameter.
        let param_start = i;

        // Detect `$(...)` (state arg), `$>(...)` (enter arg), or a bare
        // identifier (domain). The sigils delimit a typed param body
        // (`name: type [= default]`) inside their parens. Bare domain
        // params use the same body shape directly at the top level.
        //
        // Both `$(...)` and `$>(...)` are valid in the system header context;
        // the parser is at the top level inside `(` ... `)` and never inside a
        // state body, so neither sigil clashes with state-level enter handlers.
        let kind: ParamKind;
        let name;
        let param_type;
        let default;
        if source[i] == b'$' {
            // Sigil branch: `$(name: type [= default])` or `$>(...)`.
            let is_enter = i + 1 < end && source[i + 1] == b'>';
            let sigil_open_pos = if is_enter { i + 2 } else { i + 1 };
            if sigil_open_pos >= end || source[sigil_open_pos] != b'(' {
                return Err(ParseError {
                    message: format!(
                        "expected '(' after '{}' in system header param",
                        if is_enter { "$>" } else { "$" }
                    ),
                    span: Span::new(i, sigil_open_pos),
                });
            }
            i = sigil_open_pos + 1; // past `$(` or `$>(`

            // Parse the typed param body INSIDE the parens. The body
            // shape is `name: type [= default]` and ends at the matching
            // close paren `)`.
            let body = parse_typed_param_body(source, &mut i, end, /*close_at_paren=*/ true)?;
            name = body.0;
            param_type = body.1;
            default = body.2;

            // Expect closing `)`
            skip_ws(&mut i);
            if i >= end || source[i] != b')' {
                return Err(ParseError {
                    message: format!(
                        "expected ')' to close '{}(' param body",
                        if is_enter { "$>" } else { "$" }
                    ),
                    span: Span::new(i, i.saturating_add(1)),
                });
            }
            i += 1; // past `)`
            kind = if is_enter {
                ParamKind::EnterArg
            } else {
                ParamKind::StateArg
            };
        } else if is_ident_start(source[i]) {
            // Bare domain branch: `name: type [= default]` directly at
            // the top level of the system header param list.
            let body = parse_typed_param_body(source, &mut i, end, /*close_at_paren=*/ false)?;
            name = body.0;
            param_type = body.1;
            default = body.2;
            kind = ParamKind::Domain;
        } else {
            return Err(ParseError {
                message: format!(
                    "unexpected character '{}' in system header parameter list",
                    source[i] as char
                ),
                span: Span::new(i, i + 1),
            });
        }

        params.push(SystemParam {
            name,
            param_type,
            default,
            kind,
            span: Span::new(param_start, i),
        });

        skip_ws(&mut i);
        if i < end && source[i] == b',' {
            i += 1;
            continue;
        }
        if i >= end {
            break;
        }
        // For bare domain params, anything other than `,` or end is
        // an error. Sigils should already have consumed their `)`.
        return Err(ParseError {
            message: format!(
                "expected ',' or end of header param list, got '{}'",
                source[i] as char
            ),
            span: Span::new(i, i + 1),
        });
    }

    // Enforce canonical ordering: $(state) before $>(enter) before domain.
    // Track the highest group seen so far (0=state, 1=enter, 2=domain).
    let mut max_group = 0u8;
    for p in &params {
        let group = match p.kind {
            ParamKind::StateArg => 0,
            ParamKind::EnterArg => 1,
            ParamKind::Domain => 2,
        };
        if group < max_group {
            let expected = match max_group {
                1 => "enter params ($>()) must come after state params ($())",
                2 => "domain params must come last, after state ($()) and enter ($>()) params",
                _ => "parameters are out of order",
            };
            return Err(ParseError {
                message: format!(
                    "system header parameter '{}' is out of order: {}",
                    p.name, expected
                ),
                span: p.span.clone(),
            });
        }
        max_group = group;
    }

    Ok(params)
}

/// Parse a typed parameter body of shape `name: type [= default]`.
///
/// Used by `parse_system_header_params` for both sigil-wrapped params
/// (`$(name: type)`, `$>(name: type)`) and bare domain params. The
/// `close_at_paren` flag tells the parser whether the body terminates
/// at a `)` (sigil context) or at `,` / end-of-input (bare context).
///
/// Returns `(name, var_type, default)`. The cursor is left positioned
/// at the terminator (the `)` for sigil context, the `,` or end for
/// bare context). The caller is responsible for consuming the terminator.
fn parse_typed_param_body(
    source: &[u8],
    cursor: &mut usize,
    end: usize,
    close_at_paren: bool,
) -> Result<(String, Type, Option<String>), ParseError> {
    let is_ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let is_ident_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let skip_ws = |i: &mut usize| {
        while *i < end && (source[*i] == b' ' || source[*i] == b'\t') {
            *i += 1;
        }
    };

    // Parse the name (identifier)
    skip_ws(cursor);
    let name_start = *cursor;
    while *cursor < end && is_ident_cont(source[*cursor]) {
        *cursor += 1;
    }
    if name_start == *cursor || !is_ident_start(source[name_start]) {
        return Err(ParseError {
            message: "expected identifier as parameter name".to_string(),
            span: Span::new(name_start, *cursor),
        });
    }
    let name = std::str::from_utf8(&source[name_start..*cursor])
        .unwrap_or("")
        .to_string();

    // Optional `: type`
    skip_ws(cursor);
    let param_type = if *cursor < end && source[*cursor] == b':' {
        *cursor += 1; // past `:`
        skip_ws(cursor);
        let type_start = *cursor;
        // Type runs until terminator or `=`. Whitespace is part of the
        // type only inside angle brackets / parens / brackets
        // (e.g. `Map<str, int>`).
        let mut depth: i32 = 0;
        while *cursor < end {
            let b = source[*cursor];
            if depth == 0 && b == b'=' {
                break;
            }
            if depth == 0 && !close_at_paren && b == b',' {
                break;
            }
            if b == b'<' || b == b'(' || b == b'[' {
                depth += 1;
            } else if b == b'>' || b == b')' || b == b']' {
                if depth == 0 {
                    if close_at_paren && b == b')' {
                        break;
                    }
                    if !close_at_paren && b == b')' {
                        break;
                    }
                    break;
                }
                depth -= 1;
            }
            *cursor += 1;
        }
        let type_text = std::str::from_utf8(&source[type_start..*cursor])
            .unwrap_or("")
            .trim()
            .to_string();
        if type_text.is_empty() {
            return Err(ParseError {
                message: "expected type after ':'".to_string(),
                span: Span::new(type_start, *cursor),
            });
        }
        Type::Custom(type_text)
    } else {
        Type::Unknown
    };

    // Optional `= default`
    skip_ws(cursor);
    let default = if *cursor < end && source[*cursor] == b'=' {
        *cursor += 1; // past `=`
        skip_ws(cursor);
        let def_start = *cursor;
        // Default runs until terminator. Bracket depth is tracked so
        // expressions like `make_default(1, 2)` or `[1, 2, 3]` survive.
        let mut depth: i32 = 0;
        while *cursor < end {
            let b = source[*cursor];
            if depth == 0 {
                if close_at_paren && b == b')' {
                    break;
                }
                if !close_at_paren && b == b',' {
                    break;
                }
            }
            if b == b'(' || b == b'[' || b == b'{' {
                depth += 1;
            } else if b == b')' || b == b']' || b == b'}' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            *cursor += 1;
        }
        let def_text = std::str::from_utf8(&source[def_start..*cursor])
            .unwrap_or("")
            .trim()
            .to_string();
        if def_text.is_empty() {
            None
        } else {
            Some(def_text)
        }
    } else {
        None
    };

    Ok((name, param_type, default))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_py(src: &str) -> SystemAst {
        let bytes = src.as_bytes();
        let span = Span::new(0, bytes.len());
        parse_system(
            bytes,
            "TestSystem".to_string(),
            span,
            TargetLanguage::Python3,
        )
        .expect("Parse failed")
    }

    #[test]
    fn test_empty_system() {
        let sys = parse_py("");
        assert_eq!(sys.name, "TestSystem");
        assert!(sys.interface.is_empty());
        assert!(sys.machine.is_none());
        assert!(sys.actions.is_empty());
        assert!(sys.domain.is_empty());
    }

    #[test]
    fn test_interface_simple() {
        let sys = parse_py("interface:\n    start()\n    stop()");
        assert_eq!(sys.interface.len(), 2);
        assert_eq!(sys.interface[0].name, "start");
        assert_eq!(sys.interface[1].name, "stop");
    }

    #[test]
    fn test_interface_with_params() {
        let sys = parse_py("interface:\n    send(msg: str, count: int)");
        assert_eq!(sys.interface.len(), 1);
        let m = &sys.interface[0];
        assert_eq!(m.name, "send");
        assert_eq!(m.params.len(), 2);
        assert_eq!(m.params[0].name, "msg");
        assert_eq!(m.params[0].param_type, Type::Custom("str".into()));
        assert_eq!(m.params[1].name, "count");
        assert_eq!(m.params[1].param_type, Type::Custom("int".into()));
    }

    #[test]
    fn test_interface_with_return_type() {
        let sys = parse_py("interface:\n    getData(): int");
        assert_eq!(
            sys.interface[0].return_type,
            Some(Type::Custom("int".into()))
        );
    }

    #[test]
    fn test_interface_with_alias() {
        let sys = parse_py(
            r#"interface:
    foo(a: int): str = "myFoo""#,
        );
        assert_eq!(sys.interface[0].name, "foo");
        assert_eq!(sys.interface[0].return_init, Some("\"myFoo\"".to_string()));
    }

    #[test]
    fn test_machine_simple_state() {
        let sys = parse_py("machine:\n    $Idle {\n    }");
        let machine = sys.machine.unwrap();
        assert_eq!(machine.states.len(), 1);
        assert_eq!(machine.states[0].name, "Idle");
    }

    #[test]
    fn test_machine_state_with_handler() {
        let sys = parse_py(
            "machine:\n    $Idle {\n        start() {\n            -> $Running\n        }\n    }",
        );
        let machine = sys.machine.unwrap();
        assert_eq!(machine.states[0].name, "Idle");
        assert_eq!(machine.states[0].handlers.len(), 1);
        assert_eq!(machine.states[0].handlers[0].event, "start");

        // Check handler body has a transition
        let body = &machine.states[0].handlers[0].body;
        let has_transition = body
            .statements
            .iter()
            .any(|s| matches!(s, Statement::Transition(t) if t.target == "Running"));
        assert!(
            has_transition,
            "Handler body should contain transition to $Running"
        );
    }

    #[test]
    fn test_machine_enter_handler() {
        let sys =
            parse_py("machine:\n    $Init {\n        $>() {\n            x = 1\n        }\n    }");
        let machine = sys.machine.unwrap();
        let state = &machine.states[0];
        assert!(state.enter.is_some());
        let enter = state.enter.as_ref().unwrap();
        // Body should contain native code
        assert!(!enter.body.statements.is_empty());
    }

    #[test]
    fn test_machine_exit_handler() {
        let sys = parse_py(
            "machine:\n    $Init {\n        <$() {\n            cleanup()\n        }\n    }",
        );
        let machine = sys.machine.unwrap();
        let state = &machine.states[0];
        assert!(state.exit.is_some());
    }

    #[test]
    fn test_domain_simple() {
        let sys = parse_py("domain:\n    x = 0\n    name = \"hello\"");
        assert_eq!(sys.domain.len(), 2);
        assert_eq!(sys.domain[0].name, "x");
        assert_eq!(sys.domain[0].initializer_text, Some("0".to_string()));
        assert_eq!(sys.domain[1].name, "name");
        assert_eq!(
            sys.domain[1].initializer_text,
            Some("\"hello\"".to_string())
        );
    }

    #[test]
    fn test_domain_with_types() {
        let sys = parse_py("domain:\n    count: int = 0");
        assert_eq!(sys.domain[0].name, "count");
        assert_eq!(sys.domain[0].initializer_text, Some("0".to_string()));
        assert!(
            matches!(sys.domain[0].var_type, crate::frame_c::compiler::frame_ast::Type::Custom(ref s) if s == "int")
        );
    }

    #[test]
    fn test_multiple_sections() {
        let sys = parse_py(
            "interface:\n    start()\n\nmachine:\n    $Idle {\n    }\n\ndomain:\n    x = 0",
        );
        assert_eq!(sys.interface.len(), 1);
        assert!(sys.machine.is_some());
        assert_eq!(sys.domain.len(), 1);
    }

    #[test]
    fn test_section_order() {
        let sys =
            parse_py("interface:\n    start()\nmachine:\n    $A {\n    }\ndomain:\n    x = 0");
        assert_eq!(
            sys.section_order,
            vec![
                SystemSectionKind::Interface,
                SystemSectionKind::Machine,
                SystemSectionKind::Domain,
            ]
        );
    }

    #[test]
    fn test_handler_with_native_and_transition() {
        let sys = parse_py(
            "machine:\n    $A {\n        go() {\n            x = 1\n            -> $B\n            y = 2\n        }\n    }\n    $B {\n    }"
        );
        let machine = sys.machine.unwrap();
        let handler = &machine.states[0].handlers[0];
        let stmts = &handler.body.statements;

        // Should have: NativeCode("x = 1"), Transition(B), NativeCode("y = 2")
        let has_native = stmts.iter().any(|s| matches!(s, Statement::NativeCode(_)));
        let has_transition = stmts
            .iter()
            .any(|s| matches!(s, Statement::Transition(t) if t.target == "B"));
        assert!(has_native, "Should have native code");
        assert!(has_transition, "Should have transition to $B");
    }

    #[test]
    fn test_handler_push_pop() {
        let sys = parse_py(
            "machine:\n    $A {\n        go() {\n            push$\n            -> $B\n        }\n    }\n    $B {\n        back() {\n            -> pop$\n        }\n    }"
        );
        let machine = sys.machine.unwrap();

        // First handler: push$ then transition
        let stmts_a = &machine.states[0].handlers[0].body.statements;
        let has_push = stmts_a.iter().any(|s| matches!(s, Statement::StackPush(_)));
        assert!(has_push, "Should have push$");

        // Second handler: -> pop$ is now Transition(is_pop: true)
        let stmts_b = &machine.states[1].handlers[0].body.statements;
        let has_pop_transition = stmts_b
            .iter()
            .any(|s| matches!(s, Statement::Transition(t) if t.is_pop));
        assert!(
            has_pop_transition,
            "-> pop$ should produce Transition(is_pop: true)"
        );
    }

    #[test]
    fn test_actions_section() {
        let sys = parse_py("actions:\n    doThing() {\n        print(1)\n    }");
        assert_eq!(sys.actions.len(), 1);
        assert_eq!(sys.actions[0].name, "doThing");
    }

    #[test]
    fn test_operations_section() {
        let sys = parse_py("operations:\n    getValue(): int {\n        return 42\n    }");
        assert_eq!(sys.operations.len(), 1);
        assert_eq!(sys.operations[0].name, "getValue");
        assert_eq!(sys.operations[0].return_type, Type::Custom("int".into()));
    }

    #[test]
    fn test_state_with_parent() {
        let sys = parse_py("machine:\n    $Child => $Parent {\n    }");
        let machine = sys.machine.unwrap();
        assert_eq!(machine.states[0].parent, Some("Parent".to_string()));
    }

    #[test]
    fn test_state_with_params() {
        let sys = parse_py("machine:\n    $Active(x: int, y: str) {\n    }");
        let machine = sys.machine.unwrap();
        assert_eq!(machine.states[0].params.len(), 2);
        assert_eq!(machine.states[0].params[0].name, "x");
    }

    #[test]
    fn test_handler_return_sugar() {
        let sys = parse_py(
            "machine:\n    $A {\n        get() {\n            return 42\n        }\n    }",
        );
        let machine = sys.machine.unwrap();
        let stmts = &machine.states[0].handlers[0].body.statements;
        let has_return = stmts.iter().any(|s| matches!(s, Statement::Return(_)));
        assert!(has_return, "Should have return statement");
    }

    #[test]
    fn test_forward_to_parent() {
        let sys = parse_py(
            "machine:\n    $Child {\n        evt() {\n            => $^\n        }\n    }",
        );
        let machine = sys.machine.unwrap();
        let stmts = &machine.states[0].handlers[0].body.statements;
        let has_forward = stmts.iter().any(|s| matches!(s, Statement::Forward(_)));
        assert!(has_forward, "Should have forward to parent");
    }

    #[test]
    fn test_multiple_states() {
        let sys =
            parse_py("machine:\n    $Idle {\n    }\n    $Running {\n    }\n    $Done {\n    }");
        let machine = sys.machine.unwrap();
        assert_eq!(machine.states.len(), 3);
        assert_eq!(machine.states[0].name, "Idle");
        assert_eq!(machine.states[1].name, "Running");
        assert_eq!(machine.states[2].name, "Done");
    }

    // --- System header param ordering tests ---

    fn parse_header(src: &str) -> Result<Vec<SystemParam>, ParseError> {
        let bytes = src.as_bytes();
        let span = Span::new(0, bytes.len());
        parse_system_header_params(bytes, span)
    }

    #[test]
    fn test_param_order_state_then_domain() {
        let params = parse_header("$(x: int), name: str").unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "x");
        assert!(matches!(params[0].kind, ParamKind::StateArg));
        assert_eq!(params[1].name, "name");
        assert!(matches!(params[1].kind, ParamKind::Domain));
    }

    #[test]
    fn test_param_order_state_enter_domain() {
        let params = parse_header("$(x: int), $>(msg: str), name: str").unwrap();
        assert_eq!(params.len(), 3);
        assert!(matches!(params[0].kind, ParamKind::StateArg));
        assert!(matches!(params[1].kind, ParamKind::EnterArg));
        assert!(matches!(params[2].kind, ParamKind::Domain));
    }

    #[test]
    fn test_param_order_domain_only() {
        let params = parse_header("name: str, count: int").unwrap();
        assert_eq!(params.len(), 2);
        assert!(matches!(params[0].kind, ParamKind::Domain));
        assert!(matches!(params[1].kind, ParamKind::Domain));
    }

    #[test]
    fn test_param_order_state_only() {
        let params = parse_header("$(x: int)").unwrap();
        assert_eq!(params.len(), 1);
        assert!(matches!(params[0].kind, ParamKind::StateArg));
    }

    #[test]
    fn test_param_order_enter_only() {
        let params = parse_header("$>(msg: str)").unwrap();
        assert_eq!(params.len(), 1);
        assert!(matches!(params[0].kind, ParamKind::EnterArg));
    }

    #[test]
    fn test_param_order_reject_domain_before_state() {
        let err = parse_header("name: str, $(x: int)").unwrap_err();
        assert!(err.message.contains("out of order"), "got: {}", err.message);
    }

    #[test]
    fn test_param_order_reject_domain_before_enter() {
        let err = parse_header("name: str, $>(msg: str)").unwrap_err();
        assert!(err.message.contains("out of order"), "got: {}", err.message);
    }

    #[test]
    fn test_param_order_reject_enter_before_state() {
        let err = parse_header("$>(msg: str), $(x: int)").unwrap_err();
        assert!(err.message.contains("out of order"), "got: {}", err.message);
    }

    #[test]
    fn test_param_order_reject_domain_between_state_and_enter() {
        let err = parse_header("$(x: int), name: str, $>(msg: str)").unwrap_err();
        assert!(err.message.contains("out of order"), "got: {}", err.message);
    }
}
