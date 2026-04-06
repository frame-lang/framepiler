//! PHP code generation backend
//!
//! PHP is class-based with `$this->` member access, dynamic typing.
//! - `class SystemName { ... }`
//! - Constructor: `public function __construct() { ... }`
//! - Methods: `public function methodName($param) { ... }`
//! - Fields: `private $__compartment;`
//! - All variables prefixed with `$`: `$this->field`, `$var`
//! - Member access: `$this->method()`, `$this->field`
//! - `null` for null, `[]` for arrays/dicts

use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;

/// PHP backend for code generation
pub struct PhpBackend;

impl LanguageBackend for PhpBackend {
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

            CodegenNode::Import { module, items, alias } => {
                if items.is_empty() {
                    if let Some(alias) = alias {
                        format!("use {} as {};", module, alias)
                    } else {
                        format!("use {};", module)
                    }
                } else {
                    format!("use {}\\{{{}}};", module, items.join(", "))
                }
            }

            CodegenNode::Class { name, fields, methods, base_classes, is_abstract: _, .. } => {
                let mut result = String::new();

                let extends = if base_classes.is_empty() {
                    String::new()
                } else {
                    format!(" extends {}", base_classes[0])
                };

                result.push_str(&format!("{}class {}{} {{\n", ctx.get_indent(), name, extends));
                ctx.push_indent();

                // Fields
                for field in fields {
                    if let Some(ref raw_code) = field.raw_code {
                        let vis = self.emit_visibility(field.visibility);
                        result.push_str(&format!("{}{} ${};\n", ctx.get_indent(), vis, raw_code));
                    } else {
                        let vis = self.emit_visibility(field.visibility);
                        let static_kw = if field.is_static { "static " } else { "" };
                        if let Some(ref init) = field.initializer {
                            result.push_str(&format!("{}{} {}${} = {};\n",
                                ctx.get_indent(), vis, static_kw, field.name, self.emit(init, ctx)));
                        } else {
                            result.push_str(&format!("{}{} {}${};\n",
                                ctx.get_indent(), vis, static_kw, field.name));
                        }
                    }
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
                // PHP 8.1+ enums or class constants
                let mut result = format!("{}class {} {{\n", ctx.get_indent(), name);
                ctx.push_indent();
                for variant in variants {
                    if let Some(ref value) = variant.value {
                        result.push_str(&format!("{}const {} = {};\n",
                            ctx.get_indent(), variant.name, self.emit(value, ctx)));
                    } else {
                        result.push_str(&format!("{}const {};\n", ctx.get_indent(), variant.name));
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            // ===== Methods =====

            CodegenNode::Method { name, params, body, is_async: _, is_static, visibility, return_type: _, decorators: _ } => {
                let vis = self.emit_visibility(*visibility);
                let static_kw = if *is_static { "static " } else { "" };
                let params_str = self.emit_params(params);

                let mut result = format!("{}{} {}function {}({}) {{\n",
                    ctx.get_indent(), vis, static_kw, name, params_str);

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

            CodegenNode::Constructor { params, body, super_call } => {
                let params_str = self.emit_params(params);

                let mut result = format!("{}public function __construct({}) {{\n", ctx.get_indent(), params_str);
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

            CodegenNode::VarDecl { name, init, is_const: _, type_annotation: _ } => {
                // PHP local vars: $name = value
                if let Some(init_expr) = init {
                    let init_str = self.emit(init_expr, ctx);
                    format!("{}${} = {}", ctx.get_indent(), name, init_str)
                } else {
                    format!("{}${}", ctx.get_indent(), name)
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

            CodegenNode::If { condition, then_block, else_block } => {
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
                result.push_str(&format!("{}switch ({}) {{\n", ctx.get_indent(), scrutinee_str));

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

            CodegenNode::For { var, iterable, body } => {
                let mut result = String::new();
                let iter_str = self.emit(iterable, ctx);
                result.push_str(&format!("{}foreach ({} as ${}) {{\n", ctx.get_indent(), iter_str, var));

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
                // PHP doesn't have native async/await (frameworks handle this)
                self.emit(expr, ctx)
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

            CodegenNode::BinaryOp { op, left, right } => {
                self.emit_binary_op(op, left, right, ctx)
            }

            CodegenNode::UnaryOp { op, operand } => {
                self.emit_unary_op(op, operand, ctx)
            }

            CodegenNode::Call { target, args } => {
                let target_str = self.emit(target, ctx);
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", target_str, args_str.join(", "))
            }

            CodegenNode::MethodCall { object, method, args } => {
                let obj_str = self.emit(object, ctx);
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}->{}({})", obj_str, method, args_str.join(", "))
            }

            CodegenNode::FieldAccess { object, field } => {
                let obj_str = self.emit(object, ctx);
                format!("{}->{}", obj_str, field)
            }

            CodegenNode::IndexAccess { object, index } => {
                let obj_str = self.emit(object, ctx);
                let idx_str = self.emit(index, ctx);
                format!("{}[{}]", obj_str, idx_str)
            }

            CodegenNode::SelfRef => "$this".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                format!("[{}]", elems.join(", "))
            }

            CodegenNode::Dict(pairs) => {
                let pairs_str: Vec<String> = pairs.iter().map(|(k, v)| {
                    format!("{} => {}", self.emit(k, ctx), self.emit(v, ctx))
                }).collect();
                format!("[{}]", pairs_str.join(", "))
            }

            CodegenNode::Ternary { condition, then_expr, else_expr } => {
                let cond = self.emit(condition, ctx);
                let then_val = self.emit(then_expr, ctx);
                let else_val = self.emit(else_expr, ctx);
                format!("{} ? {} : {}", cond, then_val, else_val)
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = params.iter().map(|p| format!("${}", p.name)).collect::<Vec<_>>().join(", ");
                let body_str = self.emit(body, ctx);
                format!("function({}) {{ return {}; }}", params_str, body_str)
            }

            CodegenNode::Cast { expr, target_type } => {
                format!("({}){}", target_type, self.emit(expr, ctx))
            }

            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("new {}({})", class, args_str.join(", "))
            }

            // ===== Frame-Specific =====

            CodegenNode::Transition { target_state, indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}$this->__transition(new {}Compartment({}))",
                    ind,
                    ctx.system_name.as_deref().unwrap_or(""),
                    target_state)
            }

            CodegenNode::ChangeState { target_state, indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}$this->_changeState($this->{})", ind, target_state)
            }

            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }

            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}$this->_state_stack[] = $this->__compartment->copy()", ind)
            }

            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}$this->__transition(array_pop($this->_state_stack))", ind)
            }

            CodegenNode::StateContext { state_name } => {
                format!("$this->_stateContext[\"{}\"]", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}$this->{}()", ctx.get_indent(), event)
                } else {
                    format!("{}$this->{}({})", ctx.get_indent(), event, args_str.join(", "))
                }
            }

            // ===== Native Code Preservation =====

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

            CodegenNode::SplicePoint { id } => {
                format!("// SPLICE_POINT: {}", id)
            }
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        vec![] // PHP doesn't need imports for basic types
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::php()
    }

    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Php
    }

    fn null_keyword(&self) -> &'static str { "null" }
}

impl PhpBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params.iter().map(|p| {
            let mut s = format!("${}", p.name);
            if let Some(ref d) = p.default_value {
                let mut ctx = EmitContext::new();
                s.push_str(&format!(" = {}", self.emit(d, &mut ctx)));
            }
            s
        }).collect::<Vec<_>>().join(", ")
    }

    fn emit_visibility(&self, vis: Visibility) -> &'static str {
        match vis {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Protected => "protected",
        }
    }

    #[allow(dead_code)]
    fn map_type(&self, t: &str) -> String {
        match t {
            "Any" => "mixed".to_string(),
            "string" | "String" | "str" => "string".to_string(),
            "int" | "i32" | "i64" | "number" => "int".to_string(),
            "float" | "f64" | "f32" => "float".to_string(),
            "bool" | "boolean" => "bool".to_string(),
            "void" => "void".to_string(),
            other => other.to_string(),
        }
    }

    fn needs_semicolon(&self, node: &CodegenNode) -> bool {
        !matches!(node,
            CodegenNode::If { .. } |
            CodegenNode::While { .. } |
            CodegenNode::For { .. } |
            CodegenNode::Match { .. } |
            CodegenNode::Comment { .. } |
            CodegenNode::NativeBlock { .. } |
            CodegenNode::Empty
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_literal() {
        let backend = PhpBackend;
        let mut ctx = EmitContext::new();

        assert_eq!(backend.emit(&CodegenNode::int(42), &mut ctx), "42");
        assert_eq!(backend.emit(&CodegenNode::bool(true), &mut ctx), "true");
        assert_eq!(backend.emit(&CodegenNode::null(), &mut ctx), "null");
    }

    #[test]
    fn test_emit_field_access() {
        let backend = PhpBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::field(CodegenNode::self_ref(), "_state");
        assert_eq!(backend.emit(&node, &mut ctx), "$this->_state");
    }

    #[test]
    fn test_emit_method_call() {
        let backend = PhpBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::MethodCall {
            object: Box::new(CodegenNode::self_ref()),
            method: "doSomething".to_string(),
            args: vec![CodegenNode::int(42)],
        };
        assert_eq!(backend.emit(&node, &mut ctx), "$this->doSomething(42)");
    }

    #[test]
    fn test_emit_self_ref() {
        let backend = PhpBackend;
        let mut ctx = EmitContext::new();

        assert_eq!(backend.emit(&CodegenNode::self_ref(), &mut ctx), "$this");
    }
}
