//! GDScript code generation backend
//!
//! GDScript (Godot Engine) is similar to Python with key differences:
//! - `func` instead of `def`
//! - `_init` instead of `__init__`
//! - `null` / `true` / `false` instead of `None` / `True` / `False`
//! - `.pop_back()` instead of `.pop()`
//! - `.size()` instead of `len()`
//! - `ClassName.new()` instead of `ClassName()`
//! - `Variant` as universal type

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// GDScript backend for code generation
pub struct GDScriptBackend;

impl LanguageBackend for GDScriptBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String {
        match node {
            // ===== Structural =====
            CodegenNode::Module { imports, items } => {
                let mut result = String::new();

                // Check if the system class declares base classes.
                // If so, emit `extends Base` at file top and promote the
                // system to module scope (GDScript one-class-per-file convention).
                let system_bases: Option<&Vec<String>> = items.iter().rev().find_map(|item| {
                    if let CodegenNode::Class { base_classes, .. } = item {
                        if !base_classes.is_empty() {
                            return Some(base_classes);
                        }
                    }
                    None
                });
                let module_scope = system_bases.is_some();
                if let Some(bases) = system_bases {
                    result.push_str(&format!("extends {}\n\n", bases[0]));
                }

                // Emit imports
                for import in imports {
                    result.push_str(&self.emit(import, ctx));
                    result.push('\n');
                }

                if !imports.is_empty() && !items.is_empty() {
                    result.push('\n');
                }

                // Emit items. When module_scope is true, the system class
                // (identified by having base_classes) is emitted at module
                // scope — no `class Name:` wrapper. Helper classes (FrameEvent,
                // FrameContext, Compartment) remain as inner classes.
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str("\n\n");
                    }
                    if module_scope {
                        if let CodegenNode::Class {
                            base_classes,
                            fields,
                            methods,
                            ..
                        } = item
                        {
                            if !base_classes.is_empty() {
                                // System class — emit at module scope
                                for field in fields {
                                    let init = if let Some(ref init_node) = field.initializer {
                                        format!(" = {}", self.emit(init_node, ctx))
                                    } else {
                                        String::new()
                                    };
                                    result.push_str(&format!("var {}{}\n", field.name, init));
                                }
                                if !fields.is_empty() && !methods.is_empty() {
                                    result.push('\n');
                                }
                                for (j, method) in methods.iter().enumerate() {
                                    if j > 0 {
                                        result.push('\n');
                                    }
                                    result.push_str(&self.emit(method, ctx));
                                }
                                continue;
                            }
                        }
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
                // GDScript doesn't have Python-style imports, but we pass through for compatibility
                if items.is_empty() {
                    if let Some(alias) = alias {
                        format!("# import {} as {}", module, alias)
                    } else {
                        format!("# import {}", module)
                    }
                } else {
                    format!("# from {} import {}", module, items.join(", "))
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

                // When a system declares base classes (@@system Foo : Base)
                // AND we're at module scope (indent 0), emit the system
                // at module scope with `extends Base` — GDScript's
                // one-class-per-file convention. Helper classes (indent 0
                // but no bases) stay as inner classes.
                let module_scope = !base_classes.is_empty() && ctx.indent == 0;

                if module_scope {
                    // Module-scope system: fields + methods, no class wrapper.
                    // `extends Base` is emitted by the pipeline before runtime
                    // types so it's the first line of the file (Godot requires this).
                    // Emit fields at module scope
                    for field in fields {
                        let init = if let Some(ref init_node) = field.initializer {
                            format!(" = {}", self.emit(init_node, ctx))
                        } else {
                            String::new()
                        };
                        result.push_str(&format!("var {}{}\n", field.name, init));
                    }
                    if !fields.is_empty() && !methods.is_empty() {
                        result.push('\n');
                    }
                    // Emit methods at module scope
                    for (i, method) in methods.iter().enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(&self.emit(method, ctx));
                    }
                } else {
                    // Inner class or no-base class: emit with class wrapper
                    let bases = if base_classes.is_empty() {
                        String::new()
                    } else {
                        format!(" extends {}", base_classes[0])
                    };

                    result.push_str(&format!("{}class {}{}:\n", ctx.get_indent(), name, bases));

                    ctx.push_indent();

                    if methods.is_empty() && fields.is_empty() {
                        result.push_str(&format!("{}pass\n", ctx.get_indent()));
                    } else {
                        for field in fields {
                            let init = if let Some(ref init_node) = field.initializer {
                                format!(" = {}", self.emit(init_node, ctx))
                            } else {
                                String::new()
                            };
                            result.push_str(&format!(
                                "{}var {}{}\n",
                                ctx.get_indent(),
                                field.name,
                                init
                            ));
                        }
                        if !fields.is_empty() && !methods.is_empty() {
                            result.push('\n');
                        }
                        for (i, method) in methods.iter().enumerate() {
                            if i > 0 {
                                result.push('\n');
                            }
                            result.push_str(&self.emit(method, ctx));
                        }
                    }

                    ctx.pop_indent();
                }
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}class {}:\n", ctx.get_indent(), name);
                ctx.push_indent();

                for variant in variants {
                    if let Some(value) = &variant.value {
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
                result
            }

            // ===== Methods =====
            CodegenNode::Method {
                name,
                params,
                return_type,
                body,
                is_async: _,
                is_static,
                visibility: _,
                decorators,
            } => {
                let mut result = String::new();

                // Decorators (GDScript uses @annotations too)
                for decorator in decorators {
                    result.push_str(&format!("{}@{}\n", ctx.get_indent(), decorator));
                }

                // Method signature - GDScript uses `func` not `def`
                // Static methods use `static func` keyword, not decorator
                let params_str = self.emit_params(params, !*is_static);
                let return_str = if let Some(rt) = return_type {
                    let gd_type = Self::map_type(rt);
                    format!(" -> {}", gd_type)
                } else {
                    String::new()
                };

                let static_prefix = if *is_static { "static " } else { "" };
                result.push_str(&format!(
                    "{}{}func {}({}){}:\n",
                    ctx.get_indent(),
                    static_prefix,
                    name,
                    params_str,
                    return_str
                ));

                ctx.push_indent();

                // Method body - check if it only contains comments/empty nodes/empty native blocks
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
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
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
                result
            }

            CodegenNode::Constructor {
                params,
                body,
                super_call,
            } => {
                let mut result = String::new();

                // GDScript uses _init instead of __init__
                let params_str = self.emit_params(params, false);
                result.push_str(&format!(
                    "{}func _init({}):\n",
                    ctx.get_indent(),
                    params_str
                ));

                ctx.push_indent();

                // Super call if present
                if let Some(super_call) = super_call {
                    result.push_str(&self.emit(super_call, ctx));
                    result.push('\n');
                }

                // Body
                if body.is_empty() {
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
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
                if let Some(init_expr) = init {
                    let init_str = self.emit(init_expr, ctx);
                    format!("{}var {} = {}", indent, name, init_str)
                } else {
                    format!("{}var {} = null", indent, name)
                }
            }

            CodegenNode::Assignment { target, value } => {
                let target_str = self.emit(target, ctx);
                let value_str = self.emit(value, ctx);
                format!("{}{} = {}", ctx.get_indent(), target_str, value_str)
            }

            CodegenNode::Return { value } => {
                let indent = ctx.get_indent();
                if let Some(val) = value {
                    format!("{}return {}", indent, self.emit(val, ctx))
                } else {
                    format!("{}return", indent)
                }
            }

            CodegenNode::If {
                condition,
                then_block,
                else_block,
            } => {
                let mut result = String::new();
                let cond_str = self.emit(condition, ctx);
                result.push_str(&format!("{}if {}:\n", ctx.get_indent(), cond_str));

                ctx.push_indent();
                if then_block.is_empty() {
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
                } else {
                    for stmt in then_block {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                if let Some(else_stmts) = else_block {
                    result.push_str(&format!("{}else:\n", ctx.get_indent()));
                    ctx.push_indent();
                    if else_stmts.is_empty() {
                        result.push_str(&format!("{}pass\n", ctx.get_indent()));
                    } else {
                        for stmt in else_stmts {
                            result.push_str(&self.emit(stmt, ctx));
                            result.push('\n');
                        }
                    }
                    ctx.pop_indent();
                }

                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                let mut result = String::new();
                let scrutinee_str = self.emit(scrutinee, ctx);
                result.push_str(&format!("{}match {}:\n", ctx.get_indent(), scrutinee_str));

                ctx.push_indent();
                for arm in arms {
                    let pattern_str = self.emit(&arm.pattern, ctx);
                    result.push_str(&format!("{}{}:\n", ctx.get_indent(), pattern_str));

                    ctx.push_indent();
                    if arm.body.is_empty() {
                        result.push_str(&format!("{}pass\n", ctx.get_indent()));
                    } else {
                        for stmt in &arm.body {
                            result.push_str(&self.emit(stmt, ctx));
                            result.push('\n');
                        }
                    }
                    ctx.pop_indent();
                }
                ctx.pop_indent();

                result
            }

            CodegenNode::While { condition, body } => {
                let mut result = String::new();
                let cond_str = self.emit(condition, ctx);
                result.push_str(&format!("{}while {}:\n", ctx.get_indent(), cond_str));

                ctx.push_indent();
                if body.is_empty() {
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
                } else {
                    for stmt in body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

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
                    "{}for {} in {}:\n",
                    ctx.get_indent(),
                    var,
                    iter_str
                ));

                ctx.push_indent();
                if body.is_empty() {
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
                } else {
                    for stmt in body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                result
            }

            CodegenNode::Break => format!("{}break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}continue", ctx.get_indent()),

            CodegenNode::ExprStmt(expr) => {
                format!("{}{}", ctx.get_indent(), self.emit(expr, ctx))
            }

            CodegenNode::Comment { text, is_doc } => {
                if *is_doc {
                    // GDScript uses ## for doc comments
                    format!("{}## {}", ctx.get_indent(), text)
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
                format!("{}.{}", obj_str, field)
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
                    .map(|(k, v)| format!("{}: {}", self.emit(k, ctx), self.emit(v, ctx)))
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
                // GDScript ternary: value_if_true if condition else value_if_false
                format!("{} if {} else {}", then_val, cond, else_val)
            }

            CodegenNode::Lambda { params, body } => {
                // GDScript uses func(params): return body for lambdas
                let params_str = self.emit_lambda_params(params);
                let body_str = self.emit(body, ctx);
                format!("func({}): return {}", params_str, body_str)
            }

            CodegenNode::Cast { expr, target_type } => {
                // GDScript: value as Type
                let expr_str = self.emit(expr, ctx);
                format!("{} as {}", expr_str, target_type)
            }

            CodegenNode::New { class, args } => {
                // GDScript uses ClassName.new(args)
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
                    args.push("null".to_string());
                }

                if !enter_args.is_empty() {
                    let enter_str: Vec<String> =
                        enter_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", enter_str.join(", ")));
                } else {
                    args.push("null".to_string());
                }

                if !state_args.is_empty() {
                    let state_str: Vec<String> =
                        state_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", state_str.join(", ")));
                }

                format!("{}self._transition({})", ind, args.join(", "))
            }

            CodegenNode::ChangeState {
                target_state,
                state_args,
                indent,
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));

                if state_args.is_empty() {
                    format!("{}self._change_state(\"{}\")", ind, target_state)
                } else {
                    let args_str: Vec<String> =
                        state_args.iter().map(|a| self.emit(a, ctx)).collect();
                    format!(
                        "{}self._change_state(\"{}\", [{}])",
                        ind,
                        target_state,
                        args_str.join(", ")
                    )
                }
            }

            CodegenNode::Forward { to_parent, indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));

                if *to_parent {
                    format!("{}print(\"FORWARD:PARENT\")\n{}return", ind, ind)
                } else {
                    format!("{}print(\"FORWARD:PARENT\")\n{}return", ind, ind)
                }
            }

            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self._state_stack.append(self._state)", ind)
            }

            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self._transition(self._state_stack.pop_back())", ind)
            }

            CodegenNode::StateContext { state_name } => {
                format!("self._state_context[\"{}\"]", state_name)
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
                let lines: Vec<&str> = code.lines().collect();
                if lines.is_empty() {
                    return String::new();
                }

                let min_indent = lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.len() - line.trim_start().len())
                    .min()
                    .unwrap_or(0);

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

            CodegenNode::Await(expr) => {
                // GDScript uses `await` keyword
                format!("await {}", self.emit(expr, ctx))
            }

            CodegenNode::SplicePoint { id } => {
                format!("# SPLICE_POINT: {}", id)
            }
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        // GDScript has no import statement - runtime types are defined inline
        vec![]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::gdscript()
    }

    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::GDScript
    }

    // GDScript uses lowercase true/false/null (not Python's True/False/None)
    fn true_keyword(&self) -> &'static str {
        "true"
    }
    fn false_keyword(&self) -> &'static str {
        "false"
    }
    fn null_keyword(&self) -> &'static str {
        "null"
    }
    fn and_operator(&self) -> &'static str {
        "and"
    }
    fn or_operator(&self) -> &'static str {
        "or"
    }
    fn not_operator(&self) -> &'static str {
        "not "
    }
}

impl GDScriptBackend {
    /// Map Python/generic types to GDScript equivalents
    fn map_type(t: &str) -> &str {
        match t {
            "list" | "List" => "Array",
            "dict" | "Dict" => "Dictionary",
            "str" => "String",
            "int" => "int",
            "float" => "float",
            "bool" => "bool",
            "void" => "void",
            "None" | "NoneType" => "Variant",
            "Any" | "any" | "object" | "Object" => "Variant",
            other => other,
        }
    }

    /// Emit parameters for a method (without self - GDScript doesn't include self in param list)
    fn emit_params(&self, params: &[Param], _include_self: bool) -> String {
        let mut all_params = Vec::new();

        // GDScript doesn't include self in method param lists
        for param in params {
            let mut param_str = param.name.clone();
            if let Some(ref type_ann) = param.type_annotation {
                param_str.push_str(&format!(": {}", Self::map_type(type_ann)));
            }
            if let Some(ref default) = param.default_value {
                let mut ctx = EmitContext::new();
                param_str.push_str(&format!(" = {}", self.emit(default, &mut ctx)));
            }
            all_params.push(param_str);
        }

        all_params.join(", ")
    }

    /// Emit parameters for a lambda
    fn emit_lambda_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
