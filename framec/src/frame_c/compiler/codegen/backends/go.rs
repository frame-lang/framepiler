//! Go code generation backend
//!
//! Go is structurally different from other backends:
//! - No classes — uses structs with method receivers
//! - No constructors — uses factory functions (NewXxx)
//! - No `this`/`self` — uses explicit receiver parameter (s *StructName)
//! - No semicolons — newline as statement terminator
//! - Type after name in declarations: `name Type` (not `Type name`)
//! - Visibility via capitalization (uppercase = exported)
//! - No generics in runtime types — uses `any` (interface{})

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// Go backend for code generation
pub struct GoBackend;

/// RFC-0017 Phase A2 helper: replace `[]any{<param>(, <param>)*}` with
/// `[]any{}` if every comma-separated entry is a known constructor
/// param name. Used by the Go Constructor arm to strip user-arg-bound
/// enter_args / state_args from the bare `NewCounter()` body — those
/// are re-supplied by `__frame_init`.
fn go_strip_param_lists(text: &str, param_names: &[&str]) -> String {
    let needle = "[]any{";
    let mut result = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(needle_bytes) {
            let args_start = i + needle_bytes.len();
            let mut depth = 1;
            let mut j = args_start;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            if depth == 0 {
                let inner = &text[args_start..j];
                let parts: Vec<&str> = inner
                    .split(',')
                    .map(|p| p.trim())
                    .filter(|p| !p.is_empty())
                    .collect();
                if !parts.is_empty() && parts.iter().all(|p| param_names.contains(p)) {
                    result.push_str("[]any{}");
                    i = j + 1;
                    continue;
                }
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

impl LanguageBackend for GoBackend {
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

            CodegenNode::Import { module, .. } => {
                format!("import \"{}\"", module)
            }

            CodegenNode::Class {
                name,
                fields,
                methods,
                ..
            } => {
                // Go: emit struct definition + separate method definitions
                let mut result = String::new();

                // Struct definition
                result.push_str(&format!("{}type {} struct {{\n", ctx.get_indent(), name));
                ctx.push_indent();
                for field in fields {
                    result.push_str(&self.emit_field(field, ctx));
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                // Method definitions (outside struct)
                for method in methods {
                    result.push('\n');
                    result.push_str(&self.emit(method, ctx));
                }
                result
            }

            CodegenNode::Enum { name, variants } => {
                // Go: use const iota pattern
                let mut result = format!("{}type {} int\n\n", ctx.get_indent(), name);
                result.push_str(&format!("{}const (\n", ctx.get_indent()));
                ctx.push_indent();
                for (i, variant) in variants.iter().enumerate() {
                    if i == 0 {
                        result.push_str(&format!(
                            "{}{}_{} {} = iota\n",
                            ctx.get_indent(),
                            name,
                            variant.name,
                            name
                        ));
                    } else {
                        result.push_str(&format!(
                            "{}{}_{}\n",
                            ctx.get_indent(),
                            name,
                            variant.name
                        ));
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{})\n", ctx.get_indent()));
                result
            }

            CodegenNode::Method {
                name,
                params,
                return_type,
                body,
                is_static,
                visibility,
                ..
            } => {
                // Go: static methods become package-level functions
                // Instance methods use receiver: func (s *ClassName) name(...)
                let go_name = match visibility {
                    Visibility::Public => capitalize_first(name),
                    _ => name.clone(),
                };

                let params_str = self.emit_params(params);
                let return_str = if let Some(rt) = return_type {
                    let mapped = self.map_type(rt);
                    if mapped.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", mapped)
                    }
                } else {
                    String::new()
                };

                let mut result = if *is_static || system_name.is_empty() {
                    // Package-level function
                    format!(
                        "{}func {}({}){} {{\n",
                        ctx.get_indent(),
                        go_name,
                        params_str,
                        return_str
                    )
                } else {
                    // Method with receiver
                    format!(
                        "{}func (s *{}) {}({}){} {{\n",
                        ctx.get_indent(),
                        system_name,
                        go_name,
                        params_str,
                        return_str
                    )
                };

                ctx.push_indent();
                for stmt in body {
                    let stmt_str = self.emit(stmt, ctx);
                    result.push_str(&stmt_str);
                    if !stmt_str.trim().is_empty() {
                        result.push('\n');
                    }
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor { params, body, .. } => {
                // RFC-0020: two artifacts — bare NewFoo() + CreateFoo()
                // factory. The intermediate `__frame_init` member that
                // RFC-0017 emitted is gone — its body is absorbed
                // inline into `CreateFoo` with receiver `s.` rewritten
                // to local `c.` (since the factory is a package-level
                // function with no receiver).
                //
                // Call-site lowering:
                //   - `@@Counter(7)` → `CreateCounter(7)`
                //   - `@@!Counter()` → `NewCounter()`
                //
                // Body classification:
                //   - Lines that touch the kernel, set `__compartment`,
                //     reference `_context_stack`, or mention any system
                //     parameter go to `frame_init_lines` (absorbed into
                //     the factory).
                //   - Other statements stay in `NewFoo()` so
                //     `@@!Foo()` produces a usable-but-not-entered
                //     instance.
                let class_name = system_name.clone();
                let param_names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();

                ctx.push_indent();
                let mut framework_lines: Vec<String> = Vec::new();
                let mut frame_init_lines: Vec<String> = Vec::new();
                for stmt in body {
                    let mut rendered = self.emit(stmt, ctx);
                    if !rendered.ends_with('\n') && !rendered.is_empty() {
                        rendered.push('\n');
                    }
                    // Scope to kernel call + context-stack mutation.
                    // The compartment-init line (`s.__compartment =
                    // s.__prepareEnter(...)`) must run in the bare ctor
                    // too so `@@!Foo()` shells are usable (with empty-args
                    // compartment); when it mentions params the
                    // mentions_param branch below handles the split.
                    let frame_init_only = rendered.contains("__kernel(")
                        || rendered.contains("_context_stack = append(")
                        || rendered.contains("_context_stack = s._context_stack[:");
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
                        // RFC-0017 carry-over: a stripped version
                        // (param-list args replaced with empty slice
                        // literal) goes in NewFoo() too, so `@@!Foo()`
                        // still ends up with a valid (empty-args)
                        // compartment. Skip the stripped form if a
                        // param ref survives (`s.field = seed` strip
                        // is a no-op and would not compile).
                        let stripped = go_strip_param_lists(&rendered, &param_names);
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

                // Emit `func NewCounter() *Counter` — bare framework
                let mut result = format!(
                    "{}func New{}() *{} {{\n",
                    ctx.get_indent(),
                    class_name,
                    class_name
                );
                ctx.push_indent();
                result.push_str(&format!("{}s := &{}{{}}\n", ctx.get_indent(), class_name));
                for line in &framework_lines {
                    result.push_str(line);
                }
                result.push_str(&format!("{}return s\n", ctx.get_indent()));
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                // Emit `func CreateCounter(<params>) *Counter` — factory.
                // Absorb the frame_init body inline, rewriting receiver
                // references from `s.` to `c.` (since the factory is
                // a package-level function whose system instance is the
                // local `c`).
                result.push('\n');
                let create_params = self.emit_params(params);
                result.push_str(&format!(
                    "{}func Create{}({}) *{} {{\n",
                    ctx.get_indent(),
                    class_name,
                    create_params,
                    class_name
                ));
                ctx.push_indent();
                result.push_str(&format!("{}c := New{}()\n", ctx.get_indent(), class_name));
                for line in &frame_init_lines {
                    let rewritten = rewrite_go_member_refs_for_factory(line);
                    result.push_str(&rewritten);
                }
                result.push_str(&format!("{}return c\n", ctx.get_indent()));
                ctx.pop_indent();
                result.push_str(&format!("{}}}\n", ctx.get_indent()));

                result
            }

            CodegenNode::VarDecl {
                name,
                type_annotation,
                init,
                is_const: _,
            } => {
                if let Some(init_expr) = init {
                    // Short declaration with init
                    format!(
                        "{}{} := {}",
                        ctx.get_indent(),
                        name,
                        self.emit(init_expr, ctx)
                    )
                } else if let Some(ref type_ann) = type_annotation {
                    // Declaration without init — use var
                    format!(
                        "{}var {} {}",
                        ctx.get_indent(),
                        name,
                        self.map_type(type_ann)
                    )
                } else {
                    format!("{}var {} any", ctx.get_indent(), name)
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
                let mut result =
                    format!("{}if {} {{\n", ctx.get_indent(), self.emit(condition, ctx));
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
                let mut result = format!(
                    "{}switch {} {{\n",
                    ctx.get_indent(),
                    self.emit(scrutinee, ctx)
                );
                for arm in arms {
                    result.push_str(&format!(
                        "{}case {}:\n",
                        ctx.get_indent(),
                        self.emit(&arm.pattern, ctx)
                    ));
                    ctx.push_indent();
                    for stmt in &arm.body {
                        result.push_str(&self.emit(stmt, ctx));
                        result.push('\n');
                    }
                    ctx.pop_indent();
                }
                result.push_str(&format!("{}}}", ctx.get_indent()));
                result
            }

            CodegenNode::While { condition, body } => {
                let mut result =
                    format!("{}for {} {{\n", ctx.get_indent(), self.emit(condition, ctx));
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
                let mut result = format!(
                    "{}for _, {} := range {} {{\n",
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
                format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx))
            }

            CodegenNode::SelfRef => "s".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                if elems.is_empty() {
                    "[]any{}".to_string()
                } else {
                    format!("[]any{{{}}}", elems.join(", "))
                }
            }

            CodegenNode::Dict(pairs) => {
                if pairs.is_empty() {
                    "map[string]any{}".to_string()
                } else {
                    let pairs_str: Vec<String> = pairs
                        .iter()
                        .map(|(k, v)| format!("{}: {}", self.emit(k, ctx), self.emit(v, ctx)))
                        .collect();
                    format!("map[string]any{{{}}}", pairs_str.join(", "))
                }
            }

            CodegenNode::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                // Go has no ternary — use inline func
                format!(
                    "func() any {{ if {} {{ return {} }}; return {} }}()",
                    self.emit(condition, ctx),
                    self.emit(then_expr, ctx),
                    self.emit(else_expr, ctx)
                )
            }

            CodegenNode::Lambda { params, body } => {
                let params_str = params
                    .iter()
                    .map(|p| format!("{} any", p.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "func({}) any {{ return {} }}",
                    params_str,
                    self.emit(body, ctx)
                )
            }

            CodegenNode::Cast { expr, target_type } => {
                // Go type assertion
                format!("{}.({})", self.emit(expr, ctx), target_type)
            }

            CodegenNode::New { class, args } => {
                // Go: &ClassName{} or NewClassName(args)
                if args.is_empty() {
                    format!("&{}{{}}", class)
                } else {
                    let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                    format!("New{}({})", class, args_str.join(", "))
                }
            }

            // Frame-specific nodes
            CodegenNode::Transition {
                target_state,
                indent,
                ..
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!(
                    "{}s.__transition(&{}Compartment{{state: \"{}\"}})",
                    ind, system_name, target_state
                )
            }
            CodegenNode::ChangeState {
                target_state,
                indent,
                ..
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}// change_state to {}", ind, target_state)
            }
            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }
            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!(
                    "{}s._state_stack = append(s._state_stack, s.__compartment.copy())",
                    ind
                )
            }
            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}__popped := s._state_stack[len(s._state_stack)-1]\n{}s._state_stack = s._state_stack[:len(s._state_stack)-1]\n{}s.__transition(__popped)",
                    ind, ind, ind)
            }
            CodegenNode::StateContext { state_name } => {
                format!("s._state_context[\"{}\"]", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}s.{}()", ctx.get_indent(), event)
                } else {
                    format!("{}s.{}({})", ctx.get_indent(), event, args_str.join(", "))
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

    fn emit_module_imports(
        &self,
        imports: &[crate::frame_c::compiler::frame_ast::Import],
    ) -> Vec<String> {
        // RFC-0022 Phase 1 lax — Go modules forbid relative imports;
        // a real Go `import "<modpath>/<pkg>"` requires the importer
        // and importee to live in a Go module with a known module
        // path. Phase 1 emits a comment marker; Phase 2 strict will
        // accept a `--go-module-root <path>` option to resolve to
        // proper module-relative imports.
        imports
            .iter()
            .filter_map(|imp| {
                let path = imp.module.as_str();
                if path.is_empty() {
                    return None;
                }
                Some(format!(
                    "// RFC-0022 lax: import \"{}\" — Phase 2 strict will emit `import \"<modpath>/...\"`",
                    imp.module
                ))
            })
            .collect()
    }

    fn runtime_imports(&self) -> Vec<String> {
        // Go manages imports per-file; runtime types are emitted inline
        vec![]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::go()
    }
    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Go
    }
    fn null_keyword(&self) -> &'static str {
        "nil"
    }
}

impl GoBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| {
                let type_ann =
                    self.map_type(p.type_annotation.as_ref().unwrap_or(&"any".to_string()));
                format!("{} {}", p.name, type_ann)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn map_type(&self, t: &str) -> String {
        match t {
            "Any" | "object" | "Object" => "any".to_string(),
            "string" | "String" | "str" => "string".to_string(),
            "int" | "i32" | "i64" | "number" => "int".to_string(),
            "float" | "f64" | "f32" => "float64".to_string(),
            "bool" | "boolean" => "bool".to_string(),
            "void" | "None" => String::new(), // Go uses no return type for void
            other => other.to_string(),
        }
    }

    /// Emit a single Go struct field declaration line:
    ///   `<indent><name> <type>\n`
    ///
    /// Go has no field-level visibility keyword (capitalization
    /// handles export), no field-level init (factory functions assign),
    /// and no field-level const. The type is taken verbatim — matching
    /// the existing `synthesize_field_raw` behavior which does not apply
    /// `map_type` to the user-supplied type string.
    fn emit_field(&self, field: &Field, ctx: &mut EmitContext) -> String {
        // Apply Frame-type → Go-type mapping so domain fields declared
        // with the portable `str` / `int` / `bool` keywords compile.
        // Without this, `s: str = ""` produces `s str` (undefined type).
        let raw_type = field
            .type_annotation
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("any");
        let mut type_str = self.map_type(raw_type);
        // Cross-system domain reference (`inner: Counter = @@Counter()`):
        // Go's `New<System>()` constructor returns `*<System>`, so the
        // field has to be a pointer for the assignment to type-check.
        if ctx.defined_systems.contains(raw_type) {
            type_str = format!("*{}", type_str);
        }
        let comments = field.format_leading_comments(&ctx.get_indent());
        format!(
            "{}{}{} {}\n",
            comments,
            ctx.get_indent(),
            field.name,
            type_str
        )
    }
}

/// Capitalize the first letter of a string (for Go export visibility)
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Rewrite member references in absorbed frame-init lines so they
/// target a local `c` instead of a receiver `s`.
///
/// The bare-ctor body lines are rendered against the `s` receiver
/// (e.g. `s.__compartment = s.__prepareEnter(...)`,
/// `s._context_stack = append(s._context_stack, __ctx)`). When moved
/// into the package-level `CreateFoo` factory, every `s.` prefix
/// must become `c.`. Word-boundary aware so identifiers ending in
/// `s.` (rare) aren't touched.
fn rewrite_go_member_refs_for_factory(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut result = String::with_capacity(line.len() + 16);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b's'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'.'
            && (i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
        {
            result.push_str("c.");
            i += 2;
            continue;
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}
