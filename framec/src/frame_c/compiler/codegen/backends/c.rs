//! C code generation backend

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// C backend for code generation
pub struct CBackend;

impl LanguageBackend for CBackend {
    fn emit(&self, node: &CodegenNode, ctx: &mut EmitContext) -> String {
        let system_name = ctx.system_name.clone().unwrap_or_default();

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

            CodegenNode::Import { module, .. } => format!("#include <{}.h>", module),

            CodegenNode::Class {
                name,
                fields,
                methods,
                ..
            } => {
                let mut result = String::new();

                // Forward declarations for the struct and functions
                result.push_str(&format!("// Forward declarations\n"));
                result.push_str(&format!("typedef struct {} {};\n", name, name));
                result.push_str(&format!(
                    "static void {}_kernel({}* self, {}_FrameEvent* __e);\n",
                    name, name, name
                ));
                result.push_str(&format!(
                    "static void {}_router({}* self, {}_FrameEvent* __e);\n",
                    name, name, name
                ));
                result.push_str(&format!(
                    "static void {}_transition({}* self, {}_Compartment* next);\n",
                    name, name, name
                ));
                // Cascade helper forward declarations (HSM cascade
                // architecture per docs/frame_runtime.md Step 21+).
                result.push_str(&format!(
                    "static int {}_hsm_chain({}* self, const char* leaf, const char*** out_chain);\n",
                    name, name
                ));
                result.push_str(&format!(
                    "static {}_Compartment* {}_prepareEnter({}* self, const char* leaf, {}_FrameVec* state_args, {}_FrameVec* enter_args);\n",
                    name, name, name, name, name
                ));
                result.push_str(&format!(
                    "static void {}_prepareExit({}* self, {}_FrameVec* exit_args);\n",
                    name, name, name
                ));
                result.push_str(&format!(
                    "static void {}_route_to_state({}* self, const char* state_name, {}_FrameEvent* __e, {}_Compartment* compartment);\n",
                    name, name, name, name
                ));
                result.push_str(&format!(
                    "static void {}_fire_enter_cascade({}* self);\n",
                    name, name
                ));
                result.push_str(&format!(
                    "static void {}_fire_exit_cascade({}* self);\n",
                    name, name
                ));
                result.push_str(&format!(
                    "static void {}_process_transition_loop({}* self);\n",
                    name, name
                ));

                // Add forward declarations for state handler methods
                // AND per-handler methods (`_s_<State>_hdl_<kind>_<event>`).
                // Per-handler architecture adds `compartment` as a third arg
                // (see docs/frame_runtime.md § "Dispatch Model").
                for method in methods {
                    if let CodegenNode::Method {
                        name: method_name, ..
                    } = method
                    {
                        if method_name.starts_with("_state_") || method_name.starts_with("_s_") {
                            result.push_str(&format!(
                                "static void {}_{}({}* self, {}_FrameEvent* __e, {}_Compartment* compartment);\n",
                                name,
                                method_name.trim_start_matches('_'),
                                name,
                                name,
                                name
                            ));
                        }
                    }
                }

                // Add forward declarations for actions and operations
                for method in methods {
                    if let CodegenNode::Method {
                        name: method_name,
                        params,
                        return_type,
                        is_static,
                        ..
                    } = method
                    {
                        // Skip state handlers, per-handler methods, kernel,
                        // router, transition (already declared).
                        if method_name.starts_with("_state_")
                            || method_name.starts_with("_s_")
                            || method_name.starts_with("__")
                            || method_name == "new"
                            || method_name == "destroy"
                        {
                            continue;
                        }
                        // Skip interface methods (they get public declarations)
                        // Actions/Operations are not interface methods - they're internal
                        // Check if method is an action or operation by visibility and not being interface
                        let return_str = if return_type.is_none() {
                            "void".to_string()
                        } else {
                            self.convert_type_to_c(return_type, &system_name)
                        };
                        let params_str =
                            self.emit_params_with_self(params, ctx, !*is_static, &system_name);
                        // User-named actions/operations starting with `_`
                        // are emitted `static` in the function definition
                        // (see the Method arm below). The forward declaration
                        // must match — otherwise the C compiler reports
                        // `static declaration follows non-static declaration`.
                        let static_kw = if *is_static || method_name.starts_with('_') {
                            "static "
                        } else {
                            ""
                        };
                        result.push_str(&format!(
                            "{}{} {}_{} ({});\n",
                            static_kw, return_str, name, method_name, params_str
                        ));
                    }
                }
                result.push('\n');

                // Struct definition
                // C struct fields are emitted from parsed field info (name + type).
                result.push_str(&format!("{}struct {} {{\n", ctx.get_indent(), name));
                ctx.push_indent();
                for field in fields {
                    // Cross-system domain reference (`inner: Counter
                    // = @@Counter()`): the assembler emits
                    // `Counter_new()` which returns `Counter*`, so
                    // the field has to be a pointer for the assignment
                    // to type-check. Same shape as the Go fix —
                    // recognized via `ctx.defined_systems`.
                    let raw_type = field.type_annotation.as_deref().unwrap_or("");
                    let c_type = if ctx.defined_systems.contains(raw_type) {
                        format!("{}*", raw_type)
                    } else {
                        self.convert_type_to_c(&field.type_annotation, &system_name)
                    };
                    result.push_str(&format!("{}{} {};\n", ctx.get_indent(), c_type, field.name));
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}};\n\n", ctx.get_indent()));

                // Function declarations and definitions
                for method in methods {
                    result.push_str(&self.emit(method, ctx));
                    result.push('\n');
                }

                // Cross-system call rewrite. Frame source like
                // `self.inner.bump()` is emitted verbatim by the
                // native passthrough path. C structs don't carry
                // methods, so the call has to be lowered to a
                // free-function call: `Inner_bump(self->inner)`.
                // Same handling as Erlang's post-pass at
                // `erlang_system.rs:5538`. Cross-system fields are
                // identified via `field.type_annotation` ∈
                // `ctx.defined_systems`.
                let cross_sys_fields: Vec<(String, String)> = fields
                    .iter()
                    .filter_map(|field| {
                        let raw = field.type_annotation.as_deref().unwrap_or("");
                        if ctx.defined_systems.contains(raw) {
                            Some((field.name.clone(), raw.to_string()))
                        } else {
                            None
                        }
                    })
                    .collect();
                for (field_name, sys_name) in &cross_sys_fields {
                    result = rewrite_c_cross_system_calls(&result, field_name, sys_name);
                }
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}typedef enum {{\n", ctx.get_indent());
                ctx.push_indent();
                for (i, variant) in variants.iter().enumerate() {
                    let comma = if i < variants.len() - 1 { "," } else { "" };
                    result.push_str(&format!(
                        "{}{}_{}{}\n",
                        ctx.get_indent(),
                        name,
                        variant.name,
                        comma
                    ));
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}} {};\n", ctx.get_indent(), name));
                result
            }

            CodegenNode::Method {
                name,
                params,
                return_type,
                body,
                is_static,
                ..
            } => {
                // Convert return type - but for Frame machinery methods with no return type, use void not void*
                let return_str = if return_type.is_none() {
                    "void".to_string()
                } else {
                    self.convert_type_to_c(return_type, &system_name)
                };

                // For Frame system methods, add self parameter
                let is_frame_method = !*is_static && !system_name.is_empty();
                let params_str =
                    self.emit_params_with_self(params, ctx, is_frame_method, &system_name);

                // Method name - prefix with system name for ALL methods in Frame systems
                let func_name = if !system_name.is_empty() {
                    if name.starts_with("__") {
                        // Private methods like __kernel, __router -> System_kernel
                        format!("{}_{}", system_name, name.trim_start_matches('_'))
                    } else if name.starts_with("_state_") || name.starts_with("_s_") {
                        // State handlers like _state_Start -> System_state_Start.
                        // Per-handler emission uses `_s_*`. Both shapes match
                        // the forward-declaration loop that already strips `_`.
                        format!("{}_{}", system_name, name.trim_start_matches('_'))
                    } else {
                        // Public methods, and user-named private methods
                        // (e.g. action `_read`). The leading `_` is part of
                        // the user's name and must be preserved so the
                        // forward declaration (which uses `name` verbatim)
                        // matches the function definition at link time.
                        format!("{}_{}", system_name, name)
                    }
                } else {
                    name.clone()
                };

                let static_kw = if *is_static || name.starts_with("_") {
                    "static "
                } else {
                    ""
                };
                let mut result = format!(
                    "{}{}{} {}({}) {{\n",
                    ctx.get_indent(),
                    static_kw,
                    return_str,
                    func_name,
                    params_str
                );
                ctx.push_indent();

                for stmt in body {
                    let stmt_str = self.emit(stmt, ctx);
                    result.push_str(&stmt_str);
                    // Add semicolon if needed
                    if !stmt_str.trim().is_empty()
                        && !stmt_str.trim().ends_with('}')
                        && !stmt_str.trim().ends_with(';')
                        && !matches!(
                            stmt,
                            CodegenNode::If { .. }
                                | CodegenNode::While { .. }
                                | CodegenNode::For { .. }
                                | CodegenNode::Match { .. }
                                | CodegenNode::Comment { .. }
                                | CodegenNode::NativeBlock { .. }
                                | CodegenNode::Empty
                        )
                    {
                        result.push_str(";\n");
                    } else if !stmt_str.trim().is_empty() {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor { params, body, .. } => {
                let class_name = system_name.clone();
                // Use the C type converter so Frame types like `str` become
                // `char*`, `bool` becomes `bool`, etc. — `emit_params` would
                // pass the raw Frame type through unchanged and break the
                // generated function signature.
                let params_str = if params.is_empty() {
                    "void".to_string()
                } else {
                    params
                        .iter()
                        .map(|p| {
                            let type_str = self.convert_type_to_c(&p.type_annotation, &class_name);
                            format!("{} {}", type_str, p.name)
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };

                let mut result = format!(
                    "{}{}* {}_new({}) {{\n",
                    ctx.get_indent(),
                    class_name,
                    class_name,
                    params_str
                );
                ctx.push_indent();
                // calloc, not malloc: zero-initializes every field so
                // domain/state primitives without an explicit default
                // start at 0/false/NULL instead of holding uninitialized
                // memory. Frame's author-visible contract is "a declared
                // var is usable after construction"; malloc breaks it.
                result.push_str(&format!(
                    "{}{}* self = calloc(1, sizeof({}));\n",
                    ctx.get_indent(),
                    class_name,
                    class_name
                ));

                for stmt in body {
                    let stmt_str = self.emit(stmt, ctx);
                    result.push_str(&stmt_str);
                    if !stmt_str.trim().is_empty()
                        && !stmt_str.trim().ends_with('}')
                        && !stmt_str.trim().ends_with(';')
                        && !matches!(
                            stmt,
                            CodegenNode::If { .. }
                                | CodegenNode::While { .. }
                                | CodegenNode::Comment { .. }
                                | CodegenNode::Empty
                        )
                    {
                        result.push_str(";\n");
                    } else if !stmt_str.trim().is_empty() {
                        result.push('\n');
                    }
                }
                result.push_str(&format!("{}return self;\n", ctx.get_indent()));
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
                let type_str = self.convert_type_to_c(type_annotation, &system_name);
                let const_kw = if *is_const { "const " } else { "" };
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
                    let s = self.emit(stmt, ctx);
                    result.push_str(&s);
                    if !s.trim().is_empty() && !s.trim().ends_with('}') && !s.trim().ends_with(';')
                    {
                        result.push_str(";\n");
                    } else if !s.trim().is_empty() {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();

                if let Some(else_stmts) = else_block {
                    result.push_str(&format!("{}}} else {{\n", ctx.get_indent()));
                    ctx.push_indent();
                    for stmt in else_stmts {
                        let s = self.emit(stmt, ctx);
                        result.push_str(&s);
                        if !s.trim().is_empty()
                            && !s.trim().ends_with('}')
                            && !s.trim().ends_with(';')
                        {
                            result.push_str(";\n");
                        } else if !s.trim().is_empty() {
                            result.push('\n');
                        }
                    }
                    ctx.pop_indent();
                }
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::Match { scrutinee, arms } => {
                // For string comparison, use if-else chain instead of switch
                let scrutinee_str = self.emit(scrutinee, ctx);
                let is_string_match =
                    scrutinee_str.contains("_message") || scrutinee_str.contains("state");

                if is_string_match {
                    let mut result = String::new();
                    for (i, arm) in arms.iter().enumerate() {
                        let cond = if i == 0 { "if" } else { "} else if" };
                        let pattern_str = self.emit(&arm.pattern, ctx);
                        result.push_str(&format!(
                            "{}{} (strcmp({}, {}) == 0) {{\n",
                            ctx.get_indent(),
                            cond,
                            scrutinee_str,
                            pattern_str
                        ));
                        ctx.push_indent();
                        for stmt in &arm.body {
                            let s = self.emit(stmt, ctx);
                            result.push_str(&s);
                            if !s.trim().is_empty()
                                && !s.trim().ends_with('}')
                                && !s.trim().ends_with(';')
                            {
                                result.push_str(";\n");
                            } else if !s.trim().is_empty() {
                                result.push('\n');
                            }
                        }
                        ctx.pop_indent();
                    }
                    result.push_str(&format!("{}}}", ctx.get_indent()));
                    result
                } else {
                    let mut result = format!("{}switch ({}) {{\n", ctx.get_indent(), scrutinee_str);
                    ctx.push_indent();
                    for arm in arms {
                        result.push_str(&format!(
                            "{}case {}:\n",
                            ctx.get_indent(),
                            self.emit(&arm.pattern, ctx)
                        ));
                        ctx.push_indent();
                        for stmt in &arm.body {
                            let s = self.emit(stmt, ctx);
                            result.push_str(&s);
                            if !s.trim().is_empty()
                                && !s.trim().ends_with('}')
                                && !s.trim().ends_with(';')
                            {
                                result.push_str(";\n");
                            } else if !s.trim().is_empty() {
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
            }

            CodegenNode::While { condition, body } => {
                let mut result = format!(
                    "{}while ({}) {{\n",
                    ctx.get_indent(),
                    self.emit(condition, ctx)
                );
                ctx.push_indent();
                for stmt in body {
                    let s = self.emit(stmt, ctx);
                    result.push_str(&s);
                    if !s.trim().is_empty() && !s.trim().ends_with('}') && !s.trim().ends_with(';')
                    {
                        result.push_str(";\n");
                    } else if !s.trim().is_empty() {
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
                body: _,
            } => {
                // C doesn't have for-each, generate a comment
                let mut result = format!(
                    "{}/* for {} in {} */\n",
                    ctx.get_indent(),
                    var,
                    self.emit(iterable, ctx)
                );
                result.push_str(&format!(
                    "{}/* C ForEach: not reachable — Frame uses native passthrough for loops */",
                    ctx.get_indent()
                ));
                result
            }

            CodegenNode::Break => format!("{}break", ctx.get_indent()),
            CodegenNode::Continue => format!("{}continue", ctx.get_indent()),
            CodegenNode::ExprStmt(expr) => format!("{}{}", ctx.get_indent(), self.emit(expr, ctx)),
            CodegenNode::Await(expr) => self.emit(expr, ctx),
            CodegenNode::Comment { text, .. } => format!("{}/* {} */", ctx.get_indent(), text),
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
                // Convert method calls to C function calls
                let obj_str = self.emit(object, ctx);
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();

                // Special handling for common patterns
                if method == "push" || method == "append" {
                    // Convert to FrameVec_push
                    if args_str.is_empty() {
                        format!("{}_FrameVec_push({})", system_name, obj_str)
                    } else {
                        format!(
                            "{}_FrameVec_push({}, {})",
                            system_name,
                            obj_str,
                            args_str.join(", ")
                        )
                    }
                } else if method == "pop" {
                    format!("{}_FrameVec_pop({})", system_name, obj_str)
                } else if method == "copy" {
                    // Compartment copy
                    format!("{}_Compartment_copy({})", system_name, obj_str)
                } else if method == "get" {
                    // Dict get
                    format!(
                        "{}_FrameDict_get({}, {})",
                        system_name,
                        obj_str,
                        args_str.join(", ")
                    )
                } else {
                    // General method call -> function call with object as first arg
                    let all_args = if args_str.is_empty() {
                        obj_str
                    } else {
                        format!("{}, {}", obj_str, args_str.join(", "))
                    };
                    format!("{}({})", method, all_args)
                }
            }

            CodegenNode::FieldAccess { object, field } => {
                let obj_str = self.emit(object, ctx);
                // If object is self or a pointer, use ->
                if obj_str == "self" || obj_str.starts_with("self->") || obj_str.contains("->") {
                    format!("{}->{}", obj_str, field)
                } else {
                    format!("{}.{}", obj_str, field)
                }
            }

            CodegenNode::IndexAccess { object, index } => {
                format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx))
            }
            CodegenNode::SelfRef => "self".to_string(),

            CodegenNode::Array(elements) => {
                if elements.is_empty() {
                    // Empty array initialization - in C we'd initialize to NULL/0
                    "NULL".to_string()
                } else {
                    let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                    format!("{{ {} }}", elems.join(", "))
                }
            }

            CodegenNode::Dict(_) => {
                // Create a new FrameDict
                format!("{}_FrameDict_new()", system_name)
            }

            CodegenNode::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                format!(
                    "({}) ? ({}) : ({})",
                    self.emit(condition, ctx),
                    self.emit(then_expr, ctx),
                    self.emit(else_expr, ctx)
                )
            }

            CodegenNode::Lambda { .. } => "/* Lambda not supported in C */".to_string(),
            CodegenNode::Cast { expr, target_type } => {
                format!("({})({})", target_type, self.emit(expr, ctx))
            }

            CodegenNode::New { class, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                // Convert to C constructor call
                let c_class = if class.contains("Compartment") {
                    format!("{}_Compartment", system_name)
                } else if class.contains("FrameEvent") {
                    format!("{}_FrameEvent", system_name)
                } else if class.contains("FrameContext") {
                    format!("{}_FrameContext", system_name)
                } else {
                    class.clone()
                };
                format!("{}_new({})", c_class, args_str.join(", "))
            }

            // Frame-specific
            CodegenNode::Transition {
                target_state,
                indent,
                ..
            } => {
                let ind = " ".repeat(*indent);
                format!(
                    "{}{}{}_transition(self, {}_Compartment_new(\"{}\"))",
                    ctx.get_indent(),
                    ind,
                    system_name,
                    system_name,
                    target_state
                )
            }
            CodegenNode::ChangeState {
                target_state,
                indent,
                ..
            } => {
                let ind = " ".repeat(*indent);
                format!(
                    "{}{}/* change_state to {} */",
                    ctx.get_indent(),
                    ind,
                    target_state
                )
            }
            CodegenNode::Forward { indent, .. } => {
                let ind = " ".repeat(*indent);
                format!("{}{}return", ctx.get_indent(), ind)
            }
            CodegenNode::StackPush { indent } => {
                let ind = " ".repeat(*indent);
                format!("{}{}{}_FrameVec_push(self->_state_stack, {}_Compartment_copy(self->__compartment))",
                    ctx.get_indent(), ind, system_name, system_name)
            }
            CodegenNode::StackPop { indent } => {
                let ind = " ".repeat(*indent);
                format!(
                    "{}{}{}_FrameVec_pop(self->_state_stack)",
                    ctx.get_indent(),
                    ind,
                    system_name
                )
            }
            CodegenNode::StateContext { state_name } => {
                format!("/* state context for {} */", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}{}(self)", ctx.get_indent(), event)
                } else {
                    format!(
                        "{}{}(self, {})",
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
            CodegenNode::SplicePoint { id } => format!("/* SPLICE_POINT: {} */", id),
        }
    }

    fn runtime_imports(&self) -> Vec<String> {
        // Runtime imports are now included in generate_c_compartment_types
        vec![]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::c()
    }
    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::C
    }

    fn null_keyword(&self) -> &'static str {
        "NULL"
    }
    fn true_keyword(&self) -> &'static str {
        "true"
    }
    fn false_keyword(&self) -> &'static str {
        "false"
    }
}

impl CBackend {
    fn emit_params(&self, params: &[Param], _ctx: &EmitContext) -> String {
        if params.is_empty() {
            "void".to_string()
        } else {
            params
                .iter()
                .map(|p| {
                    let type_ann = p
                        .type_annotation
                        .as_ref()
                        .unwrap_or(&"int".to_string())
                        .clone();
                    format!("{} {}", type_ann, p.name)
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    fn emit_params_with_self(
        &self,
        params: &[Param],
        _ctx: &EmitContext,
        add_self: bool,
        system_name: &str,
    ) -> String {
        let mut result = Vec::new();

        if add_self && !system_name.is_empty() {
            result.push(format!("{}* self", system_name));
        }

        for p in params {
            let type_str = self.convert_type_to_c(&p.type_annotation, system_name);
            result.push(format!("{} {}", type_str, p.name));
        }

        if result.is_empty() {
            "void".to_string()
        } else {
            result.join(", ")
        }
    }

    /// Convert Frame/Python/TypeScript types to C types
    fn convert_type_to_c(&self, type_ann: &Option<String>, system_name: &str) -> String {
        match type_ann.as_ref().map(|s| s.as_str()) {
            None => "void*".to_string(),
            Some("void") | Some("None") => "void".to_string(),
            Some("bool") | Some("boolean") => "bool".to_string(),
            Some("int") | Some("number") | Some("Any") => "int".to_string(), // Default Any to int for C
            Some("float") | Some("double") => "double".to_string(),
            Some("str") | Some("string") | Some("String") => "char*".to_string(),
            Some("list") | Some("List") | Some("Array") | Some("Array<any>") => {
                format!("{}_FrameVec*", system_name)
            }
            Some("dict") | Some("Dict") | Some("Record<string, any>") => {
                format!("{}_FrameDict*", system_name)
            }
            Some(t) if t.contains("Compartment") => {
                format!("{}_Compartment*", system_name)
            }
            Some(t) if t.contains("FrameEvent") => {
                format!("{}_FrameEvent*", system_name)
            }
            Some(t) if t.contains("FrameContext") => {
                format!("{}_FrameContext*", system_name)
            }
            Some(t) if t.ends_with("| null") || t.ends_with("| None") => {
                // Optional type - just use the base type (will be pointer)
                let base = match t.split('|').next() {
                    Some(b) => b.trim(),
                    None => t.trim(),
                };
                self.convert_type_to_c(&Some(base.to_string()), system_name)
            }
            Some(other) => other.to_string(),
        }
    }
}

/// Rewrite `self[.|->]FIELD[.|->]METHOD(ARGS)` in `code` to
/// `<sys_name>_METHOD(self->FIELD[, ARGS])`. C structs don't carry
/// methods, so cross-system dot-call sites have to be lowered to
/// free-function calls. Mirrors Erlang's post-pass at
/// `erlang_system.rs:5538`.
fn rewrite_c_cross_system_calls(code: &str, field_name: &str, sys_name: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    let bytes = code.as_bytes();
    while cursor < bytes.len() {
        let rel = match code[cursor..].find("self") {
            Some(r) => r,
            None => break,
        };
        let abs = cursor + rel;
        // Token-boundary: previous char must not be an identifier char.
        if abs > 0 {
            let prev = bytes[abs - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                out.push_str(&code[cursor..abs + 4]);
                cursor = abs + 4;
                continue;
            }
        }
        let after_self = abs + 4;
        // After `self`, expect `.` or `->`.
        let after_sep1 = if code[after_self..].starts_with("->") {
            after_self + 2
        } else if code[after_self..].starts_with('.') {
            after_self + 1
        } else {
            out.push_str(&code[cursor..after_self]);
            cursor = after_self;
            continue;
        };
        // Match the field name.
        if !code[after_sep1..].starts_with(field_name) {
            out.push_str(&code[cursor..after_self]);
            cursor = after_self;
            continue;
        }
        let field_end = after_sep1 + field_name.len();
        // Token-boundary: next char must not be an identifier char.
        if field_end < bytes.len() {
            let nx = bytes[field_end];
            if nx.is_ascii_alphanumeric() || nx == b'_' {
                out.push_str(&code[cursor..after_self]);
                cursor = after_self;
                continue;
            }
        }
        // After the field, expect `.` or `->`.
        let after_sep2 = if code[field_end..].starts_with("->") {
            field_end + 2
        } else if code[field_end..].starts_with('.') {
            field_end + 1
        } else {
            out.push_str(&code[cursor..after_self]);
            cursor = after_self;
            continue;
        };
        // Match a method identifier.
        let method_start = after_sep2;
        let mut method_end = method_start;
        while method_end < bytes.len()
            && (bytes[method_end].is_ascii_alphanumeric() || bytes[method_end] == b'_')
        {
            method_end += 1;
        }
        if method_end == method_start || method_end >= bytes.len() || bytes[method_end] != b'(' {
            out.push_str(&code[cursor..after_self]);
            cursor = after_self;
            continue;
        }
        let method = &code[method_start..method_end];
        // Find the matching `)` — paren-balance, ignore strings/chars
        // for now (C call args rarely contain unbalanced ones; matches
        // Erlang precedent).
        let args_open = method_end;
        let mut depth: i32 = 1;
        let mut p = args_open + 1;
        while p < bytes.len() && depth > 0 {
            match bytes[p] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            p += 1;
        }
        if depth != 0 {
            out.push_str(&code[cursor..p.min(bytes.len())]);
            cursor = p;
            continue;
        }
        let args_inner = &code[args_open + 1..p - 1];
        let args_inner_trim = args_inner.trim();
        out.push_str(&code[cursor..abs]);
        if args_inner_trim.is_empty() {
            out.push_str(&format!("{}_{}(self->{})", sys_name, method, field_name));
        } else {
            out.push_str(&format!(
                "{}_{}(self->{}, {})",
                sys_name, method, field_name, args_inner
            ));
        }
        cursor = p;
    }
    out.push_str(&code[cursor..]);
    out
}
