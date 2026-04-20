//! Kotlin code generation backend

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// Kotlin backend for code generation
pub struct KotlinBackend;

impl LanguageBackend for KotlinBackend {
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

            CodegenNode::Import { module, items, .. } => {
                if items.is_empty() {
                    format!("import {}.*", module)
                } else {
                    items
                        .iter()
                        .map(|i| format!("import {}.{}", module, i))
                        .collect::<Vec<_>>()
                        .join("\n")
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
                    format!(" : {}()", base_classes[0])
                };

                // Kotlin uses a primary constructor declared on the class
                // header. We extract the parameter list from the first
                // Constructor codegen node in `methods` and emit them as
                // `class Robot(val x: Int, val name: String)`. The system
                // params are then in scope inside the `init {}` block that
                // the Constructor case will emit. Without this, system
                // params declared in the AST would be invisible to the
                // init body and the call site `Robot(42)` would fail to
                // compile (no matching constructor).
                let primary_params = methods
                    .iter()
                    .find_map(|m| match m {
                        CodegenNode::Constructor { params, .. } => Some(params.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                let primary_ctor = if primary_params.is_empty() {
                    String::new()
                } else {
                    // Collect domain field names/const-ness to detect collisions.
                    // - Param collides with const field → promote to `val` (the field
                    //   declaration is suppressed below, primary constructor IS the field)
                    // - Param collides with mutable field → bare param (field decl + init body)
                    // - No collision → `val` for accessibility in handler bodies
                    let field_names: std::collections::HashSet<&str> =
                        fields.iter().map(|f| f.name.as_str()).collect();
                    let const_field_names: std::collections::HashSet<&str> = fields
                        .iter()
                        .filter(|f| f.is_const)
                        .map(|f| f.name.as_str())
                        .collect();
                    let params_str = primary_params
                        .iter()
                        .map(|p| {
                            let type_ann = self.map_type(
                                p.type_annotation.as_ref().unwrap_or(&"Any?".to_string()),
                            );
                            let default_suffix = p
                                .default_value
                                .as_ref()
                                .map(|d| format!(" = {}", self.emit(d, ctx)))
                                .unwrap_or_default();
                            if const_field_names.contains(p.name.as_str()) {
                                // const field — primary ctor declares it as val
                                format!("val {}: {}{}", p.name, type_ann, default_suffix)
                            } else if field_names.contains(p.name.as_str()) {
                                // Mutable field collision — bare param
                                format!("{}: {}{}", p.name, type_ann, default_suffix)
                            } else {
                                // No collision — promote to val property
                                format!("val {}: {}{}", p.name, type_ann, default_suffix)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({})", params_str)
                };

                // Kotlin: public by default, no access modifier needed
                result.push_str(&format!(
                    "{}{}class {}{}{} {{\n",
                    ctx.get_indent(),
                    abstract_kw,
                    name,
                    primary_ctor,
                    extends
                ));
                ctx.push_indent();

                // Const fields whose name collides with a primary-constructor
                // param are declared as `val name: Type` directly on the class
                // header (above) — skip them in the body.
                let primary_param_names: std::collections::HashSet<&str> =
                    primary_params.iter().map(|p| p.name.as_str()).collect();
                for field in fields {
                    if field.is_const && primary_param_names.contains(field.name.as_str()) {
                        continue;
                    }
                    result.push_str(&self.emit_field(field, ctx));
                }
                if !fields.is_empty() && !methods.is_empty() {
                    result.push('\n');
                }

                // Partition methods: emit non-static first, then group
                // all static methods inside a single `companion object`.
                let (statics, non_statics): (Vec<_>, Vec<_>) =
                    methods.iter().enumerate().partition(|(_, m)| {
                        matches!(
                            m,
                            CodegenNode::Method {
                                is_static: true,
                                ..
                            }
                        )
                    });

                for (i, (_, method)) in non_statics.iter().enumerate() {
                    if i > 0 {
                        result.push('\n');
                    }
                    result.push_str(&self.emit(method, ctx));
                }

                if !statics.is_empty() {
                    if !non_statics.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&format!("{}companion object {{\n", ctx.get_indent()));
                    ctx.push_indent();
                    for (i, (_, method)) in statics.iter().enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(&self.emit(method, ctx));
                    }
                    ctx.pop_indent();
                    result.push_str(&format!("{}}}\n", ctx.get_indent()));
                }

                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}enum class {} {{\n", ctx.get_indent(), name);
                ctx.push_indent();
                for (i, variant) in variants.iter().enumerate() {
                    let sep = if i < variants.len() - 1 { "," } else { "" };
                    result.push_str(&format!("{}{}{}\n", ctx.get_indent(), variant.name, sep));
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Method {
                name,
                params,
                return_type,
                body,
                is_async,
                is_static,
                visibility,
                ..
            } => {
                let vis = self.emit_visibility_kotlin(*visibility);
                let vis_prefix = if vis.is_empty() {
                    String::new()
                } else {
                    format!("{} ", vis)
                };
                let params_str = self.emit_params(params);

                // Kotlin uses "fun" keyword, return type after params with ":"
                let return_str = return_type
                    .as_ref()
                    .map(|t| format!(": {}", self.map_type(t)))
                    .unwrap_or_default();

                // Kotlin async == `suspend fun`. No return-type wrapping —
                // a suspend fun returning T is callable as T from other
                // suspend contexts (coroutines compile to continuations).
                let suspend_prefix = if *is_async { "suspend " } else { "" };

                // Static methods are wrapped in a single `companion object`
                // block by the Class emitter — emit a bare `fun` here.
                let mut result = format!(
                    "{}{}{}fun {}({}){} {{\n",
                    ctx.get_indent(),
                    vis_prefix,
                    suspend_prefix,
                    name,
                    params_str,
                    return_str
                );

                ctx.push_indent();
                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    result.push('\n');
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
                // Kotlin: use init { } block for constructor body
                let _params_str = self.emit_params(params);

                let mut result = format!("{}init {{\n", ctx.get_indent());
                ctx.push_indent();

                if let Some(sc) = super_call {
                    result.push_str(&format!("{}{}\n", ctx.get_indent(), self.emit(sc, ctx)));
                }

                for stmt in body {
                    result.push_str(&self.emit(stmt, ctx));
                    // Kotlin: no semicolons
                    result.push('\n');
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
                let decl_kw = if *is_const { "val" } else { "var" };
                let type_str = type_annotation
                    .as_ref()
                    .map(|t| format!(": {}", self.map_type(t)))
                    .unwrap_or_default();
                if let Some(init_expr) = init {
                    format!(
                        "{}{} {}{} = {}",
                        ctx.get_indent(),
                        decl_kw,
                        name,
                        type_str,
                        self.emit(init_expr, ctx)
                    )
                } else {
                    format!("{}{} {}{}", ctx.get_indent(), decl_kw, name, type_str)
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
                // Kotlin uses "when" instead of "switch"
                let mut result = format!(
                    "{}when ({}) {{\n",
                    ctx.get_indent(),
                    self.emit(scrutinee, ctx)
                );
                ctx.push_indent();
                for arm in arms {
                    result.push_str(&format!(
                        "{}{} -> {{\n",
                        ctx.get_indent(),
                        self.emit(&arm.pattern, ctx)
                    ));
                    ctx.push_indent();
                    for stmt in &arm.body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                    result.push_str(&format!("{}}}\n", ctx.get_indent()));
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
                    result.push('\n');
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
                // Kotlin: for (item in collection)
                let mut result = format!(
                    "{}for ({} in {}) {{\n",
                    ctx.get_indent(),
                    var,
                    self.emit(iterable, ctx)
                );
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
                if *is_doc {
                    format!("{}/** {} */", ctx.get_indent(), text)
                } else {
                    format!("{}// {}", ctx.get_indent(), text)
                }
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

            CodegenNode::MethodCall {
                object,
                method,
                args,
            } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!(
                    "{}.{}({})",
                    self.emit(object, ctx),
                    method,
                    args_str.join(", ")
                )
            }

            CodegenNode::FieldAccess { object, field } => {
                format!("{}.{}", self.emit(object, ctx), field)
            }
            // Kotlin: use indexer syntax obj[index]
            CodegenNode::IndexAccess { object, index } => {
                format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx))
            }
            CodegenNode::SelfRef => "this".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                if elems.is_empty() {
                    "mutableListOf<Any?>()".to_string()
                } else {
                    format!("mutableListOf({})", elems.join(", "))
                }
            }

            CodegenNode::Dict(pairs) => {
                if pairs.is_empty() {
                    "mutableMapOf<String, Any?>()".to_string()
                } else {
                    let pairs_str: Vec<String> = pairs
                        .iter()
                        .map(|(k, v)| format!("{} to {}", self.emit(k, ctx), self.emit(v, ctx)))
                        .collect();
                    format!("mutableMapOf({})", pairs_str.join(", "))
                }
            }

            CodegenNode::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                // Kotlin: if-else expression (no ternary operator)
                format!(
                    "if ({}) {} else {}",
                    self.emit(condition, ctx),
                    self.emit(then_expr, ctx),
                    self.emit(else_expr, ctx)
                )
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ {} -> {} }}", params_str, self.emit(body, ctx))
            }

            // Kotlin: "expr as Type" for cast
            CodegenNode::Cast { expr, target_type } => {
                format!("{} as {}", self.emit(expr, ctx), target_type)
            }

            // Kotlin: no "new" keyword
            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                format!("{}({})", class, args_str.join(", "))
            }

            // Frame-specific (expanded upstream as NativeBlock in normal pipeline)
            CodegenNode::Transition {
                target_state,
                indent,
                ..
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!(
                    "{}this.__transition({}Compartment({}))",
                    ind,
                    ctx.system_name.as_deref().unwrap_or(""),
                    target_state
                )
            }
            CodegenNode::ChangeState {
                target_state,
                indent,
                ..
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this._changeState(this.{})", ind, target_state)
            }
            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }
            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this._state_stack.add(this.__compartment.copy())", ind)
            }
            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!(
                    "{}this.__transition(this._state_stack.removeAt(this._state_stack.size - 1))",
                    ind
                )
            }
            CodegenNode::StateContext { state_name } => {
                format!("this._stateContext[\"{}\"]]", state_name)
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
        vec![]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::kotlin()
    }
    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Kotlin
    }
    fn null_keyword(&self) -> &'static str {
        "null"
    }
}

impl KotlinBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| {
                let type_ann =
                    self.map_type(p.type_annotation.as_ref().unwrap_or(&"Any?".to_string()));
                format!("{}: {}", p.name, type_ann)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Kotlin visibility: public is default (omit), private and protected are explicit
    fn emit_visibility_kotlin(&self, vis: Visibility) -> &'static str {
        match vis {
            Visibility::Public => "", // public is default in Kotlin
            Visibility::Private => "private",
            Visibility::Protected => "protected",
        }
    }

    fn map_type(&self, t: &str) -> String {
        match t {
            "Any" | "Object" | "object" => "Any?".to_string(),
            "string" | "str" => "String".to_string(),
            "String" => "String".to_string(),
            "int" | "i32" | "i64" | "number" => "Int".to_string(),
            "float" | "f64" | "f32" | "double" => "Double".to_string(),
            "bool" | "boolean" => "Boolean".to_string(),
            "Boolean" => "Boolean".to_string(),
            "void" => "Unit".to_string(),
            "var" => "Any?".to_string(),
            other => other.to_string(),
        }
    }

    /// Type-appropriate placeholder used when a Public domain field has
    /// no declaration-scope initializer (because it was stripped to the
    /// constructor body to take a system-param value). Kotlin requires
    /// every property to be initialized; the placeholder is overwritten
    /// by the `init { }` block.
    fn kotlin_default_for_type(&self, type_str: &str) -> &'static str {
        match type_str {
            "Int" | "Long" | "Short" | "Byte" => "0",
            "Float" | "Double" => "0.0",
            "Boolean" => "false",
            "String" => "\"\"",
            _ => "null",
        }
    }

    /// Emit a single Kotlin class-property declaration line:
    ///   `<indent>[<vis> ][val|var ]<name>[: <type>][ = <init>]\n`
    ///
    /// Type slot:
    /// - present → emit `: <type>` (after `map_type`)
    /// - absent → omit; Kotlin infers from the initializer
    ///
    /// Initializer policy:
    /// - structured `initializer` present → use it verbatim
    /// - missing AND visibility is `Public` AND type is present → emit
    ///   a type-appropriate placeholder (e.g. `0`, `""`) so Kotlin's
    ///   "must be initialized" rule is satisfied; the constructor body
    ///   overwrites with the real value
    /// - missing AND non-`Public` (auto-generated kernel field) → emit
    ///   no init; the `init { }` block performs the assignment
    ///
    /// Visibility `Public` is Kotlin's default and is OMITTED.
    fn emit_field(&self, field: &Field, ctx: &mut EmitContext) -> String {
        let vis = self.emit_visibility_kotlin(field.visibility);
        let var_kw = if field.is_const { "val " } else { "var " };
        let type_str_opt = field.type_annotation.as_ref().map(|t| self.map_type(t));
        let type_suffix = match &type_str_opt {
            Some(t) => format!(": {}", t),
            None => String::new(),
        };
        let init_suffix = match (&field.initializer, field.visibility, &type_str_opt) {
            (Some(init), _, _) => format!(" = {}", self.emit(init, ctx)),
            (None, Visibility::Public, Some(t)) => {
                format!(" = {}", self.kotlin_default_for_type(t))
            }
            (None, _, _) => String::new(),
        };
        if vis.is_empty() {
            format!(
                "{}{}{}{}{}\n",
                ctx.get_indent(),
                var_kw,
                field.name,
                type_suffix,
                init_suffix
            )
        } else {
            format!(
                "{}{} {}{}{}{}\n",
                ctx.get_indent(),
                vis,
                var_kw,
                field.name,
                type_suffix,
                init_suffix
            )
        }
    }
}
