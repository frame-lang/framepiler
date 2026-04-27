//! Dart code generation backend

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// Dart backend for code generation
pub struct DartBackend;

impl LanguageBackend for DartBackend {
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
                        format!("import '{}' as {};", module, alias)
                    } else {
                        format!("import '{}';", module)
                    }
                } else {
                    // Dart doesn't have named imports like TS — use show
                    format!("import '{}' show {};", module, items.join(", "))
                }
            }

            CodegenNode::Class {
                name,
                fields,
                methods,
                base_classes,
                is_abstract,
                ..
            } => {
                let mut result = String::new();

                let abstract_kw = if *is_abstract { "abstract " } else { "" };
                let extends = if base_classes.is_empty() {
                    String::new()
                } else {
                    format!(" extends {}", base_classes[0])
                };

                result.push_str(&format!(
                    "{}{}class {}{} {{\n",
                    ctx.get_indent(),
                    abstract_kw,
                    name,
                    extends
                ));

                ctx.push_indent();

                // Fields
                for field in fields {
                    result.push_str(&self.emit_field(field, ctx));
                }

                if !fields.is_empty() && !methods.is_empty() {
                    result.push('\n');
                }

                // Methods — temporarily set system_name to class name for constructor emission
                let prev_system = ctx.system_name.clone();
                ctx.system_name = Some(name.clone());
                for (i, method) in methods.iter().enumerate() {
                    if i > 0 {
                        result.push('\n');
                    }
                    result.push_str(&self.emit(method, ctx));
                }
                ctx.system_name = prev_system;

                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}enum {} {{\n", ctx.get_indent(), name);
                ctx.push_indent();

                for (i, variant) in variants.iter().enumerate() {
                    let comma = if i < variants.len() - 1 { "," } else { "" };
                    if let Some(value) = &variant.value {
                        result.push_str(&format!(
                            "{}{} = {}{}\n",
                            ctx.get_indent(),
                            variant.name,
                            self.emit(value, ctx),
                            comma
                        ));
                    } else {
                        result.push_str(&format!(
                            "{}{}{}\n",
                            ctx.get_indent(),
                            variant.name,
                            comma
                        ));
                    }
                }

                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
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
                visibility,
                decorators: _,
            } => {
                let mut result = String::new();

                let static_kw = if *is_static { "static " } else { "" };
                // Dart: `async` is a *body* modifier, placed after the
                // parameter list and before the `{`. The return type must
                // also be wrapped in `Future<T>` (or `Future<void>` for
                // void methods). `async void` is accepted but considered a
                // lint warning; `Future<void>` is canonical.
                let params_str = self.emit_params(params);
                let raw_return = return_type
                    .as_ref()
                    .map(|rt| self.convert_type(rt))
                    .unwrap_or_else(|| "void".to_string());
                let return_str = if *is_async {
                    format!("Future<{}>", raw_return)
                } else {
                    raw_return
                };
                let async_suffix = if *is_async { " async" } else { "" };

                // Dart visibility: don't add _ prefix for methods called from
                // handler code (actions, operations, state methods). The _ prefix
                // is library-private in Dart, not class-private, and handler code
                // references these methods by their original names.
                let method_name = name.clone();

                result.push_str(&format!(
                    "{}{}{} {}({}){} {{\n",
                    ctx.get_indent(),
                    static_kw,
                    return_str,
                    method_name,
                    params_str,
                    async_suffix,
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
                let class_name = ctx
                    .system_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string());

                let params_str = self.emit_params(params);
                result.push_str(&format!(
                    "{}{}({}) {{\n",
                    ctx.get_indent(),
                    class_name,
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
                type_annotation,
                init,
                is_const,
            } => {
                let keyword = if *is_const { "final" } else { "var" };
                let type_ann = type_annotation
                    .as_ref()
                    .map(|t| format!("{} ", self.convert_type(t)))
                    .unwrap_or_default();

                // If we have a type annotation, use it instead of var/final keyword
                let decl = if !type_ann.is_empty() {
                    if *is_const {
                        format!("{}final {}{}", ctx.get_indent(), type_ann, name)
                    } else {
                        format!("{}{}{}", ctx.get_indent(), type_ann, name)
                    }
                } else {
                    format!("{}{} {}", ctx.get_indent(), keyword, name)
                };

                if let Some(init_expr) = init {
                    let init_str = self.emit(init_expr, ctx);
                    format!("{} = {}", decl, init_str)
                } else {
                    decl
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
                    "{}for (final {} in {}) {{\n",
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

            CodegenNode::Comment { text, is_doc } => {
                if *is_doc {
                    format!("{}/// {}", ctx.get_indent(), text)
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
                if pairs.is_empty() {
                    // Dart: empty {} is a Set literal; use typed map literal
                    "<String, dynamic>{}".to_string()
                } else {
                    let pairs_str: Vec<String> = pairs
                        .iter()
                        .map(|(k, v)| format!("{}: {}", self.emit(k, ctx), self.emit(v, ctx)))
                        .collect();
                    format!("{{ {} }}", pairs_str.join(", "))
                }
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

            CodegenNode::Cast { expr, target_type } => {
                let expr_str = self.emit(expr, ctx);
                format!("{} as {}", expr_str, target_type)
            }

            CodegenNode::New { class, args } => {
                // Dart: no `new` keyword needed
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
                format!("{}this._stateStack.add(this._state)", ind)
            }

            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this._transition(this._stateStack.removeLast())", ind)
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

            CodegenNode::Await(expr) => {
                format!("await {}", self.emit(expr, ctx))
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
        ClassSyntax::dart()
    }

    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Dart
    }
}

impl DartBackend {
    /// Convert type annotation to Dart types
    fn convert_type(&self, type_str: &str) -> String {
        // Handle TS union types "Type | null" -> "Type?"
        if type_str.contains(" | null") {
            let base = type_str.replace(" | null", "");
            let converted = self.convert_type(&base);
            return format!("{}?", converted);
        }
        match type_str {
            "Any" | "any" => "dynamic".to_string(),
            "int" => "int".to_string(),
            "float" | "number" => "double".to_string(),
            "str" | "string" => "String".to_string(),
            "bool" | "boolean" => "bool".to_string(),
            "None" | "void" => "void".to_string(),
            "List" => "List<dynamic>".to_string(),
            "Dict" => "Map<String, dynamic>".to_string(),
            s if s.starts_with("Record<") => "Map<String, dynamic>".to_string(),
            other => other.to_string(),
        }
    }

    fn emit_params(&self, params: &[Param]) -> String {
        // Dart requires optional params to be inside [...] (positional) or {...} (named)
        // Split into required and optional params
        let mut required: Vec<String> = Vec::new();
        let mut optional: Vec<String> = Vec::new();

        for p in params {
            let type_str = p
                .type_annotation
                .as_ref()
                .map(|t| self.convert_type(t))
                .unwrap_or_else(|| "dynamic".to_string());
            if let Some(ref d) = p.default_value {
                let mut ctx = EmitContext::new();
                optional.push(format!(
                    "{} {} = {}",
                    type_str,
                    p.name,
                    self.emit(d, &mut ctx)
                ));
            } else {
                required.push(format!("{} {}", type_str, p.name));
            }
        }

        if optional.is_empty() {
            required.join(", ")
        } else if required.is_empty() {
            format!("[{}]", optional.join(", "))
        } else {
            format!("{}, [{}]", required.join(", "), optional.join(", "))
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

    /// Emit a single Dart class-field declaration line:
    ///   `<indent>[static ][late ]<type> <name>[ = <init>];\n`
    ///
    /// `late` is added when the field has no declaration-scope
    /// initializer AND the type is non-nullable AND the field isn't
    /// static — Dart requires non-nullable fields to have a definite
    /// init point, and the constructor body's `this.field = ...`
    /// satisfies that only with `late`.
    ///
    /// Private fields (Frame's `Visibility::Private`) get a leading
    /// underscore added — Dart's library-private convention. Names
    /// already starting with `_` are left as-is.
    fn emit_field(&self, field: &Field, ctx: &mut EmitContext) -> String {
        let static_kw = if field.is_static { "static " } else { "" };
        let type_str = match &field.type_annotation {
            Some(t) => self.convert_type(t),
            None => "dynamic".to_string(),
        };
        let init_suffix = match &field.initializer {
            Some(init) => format!(" = {}", self.emit(init, ctx)),
            None => String::new(),
        };
        let field_name =
            if matches!(field.visibility, Visibility::Private) && !field.name.starts_with('_') {
                format!("_{}", field.name)
            } else {
                field.name.clone()
            };
        let is_nullable = type_str.ends_with('?') || type_str == "dynamic";
        let late_kw = if init_suffix.is_empty() && !is_nullable && !field.is_static {
            "late "
        } else {
            ""
        };
        let comments = field.format_leading_comments(&ctx.get_indent());
        format!(
            "{}{}{}{}{} {}{};\n",
            comments,
            ctx.get_indent(),
            static_kw,
            late_kw,
            type_str,
            field_name,
            init_suffix
        )
    }
}
