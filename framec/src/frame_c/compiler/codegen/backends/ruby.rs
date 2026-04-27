//! Ruby code generation backend
//!
//! Ruby uses `end` to close blocks instead of `}`:
//! - `class ClassName ... end`
//! - `def method_name(params) ... end`
//! - `def initialize ... end` for constructor
//! - `if/elsif/else/end`, `while/end`, `case/when/end`
//! - `@field` for instance variables
//! - `self.method()` for calling own methods
//! - `nil` for null, no semicolons
//! - `ClassName.new(args)` for instantiation

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// Ruby backend for code generation
pub struct RubyBackend;

impl LanguageBackend for RubyBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String {
        match node {
            // ===== Structural =====
            CodegenNode::Module { imports, items } => {
                let mut result = String::new();
                for import in imports {
                    result.push_str(&self.emit(import, ctx));
                    result.push('\n');
                }
                if !imports.is_empty() && !items.is_empty() {
                    result.push('\n');
                }
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str("\n\n");
                    }
                    result.push_str(&self.emit(item, ctx));
                }
                result
            }

            CodegenNode::Import {
                module,
                items,
                alias,
            } => {
                if items.is_empty() {
                    if let Some(alias) = alias {
                        format!("require '{}' # as {}", module, alias)
                    } else {
                        format!("require '{}'", module)
                    }
                } else {
                    format!("require '{}'", module)
                }
            }

            CodegenNode::Class {
                name,
                fields,
                methods,
                base_classes,
                is_abstract: _,
                ..
            } => {
                let mut result = String::new();

                let extends = if base_classes.is_empty() {
                    String::new()
                } else {
                    format!(" < {}", base_classes[0])
                };

                result.push_str(&format!("{}class {}{}\n", ctx.get_indent(), name, extends));
                ctx.push_indent();

                // Fields — Ruby uses attr_accessor for everything; the
                // constructor handles per-field initialization. The
                // structured Field slots (type, init, is_const) are
                // intentionally unused — they'd only matter if we ever
                // emit Ruby class-level constants, which we don't.
                for field in fields {
                    result.push_str(&self.emit_field(field, ctx));
                }
                if !fields.is_empty() && !methods.is_empty() {
                    result.push('\n');
                }

                // Methods
                for (i, method) in methods.iter().enumerate() {
                    if i > 0 {
                        result.push('\n');
                    }
                    result.push_str(&self.emit(method, ctx));
                }

                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            CodegenNode::Enum { name, variants } => {
                // Ruby: use a module with constants
                let mut result = format!("{}module {}\n", ctx.get_indent(), name);
                ctx.push_indent();
                for variant in variants {
                    if let Some(ref value) = variant.value {
                        result.push_str(&format!(
                            "{}{} = {}\n",
                            ctx.get_indent(),
                            variant.name,
                            self.emit(value, ctx)
                        ));
                    } else {
                        result.push_str(&format!(
                            "{}{} = \"{}\"\n",
                            ctx.get_indent(),
                            variant.name,
                            variant.name
                        ));
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            // ===== Methods =====
            CodegenNode::Method {
                name,
                params,
                body,
                is_async: _,
                is_static,
                visibility,
                return_type: _,
                decorators: _,
            } => {
                let mut result = String::new();

                let params_str = self.emit_params(params);
                let static_prefix = if *is_static { "self." } else { "" };

                // Ruby: visibility is handled separately (private/protected blocks)
                // For generated code, just emit def
                let params_part = if params_str.is_empty() {
                    String::new()
                } else {
                    format!("({})", params_str)
                };

                result.push_str(&format!(
                    "{}def {}{}{}\n",
                    ctx.get_indent(),
                    static_prefix,
                    name,
                    params_part
                ));

                ctx.push_indent();

                // Method body
                let has_executable_code = body.iter().any(|stmt| match stmt {
                    CodegenNode::Comment { .. } | CodegenNode::Empty => false,
                    CodegenNode::NativeBlock { code, .. } => code.lines().any(|line| {
                        let trimmed = line.trim();
                        !trimmed.is_empty() && !trimmed.starts_with('#')
                    }),
                    _ => true,
                });

                if body.is_empty() || !has_executable_code {
                    for stmt in body {
                        if matches!(stmt, CodegenNode::Comment { .. }) {
                            result.push_str(&self.emit(stmt, ctx));
                            result.push('\n');
                        }
                    }
                    // Ruby: empty method body is fine (returns nil)
                } else {
                    for stmt in body {
                        result.push_str(&self.emit(stmt, ctx));
                        if !matches!(
                            stmt,
                            CodegenNode::Comment { .. }
                                | CodegenNode::Empty
                                | CodegenNode::If { .. }
                                | CodegenNode::While { .. }
                                | CodegenNode::For { .. }
                                | CodegenNode::Match { .. }
                        ) {
                            result.push('\n');
                        }
                    }
                }

                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor {
                params,
                body,
                super_call,
            } => {
                let mut result = String::new();

                let params_str = self.emit_params(params);
                let params_part = if params_str.is_empty() {
                    String::new()
                } else {
                    format!("({})", params_str)
                };

                result.push_str(&format!(
                    "{}def initialize{}\n",
                    ctx.get_indent(),
                    params_part
                ));
                ctx.push_indent();

                if let Some(super_call) = super_call {
                    result.push_str(&self.emit(super_call, ctx));
                    result.push('\n');
                }

                if body.is_empty() {
                    // Empty constructor is fine in Ruby
                } else {
                    for stmt in body {
                        result.push_str(&self.emit(stmt, ctx));
                        if !matches!(
                            stmt,
                            CodegenNode::Comment { .. }
                                | CodegenNode::Empty
                                | CodegenNode::If { .. }
                                | CodegenNode::While { .. }
                                | CodegenNode::For { .. }
                                | CodegenNode::Match { .. }
                        ) {
                            result.push('\n');
                        }
                    }
                }

                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            // ===== Statements =====
            CodegenNode::VarDecl {
                name,
                init,
                is_const: _,
                type_annotation: _,
            } => {
                // Ruby: just name = value (no keyword, no type)
                if let Some(init_expr) = init {
                    let init_str = self.emit(init_expr, ctx);
                    format!("{}{} = {}", ctx.get_indent(), name, init_str)
                } else {
                    format!("{}{} = nil", ctx.get_indent(), name)
                }
            }

            CodegenNode::Assignment { target, value } => {
                let target_str = self.emit(target, ctx);
                let value_str = self.emit(value, ctx);
                format!("{}{} = {}", ctx.get_indent(), target_str, value_str)
            }

            CodegenNode::Return { value } => {
                if let Some(val) = value {
                    format!("{}return {}", ctx.get_indent(), self.emit(val, ctx))
                } else {
                    format!("{}return", ctx.get_indent())
                }
            }

            CodegenNode::If {
                condition,
                then_block,
                else_block,
            } => {
                let mut result = String::new();
                let cond_str = self.emit(condition, ctx);
                result.push_str(&format!("{}if {}\n", ctx.get_indent(), cond_str));

                ctx.push_indent();
                if then_block.is_empty() {
                    // Ruby: empty block is fine
                } else {
                    for stmt in then_block {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                if let Some(else_stmts) = else_block {
                    result.push_str(&format!("{}else\n", ctx.get_indent()));
                    ctx.push_indent();
                    for stmt in else_stmts {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }

                result.push_str(&format!("{}end", ctx.get_indent()));
                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                // Ruby: case/when
                let mut result = String::new();
                let scrutinee_str = self.emit(scrutinee, ctx);
                result.push_str(&format!("{}case {}\n", ctx.get_indent(), scrutinee_str));

                for arm in arms {
                    let pattern_str = self.emit(&arm.pattern, ctx);
                    result.push_str(&format!("{}when {}\n", ctx.get_indent(), pattern_str));

                    ctx.push_indent();
                    for stmt in &arm.body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }

                result.push_str(&format!("{}end", ctx.get_indent()));
                result
            }

            CodegenNode::While { condition, body } => {
                let mut result = String::new();
                let cond_str = self.emit(condition, ctx);
                result.push_str(&format!("{}while {}\n", ctx.get_indent(), cond_str));

                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();

                result.push_str(&format!("{}end", ctx.get_indent()));
                result
            }

            CodegenNode::For {
                var,
                iterable,
                body,
            } => {
                // Ruby: collection.each do |item| ... end
                let mut result = String::new();
                let iter_str = self.emit(iterable, ctx);
                result.push_str(&format!(
                    "{}{}.each do |{}|\n",
                    ctx.get_indent(),
                    iter_str,
                    var
                ));

                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();

                result.push_str(&format!("{}end", ctx.get_indent()));
                result
            }

            CodegenNode::Break => format!("{}break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}next", ctx.get_indent()), // Ruby uses 'next'

            CodegenNode::ExprStmt(expr) => {
                format!("{}{}", ctx.get_indent(), self.emit(expr, ctx))
            }

            CodegenNode::Await(expr) => {
                // Ruby doesn't have native async/await
                self.emit(expr, ctx)
            }

            CodegenNode::Comment { text, is_doc } => {
                if *is_doc {
                    format!("{}# {}", ctx.get_indent(), text)
                } else {
                    format!("{}# {}", ctx.get_indent(), text)
                }
            }

            CodegenNode::Empty => String::new(),

            // ===== Expressions =====
            CodegenNode::Ident(name) => name.clone(),

            CodegenNode::Literal(lit) => self.emit_literal(lit, ctx),

            CodegenNode::BinaryOp { op, left, right } => self.emit_binary_op(op, left, right, ctx),

            CodegenNode::UnaryOp { op, operand } => self.emit_unary_op(op, operand, ctx),

            CodegenNode::Call { target, args } => {
                let target_str = self.emit(target, ctx);
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", target_str, args_str.join(", "))
            }

            CodegenNode::MethodCall {
                object,
                method,
                args,
            } => {
                let obj_str = self.emit(object, ctx);
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}.{}({})", obj_str, method, args_str.join(", "))
            }

            CodegenNode::FieldAccess { object, field } => {
                let obj_str = self.emit(object, ctx);
                // Ruby: self.field becomes @field for instance vars
                if obj_str == "self" {
                    format!("@{}", field)
                } else {
                    format!("{}.{}", obj_str, field)
                }
            }

            CodegenNode::IndexAccess { object, index } => {
                let obj_str = self.emit(object, ctx);
                let idx_str = self.emit(index, ctx);
                format!("{}[{}]", obj_str, idx_str)
            }

            CodegenNode::SelfRef => "self".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                format!("[{}]", elems.join(", "))
            }

            CodegenNode::Dict(pairs) => {
                let pairs_str: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{} => {}", self.emit(k, ctx), self.emit(v, ctx)))
                    .collect();
                format!("{{{}}}", pairs_str.join(", "))
            }

            CodegenNode::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                let cond = self.emit(condition, ctx);
                let then_val = self.emit(then_expr, ctx);
                let else_val = self.emit(else_expr, ctx);
                format!("{} ? {} : {}", cond, then_val, else_val)
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let body_str = self.emit(body, ctx);
                format!("->({}){{{}}}", params_str, body_str)
            }

            CodegenNode::Cast {
                expr,
                target_type: _,
            } => {
                // Ruby: no casts (dynamic typing)
                self.emit(expr, ctx)
            }

            CodegenNode::New { class, args } => {
                // Ruby: ClassName.new(args)
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}.new({})", class, args_str.join(", "))
            }

            // ===== Frame-Specific =====
            CodegenNode::Transition {
                target_state,
                exit_args,
                enter_args,
                state_args,
                indent,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                let mut args = vec![format!("\"{}\"", target_state)];

                if !exit_args.is_empty() {
                    let exit_str: Vec<String> =
                        exit_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", exit_str.join(", ")));
                } else {
                    args.push("nil".to_string());
                }

                if !enter_args.is_empty() {
                    let enter_str: Vec<String> =
                        enter_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", enter_str.join(", ")));
                } else {
                    args.push("nil".to_string());
                }

                if !state_args.is_empty() {
                    let state_str: Vec<String> =
                        state_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", state_str.join(", ")));
                }

                format!("{}@_transition({})", ind, args.join(", "))
            }

            CodegenNode::ChangeState {
                target_state,
                state_args,
                indent,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                if state_args.is_empty() {
                    format!("{}@_change_state(\"{}\")", ind, target_state)
                } else {
                    let args_str: Vec<String> =
                        state_args.iter().map(|a| self.emit(a, ctx)).collect();
                    format!(
                        "{}@_change_state(\"{}\", [{}])",
                        ind,
                        target_state,
                        args_str.join(", ")
                    )
                }
            }

            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }

            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}@_state_stack.push(@__compartment.copy)", ind)
            }

            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}__transition(@_state_stack.pop)", ind)
            }

            CodegenNode::StateContext { state_name } => {
                format!("@_state_context[\"{}\"]", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}self.{}()", ctx.get_indent(), event)
                } else {
                    format!(
                        "{}self.{}({})",
                        ctx.get_indent(),
                        event,
                        args_str.join(", ")
                    )
                }
            }

            // ===== Native Code Preservation =====
            CodegenNode::NativeBlock { code, span: _ } => {
                // Re-indent native code to current context
                let lines: Vec<&str> = code.lines().collect();
                if lines.is_empty() {
                    return String::new();
                }

                // Find minimum non-empty line indentation
                let min_indent = lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.len() - line.trim_start().len())
                    .min()
                    .unwrap_or(0);

                // Re-indent each line to current context
                let indent = ctx.get_indent();
                let mut result = String::new();
                for (i, line) in lines.iter().enumerate() {
                    if line.trim().is_empty() {
                        if i < lines.len() - 1 {
                            result.push('\n');
                        }
                    } else {
                        let stripped = if line.len() >= min_indent {
                            &line[min_indent..]
                        } else {
                            line.trim_start()
                        };
                        result.push_str(&indent);
                        result.push_str(stripped);
                        if i < lines.len() - 1 {
                            result.push('\n');
                        }
                    }
                }
                result
            }

            CodegenNode::SplicePoint { id } => {
                format!("# SPLICE_POINT: {}", id)
            }
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        vec![] // Ruby doesn't need imports for basic types
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::ruby()
    }

    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Ruby
    }

    fn null_keyword(&self) -> &'static str {
        "nil"
    }
    fn and_operator(&self) -> &'static str {
        "&&"
    }
    fn or_operator(&self) -> &'static str {
        "||"
    }
    fn not_operator(&self) -> &'static str {
        "!"
    }
}

impl RubyBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| {
                let mut s = p.name.clone();
                if let Some(ref d) = p.default_value {
                    let mut ctx = EmitContext::new();
                    s.push_str(&format!(" = {}", self.emit(d, &mut ctx)));
                }
                s
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Emit a Ruby field as `attr_accessor :<name>`. Per-field type,
    /// initializer, visibility, and is_const are all irrelevant for
    /// Ruby class fields — initialization happens in the constructor
    /// via `@name = value`.
    fn emit_field(&self, field: &Field, ctx: &mut EmitContext) -> String {
        let comments = field.format_leading_comments(&ctx.get_indent());
        format!(
            "{}{}attr_accessor :{}\n",
            comments,
            ctx.get_indent(),
            field.name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_literal() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        assert_eq!(backend.emit(&CodegenNode::int(42), &mut ctx), "42");
        assert_eq!(backend.emit(&CodegenNode::bool(true), &mut ctx), "true");
        assert_eq!(backend.emit(&CodegenNode::null(), &mut ctx), "nil");
    }

    #[test]
    fn test_emit_field_access() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::field(CodegenNode::self_ref(), "_state");
        assert_eq!(backend.emit(&node, &mut ctx), "@_state");
    }

    #[test]
    fn test_emit_method_call() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::MethodCall {
            object: Box::new(CodegenNode::self_ref()),
            method: "do_something".to_string(),
            args: vec![CodegenNode::int(42)],
        };
        assert_eq!(backend.emit(&node, &mut ctx), "self.do_something(42)");
    }

    #[test]
    fn test_emit_new() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::New {
            class: "MyClass".to_string(),
            args: vec![CodegenNode::string("hello")],
        };
        assert_eq!(backend.emit(&node, &mut ctx), "MyClass.new(\"hello\")");
    }

    #[test]
    fn test_emit_self_ref() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        assert_eq!(backend.emit(&CodegenNode::self_ref(), &mut ctx), "self");
    }

    #[test]
    fn test_emit_class() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::Class {
            name: "TestClass".to_string(),
            fields: vec![],
            methods: vec![],
            base_classes: vec![],
            is_abstract: false,
            derives: vec![],
            visibility: Visibility::Public,
        };

        let result = backend.emit(&node, &mut ctx);
        assert!(result.contains("class TestClass"));
        assert!(result.contains("end"));
    }

    #[test]
    fn test_emit_method() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::Method {
            name: "test_method".to_string(),
            params: vec![Param::new("x")],
            return_type: None,
            body: vec![CodegenNode::ret(Some(CodegenNode::string("hello")))],
            is_async: false,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        };

        let result = backend.emit(&node, &mut ctx);
        assert!(result.contains("def test_method(x)"));
        assert!(result.contains("return \"hello\""));
        assert!(result.contains("end"));
    }

    #[test]
    fn test_emit_if() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::if_stmt(
            CodegenNode::bool(true),
            vec![CodegenNode::ret(Some(CodegenNode::int(1)))],
            Some(vec![CodegenNode::ret(Some(CodegenNode::int(0)))]),
        );

        let result = backend.emit(&node, &mut ctx);
        assert!(result.contains("if true"));
        assert!(result.contains("return 1"));
        assert!(result.contains("else"));
        assert!(result.contains("return 0"));
        assert!(result.contains("end"));
    }

    #[test]
    fn test_emit_dict() {
        let backend = RubyBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::Dict(vec![(CodegenNode::string("a"), CodegenNode::int(1))]);
        assert_eq!(backend.emit(&node, &mut ctx), "{\"a\" => 1}");
    }
}
