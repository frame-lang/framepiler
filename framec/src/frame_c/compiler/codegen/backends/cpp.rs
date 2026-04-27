//! C++23 code generation backend

use crate::frame_c::compiler::codegen::ast::*;
use crate::frame_c::compiler::codegen::backend::*;
use crate::frame_c::visitors::TargetLanguage;

/// Coroutine promise-type emitted at file scope before any async Frame
/// class. Header-guarded so a file with multiple `@@system`s (each
/// potentially async) only declares the template once.
const FRAME_TASK_PRELUDE: &str = r#"#ifndef FRAME_TASK_H
#define FRAME_TASK_H
#include <coroutine>
#include <exception>
#include <utility>

template <typename T>
struct FrameTask {
    struct promise_type {
        T value_{};
        std::exception_ptr err_;
        FrameTask get_return_object() noexcept { return FrameTask{std::coroutine_handle<promise_type>::from_promise(*this)}; }
        std::suspend_never initial_suspend() noexcept { return {}; }
        std::suspend_always final_suspend() noexcept { return {}; }
        template <typename U> void return_value(U&& v) { value_ = std::forward<U>(v); }
        void unhandled_exception() noexcept { err_ = std::current_exception(); }
    };
    std::coroutine_handle<promise_type> h_;
    explicit FrameTask(std::coroutine_handle<promise_type> h) noexcept : h_(h) {}
    FrameTask(const FrameTask&) = delete;
    FrameTask& operator=(const FrameTask&) = delete;
    FrameTask(FrameTask&& o) noexcept : h_(std::exchange(o.h_, {})) {}
    FrameTask& operator=(FrameTask&& o) noexcept { if (this != &o) { if (h_) h_.destroy(); h_ = std::exchange(o.h_, {}); } return *this; }
    ~FrameTask() { if (h_) h_.destroy(); }
    bool await_ready() const noexcept { return h_.done(); }
    void await_suspend(std::coroutine_handle<>) const noexcept {}
    T await_resume() { if (h_.promise().err_) std::rethrow_exception(h_.promise().err_); return std::move(h_.promise().value_); }
    T get() { if (h_.promise().err_) std::rethrow_exception(h_.promise().err_); return std::move(h_.promise().value_); }
};

template <>
struct FrameTask<void> {
    struct promise_type {
        std::exception_ptr err_;
        FrameTask get_return_object() noexcept { return FrameTask{std::coroutine_handle<promise_type>::from_promise(*this)}; }
        std::suspend_never initial_suspend() noexcept { return {}; }
        std::suspend_always final_suspend() noexcept { return {}; }
        void return_void() noexcept {}
        void unhandled_exception() noexcept { err_ = std::current_exception(); }
    };
    std::coroutine_handle<promise_type> h_;
    explicit FrameTask(std::coroutine_handle<promise_type> h) noexcept : h_(h) {}
    FrameTask(const FrameTask&) = delete;
    FrameTask& operator=(const FrameTask&) = delete;
    FrameTask(FrameTask&& o) noexcept : h_(std::exchange(o.h_, {})) {}
    FrameTask& operator=(FrameTask&& o) noexcept { if (this != &o) { if (h_) h_.destroy(); h_ = std::exchange(o.h_, {}); } return *this; }
    ~FrameTask() { if (h_) h_.destroy(); }
    bool await_ready() const noexcept { return h_.done(); }
    void await_suspend(std::coroutine_handle<>) const noexcept {}
    void await_resume() { if (h_.promise().err_) std::rethrow_exception(h_.promise().err_); }
    void get() { if (h_.promise().err_) std::rethrow_exception(h_.promise().err_); }
};
#endif // FRAME_TASK_H

"#;

/// Rewrite every `return;` / `return expr;` in an async-method body to
/// `co_return;` / `co_return expr;`. The existing state-dispatch and
/// frame-expansion emitters sprinkle plain `return`s in transition/forward
/// code paths (~20 sites across the codebase), and each is synchronous
/// when emitted. Inside a C++ coroutine, plain `return` is ill-formed —
/// the compiler refuses to mix the two. Post-processing the body string
/// keeps the upstream emitters language-agnostic.
///
/// Word-boundary match on `return` (leading whitespace OK, not inside an
/// identifier) — we don't want to clobber `returned`, string literals, or
/// a `// return` comment. This walks lines, checks the trimmed prefix,
/// and preserves indentation and trailing content (e.g. `return __result;`).
fn rewrite_return_to_co_return(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 64);
    for line in body.split_inclusive('\n') {
        let trimmed_start = line.trim_start();
        let indent_len = line.len() - trimmed_start.len();
        // Bail if this isn't a `return` statement. Comment lines
        // (`// ...`) already leave `return` inside a non-statement
        // position, so a strict prefix check is safe.
        let bare_return = trimmed_start == "return;" || trimmed_start == "return;\n";
        let return_with_val = trimmed_start.starts_with("return ")
            && !trimmed_start.starts_with("return_")  // defensive: no identifier collision
            && !trimmed_start.starts_with("returns");
        if bare_return || return_with_val {
            out.push_str(&line[..indent_len]);
            out.push_str("co_");
            out.push_str(trimmed_start);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// C++23 backend for code generation
pub struct CppBackend;

impl LanguageBackend for CppBackend {
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

            CodegenNode::Import { module, .. } => format!("#include <{}>", module),

            CodegenNode::Class {
                name,
                fields,
                methods,
                base_classes,
                is_abstract: _,
                ..
            } => {
                let mut result = String::new();
                let extends = if base_classes.is_empty() {
                    String::new()
                } else {
                    format!(" : public {}", base_classes.join(", public "))
                };

                // C++ async: if any method is async, emit a self-contained
                // `FrameTask<T>` coroutine promise at file scope before the
                // class. `#ifndef FRAME_TASK_H` guards multi-@@system
                // files (e.g. 33_ai_agent) from re-declaring the template.
                //
                // Model: `initial_suspend` is `suspend_never` (bodies run
                // synchronously from creation to the first real await or
                // `co_return`), and `final_suspend` is `suspend_always` so
                // the caller can extract the return value before the
                // handle is destroyed. This matches Frame's semantics:
                // the dispatch chain has no true async I/O — `co_await`
                // just threads return values.
                let has_async = methods
                    .iter()
                    .any(|m| matches!(m, CodegenNode::Method { is_async: true, .. }));
                if has_async {
                    result.push_str(FRAME_TASK_PRELUDE);
                }

                result.push_str(&format!(
                    "{}class {}{} {{\n",
                    ctx.get_indent(),
                    name,
                    extends
                ));

                // Group fields and methods by visibility using sections
                // Private section first (fields + methods)
                let private_fields: Vec<_> = fields
                    .iter()
                    .filter(|f| !matches!(f.visibility, Visibility::Public))
                    .collect();
                let private_methods: Vec<_> = methods
                    .iter()
                    .filter(|m| {
                        if let CodegenNode::Method { visibility, .. } = m {
                            !matches!(visibility, Visibility::Public)
                        } else {
                            false
                        }
                    })
                    .collect();

                if !private_fields.is_empty() || !private_methods.is_empty() {
                    result.push_str("private:\n");
                    ctx.push_indent();
                    for field in &private_fields {
                        result.push_str(&self.emit_field(field, ctx));
                    }
                    if !private_fields.is_empty() && !private_methods.is_empty() {
                        result.push('\n');
                    }
                    for (i, method) in private_methods.iter().enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(&self.emit(method, ctx));
                    }
                    ctx.pop_indent();
                    result.push('\n');
                }

                // Public section (fields + constructor + methods)
                let public_fields: Vec<_> = fields
                    .iter()
                    .filter(|f| matches!(f.visibility, Visibility::Public))
                    .collect();
                let public_methods: Vec<_> = methods
                    .iter()
                    .filter(|m| {
                        matches!(m, CodegenNode::Constructor { .. })
                            || matches!(
                                m,
                                CodegenNode::Method {
                                    visibility: Visibility::Public,
                                    ..
                                }
                            )
                            // NativeBlock at class scope is used for class-
                            // static declarations (e.g. the `@@persist`
                            // __skipInitialEnter flag) that don't fit the
                            // Method/Field/Constructor node shapes. Emit
                            // them alongside public members.
                            || matches!(m, CodegenNode::NativeBlock { .. })
                    })
                    .collect();

                if !public_fields.is_empty() || !public_methods.is_empty() {
                    result.push_str("public:\n");
                    ctx.push_indent();
                    for field in &public_fields {
                        result.push_str(&self.emit_field(field, ctx));
                    }
                    if !public_fields.is_empty() && !public_methods.is_empty() {
                        result.push('\n');
                    }
                    for (i, method) in public_methods.iter().enumerate() {
                        if i > 0 {
                            result.push('\n');
                        }
                        result.push_str(&self.emit(method, ctx));
                    }
                    ctx.pop_indent();
                }

                result.push_str(&format!("{}}};\n", ctx.get_indent()));
                result
            }

            CodegenNode::Enum { name, variants } => {
                let mut result = format!("{}enum class {} {{\n", ctx.get_indent(), name);
                ctx.push_indent();
                for (i, variant) in variants.iter().enumerate() {
                    let comma = if i < variants.len() - 1 { "," } else { "" };
                    result.push_str(&format!("{}{}{}\n", ctx.get_indent(), variant.name, comma));
                }
                ctx.pop_indent();
                result.push_str(&format!("{}}};\n", ctx.get_indent()));
                result
            }

            CodegenNode::Method {
                name,
                params,
                return_type,
                body,
                is_async,
                is_static,
                visibility: _,
                ..
            } => {
                let static_kw = if *is_static { "static " } else { "" };
                let raw_return = self.map_type(return_type.as_ref().unwrap_or(&"void".to_string()));
                // C++23 async: methods marked async become coroutines
                // returning `FrameTask<T>`. Void methods return `FrameTask<void>`.
                // Bodies must `co_return` (or `co_return value`) — the
                // interface_gen Cpp arm emits `co_return` directly, and
                // any transition-level `return;` / `return expr;` left in
                // the rest of the body is rewritten after emission below
                // (mixing `return` with `co_return` / `co_await` is a
                // hard compile error in a coroutine).
                let return_str = if *is_async {
                    format!("FrameTask<{}>", raw_return)
                } else {
                    raw_return
                };
                let params_str = self.emit_params(params);

                let mut body_str = String::new();
                ctx.push_indent();
                for stmt in body {
                    body_str.push_str(&self.emit(stmt, ctx));
                    if self.needs_semicolon(stmt) {
                        body_str.push_str(";\n");
                    } else {
                        body_str.push('\n');
                    }
                }
                ctx.pop_indent();

                if *is_async {
                    body_str = rewrite_return_to_co_return(&body_str);
                }

                let mut result = format!(
                    "{}{}{} {}({}) {{\n",
                    ctx.get_indent(),
                    static_kw,
                    return_str,
                    name,
                    params_str
                );
                result.push_str(&body_str);
                result.push_str(&format!("{}}}\n", ctx.get_indent()));
                result
            }

            CodegenNode::Constructor {
                params,
                body,
                super_call,
            } => {
                let class_name = ctx.system_name.clone().unwrap_or("Class".to_string());
                let params_str = self.emit_params(params);
                let init_list = super_call
                    .as_ref()
                    .map(|sc| format!(" : {}", self.emit(sc, ctx)))
                    .unwrap_or_default();

                let mut result = format!(
                    "{}{}({}){} {{\n",
                    ctx.get_indent(),
                    class_name,
                    params_str,
                    init_list
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

            CodegenNode::VarDecl {
                name,
                type_annotation,
                init,
                is_const,
            } => {
                let const_kw = if *is_const { "const " } else { "" };
                let type_str = type_annotation
                    .as_ref()
                    .unwrap_or(&"auto".to_string())
                    .clone();
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
                    "{}for (auto {} : {}) {{\n",
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
            CodegenNode::Await(expr) => {
                format!("co_await {}", self.emit(expr, ctx))
            }
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
                    "{}->{}({})",
                    self.emit(object, ctx),
                    method,
                    args_str.join(", ")
                )
            }

            CodegenNode::FieldAccess { object, field } => {
                format!("{}->{}", self.emit(object, ctx), field)
            }
            CodegenNode::IndexAccess { object, index } => {
                format!("{}[{}]", self.emit(object, ctx), self.emit(index, ctx))
            }
            CodegenNode::SelfRef => "this".to_string(),

            CodegenNode::Array(elements) => {
                let elems: Vec<String> = elements.iter().map(|e| self.emit(e, ctx)).collect();
                format!("{{ {} }}", elems.join(", "))
            }

            CodegenNode::Dict(pairs) => {
                let pairs_str: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{{{}, {}}}", self.emit(k, ctx), self.emit(v, ctx)))
                    .collect();
                format!(
                    "std::unordered_map<std::string, std::any>{{ {} }}",
                    pairs_str.join(", ")
                )
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
                    .map(|p| {
                        let t = p
                            .type_annotation
                            .as_ref()
                            .unwrap_or(&"auto".to_string())
                            .clone();
                        format!("{} {}", t, p.name)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[&]({}) {{ return {}; }}", params_str, self.emit(body, ctx))
            }

            CodegenNode::Cast { expr, target_type } => {
                format!("static_cast<{}>({})", target_type, self.emit(expr, ctx))
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
                format!("{}this->_transition({})", ind, target_state)
            }
            CodegenNode::ChangeState {
                target_state,
                indent,
                ..
            } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}this->_changeState({})", ind, target_state)
            }
            CodegenNode::Forward { indent, .. } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}return", ind)
            }
            CodegenNode::StackPush { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}_state_stack.push_back(__compartment->clone())", ind)
            }
            CodegenNode::StackPop { indent } => {
                let ind = format!("{}{}", ctx.get_indent(), " ".repeat(*indent));
                format!("{}auto __saved = std::move(_state_stack.back()); _state_stack.pop_back(); __transition(std::move(__saved))", ind)
            }
            CodegenNode::StateContext { state_name } => {
                format!("this->_stateContext[\"{}\"]", state_name)
            }

            CodegenNode::SendEvent { event, args } => {
                let args_str: Vec<String> = args.iter().map(|a| self.emit(a, ctx)).collect();
                if args_str.is_empty() {
                    format!("{}this->{}()", ctx.get_indent(), event)
                } else {
                    format!(
                        "{}this->{}({})",
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
        vec![
            "#include <string>".to_string(),
            "#include <unordered_map>".to_string(),
            "#include <vector>".to_string(),
            "#include <any>".to_string(),
            "#include <memory>".to_string(),
            "#include <functional>".to_string(),
        ]
    }

    fn class_syntax(&self) -> ClassSyntax {
        ClassSyntax::cpp()
    }
    fn target_language(&self) -> TargetLanguage {
        TargetLanguage::Cpp
    }
    fn null_keyword(&self) -> &'static str {
        "nullptr"
    }
}

impl CppBackend {
    fn emit_params(&self, params: &[Param]) -> String {
        params
            .iter()
            .map(|p| {
                let type_ann = p
                    .type_annotation
                    .as_ref()
                    .unwrap_or(&"auto".to_string())
                    .clone();
                // Map generic types to C++ equivalents
                let type_ann = match type_ann.as_str() {
                    "Any" => "std::any".to_string(),
                    "string" | "String" | "str" => "std::string".to_string(),
                    "number" => "int".to_string(),
                    "boolean" => "bool".to_string(),
                    _ => type_ann,
                };
                if let Some(ref default_val) = p.default_value {
                    let default_str =
                        self.emit(default_val, &mut super::super::backend::EmitContext::new());
                    format!("{} {} = {}", type_ann, p.name, default_str)
                } else {
                    format!("{} {}", type_ann, p.name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn map_type(&self, t: &str) -> String {
        match t {
            "Any" => "std::any".to_string(),
            "string" | "String" | "str" => "std::string".to_string(),
            "number" => "int".to_string(),
            "boolean" => "bool".to_string(),
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

    /// Emit a single C++ field declaration line:
    ///   `<indent>[const ]<type> <name>[ = <init>];\n`
    fn emit_field(&self, field: &Field, ctx: &mut EmitContext) -> String {
        let const_kw = if field.is_const { "const " } else { "" };
        let raw_type = field
            .type_annotation
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("void*");
        // Cross-system domain reference (`inner: Counter = @@Counter()`):
        // the codegen emits `new Counter()` for the initializer
        // (returns `Counter*`); a bare `Counter` field by value can't
        // accept the pointer. The convention used by existing matrix
        // demos (`tests/common/positive/demos/20_multi_system_composition.fcpp`)
        // is `std::shared_ptr<T>` — wrap the type and rewrite the
        // initializer's `new T(...)` to `std::make_shared<T>(...)`.
        let is_system_ref = ctx.defined_systems.contains(raw_type);
        let type_str = if is_system_ref {
            format!("std::shared_ptr<{}>", raw_type)
        } else {
            raw_type.to_string()
        };
        let init_suffix = match &field.initializer {
            Some(init) => {
                let mut init_src = self.emit(init, ctx);
                if is_system_ref {
                    let needle = format!("new {}(", raw_type);
                    let replacement = format!("std::make_shared<{}>(", raw_type);
                    init_src = init_src.replace(&needle, &replacement);
                }
                format!(" = {}", init_src)
            }
            None => String::new(),
        };
        format!(
            "{}{}{} {}{};\n",
            ctx.get_indent(),
            const_kw,
            type_str,
            field.name,
            init_suffix
        )
    }
}
