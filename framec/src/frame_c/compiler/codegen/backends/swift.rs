//! Swift code generation backend

use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;

/// Swift backend for code generation
pub struct SwiftBackend;

impl LanguageBackend for SwiftBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String {
        match node {
            CodegenNode::Module { imports, items } => {
                let mut result = String::new();
                for import in imports {
                    result.push_str(&self.emit(import, ctx));
                    result.push('\n');
                }
                if !imports.is_empty() && !items.is_empty() { result.push('\n'); }
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { result.push_str("\n\n"); }
                    result.push_str(&self.emit(item, ctx));
                }
                result
            }

            CodegenNode::Import { module, items, .. } => {
                if items.is_empty() {
                    format!("import {}", module)
                } else {
                    // Swift doesn't have selective imports, just import the module
                    format!("import {}", module)
                }
            }

            CodegenNode::Class { name, fields, methods, base_classes, is_abstract, .. } => {
                let mut result = String::new();
                // Swift doesn't have abstract classes, but we can note it
                let _abstract_kw = if *is_abstract { "/* abstract */ " } else { "" };
                let extends = if base_classes.is_empty() {
                    String::new()
                } else {
                    format!(": {}", base_classes[0])
                };

                result.push_str(&format!("{}class {}{} {{\n", ctx.get_indent(), name, extends));
                ctx.push_indent();

                for field in fields {
                    if let Some(ref raw_code) = field.raw_code {
                        // Raw code from domain section — Swift requires var/let prefix
                        // Also map Frame generic types (number, string, etc.) to Swift types
                        let mapped_code = self.map_domain_types(raw_code);
                        let trimmed = mapped_code.trim();
                        let needs_var = !trimmed.starts_with("var ") && !trimmed.starts_with("let ");
                        let var_prefix = if needs_var { "var " } else { "" };
                        let vis = self.emit_visibility_swift(field.visibility);
                        if vis.is_empty() {
                            result.push_str(&format!("{}{}{}\n", ctx.get_indent(), var_prefix, mapped_code));
                        } else {
                            result.push_str(&format!("{}{} {}{}\n", ctx.get_indent(), vis, var_prefix, mapped_code));
                        }
                    } else {
                        let vis = self.emit_visibility_swift(field.visibility);
                        let type_ann = self.map_type(field.type_annotation.as_ref().unwrap_or(&"Any?".to_string()));
                        if vis.is_empty() {
                            result.push_str(&format!("{}var {}: {}\n", ctx.get_indent(), field.name, type_ann));
                        } else {
                            result.push_str(&format!("{}{} var {}: {}\n", ctx.get_indent(), vis, field.name, type_ann));
                        }
                    }
                }
                if !fields.is_empty() && !methods.is_empty() { result.push('\n'); }

                for (i, method) in methods.iter().enumerate() {
                    if i > 0 { result.push('\n'); }
                    result.push_str(&self.emit(method, ctx));
                }

                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}enum {} {{\n", ctx.get_indent(), name);
                ctx.push_indent();
                for variant in variants {
                    result.push_str(&format!("{}case {}\n", ctx.get_indent(), variant.name));
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Method { name, params, return_type, body, is_async: _, is_static, visibility, .. } => {
                let vis = self.emit_visibility_swift(*visibility);
                let vis_prefix = if vis.is_empty() { String::new() } else { format!("{} ", vis) };
                let params_str = self.emit_params(params);

                // Swift uses "func" keyword, return type after params with " -> "
                let return_str = return_type.as_ref()
                    .filter(|t| t.as_str() != "void" && t.as_str() != "Void")
                    .map(|t| format!(" -> {}", self.map_type(t)))
                    .unwrap_or_default();

                let static_kw = if *is_static { "static " } else { "" };
                let mut result = format!("{}{}{}func {}({}){} {{\n",
                    ctx.get_indent(), vis_prefix, static_kw, name, params_str, return_str);

                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    // Swift: no semicolons
                    result.push('\n');
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor { params, body, super_call } => {
                // Swift: init() for constructor
                let params_str = self.emit_params(params);
                let mut result = format!("{}init({}) {{\n", ctx.get_indent(), params_str);
                ctx.push_indent();

                if let Some(sc) = super_call {
                    result.push_str(&format!("{}{}\n", ctx.get_indent(), self.emit(sc, ctx)));
                }

                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    // Swift: no semicolons
                    result.push('\n');
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::VarDecl { name, type_annotation, init, is_const } => {
                let decl_kw = if *is_const { "let" } else { "var" };
                let type_str = type_annotation.as_ref()
                    .map(|t| format!(": {}", self.map_type(t)))
                    .unwrap_or_default();
                if let Some(init_expr) = init {
                    format!("{}{} {}{} = {}", ctx.get_indent(), decl_kw, name, type_str, self.emit(init_expr, ctx))
                } else {
                    format!("{}{} {}{}", ctx.get_indent(), decl_kw, name, type_str)
                }
            }

            CodegenNode::Assignment { target, value } => {
                format!("{}{} = {}", ctx.get_indent(), self.emit(target, ctx), self.emit(value, ctx))
            }

            CodegenNode::Return { value } => {
                if let Some(val) = value {
                    format!("{}return {}", ctx.get_indent(), self.emit(val, ctx))
                } else {
                    format!("{}return", ctx.get_indent())
                }
            }

            CodegenNode::If { condition, then_block, else_block } => {
                let mut result = format!("{}if ({}) {{\n", ctx.get_indent(), self.emit(condition, ctx));
                ctx.push_indent();
                for stmt in then_block {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();

                if let Some(else_stmts) = else_block {
                    result.push_str(&format!("{}}} else {{\n", ctx.get_indent()));
                    ctx.push_indent();
                    for stmt in else_stmts {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                // Swift uses "switch" with no fallthrough by default
                let mut result = format!("{}switch ({}) {{\n", ctx.get_indent(), self.emit(scrutinee, ctx));
                ctx.push_indent();
                for arm in arms {
                    result.push_str(&format!("{}case {}:\n", ctx.get_indent(), self.emit(&arm.pattern, ctx)));
                    ctx.push_indent();
                    for stmt in &arm.body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::While { condition, body } => {
                let mut result = format!("{}while ({}) {{\n", ctx.get_indent(), self.emit(condition, ctx));
                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::For { var, iterable, body } => {
                // Swift: for item in collection
                let mut result = format!("{}for {} in {} {{\n", ctx.get_indent(), var, self.emit(iterable, ctx));
                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::Break => format!("{}break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}continue", ctx.get_indent()),
            CodegenNode::ExprStmt(expr) => format!("{}{}", ctx.get_indent(), self.emit(expr, ctx)),
            CodegenNode::Await(expr) => self.emit(expr, ctx),

            CodegenNode::Comment { text, is_doc } => {
                if *is_doc { format!("{}/// {}", ctx.get_indent(), text) }
                else { format!("{}// {}", ctx.get_indent(), text) }
            }

            CodegenNode::Empty => String::new(),
            CodegenNode::Ident(name) => name.clone(),
            CodegenNode::Literal(lit) => self.emit_literal(lit, ctx),
            CodegenNode::BinaryOp { op, left, right } => self.emit_binary_op(op, left, right, ctx),
            CodegenNode::UnaryOp { op, operand } => self.emit_unary_op(op, operand, ctx),

            CodegenNode::Call { target, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", self.emit(target, ctx), args_str.join(", "))
            }

            CodegenNode::MethodCall { object, method, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}.{}({})", self.emit(object, ctx), method, args_str.join(", "))
            }

            CodegenNode::FieldAccess { object, field } => format!("{}.{}", self.emit(object, ctx), field),
            // Swift: use indexer syntax obj[index]
            CodegenNode::IndexAccess { object, index } => format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx)),
            CodegenNode::SelfRef => "self".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                if elems.is_empty() {
                    "[Any?]()".to_string()
                } else {
                    format!("[{}]", elems.join(", "))
                }
            }

            CodegenNode::Dict(pairs) => {
                if pairs.is_empty() {
                    "[String: Any]()".to_string()
                } else {
                    let pairs_str: Vec<String> = pairs.iter().map(|(k, v)| {
                        format!("{}: {}", self.emit(k, ctx), self.emit(v, ctx))
                    }).collect();
                    format!("[{}]", pairs_str.join(", "))
                }
            }

            CodegenNode::Ternary { condition, then_expr, else_expr } => {
                format!("{} ? {} : {}", self.emit(condition, ctx), self.emit(then_expr, ctx), self.emit(else_expr, ctx))
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = params.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ");
                format!("{{ {} in {} }}", params_str, self.emit(body, ctx))
            }

            // Swift: "expr as! Type" for forced cast
            CodegenNode::Cast { expr, target_type } => format!("{} as! {}", self.emit(expr, ctx), target_type),

            // Swift: no "new" keyword
            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", class, args_str.join(", "))
            }

            // Frame-specific (expanded upstream as NativeBlock in normal pipeline)
            CodegenNode::Transition { target_state, indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self.__transition({}Compartment(state: \"{}\"))", ind, ctx.system_name.as_deref().unwrap_or(""), target_state)
            }
            CodegenNode::ChangeState { target_state, indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self._changeState(self.{})", ind, target_state)
            }
            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }
            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self._state_stack.append(self.__compartment.copy())", ind)
            }
            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self.__transition(self._state_stack.removeLast())", ind)
            }
            CodegenNode::StateContext { state_name } => format!("self._stateContext[\"{}\"]", state_name),

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}self.{}()", ctx.get_indent(), event)
                } else {
                    format!("{}self.{}({})", ctx.get_indent(), event, args_str.join(", "))
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
        vec!["import Foundation".to_string()]
    }

    fn class_syntax(&self) -> ClassSyntax { ClassSyntax::swift() }
    fn target_language(&self) -> TargetLanguage { TargetLanguage::Swift }
    fn null_keyword(&self) -> &'static str { "nil" }
}

impl SwiftBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params.iter().map(|p| {
            let type_ann = self.map_type(p.type_annotation.as_ref().unwrap_or(&"Any?".to_string()));
            format!("_ {}: {}", p.name, type_ann)
        }).collect::<Vec<_>>().join(", ")
    }

    /// Swift visibility: internal is default (omit), private and public are explicit
    fn emit_visibility_swift(&self, vis: Visibility) -> &'static str {
        match vis {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Protected => "internal", // Swift doesn't have protected, use internal
        }
    }

    fn map_type(&self, t: &str) -> String {
        let t = t.trim();
        // Handle nullable types: "Type | nil" or "Type | null" -> "Type?"
        if let Some(pipe_pos) = t.find('|') {
            let base = t[..pipe_pos].trim();
            let suffix = t[pipe_pos + 1..].trim();
            if suffix == "nil" || suffix == "null" || suffix == "None" {
                return format!("{}?", self.map_type(base));
            }
        }
        // Handle array types like "string[]", "number[]", etc.
        if let Some(base) = t.strip_suffix("[]") {
            return format!("[{}]", self.map_type(base));
        }
        match t {
            "Any" | "Object" | "object" => "Any?".to_string(),
            "string" | "str" => "String".to_string(),
            "String" => "String".to_string(),
            "int" | "i32" | "i64" | "number" => "Int".to_string(),
            "float" | "f64" | "f32" | "double" => "Double".to_string(),
            "bool" | "boolean" => "Bool".to_string(),
            "Boolean" => "Bool".to_string(),
            "void" => "Void".to_string(),
            "var" => "Any?".to_string(),
            other => other.to_string(),
        }
    }

    /// Map Frame generic types in raw domain code to Swift types.
    /// Handles patterns like "name: number = 0" -> "name: Int = 0"
    /// and "name: string[] = []" -> "name: [String] = []"
    fn map_domain_types(&self, raw: &str) -> String {
        // Find the colon that separates name from type
        if let Some(colon_pos) = raw.find(':') {
            let name_part = &raw[..colon_pos];
            let rest = raw[colon_pos + 1..].trim();
            // Split type from initializer (= ...)
            let (type_part, init_part) = if let Some(eq_pos) = rest.find('=') {
                (rest[..eq_pos].trim(), Some(rest[eq_pos..].to_string()))
            } else {
                (rest.trim(), None)
            };
            let mapped_type = self.map_type(type_part);
            if let Some(init) = init_part {
                format!("{}: {} {}", name_part, mapped_type, init)
            } else {
                format!("{}: {}", name_part, mapped_type)
            }
        } else {
            raw.to_string()
        }
    }
}
