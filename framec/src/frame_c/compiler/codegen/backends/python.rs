//! Python code generation backend
//!
//! This is the reference implementation of the LanguageBackend trait.

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// RFC-0017 Phase A0 helper: word-boundary check for a Python identifier.
fn python_identifier_present(text: &str, ident: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| w == ident)
}

/// RFC-0017 Phase A0 helper: replace `[<param>(, <param>)*]` substrings
/// with `[]` if every comma-separated entry is a known param name. Used
/// by the Python Constructor arm to strip user-arg-bound enter_args /
/// state_args from the bare `__init__` body — those are re-supplied by
/// `__frame_init` in the proper init path.
fn python_strip_param_lists(text: &str, param_names: &[&str]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            let mut depth = 1;
            let mut j = i + 1;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            if depth == 0 {
                let inner: String = chars[i + 1..j].iter().collect();
                let parts: Vec<&str> = inner
                    .split(',')
                    .map(|p| p.trim())
                    .filter(|p| !p.is_empty())
                    .collect();
                if !parts.is_empty() && parts.iter().all(|p| param_names.contains(p)) {
                    result.push_str("[]");
                    i = j + 1;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Python backend for code generation
pub struct PythonBackend;

impl LanguageBackend for PythonBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String {
        match node {
            // ===== Structural =====
            CodegenNode::Module { imports, items } => {
                let mut result = String::new();

                // Emit imports
                for import in imports {
                    result.push_str(&self.emit(import, ctx));
                    result.push('\n');
                }

                if !imports.is_empty() && !items.is_empty() {
                    result.push('\n');
                }

                // Emit items
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
                        format!("import {} as {}", module, alias)
                    } else {
                        format!("import {}", module)
                    }
                } else {
                    format!("from {} import {}", module, items.join(", "))
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

                // Class declaration
                let bases = if base_classes.is_empty() {
                    String::new()
                } else {
                    format!("({})", base_classes.join(", "))
                };

                result.push_str(&format!("{}class {}{}:\n", ctx.get_indent(), name, bases));

                ctx.push_indent();

                // Class docstring placeholder
                if methods.is_empty() && fields.is_empty() {
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
                } else {
                    // Temporarily expose this class's name as `system_name`
                    // so the Constructor arm can identify framework helper
                    // classes (FrameEvent / FrameContext / Compartment) and
                    // skip the RFC-0017 init decouple split for them.
                    let prev_system = ctx.system_name.clone();
                    ctx.system_name = Some(name.clone());
                    for (i, method) in methods.iter().enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(&self.emit(method, ctx));
                    }
                    ctx.system_name = prev_system;
                }

                ctx.pop_indent();
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
                is_async,
                is_static,
                visibility: _,
                decorators,
            } => {
                let mut result = String::new();

                // Decorators
                for decorator in decorators {
                    result.push_str(&format!("{}@{}\n", ctx.get_indent(), decorator));
                }

                if *is_static {
                    result.push_str(&format!("{}@staticmethod\n", ctx.get_indent()));
                }

                // Method signature.
                // RFC-0015: `@@[create(name)]` factories render as
                // `@classmethod` — the decorator string `"classmethod"`
                // is in `decorators` (emitted above), and the first
                // parameter is `cls`, not `self`.
                let is_classmethod = decorators.iter().any(|d| d == "classmethod");
                let params_str = if is_classmethod {
                    let mut s = String::from("cls");
                    let rest = self.emit_params(params, false);
                    if !rest.is_empty() {
                        s.push_str(", ");
                        s.push_str(&rest);
                    }
                    s
                } else {
                    self.emit_params(params, !*is_static)
                };
                let async_prefix = if *is_async { "async " } else { "" };
                // Convert return type: void -> None, others as-is
                let return_str = if let Some(rt) = return_type {
                    let py_type = match rt.as_str() {
                        "void" => "None",
                        other => other,
                    };
                    format!(" -> {}", py_type)
                } else {
                    String::new()
                };

                result.push_str(&format!(
                    "{}{}def {}({}){}:\n",
                    ctx.get_indent(),
                    async_prefix,
                    name,
                    params_str,
                    return_str
                ));

                ctx.push_indent();

                // Method body - check if it only contains comments/empty nodes/empty native blocks
                // For Python, a native block with only comment lines is not executable code
                let has_executable_code = body.iter().any(|stmt| {
                    match stmt {
                        CodegenNode::Comment { .. } | CodegenNode::Empty => false,
                        CodegenNode::NativeBlock { code, .. } => {
                            // Check if native block has any non-comment, non-whitespace lines
                            code.lines().any(|line| {
                                let trimmed = line.trim();
                                !trimmed.is_empty() && !trimmed.starts_with('#')
                            })
                        }
                        _ => true,
                    }
                });

                if body.is_empty() || !has_executable_code {
                    // Emit any comments first
                    for stmt in body {
                        if matches!(stmt, CodegenNode::Comment { .. }) {
                            result.push_str(&self.emit(stmt, ctx));
                            result.push('\n');
                        }
                    }
                    // Then add pass
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
                // RFC-0017 (Arc A Phase A0): split the constructor into:
                //   __init__(self)             — bare framework setup
                //   _frame_init(self, <params>) — user-arg-bound + cascade
                //   _create(cls, <params>)     — static factory (two-step)
                //
                // Single-underscore on `_create` / `_frame_init` avoids
                // Python's double-underscore name-mangling so external
                // callers (factory call sites) can reach `Counter._create`
                // without the mangled `_Counter__create` form.
                //
                // Classification of body items:
                //   - Statements containing `__fire_enter_cascade` or
                //     `__process_transition_loop` → `_frame_init` only.
                //   - Statements that mention any constructor param by
                //     name (typically the `__prepareEnter` call binding
                //     enter/state args) → `_frame_init` (with full
                //     args) + `__init__` (with the param-list arg
                //     stripped to `[]` so the compartment is created
                //     with empty enter_args until `_frame_init` runs).
                //   - All other body items → `__init__` only.
                //
                // Call-site lowering:
                //   - `@@Counter(7)` → `Counter._create(7)`
                //   - `@@!Counter()` → `Counter()`
                //
                // The factory pattern unifies parameterized and zero-arg
                // systems: `Counter._create()` always means "construct
                // and run init". Bare `Counter()` always means "no init".
                let mut result = String::new();
                let class_name = ctx
                    .system_name
                    .clone()
                    .unwrap_or_else(|| "Class".to_string());
                let param_names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();

                // Skip the decouple split for framework helper classes
                // (FrameEvent / FrameContext / Compartment). They aren't
                // user-facing systems and don't need `__frame_init` /
                // `__create`. Fall back to the original single-method shape.
                let is_frame_helper = class_name.ends_with("FrameEvent")
                    || class_name.ends_with("FrameContext")
                    || class_name.ends_with("Compartment");
                if is_frame_helper {
                    let params_str = self.emit_params(params, true);
                    result.push_str(&format!(
                        "{}def __init__({}):\n",
                        ctx.get_indent(),
                        params_str
                    ));
                    ctx.push_indent();
                    if let Some(sc) = super_call {
                        result.push_str(&self.emit(sc, ctx));
                        result.push('\n');
                    }
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
                    return result;
                }

                // Render body once at the body-indent level so the lines
                // can be reused across both `__init__` and `__frame_init`.
                ctx.push_indent();
                let mut init_lines: Vec<String> = Vec::new();
                let mut frame_init_lines: Vec<String> = Vec::new();
                if let Some(sc) = super_call {
                    let s = self.emit(sc, ctx);
                    init_lines.push(s);
                }
                for stmt in body {
                    let rendered = self.emit(stmt, ctx);
                    let frame_init_only = rendered.contains("__fire_enter_cascade")
                        || rendered.contains("__process_transition_loop");
                    if frame_init_only {
                        frame_init_lines.push(rendered);
                        continue;
                    }
                    let mentions_param = param_names
                        .iter()
                        .any(|p| python_identifier_present(&rendered, p));
                    if mentions_param {
                        frame_init_lines.push(rendered.clone());
                        init_lines.push(python_strip_param_lists(&rendered, &param_names));
                    } else {
                        init_lines.push(rendered);
                    }
                }
                ctx.pop_indent();

                // Emit `def __init__(self):` — bare framework
                result.push_str(&format!("{}def __init__(self):\n", ctx.get_indent()));
                ctx.push_indent();
                if init_lines.is_empty() {
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
                } else {
                    for line in &init_lines {
                        result.push_str(line);
                        if !line.ends_with('\n') {
                            result.push('\n');
                        }
                    }
                }
                ctx.pop_indent();

                // Emit `def _frame_init(self, <params>):` — user-arg-bound
                result.push('\n');
                let frame_init_params = self.emit_params(params, true);
                result.push_str(&format!(
                    "{}def _frame_init({}):\n",
                    ctx.get_indent(),
                    frame_init_params
                ));
                ctx.push_indent();
                if frame_init_lines.is_empty() {
                    result.push_str(&format!("{}pass\n", ctx.get_indent()));
                } else {
                    for line in &frame_init_lines {
                        result.push_str(line);
                        if !line.ends_with('\n') {
                            result.push('\n');
                        }
                    }
                }
                ctx.pop_indent();

                // Emit `@classmethod def _create(cls, <params>):` — factory
                result.push('\n');
                result.push_str(&format!("{}@classmethod\n", ctx.get_indent()));
                let create_params = if params.is_empty() {
                    "cls".to_string()
                } else {
                    format!("cls, {}", self.emit_params(params, false))
                };
                result.push_str(&format!(
                    "{}def _create({}):\n",
                    ctx.get_indent(),
                    create_params
                ));
                ctx.push_indent();
                result.push_str(&format!("{}c = cls()\n", ctx.get_indent()));
                let arg_pass = param_names.join(", ");
                result.push_str(&format!(
                    "{}c._frame_init({})\n",
                    ctx.get_indent(),
                    arg_pass
                ));
                result.push_str(&format!("{}return c\n", ctx.get_indent()));
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
                    format!("{}{} = {}", indent, name, init_str)
                } else {
                    format!("{}{} = None", indent, name)
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
                    result.push_str(&format!("{}case {}:\n", ctx.get_indent(), pattern_str));

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

            CodegenNode::Await(expr) => {
                format!("await {}", self.emit(expr, ctx))
            }

            CodegenNode::Comment { text, is_doc } => {
                if *is_doc {
                    format!("{}\"\"\"{}\"\"\"", ctx.get_indent(), text)
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
                format!("{} if {} else {}", then_val, cond, else_val)
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = self.emit_lambda_params(params);
                let body_str = self.emit(body, ctx);
                format!("lambda {}: {}", params_str, body_str)
            }

            CodegenNode::Cast { expr, target_type } => {
                // Python doesn't have casts, use type constructors
                let expr_str = self.emit(expr, ctx);
                format!("{}({})", target_type, expr_str)
            }

            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", class, args_str.join(", "))
            }

            // ===== Frame-Specific =====
            CodegenNode::Transition {
                target_state,
                exit_args,
                enter_args,
                state_args,
                indent,
            } => {
                // Generate Frame transition call with string-based state dispatch
                // self._transition("TargetState", exit_args, enter_args)
                // Add relative indent to context indent (relative indent is how much more
                // indented this statement is compared to the handler body base)
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));

                // Use string-based state name for dispatch
                let mut args = vec![format!("\"{}\"", target_state)];

                if !exit_args.is_empty() {
                    let exit_str: Vec<String> =
                        exit_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", exit_str.join(", ")));
                } else {
                    args.push("None".to_string());
                }

                if !enter_args.is_empty() {
                    let enter_str: Vec<String> =
                        enter_args.iter().map(|a| self.emit(a, ctx)).collect();
                    args.push(format!("[{}]", enter_str.join(", ")));
                } else {
                    args.push("None".to_string());
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
                // Add relative indent to context indent
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));

                // Use string-based state name for dispatch
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
                // Forward dispatches event to parent state in HSM
                // Add relative indent to context indent
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));

                if *to_parent {
                    format!("{}print(\"FORWARD:PARENT\")\n{}return", ind, ind)
                } else {
                    format!("{}print(\"FORWARD:PARENT\")\n{}return", ind, ind)
                }
            }

            CodegenNode::StackPush { indent } => {
                // Add relative indent to context indent
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self._state_stack.append(self._state)", ind)
            }

            CodegenNode::StackPop { indent } => {
                // Add relative indent to context indent
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}self._transition(self._state_stack.pop())", ind)
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
        vec!["from typing import Any, Optional, List, Dict, Callable".to_string()]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::python()
    }

    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Python3
    }

    fn true_keyword(&self) -> &'static str {
        "True"
    }
    fn false_keyword(&self) -> &'static str {
        "False"
    }
    fn null_keyword(&self) -> &'static str {
        "None"
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

impl PythonBackend {
    /// Emit parameters for a method (with optional self)
    fn emit_params(&self, params: &[Param], include_self: bool) -> String {
        let mut all_params = Vec::new();

        if include_self {
            all_params.push("self".to_string());
        }

        for param in params {
            let mut param_str = param.name.clone();
            if let Some(ref type_ann) = param.type_annotation {
                param_str.push_str(&format!(": {}", type_ann));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_literal() {
        let backend = PythonBackend;
        let mut ctx = EmitContext::new();

        assert_eq!(backend.emit(&CodegenNode::int(42), &mut ctx), "42");
        assert_eq!(
            backend.emit(&CodegenNode::string("hello"), &mut ctx),
            "\"hello\""
        );
        assert_eq!(backend.emit(&CodegenNode::bool(true), &mut ctx), "True");
        assert_eq!(backend.emit(&CodegenNode::null(), &mut ctx), "None");
    }

    #[test]
    fn test_emit_field_access() {
        let backend = PythonBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::field(CodegenNode::self_ref(), "_state");
        assert_eq!(backend.emit(&node, &mut ctx), "self._state");
    }

    #[test]
    fn test_emit_method_call() {
        let backend = PythonBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::method_call(
            CodegenNode::self_ref(),
            "_transition",
            vec![CodegenNode::ident("new_state")],
        );
        assert_eq!(backend.emit(&node, &mut ctx), "self._transition(new_state)");
    }

    #[test]
    fn test_emit_class() {
        let backend = PythonBackend;
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
        assert!(result.contains("class TestClass:"));
        assert!(result.contains("pass"));
    }

    #[test]
    fn test_emit_method() {
        let backend = PythonBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::Method {
            name: "test_method".to_string(),
            params: vec![Param::new("x").with_type("int")],
            return_type: Some("str".to_string()),
            body: vec![CodegenNode::ret(Some(CodegenNode::string("hello")))],
            is_async: false,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        };

        let result = backend.emit(&node, &mut ctx);
        assert!(result.contains("def test_method(self, x: int) -> str:"));
        assert!(result.contains("return \"hello\""));
    }

    #[test]
    fn test_emit_if() {
        let backend = PythonBackend;
        let mut ctx = EmitContext::new();

        let node = CodegenNode::if_stmt(
            CodegenNode::bool(true),
            vec![CodegenNode::ret(Some(CodegenNode::int(1)))],
            Some(vec![CodegenNode::ret(Some(CodegenNode::int(0)))]),
        );

        let result = backend.emit(&node, &mut ctx);
        assert!(result.contains("if True:"));
        assert!(result.contains("return 1"));
        assert!(result.contains("else:"));
        assert!(result.contains("return 0"));
    }

    #[test]
    fn test_emit_transition() {
        let backend = PythonBackend;
        let mut ctx = EmitContext::new();

        // Note: target_state is just the state name, not prefixed with _s_
        let node = CodegenNode::Transition {
            target_state: "Running".to_string(),
            exit_args: vec![],
            enter_args: vec![],
            state_args: vec![],
            indent: 0,
        };

        let result = backend.emit(&node, &mut ctx);
        // String-based state dispatch: self._transition("Running", ...)
        assert!(result.contains("self._transition(\"Running\""));
    }
}
