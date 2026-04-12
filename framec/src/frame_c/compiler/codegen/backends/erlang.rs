//! Erlang code generation backend — gen_statem native
//!
//! Generates OTP gen_statem modules instead of classes.
//! States become function clauses, domain vars become a -record(data, {}).

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// Erlang backend for code generation
pub struct ErlangBackend;

impl LanguageBackend for ErlangBackend {
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
                items: _,
                alias: _,
            } => {
                format!("-include(\"{}\").", module)
            }

            CodegenNode::Class {
                name: _,
                fields: _,
                methods,
                base_classes: _,
                is_abstract: _,
                derives: _,
            } => {
                // Erlang: module functions — no class wrapper
                let mut result = String::new();
                for (i, method) in methods.iter().enumerate() {
                    if i > 0 {
                        result.push('\n');
                    }
                    result.push_str(&self.emit(method, ctx));
                }
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("%% Enum: {}\n", name);
                for variant in variants {
                    result.push_str(&format!(
                        "-define({}, {}).\n",
                        variant.name,
                        variant
                            .value
                            .as_ref()
                            .map(|v| self.emit(v, ctx))
                            .unwrap_or_else(|| format!("\"{}\"", variant.name))
                    ));
                }
                result
            }

            // ===== Methods =====
            CodegenNode::Method {
                name,
                params,
                return_type: _,
                body,
                is_async: _,
                is_static: _,
                visibility: _,
                decorators: _,
            } => {
                let param_list = params
                    .iter()
                    .map(|p| capitalize_first(&p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut result = format!("{}({}) ->\n", name, param_list);
                ctx.push_indent();
                let body_strs: Vec<String> = body
                    .iter()
                    .map(|stmt| self.emit(stmt, ctx))
                    .filter(|s| !s.is_empty())
                    .collect();
                result.push_str(&body_strs.join(",\n"));
                ctx.pop_indent();
                result.push_str(".\n");
                result
            }

            CodegenNode::Constructor {
                params: _,
                body,
                super_call: _,
            } => {
                let mut result = String::new();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                }
                result
            }

            // ===== Statements =====
            CodegenNode::VarDecl {
                name,
                type_annotation: _,
                init,
                is_const: _,
            } => {
                let indent = ctx.get_indent();
                let var_name = capitalize_first(name);
                if let Some(init_expr) = init {
                    format!("{}{} = {}", indent, var_name, self.emit(init_expr, ctx))
                } else {
                    format!("{}{} = undefined", indent, var_name)
                }
            }

            CodegenNode::Assignment { target, value } => {
                format!(
                    "{}{} = {}",
                    ctx.get_indent(),
                    self.emit(target, ctx),
                    self.emit(value, ctx)
                )
            }

            CodegenNode::Return { value } => {
                if let Some(val) = value {
                    self.emit(val, ctx)
                } else {
                    "ok".to_string()
                }
            }

            CodegenNode::If {
                condition,
                then_block,
                else_block,
            } => {
                let mut result = format!(
                    "{}case {} of\n",
                    ctx.get_indent(),
                    self.emit(condition, ctx)
                );
                ctx.push_indent();
                result.push_str(&format!("{}true ->\n", ctx.get_indent()));
                ctx.push_indent();
                for stmt in then_block {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();
                if let Some(els) = else_block {
                    result.push_str(&format!("{}; false ->\n", ctx.get_indent()));
                    ctx.push_indent();
                    for stmt in els {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }
                ctx.pop_indent();
                result.push_str(&format!("{}end", ctx.get_indent()));
                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                let mut result = format!(
                    "{}case {} of\n",
                    ctx.get_indent(),
                    self.emit(scrutinee, ctx)
                );
                ctx.push_indent();
                for (i, arm) in arms.iter().enumerate() {
                    if i > 0 {
                        result.push_str(";\n");
                    }
                    let pattern_str = self.emit(&arm.pattern, ctx);
                    result.push_str(&format!("{}{} ->\n", ctx.get_indent(), pattern_str));
                    ctx.push_indent();
                    let body_strs: Vec<String> =
                        arm.body.iter().map(|s| self.emit(s, ctx)).collect();
                    result.push_str(&body_strs.join(",\n"));
                    ctx.pop_indent();
                }
                ctx.pop_indent();
                result.push_str(&format!("\n{}end", ctx.get_indent()));
                result
            }

            CodegenNode::While {
                condition: _,
                body: _,
            } => {
                format!(
                    "{}%% while loop (requires recursive implementation)",
                    ctx.get_indent()
                )
            }

            CodegenNode::For {
                var,
                iterable,
                body: _,
            } => {
                format!(
                    "{}lists:foreach(fun({}) -> ok end, {})",
                    ctx.get_indent(),
                    capitalize_first(var),
                    self.emit(iterable, ctx)
                )
            }

            CodegenNode::Break => format!("{}%% break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}%% continue", ctx.get_indent()),

            CodegenNode::ExprStmt(expr) => {
                format!("{}{}", ctx.get_indent(), self.emit(expr, ctx))
            }

            CodegenNode::Await(expr) => self.emit(expr, ctx),

            CodegenNode::Comment { text, is_doc: _ } => {
                format!("{}%% {}", ctx.get_indent(), text)
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
                format!("{}:{}({})", obj_str, method, args_str.join(", "))
            }

            CodegenNode::FieldAccess { object, field } => {
                let obj_str = self.emit(object, ctx);
                if obj_str == "self" || obj_str == "Data" {
                    format!("Data#data.{}", field)
                } else {
                    format!("maps:get({}, {})", field, obj_str)
                }
            }

            CodegenNode::IndexAccess { object, index } => {
                format!(
                    "maps:get({}, {})",
                    self.emit(index, ctx),
                    self.emit(object, ctx)
                )
            }

            CodegenNode::SelfRef => "Data".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                format!("[{}]", elems.join(", "))
            }

            CodegenNode::Dict(pairs) => {
                let pairs_str: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{} => {}", self.emit(k, ctx), self.emit(v, ctx)))
                    .collect();
                format!("#{{{}}}", pairs_str.join(", "))
            }

            CodegenNode::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                format!(
                    "case {} of true -> {}; false -> {} end",
                    self.emit(condition, ctx),
                    self.emit(then_expr, ctx),
                    self.emit(else_expr, ctx)
                )
            }

            CodegenNode::Lambda { params, body } => {
                let param_list = params
                    .iter()
                    .map(|p| capitalize_first(&p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let body_str = self.emit(body, ctx);
                format!("fun({}) -> {} end", param_list, body_str)
            }

            CodegenNode::Cast {
                expr,
                target_type: _,
            } => self.emit(expr, ctx),

            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                let module_name = class.to_lowercase();
                format!("{}:start_link({})", module_name, args_str.join(", "))
            }

            // ===== Frame-Specific =====
            CodegenNode::Transition {
                target_state,
                exit_args: _,
                enter_args: _,
                state_args: _,
                indent,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!(
                    "{}{{next_state, {}, Data}}",
                    ind,
                    target_state.to_lowercase()
                )
            }

            CodegenNode::ChangeState {
                target_state,
                state_args: _,
                indent,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!(
                    "{}{{next_state, {}, Data}}",
                    ind,
                    target_state.to_lowercase()
                )
            }

            CodegenNode::Forward {
                indent,
                to_parent: _,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}{{keep_state, Data}}", ind)
            }

            CodegenNode::NativeBlock { code, span: _ } => {
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

            // Catch remaining variants
            _ => String::new(),
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        vec![]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::erlang()
    }

    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Erlang
    }

    fn null_keyword(&self) -> &'static str {
        "undefined"
    }
    fn true_keyword(&self) -> &'static str {
        "true"
    }
    fn false_keyword(&self) -> &'static str {
        "false"
    }
}

/// Capitalize first letter of a string (Erlang variable naming convention)
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
