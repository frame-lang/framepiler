//! Java code generation backend

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// Java backend for code generation
pub struct JavaBackend;

/// RFC-0017 Phase A1 helper: replace `new ArrayList<>(java.util.Arrays.asList(<param>...))`
/// with `new ArrayList<>()` if every comma-separated entry in the inner
/// `asList(...)` is a known constructor param name. Used by the Java
/// Constructor arm to strip user-arg-bound enter_args / state_args from
/// the bare `Counter()` body — those are re-supplied by `__frame_init`.
fn java_strip_param_lists(text: &str, param_names: &[&str]) -> String {
    let needle = "new ArrayList<>(java.util.Arrays.asList(";
    let mut result = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(needle_bytes) {
            // Find the matching `)` for the asList call.
            let args_start = i + needle_bytes.len();
            let mut depth = 1;
            let mut j = args_start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            // Now bytes[j] is the closing `)` of asList. The outer
            // ArrayList<>(...) closes at j+1.
            if depth == 0 && j + 1 < bytes.len() && bytes[j + 1] == b')' {
                let inner = &text[args_start..j];
                let parts: Vec<&str> = inner
                    .split(',')
                    .map(|p| p.trim())
                    .filter(|p| !p.is_empty())
                    .collect();
                if !parts.is_empty() && parts.iter().all(|p| param_names.contains(p)) {
                    result.push_str("new ArrayList<>()");
                    i = j + 2; // skip past both closing parens
                    continue;
                }
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

impl LanguageBackend for JavaBackend {
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
                    format!("import {}.*;", module)
                } else {
                    items
                        .iter()
                        .map(|i| format!("import {}.{};", module, i))
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
                visibility,
                ..
            } => {
                let mut result = String::new();
                let vis_kw = match visibility {
                    Visibility::Public => "public ",
                    _ => "",
                };
                let abstract_kw = if *is_abstract { "abstract " } else { "" };
                let extends = if base_classes.is_empty() {
                    String::new()
                } else {
                    format!(" extends {}", base_classes[0])
                };

                result.push_str(&format!(
                    "{}{}{}class {}{} {{\n",
                    ctx.get_indent(),
                    vis_kw,
                    abstract_kw,
                    name,
                    extends
                ));
                ctx.push_indent();

                for field in fields {
                    result.push_str(&self.emit_field(field, ctx));
                }
                if !fields.is_empty() && !methods.is_empty() {
                    result.push('\n');
                }

                // Temporarily expose this class's name as `system_name` so
                // the Constructor arm can distinguish system vs framework
                // helper classes (FrameEvent / FrameContext / Compartment)
                // and apply the RFC-0017 init-decouple split only to the
                // system class.
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
                let mut result = format!("{}public enum {} {{\n", ctx.get_indent(), name);
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
                let vis = self.emit_visibility(*visibility);
                let static_kw = if *is_static { "static " } else { "" };
                let raw_return = self.map_type(return_type.as_ref().unwrap_or(&"void".to_string()));
                // Java has no async keyword — async methods return
                // `CompletableFuture<T>` (or `CompletableFuture<Void>` for
                // void). The interface_gen body wraps results in
                // `CompletableFuture.completedFuture(...)` to match.
                let return_str = if *is_async {
                    if raw_return == "void" {
                        "java.util.concurrent.CompletableFuture<Void>".to_string()
                    } else {
                        // Primitive wrappers map 1:1 via java_map_type already.
                        format!("java.util.concurrent.CompletableFuture<{}>", raw_return)
                    }
                } else {
                    raw_return
                };
                let params_str = self.emit_params(params);

                let mut result = format!(
                    "{}{} {}{} {}({}) {{\n",
                    ctx.get_indent(),
                    vis,
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
                // RFC-0017 Phase A1: split the system constructor into:
                //   public Counter()                       — bare framework
                //   public void __frame_init(<params>)     — user $> + cascade
                //   public static Counter __create(<params>) — factory
                //
                // For framework helper classes (FrameEvent / FrameContext
                // / Compartment), keep the original single-ctor emission —
                // they're not user-facing systems.
                let class_name = ctx.system_name.clone().unwrap_or("Class".to_string());
                let is_frame_helper = class_name.ends_with("FrameEvent")
                    || class_name.ends_with("FrameContext")
                    || class_name.ends_with("Compartment");

                if is_frame_helper {
                    let params_str = self.emit_params(params);
                    let mut result = format!(
                        "{}public {}({}) {{\n",
                        ctx.get_indent(),
                        class_name,
                        params_str
                    );
                    ctx.push_indent();
                    if let Some(sc) = super_call {
                        result.push_str(&format!("{}{};\n", ctx.get_indent(), self.emit(sc, ctx)));
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
                    return result;
                }

                // System class: classify body items.
                //
                // Render each stmt once at body-indent so lines can be
                // reused across both `Counter()` ctor body and the
                // `__frame_init` method body.
                let param_names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                ctx.push_indent();
                let mut framework_lines: Vec<String> = Vec::new();
                let mut frame_init_lines: Vec<String> = Vec::new();
                if let Some(sc) = super_call {
                    framework_lines.push(format!("{}{};", ctx.get_indent(), self.emit(sc, ctx)));
                }
                for stmt in body {
                    let mut rendered = self.emit(stmt, ctx);
                    if self.needs_semicolon(stmt) && !rendered.ends_with(";\n") {
                        if !rendered.ends_with('\n') {
                            rendered.push_str(";\n");
                        } else {
                            let trimmed = rendered.trim_end_matches('\n').to_string();
                            rendered = format!("{};\n", trimmed);
                        }
                    } else if !rendered.ends_with('\n') {
                        rendered.push('\n');
                    }
                    let frame_init_only = rendered.contains("__fire_enter_cascade")
                        || rendered.contains("__process_transition_loop");
                    if frame_init_only {
                        frame_init_lines.push(rendered);
                        continue;
                    }
                    let mentions_param = param_names.iter().any(|p| {
                        rendered
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|w| w == *p)
                    });
                    if mentions_param {
                        frame_init_lines.push(rendered.clone());
                        // RFC-0017: strip handles `Arrays.asList(seed)`
                        // → empty in prepareEnter args. For plain
                        // `this.field = seed` (Domain-kind param)
                        // strip is a no-op; emitting into the no-arg
                        // bare ctor would leave `seed` undefined.
                        // Skip when a param ref survives the strip.
                        let stripped = java_strip_param_lists(&rendered, &param_names);
                        let still_refs_param = param_names.iter().any(|p| {
                            stripped
                                .split(|c: char| !c.is_alphanumeric() && c != '_')
                                .any(|w| w == *p)
                        });
                        if !still_refs_param {
                            framework_lines.push(stripped);
                        }
                    } else {
                        framework_lines.push(rendered);
                    }
                }
                ctx.pop_indent();

                // Emit `public Counter()` — bare framework
                let mut result = format!("{}public {}() {{\n", ctx.get_indent(), class_name);
                ctx.push_indent();
                for line in &framework_lines {
                    result.push_str(line);
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                // Emit `public void __frame_init(<params>)`
                result.push('\n');
                let frame_init_params = self.emit_params(params);
                result.push_str(&format!(
                    "{}public void __frame_init({}) {{\n",
                    ctx.get_indent(),
                    frame_init_params
                ));
                ctx.push_indent();
                for line in &frame_init_lines {
                    result.push_str(line);
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                // Emit `public static Counter __create(<params>)` — factory
                result.push('\n');
                let create_params = self.emit_params(params);
                result.push_str(&format!(
                    "{}public static {} __create({}) {{\n",
                    ctx.get_indent(),
                    class_name,
                    create_params
                ));
                ctx.push_indent();
                result.push_str(&format!(
                    "{}{} c = new {}();\n",
                    ctx.get_indent(),
                    class_name,
                    class_name
                ));
                let arg_pass: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
                result.push_str(&format!(
                    "{}c.__frame_init({});\n",
                    ctx.get_indent(),
                    arg_pass.join(", ")
                ));
                result.push_str(&format!("{}return c;\n", ctx.get_indent()));
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
                let final_kw = if *is_const { "final " } else { "" };
                let type_str =
                    self.map_type(type_annotation.as_ref().unwrap_or(&"var".to_string()));
                if let Some(init_expr) = init {
                    format!(
                        "{}{}{} {} = {}",
                        ctx.get_indent(),
                        final_kw,
                        type_str,
                        name,
                        self.emit(init_expr, ctx)
                    )
                } else {
                    format!("{}{}{} {}", ctx.get_indent(), final_kw, type_str, name)
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
                    "{}for (var {} : {}) {{\n",
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
            CodegenNode::IndexAccess { object, index } => {
                format!("{}.get({})", self.emit(object, ctx), self.emit(index, ctx))
            }
            CodegenNode::SelfRef => "this".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                format!("Arrays.asList({})", elems.join(", "))
            }

            CodegenNode::Dict(pairs) => {
                let pairs_str: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| {
                        format!("Map.entry({}, {})", self.emit(k, ctx), self.emit(v, ctx))
                    })
                    .collect();
                format!("Map.ofEntries({})", pairs_str.join(", "))
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
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({}) -> {}", params_str, self.emit(body, ctx))
            }

            CodegenNode::Cast { expr, target_type } => {
                format!("({}){}", target_type, self.emit(expr, ctx))
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
                format!(
                    "{}this.__transition(new {}Compartment({}))",
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
                    "{}this.__transition(this._state_stack.remove(this._state_stack.size() - 1))",
                    ind
                )
            }
            CodegenNode::StateContext { state_name } => {
                format!("this._stateContext.get(\"{}\")", state_name)
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
        vec!["import java.util.*;".to_string()]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::java()
    }
    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Java
    }
    fn null_keyword(&self) -> &'static str {
        "null"
    }
}

impl JavaBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| {
                let type_ann =
                    self.map_type(p.type_annotation.as_ref().unwrap_or(&"Object".to_string()));
                format!("{} {}", type_ann, p.name)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn emit_visibility(&self, vis: Visibility) -> &'static str {
        match vis {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Protected => "protected",
        }
    }

    /// Emit a single Java field declaration line:
    ///   `<indent><vis> [final ]<type> <name>[ = <init>];\n`
    ///
    /// `final` is emitted only when the field has an initializer at
    /// declaration scope. When the init was stripped (because it
    /// references a constructor param), the constructor body assigns
    /// the field — incompatible with `final`.
    fn emit_field(&self, field: &Field, ctx: &mut EmitContext) -> String {
        let vis = self.emit_visibility(field.visibility);
        let final_kw = if field.is_const && field.initializer.is_some() {
            "final "
        } else {
            ""
        };
        // Route raw Frame type keywords through `map_type` so portable
        // names like `str` / `bool` become native `String` / `boolean`
        // in the emitted Java code, matching Rust / Go / C# / Dart
        // behaviour. Prior code copied the raw token verbatim and
        // relied on the author writing Java-native types directly.
        let raw_type = field
            .type_annotation
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("Object");
        let mapped = self.map_type(raw_type);
        let type_str = mapped.as_str();
        let init_suffix = match &field.initializer {
            Some(init) => format!(" = {}", self.emit(init, ctx)),
            None => String::new(),
        };
        let comments = field.format_leading_comments(&ctx.get_indent());
        format!(
            "{}{}{} {}{} {}{};\n",
            comments,
            ctx.get_indent(),
            vis,
            final_kw,
            type_str,
            field.name,
            init_suffix
        )
    }

    fn map_type(&self, t: &str) -> String {
        match t {
            "Any" => "Object".to_string(),
            "string" | "String" | "str" => "String".to_string(),
            "int" | "i32" | "i64" | "number" => "int".to_string(),
            "float" | "f64" | "f32" => "double".to_string(),
            "bool" | "boolean" => "boolean".to_string(),
            "void" => "void".to_string(),
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
