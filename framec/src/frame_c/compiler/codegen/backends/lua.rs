//! Lua code generation backend
//!
//! Lua uses table+metatable for OOP:
//! - Classes: `local ClassName = {}; ClassName.__index = ClassName`
//! - Constructor: `function ClassName.new(args) ... end`
//! - Methods: `function ClassName:method(params) ... end` (colon syntax)
//! - Self: `self` (implicit via `:`)
//! - Fields: `self.field`
//! - Null: `nil`, Booleans: `true`/`false`
//! - Comments: `--`, Block comments: `--[[ ]]`
//! - String concat: `..`
//! - No semicolons, no type annotations
//! - Blocks use `then`/`do`/`end` (not `{ }`)

use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;

pub struct LuaBackend;

impl LanguageBackend for LuaBackend {
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
                    if i > 0 { result.push_str("\n\n"); }
                    result.push_str(&self.emit(item, ctx));
                }
                result
            }

            CodegenNode::Import { module, items: _, alias: _ } => {
                format!("local {} = require(\"{}\")", module, module)
            }

            CodegenNode::Class { name, fields: _, methods, base_classes: _, is_abstract: _, .. } => {
                let mut result = String::new();
                result.push_str(&format!("{}local {} = {{}}\n", ctx.get_indent(), name));
                result.push_str(&format!("{}{}.__index = {}\n", ctx.get_indent(), name, name));

                for method in methods {
                    result.push('\n');
                    result.push_str(&self.emit_with_class(method, ctx, name));
                }
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}local {} = {{}}\n", ctx.get_indent(), name);
                for variant in variants {
                    if let Some(value) = &variant.value {
                        result.push_str(&format!("{}{}.{} = {}\n", ctx.get_indent(), name, variant.name, self.emit(value, ctx)));
                    } else {
                        result.push_str(&format!("{}{}.{} = \"{}\"\n", ctx.get_indent(), name, variant.name, variant.name));
                    }
                }
                result
            }

            // ===== Methods =====

            CodegenNode::Method { name, params, return_type: _, body, is_async: _, is_static, visibility: _, decorators: _ } => {
                let mut result = String::new();
                let class_name = ctx.extra.get("class_name").cloned().unwrap_or("M".to_string());
                let params_str = self.emit_params(params);

                if *is_static {
                    result.push_str(&format!("{}function {}.{}({})\n", ctx.get_indent(), class_name, name, params_str));
                } else {
                    result.push_str(&format!("{}function {}:{}({})\n", ctx.get_indent(), class_name, name, params_str));
                }

                ctx.push_indent();
                let has_code = body.iter().any(|s| !matches!(s, CodegenNode::Comment { .. } | CodegenNode::Empty));
                if body.is_empty() || !has_code {
                    result.push_str(&format!("{}-- empty\n", ctx.get_indent()));
                } else {
                    for stmt in body {
                        result.push_str(&self.emit(stmt, ctx));
                        if !matches!(stmt, CodegenNode::Comment { .. } | CodegenNode::Empty | CodegenNode::If { .. } | CodegenNode::While { .. } | CodegenNode::For { .. } | CodegenNode::Match { .. }) {
                            result.push('\n');
                        }
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor { params, body, super_call } => {
                let mut result = String::new();
                let class_name = ctx.extra.get("class_name").cloned().unwrap_or("M".to_string());
                let params_str = self.emit_params(params);

                result.push_str(&format!("{}function {}.new({})\n", ctx.get_indent(), class_name, params_str));
                ctx.push_indent();
                result.push_str(&format!("{}local self = setmetatable({{}}, {})\n", ctx.get_indent(), class_name));

                if let Some(sc) = super_call {
                    result.push_str(&self.emit(sc, ctx));
                    result.push('\n');
                }
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    if !matches!(stmt, CodegenNode::Comment { .. } | CodegenNode::Empty | CodegenNode::If { .. } | CodegenNode::While { .. } | CodegenNode::For { .. } | CodegenNode::Match { .. }) {
                        result.push('\n');
                    }
                }
                result.push_str(&format!("{}return self\n", ctx.get_indent()));
                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            // ===== Statements =====

            CodegenNode::VarDecl { name, type_annotation: _, init, is_const: _ } => {
                let indent = ctx.get_indent();
                if let Some(init_expr) = init {
                    format!("{}local {} = {}", indent, name, self.emit(init_expr, ctx))
                } else {
                    format!("{}local {} = nil", indent, name)
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
                let mut result = String::new();
                result.push_str(&format!("{}if {} then\n", ctx.get_indent(), self.emit(condition, ctx)));
                ctx.push_indent();
                if then_block.is_empty() {
                    result.push_str(&format!("{}-- empty\n", ctx.get_indent()));
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
                    if else_stmts.is_empty() {
                        result.push_str(&format!("{}-- empty\n", ctx.get_indent()));
                    } else {
                        for stmt in else_stmts {
                            result.push_str(&self.emit(stmt, ctx));
                            result.push('\n');
                        }
                    }
                    ctx.pop_indent();
                }
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                let mut result = String::new();
                let s = self.emit(scrutinee, ctx);
                for (i, arm) in arms.iter().enumerate() {
                    let p = self.emit(&arm.pattern, ctx);
                    if i == 0 {
                        result.push_str(&format!("{}if {} == {} then\n", ctx.get_indent(), s, p));
                    } else {
                        result.push_str(&format!("{}elseif {} == {} then\n", ctx.get_indent(), s, p));
                    }
                    ctx.push_indent();
                    for stmt in &arm.body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            CodegenNode::While { condition, body } => {
                let mut result = String::new();
                result.push_str(&format!("{}while {} do\n", ctx.get_indent(), self.emit(condition, ctx)));
                ctx.push_indent();
                for stmt in body { result.push_str(&self.emit(stmt, ctx)); result.push('\n'); }
                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            CodegenNode::For { var, iterable, body } => {
                let mut result = String::new();
                result.push_str(&format!("{}for _, {} in ipairs({}) do\n", ctx.get_indent(), var, self.emit(iterable, ctx)));
                ctx.push_indent();
                for stmt in body { result.push_str(&self.emit(stmt, ctx)); result.push('\n'); }
                ctx.pop_indent();
                result.push_str(&format!("{}end\n", ctx.get_indent()));
                result
            }

            CodegenNode::Break => format!("{}break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}-- continue (not native in Lua)", ctx.get_indent()),
            CodegenNode::ExprStmt(expr) => format!("{}{}", ctx.get_indent(), self.emit(expr, ctx)),
            CodegenNode::Await(expr) => self.emit(expr, ctx),
            CodegenNode::Comment { text, .. } => format!("{}-- {}", ctx.get_indent(), text),
            CodegenNode::Empty => String::new(),

            // ===== Expressions =====

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
                format!("{}:{}({})", self.emit(object, ctx), method, args_str.join(", "))
            }

            CodegenNode::FieldAccess { object, field } => {
                format!("{}.{}", self.emit(object, ctx), field)
            }

            CodegenNode::IndexAccess { object, index } => {
                format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx))
            }

            CodegenNode::SelfRef => "self".to_string(),
            CodegenNode::Array(elems) => {
                let s: Vec<String> = elems.iter().map(|e| self.emit(e, ctx)).collect();
                format!("{{{}}}", s.join(", "))
            }
            CodegenNode::Dict(pairs) => {
                let s: Vec<String> = pairs.iter().map(|(k, v)| format!("[{}] = {}", self.emit(k, ctx), self.emit(v, ctx))).collect();
                format!("{{{}}}", s.join(", "))
            }
            CodegenNode::Ternary { condition, then_expr, else_expr } => {
                format!("({} and {} or {})", self.emit(condition, ctx), self.emit(then_expr, ctx), self.emit(else_expr, ctx))
            }
            CodegenNode::Lambda { params, body } => {
                let p = params.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ");
                format!("function({}) return {} end", p, self.emit(body, ctx))
            }
            CodegenNode::Cast { expr, .. } => self.emit(expr, ctx),
            CodegenNode::New { class, args } => {
                let a: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}.new({})", class, a.join(", "))
            }

            // ===== Frame-Specific =====

            CodegenNode::Transition { target_state, exit_args, enter_args, state_args, indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                let mut a = vec![format!("\"{}\"", target_state)];
                if !exit_args.is_empty() {
                    let s: Vec<String> = exit_args.iter().map(|x| self.emit(x, ctx)).collect();
                    a.push(format!("{{{}}}", s.join(", ")));
                } else { a.push("nil".to_string()); }
                if !enter_args.is_empty() {
                    let s: Vec<String> = enter_args.iter().map(|x| self.emit(x, ctx)).collect();
                    a.push(format!("{{{}}}", s.join(", ")));
                } else { a.push("nil".to_string()); }
                if !state_args.is_empty() {
                    let s: Vec<String> = state_args.iter().map(|x| self.emit(x, ctx)).collect();
                    a.push(format!("{{{}}}", s.join(", ")));
                }
                format!("{}self:__transition({})", ind, a.join(", "))
            }

            CodegenNode::ChangeState { target_state, state_args, indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                if state_args.is_empty() {
                    format!("{}self:__change_state(\"{}\")", ind, target_state)
                } else {
                    let a: Vec<String> = state_args.iter().map(|x| self.emit(x, ctx)).collect();
                    format!("{}self:__change_state(\"{}\", {{{}}})", ind, target_state, a.join(", "))
                }
            }

            CodegenNode::Forward { to_parent: _, indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }

            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}table.insert(self.__state_stack, self.__compartment:copy())", ind)
            }

            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self:__transition(table.remove(self.__state_stack))", ind)
            }

            CodegenNode::StateContext { state_name } => {
                format!("self.__state_context[\"{}\"]", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let a: Vec<String> = args.iter().map(|x| self.emit(x, ctx)).collect();
                if a.is_empty() {
                    format!("{}self:{}()", ctx.get_indent(), event)
                } else {
                    format!("{}self:{}({})", ctx.get_indent(), event, a.join(", "))
                }
            }

            CodegenNode::NativeBlock { code, span: _ } => {
                // Transform if/else { } to Lua if/then/end before emitting
                let code = crate::frame_c::compiler::codegen::block_transform::transform_blocks(
                    code, crate::frame_c::compiler::codegen::block_transform::BlockTransformMode::Lua);
                let lines: Vec<&str> = code.lines().collect();
                if lines.is_empty() { return String::new(); }
                let min_indent = lines.iter()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.len() - l.trim_start().len())
                    .min().unwrap_or(0);
                let indent = ctx.get_indent();
                let mut result = String::new();
                for (i, line) in lines.iter().enumerate() {
                    if line.trim().is_empty() {
                        if i < lines.len() - 1 { result.push('\n'); }
                    } else {
                        let stripped = if line.len() >= min_indent { &line[min_indent..] } else { line.trim_start() };
                        result.push_str(&indent);
                        result.push_str(stripped);
                        if i < lines.len() - 1 { result.push('\n'); }
                    }
                }
                result
            }

            CodegenNode::SplicePoint { id } => format!("-- SPLICE_POINT: {}", id),
        }
    }

    fn runtime_imports(&self) -> Vec<String> { vec![] }
    fn class_syntax(&self) -> ClassSyntax { ClassSyntax::lua() }
    fn target_language(&self) -> TargetLanguage { TargetLanguage::Lua }
    fn null_keyword(&self) -> &'static str { "nil" }
    fn and_operator(&self) -> &'static str { "and" }
    fn or_operator(&self) -> &'static str { "or" }
    fn not_operator(&self) -> &'static str { "not " }
}

impl LuaBackend {
    fn emit_with_class(&self, node: &CodegenNode, ctx: &mut EmitContext, class_name: &str) -> String {
        ctx.extra.insert("class_name".to_string(), class_name.to_string());
        self.emit(node, ctx)
    }

    fn emit_params(&self, params: &[Param]) -> String {
        params.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")
    }
}
