//! Actions and operations parser.
//!
//! Parses the `actions:` and `operations:` sections — each is a sequence of
//! methods with parameter lists and a native-body block. The two are
//! near-symmetrical (operations additionally accept the `static` modifier and
//! a return type); they share helper conventions but diverge enough in error
//! codes and shape to warrant separate methods.

use super::{ParseError, Parser};
use crate::frame_c::compiler::frame_ast::*;
use crate::frame_c::compiler::lexer::Token;

impl<'a> Parser<'a> {
    // ========================================================================
    // Actions Section
    // ========================================================================

    pub(super) fn parse_actions(&mut self) -> Result<Vec<ActionAst>, ParseError> {
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

    pub(super) fn parse_operations(&mut self) -> Result<Vec<OperationAst>, ParseError> {
        let mut ops = Vec::new();
        // Pending RFC-0013 attribute tokens accumulate here until the
        // next operation declaration consumes them. RFC-0012 amendment
        // 2026-05-02 uses this for `@@[save]` / `@@[load]` on persist
        // operations; same plumbing as `interface:` and `machine:`.
        let mut pending_attrs: Vec<crate::frame_c::compiler::frame_ast::Attribute> = Vec::new();
        loop {
            let tok = self.peek()?;
            match tok {
                Token::Attribute { .. } => {
                    let spanned = self.advance()?;
                    if let Token::Attribute { name, args } = spanned.token {
                        pending_attrs.push(crate::frame_c::compiler::frame_ast::Attribute {
                            name,
                            args,
                            span: spanned.span,
                        });
                    }
                }
                Token::Ident(_) => {
                    let mut op = self.parse_operation()?;
                    op.attributes = std::mem::take(&mut pending_attrs);
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
            attributes: vec![],
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
}
