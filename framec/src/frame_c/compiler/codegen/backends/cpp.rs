//! C++17 code generation backend

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// C++17 backend for code generation
pub struct CppBackend;

impl LanguageBackend for CppBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String {
        match node {
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

            CodegenNode::Import { module, .. } => format!("#include <{}>", module),

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
                    format!(" : public {}", base_classes.join(", public "))
                };

                result.push_str(&format!(
                    "{}class {}{} {{\n",
                    ctx.get_indent(),
                    name,
                    extends
                ));

                // Group fields and methods by visibility using sections
                // Private section first (fields + methods)
                let private_fields: Vec<_> = fields
                    .iter()
                    .filter(|f| !matches!(f.visibility, Visibility::Public))
                    .collect();
                let private_methods: Vec<_> = methods
                    .iter()
                    .filter(|m| {
                        if let CodegenNode::Method { visibility, .. } = m {
                            !matches!(visibility, Visibility::Public)
                        } else {
                            false
                        }
                    })
                    .collect();

                if !private_fields.is_empty() || !private_methods.is_empty() {
                    result.push_str("private:\n");
                    ctx.push_indent();
                    for field in &private_fields {
                        if let Some(ref raw_code) = field.raw_code {
                            result.push_str(&format!("{}{};\n", ctx.get_indent(), raw_code));
                        } else {
                            let type_ann = field
                                .type_annotation
                                .as_ref()
                                .unwrap_or(&"void*".to_string())
                                .clone();
                            result.push_str(&format!(
                                "{}{} {};\n",
                                ctx.get_indent(),
                                type_ann,
                                field.name
                            ));
                        }
                    }
                    if !private_fields.is_empty() && !private_methods.is_empty() {
                        result.push('\n');
                    }
                    for (i, method) in private_methods.iter().enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(&self.emit(method, ctx));
                    }
                    ctx.pop_indent();
                    result.push('\n');
                }

                // Public section (fields + constructor + methods)
                let public_fields: Vec<_> = fields
                    .iter()
                    .filter(|f| matches!(f.visibility, Visibility::Public))
                    .collect();
                let public_methods: Vec<_> = methods
                    .iter()
                    .filter(|m| {
                        matches!(m, CodegenNode::Constructor { .. })
                            || matches!(
                                m,
                                CodegenNode::Method {
                                    visibility: Visibility::Public,
                                    ..
                                }
                            )
                    })
                    .collect();

                if !public_fields.is_empty() || !public_methods.is_empty() {
                    result.push_str("public:\n");
                    ctx.push_indent();
                    for field in &public_fields {
                        if let Some(ref raw_code) = field.raw_code {
                            result.push_str(&format!("{}{};\n", ctx.get_indent(), raw_code));
                        } else {
                            let type_ann = field
                                .type_annotation
                                .as_ref()
                                .unwrap_or(&"void*".to_string())
                                .clone();
                            result.push_str(&format!(
                                "{}{} {};\n",
                                ctx.get_indent(),
                                type_ann,
                                field.name
                            ));
                        }
                    }
                    if !public_fields.is_empty() && !public_methods.is_empty() {
                        result.push('\n');
                    }
                    for (i, method) in public_methods.iter().enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(&self.emit(method, ctx));
                    }
                    ctx.pop_indent();
                }

                result.push_str(&format!("{}}};\n", ctx.get_indent()));
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}enum class {} {{\n", ctx.get_indent(), name);
                ctx.push_indent();
                for (i, variant) in variants.iter().enumerate() {
                    let comma = if i < variants.len() - 1 { "," } else { "" };
                    result.push_str(&format!("{}{}{}\n", ctx.get_indent(), variant.name, comma));
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}};\n", ctx.get_indent()));
                result
            }

            CodegenNode::Method {
                name,
                params,
                return_type,
                body,
                is_async: _,
                is_static,
                visibility: _,
                ..
            } => {
                let static_kw = if *is_static { "static " } else { "" };
                let return_str = self.map_type(return_type.as_ref().unwrap_or(&"void".to_string()));
                let params_str = self.emit_params(params);

                let mut result = format!(
                    "{}{}{} {}({}) {{\n",
                    ctx.get_indent(),
                    static_kw,
                    return_str,
                    name,
                    params_str
                );

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
                let class_name = ctx.system_name.clone().unwrap_or("Class".to_string());
                let params_str = self.emit_params(params);
                let init_list = super_call
                    .as_ref()
                    .map(|sc| format!(" : {}", self.emit(sc, ctx)))
                    .unwrap_or_default();

                let mut result = format!(
                    "{}{}({}){} {{\n",
                    ctx.get_indent(),
                    class_name,
                    params_str,
                    init_list
                );
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

            CodegenNode::VarDecl {
                name,
                type_annotation,
                init,
                is_const,
            } => {
                let const_kw = if *is_const { "const " } else { "" };
                let type_str = type_annotation
                    .as_ref()
                    .unwrap_or(&"auto".to_string())
                    .clone();
                if let Some(init_expr) = init {
                    format!(
                        "{}{}{} {} = {}",
                        ctx.get_indent(),
                        const_kw,
                        type_str,
                        name,
                        self.emit(init_expr, ctx)
                    )
                } else {
                    format!("{}{}{} {}", ctx.get_indent(), const_kw, type_str, name)
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
                let mut result = format!(
                    "{}if ({}) {{\n",
                    ctx.get_indent(),
                    self.emit(condition, ctx)
                );
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
                let mut result = format!(
                    "{}switch ({}) {{\n",
                    ctx.get_indent(),
                    self.emit(scrutinee, ctx)
                );
                ctx.push_indent();
                for arm in arms {
                    result.push_str(&format!(
                        "{}case {}:\n",
                        ctx.get_indent(),
                        self.emit(&arm.pattern, ctx)
                    ));
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
                let mut result = format!(
                    "{}while ({}) {{\n",
                    ctx.get_indent(),
                    self.emit(condition, ctx)
                );
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
                let mut result = format!(
                    "{}for (auto {} : {}) {{\n",
                    ctx.get_indent(),
                    var,
                    self.emit(iterable, ctx)
                );
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
            CodegenNode::ExprStmt(expr) => format!("{}{}", ctx.get_indent(), self.emit(expr, ctx)),
            CodegenNode::Await(expr) => {
                format!("co_await {}", self.emit(expr, ctx))
            }
            CodegenNode::Comment { text, .. } => format!("{}// {}", ctx.get_indent(), text),
            CodegenNode::Empty => String::new(),

            CodegenNode::Ident(name) => name.clone(),
            CodegenNode::Literal(lit) => self.emit_literal(lit, ctx),
            CodegenNode::BinaryOp { op, left, right } => self.emit_binary_op(op, left, right, ctx),
            CodegenNode::UnaryOp { op, operand } => self.emit_unary_op(op, operand, ctx),

            CodegenNode::Call { target, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", self.emit(target, ctx), args_str.join(", "))
            }

            CodegenNode::MethodCall {
                object,
                method,
                args,
            } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!(
                    "{}->{}({})",
                    self.emit(object, ctx),
                    method,
                    args_str.join(", ")
                )
            }

            CodegenNode::FieldAccess { object, field } => {
                format!("{}->{}", self.emit(object, ctx), field)
            }
            CodegenNode::IndexAccess { object, index } => {
                format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx))
            }
            CodegenNode::SelfRef => "this".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                format!("{{ {} }}", elems.join(", "))
            }

            CodegenNode::Dict(pairs) => {
                let pairs_str: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{{{}, {}}}", self.emit(k, ctx), self.emit(v, ctx)))
                    .collect();
                format!(
                    "std::unordered_map<std::string, std::any>{{ {} }}",
                    pairs_str.join(", ")
                )
            }

            CodegenNode::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                format!(
                    "{} ? {} : {}",
                    self.emit(condition, ctx),
                    self.emit(then_expr, ctx),
                    self.emit(else_expr, ctx)
                )
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = params
                    .iter()
                    .map(|p| {
                        let t = p
                            .type_annotation
                            .as_ref()
                            .unwrap_or(&"auto".to_string())
                            .clone();
                        format!("{} {}", t, p.name)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[&]({}) {{ return {}; }}", params_str, self.emit(body, ctx))
            }

            CodegenNode::Cast { expr, target_type } => {
                format!("static_cast<{}>({})", target_type, self.emit(expr, ctx))
            }

            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("new {}({})", class, args_str.join(", "))
            }

            // Frame-specific (expanded upstream as NativeBlock in normal pipeline)
            CodegenNode::Transition {
                target_state,
                indent,
                ..
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this->_transition({})", ind, target_state)
            }
            CodegenNode::ChangeState {
                target_state,
                indent,
                ..
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this->_changeState({})", ind, target_state)
            }
            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }
            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}_state_stack.push_back(__compartment->clone())", ind)
            }
            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}auto __saved = std::move(_state_stack.back()); _state_stack.pop_back(); __transition(std::move(__saved))", ind)
            }
            CodegenNode::StateContext { state_name } => {
                format!("this->_stateContext[\"{}\"]", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}this->{}()", ctx.get_indent(), event)
                } else {
                    format!(
                        "{}this->{}({})",
                        ctx.get_indent(),
                        event,
                        args_str.join(", ")
                    )
                }
            }

            CodegenNode::NativeBlock { code, .. } => {
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
            CodegenNode::SplicePoint { id } => format!("// SPLICE_POINT: {}", id),
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        vec![
            "#include <string>".to_string(),
            "#include <unordered_map>".to_string(),
            "#include <vector>".to_string(),
            "#include <any>".to_string(),
            "#include <memory>".to_string(),
            "#include <functional>".to_string(),
        ]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::cpp()
    }
    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Cpp
    }
    fn null_keyword(&self) -> &'static str {
        "nullptr"
    }
}

impl CppBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| {
                let type_ann = p
                    .type_annotation
                    .as_ref()
                    .unwrap_or(&"auto".to_string())
                    .clone();
                // Map generic types to C++ equivalents
                let type_ann = match type_ann.as_str() {
                    "Any" => "std::any".to_string(),
                    "string" | "String" | "str" => "std::string".to_string(),
                    "number" => "int".to_string(),
                    "boolean" => "bool".to_string(),
                    _ => type_ann,
                };
                if let Some(ref default_val) = p.default_value {
                    let default_str = self.emit(default_val, &mut super::super::backend::EmitContext::new());
                    format!("{} {} = {}", type_ann, p.name, default_str)
                } else {
                    format!("{} {}", type_ann, p.name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn map_type(&self, t: &str) -> String {
        match t {
            "Any" => "std::any".to_string(),
            "string" | "String" | "str" => "std::string".to_string(),
            "number" => "int".to_string(),
            "boolean" => "bool".to_string(),
            other => other.to_string(),
        }
    }

    fn needs_semicolon(&self, node: &CodegenNode) -> bool {
        !matches!(
            node,
            CodegenNode::If { .. }
                | CodegenNode::While { .. }
                | CodegenNode::For { .. }
                | CodegenNode::Match { .. }
                | CodegenNode::Comment { .. }
                | CodegenNode::NativeBlock { .. }
                | CodegenNode::Empty
        )
    }
}
