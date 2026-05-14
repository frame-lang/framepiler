//! Interface wrapper, action, operation, and persistence code generation.
//!
//! Generates:
//! - Interface method wrappers (public API → kernel dispatch)
//! - Action method bodies (native code with self.X rewriting)
//! - Operation method bodies (static/class methods)
//! - Persistence serialization/deserialization methods

mod dart_types;
mod extract;
mod nested_registry;
mod persist;
mod utility;

pub(crate) use extract::{extract_body_content, extract_tagged_system_name};

use dart_types::{dart_conv_expr, parse_dart_type, render_dart_type, DartTypeNode};
use utility::{frame_return_default, is_dynamic_target};

pub use nested_registry::{
    get_nested_system_domain_params, nested_uses_new_contract, set_new_contract_systems,
    set_nested_system_domain_params,
};

use std::collections::{HashMap, HashSet};

use super::ast::{CodegenNode, Param, Visibility};
use super::codegen_utils::{
    cpp_map_type, cpp_wrap_any_arg, csharp_map_type, expression_to_string, go_map_type,
    java_map_type, kotlin_map_type, swift_map_type, to_snake_case, type_to_cpp_string,
    type_to_string, HandlerContext,
};
use crate::frame_c::compiler::frame_ast::{
    ActionAst, InterfaceMethod, MethodParam, OperationAst, Span, SystemAst, Type,
};
use crate::frame_c::visitors::TargetLanguage;


/// Generate interface wrapper methods
///
/// For Python/TypeScript: Create FrameEvent and call __kernel
/// For Rust: Use match-based dispatch directly
///
/// If no explicit interface is defined, auto-generate interface methods from
/// unique event handlers found in the machine states (excluding lifecycle events).
pub(crate) fn generate_interface_wrappers(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> Vec<CodegenNode> {
    // Get the target language from the syntax
    let lang = syntax.language;
    let event_class = format!("{}FrameEvent", system.name);

    // If explicit interface is defined, use it
    // Otherwise, collect unique events from state handlers
    let interface_methods: Vec<InterfaceMethod> = if !system.interface.is_empty() {
        system.interface.clone()
    } else {
        // Auto-generate interface from event handlers
        let mut events: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut method_info: std::collections::HashMap<String, (Vec<MethodParam>, Option<Type>)> =
            std::collections::HashMap::new();

        if let Some(ref machine) = system.machine {
            for state in &machine.states {
                for handler in &state.handlers {
                    // Skip lifecycle events
                    if handler.event == "$>"
                        || handler.event == "<$"
                        || handler.event == "$>|"
                        || handler.event == "<$|"
                    {
                        continue;
                    }
                    if events.insert(handler.event.clone()) {
                        // First time seeing this event - capture its params and return type
                        let params: Vec<MethodParam> = handler
                            .params
                            .iter()
                            .map(|p| MethodParam {
                                name: p.name.clone(),
                                param_type: p.param_type.clone(),
                                default: None,
                                span: Span::new(0, 0),
                            })
                            .collect();
                        method_info
                            .insert(handler.event.clone(), (params, handler.return_type.clone()));
                    }
                }
            }
        }

        events
            .into_iter()
            .map(|event| {
                let (params, return_type) = method_info.get(&event).cloned().unwrap_or_default();
                InterfaceMethod {
                    name: event,
                    params,
                    return_type,
                    return_init: None,
                    is_async: false,
                    is_static: false,
                    leading_comments: Vec::new(),
                    attributes: Vec::new(),
                    span: Span::new(0, 0),
                }
            })
            .collect()
    };

    interface_methods.iter().flat_map(|method| {
        let params: Vec<Param> = method.params.iter().map(|p| {
            let type_str = type_to_string(&p.param_type);
            Param::new(&p.name).with_type(&type_str)
        }).collect();

        let _args: Vec<CodegenNode> = method.params.iter()
            .map(|p| CodegenNode::ident(&p.name))
            .collect();

        // Language-specific dispatch - all languages now use kernel pattern
        let body_stmt = match lang {
            TargetLanguage::Rust => {
                super::rust_system::generate_rust_interface_body(
                    &system.name,
                    method,
                    &event_class,
                )
            }
            TargetLanguage::Python3 => {
                // Python: Create FrameEvent + FrameContext, push context, call __kernel, pop and return
                // Parameters are packed positionally into a list
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    format!("[{}]", param_items.join(", "))
                };

                // Python is dynamic per docs/frame_runtime.md § "Return
                // values across target languages": source-level type
                // annotations are documentation only. The wrapper
                // returns whatever's in the FrameContext's return slot
                // — including None when no @@:return was written.
                // Don't init the slot to a type default here; that
                // would contradict the documented dynamic-lang
                // contract and break test 14_system_return_default.
                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                // Python is a dynamic target: the wrapper always exposes
                // the FrameContext's return slot, regardless of declared
                // return type (see docs/frame_runtime.md § "Return values
                // across target languages"). Source-level `: type` is
                // documentation only here.
                let has_return = is_dynamic_target(lang)
                    || method.return_type.is_some()
                    || method.return_init.is_some();
                if has_return {
                    CodegenNode::NativeBlock {
                        code: format!(
                            r#"__e = {}("{}", {})
__ctx = {}(__e, None){}
self._context_stack.append(__ctx)
self.__kernel(__e)
return self._context_stack.pop()._return"#,
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            r#"__e = {}("{}", {})
__ctx = {}(__e, None){}
self._context_stack.append(__ctx)
self.__kernel(__e)
self._context_stack.pop()"#,
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                // TypeScript/JavaScript: Create FrameEvent + FrameContext, push context, call __kernel, pop and return
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    format!("[{}]", param_items.join(", "))
                };

                // Per docs/frame_runtime.md § "Return values across
                // target languages": all backends return the slot's
                // natural null/undefined when no @@:return was
                // written. Source-level type annotations don't
                // change the runtime default.
                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {};", init_expr)
                } else {
                    String::new()
                };

                // TypeScript uses ! (non-null assertion), JavaScript doesn't
                let pop_suffix = if matches!(lang, TargetLanguage::TypeScript) { "!" } else { "" };

                // JavaScript is dynamic (always returns); TypeScript is
                // strongly-typed (conditional on declared type).
                if is_dynamic_target(lang)
                    || method.return_type.is_some()
                    || method.return_init.is_some()
                {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "const __e = new {}(\"{}\", {});\nconst __ctx = new {}(__e, null);{}\nthis._context_stack.push(__ctx);\nthis.__kernel(__e);\nreturn this._context_stack.pop(){}._return;",
                            event_class, method.name, params_code, context_class, default_init, pop_suffix
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "const __e = new {}(\"{}\", {});\nconst __ctx = new {}(__e, null);{}\nthis._context_stack.push(__ctx);\nthis.__kernel(__e);\nthis._context_stack.pop();",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::Php => {
                // PHP: Create FrameEvent + FrameContext, push context, call __kernel, pop and return
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("${}", p.name))
                        .collect();
                    format!("[{}]", param_items.join(", "))
                };

                // PHP is dynamic — see Python branch comment.
                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n$__ctx->_return = {};", init_expr)
                } else {
                    String::new()
                };

                if is_dynamic_target(lang)
                    || method.return_type.is_some()
                    || method.return_init.is_some()
                {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "$__e = new {}(\"{}\", {});\n$__ctx = new {}($__e, null);{}\n$this->_context_stack[] = $__ctx;\n$this->__kernel($__e);\nreturn array_pop($this->_context_stack)->_return;",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "$__e = new {}(\"{}\", {});\n$__ctx = new {}($__e, null);{}\n$this->_context_stack[] = $__ctx;\n$this->__kernel($__e);\narray_pop($this->_context_stack);",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::Ruby => {
                // Ruby: Create FrameEvent + FrameContext, push context, call __kernel, pop and return
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    format!("[{}]", param_items.join(", "))
                };

                // Ruby is dynamic — see Python branch comment.
                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                if is_dynamic_target(lang)
                    || method.return_type.is_some()
                    || method.return_init.is_some()
                {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "__e = {}.new(\"{}\", {})\n__ctx = {}.new(__e, nil){}\n@_context_stack.push(__ctx)\n__kernel(__e)\nreturn @_context_stack.pop._return",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "__e = {}.new(\"{}\", {})\n__ctx = {}.new(__e, nil){}\n@_context_stack.push(__ctx)\n__kernel(__e)\n@_context_stack.pop",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::C => {
                // C: Create FrameEvent + FrameContext, push context, call kernel, pop and return
                let sys = &system.name;

                // Build parameters dict creation (with semicolon).
                // Float/double params route through `Sys_pack_double`
                // (memcpy bit-pun) — `(void*)(intptr_t)(0.5)` truncates
                // to integer and breaks any float-typed interface arg.
                let params_code = if method.params.is_empty() {
                    format!("{}_FrameEvent* __e = {}_FrameEvent_new(\"{}\", NULL, 0);", sys, sys, method.name)
                } else {
                    let mut code = format!("{}_FrameVec* __params = {}_FrameVec_new();\n", sys, sys);
                    for p in &method.params {
                        let p_type = type_to_string(&p.param_type);
                        let push_arg = match p_type.trim() {
                            "float" | "double" | "f32" | "f64" => {
                                format!("{}_pack_double({})", sys, p.name)
                            }
                            _ => format!("(void*)(intptr_t){}", p.name),
                        };
                        code.push_str(&format!(
                            "{}_FrameVec_push(__params, {});\n",
                            sys, push_arg
                        ));
                    }
                    code.push_str(&format!("{}_FrameEvent* __e = {}_FrameEvent_new(\"{}\", __params, 1);", sys, sys, method.name));
                    code
                };

                // Check if method has a non-void return type
                let return_type_str = method.return_type.as_ref().map(|t| type_to_string(t));
                // Convert Frame types to C types
                let return_type_str = return_type_str.map(|s| {
                    match s.as_str() {
                        "str" | "string" | "String" => "char*".to_string(),
                        "bool" | "boolean" => "bool".to_string(),
                        "int" | "number" | "Any" => "int".to_string(),
                        "float" | "double" => "double".to_string(),
                        "void" | "None" => "void".to_string(),
                        _ => s
                    }
                });
                let has_return_value = return_type_str.as_ref()
                    .map(|s| s != "void" && s != "None")
                    .unwrap_or(false);

                // Set default return value after context creation. For
                // float/double returns, pack via the memcpy helper since
                // `(void*)(intptr_t)(3.14)` truncates to an integer.
                let is_double_return = return_type_str
                    .as_ref()
                    .map(|s| s == "double")
                    .unwrap_or(false);
                let default_init = if let Some(ref init_expr) = method.return_init {
                    if is_double_return {
                        format!("\n__ctx->_return = {}_pack_double({});", sys, init_expr)
                    } else {
                        format!("\n__ctx->_return = (void*)(intptr_t)({});", init_expr)
                    }
                } else {
                    String::new()
                };

                if let (true, Some(return_type_str)) = (has_return_value, return_type_str) {
                    // Unpack based on declared return type:
                    //   bool/int  → `(intptr_t)__ctx->_return`  (truncates to int size)
                    //   double    → `Sys_unpack_double(...)`     (memcpy round-trip)
                    //   str/ptr   → `(T)__ctx->_return`          (pointer already fits)
                    let extract = if return_type_str == "double" {
                        format!("{}_unpack_double(__result_ctx->_return)", sys)
                    } else {
                        let cast = match return_type_str.as_str() {
                            "bool" | "int" => "(intptr_t)",
                            _ => "",
                        };
                        format!("({}){}__result_ctx->_return", return_type_str, cast)
                    };
                    CodegenNode::NativeBlock {
                        code: format!(
                            r#"{}
{}_FrameContext* __ctx = {}_FrameContext_new(__e, NULL);{}
{}_FrameVec_push(self->_context_stack, __ctx);
{}_kernel(self, __e);
{}_FrameContext* __result_ctx = ({}_FrameContext*){}_FrameVec_pop(self->_context_stack);
{} __result = {};
{}_FrameContext_destroy(__result_ctx);
{}_FrameEvent_destroy(__e);
return __result;"#,
                            params_code, sys, sys, default_init, sys, sys, sys, sys, sys,
                            return_type_str, extract, sys, sys
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            r#"{}
{}_FrameContext* __ctx = {}_FrameContext_new(__e, NULL);{}
{}_FrameVec_push(self->_context_stack, __ctx);
{}_kernel(self, __e);
{}_FrameContext* __result_ctx = ({}_FrameContext*){}_FrameVec_pop(self->_context_stack);
{}_FrameContext_destroy(__result_ctx);
{}_FrameEvent_destroy(__e);"#,
                            params_code, sys, sys, default_init, sys, sys, sys, sys, sys, sys, sys
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::Cpp => {
                let context_class = format!("{}FrameContext", system.name);
                // Map the Frame return type to its C++ equivalent BEFORE
                // any further use. Without this `: str` returns leak into
                // `std::any_cast<str>` (undefined identifier) and `: void`
                // returns leak into `std::any_cast<void>` (which is a
                // template substitution failure). The mapper translates
                // `str`/`string`/`String` to `std::string`, `bool`/`boolean`
                // to `bool`, etc.
                let raw_return_type = method.return_type.as_ref()
                    .map(|t| type_to_string(t))
                    .unwrap_or_else(|| "void".to_string());
                let return_type_str = cpp_map_type(&raw_return_type);
                // A method with `: void` declared has no value to extract
                // from the return slot — treat it as no-return for the
                // any_cast/return path. The context still needs to be
                // pushed so the handler can run, but we never read back.
                let returns_value = (method.return_type.is_some() || method.return_init.is_some())
                    && return_type_str != "void";
                // System is async when any interface method is declared
                // async. Bodies `co_return` instead of `return`; internal
                // dispatch calls get `co_await` prefix (see `add_await_to_string`).
                let system_is_async = system.interface.iter().any(|m| m.is_async);

                let mut code = String::new();

                // Build params list
                if !method.params.is_empty() {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    code.push_str(&format!("{} __e(\"{}\", std::vector<std::any>{{{}}});\n",
                        event_class, method.name, param_items.join(", ")));
                } else {
                    code.push_str(&format!("{} __e(\"{}\");\n", event_class, method.name));
                }

                // Create context with default return
                if let Some(ref init) = method.return_init {
                    // Wrap string literals for std::any (const char* != std::string)
                    let init_wrapped = if init.starts_with('"') && return_type_str == "std::string" {
                        format!("std::string({})", init)
                    } else {
                        init.clone()
                    };
                    code.push_str(&format!("{} __ctx(std::move(__e), std::any({}));\n", context_class, init_wrapped));
                } else if returns_value {
                    code.push_str(&format!("{} __ctx(std::move(__e), std::any({}()));\n", context_class, return_type_str));
                } else {
                    code.push_str(&format!("{} __ctx(std::move(__e));\n", context_class));
                }

                code.push_str("_context_stack.push_back(std::move(__ctx));\n");
                // Async: await the kernel so nested co_awaits work; sync:
                // plain call.
                if system_is_async {
                    code.push_str("co_await __kernel(_context_stack.back()._event);\n");
                } else {
                    code.push_str("__kernel(_context_stack.back()._event);\n");
                }

                let ret_kw = if system_is_async { "co_return" } else { "return" };
                if returns_value {
                    code.push_str(&format!("auto __result = std::any_cast<{}>(std::move(_context_stack.back()._return));\n", return_type_str));
                    code.push_str("_context_stack.pop_back();\n");
                    code.push_str(&format!("{} __result;", ret_kw));
                } else if system_is_async {
                    code.push_str("_context_stack.pop_back();\n");
                    code.push_str("co_return;");
                } else {
                    code.push_str("_context_stack.pop_back();");
                }

                CodegenNode::NativeBlock { code, span: None }
            }
            TargetLanguage::Java => {
                let context_class = format!("{}FrameContext", system.name);
                let has_return = method.return_type.is_some() || method.return_init.is_some();
                let return_type_str = method.return_type.as_ref()
                    .map(|t| type_to_string(t))
                    .unwrap_or_else(|| "void".to_string());
                // Java has no native async/await; methods declared
                // `async` in the Frame interface return
                // `CompletableFuture<T>` with the body running
                // synchronously and wrapping its result via
                // `completedFuture(...)`. Sync methods in the same
                // interface keep their plain return — Java doesn't
                // cascade async-ness across the whole interface
                // (unlike e.g. TypeScript or Kotlin). The user opts
                // each method in by declaring it `async`.
                let method_is_async = method.is_async;

                let mut code = String::new();

                // Build params list
                if !method.params.is_empty() {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    code.push_str(&format!("{} __e = new {}(\"{}\", new ArrayList<>(Arrays.asList({})));\n",
                        event_class, event_class, method.name, param_items.join(", ")));
                } else {
                    code.push_str(&format!("{} __e = new {}(\"{}\", new ArrayList<>());\n", event_class, event_class, method.name));
                }

                // Create context with default return. Per
                // docs/frame_runtime.md, all backends return the
                // slot's natural null when no @@:return is written.
                // Java's `(int) null` (auto-unbox) does throw NPE if
                // the user-written interface declares `: int` and
                // no handler writes @@:return — but that's a
                // user-bug pattern that the existing test corpus
                // doesn't exercise (every existing typed-int
                // handler explicitly writes @@:return). Documented
                // contract takes precedence.
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("{} __ctx = new {}(__e, {});\n", context_class, context_class, init));
                } else {
                    code.push_str(&format!("{} __ctx = new {}(__e, null);\n", context_class, context_class));
                }

                code.push_str("_context_stack.add(__ctx);\n");
                code.push_str("__kernel(_context_stack.get(_context_stack.size() - 1)._event);\n");

                if has_return && return_type_str != "void" && return_type_str != "Any" && return_type_str != "Object" {
                    let java_type = java_map_type(&return_type_str);
                    code.push_str(&format!("{} __result = ({}) _context_stack.get(_context_stack.size() - 1)._return;\n", java_type, java_type));
                    code.push_str("_context_stack.remove(_context_stack.size() - 1);\n");
                    if method_is_async {
                        code.push_str("return java.util.concurrent.CompletableFuture.completedFuture(__result);");
                    } else {
                        code.push_str("return __result;");
                    }
                } else {
                    code.push_str("_context_stack.remove(_context_stack.size() - 1);");
                    if method_is_async {
                        code.push_str("\nreturn java.util.concurrent.CompletableFuture.completedFuture(null);");
                    }
                }

                CodegenNode::NativeBlock { code, span: None }
            }
            TargetLanguage::Kotlin => {
                let context_class = format!("{}FrameContext", system.name);
                let has_return = method.return_type.is_some() || method.return_init.is_some();
                let return_type_str = method.return_type.as_ref()
                    .map(|t| type_to_string(t))
                    .unwrap_or_else(|| "void".to_string());

                let mut code = String::new();

                // Build params list — Kotlin: no new, no semicolons, mutableListOf
                if !method.params.is_empty() {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    code.push_str(&format!("val __e = {}(\"{}\", mutableListOf({}))\n",
                        event_class, method.name, param_items.join(", ")));
                } else {
                    code.push_str(&format!("val __e = {}(\"{}\", mutableListOf())\n", event_class, method.name));
                }

                // Create context with type-appropriate default return.
                // If handler is suppressed (e.g., transition guard), _return
                // keeps this default — wrappers never see null for typed returns.
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("val __ctx = {}(__e, {})\n", context_class, init));
                } else if has_return && return_type_str != "void" {
                    let kotlin_type = kotlin_map_type(&return_type_str);
                    let kt_default = match kotlin_type.as_str() {
                        "Int" | "Long" => "0",
                        "Double" | "Float" => "0.0",
                        "Boolean" => "false",
                        "String" => "\"\"",
                        _ => "null",
                    };
                    code.push_str(&format!("val __ctx = {}(__e, {})\n", context_class, kt_default));
                } else {
                    code.push_str(&format!("val __ctx = {}(__e, null)\n", context_class));
                }

                code.push_str("_context_stack.add(__ctx)\n");
                code.push_str("__kernel(_context_stack[_context_stack.size - 1]._event)\n");

                if has_return && return_type_str != "void" && return_type_str != "Any" && return_type_str != "Any?" {
                    let kotlin_type = kotlin_map_type(&return_type_str);
                    code.push_str(&format!("val __result = _context_stack[_context_stack.size - 1]._return as {}\n", kotlin_type));
                    code.push_str("_context_stack.removeAt(_context_stack.size - 1)\n");
                    code.push_str("return __result");
                } else {
                    code.push_str("_context_stack.removeAt(_context_stack.size - 1)");
                }

                CodegenNode::NativeBlock { code, span: None }
            }
            TargetLanguage::Swift => {
                let context_class = format!("{}FrameContext", system.name);
                let has_return = method.return_type.is_some() || method.return_init.is_some();
                let return_type_str = method.return_type.as_ref()
                    .map(|t| type_to_string(t))
                    .unwrap_or_else(|| "void".to_string());

                let mut code = String::new();

                // Build params list — Swift: [Any] array literal
                if !method.params.is_empty() {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    code.push_str(&format!("let __e = {}(message: \"{}\", parameters: [{}])\n",
                        event_class, method.name, param_items.join(", ")));
                } else {
                    code.push_str(&format!("let __e = {}(message: \"{}\", parameters: [])\n", event_class, method.name));
                }

                // Create context with type-appropriate default return
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("let __ctx = {}(event: __e, defaultReturn: {})\n", context_class, init));
                } else if has_return && return_type_str != "void" {
                    let swift_type = swift_map_type(&return_type_str);
                    let sw_default = if swift_type.ends_with('?') {
                        "nil"
                    } else {
                        match swift_type.as_str() {
                            "Int" => "0",
                            "Double" | "Float" => "0.0",
                            "Bool" => "false",
                            "String" => "\"\"",
                            _ => "nil",
                        }
                    };
                    code.push_str(&format!("let __ctx = {}(event: __e, defaultReturn: {})\n", context_class, sw_default));
                } else {
                    code.push_str(&format!("let __ctx = {}(event: __e)\n", context_class));
                }

                code.push_str("_context_stack.append(__ctx)\n");
                code.push_str("__kernel(_context_stack[_context_stack.count - 1]._event)\n");

                if has_return && return_type_str != "void" && return_type_str != "Any" && return_type_str != "Any?" {
                    let swift_type = swift_map_type(&return_type_str);
                    code.push_str(&format!("let __result = _context_stack[_context_stack.count - 1]._return as! {}\n", swift_type));
                    code.push_str("_context_stack.removeLast()\n");
                    code.push_str("return __result");
                } else {
                    code.push_str("_context_stack.removeLast()");
                }

                CodegenNode::NativeBlock { code, span: None }
            }
            TargetLanguage::CSharp => {
                let context_class = format!("{}FrameContext", system.name);
                let has_return = method.return_type.is_some() || method.return_init.is_some();
                let return_type_str = method.return_type.as_ref()
                    .map(|t| type_to_string(t))
                    .unwrap_or_else(|| "void".to_string());

                let mut code = String::new();

                // Build params list
                if !method.params.is_empty() {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    code.push_str(&format!("{} __e = new {}(\"{}\", new List<object> {{ {} }});\n",
                        event_class, event_class, method.name, param_items.join(", ")));
                } else {
                    code.push_str(&format!("{} __e = new {}(\"{}\", new List<object>());\n", event_class, event_class, method.name));
                }

                // C# follows the same documented contract as Java
                // (see Java branch above): return slot's natural
                // null when no @@:return is written.
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("{} __ctx = new {}(__e, {});\n", context_class, context_class, init));
                } else {
                    code.push_str(&format!("{} __ctx = new {}(__e, null);\n", context_class, context_class));
                }

                code.push_str("_context_stack.Add(__ctx);\n");
                code.push_str("__kernel(_context_stack[_context_stack.Count - 1]._event);\n");

                if has_return && return_type_str != "void" && return_type_str != "Any" && return_type_str != "object" {
                    let cs_type = csharp_map_type(&return_type_str);
                    code.push_str(&format!("var __result = ({}) _context_stack[_context_stack.Count - 1]._return;\n", cs_type));
                    code.push_str("_context_stack.RemoveAt(_context_stack.Count - 1);\n");
                    code.push_str("return __result;");
                } else {
                    code.push_str("_context_stack.RemoveAt(_context_stack.Count - 1);");
                }

                CodegenNode::NativeBlock { code, span: None }
            }
            TargetLanguage::Go => {
                let has_return = method.return_type.is_some() || method.return_init.is_some();
                let return_type_str = method.return_type.as_ref()
                    .map(|t| type_to_string(t))
                    .unwrap_or_else(|| "void".to_string());

                let mut code = String::new();

                // Build params list
                if !method.params.is_empty() {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    code.push_str(&format!("__e := {}FrameEvent{{_message: \"{}\", _parameters: []any{{{}}}}}\n",
                        system.name, method.name, param_items.join(", ")));
                } else {
                    code.push_str(&format!("__e := {}FrameEvent{{_message: \"{}\", _parameters: []any{{}}}}\n", system.name, method.name));
                }

                // Create context with default return
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("__ctx := {}FrameContext{{_event: __e, _return: {}, _data: make(map[string]any)}}\n",
                        system.name, init));
                } else {
                    code.push_str(&format!("__ctx := {}FrameContext{{_event: __e, _data: make(map[string]any)}}\n",
                        system.name));
                }

                code.push_str("s._context_stack = append(s._context_stack, __ctx)\n");
                code.push_str("s.__kernel(&s._context_stack[len(s._context_stack)-1]._event)\n");

                if has_return && return_type_str != "void" && return_type_str != "Any" {
                    let go_type = go_map_type(&return_type_str);
                    if go_type.is_empty() {
                        code.push_str("s._context_stack = s._context_stack[:len(s._context_stack)-1]");
                    } else {
                        code.push_str(&format!("var __result {}\nif __rv := s._context_stack[len(s._context_stack)-1]._return; __rv != nil {{ __result = __rv.({}) }}\n", go_type, go_type));
                        code.push_str("s._context_stack = s._context_stack[:len(s._context_stack)-1]\n");
                        code.push_str("return __result");
                    }
                } else {
                    code.push_str("s._context_stack = s._context_stack[:len(s._context_stack)-1]");
                }

                CodegenNode::NativeBlock { code, span: None }
            }
            TargetLanguage::Lua => {
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "{}".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    format!("{{{}}}", param_items.join(", "))
                };

                // Lua is dynamic — see Python branch comment.
                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                if is_dynamic_target(lang)
                    || method.return_type.is_some()
                    || method.return_init.is_some()
                {
                    // Lua's block transformer treats `return` as terminal
                    // and strips any token that follows it on the same
                    // line. Emitting `local __ret = ...; pop; return __ret`
                    // would have `return __ret` collapse to bare `return`,
                    // discarding the value. Use `table.remove(...)._return`
                    // as a single expression so the return statement has
                    // the value embedded inline — no trailing token to
                    // strip.
                    CodegenNode::NativeBlock {
                        code: format!(
                            "local __e = {}.new(\"{}\", {})\nlocal __ctx = {}.new(__e, nil){}\nself._context_stack[#self._context_stack + 1] = __ctx\nself:__kernel(__e)\nreturn table.remove(self._context_stack)._return",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "local __e = {}.new(\"{}\", {})\nlocal __ctx = {}.new(__e, nil){}\nself._context_stack[#self._context_stack + 1] = __ctx\nself:__kernel(__e)\nself._context_stack[#self._context_stack] = nil",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::Dart => {
                // Dart: similar to TypeScript
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    format!("[{}]", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {};", init_expr)
                } else {
                    String::new()
                };

                if method.return_type.is_some() || method.return_init.is_some() {
                    let rt = method.return_type.as_ref().map(|t| type_to_string(t)).unwrap_or_default();
                    let dart_default = match rt.as_str() {
                        "int" | "num" => "0",
                        "double" | "float" => "0.0",
                        "bool" => "false",
                        "String" | "str" | "string" => "''",
                        _ => "null",
                    };
                    CodegenNode::NativeBlock {
                        code: format!(
                            "final __e = {}(\"{}\", {});\nfinal __ctx = {}(__e, {});{}\n_context_stack.add(__ctx);\n__kernel(__e);\nreturn _context_stack.removeLast()._return;",
                            event_class, method.name, params_code, context_class, dart_default, default_init
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "final __e = {}(\"{}\", {});\nfinal __ctx = {}(__e, null);{}\n_context_stack.add(__ctx);\n__kernel(__e);\n_context_stack.removeLast();",
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::GDScript => {
                // GDScript: similar to Python
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "[]".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| p.name.clone())
                        .collect();
                    format!("[{}]", param_items.join(", "))
                };

                // GDScript is dynamic — see Python branch comment.
                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                let has_return = is_dynamic_target(lang)
                    || method.return_type.is_some()
                    || method.return_init.is_some();
                if has_return {
                    CodegenNode::NativeBlock {
                        code: format!(
                            r#"var __e = {}.new("{}", {})
var __ctx = {}.new(__e, null){}
self._context_stack.append(__ctx)
self.__kernel(__e)
return self._context_stack.pop_back()._return"#,
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                } else {
                    CodegenNode::NativeBlock {
                        code: format!(
                            r#"var __e = {}.new("{}", {})
var __ctx = {}.new(__e, null){}
self._context_stack.append(__ctx)
self.__kernel(__e)
self._context_stack.pop_back()"#,
                            event_class, method.name, params_code, context_class, default_init
                        ),
                        span: None,
                    }
                }
            }
            TargetLanguage::Erlang => {
                // Erlang interface wrappers are generated by erlang_system.rs
                CodegenNode::Empty
            }
            TargetLanguage::Graphviz => unreachable!(),
        };

        // Source-level comments preceding this `interface:` method
        // declaration emit as `NativeBlock` nodes before the
        // `Method` itself. Per-backend `NativeBlock` rendering
        // already handles indentation, so the comment lines land
        // above the wrapper at the correct class-body depth. The
        // text passes through verbatim — Frame source for a target
        // already uses that target's comment leader (Oceans Model).
        let mut nodes = Vec::new();
        for comment in &method.leading_comments {
            nodes.push(CodegenNode::NativeBlock {
                code: comment.clone(),
                span: None,
            });
        }
        nodes.push(CodegenNode::Method {
            name: method.name.clone(),
            params,
            return_type: method.return_type.as_ref().map(|t| type_to_string(t)),
            body: vec![body_stmt],
            is_async: false,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        });
        nodes
    }).collect()
}

/// Generate action method (and any leading-comment trivia).
///
/// Returns a `Vec<CodegenNode>`: zero-or-more `NativeBlock` comment
/// nodes followed by the action's `Method` node. Callers `.extend`
/// the result into the class's `methods` list. Extracts native code
/// from source using the body span (Oceans Model).
pub(crate) fn generate_action(
    action: &ActionAst,
    syntax: &super::backend::ClassSyntax,
    source: &[u8],
) -> Vec<CodegenNode> {
    let params: Vec<Param> = action
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            Param::new(&p.name).with_type(&type_str)
        })
        .collect();

    // Extract native code from source using span (oceans model)
    let mut code = extract_body_content(source, &action.body.span);

    // Lower `@@:(expr)` and `@@:system.state` in the action body. Per
    // `_scratch/bug_at_at_colon_paren_in_actions_passthrough.md`, the
    // concise return-sigil `@@:(expr)` is intuitive enough that users
    // reach for it in actions even though the doc table prescribes
    // native `return`. The shared expansion lowers it to
    // `return expr` (target-language idiomatic) so action bodies
    // get the DTRT treatment instead of passing the literal sigil
    // through to the target compiler. Same path as `generate_operation`.
    code = super::frame_expansion::expand_system_state_in_code(&code, syntax.language);

    let ret_type = match &action.return_type {
        crate::frame_c::compiler::frame_ast::Type::Unknown => None,
        t => Some(type_to_string(t)),
    };

    let mut nodes: Vec<CodegenNode> = action
        .leading_comments
        .iter()
        .map(|c| CodegenNode::NativeBlock {
            code: c.clone(),
            span: None,
        })
        .collect();
    nodes.push(CodegenNode::Method {
        name: action.name.clone(),
        params,
        return_type: ret_type,
        body: vec![CodegenNode::NativeBlock {
            code,
            span: Some(action.body.span.clone()),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });
    nodes
}

/// Generate operation method
///
/// Extracts native code from source using the body span
pub(crate) fn generate_operation(
    operation: &OperationAst,
    syntax: &super::backend::ClassSyntax,
    source: &[u8],
) -> Vec<CodegenNode> {
    let params: Vec<Param> = operation
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            Param::new(&p.name).with_type(&type_str)
        })
        .collect();

    // Extract native code from source using span (oceans model)
    let mut code = extract_body_content(source, &operation.body.span);

    // Expand @@:system.state and @@:(expr) in operation bodies.
    // @@:system.state requires `self` (non-static only), but @@:(expr) works in both.
    code = super::frame_expansion::expand_system_state_in_code(&code, syntax.language);

    let mut nodes: Vec<CodegenNode> = operation
        .leading_comments
        .iter()
        .map(|c| CodegenNode::NativeBlock {
            code: c.clone(),
            span: None,
        })
        .collect();
    // Backend handles is_static flag for @staticmethod decorator
    nodes.push(CodegenNode::Method {
        name: operation.name.clone(),
        params,
        return_type: match &operation.return_type {
            crate::frame_c::compiler::frame_ast::Type::Unknown => None, // void
            t => Some(type_to_string(t)),
        },
        body: vec![CodegenNode::NativeBlock {
            code,
            span: Some(operation.body.span.clone()),
        }],
        is_async: false,
        is_static: operation.is_static,
        visibility: Visibility::Public,
        decorators: vec![],
    });
    nodes
}

/// Generate persistence methods (save_state, restore_state) for @@persist
pub(crate) fn generate_persistence_methods(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    match syntax.language {
        TargetLanguage::Python3 => methods.extend(persist::python::generate(system)),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            let is_ts = matches!(syntax.language, TargetLanguage::TypeScript);
            methods.extend(persist::javascript::generate(system, is_ts));
        }
        TargetLanguage::Rust => {
            methods.extend(super::rust_system::generate_rust_persistence_methods(
                system,
            ));
        }
        TargetLanguage::C => {
            // RFC-0012 amendment: branch on new contract. C is the
            // most divergent backend — there's no implicit `self`,
            // every "method" is `Sys_method(Sys* self, args)`. Under
            // legacy, restore is a factory `Sys_restore_state(json)`
            // that returns a fresh `Sys*`. Under new contract, load
            // takes `Sys* self` as first arg and populates it.
            let uses_new_contract = system.uses_new_persist_contract();
            let save_method_name = system
                .save_op_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "save_state".to_string());
            let load_method_name = system
                .load_op_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "restore_state".to_string());
            let load_param_name = system
                .load_op_param_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "json".to_string());
            let target = if uses_new_contract {
                "self"
            } else {
                "instance"
            };

            // C uses cJSON library (requires cJSON.h/cJSON.c or -lcjson)
            // HSM persistence: serialize entire compartment chain including parent_compartment

            // First, generate helper functions for compartment serialization/deserialization
            // These will be generated as static functions before save_state/restore_state

            // Generate serialize_compartment helper
            let mut serialize_helper = String::new();
            serialize_helper.push_str(&format!(
                "static cJSON* {}_serialize_compartment({}_Compartment* comp) {{\n",
                system.name, system.name
            ));
            serialize_helper.push_str("    if (!comp) return cJSON_CreateNull();\n");
            serialize_helper.push_str("    cJSON* obj = cJSON_CreateObject();\n");
            serialize_helper
                .push_str("    cJSON_AddStringToObject(obj, \"state\", comp->state);\n");
            // Serialize state_vars (iterate over bucket-based linked list)
            serialize_helper.push_str("    cJSON* vars = cJSON_CreateObject();\n");
            serialize_helper.push_str(&format!(
                "    {}_FrameDict* sv = comp->state_vars;\n",
                system.name
            ));
            serialize_helper.push_str("    if (sv) {\n");
            serialize_helper.push_str("        for (int i = 0; i < sv->bucket_count; i++) {\n");
            serialize_helper.push_str(&format!(
                "            {}_FrameDictEntry* entry = sv->buckets[i];\n",
                system.name
            ));
            serialize_helper.push_str("            while (entry) {\n");
            serialize_helper.push_str("                cJSON_AddNumberToObject(vars, entry->key, (double)(intptr_t)entry->value);\n");
            serialize_helper.push_str("                entry = entry->next;\n");
            serialize_helper.push_str("            }\n");
            serialize_helper.push_str("        }\n");
            serialize_helper.push_str("    }\n");
            serialize_helper.push_str("    cJSON_AddItemToObject(obj, \"state_vars\", vars);\n");
            // state_args + enter_args — type-aware via per-state branch.
            // C stores FrameVec values as `void*`. For int types,
            // `(intptr_t) ptr` round-trips. For float/double types,
            // we use `Sys_pack_double` / `Sys_unpack_double` (memcpy
            // bit-pun). Without per-state typing, doubles truncate
            // through the int path — D8 fix introduces the per-state
            // branch by reading `comp->state` and dispatching to the
            // declared param types.
            //
            // Build per-state arg type metadata at codegen time.
            let type_to_string = |t: &crate::frame_c::compiler::frame_ast::Type| -> String {
                match t {
                    crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.clone(),
                    crate::frame_c::compiler::frame_ast::Type::Unknown => "int".to_string(),
                }
            };
            let state_arg_types: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .map(|s| {
                            let types: Vec<String> = s
                                .params
                                .iter()
                                .map(|p| type_to_string(&p.param_type))
                                .collect();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let state_enter_arg_types: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .map(|s| {
                            let types: Vec<String> = s
                                .enter
                                .as_ref()
                                .map(|e| {
                                    e.params
                                        .iter()
                                        .map(|p| type_to_string(&p.param_type))
                                        .collect()
                                })
                                .unwrap_or_default();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Symbol-mangled dispatch: framec emits
            // `<sys>_persist_pack_<mangled>(value)` and
            // `<sys>_persist_unpack_<mangled>(json)` calls. The runtime
            // (in runtime.rs) defines the symbols for blessed types
            // (int / str / bool / double / list / dict). framec is
            // type-ignorant beyond a tiny alias-normalisation table —
            // it doesn't parse generics, doesn't recognise library
            // APIs, doesn't branch on element types.
            let c_mangle_type = |t: &str| -> String {
                let t = t.trim();
                // Normalise common aliases to canonical symbol names.
                // Anything not in this table maps verbatim (sanitised
                // to a valid C identifier suffix).
                let canonical = match t {
                    "i32" | "i64" | "isize" | "uint" | "uintptr_t" | "intptr_t" | "long"
                    | "short" => "int",
                    "f32" | "f64" | "float" => "double",
                    "boolean" => "bool",
                    "string" | "String" | "str" | "char*" | "const char*" => "str",
                    "List" | "Array" | "Array<any>" => "list",
                    "Dict" | "Record<string, any>" => "dict",
                    other => other,
                };
                canonical
                    .chars()
                    .map(|c| match c {
                        'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => c,
                        '*' => 'P',
                        _ => '_',
                    })
                    .collect()
            };
            let c_pack_for = |frame_type: &str, val_expr: &str, sys_name: &str| -> String {
                format!(
                    "{sys_name}_persist_pack_{m}({val_expr})",
                    m = c_mangle_type(frame_type)
                )
            };
            serialize_helper.push_str("    cJSON* sa = cJSON_CreateArray();\n");
            serialize_helper.push_str("    if (comp->state_args) {\n");
            for (state_name, types) in &state_arg_types {
                if types.is_empty() {
                    continue;
                }
                serialize_helper.push_str(&format!(
                    "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
                    state_name
                ));
                for (i, t) in types.iter().enumerate() {
                    let val_expr = format!("comp->state_args->items[{}]", i);
                    serialize_helper.push_str(&format!(
                        "            if ({i} < comp->state_args->size) cJSON_AddItemToArray(sa, {pack});\n",
                        i = i,
                        pack = c_pack_for(t, &val_expr, &system.name)
                    ));
                }
                serialize_helper.push_str("        }\n");
            }
            serialize_helper.push_str("    }\n");
            serialize_helper.push_str("    cJSON_AddItemToObject(obj, \"state_args\", sa);\n");
            serialize_helper.push_str("    cJSON* ea = cJSON_CreateArray();\n");
            serialize_helper.push_str("    if (comp->enter_args) {\n");
            for (state_name, types) in &state_enter_arg_types {
                if types.is_empty() {
                    continue;
                }
                serialize_helper.push_str(&format!(
                    "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
                    state_name
                ));
                for (i, t) in types.iter().enumerate() {
                    let val_expr = format!("comp->enter_args->items[{}]", i);
                    serialize_helper.push_str(&format!(
                        "            if ({i} < comp->enter_args->size) cJSON_AddItemToArray(ea, {pack});\n",
                        i = i,
                        pack = c_pack_for(t, &val_expr, &system.name)
                    ));
                }
                serialize_helper.push_str("        }\n");
            }
            serialize_helper.push_str("    }\n");
            serialize_helper.push_str("    cJSON_AddItemToObject(obj, \"enter_args\", ea);\n");
            // Recursively serialize parent
            serialize_helper.push_str(&format!("    cJSON_AddItemToObject(obj, \"parent_compartment\", {}_serialize_compartment(comp->parent_compartment));\n", system.name));
            serialize_helper.push_str("    return obj;\n");
            serialize_helper.push_str("}\n\n");

            // Generate deserialize_compartment helper
            let mut deserialize_helper = String::new();
            deserialize_helper.push_str(&format!(
                "static {}_Compartment* {}_deserialize_compartment(cJSON* data) {{\n",
                system.name, system.name
            ));
            deserialize_helper.push_str("    if (!data || cJSON_IsNull(data)) return NULL;\n");
            deserialize_helper
                .push_str("    cJSON* state_item = cJSON_GetObjectItem(data, \"state\");\n");
            // strdup the state string since cJSON memory will be freed
            deserialize_helper.push_str(&format!(
                "    {}_Compartment* comp = {}_Compartment_new(strdup(state_item->valuestring));\n",
                system.name, system.name
            ));
            // Deserialize state_vars
            deserialize_helper
                .push_str("    cJSON* vars = cJSON_GetObjectItem(data, \"state_vars\");\n");
            deserialize_helper.push_str("    if (vars) {\n");
            deserialize_helper.push_str("        cJSON* var_item;\n");
            deserialize_helper.push_str("        cJSON_ArrayForEach(var_item, vars) {\n");
            deserialize_helper.push_str(&format!("            {}_FrameDict_set(comp->state_vars, var_item->string, (void*)(intptr_t)(int)var_item->valuedouble);\n", system.name));
            deserialize_helper.push_str("        }\n");
            deserialize_helper.push_str("    }\n");
            // Symbol-mangled dispatch (mirrors c_pack_for above).
            let c_unpack_for = |frame_type: &str, json_expr: &str, sys_name: &str| -> String {
                format!(
                    "{sys_name}_persist_unpack_{m}({json_expr})",
                    m = c_mangle_type(frame_type)
                )
            };
            deserialize_helper
                .push_str("    cJSON* sa = cJSON_GetObjectItem(data, \"state_args\");\n");
            deserialize_helper.push_str("    if (sa) {\n");
            for (state_name, types) in &state_arg_types {
                if types.is_empty() {
                    continue;
                }
                deserialize_helper.push_str(&format!(
                    "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
                    state_name
                ));
                for (i, t) in types.iter().enumerate() {
                    deserialize_helper.push_str(&format!(
                        "            cJSON* sa_item_{i} = cJSON_GetArrayItem(sa, {i});\n"
                    ));
                    deserialize_helper.push_str(&format!(
                        "            if (sa_item_{i}) {sys}_FrameVec_push(comp->state_args, {unpack});\n",
                        i = i,
                        sys = system.name,
                        unpack = c_unpack_for(t, &format!("sa_item_{i}"), &system.name)
                    ));
                }
                deserialize_helper.push_str("        }\n");
            }
            deserialize_helper.push_str("    }\n");
            // Deserialize enter_args (positional)
            deserialize_helper
                .push_str("    cJSON* ea = cJSON_GetObjectItem(data, \"enter_args\");\n");
            deserialize_helper.push_str("    if (ea) {\n");
            for (state_name, types) in &state_enter_arg_types {
                if types.is_empty() {
                    continue;
                }
                deserialize_helper.push_str(&format!(
                    "        if (strcmp(comp->state, \"{}\") == 0) {{\n",
                    state_name
                ));
                for (i, t) in types.iter().enumerate() {
                    deserialize_helper.push_str(&format!(
                        "            cJSON* ea_item_{i} = cJSON_GetArrayItem(ea, {i});\n"
                    ));
                    deserialize_helper.push_str(&format!(
                        "            if (ea_item_{i}) {sys}_FrameVec_push(comp->enter_args, {unpack});\n",
                        i = i,
                        sys = system.name,
                        unpack = c_unpack_for(t, &format!("ea_item_{i}"), &system.name)
                    ));
                }
                deserialize_helper.push_str("        }\n");
            }
            deserialize_helper.push_str("    }\n");
            // Recursively deserialize parent
            deserialize_helper.push_str(
                "    cJSON* parent = cJSON_GetObjectItem(data, \"parent_compartment\");\n",
            );
            deserialize_helper.push_str(&format!(
                "    comp->parent_compartment = {}_deserialize_compartment(parent);\n",
                system.name
            ));
            deserialize_helper.push_str("    return comp;\n");
            deserialize_helper.push_str("}\n\n");

            // Add helper functions as a NativeBlock method (will be output before other methods)
            methods.push(CodegenNode::NativeBlock {
                code: serialize_helper + &deserialize_helper,
                span: None,
            });

            // Generate save_state function - returns char* (JSON string, caller must free)
            let mut save_body = String::new();
            // Quiescent contract (E700): C uses fprintf+abort since
            // standard error returns are runtime-defined and the caller
            // signature is char*. Aligns with assert-style fatal in
            // other no-exception backends. See RFC-0012.
            save_body.push_str(&format!(
                "if ({0}_FrameVec_size(self->_context_stack) > 0) {{ fprintf(stderr, \"E700: system not quiescent\\n\"); abort(); }}\n",
                system.name
            ));
            save_body.push_str("cJSON* root = cJSON_CreateObject();\n");
            // Serialize entire compartment chain
            save_body.push_str(&format!("cJSON_AddItemToObject(root, \"_compartment\", {}_serialize_compartment(self->__compartment));\n", system.name));

            // Serialize state stack — full compartment chain per entry,
            // matching the current-compartment serialization shape so
            // pop after restore restores state_args/enter_args/state_vars.
            save_body.push_str("cJSON* stack_arr = cJSON_CreateArray();\n");
            save_body.push_str(&format!(
                "for (int i = 0; i < {}_FrameVec_size(self->_state_stack); i++) {{\n",
                system.name
            ));
            save_body.push_str(&format!("    {}_Compartment* comp = ({}_Compartment*){}_FrameVec_get(self->_state_stack, i);\n",
                system.name, system.name, system.name));
            save_body.push_str(&format!(
                "    cJSON_AddItemToArray(stack_arr, {}_serialize_compartment(comp));\n",
                system.name
            ));
            save_body.push_str("}\n");
            save_body.push_str("cJSON_AddItemToObject(root, \"_state_stack\", stack_arr);\n");

            // Serialize domain variables. Nested @@SystemName() instances
            // round-trip via the child's save_state — parse the returned
            // JSON string, embed as a sub-object, free the heap string.
            for var in &system.domain {
                // RFC-0016.1: `@@[no_persist]` fields are transient — skip.
                if var.attributes.iter().any(|a| a.name == "no_persist") {
                    continue;
                }
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    save_body.push_str(&format!(
                        "if (self->{name}) {{\n\
                         \x20   char* __child_json_{name} = {child}_save_state(self->{name});\n\
                         \x20   cJSON* __child_obj_{name} = cJSON_Parse(__child_json_{name});\n\
                         \x20   cJSON_AddItemToObject(root, \"{name}\", __child_obj_{name});\n\
                         \x20   free(__child_json_{name});\n\
                         }} else {{\n\
                         \x20   cJSON_AddNullToObject(root, \"{name}\");\n\
                         }}\n",
                        name = var.name,
                        child = child_sys
                    ));
                    continue;
                }
                // Type-ignorant: framec mangles the declared type to a
                // symbol suffix and dispatches to the runtime's
                // `<sys>_persist_pack_field_<m>(&self->x)` helper — no
                // int-vs-str-vs-… branch here (see
                // docs/contributing/type-ignorant-codegen.md). The
                // field-form helpers take `&self->x` (a pointer to the
                // statically-typed field), so nothing has to cast.
                let type_str = type_to_string(&var.var_type);
                save_body.push_str(&format!(
                    "cJSON_AddItemToObject(root, \"{name}\", {sys}_persist_pack_field_{m}((void*)&self->{name}));\n",
                    name = var.name,
                    sys = system.name,
                    m = c_mangle_type(&type_str)
                ));
            }

            save_body.push_str("char* json = cJSON_PrintUnformatted(root);\n");
            save_body.push_str("cJSON_Delete(root);\n");
            save_body.push_str("return json;");

            methods.push(CodegenNode::Method {
                name: save_method_name.clone(),
                params: vec![],
                return_type: Some("char*".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // Load function — instance method (new contract, takes
            // `Sys* self` as the implicit first arg via is_static=
            // false) or static factory (legacy, returns `Sys*`).
            // Frame's C codegen translates `is_static=false` into a
            // function with `Sys*` first parameter; we use that for
            // both shapes but the body and return shape differ.
            let mut restore_body = String::new();
            restore_body.push_str(&format!(
                "cJSON* root = cJSON_Parse({});\n",
                load_param_name
            ));
            // Legacy: NULL on parse error (returning function);
            // new contract: just bail without writing.
            if uses_new_contract {
                restore_body.push_str("if (!root) return;\n\n");
            } else {
                restore_body.push_str("if (!root) return NULL;\n\n");
            }

            // Legacy: malloc a fresh `Sys*` and init its lists. New
            // contract: `self` is already constructed; we'll
            // overwrite its compartment + state stack below.
            if !uses_new_contract {
                restore_body.push_str(&format!(
                    "{}* instance = malloc(sizeof({}));\n",
                    system.name, system.name
                ));
                restore_body.push_str(&format!(
                    "instance->_state_stack = {}_FrameVec_new();\n",
                    system.name
                ));
                restore_body.push_str(&format!(
                    "instance->_context_stack = {}_FrameVec_new();\n",
                    system.name
                ));
            }
            restore_body.push_str(&format!("{}->__next_compartment = NULL;\n\n", target));

            // Restore entire compartment chain
            restore_body
                .push_str("cJSON* comp_data = cJSON_GetObjectItem(root, \"_compartment\");\n");
            restore_body.push_str(&format!(
                "{}->__compartment = {}_deserialize_compartment(comp_data);\n\n",
                target, system.name
            ));

            // Restore state stack — uses deserialize_compartment so each
            // entry gets its full state_args / enter_args / state_vars
            // restored, matching the save side.
            restore_body
                .push_str("cJSON* stack_arr = cJSON_GetObjectItem(root, \"_state_stack\");\n");
            restore_body.push_str("if (stack_arr) {\n");
            restore_body.push_str("    cJSON* stack_item;\n");
            restore_body.push_str("    cJSON_ArrayForEach(stack_item, stack_arr) {\n");
            restore_body.push_str(&format!(
                "        {}_Compartment* comp = {}_deserialize_compartment(stack_item);\n",
                system.name, system.name
            ));
            restore_body.push_str("        if (comp) {\n");
            restore_body.push_str(&format!(
                "            {}_FrameVec_push({}->_state_stack, comp);\n",
                system.name, target
            ));
            restore_body.push_str("        }\n");
            restore_body.push_str("    }\n");
            restore_body.push_str("}\n\n");

            // Restore domain variables. Nested @@SystemName() instances
            // round-trip via the child's restore_state — print the
            // sub-object back to a JSON string, hand it to the child
            // factory, then free the temporary string.
            for var in &system.domain {
                // RFC-0016.1: `@@[no_persist]` fields aren't in the blob.
                if var.attributes.iter().any(|a| a.name == "no_persist") {
                    continue;
                }
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    let body = if nested_uses_new_contract(child_sys) {
                        format!(
                            "{{\n\
                             \x20   cJSON* __child_obj_{name} = cJSON_GetObjectItem(root, \"{name}\");\n\
                             \x20   if (__child_obj_{name} && !cJSON_IsNull(__child_obj_{name})) {{\n\
                             \x20       char* __child_json_{name} = cJSON_PrintUnformatted(__child_obj_{name});\n\
                             \x20       {tgt}->{name} = {child}_new();\n\
                             \x20       {child}_restore_state({tgt}->{name}, __child_json_{name});\n\
                             \x20       free(__child_json_{name});\n\
                             \x20   }} else {{\n\
                             \x20       {tgt}->{name} = NULL;\n\
                             \x20   }}\n\
                             }}\n",
                            tgt = target,
                            name = var.name,
                            child = child_sys
                        )
                    } else {
                        format!(
                            "{{\n\
                             \x20   cJSON* __child_obj_{name} = cJSON_GetObjectItem(root, \"{name}\");\n\
                             \x20   if (__child_obj_{name} && !cJSON_IsNull(__child_obj_{name})) {{\n\
                             \x20       char* __child_json_{name} = cJSON_PrintUnformatted(__child_obj_{name});\n\
                             \x20       {tgt}->{name} = {child}_restore_state(__child_json_{name});\n\
                             \x20       free(__child_json_{name});\n\
                             \x20   }} else {{\n\
                             \x20       {tgt}->{name} = NULL;\n\
                             \x20   }}\n\
                             }}\n",
                            tgt = target,
                            name = var.name,
                            child = child_sys
                        )
                    };
                    restore_body.push_str(&body);
                    continue;
                }
                // Type-ignorant: mirror of the pack side — dispatch to
                // `<sys>_persist_unpack_field_<m>(json, &self->x)`,
                // which writes through the typed pointer.
                let type_str = type_to_string(&var.var_type);
                restore_body.push_str(&format!(
                    "{sys}_persist_unpack_field_{m}(cJSON_GetObjectItem(root, \"{name}\"), (void*)&{tgt}->{name});\n",
                    sys = system.name,
                    m = c_mangle_type(&type_str),
                    name = var.name,
                    tgt = target
                ));
            }

            restore_body.push_str("\ncJSON_Delete(root);\n");
            if !uses_new_contract {
                restore_body.push_str("return instance;");
            }

            let (load_return, load_static) = if uses_new_contract {
                // Instance method (takes Sys* self implicitly).
                (None, false)
            } else {
                // Static factory: returns Sys*.
                (Some(format!("{}*", system.name)), true)
            };
            methods.push(CodegenNode::Method {
                name: load_method_name.clone(),
                params: vec![Param::new(&load_param_name).with_type("const char*")],
                return_type: load_return,
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: load_static,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Cpp => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // RFC-0012 amendment: branch on new contract.
            let uses_new_contract = system.uses_new_persist_contract();
            let save_method_name = system
                .save_op_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "save_state".to_string());
            let load_method_name = system
                .load_op_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "restore_state".to_string());
            let load_param_name = system
                .load_op_param_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "json".to_string());
            let target = if uses_new_contract {
                "(*this)"
            } else {
                "__instance"
            };

            // Collect all state vars with their types for serialization
            let all_state_vars: Vec<(&str, &str, &str)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .flat_map(|s| {
                            s.state_vars.iter().map(move |sv| {
                                let type_str = match &sv.var_type {
                                    crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                                        t.as_str()
                                    }
                                    crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
                                };
                                (s.name.as_str(), sv.name.as_str(), type_str)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            // save_state() — recursive compartment chain serialization
            let mut save_body = String::new();
            // Quiescent contract (E700): see RFC-0012.
            save_body.push_str("if (!_context_stack.empty()) throw std::runtime_error(\"E700: system not quiescent\");\n");

            // Helper lambda to serialize a compartment and its parent chain
            save_body.push_str(&format!(
                "std::function<nlohmann::json(const {0}*)> __ser = [&](const {0}* c) -> nlohmann::json {{\n",
                compartment_class
            ));
            save_body.push_str("    if (!c) return nullptr;\n");
            save_body.push_str("    nlohmann::json __cj;\n");
            save_body.push_str("    __cj[\"state\"] = c->state;\n");
            save_body.push_str("    nlohmann::json __sv;\n");
            save_body.push_str("    for (auto& [k, v] : c->state_vars) {\n");
            for (_state, var_name, var_type) in &all_state_vars {
                let cpp_type = cpp_map_type(var_type);
                save_body.push_str(&format!(
                    "        if (k == \"{}\") {{ try {{ __sv[k] = std::any_cast<{}>(v); }} catch(...) {{}} }}\n",
                    var_name, cpp_type
                ));
            }
            save_body.push_str("    }\n");
            save_body.push_str("    __cj[\"state_vars\"] = __sv;\n");
            // state_args + enter_args — same compartment-context fix
            // pattern as Java/Kotlin/C#/Swift. C++ uses std::any so
            // best-effort cast to int (the most common arg type); other
            // types pass through as null which is a known limitation
            // pending a richer std::any-aware serializer.
            // Try int first, then double — covers the common Frame
            // numeric types. Strings/bools/other any-shaped values
            // currently pass through as null (limitation of std::any
            // roundtrip without a richer type tag). D8 fix added the
            // Type-ignorant: framec emits the declared type `T`
            // verbatim into `std::any_cast<T>` and `nlohmann::json
            // ::get<T>()`; nlohmann's ADL handles primitives, std::
            // vector, std::map, std::unordered_map, std::string, and
            // user types with to_json/from_json. No type-string
            // parsing in framec.
            let cpp_state_arg_decls: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .map(|s| {
                            let types: Vec<String> = s
                                .params
                                .iter()
                                .map(|p| match &p.param_type {
                                    crate::frame_c::compiler::frame_ast::Type::Custom(s) => {
                                        s.clone()
                                    }
                                    crate::frame_c::compiler::frame_ast::Type::Unknown => {
                                        String::new()
                                    }
                                })
                                .collect();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();
            save_body.push_str("    nlohmann::json __sa = nlohmann::json::array();\n");
            save_body.push_str("    {\n");
            let mut any_state_arg_branch = false;
            // Type-ignorant typed serialize. framec emits the
            // declared type `T` verbatim into `std::any_cast<T>` and
            // wraps the result in `nlohmann::json(...)`. nlohmann's
            // ADL handles primitives, std::vector, std::map,
            // std::unordered_map, std::string, user types with
            // to_json overloads — without framec naming any.
            for (state_name, types) in &cpp_state_arg_decls {
                if types.is_empty() {
                    continue;
                }
                if !any_state_arg_branch {
                    any_state_arg_branch = true;
                }
                save_body.push_str(&format!("    if (c->state == \"{}\") {{\n", state_name));
                for (i, t) in types.iter().enumerate() {
                    if t.is_empty() {
                        save_body.push_str(&format!(
                            "        if (c->state_args.size() > {i}) {{ try {{ __sa.push_back(std::any_cast<int>(c->state_args[{i}])); }} catch(...) {{ try {{ __sa.push_back(std::any_cast<double>(c->state_args[{i}])); }} catch(...) {{ __sa.push_back(nullptr); }} }} }}\n"
                        ));
                    } else {
                        save_body.push_str(&format!(
                            "        if (c->state_args.size() > {i}) {{ try {{ __sa.push_back(nlohmann::json(std::any_cast<{t}>(c->state_args[{i}]))); }} catch(...) {{ __sa.push_back(nullptr); }} }}\n"
                        ));
                    }
                }
                save_body.push_str("    } else \n");
            }
            // Generic fallback for states without state_args at all.
            save_body.push_str("    {\n");
            save_body.push_str("        for (const auto& v : c->state_args) { try { __sa.push_back(std::any_cast<int>(v)); } catch(...) { try { __sa.push_back(std::any_cast<double>(v)); } catch(...) { __sa.push_back(nullptr); } } }\n");
            save_body.push_str("    }\n");
            save_body.push_str("    }\n");
            save_body.push_str("    __cj[\"state_args\"] = __sa;\n");
            // D13 fix: per-state typed enter_args (mirror of state_args
            // path above). Without this, declared `$>(items: vector<T>)`
            // round-trips as int/double-only and a user that reads
            // __compartment->enter_args[i] post-restore hits bad_any_cast.
            let cpp_enter_arg_decls: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .map(|s| {
                            let types: Vec<String> = s
                                .enter
                                .as_ref()
                                .map(|e| {
                                    e.params
                                        .iter()
                                        .map(|p| match &p.param_type {
                                            crate::frame_c::compiler::frame_ast::Type::Custom(
                                                s,
                                            ) => s.clone(),
                                            crate::frame_c::compiler::frame_ast::Type::Unknown => {
                                                String::new()
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();
            save_body.push_str("    nlohmann::json __ea = nlohmann::json::array();\n");
            save_body.push_str("    {\n");
            let mut any_enter_branch = false;
            for (state_name, types) in &cpp_enter_arg_decls {
                if types.is_empty() {
                    continue;
                }
                if !any_enter_branch {
                    any_enter_branch = true;
                }
                save_body.push_str(&format!("    if (c->state == \"{}\") {{\n", state_name));
                for (i, t) in types.iter().enumerate() {
                    if t.is_empty() {
                        save_body.push_str(&format!(
                            "        if (c->enter_args.size() > {i}) {{ try {{ __ea.push_back(std::any_cast<int>(c->enter_args[{i}])); }} catch(...) {{ try {{ __ea.push_back(std::any_cast<double>(c->enter_args[{i}])); }} catch(...) {{ __ea.push_back(nullptr); }} }} }}\n"
                        ));
                    } else {
                        save_body.push_str(&format!(
                            "        if (c->enter_args.size() > {i}) {{ try {{ __ea.push_back(nlohmann::json(std::any_cast<{t}>(c->enter_args[{i}]))); }} catch(...) {{ __ea.push_back(nullptr); }} }}\n"
                        ));
                    }
                }
                save_body.push_str("    } else\n");
            }
            save_body.push_str("    {\n");
            save_body.push_str("        for (const auto& v : c->enter_args) { try { __ea.push_back(std::any_cast<int>(v)); } catch(...) { try { __ea.push_back(std::any_cast<double>(v)); } catch(...) { __ea.push_back(nullptr); } } }\n");
            save_body.push_str("    }\n");
            save_body.push_str("    }\n");
            save_body.push_str("    __cj[\"enter_args\"] = __ea;\n");
            save_body.push_str("    __cj[\"parent\"] = __ser(c->parent_compartment.get());\n");
            save_body.push_str("    return __cj;\n");
            save_body.push_str("};\n");

            save_body.push_str("nlohmann::json __j;\n");
            save_body.push_str("__j[\"_compartment\"] = __ser(__compartment.get());\n");

            // Serialize state stack
            save_body.push_str("nlohmann::json __stack = nlohmann::json::array();\n");
            save_body
                .push_str("for (auto& c : _state_stack) { __stack.push_back(__ser(c.get())); }\n");
            save_body.push_str("__j[\"_state_stack\"] = __stack;\n");

            // Serialize domain vars. Nested @@SystemName() instances
            // round-trip via child save_state (recursing into JSON).
            // Multi-system C++ wraps nested instances in shared_ptr<T>,
            // so access uses arrow notation.
            for var in &system.domain {
                // RFC-0016.1: `@@[no_persist]` fields are transient — skip.
                if var.attributes.iter().any(|a| a.name == "no_persist") {
                    continue;
                }
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "__j[\"{0}\"] = {0} ? nlohmann::json::parse({0}->save_state()) : nlohmann::json(nullptr);\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("__j[\"{}\"] = {};\n", var.name, var.name));
                }
            }

            save_body.push_str("return __j.dump();");

            methods.push(CodegenNode::Method {
                name: save_method_name.clone(),
                params: vec![],
                return_type: Some("std::string".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // Load body — instance method (new contract) or static
            // factory (legacy with __skipInitialEnter dance around
            // value-init `T __instance;`).
            let mut restore_body = String::new();

            // Helper lambda to deserialize compartment chain recursively
            restore_body.push_str(&format!(
                "std::function<std::unique_ptr<{0}>(const nlohmann::json&)> __deser = [&](const nlohmann::json& d) -> std::unique_ptr<{0}> {{\n",
                compartment_class
            ));
            restore_body.push_str("    if (d.is_null()) return nullptr;\n");
            restore_body.push_str(&format!(
                "    auto c = std::make_unique<{}>(std::string(d[\"state\"]));\n",
                compartment_class
            ));
            restore_body.push_str("    if (d.contains(\"state_vars\")) {\n");
            restore_body.push_str("        auto& sv = d[\"state_vars\"];\n");
            for (_state, var_name, var_type) in &all_state_vars {
                let cpp_type = cpp_map_type(var_type);
                restore_body.push_str(&format!(
                    "        if (sv.contains(\"{0}\")) {{ c->state_vars[\"{0}\"] = std::any(sv[\"{0}\"].get<{1}>()); }}\n",
                    var_name, cpp_type
                ));
            }
            restore_body.push_str("    }\n");
            // Deserialize state_args + enter_args. Branch on JSON
            // numeric kind: integer → std::any(int); float → std::any(
            // double). D8 fix added the double path; without it,
            // float state-args lost precision and the per-handler
            // any_cast<double> threw bad_any_cast.
            // D10: per-state typed deserialize for vector args.
            // For each state with a `std::vector<T>` declared
            // state-arg, branch on `c->state` and rebuild the vector
            // from the JSON array. Falls through to the generic
            // int/double scalar path for non-list states.
            // Type-ignorant typed restore. framec emits the declared
            // type `T` verbatim; nlohmann::json's `get<T>()` handles
            // any T it can deserialize (primitives, std::vector,
            // std::map, std::string, user types with from_json).
            restore_body.push_str(
                "    if (d.contains(\"state_args\") && d[\"state_args\"].is_array()) {\n",
            );
            restore_body.push_str("        const auto& __sa = d[\"state_args\"];\n");
            let mut any_typed_state = false;
            for (state_name, types) in &cpp_state_arg_decls {
                if types.is_empty() {
                    continue;
                }
                if !any_typed_state {
                    any_typed_state = true;
                }
                restore_body.push_str(&format!("        if (c->state == \"{}\") {{\n", state_name));
                for (i, t) in types.iter().enumerate() {
                    if t.is_empty() {
                        restore_body.push_str(&format!(
                            "            if (__sa.size() > {i}) {{ if (__sa[{i}].is_number_integer()) c->state_args.push_back(std::any(__sa[{i}].get<int>())); else if (__sa[{i}].is_number_float()) c->state_args.push_back(std::any(__sa[{i}].get<double>())); }}\n"
                        ));
                    } else {
                        restore_body.push_str(&format!(
                            "            if (__sa.size() > {i}) {{ try {{ c->state_args.push_back(std::any(__sa[{i}].get<{t}>())); }} catch(...) {{ }} }}\n"
                        ));
                    }
                }
                restore_body.push_str("        } else \n");
            }
            // Fallback for states without typed state-args.
            restore_body.push_str("        {\n");
            restore_body.push_str("            for (const auto& v : __sa) {\n");
            restore_body.push_str("                if (v.is_number_integer()) c->state_args.push_back(std::any(v.get<int>()));\n");
            restore_body.push_str("                else if (v.is_number_float()) c->state_args.push_back(std::any(v.get<double>()));\n");
            restore_body.push_str("            }\n");
            restore_body.push_str("        }\n");
            restore_body.push_str("    }\n");
            // D13 fix: per-state typed enter_args (mirror state_args).
            restore_body.push_str(
                "    if (d.contains(\"enter_args\") && d[\"enter_args\"].is_array()) {\n",
            );
            restore_body.push_str("        const auto& __ea = d[\"enter_args\"];\n");
            let mut any_typed_enter = false;
            for (state_name, types) in &cpp_enter_arg_decls {
                if types.is_empty() {
                    continue;
                }
                if !any_typed_enter {
                    any_typed_enter = true;
                }
                restore_body.push_str(&format!("        if (c->state == \"{}\") {{\n", state_name));
                for (i, t) in types.iter().enumerate() {
                    if t.is_empty() {
                        restore_body.push_str(&format!(
                            "            if (__ea.size() > {i}) {{ if (__ea[{i}].is_number_integer()) c->enter_args.push_back(std::any(__ea[{i}].get<int>())); else if (__ea[{i}].is_number_float()) c->enter_args.push_back(std::any(__ea[{i}].get<double>())); }}\n"
                        ));
                    } else {
                        restore_body.push_str(&format!(
                            "            if (__ea.size() > {i}) {{ try {{ c->enter_args.push_back(std::any(__ea[{i}].get<{t}>())); }} catch(...) {{ }} }}\n"
                        ));
                    }
                }
                restore_body.push_str("        } else \n");
            }
            restore_body.push_str("        {\n");
            restore_body.push_str("            for (const auto& v : __ea) {\n");
            restore_body.push_str("                if (v.is_number_integer()) c->enter_args.push_back(std::any(v.get<int>()));\n");
            restore_body.push_str("                else if (v.is_number_float()) c->enter_args.push_back(std::any(v.get<double>()));\n");
            restore_body.push_str("            }\n");
            restore_body.push_str("        }\n");
            restore_body.push_str("    }\n");
            restore_body
                .push_str("    if (d.contains(\"parent\") && !d[\"parent\"].is_null()) {\n");
            restore_body.push_str("        c->parent_compartment = __deser(d[\"parent\"]);\n");
            restore_body.push_str("    }\n");
            restore_body.push_str("    return c;\n");
            restore_body.push_str("};\n");

            restore_body.push_str(&format!(
                "auto __j = nlohmann::json::parse({});\n",
                load_param_name
            ));
            // RFC-0017 Phase A7: legacy `__skipInitialEnter`-based
            // restore path removed. The new persist contract (RFC-0012
            // amendment; E814 hard-cut) mutates `*this` in place.
            let _ = uses_new_contract;
            restore_body.push_str(&format!(
                "{}.__compartment = __deser(__j[\"_compartment\"]);\n",
                target
            ));

            // Restore state stack
            restore_body.push_str("if (__j.contains(\"_state_stack\")) {\n");
            restore_body.push_str("    for (auto& __sc : __j[\"_state_stack\"]) {\n");
            restore_body.push_str(&format!(
                "        {}._state_stack.push_back(__deser(__sc));\n",
                target
            ));
            restore_body.push_str("    }\n");
            restore_body.push_str("}\n");

            // Restore domain vars. Nested @@SystemName() instances
            // re-hydrate via child restore_state.
            for var in &system.domain {
                // RFC-0016.1: `@@[no_persist]` fields aren't in the blob.
                if var.attributes.iter().any(|a| a.name == "no_persist") {
                    continue;
                }
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    if nested_uses_new_contract(child_sys) {
                        restore_body.push_str(&format!(
                            "if (__j.contains(\"{0}\") && !__j[\"{0}\"].is_null()) {{ {tgt}.{0} = std::make_shared<{1}>(); {tgt}.{0}->restore_state(__j[\"{0}\"].dump()); }}\n",
                            var.name, child_sys, tgt = target
                        ));
                    } else {
                        restore_body.push_str(&format!(
                            "if (__j.contains(\"{0}\") && !__j[\"{0}\"].is_null()) {{ {tgt}.{0} = std::make_shared<{1}>({1}::restore_state(__j[\"{0}\"].dump())); }}\n",
                            var.name, child_sys, tgt = target
                        ));
                    }
                } else {
                    restore_body.push_str(&format!(
                        "if (__j.contains(\"{0}\")) {{ __j[\"{0}\"].get_to({tgt}.{0}); }}\n",
                        var.name,
                        tgt = target
                    ));
                }
            }

            if !uses_new_contract {
                restore_body.push_str("return __instance;");
            }

            let (load_return, load_static) = if uses_new_contract {
                (None, false)
            } else {
                (Some(sys.clone()), true)
            };
            methods.push(CodegenNode::Method {
                name: load_method_name.clone(),
                params: vec![Param::new(&load_param_name).with_type("const std::string&")],
                return_type: load_return,
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: load_static,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Java => methods.extend(persist::java::generate(system)),
        TargetLanguage::CSharp => methods.extend(persist::csharp::generate(system)),
        TargetLanguage::Php => methods.extend(persist::php::generate(system)),
        TargetLanguage::Kotlin => methods.extend(persist::kotlin::generate(system)),
        TargetLanguage::Swift => methods.extend(persist::swift::generate(system)),
        TargetLanguage::Ruby => methods.extend(persist::ruby::generate(system)),
        TargetLanguage::Go => {
            let compartment_type = format!("{}Compartment", system.name);

            // RFC-0012 amendment: branch on new contract. Go uses
            // PascalCase legacy: SaveState (method on s) and
            // RestoreSysName (package function returning *SysName).
            // New contract makes both receiver methods on `s`.
            let uses_new_contract = system.uses_new_persist_contract();
            let save_method_name = system
                .save_op_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "SaveState".to_string());
            let load_method_name = system
                .load_op_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("Restore{}", system.name));
            let load_param_name = system
                .load_op_param_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "jsonStr".to_string());
            // Go: methods on the receiver use `s.` (the standard
            // receiver name framec emits); package-level factory
            // populates a fresh `instance := &Sys{}` via `instance.`.
            let target = if uses_new_contract { "s" } else { "instance" };

            // save_state — serialize to JSON via encoding/json
            let mut save_body = String::new();
            // Quiescent contract (E700): see RFC-0012.
            save_body.push_str(
                "if len(s._context_stack) > 0 { panic(\"E700: system not quiescent\") }\n",
            );
            save_body.push_str(&format!(
                "var serializeComp func(c *{}) interface{{}}\n",
                compartment_type
            ));
            save_body.push_str(&format!(
                "serializeComp = func(c *{}) interface{{}} {{\n",
                compartment_type
            ));
            save_body.push_str("    if c == nil { return nil }\n");
            save_body.push_str("    return map[string]interface{}{\n");
            save_body.push_str("        \"state\": c.state,\n");
            save_body.push_str("        \"state_args\": c.stateArgs,\n");
            save_body.push_str("        \"state_vars\": c.stateVars,\n");
            save_body.push_str("        \"enter_args\": c.enterArgs,\n");
            save_body.push_str("        \"exit_args\": c.exitArgs,\n");
            save_body.push_str("        \"forward_event\": c.forwardEvent,\n");
            save_body
                .push_str("        \"parent_compartment\": serializeComp(c.parentCompartment),\n");
            save_body.push_str("    }\n");
            save_body.push_str("}\n");
            save_body.push_str("data := map[string]interface{}{\n");
            save_body.push_str("    \"_compartment\": serializeComp(s.__compartment),\n");
            save_body.push_str("    \"_state_stack\": func() []interface{} {\n");
            save_body.push_str("        var arr []interface{}\n");
            save_body.push_str("        for _, c := range s._state_stack { arr = append(arr, serializeComp(c)) }\n");
            save_body.push_str("        return arr\n");
            save_body.push_str("    }(),\n");
            for var in &system.domain {
                // RFC-0016.1: `@@[no_persist]` fields are transient — skip.
                if var.attributes.iter().any(|a| a.name == "no_persist") {
                    continue;
                }
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    // Nested @@SystemName(): recurse into child save_state.
                    save_body.push_str(&format!(
                        "    \"{0}\": func() interface{{}} {{ var __raw interface{{}}; _ = json.Unmarshal([]byte(s.{0}.SaveState()), &__raw); return __raw }}(),\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("    \"{}\": s.{},\n", var.name, var.name));
                }
            }
            save_body.push_str("}\n");
            save_body.push_str("jsonBytes, _ := json.Marshal(data)\n");
            save_body.push_str("return string(jsonBytes)");

            methods.push(CodegenNode::Method {
                name: save_method_name.clone(),
                params: vec![],
                return_type: Some("string".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // Load body — receiver method (new contract) or package
            // factory (legacy). Go's `is_static=true` here means
            // "package-level free function with no receiver"; `false`
            // means "method with `(s *Sys)` receiver".
            let mut restore_body = String::new();
            // Rename internal var to `_parsed` so the user's load-op
            // param can also be named `data` (RFC-0012 amendment).
            restore_body.push_str("var _parsed map[string]interface{}\n");
            restore_body.push_str(&format!(
                "json.Unmarshal([]byte({}), &_parsed)\n",
                load_param_name
            ));
            restore_body.push_str(&format!(
                "var deserializeComp func(d interface{{}}) *{}\n",
                compartment_type
            ));
            restore_body.push_str(&format!(
                "deserializeComp = func(d interface{{}}) *{} {{\n",
                compartment_type
            ));
            restore_body.push_str("    if d == nil { return nil }\n");
            restore_body.push_str("    m := d.(map[string]interface{})\n");
            restore_body.push_str(&format!(
                "    comp := new{}Compartment(m[\"state\"].(string))\n",
                system.name
            ));
            // state_vars is a map (keyed by $.varName); state_args / enter_args /
            // exit_args are positional slices (Vec of interface{}).
            //
            // JSON numbers come back as float64 in Go's encoding/json default;
            // convert whole-number float64 → int so type assertions in
            // generated handlers (e.g. `slot := stateArgs[0].(int)`) work.
            restore_body.push_str("    normalizeNum := func(v interface{}) interface{} {\n");
            restore_body.push_str(
                "        if f, ok := v.(float64); ok && f == float64(int(f)) { return int(f) }\n",
            );
            restore_body.push_str("        return v\n");
            restore_body.push_str("    }\n");
            restore_body.push_str("    if sv, ok := m[\"state_vars\"].(map[string]interface{}); ok { for k, v := range sv { comp.stateVars[k] = normalizeNum(v) } }\n");
            restore_body.push_str("    if sa, ok := m[\"state_args\"].([]interface{}); ok { for _, v := range sa { comp.stateArgs = append(comp.stateArgs, normalizeNum(v)) } }\n");
            restore_body.push_str("    if ea, ok := m[\"enter_args\"].([]interface{}); ok { for _, v := range ea { comp.enterArgs = append(comp.enterArgs, normalizeNum(v)) } }\n");
            restore_body.push_str("    if xa, ok := m[\"exit_args\"].([]interface{}); ok { for _, v := range xa { comp.exitArgs = append(comp.exitArgs, normalizeNum(v)) } }\n");
            // Type-ignorant typed restore via marshal-roundtrip.
            // Go's encoding/json reflection handles arbitrary types
            // (primitive / slice / map / nested / user struct) when
            // unmarshalling into a typed target. framec emits the
            // declared type verbatim — no parsing of generics, no
            // element-type enumeration, no per-shape branching.
            let go_typed_conv = |declared_type: &str, idx: usize, slot: &str| -> String {
                let t = declared_type.trim();
                if t.is_empty() {
                    return String::new();
                }
                let src = format!("comp.{slot}[{idx}]");
                format!(
                    "    if len(comp.{slot}) > {idx} {{\n\
                     \x20       if __raw, __err := json.Marshal({src}); __err == nil {{\n\
                     \x20           var __typed {t}\n\
                     \x20           if json.Unmarshal(__raw, &__typed) == nil {{\n\
                     \x20               comp.{slot}[{idx}] = __typed\n\
                     \x20           }}\n\
                     \x20       }}\n\
                     \x20   }}\n"
                )
            };
            let go_state_arg_decls: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .map(|s| {
                            let types: Vec<String> = s
                                .params
                                .iter()
                                .map(|p| match &p.param_type {
                                    crate::frame_c::compiler::frame_ast::Type::Custom(s) => {
                                        s.clone()
                                    }
                                    crate::frame_c::compiler::frame_ast::Type::Unknown => {
                                        String::new()
                                    }
                                })
                                .collect();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let go_enter_arg_decls: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .map(|s| {
                            let types: Vec<String> = s
                                .enter
                                .as_ref()
                                .map(|e| {
                                    e.params
                                        .iter()
                                        .map(|p| match &p.param_type {
                                            crate::frame_c::compiler::frame_ast::Type::Custom(
                                                s,
                                            ) => s.clone(),
                                            crate::frame_c::compiler::frame_ast::Type::Unknown => {
                                                String::new()
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (state_name, types) in &go_state_arg_decls {
                let mut branch = String::new();
                for (i, t) in types.iter().enumerate() {
                    let conv = go_typed_conv(t, i, "stateArgs");
                    if !conv.is_empty() {
                        branch.push_str(&conv);
                    }
                }
                if !branch.is_empty() {
                    restore_body.push_str(&format!(
                        "    if comp.state == \"{}\" {{\n{}    }}\n",
                        state_name, branch
                    ));
                }
            }
            for (state_name, types) in &go_enter_arg_decls {
                let mut branch = String::new();
                for (i, t) in types.iter().enumerate() {
                    let conv = go_typed_conv(t, i, "enterArgs");
                    if !conv.is_empty() {
                        branch.push_str(&conv);
                    }
                }
                if !branch.is_empty() {
                    restore_body.push_str(&format!(
                        "    if comp.state == \"{}\" {{\n{}    }}\n",
                        state_name, branch
                    ));
                }
            }
            restore_body.push_str("    // forward_event is typically nil in persisted state\n");
            restore_body.push_str(
                "    comp.parentCompartment = deserializeComp(m[\"parent_compartment\"])\n",
            );
            restore_body.push_str("    return comp\n");
            restore_body.push_str("}\n");
            // Legacy only: allocate a fresh instance struct. New
            // contract mutates `s` (the receiver) in place.
            if !uses_new_contract {
                restore_body.push_str(&format!("instance := &{}{{}}\n", system.name));
            }
            restore_body.push_str(&format!(
                "{}.__compartment = deserializeComp(_parsed[\"_compartment\"])\n",
                target
            ));
            restore_body.push_str(&format!("{}.__next_compartment = nil\n", target));
            restore_body
                .push_str("if stack, ok := _parsed[\"_state_stack\"].([]interface{}); ok {\n");
            restore_body.push_str(&format!(
                "    {}._state_stack = make([]*{}, 0, len(stack))\n",
                target, compartment_type
            ));
            restore_body.push_str(&format!(
                "    for _, c := range stack {{ {0}._state_stack = append({0}._state_stack, deserializeComp(c)) }}\n",
                target
            ));
            restore_body.push_str("}\n");
            for var in &system.domain {
                // RFC-0016.1: `@@[no_persist]` fields aren't in the blob.
                if var.attributes.iter().any(|a| a.name == "no_persist") {
                    continue;
                }
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    if nested_uses_new_contract(child_sys) {
                        restore_body.push_str(&format!(
                            "if __raw_{1}, err_{1} := json.Marshal(_parsed[\"{1}\"]); err_{1} == nil {{ {0}.{1} = New{2}(); {0}.{1}.LoadState(string(__raw_{1})) }}\n",
                            target, var.name, child_sys
                        ));
                    } else {
                        restore_body.push_str(&format!(
                            "if __raw_{1}, err_{1} := json.Marshal(_parsed[\"{1}\"]); err_{1} == nil {{ {0}.{1} = Restore{2}(string(__raw_{1})) }}\n",
                            target, var.name, child_sys
                        ));
                    }
                } else {
                    let declared = match &var.var_type {
                        crate::frame_c::compiler::frame_ast::Type::Custom(name) => {
                            go_map_type(name)
                        }
                        _ => "interface{}".to_string(),
                    };
                    let go_extract = format!(
                        "func() {t} {{ var __typed {t}; if __raw, err := json.Marshal(_parsed[\"{name}\"]); err == nil {{ json.Unmarshal(__raw, &__typed) }}; return __typed }}()",
                        t = declared,
                        name = var.name,
                    );
                    restore_body.push_str(&format!("{}.{} = {}\n", target, var.name, go_extract));
                }
            }
            if !uses_new_contract {
                restore_body.push_str("return instance");
            }

            let (load_return, load_static) = if uses_new_contract {
                (None, false)
            } else {
                (Some(format!("*{}", system.name)), true)
            };
            methods.push(CodegenNode::Method {
                name: load_method_name.clone(),
                params: vec![Param::new(&load_param_name).with_type("string")],
                return_type: load_return,
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: load_static,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Erlang => {
            // Persistence handled in erlang_system.rs via gen_statem save_state/load_state
        }
        TargetLanguage::Dart => methods.extend(persist::dart::generate(system)),
        TargetLanguage::Lua => methods.extend(persist::lua::generate(system)),
        TargetLanguage::GDScript => methods.extend(persist::gdscript::generate(system)),
        TargetLanguage::Graphviz => unreachable!(),
    }

    methods
}

// ============================================================================
// Erlang gen_statem code generation
// ============================================================================
