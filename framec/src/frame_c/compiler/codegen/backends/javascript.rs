//! JavaScript (ESM) code generation backend
//!
//! Structurally identical to TypeScript minus type annotations.

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// JavaScript backend for code generation
pub struct JavaScriptBackend;

impl LanguageBackend for JavaScriptBackend {
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
                        format!("import * as {} from \"{}\";", alias, module)
                    } else {
                        format!("import \"{}\"", module)
                    }
                } else {
                    format!("import {{ {} }} from \"{}\";", items.join(", "), module)
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
                    format!(" extends {}", base_classes[0])
                };

                result.push_str(&format!(
                    "{}export class {}{} {{\n",
                    ctx.get_indent(),
                    name,
                    extends
                ));

                ctx.push_indent();

                // Fields — no type annotations, no visibility keywords
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
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                result
            }

            CodegenNode::Enum { name, variants } => {
                // JS doesn't have enums — emit as frozen object
                let mut result = format!("{}const {} = Object.freeze({{\n", ctx.get_indent(), name);
                ctx.push_indent();

                for (i, variant) in variants.iter().enumerate() {
                    let comma = if i < variants.len() - 1 { "," } else { "" };
                    if let Some(value) = &variant.value {
                        result.push_str(&format!(
                            "{}{}: {}{}\n",
                            ctx.get_indent(),
                            variant.name,
                            self.emit(value, ctx),
                            comma
                        ));
                    } else {
                        result.push_str(&format!(
                            "{}{}: {}{}\n",
                            ctx.get_indent(),
                            variant.name,
                            i,
                            comma
                        ));
                    }
                }

                ctx.pop_indent();
                result.push_str(&format!("{}}});\n", ctx.get_indent()));
                result
            }

            // ===== Methods =====
            CodegenNode::Method {
                name,
                params,
                body,
                is_async,
                is_static,
                visibility: _,
                decorators: _,
                return_type: _,
            } => {
                let mut result = String::new();

                // No visibility keywords in JS
                let static_kw = if *is_static { "static " } else { "" };
                let async_kw = if *is_async { "async " } else { "" };
                let params_str = self.emit_params(params);

                result.push_str(&format!(
                    "{}{}{}{}({}) {{\n",
                    ctx.get_indent(),
                    static_kw,
                    async_kw,
                    name,
                    params_str
                ));

                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    if self.needs_semicolon(stmt) {
                        result.push_str(";\n");
                    } else {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor {
                params,
                body,
                super_call,
            } => {
                let mut result = String::new();

                let params_str = self.emit_params(params);
                result.push_str(&format!(
                    "{}constructor({}) {{\n",
                    ctx.get_indent(),
                    params_str
                ));

                ctx.push_indent();

                if let Some(super_call) = super_call {
                    result.push_str(&self.emit(super_call, ctx));
                    result.push_str(";\n");
                }

                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    if self.needs_semicolon(stmt) {
                        result.push_str(";\n");
                    } else {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            // ===== Statements =====
            CodegenNode::VarDecl {
                name,
                init,
                is_const,
                type_annotation: _,
            } => {
                let keyword = if *is_const { "const" } else { "let" };
                // No type annotation in JS

                if let Some(init_expr) = init {
                    let init_str = self.emit(init_expr, ctx);
                    format!("{}{} {} = {}", ctx.get_indent(), keyword, name, init_str)
                } else {
                    format!("{}{} {}", ctx.get_indent(), keyword, name)
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
                result.push_str(&format!("{}if ({}) {{\n", ctx.get_indent(), cond_str));

                ctx.push_indent();
                for stmt in then_block {
                    result.push_str(&self.emit(stmt, ctx));
                    if self.needs_semicolon(stmt) {
                        result.push_str(";\n");
                    } else {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                if let Some(else_stmts) = else_block {
                    result.push_str(&format!("{}}} else {{\n", ctx.get_indent()));
                    ctx.push_indent();
                    for stmt in else_stmts {
                        result.push_str(&self.emit(stmt, ctx));
                        if self.needs_semicolon(stmt) {
                            result.push_str(";\n");
                        } else {
                            result.push('\n');
                        }
                    }
                    ctx.pop_indent();
                }

                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                let mut result = String::new();
                let scrutinee_str = self.emit(scrutinee, ctx);
                result.push_str(&format!(
                    "{}switch ({}) {{\n",
                    ctx.get_indent(),
                    scrutinee_str
                ));

                ctx.push_indent();
                for arm in arms {
                    let pattern_str = self.emit(&arm.pattern, ctx);
                    result.push_str(&format!("{}case {}:\n", ctx.get_indent(), pattern_str));

                    ctx.push_indent();
                    for stmt in &arm.body {
                        result.push_str(&self.emit(stmt, ctx));
                        if self.needs_semicolon(stmt) {
                            result.push_str(";\n");
                        } else {
                            result.push('\n');
                        }
                    }
                    result.push_str(&format!("{}break;\n", ctx.get_indent()));
                    ctx.pop_indent();
                }
                ctx.pop_indent();

                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::While { condition, body } => {
                let mut result = String::new();
                let cond_str = self.emit(condition, ctx);
                result.push_str(&format!("{}while ({}) {{\n", ctx.get_indent(), cond_str));

                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    if self.needs_semicolon(stmt) {
                        result.push_str(";\n");
                    } else {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::For {
                var,
                iterable,
                body,
            } => {
                let mut result = String::new();
                let iter_str = self.emit(iterable, ctx);
                result.push_str(&format!(
                    "{}for (const {} of {}) {{\n",
                    ctx.get_indent(),
                    var,
                    iter_str
                ));

                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    if self.needs_semicolon(stmt) {
                        result.push_str(";\n");
                    } else {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::Break => format!("{}break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}continue", ctx.get_indent()),

            CodegenNode::ExprStmt(expr) => {
                format!("{}{}", ctx.get_indent(), self.emit(expr, ctx))
            }

            CodegenNode::Await(expr) => {
                format!("await {}", self.emit(expr, ctx))
            }

            CodegenNode::Comment { text, is_doc } => {
                if *is_doc {
                    format!("{}/** {} */", ctx.get_indent(), text)
                } else {
                    format!("{}// {}", ctx.get_indent(), text)
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
                format!("{}.{}", obj_str, field)
            }

            CodegenNode::IndexAccess { object, index } => {
                let obj_str = self.emit(object, ctx);
                let idx_str = self.emit(index, ctx);
                format!("{}[{}]", obj_str, idx_str)
            }

            CodegenNode::SelfRef => "this".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                format!("[{}]", elems.join(", "))
            }

            CodegenNode::Dict(pairs) => {
                let pairs_str: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", self.emit(k, ctx), self.emit(v, ctx)))
                    .collect();
                format!("{{ {} }}", pairs_str.join(", "))
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
                format!("({}) => {}", params_str, body_str)
            }

            CodegenNode::Cast {
                expr,
                target_type: _,
            } => {
                // JS has no cast syntax — just emit the expression
                self.emit(expr, ctx)
            }

            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("new {}({})", class, args_str.join(", "))
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
                let mut args = vec![format!("this.{}", target_state)];

                if !exit_args.is_empty() || !enter_args.is_empty() || !state_args.is_empty() {
                    let exit_str: Vec<String> =
                        exit_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", exit_str.join(", ")));
                }

                if !enter_args.is_empty() || !state_args.is_empty() {
                    let enter_str: Vec<String> =
                        enter_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", enter_str.join(", ")));
                }

                if !state_args.is_empty() {
                    let state_str: Vec<String> =
                        state_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", state_str.join(", ")));
                }

                format!("{}this._transition({})", ind, args.join(", "))
            }

            CodegenNode::ChangeState {
                target_state,
                state_args,
                indent,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                if state_args.is_empty() {
                    format!("{}this._changeState(this.{})", ind, target_state)
                } else {
                    let args_str: Vec<String> =
                        state_args.iter().map(|a| self.emit(a, ctx)).collect();
                    format!(
                        "{}this._changeState(this.{}, [{}])",
                        ind,
                        target_state,
                        args_str.join(", ")
                    )
                }
            }

            CodegenNode::Forward {
                to_parent: _,
                indent,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }

            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this._stateStack.push(this._state)", ind)
            }

            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this._transition(this._stateStack.pop())", ind)
            }

            CodegenNode::StateContext { state_name } => {
                format!("this._stateContext[\"{}\"]", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}this.{}()", ctx.get_indent(), event)
                } else {
                    format!(
                        "{}this.{}({})",
                        ctx.get_indent(),
                        event,
                        args_str.join(", ")
                    )
                }
            }

            // ===== Native Code Preservation =====
            CodegenNode::NativeBlock { code, span: _ } => {
                // Apply current indentation to each line of native code
                let indent = ctx.get_indent();
                code.lines()
                    .map(|line| {
                        if line.trim().is_empty() {
                            String::new()
                        } else {
                            format!("{}{}", indent, line)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }

            CodegenNode::SplicePoint { id } => {
                format!("// SPLICE_POINT: {}", id)
            }
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        vec![]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::javascript()
    }

    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::JavaScript
    }
}

impl JavaScriptBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| {
                let mut s = p.name.clone();
                // No type annotations in JS
                if let Some(ref d) = p.default_value {
                    let mut ctx = EmitContext::new();
                    s.push_str(&format!(" = {}", self.emit(d, &mut ctx)));
                }
                s
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn needs_semicolon(&self, node: &CodegenNode) -> bool {
        !matches!(
            node,
            CodegenNode::If { .. } |
            CodegenNode::While { .. } |
            CodegenNode::For { .. } |
            CodegenNode::Match { .. } |
            CodegenNode::Comment { .. } |
            CodegenNode::NativeBlock { .. } |  // Native blocks have their own semicolons
            CodegenNode::Empty
        )
    }

    /// Emit a single JavaScript class-field declaration line:
    ///   `<indent>[static ]<name>[ = <init>];\n`
    ///
    /// JS class fields carry no type annotation and no visibility
    /// keyword. `is_const` is irrelevant for JS class fields (use
    /// `Object.freeze` or naming convention instead) and is intentionally
    /// not emitted — matching the existing `synthesize_field_raw` output
    /// for JS, which never includes a const prefix.
    fn emit_field(&self, field: &Field, ctx: &mut EmitContext) -> String {
        let static_kw = if field.is_static { "static " } else { "" };
        let init_suffix = match &field.initializer {
            Some(init) => format!(" = {}", self.emit(init, ctx)),
            None => String::new(),
        };
        let comments = field.format_leading_comments(&ctx.get_indent());
        format!(
            "{}{}{}{}{};\n",
            comments,
            ctx.get_indent(),
            static_kw,
            field.name,
            init_suffix
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_literal() {
        let backend = JavaScriptBackend;
        let mut ctx = EmitContext::new();

        assert_eq!(backend.emit(&CodegenNode::int(42), &mut ctx), "42");
        assert_eq!(backend.emit(&CodegenNode::bool(true), &mut ctx), "true");
        assert_eq!(backend.emit(&CodegenNode::null(), &mut ctx), "null");
    }

    #[test]
    fn test_emit_field_access() {
        let backend = JavaScriptBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::field(CodegenNode::self_ref(), "_state");
        assert_eq!(backend.emit(&node, &mut ctx), "this._state");
    }

    #[test]
    fn test_emit_class_no_types() {
        let backend = JavaScriptBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::Class {
            name: "TestClass".to_string(),
            fields: vec![Field::new("_state").with_visibility(Visibility::Private)],
            methods: vec![],
            base_classes: vec![],
            is_abstract: false,
            derives: vec![],
            visibility: Visibility::Public,
        };

        let result = backend.emit(&node, &mut ctx);
        assert!(result.contains("export class TestClass {"));
        assert!(result.contains("_state;"));
        // Should NOT contain type annotations or visibility keywords
        assert!(!result.contains("private"));
        assert!(!result.contains(": "));
    }

    #[test]
    fn test_emit_method_no_types() {
        let backend = JavaScriptBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::Method {
            name: "doSomething".to_string(),
            params: vec![Param::new("x").with_type("number")],
            return_type: Some("string".to_string()),
            body: vec![],
            is_async: false,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        };

        let result = backend.emit(&node, &mut ctx);
        assert!(result.contains("doSomething(x)"));
        // Should NOT contain type annotations or visibility keywords
        assert!(!result.contains("number"));
        assert!(!result.contains("string"));
        assert!(!result.contains("public"));
    }
}
