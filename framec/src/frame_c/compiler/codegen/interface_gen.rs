//! Interface wrapper, action, operation, and persistence code generation.
//!
//! Generates:
//! - Interface method wrappers (public API → kernel dispatch)
//! - Action method bodies (native code with self.X rewriting)
//! - Operation method bodies (static/class methods)
//! - Persistence serialization/deserialization methods

use super::ast::{CodegenNode, Param, Visibility};
use super::codegen_utils::{
    cpp_map_type, cpp_wrap_any_arg, csharp_map_type, expression_to_string, go_map_type,
    is_bool_type, is_float_type, is_int_type, is_string_type, java_map_type, kotlin_map_type,
    swift_map_type, to_snake_case, type_to_cpp_string, type_to_string, HandlerContext,
};
use crate::frame_c::compiler::frame_ast::{
    ActionAst, InterfaceMethod, MethodParam, OperationAst, Span, SystemAst, Type,
};
use crate::frame_c::visitors::TargetLanguage;

/// True for dynamically-typed targets where every function returns
/// *something* regardless of the source's declared return type. The
/// interface method wrapper for these targets always exposes the
/// FrameContext's return slot to the caller — there's no `void` to
/// honor (see docs/frame_runtime.md § "Return values across target
/// languages"). For statically-typed targets, the wrapper conditions
/// on the source's declared return type so it doesn't try to return
/// from a `void`-typed method.
fn is_dynamic_target(lang: TargetLanguage) -> bool {
    matches!(
        lang,
        TargetLanguage::Python3
            | TargetLanguage::JavaScript
            | TargetLanguage::Ruby
            | TargetLanguage::Lua
            | TargetLanguage::Php
            | TargetLanguage::GDScript
            | TargetLanguage::Erlang
    )
}

/// Compute the type-default literal for a Frame return type, per
/// language. Used by interface wrappers to initialize the
/// FrameContext._return slot so that handlers that don't explicitly
/// write @@:return still produce a valid type-default at the wrapper
/// boundary — rather than null/None which crashes typed langs on
/// unboxing (Java/C#) or violates the typed-return contract on
/// dynamic langs (Python/JS/Ruby/Lua/PHP returning None when an
/// `: int` was promised).
///
/// Frame source uses canonical type names: `int`, `str`, `bool`,
/// `float`, `double`, `long`. Each backend maps them to its own
/// type system; defaults follow each language's own zero-value
/// convention (0 for ints, "" for strings, false for booleans).
///
/// Unknown types fall back to the language's null/None — caller
/// can opt out by checking the unspecified-default sentinel
/// (returns None from this function for void returns).
fn frame_return_default(lang: TargetLanguage, type_str: &str) -> String {
    let t = type_str.trim();
    // Common int/string/bool patterns each lang accepts.
    let is_int = matches!(
        t,
        "int" | "Int" | "i32" | "i64" | "long" | "Long" | "Integer"
    );
    let is_str = matches!(
        t,
        "str" | "string" | "String"
    );
    let is_bool = matches!(
        t,
        "bool" | "boolean" | "Boolean"
    );
    let is_float = matches!(
        t,
        "float" | "Float" | "double" | "Double" | "f32" | "f64"
    );

    match lang {
        TargetLanguage::Python3 => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "False".to_string() }
            else if is_float { "0.0".to_string() }
            else { "None".to_string() }
        }
        TargetLanguage::JavaScript | TargetLanguage::TypeScript => {
            if is_int || is_float { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else { "null".to_string() }
        }
        TargetLanguage::Ruby => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "nil".to_string() }
        }
        TargetLanguage::Lua => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "nil".to_string() }
        }
        TargetLanguage::Php => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "null".to_string() }
        }
        TargetLanguage::Java => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "null".to_string() }
        }
        TargetLanguage::CSharp => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "null".to_string() }
        }
        TargetLanguage::Kotlin => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "null".to_string() }
        }
        TargetLanguage::Dart => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "null".to_string() }
        }
        TargetLanguage::GDScript => {
            if is_int { "0".to_string() }
            else if is_str { "\"\"".to_string() }
            else if is_bool { "false".to_string() }
            else if is_float { "0.0".to_string() }
            else { "null".to_string() }
        }
        // Other langs (Rust, Go, C, C++, Swift, Erlang) handle
        // defaults via their own context-init paths; this helper
        // is currently called only by the wrappers above. Return
        // a generic null marker for safety — those backends don't
        // wire it through.
        _ => "null".to_string(),
    }
}

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
    _syntax: &super::backend::ClassSyntax,
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
    let code = extract_body_content(source, &action.body.span);

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

/// Extract body content from source using span
///
/// Strips the outer braces and extracts the inner content while preserving
/// consistent line-by-line indentation for proper re-indentation by backends.
pub(crate) fn extract_body_content(
    source: &[u8],
    span: &crate::frame_c::compiler::frame_ast::Span,
) -> String {
    let bytes = &source[span.start..span.end];
    let content = String::from_utf8_lossy(bytes).to_string();

    // Strip outer braces if present
    let trimmed = content.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        // Extract content between braces
        let inner = &trimmed[1..trimmed.len() - 1];

        // Split into lines, preserving structure
        let lines: Vec<&str> = inner.lines().collect();

        // Skip leading and trailing empty lines, but preserve internal structure
        let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
        let end = lines
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map(|i| i + 1)
            .unwrap_or(lines.len());

        if start >= end {
            return String::new();
        }

        // Return lines with preserved indentation - let NativeBlock emitter normalize
        lines[start..end].join("\n")
    } else {
        trimmed.to_string()
    }
}

/// Extract the child @@System() name from a domain field's initializer text.
/// Returns Some("Counter") for `@@Counter()`, `@@Counter(args)`, etc.
/// Returns None for any non-tagged-system initializer (primitives, native
/// constructors like `new Counter()` after expand_tagged_in_domain has
/// already run, etc.).
///
/// Used by persist codegen to detect domain fields holding nested system
/// instances. For those, save_state recurses into the child's saveState
/// and restore_state rebuilds via the child's restoreState — preserving
/// class identity through a JSON round-trip that would otherwise produce
/// a plain object dict.
pub(crate) fn extract_tagged_system_name(init: &str) -> Option<&str> {
    let s = init.trim();
    let rest = s.strip_prefix("@@")?;
    let end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

/// Dart type-tree node. Used by the Dart persist-restore emitter to
/// produce deep-typed comprehension expressions from type-string
/// declarations. Architecturally type-ignorant: parses only `List<...>`
/// and `Map<...,...>` shapes; everything else passes through as
/// Primitive(name) and is emitted as `value as <name>`.
enum DartTypeNode {
    Primitive(String),
    List(Box<DartTypeNode>),
    Map(Box<DartTypeNode>, Box<DartTypeNode>),
}

fn parse_dart_type(s: &str) -> DartTypeNode {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("List<").and_then(|x| x.strip_suffix('>')) {
        return DartTypeNode::List(Box::new(parse_dart_type(inner)));
    }
    if let Some(inner) = s.strip_prefix("Map<").and_then(|x| x.strip_suffix('>')) {
        // Find top-level comma (not nested in <>).
        let mut depth = 0i32;
        let mut comma_pos: Option<usize> = None;
        for (i, c) in inner.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth == 0 => {
                    comma_pos = Some(i);
                    break;
                }
                _ => {}
            }
        }
        if let Some(p) = comma_pos {
            let k = parse_dart_type(&inner[..p]);
            let v = parse_dart_type(&inner[p + 1..]);
            return DartTypeNode::Map(Box::new(k), Box::new(v));
        }
    }
    DartTypeNode::Primitive(s.to_string())
}

fn render_dart_type(t: &DartTypeNode) -> String {
    match t {
        DartTypeNode::Primitive(s) => s.clone(),
        DartTypeNode::List(inner) => format!("List<{}>", render_dart_type(inner)),
        DartTypeNode::Map(k, v) => {
            format!("Map<{}, {}>", render_dart_type(k), render_dart_type(v))
        }
    }
}

/// Emit a Dart expression that converts `input` (type `dynamic`) to
/// the typed shape described by `t`. Uses comprehensions (`<T>[for ...]`
/// / `<K,V>{for ...}`) to produce genuinely-typed collections — the
/// only Dart construct that reliably bridges `dynamic`-shaped JSON
/// output to reified-generic typed fields without per-access casts.
///
/// Variable names carry a depth suffix so nested comprehensions don't
/// shadow each other (`__e1`, `__me2`, etc.).
fn dart_conv_expr(t: &DartTypeNode, input: &str) -> String {
    dart_conv_expr_at(t, input, 0)
}

fn dart_conv_expr_at(t: &DartTypeNode, input: &str, depth: usize) -> String {
    match t {
        DartTypeNode::Primitive(name) => match name.as_str() {
            "int" => format!("({input} as num).toInt()"),
            "double" => format!("({input} as num).toDouble()"),
            "num" => format!("{input} as num"),
            "String" => format!("{input} as String"),
            "bool" => format!("{input} as bool"),
            "dynamic" | "Object" | "Object?" => input.to_string(),
            other => format!("{input} as {other}"),
        },
        DartTypeNode::List(inner) => {
            let var = format!("__e{}", depth);
            let elem = dart_conv_expr_at(inner, &var, depth + 1);
            let inner_t = render_dart_type(inner);
            format!("<{inner_t}>[for (var {var} in ({input} as List)) {elem}]")
        }
        DartTypeNode::Map(k, v) => {
            let var = format!("__me{}", depth);
            let k_expr = dart_conv_expr_at(k, &format!("{var}.key"), depth + 1);
            let v_expr = dart_conv_expr_at(v, &format!("{var}.value"), depth + 1);
            let k_t = render_dart_type(k);
            let v_t = render_dart_type(v);
            format!(
                "<{k_t}, {v_t}>{{for (var {var} in ({input} as Map).entries) {k_expr}: {v_expr}}}"
            )
        }
    }
}

/// Generate persistence methods (save_state, restore_state) for @@persist
pub(crate) fn generate_persistence_methods(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    match syntax.language {
        TargetLanguage::Python3 => {
            // Python uses pickle by default (stdlib, complete serialization)
            // Generate save_state method - returns bytes
            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: Some("bytes".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: "import pickle\nreturn pickle.dumps(self)".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // Generate restore_state - takes bytes, returns instance
            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("data").with_type("bytes")],
                return_type: Some(format!("'{}'", system.name)),
                body: vec![CodegenNode::NativeBlock {
                    code: "import pickle\nreturn pickle.loads(data)".to_string(),
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            let is_ts = matches!(syntax.language, TargetLanguage::TypeScript);
            // Generate saveState method
            // Phase 14.6: Serialize compartment structure including HSM parent_compartment chain
            let mut save_body = String::new();
            // Helper to serialize compartment chain recursively
            if is_ts {
                save_body.push_str("const serializeComp = (c: any): any => {\n");
            } else {
                save_body.push_str("const serializeComp = (c) => {\n");
            }
            save_body.push_str("    if (!c) return null;\n");
            save_body.push_str("    return {\n");
            save_body.push_str("        state: c.state,\n");
            save_body.push_str("        state_args: {...c.state_args},\n");
            save_body.push_str("        state_vars: {...c.state_vars},\n");
            save_body.push_str("        enter_args: {...c.enter_args},\n");
            save_body.push_str("        exit_args: {...c.exit_args},\n");
            save_body.push_str("        forward_event: c.forward_event,\n");
            save_body
                .push_str("        parent_compartment: serializeComp(c.parent_compartment),\n");
            save_body.push_str("    };\n");
            save_body.push_str("};\n");
            save_body.push_str("return JSON.stringify({\n");
            save_body.push_str("    _compartment: serializeComp(this.__compartment),\n");
            // Stack stores compartment objects - serialize each with its parent chain
            if is_ts {
                save_body.push_str(
                    "    _state_stack: this._state_stack.map((c: any) => serializeComp(c)),\n",
                );
            } else {
                save_body.push_str(
                    "    _state_stack: this._state_stack.map((c) => serializeComp(c)),\n",
                );
            }

            // Add domain variables. Nested system instances (declared
            // with `@@SystemName()` initializer) round-trip via the
            // child's saveState/restoreState — preserving class identity
            // through JSON. Plain values pass through verbatim.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "    {0}: this.{0} ? JSON.parse(this.{0}.saveState()) : null,\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("    {}: this.{},\n", var.name, var.name));
                }
            }

            save_body.push_str("});\n");

            methods.push(CodegenNode::Method {
                name: "saveState".to_string(),
                params: vec![],
                return_type: Some("string".to_string()), // Returns JSON string
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // Generate restoreState static method
            // Phase 14.6: Restore compartment structure including HSM parent_compartment chain
            let mut restore_body = String::new();
            // Helper to deserialize compartment chain recursively
            if is_ts {
                restore_body.push_str(&format!(
                    "const deserializeComp = (data: any): {}Compartment | null => {{\n",
                    system.name
                ));
            } else {
                restore_body.push_str("const deserializeComp = (data) => {\n");
            }
            restore_body.push_str("    if (!data) return null;\n");
            restore_body.push_str(&format!(
                "    const comp = new {}Compartment(data.state);\n",
                system.name
            ));
            restore_body.push_str("    comp.state_args = {...(data.state_args || {})};\n");
            restore_body.push_str("    comp.state_vars = {...(data.state_vars || {})};\n");
            restore_body.push_str("    comp.enter_args = {...(data.enter_args || {})};\n");
            restore_body.push_str("    comp.exit_args = {...(data.exit_args || {})};\n");
            restore_body.push_str("    comp.forward_event = data.forward_event;\n");
            restore_body.push_str(
                "    comp.parent_compartment = deserializeComp(data.parent_compartment);\n",
            );
            restore_body.push_str("    return comp;\n");
            restore_body.push_str("};\n");
            restore_body.push_str("const data = JSON.parse(json);\n");
            restore_body.push_str(&format!(
                "const instance = Object.create({}.prototype);\n",
                system.name
            ));
            // Restore compartment with full parent chain
            restore_body.push_str("instance.__compartment = deserializeComp(data._compartment);\n");
            restore_body.push_str("instance.__next_compartment = null;\n");
            // Restore stack - each element is a serialized compartment with its parent chain
            if is_ts {
                restore_body.push_str("instance._state_stack = (data._state_stack || []).map((c: any) => deserializeComp(c));\n");
            } else {
                restore_body.push_str("instance._state_stack = (data._state_stack || []).map((c) => deserializeComp(c));\n");
            }
            restore_body.push_str("instance._context_stack = [];\n");

            // Restore domain variables. Nested system instances rebuild
            // via the child's restoreState — recovering class identity
            // (methods are callable post-restore). Plain values pass
            // through verbatim.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    restore_body.push_str(&format!(
                        "instance.{0} = data.{0} != null ? {1}.restoreState(JSON.stringify(data.{0})) : null;\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!("instance.{} = data.{};\n", var.name, var.name));
                }
            }

            restore_body.push_str("return instance;");

            methods.push(CodegenNode::Method {
                name: "restoreState".to_string(),
                params: vec![Param::new("json").with_type("string")],
                return_type: Some(system.name.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Rust => {
            methods.extend(super::rust_system::generate_rust_persistence_methods(
                system,
            ));
        }
        TargetLanguage::C => {
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
                            let types: Vec<String> =
                                s.params.iter().map(|p| type_to_string(&p.param_type)).collect();
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
                    "i32" | "i64" | "isize" | "uint" | "uintptr_t"
                    | "intptr_t" | "long" | "short" => "int",
                    "f32" | "f64" | "float" => "double",
                    "boolean" => "bool",
                    "string" | "String" | "str" | "char*"
                    | "const char*" => "str",
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

            // Serialize domain variables
            for var in &system.domain {
                let type_str = type_to_string(&var.var_type);

                let json_add = if is_int_type(&type_str) {
                    format!(
                        "cJSON_AddNumberToObject(root, \"{}\", (double)self->{});\n",
                        var.name, var.name
                    )
                } else if is_float_type(&type_str) {
                    format!(
                        "cJSON_AddNumberToObject(root, \"{}\", self->{});\n",
                        var.name, var.name
                    )
                } else if is_bool_type(&type_str) {
                    format!(
                        "cJSON_AddBoolToObject(root, \"{}\", self->{});\n",
                        var.name, var.name
                    )
                } else if is_string_type(&type_str) {
                    format!(
                        "cJSON_AddStringToObject(root, \"{}\", self->{});\n",
                        var.name, var.name
                    )
                } else {
                    format!(
                        "cJSON_AddNumberToObject(root, \"{}\", (double)(intptr_t)self->{});\n",
                        var.name, var.name
                    )
                };
                save_body.push_str(&json_add);
            }

            save_body.push_str("char* json = cJSON_PrintUnformatted(root);\n");
            save_body.push_str("cJSON_Delete(root);\n");
            save_body.push_str("return json;");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
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

            // Generate restore_state function - takes const char*, returns instance pointer
            let mut restore_body = String::new();
            restore_body.push_str("cJSON* root = cJSON_Parse(json);\n");
            restore_body.push_str("if (!root) return NULL;\n\n");

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
            restore_body.push_str("instance->__next_compartment = NULL;\n\n");

            // Restore entire compartment chain
            restore_body
                .push_str("cJSON* comp_data = cJSON_GetObjectItem(root, \"_compartment\");\n");
            restore_body.push_str(&format!(
                "instance->__compartment = {}_deserialize_compartment(comp_data);\n\n",
                system.name
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
                "            {}_FrameVec_push(instance->_state_stack, comp);\n",
                system.name
            ));
            restore_body.push_str("        }\n");
            restore_body.push_str("    }\n");
            restore_body.push_str("}\n\n");

            // Restore domain variables
            for var in &system.domain {
                let type_str = type_to_string(&var.var_type);

                let json_get = if is_int_type(&type_str) {
                    format!(
                        "instance->{} = (int)cJSON_GetObjectItem(root, \"{}\")->valuedouble;\n",
                        var.name, var.name
                    )
                } else if is_float_type(&type_str) {
                    format!(
                        "instance->{} = cJSON_GetObjectItem(root, \"{}\")->valuedouble;\n",
                        var.name, var.name
                    )
                } else if is_bool_type(&type_str) {
                    format!(
                        "instance->{} = cJSON_IsTrue(cJSON_GetObjectItem(root, \"{}\"));\n",
                        var.name, var.name
                    )
                } else if is_string_type(&type_str) {
                    format!(
                        "instance->{} = strdup(cJSON_GetObjectItem(root, \"{}\")->valuestring);\n",
                        var.name, var.name
                    )
                } else {
                    format!(
                        "instance->{} = (int)cJSON_GetObjectItem(root, \"{}\")->valuedouble;\n",
                        var.name, var.name
                    )
                };
                restore_body.push_str(&json_get);
            }

            restore_body.push_str("\ncJSON_Delete(root);\n");
            restore_body.push_str("return instance;");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json").with_type("const char*")],
                return_type: Some(format!("{}*", system.name)),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Cpp => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

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
                save_body.push_str(&format!(
                    "    if (c->state == \"{}\") {{\n",
                    state_name
                ));
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
                                            crate::frame_c::compiler::frame_ast::Type::Custom(s) => {
                                                s.clone()
                                            }
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
                save_body.push_str(&format!(
                    "    if (c->state == \"{}\") {{\n",
                    state_name
                ));
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
                name: "save_state".to_string(),
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

            // restore_state(json) — static method with recursive compartment deserialization
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
            restore_body.push_str("    if (d.contains(\"state_args\") && d[\"state_args\"].is_array()) {\n");
            restore_body.push_str("        const auto& __sa = d[\"state_args\"];\n");
            let mut any_typed_state = false;
            for (state_name, types) in &cpp_state_arg_decls {
                if types.is_empty() {
                    continue;
                }
                if !any_typed_state {
                    any_typed_state = true;
                }
                restore_body.push_str(&format!(
                    "        if (c->state == \"{}\") {{\n",
                    state_name
                ));
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
            restore_body.push_str("    if (d.contains(\"enter_args\") && d[\"enter_args\"].is_array()) {\n");
            restore_body.push_str("        const auto& __ea = d[\"enter_args\"];\n");
            let mut any_typed_enter = false;
            for (state_name, types) in &cpp_enter_arg_decls {
                if types.is_empty() {
                    continue;
                }
                if !any_typed_enter {
                    any_typed_enter = true;
                }
                restore_body.push_str(&format!(
                    "        if (c->state == \"{}\") {{\n",
                    state_name
                ));
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

            restore_body.push_str("auto __j = nlohmann::json::parse(json);\n");
            // Suppress the initial-state $>() dispatch on the restored
            // instance. See Swift for rationale.
            restore_body.push_str(&format!("{}::__skipInitialEnter = true;\n", sys));
            restore_body.push_str(&format!("{} __instance;\n", sys));
            restore_body.push_str(&format!("{}::__skipInitialEnter = false;\n", sys));
            restore_body.push_str("__instance.__compartment = __deser(__j[\"_compartment\"]);\n");

            // Restore state stack
            restore_body.push_str("if (__j.contains(\"_state_stack\")) {\n");
            restore_body.push_str("    for (auto& __sc : __j[\"_state_stack\"]) {\n");
            restore_body.push_str("        __instance._state_stack.push_back(__deser(__sc));\n");
            restore_body.push_str("    }\n");
            restore_body.push_str("}\n");

            // Restore domain vars. Nested @@SystemName() instances
            // re-hydrate via child restore_state.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    // shared_ptr field — wrap restored value in make_shared.
                    restore_body.push_str(&format!(
                        "if (__j.contains(\"{0}\") && !__j[\"{0}\"].is_null()) {{ __instance.{0} = std::make_shared<{1}>({1}::restore_state(__j[\"{0}\"].dump())); }}\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!(
                        "if (__j.contains(\"{0}\")) {{ __j[\"{0}\"].get_to(__instance.{0}); }}\n",
                        var.name
                    ));
                }
            }

            restore_body.push_str("return __instance;");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json").with_type("const std::string&")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Java => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Collect (state_name, [param_java_types]) for states with declared
            // params. Used to emit per-state typed restore for state_args /
            // enter_args via Jackson's TypeReference. The architecture is
            // type-ignorant in framec — we walk state.params and emit the
            // user's declared type strings verbatim into
            // `new TypeReference<USER_TYPE>(){}`. Jackson does deep typed
            // conversion via reflection.
            //
            // Java primitives must be boxed for TypeReference (generics
            // can't take primitives — `TypeReference<int>` is illegal).
            let java_box = |t: &str| -> String {
                match t {
                    "int" => "Integer".to_string(),
                    "double" => "Double".to_string(),
                    "float" => "Float".to_string(),
                    "boolean" => "Boolean".to_string(),
                    "long" => "Long".to_string(),
                    "char" => "Character".to_string(),
                    "byte" => "Byte".to_string(),
                    "short" => "Short".to_string(),
                    other => other.to_string(),
                }
            };
            let state_param_types: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .filter(|s| !s.params.is_empty())
                        .map(|s| {
                            let types: Vec<String> = s
                                .params
                                .iter()
                                .map(|p| match &p.param_type {
                                    crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                                        java_box(&java_map_type(t))
                                    }
                                    _ => "Object".to_string(),
                                })
                                .collect();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();

            // __serComp — recursive compartment serializer. Builds a plain
            // Map<String, Object> tree; Jackson handles nested types
            // (Map, List, primitives) at writeValueAsString time via its
            // built-in per-type serializers.
            let mut ser_body = String::new();
            ser_body.push_str("if (comp == null) return null;\n");
            ser_body.push_str("var j = new java.util.LinkedHashMap<String, Object>();\n");
            ser_body.push_str("j.put(\"state\", comp.state);\n");
            ser_body.push_str(
                "j.put(\"state_vars\", new java.util.LinkedHashMap<>(comp.state_vars));\n",
            );
            ser_body.push_str("j.put(\"state_args\", new java.util.ArrayList<>(comp.state_args));\n");
            ser_body.push_str("j.put(\"enter_args\", new java.util.ArrayList<>(comp.enter_args));\n");
            ser_body.push_str("j.put(\"parent\", __serComp(comp.parent_compartment));\n");
            ser_body.push_str("return j;");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp").with_type(&compartment_class)],
                return_type: Some("Object".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: ser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __deserComp — recursive compartment deserializer.
            // Per-state TypeReference is the win: state_args[i] for state
            // `Active(m: Map<String, List<Integer>>)` is restored as a
            // proper Map<String, List<Integer>> with Integer elements,
            // not a generic LinkedHashMap<String, ArrayList<Number>>. The
            // user's handler can then index `m[key]` directly without
            // defensive casts at every access.
            let mut deser_body = String::new();
            deser_body.push_str("if (node == null || node.isNull()) return null;\n");
            deser_body.push_str(&format!(
                "var c = new {}(node.get(\"state\").asText());\n",
                compartment_class
            ));
            deser_body.push_str("if (node.has(\"state_vars\")) {\n");
            deser_body.push_str("    var fields = node.get(\"state_vars\").fields();\n");
            deser_body.push_str("    while (fields.hasNext()) {\n");
            deser_body.push_str("        var e = fields.next();\n");
            deser_body
                .push_str("        c.state_vars.put(e.getKey(), mapper.convertValue(e.getValue(), Object.class));\n");
            deser_body.push_str("    }\n");
            deser_body.push_str("}\n");
            deser_body.push_str(
                "var __sa = node.has(\"state_args\") ? node.get(\"state_args\") : null;\n",
            );
            deser_body.push_str(
                "var __ea = node.has(\"enter_args\") ? node.get(\"enter_args\") : null;\n",
            );
            if !state_param_types.is_empty() {
                deser_body.push_str("switch (c.state) {\n");
                for (state_name, param_types) in &state_param_types {
                    deser_body.push_str(&format!("    case \"{}\":\n", state_name));
                    for (i, ty) in param_types.iter().enumerate() {
                        deser_body.push_str(&format!(
                            "        if (__sa != null && __sa.size() > {i}) c.state_args.add(mapper.convertValue(__sa.get({i}), new com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}));\n"
                        ));
                        deser_body.push_str(&format!(
                            "        if (__ea != null && __ea.size() > {i}) c.enter_args.add(mapper.convertValue(__ea.get({i}), new com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}));\n"
                        ));
                    }
                    deser_body.push_str("        break;\n");
                }
                deser_body.push_str("    default:\n");
                deser_body.push_str(
                    "        if (__sa != null) for (var n : __sa) c.state_args.add(mapper.convertValue(n, Object.class));\n",
                );
                deser_body.push_str(
                    "        if (__ea != null) for (var n : __ea) c.enter_args.add(mapper.convertValue(n, Object.class));\n",
                );
                deser_body.push_str("        break;\n");
                deser_body.push_str("}\n");
            } else {
                deser_body.push_str(
                    "if (__sa != null) for (var n : __sa) c.state_args.add(mapper.convertValue(n, Object.class));\n",
                );
                deser_body.push_str(
                    "if (__ea != null) for (var n : __ea) c.enter_args.add(mapper.convertValue(n, Object.class));\n",
                );
            }
            deser_body.push_str(
                "if (node.has(\"parent\") && !node.get(\"parent\").isNull()) c.parent_compartment = __deserComp(node.get(\"parent\"), mapper);\n",
            );
            deser_body.push_str("return c;");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![
                    Param::new("node").with_type("com.fasterxml.jackson.databind.JsonNode"),
                    Param::new("mapper").with_type("com.fasterxml.jackson.databind.ObjectMapper"),
                ],
                return_type: Some(compartment_class.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: deser_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // save_state — Jackson handles nested Map/List/primitives at
            // writeValueAsString time. Wraps Jackson's checked exception
            // as RuntimeException so the public API stays unchecked.
            let mut save_body = String::new();
            save_body.push_str("var mapper = new com.fasterxml.jackson.databind.ObjectMapper();\n");
            save_body.push_str("var __j = new java.util.LinkedHashMap<String, Object>();\n");
            save_body.push_str("__j.put(\"_compartment\", __serComp(__compartment));\n");
            save_body.push_str("var __stack = new java.util.ArrayList<Object>();\n");
            save_body.push_str("for (var c : _state_stack) __stack.add(__serComp(c));\n");
            save_body.push_str("__j.put(\"_state_stack\", __stack);\n");
            for var in &system.domain {
                save_body.push_str(&format!("__j.put(\"{}\", {});\n", var.name, var.name));
            }
            save_body.push_str("try { return mapper.writeValueAsString(__j); } catch (Exception e) { throw new RuntimeException(e); }");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: Some("String".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restore_state — Jackson readTree + per-state typed restore.
            // Domain vars use TypeReference per declared type so List<T> /
            // Map<K,V> domain fields recover their full typed shape (no
            // erasure-cast workaround needed at user-access sites).
            let mut restore_body = String::new();
            restore_body.push_str("var mapper = new com.fasterxml.jackson.databind.ObjectMapper();\n");
            restore_body.push_str(
                "com.fasterxml.jackson.databind.JsonNode __j;\n",
            );
            restore_body.push_str(
                "try { __j = mapper.readTree(json); } catch (Exception e) { throw new RuntimeException(e); }\n",
            );
            restore_body.push_str("__skipInitialEnter = true;\n");
            restore_body.push_str(&format!("var __instance = new {}();\n", sys));
            restore_body.push_str("__skipInitialEnter = false;\n");
            restore_body.push_str(
                "__instance.__compartment = __deserComp(__j.get(\"_compartment\"), mapper);\n",
            );
            restore_body.push_str("if (__j.has(\"_state_stack\")) {\n");
            restore_body.push_str("    __instance._state_stack = new java.util.ArrayList<>();\n");
            restore_body.push_str(
                "    for (var __sc : __j.get(\"_state_stack\")) __instance._state_stack.add(__deserComp(__sc, mapper));\n",
            );
            restore_body.push_str("}\n");
            for var in &system.domain {
                let java_type: String = match &var.var_type {
                    crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                        // Domain fields keep their declared Java type (incl.
                        // primitives like `int x`); the TypeReference itself
                        // needs the boxed form, but the assignment target is
                        // the field, which Jackson will unbox correctly.
                        java_box(&java_map_type(t))
                    }
                    _ => "Object".to_string(),
                };
                restore_body.push_str(&format!(
                    "if (__j.has(\"{name}\")) __instance.{name} = mapper.convertValue(__j.get(\"{name}\"), new com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}});\n",
                    name = var.name,
                    ty = java_type
                ));
            }
            restore_body.push_str("return __instance;");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json").with_type("String")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::CSharp => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Private helper method for recursive compartment serialization.
            // Includes state_args + enter_args (compartment-context fields)
            // — same root pattern as Java/Kotlin fixes above.
            let mut ser_body = String::new();
            ser_body.push_str("if (comp == null) return null;\n");
            ser_body.push_str("var j = new Dictionary<string, object>();\n");
            ser_body.push_str("j[\"state\"] = comp.state;\n");
            ser_body.push_str("var sv = new Dictionary<string, object>(comp.state_vars);\n");
            ser_body.push_str("j[\"state_vars\"] = sv;\n");
            ser_body.push_str("j[\"state_args\"] = new List<object>(comp.state_args);\n");
            ser_body.push_str("j[\"enter_args\"] = new List<object>(comp.enter_args);\n");
            ser_body.push_str("j[\"parent\"] = __SerComp(comp.parent_compartment);\n");
            ser_body.push_str("return j;");

            methods.push(CodegenNode::Method {
                name: "__SerComp".to_string(),
                params: vec![Param::new("comp").with_type(&compartment_class)],
                return_type: Some("object".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: ser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // Private helper for deserialization — uses JsonElement
            let mut deser_body = String::new();
            deser_body.push_str(
                "if (el.ValueKind == System.Text.Json.JsonValueKind.Null) return null;\n",
            );
            deser_body.push_str(&format!(
                "var c = new {}(el.GetProperty(\"state\").GetString());\n",
                compartment_class
            ));
            deser_body.push_str("if (el.TryGetProperty(\"state_vars\", out var sv) && sv.ValueKind == System.Text.Json.JsonValueKind.Object) {\n");
            deser_body.push_str("    foreach (var kv in sv.EnumerateObject()) {\n");
            // Number values may be int or float. Try Int32 first (the
            // common case — preserves the existing typed reads at the
            // handler site), then fall back to Int64 for large ints,
            // then double for fractionals. Avoids the "Int64 cast to
            // Int32 fails" pitfall when handler does `(int) value`.
            deser_body.push_str("        if (kv.Value.ValueKind == System.Text.Json.JsonValueKind.Number) { if (kv.Value.TryGetInt32(out int __ii)) c.state_vars[kv.Name] = __ii; else if (kv.Value.TryGetInt64(out long __il)) c.state_vars[kv.Name] = __il; else c.state_vars[kv.Name] = kv.Value.GetDouble(); }\n");
            deser_body.push_str("        else if (kv.Value.ValueKind == System.Text.Json.JsonValueKind.String) c.state_vars[kv.Name] = kv.Value.GetString();\n");
            deser_body.push_str("        else c.state_vars[kv.Name] = kv.Value.ToString();\n");
            deser_body.push_str("    }\n");
            deser_body.push_str("}\n");
            // Generic deserialize: each JSON element → `object` in
            // state_args. Arrays recurse into List<object> so the
            // per-state typed pass (below) can convert to the
            // declared `List<T>`. The recursion handles arbitrary
            // nesting depth (List<List<...>>).
            deser_body.push_str("if (el.TryGetProperty(\"state_args\", out var sa) && sa.ValueKind == System.Text.Json.JsonValueKind.Array) {\n");
            deser_body.push_str("    foreach (var v in sa.EnumerateArray()) c.state_args.Add(__convertJsonValue(v));\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if (el.TryGetProperty(\"enter_args\", out var ea) && ea.ValueKind == System.Text.Json.JsonValueKind.Array) {\n");
            deser_body.push_str("    foreach (var v in ea.EnumerateArray()) c.enter_args.Add(__convertJsonValue(v));\n");
            deser_body.push_str("}\n");
            // Type-ignorant typed restore via JsonSerializer
            // round-trip. framec emits the declared type verbatim;
            // System.Text.Json reflection handles primitives,
            // List<T>, Dictionary<K,V>, nested structures, and user
            // types with [JsonPropertyName] attributes — without
            // framec parsing generics or detecting container kinds.
            let cs_typed_conv = |declared_type: &str, idx: usize, slot: &str| -> String {
                let t = declared_type.trim();
                if t.is_empty() {
                    return String::new();
                }
                format!(
                    "    if (c.{slot}.Count > {idx} && c.{slot}[{idx}] != null) {{\n\
                     \x20       try {{\n\
                     \x20           var __raw = System.Text.Json.JsonSerializer.Serialize(c.{slot}[{idx}]);\n\
                     \x20           c.{slot}[{idx}] = System.Text.Json.JsonSerializer.Deserialize<{t}>(__raw);\n\
                     \x20       }} catch {{ /* leave generic value in place */ }}\n\
                     \x20   }}\n"
                )
            };
            let state_arg_decls: Vec<(String, Vec<String>)> = system
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
            let enter_arg_decls: Vec<(String, Vec<String>)> = system
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
                                            crate::frame_c::compiler::frame_ast::Type::Custom(s) => {
                                                s.clone()
                                            }
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
            let mut any_per_state = false;
            for (state_name, types) in &state_arg_decls {
                let mut branch = String::new();
                for (i, t) in types.iter().enumerate() {
                    let conv = cs_typed_conv(t, i, "state_args");
                    if !conv.is_empty() {
                        branch.push_str(&conv);
                    }
                }
                if !branch.is_empty() {
                    if !any_per_state {
                        deser_body.push_str(
                            "// D10 per-state typed list conversion\n",
                        );
                        any_per_state = true;
                    }
                    deser_body.push_str(&format!(
                        "if (c.state == \"{}\") {{\n{}}}\n",
                        state_name, branch
                    ));
                }
            }
            for (state_name, types) in &enter_arg_decls {
                let mut branch = String::new();
                for (i, t) in types.iter().enumerate() {
                    let conv = cs_typed_conv(t, i, "enter_args");
                    if !conv.is_empty() {
                        branch.push_str(&conv);
                    }
                }
                if !branch.is_empty() {
                    if !any_per_state {
                        deser_body.push_str(
                            "// D10 per-state typed list conversion\n",
                        );
                        any_per_state = true;
                    }
                    deser_body.push_str(&format!(
                        "if (c.state == \"{}\") {{\n{}}}\n",
                        state_name, branch
                    ));
                }
            }
            deser_body.push_str("if (el.TryGetProperty(\"parent\", out var p) && p.ValueKind != System.Text.Json.JsonValueKind.Null) {\n");
            deser_body.push_str("    c.parent_compartment = __DeserComp(p);\n");
            deser_body.push_str("}\n");
            deser_body.push_str("return c;");

            methods.push(CodegenNode::Method {
                name: "__DeserComp".to_string(),
                params: vec![Param::new("el").with_type("System.Text.Json.JsonElement")],
                return_type: Some(compartment_class.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: deser_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // Recursive JsonElement → object converter. Numbers /
            // strings / booleans return their primitive form;
            // arrays return List<object>; objects return
            // Dictionary<string, object>. Recurses for nested
            // structures. The per-state typed pass then recovers
            // `List<T>` / `Dictionary<K,V>` from the generic tree.
            let mut conv_body = String::new();
            conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.Number) {\n");
            conv_body.push_str("    if (v.TryGetInt32(out int __i)) return __i;\n");
            conv_body.push_str("    if (v.TryGetInt64(out long __l)) return __l;\n");
            conv_body.push_str("    return v.GetDouble();\n");
            conv_body.push_str("}\n");
            conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.String) return v.GetString();\n");
            conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.True) return true;\n");
            conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.False) return false;\n");
            conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.Array) {\n");
            conv_body.push_str("    var __list = new System.Collections.Generic.List<object>();\n");
            conv_body.push_str("    foreach (var __ne in v.EnumerateArray()) __list.Add(__convertJsonValue(__ne));\n");
            conv_body.push_str("    return __list;\n");
            conv_body.push_str("}\n");
            conv_body.push_str("if (v.ValueKind == System.Text.Json.JsonValueKind.Object) {\n");
            conv_body.push_str("    var __dict = new System.Collections.Generic.Dictionary<string, object>();\n");
            conv_body.push_str("    foreach (var __prop in v.EnumerateObject()) __dict[__prop.Name] = __convertJsonValue(__prop.Value);\n");
            conv_body.push_str("    return __dict;\n");
            conv_body.push_str("}\n");
            conv_body.push_str("return v.ToString();");
            methods.push(CodegenNode::Method {
                name: "__convertJsonValue".to_string(),
                params: vec![Param::new("v").with_type("System.Text.Json.JsonElement")],
                return_type: Some("object".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: conv_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("var __j = new Dictionary<string, object>();\n");
            save_body.push_str("__j[\"_compartment\"] = __SerComp(__compartment);\n");
            save_body.push_str("var __stack = new List<object>();\n");
            save_body.push_str("foreach (var c in _state_stack) { __stack.Add(__SerComp(c)); }\n");
            save_body.push_str("__j[\"_state_stack\"] = __stack;\n");

            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "__j[\"{0}\"] = {0} != null ? System.Text.Json.JsonDocument.Parse({0}.SaveState()).RootElement.Clone() : (object)null;\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("__j[\"{}\"] = {};\n", var.name, var.name));
                }
            }

            save_body.push_str("var __opts = new System.Text.Json.JsonSerializerOptions { TypeInfoResolver = new System.Text.Json.Serialization.Metadata.DefaultJsonTypeInfoResolver() };\n");
            save_body.push_str("return System.Text.Json.JsonSerializer.Serialize(__j, __opts);");

            methods.push(CodegenNode::Method {
                name: "SaveState".to_string(),
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

            // RestoreState(json) — static method.
            //
            // Uses `RuntimeHelpers.GetUninitializedObject` to create the
            // instance WITHOUT running the constructor. The constructor
            // would otherwise dispatch the initial-state $>() enter
            // handler, leaking side effects on every restore. Instance
            // fields that the ctor would have populated (_state_stack,
            // _context_stack) are set up explicitly below.
            let mut restore_body = String::new();
            restore_body.push_str("var __doc = System.Text.Json.JsonDocument.Parse(json);\n");
            restore_body.push_str("var __root = __doc.RootElement;\n");
            restore_body.push_str(&format!(
                "var __instance = ({0})System.Runtime.CompilerServices.RuntimeHelpers.GetUninitializedObject(typeof({0}));\n",
                sys,
            ));
            restore_body.push_str(&format!(
                "__instance._state_stack = new List<{}>();\n",
                compartment_class,
            ));
            restore_body.push_str(&format!(
                "__instance._context_stack = new List<{}FrameContext>();\n",
                sys,
            ));
            restore_body.push_str(
                "__instance.__compartment = __DeserComp(__root.GetProperty(\"_compartment\"));\n",
            );
            restore_body
                .push_str("if (__root.TryGetProperty(\"_state_stack\", out var __stack)) {\n");
            restore_body.push_str(&format!(
                "    __instance._state_stack = new List<{}>();\n",
                compartment_class
            ));
            restore_body.push_str("    foreach (var item in __stack.EnumerateArray()) { __instance._state_stack.Add(__DeserComp(item)); }\n");
            restore_body.push_str("}\n");

            // Type-ignorant: emit the declared type verbatim as
            // JsonSerializer.Deserialize<T>'s generic parameter.
            // System.Text.Json reflection handles primitives, lists,
            // dicts, and user types with [JsonPropertyName].
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    restore_body.push_str(&format!(
                        "if (__root.TryGetProperty(\"{name}\", out var __{name})) {{ if (__{name}.ValueKind != System.Text.Json.JsonValueKind.Null) {{ __instance.{name} = {child}.RestoreState(__{name}.GetRawText()); }} }}\n",
                        name = var.name,
                        child = child_sys
                    ));
                } else {
                    let declared = match &var.var_type {
                        crate::frame_c::compiler::frame_ast::Type::Custom(t) => csharp_map_type(t),
                        _ => "object".to_string(),
                    };
                    restore_body.push_str(&format!(
                        "if (__root.TryGetProperty(\"{name}\", out var __{name})) {{ try {{ __instance.{name} = System.Text.Json.JsonSerializer.Deserialize<{t}>(__{name}.GetRawText()); }} catch {{ }} }}\n",
                        name = var.name,
                        t = declared
                    ));
                }
            }

            restore_body.push_str("return __instance;");

            methods.push(CodegenNode::Method {
                name: "RestoreState".to_string(),
                params: vec![Param::new("json").with_type("string")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Php => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Private helper for recursive compartment serialization.
            // Includes state_args + enter_args (compartment-context fields)
            // — same root pattern as Java/Kotlin/C#/Swift fixes.
            let mut ser_body = String::new();
            ser_body.push_str("if ($comp === null) return null;\n");
            ser_body
                .push_str("$j = ['state' => $comp->state, 'state_vars' => $comp->state_vars];\n");
            ser_body.push_str("$j['state_args'] = $comp->state_args;\n");
            ser_body.push_str("$j['enter_args'] = $comp->enter_args;\n");
            ser_body.push_str("$j['parent'] = $this->__serComp($comp->parent_compartment);\n");
            ser_body.push_str("return $j;");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: ser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // Private helper for deserialization
            let mut deser_body = String::new();
            deser_body.push_str("if ($data === null) return null;\n");
            deser_body.push_str(&format!(
                "$c = new {}($data['state']);\n",
                compartment_class
            ));
            deser_body.push_str(
                "if (isset($data['state_vars'])) $c->state_vars = $data['state_vars'];\n",
            );
            deser_body.push_str("if (isset($data['state_args'])) $c->state_args = $data['state_args'];\n");
            deser_body.push_str("if (isset($data['enter_args'])) $c->enter_args = $data['enter_args'];\n");
            deser_body.push_str("if (isset($data['parent'])) $c->parent_compartment = self::__deserComp($data['parent']);\n");
            deser_body.push_str("return $c;");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![Param::new("data")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: deser_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("$j = [];\n");
            save_body.push_str("$j['_compartment'] = $this->__serComp($this->__compartment);\n");
            save_body.push_str("$stack = [];\n");
            save_body.push_str(
                "foreach ($this->_state_stack as $c) { $stack[] = $this->__serComp($c); }\n",
            );
            save_body.push_str("$j['_state_stack'] = $stack;\n");
            // Domain vars: nested @@SystemName() instances round-trip via
            // child's save_state/restore_state — preserves class identity.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "$j['{0}'] = $this->{0} !== null ? json_decode($this->{0}->save_state(), true) : null;\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("$j['{}'] = $this->{};\n", var.name, var.name));
                }
            }
            save_body.push_str("return json_encode($j);");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restore_state($json) — static.
            //
            // Uses ReflectionClass::newInstanceWithoutConstructor so the
            // generated class's __construct does NOT run. __construct
            // dispatches the initial state's $>() enter handler; a
            // restored instance must not re-fire that. Instance props
            // that __construct would normally set up are populated here
            // from the saved blob.
            let mut restore_body = String::new();
            restore_body.push_str("$j = json_decode($json, true);\n");
            restore_body.push_str(&format!(
                "$instance = (new \\ReflectionClass({}::class))->newInstanceWithoutConstructor();\n",
                sys
            ));
            restore_body.push_str("$instance->_state_stack = [];\n");
            restore_body.push_str("$instance->_context_stack = [];\n");
            restore_body
                .push_str("$instance->__compartment = self::__deserComp($j['_compartment']);\n");
            restore_body.push_str("if (isset($j['_state_stack'])) {\n");
            restore_body.push_str("    foreach ($j['_state_stack'] as $sc) { $instance->_state_stack[] = self::__deserComp($sc); }\n");
            restore_body.push_str("}\n");
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    restore_body.push_str(&format!(
                        "if (isset($j['{0}']) && $j['{0}'] !== null) $instance->{0} = {1}::restore_state(json_encode($j['{0}']));\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!(
                        "if (isset($j['{}'])) $instance->{} = $j['{}'];\n",
                        var.name, var.name, var.name
                    ));
                }
            }
            restore_body.push_str("return $instance;");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Kotlin => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Same per-state typed restore pattern as Java. Kotlin's
            // reified generics make TypeReference work cleanly; primitives
            // map naturally because Kotlin auto-boxes Int → java.lang.Integer
            // etc. for generic contexts.
            let kt_box = |t: &str| -> String {
                match t {
                    "Int" => "Int".to_string(),
                    "Long" => "Long".to_string(),
                    "Double" => "Double".to_string(),
                    "Float" => "Float".to_string(),
                    "Boolean" => "Boolean".to_string(),
                    "String" => "String".to_string(),
                    other => other.to_string(),
                }
            };
            let state_param_types: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .filter(|s| !s.params.is_empty())
                        .map(|s| {
                            let types: Vec<String> = s
                                .params
                                .iter()
                                .map(|p| match &p.param_type {
                                    crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                                        kt_box(&kotlin_map_type(t))
                                    }
                                    _ => "Any".to_string(),
                                })
                                .collect();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();

            // __serComp — builds Map<String, Any?> tree; Jackson handles
            // nested types at writeValueAsString time.
            let mut ser_body = String::new();
            ser_body.push_str("if (comp == null) return null\n");
            ser_body.push_str("val j = java.util.LinkedHashMap<String, Any?>()\n");
            ser_body.push_str("j[\"state\"] = comp.state\n");
            ser_body.push_str("j[\"state_vars\"] = java.util.LinkedHashMap(comp.state_vars)\n");
            ser_body.push_str("j[\"state_args\"] = java.util.ArrayList(comp.state_args)\n");
            ser_body.push_str("j[\"enter_args\"] = java.util.ArrayList(comp.enter_args)\n");
            ser_body.push_str("j[\"parent\"] = __serComp(comp.parent_compartment)\n");
            ser_body.push_str("return j");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp").with_type(&format!("{}?", compartment_class))],
                return_type: Some("Any?".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: ser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // __deserComp — Jackson JsonNode walk + per-state typed restore.
            let mut deser_body = String::new();
            deser_body.push_str("if (node == null || node.isNull) return null\n");
            deser_body.push_str(&format!(
                "val c = {}(node.get(\"state\").asText())\n",
                compartment_class
            ));
            deser_body.push_str("if (node.has(\"state_vars\")) {\n");
            deser_body.push_str("    val fields = node.get(\"state_vars\").fields()\n");
            deser_body.push_str("    while (fields.hasNext()) {\n");
            deser_body.push_str("        val e = fields.next()\n");
            deser_body.push_str(
                "        c.state_vars[e.key] = mapper.convertValue(e.value, Any::class.java)\n",
            );
            deser_body.push_str("    }\n");
            deser_body.push_str("}\n");
            deser_body.push_str(
                "val __sa: com.fasterxml.jackson.databind.JsonNode? = if (node.has(\"state_args\")) node.get(\"state_args\") else null\n",
            );
            deser_body.push_str(
                "val __ea: com.fasterxml.jackson.databind.JsonNode? = if (node.has(\"enter_args\")) node.get(\"enter_args\") else null\n",
            );
            if !state_param_types.is_empty() {
                deser_body.push_str("when (c.state) {\n");
                for (state_name, param_types) in &state_param_types {
                    deser_body.push_str(&format!("    \"{}\" -> {{\n", state_name));
                    for (i, ty) in param_types.iter().enumerate() {
                        deser_body.push_str(&format!(
                            "        if (__sa != null && __sa.size() > {i}) c.state_args.add(mapper.convertValue(__sa.get({i}), object : com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}))\n"
                        ));
                        deser_body.push_str(&format!(
                            "        if (__ea != null && __ea.size() > {i}) c.enter_args.add(mapper.convertValue(__ea.get({i}), object : com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}}))\n"
                        ));
                    }
                    deser_body.push_str("    }\n");
                }
                deser_body.push_str("    else -> {\n");
                deser_body.push_str(
                    "        if (__sa != null) for (n in __sa) c.state_args.add(mapper.convertValue(n, Any::class.java))\n",
                );
                deser_body.push_str(
                    "        if (__ea != null) for (n in __ea) c.enter_args.add(mapper.convertValue(n, Any::class.java))\n",
                );
                deser_body.push_str("    }\n");
                deser_body.push_str("}\n");
            } else {
                deser_body.push_str(
                    "if (__sa != null) for (n in __sa) c.state_args.add(mapper.convertValue(n, Any::class.java))\n",
                );
                deser_body.push_str(
                    "if (__ea != null) for (n in __ea) c.enter_args.add(mapper.convertValue(n, Any::class.java))\n",
                );
            }
            deser_body.push_str(
                "if (node.has(\"parent\") && !node.get(\"parent\").isNull) c.parent_compartment = __deserComp(node.get(\"parent\"), mapper)\n",
            );
            deser_body.push_str("return c");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![
                    Param::new("node").with_type("com.fasterxml.jackson.databind.JsonNode?"),
                    Param::new("mapper").with_type("com.fasterxml.jackson.databind.ObjectMapper"),
                ],
                return_type: Some(format!("{}?", compartment_class)),
                body: vec![CodegenNode::NativeBlock {
                    code: deser_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // save_state — Jackson writeValueAsString.
            let mut save_body = String::new();
            save_body.push_str("val mapper = com.fasterxml.jackson.databind.ObjectMapper()\n");
            save_body.push_str("val j = java.util.LinkedHashMap<String, Any?>()\n");
            save_body.push_str("j[\"_compartment\"] = __serComp(__compartment)\n");
            save_body.push_str("val stack = java.util.ArrayList<Any?>()\n");
            save_body.push_str("for (c in _state_stack) stack.add(__serComp(c))\n");
            save_body.push_str("j[\"_state_stack\"] = stack\n");
            for var in &system.domain {
                save_body.push_str(&format!("j[\"{}\"] = {}\n", var.name, var.name));
            }
            save_body.push_str("return mapper.writeValueAsString(j)");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: Some("String".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restore_state — Jackson readTree + per-state typed restore.
            let mut restore_body = String::new();
            restore_body.push_str("val mapper = com.fasterxml.jackson.databind.ObjectMapper()\n");
            restore_body.push_str("val j = mapper.readTree(json)\n");
            restore_body.push_str(&format!("{}.__skipInitialEnter = true\n", sys));
            restore_body.push_str(&format!("val instance = {}()\n", sys));
            restore_body.push_str(&format!("{}.__skipInitialEnter = false\n", sys));
            restore_body.push_str(
                "instance.__compartment = __deserComp(j.get(\"_compartment\"), mapper)!!\n",
            );
            restore_body.push_str("if (j.has(\"_state_stack\")) {\n");
            restore_body.push_str("    instance._state_stack = mutableListOf()\n");
            restore_body.push_str(
                "    for (sc in j.get(\"_state_stack\")) instance._state_stack.add(__deserComp(sc, mapper)!!)\n",
            );
            restore_body.push_str("}\n");
            for var in &system.domain {
                let mapped = match &var.var_type {
                    crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                        kt_box(&kotlin_map_type(t))
                    }
                    _ => "Any".to_string(),
                };
                restore_body.push_str(&format!(
                    "if (j.has(\"{name}\")) instance.{name} = mapper.convertValue(j.get(\"{name}\"), object : com.fasterxml.jackson.core.type.TypeReference<{ty}>(){{}})\n",
                    name = var.name,
                    ty = mapped
                ));
            }
            restore_body.push_str("return instance");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json").with_type("String")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Swift => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Swift uses Foundation JSONSerialization — dict-based serialization.
            // Includes state_args + enter_args (compartment-context fields)
            // — same root pattern as Java/Kotlin/C# fixes.
            let mut ser_body = String::new();
            ser_body.push_str("if comp == nil { return nil }\n");
            ser_body.push_str("var j: [String: Any] = [:]\n");
            ser_body.push_str("j[\"state\"] = comp!.state\n");
            ser_body.push_str("var sv: [String: Any] = [:]\n");
            ser_body.push_str("for (k, v) in comp!.state_vars { sv[k] = v }\n");
            ser_body.push_str("j[\"state_vars\"] = sv\n");
            ser_body.push_str("j[\"state_args\"] = comp!.state_args\n");
            ser_body.push_str("j[\"enter_args\"] = comp!.enter_args\n");
            ser_body.push_str("j[\"parent\"] = __serComp(comp!.parent_compartment) as Any\n");
            ser_body.push_str("return j");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp").with_type(&format!("{}?", compartment_class))],
                return_type: Some("[String: Any]?".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: ser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            let mut deser_body = String::new();
            deser_body.push_str("guard let d = dict else { return nil }\n");
            deser_body.push_str("guard let state = d[\"state\"] as? String else { return nil }\n");
            deser_body.push_str(&format!("let c = {}(state: state)\n", compartment_class));
            deser_body.push_str("if let sv = d[\"state_vars\"] as? [String: Any] {\n");
            deser_body.push_str("    for (k, v) in sv { c.state_vars[k] = v }\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if let sa = d[\"state_args\"] as? [Any] {\n");
            deser_body.push_str("    c.state_args = sa\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if let ea = d[\"enter_args\"] as? [Any] {\n");
            deser_body.push_str("    c.enter_args = ea\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if let parent = d[\"parent\"] as? [String: Any] {\n");
            deser_body.push_str("    c.parent_compartment = __deserComp(parent)\n");
            deser_body.push_str("}\n");
            deser_body.push_str("return c");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![Param::new("dict").with_type("[String: Any]?")],
                return_type: Some(format!("{}?", compartment_class)),
                body: vec![CodegenNode::NativeBlock {
                    code: deser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("var j: [String: Any] = [:]\n");
            save_body.push_str("j[\"_compartment\"] = __serComp(__compartment) as Any\n");
            save_body.push_str("var stack: [[String: Any]] = []\n");
            save_body.push_str(
                "for c in _state_stack { if let s = __serComp(c) { stack.append(s) } }\n",
            );
            save_body.push_str("j[\"_state_stack\"] = stack\n");
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "if let __raw_{0} = {0}.saveState().data(using: .utf8), let __nested_{0} = try? JSONSerialization.jsonObject(with: __raw_{0}) {{ j[\"{0}\"] = __nested_{0} }}\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("j[\"{}\"] = {}\n", var.name, var.name));
                }
            }
            save_body.push_str("let data = try! JSONSerialization.data(withJSONObject: j)\n");
            save_body.push_str("return String(data: data, encoding: .utf8)!");

            methods.push(CodegenNode::Method {
                name: "saveState".to_string(),
                params: vec![],
                return_type: Some("String".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restoreState — static method.
            //
            // Sets `__skipInitialEnter` before calling init so the
            // initial-state $>() enter handler does NOT fire on a
            // restored instance. The flag is a class-level static;
            // it's reset immediately after init to keep the default
            // (fire-ENTER-on-construct) behavior for subsequent
            // `Canary()` calls. Matches Python's pickle semantics
            // where restore does not invoke __init__.
            let mut restore_body = String::new();
            restore_body.push_str("let data = json.data(using: .utf8)!\n");
            restore_body.push_str(
                "let j = try! JSONSerialization.jsonObject(with: data) as! [String: Any]\n",
            );
            restore_body.push_str(&format!("{}.__skipInitialEnter = true\n", sys));
            restore_body.push_str(&format!("let instance = {}()\n", sys));
            restore_body.push_str(&format!("{}.__skipInitialEnter = false\n", sys));
            restore_body.push_str("instance.__compartment = instance.__deserComp(j[\"_compartment\"] as? [String: Any])!\n");
            restore_body.push_str("if let stack = j[\"_state_stack\"] as? [[String: Any]] {\n");
            restore_body.push_str("    instance._state_stack = []\n");
            restore_body.push_str("    for sc in stack { if let c = instance.__deserComp(sc) { instance._state_stack.append(c) } }\n");
            restore_body.push_str("}\n");
            // Type-ignorant: wrap the value in an array, encode to
            // JSON via JSONSerialization, then decode as [T].first.
            // The array-wrap trick avoids JSONSerialization's
            // top-level-must-be-container restriction. framec emits
            // `T` verbatim; Codable conformance handles the typing.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    restore_body.push_str(&format!(
                        "if let __raw_{0} = j[\"{0}\"], let __data_{0} = try? JSONSerialization.data(withJSONObject: __raw_{0}), let __json_{0} = String(data: __data_{0}, encoding: .utf8) {{ instance.{0} = {1}.restoreState(__json_{0}) }}\n",
                        var.name, child_sys
                    ));
                } else {
                    let swift_type = match &var.var_type {
                        crate::frame_c::compiler::frame_ast::Type::Custom(t) => swift_map_type(t),
                        _ => "Any".to_string(),
                    };
                    if swift_type == "Any" {
                        restore_body.push_str(&format!(
                            "if let v = j[\"{0}\"] {{ instance.{0} = v }}\n",
                            var.name
                        ));
                    } else {
                        restore_body.push_str(&format!(
                            "if let __raw = j[\"{name}\"], let __data = try? JSONSerialization.data(withJSONObject: [__raw]), let __arr = try? JSONDecoder().decode([{t}].self, from: __data), let __v = __arr.first {{ instance.{name} = __v }}\n",
                            name = var.name,
                            t = swift_type
                        ));
                    }
                }
            }
            restore_body.push_str("return instance");

            methods.push(CodegenNode::Method {
                name: "restoreState".to_string(),
                params: vec![Param::new("json").with_type("String")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Ruby => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Ruby uses JSON (stdlib) — hash-based serialization
            // Private helper: serialize compartment chain
            let mut ser_body = String::new();
            ser_body.push_str("return nil if comp.nil?\n");
            ser_body.push_str("j = {}\n");
            ser_body.push_str("j[\"state\"] = comp.state\n");
            ser_body.push_str("sv = {}\n");
            ser_body.push_str("comp.state_vars.each { |k, v| sv[k] = v }\n");
            ser_body.push_str("j[\"state_vars\"] = sv\n");
            ser_body.push_str("j[\"state_args\"] = comp.state_args\n");
            ser_body.push_str("j[\"enter_args\"] = comp.enter_args\n");
            ser_body.push_str("j[\"parent\"] = __ser_comp(comp.parent_compartment)\n");
            ser_body.push_str("j");

            methods.push(CodegenNode::Method {
                name: "__ser_comp".to_string(),
                params: vec![Param::new("comp")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: ser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // Private helper: deserialize compartment chain
            let mut deser_body = String::new();
            deser_body.push_str("return nil if data.nil?\n");
            deser_body.push_str(&format!("c = {}.new(data[\"state\"])\n", compartment_class));
            deser_body.push_str("if data[\"state_vars\"]\n");
            deser_body.push_str("  data[\"state_vars\"].each { |k, v| c.state_vars[k] = v }\n");
            deser_body.push_str("end\n");
            deser_body.push_str("c.state_args = data[\"state_args\"] if data[\"state_args\"]\n");
            deser_body.push_str("c.enter_args = data[\"enter_args\"] if data[\"enter_args\"]\n");
            deser_body.push_str("if data[\"parent\"]\n");
            deser_body.push_str("  c.parent_compartment = __deser_comp(data[\"parent\"])\n");
            deser_body.push_str("end\n");
            deser_body.push_str("c");

            methods.push(CodegenNode::Method {
                name: "__deser_comp".to_string(),
                params: vec![Param::new("data")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: deser_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("j = {}\n");
            save_body.push_str("j[\"_compartment\"] = __ser_comp(@__compartment)\n");
            save_body.push_str("stack = []\n");
            save_body.push_str("@_state_stack.each { |c| stack.push(__ser_comp(c)) }\n");
            save_body.push_str("j[\"_state_stack\"] = stack\n");
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "j[\"{0}\"] = @{0}.nil? ? nil : JSON.parse(@{0}.save_state)\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("j[\"{}\"] = @{}\n", var.name, var.name));
                }
            }
            save_body.push_str("JSON.generate(j)");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restore_state(json) — class method (static).
            //
            // Uses `allocate` instead of `new` so the constructor does NOT
            // run. This matters because the generated `initialize` dispatches
            // the initial state's `$>()` enter handler; a restored instance
            // must not re-fire that (it would leak side effects of a state
            // the caller never logically entered from). Instance variables
            // that `initialize` would normally set up are populated here
            // explicitly from the saved blob.
            let mut restore_body = String::new();
            restore_body.push_str("j = JSON.parse(json)\n");
            restore_body.push_str(&format!("instance = {}.allocate\n", sys));
            restore_body.push_str("instance.instance_variable_set(:@_context_stack, [])\n");
            restore_body.push_str("instance.instance_variable_set(:@__next_compartment, nil)\n");
            restore_body.push_str("instance.instance_variable_set(:@__compartment, instance.send(:__deser_comp, j[\"_compartment\"]))\n");
            restore_body.push_str("if j[\"_state_stack\"]\n");
            restore_body.push_str("  instance.instance_variable_set(:@_state_stack, j[\"_state_stack\"].map { |sc| instance.send(:__deser_comp, sc) })\n");
            restore_body.push_str("else\n");
            restore_body.push_str("  instance.instance_variable_set(:@_state_stack, [])\n");
            restore_body.push_str("end\n");
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    restore_body.push_str(&format!(
                        "if j.key?(\"{0}\") then instance.{0} = j[\"{0}\"].nil? ? nil : {1}.restore_state(JSON.generate(j[\"{0}\"])) end\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!(
                        "instance.{} = j[\"{}\"] if j.key?(\"{}\")\n",
                        var.name, var.name, var.name
                    ));
                }
            }
            restore_body.push_str("instance");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Go => {
            let compartment_type = format!("{}Compartment", system.name);

            // save_state — serialize to JSON via encoding/json
            let mut save_body = String::new();
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
                name: "SaveState".to_string(),
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

            // restore_state — static function
            let mut restore_body = String::new();
            restore_body.push_str("var data map[string]interface{}\n");
            restore_body.push_str("json.Unmarshal([]byte(jsonStr), &data)\n");
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
            restore_body.push_str("        if f, ok := v.(float64); ok && f == float64(int(f)) { return int(f) }\n");
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
            let go_typed_conv = |declared_type: &str,
                                  idx: usize,
                                  slot: &str|
             -> String {
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
                                            crate::frame_c::compiler::frame_ast::Type::Custom(s) => {
                                                s.clone()
                                            }
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
            restore_body.push_str(&format!("instance := &{}{{}}\n", system.name));
            restore_body
                .push_str("instance.__compartment = deserializeComp(data[\"_compartment\"])\n");
            restore_body.push_str("instance.__next_compartment = nil\n");
            restore_body.push_str("if stack, ok := data[\"_state_stack\"].([]interface{}); ok {\n");
            restore_body.push_str(&format!(
                "    instance._state_stack = make([]*{}, 0, len(stack))\n",
                compartment_type
            ));
            restore_body.push_str("    for _, c := range stack { instance._state_stack = append(instance._state_stack, deserializeComp(c)) }\n");
            restore_body.push_str("}\n");
            // Type-ignorant domain restore via marshal-roundtrip.
            // framec emits the declared Go type verbatim;
            // encoding/json reflection handles primitives, slices,
            // maps, and user structs alike.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    // Nested @@SystemName(): re-hydrate via child Restore<Name>.
                    restore_body.push_str(&format!(
                        "if __raw_{0}, err_{0} := json.Marshal(data[\"{0}\"]); err_{0} == nil {{ instance.{0} = Restore{1}(string(__raw_{0})) }}\n",
                        var.name, child_sys
                    ));
                } else {
                    let declared = match &var.var_type {
                        crate::frame_c::compiler::frame_ast::Type::Custom(name) => go_map_type(name),
                        _ => "interface{}".to_string(),
                    };
                    let go_extract = format!(
                        "func() {t} {{ var __typed {t}; if __raw, err := json.Marshal(data[\"{name}\"]); err == nil {{ json.Unmarshal(__raw, &__typed) }}; return __typed }}()",
                        t = declared,
                        name = var.name,
                    );
                    restore_body.push_str(&format!("instance.{} = {}\n", var.name, go_extract));
                }
            }
            restore_body.push_str("return instance");

            methods.push(CodegenNode::Method {
                name: format!("Restore{}", system.name),
                params: vec![Param::new("jsonStr").with_type("string")],
                return_type: Some(format!("*{}", system.name)),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Erlang => {
            // Persistence handled in erlang_system.rs via gen_statem save_state/load_state
        }
        TargetLanguage::Dart => {
            let compartment_type = format!("{}Compartment", system.name);

            // save_state — serialize to JSON via dart:convert
            let mut save_body = String::new();
            save_body.push_str(&format!(
                "Map<String, dynamic>? serializeComp({}? comp) {{\n",
                compartment_type
            ));
            save_body.push_str("    if (comp == null) return null;\n");
            save_body.push_str("    return {\n");
            save_body.push_str("        'state': comp.state,\n");
            // state_args / enter_args / exit_args are List<dynamic> (positional)
            // since the HashMap→Vec migration. state_vars stays a Map keyed by var name.
            save_body.push_str("        'state_args': List<dynamic>.from(comp.state_args),\n");
            save_body
                .push_str("        'state_vars': Map<String, dynamic>.from(comp.state_vars),\n");
            save_body.push_str("        'enter_args': List<dynamic>.from(comp.enter_args),\n");
            save_body.push_str("        'exit_args': List<dynamic>.from(comp.exit_args),\n");
            save_body.push_str("        'forward_event': comp.forward_event,\n");
            save_body.push_str(
                "        'parent_compartment': serializeComp(comp.parent_compartment),\n",
            );
            save_body.push_str("    };\n");
            save_body.push_str("}\n");
            save_body.push_str("return jsonEncode({\n");
            save_body.push_str("    '_compartment': serializeComp(this.__compartment),\n");
            save_body.push_str(
                "    '_state_stack': this._state_stack.map((c) => serializeComp(c)).toList(),\n",
            );
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "    '{0}': this.{0} != null ? jsonDecode(this.{0}.saveState()) : null,\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("    '{}': this.{},\n", var.name, var.name));
                }
            }
            save_body.push_str("});");

            methods.push(CodegenNode::Method {
                name: "saveState".to_string(),
                params: vec![],
                return_type: Some("String".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // _restore — private named constructor for deserialization.
            // Creates an uninitialized instance (skips the normal
            // constructor's $> enter event and compartment setup).
            // Initializes `late` fields (_state_stack, _context_stack)
            // so the instance is safe to touch before restoreState()
            // finishes populating the compartment chain.
            methods.push(CodegenNode::NativeBlock {
                code: format!(
                    "{system}._restore() : __compartment = {comp}(\"\"), __next_compartment = null {{\n\
                     \x20   _state_stack = [];\n\
                     \x20   _context_stack = [];\n\
                     }}",
                    system = system.name,
                    comp = compartment_type,
                ),
                span: None,
            });

            // Per-state typed restore data. For each state with declared
            // params, we'll switch on the state name in deserializeComp
            // and emit per-arg Dart comprehension conversion. Same idea
            // as the JVM Jackson + TypeReference migration: framec emits
            // the user's declared type strings verbatim into a
            // comprehension; Dart's reified generics then carry the
            // typed shape through the rest of the runtime so user
            // handlers can index without defensive casts.
            //
            // .cast<>() (the previous pattern) silently fails for
            // nested generics — `Map.cast<String, List<int>>()` returns
            // a view that throws at element access because the inner
            // List<dynamic> is not actually a List<int>. Comprehensions
            // construct genuinely typed collections, which is the only
            // bridge that survives Dart's reification rules.
            let dart_state_param_types: Vec<(String, Vec<String>)> = system
                .machine
                .as_ref()
                .map(|m| {
                    m.states
                        .iter()
                        .filter(|s| !s.params.is_empty())
                        .map(|s| {
                            let types: Vec<String> = s
                                .params
                                .iter()
                                .map(|p| match &p.param_type {
                                    crate::frame_c::compiler::frame_ast::Type::Custom(t) => {
                                        t.trim().to_string()
                                    }
                                    _ => "dynamic".to_string(),
                                })
                                .collect();
                            (s.name.clone(), types)
                        })
                        .collect()
                })
                .unwrap_or_default();

            // restore_state — static method
            let mut restore_body = String::new();
            restore_body.push_str(&format!(
                "{}? deserializeComp(dynamic data) {{\n",
                compartment_type
            ));
            restore_body.push_str("    if (data == null || data is! Map) return null;\n");
            restore_body.push_str(&format!(
                "    final comp = {}(data['state'] as String);\n",
                compartment_type
            ));
            restore_body.push_str(
                "    comp.state_vars = Map<String, dynamic>.from(data['state_vars'] ?? {});\n",
            );
            restore_body.push_str(
                "    final __saRaw = (data['state_args'] as List?) ?? <dynamic>[];\n",
            );
            restore_body.push_str(
                "    final __eaRaw = (data['enter_args'] as List?) ?? <dynamic>[];\n",
            );
            restore_body.push_str(
                "    comp.exit_args = List<dynamic>.from(data['exit_args'] ?? <dynamic>[]);\n",
            );
            if !dart_state_param_types.is_empty() {
                restore_body.push_str("    switch (comp.state) {\n");
                for (state_name, param_types) in &dart_state_param_types {
                    restore_body.push_str(&format!("        case '{}':\n", state_name));
                    for (i, ty_str) in param_types.iter().enumerate() {
                        let parsed = parse_dart_type(ty_str);
                        let conv_sa = dart_conv_expr(&parsed, &format!("__saRaw[{i}]"));
                        let conv_ea = dart_conv_expr(&parsed, &format!("__eaRaw[{i}]"));
                        restore_body.push_str(&format!(
                            "            if (__saRaw.length > {i}) comp.state_args.add({conv_sa});\n"
                        ));
                        restore_body.push_str(&format!(
                            "            if (__eaRaw.length > {i}) comp.enter_args.add({conv_ea});\n"
                        ));
                    }
                    restore_body.push_str("            break;\n");
                }
                restore_body.push_str("        default:\n");
                restore_body
                    .push_str("            comp.state_args.addAll(__saRaw);\n");
                restore_body
                    .push_str("            comp.enter_args.addAll(__eaRaw);\n");
                restore_body.push_str("            break;\n");
                restore_body.push_str("    }\n");
            } else {
                restore_body.push_str("    comp.state_args.addAll(__saRaw);\n");
                restore_body.push_str("    comp.enter_args.addAll(__eaRaw);\n");
            }
            restore_body.push_str("    comp.forward_event = data['forward_event'];\n");
            restore_body.push_str(
                "    comp.parent_compartment = deserializeComp(data['parent_compartment']);\n",
            );
            restore_body.push_str("    return comp;\n");
            restore_body.push_str("}\n");
            restore_body.push_str("final data = jsonDecode(json) as Map<String, dynamic>;\n");
            restore_body.push_str(&format!("final instance = {}._restore();\n", system.name));
            restore_body
                .push_str("instance.__compartment = deserializeComp(data['_compartment'])!;\n");
            restore_body.push_str("instance.__next_compartment = null;\n");
            restore_body.push_str(&format!(
                "instance._state_stack = (data['_state_stack'] as List?)?.map((c) => deserializeComp(c)!).toList() ?? <{}>[];\n",
                compartment_type
            ));
            // Domain field restores via the comprehension emitter.
            // For Map<String, List<int>>, emits something like:
            //   instance.x = <String, List<int>>{for (var __me in (data['x'] as Map).entries) __me.key as String: <int>[for (var __e in (__me.value as List)) (__e as num).toInt()]};
            // This produces a genuinely typed collection rather than
            // the broken `.cast<>()` view that was in place before.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    // Nested @@SystemName instance: re-hydrate via child's
                    // restoreState. data[name] is the embedded JSON object;
                    // jsonEncode it back to string and let child rebuild.
                    restore_body.push_str(&format!(
                        "instance.{0} = data['{0}'] != null ? {1}.restoreState(jsonEncode(data['{0}'])) : {1}();\n",
                        var.name, child_sys
                    ));
                } else {
                    let ty = match &var.var_type {
                        crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.trim(),
                        _ => "dynamic",
                    };
                    let parsed = parse_dart_type(ty);
                    let conv = dart_conv_expr(&parsed, &format!("data['{}']", var.name));
                    restore_body.push_str(&format!("instance.{} = {};\n", var.name, conv));
                }
            }
            restore_body.push_str("return instance;");

            methods.push(CodegenNode::Method {
                name: "restoreState".to_string(),
                params: vec![Param::new("json").with_type("String")],
                return_type: Some(system.name.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Lua => {
            let compartment_type = format!("{}Compartment", system.name);

            // save_state — serialize to JSON via cjson
            let mut save_body = String::new();
            save_body.push_str("local json = require(\"cjson\")\n");
            save_body.push_str("local function serialize_comp(comp)\n");
            save_body.push_str("    if not comp then return nil end\n");
            save_body.push_str("    local t = {}\n");
            save_body.push_str("    t.state = comp.state\n");
            save_body.push_str("    t.state_args = comp.state_args\n");
            save_body.push_str("    t.state_vars = comp.state_vars\n");
            save_body.push_str("    t.enter_args = comp.enter_args\n");
            save_body.push_str("    t.exit_args = comp.exit_args\n");
            save_body.push_str("    t.forward_event = comp.forward_event\n");
            save_body
                .push_str("    t.parent_compartment = serialize_comp(comp.parent_compartment)\n");
            save_body.push_str("    return t\n");
            save_body.push_str("end\n");
            save_body.push_str("local stack = {}\n");
            save_body.push_str("for _, c in ipairs(self._state_stack) do\n");
            save_body.push_str("    stack[#stack + 1] = serialize_comp(c)\n");
            save_body.push_str("end\n");
            save_body.push_str("local result = {}\n");
            save_body.push_str("result._compartment = serialize_comp(self.__compartment)\n");
            save_body.push_str("result._state_stack = stack\n");
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "result.{0} = (self.{0} ~= nil) and json.decode(self.{0}:save_state()) or nil\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!("result.{} = self.{}\n", var.name, var.name));
                }
            }
            save_body.push_str("return json.encode(result)");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
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

            // restore_state — module-level function
            let mut restore_body = String::new();
            restore_body.push_str("local json = require(\"cjson\")\n");
            restore_body.push_str("local data = json.decode(json_str)\n");
            restore_body.push_str("local function deserialize_comp(d)\n");
            restore_body.push_str("    if not d then return nil end\n");
            restore_body.push_str(&format!(
                "    local comp = {}.new(d.state)\n",
                compartment_type
            ));
            restore_body.push_str("    comp.state_args = d.state_args or {}\n");
            restore_body.push_str("    comp.state_vars = d.state_vars or {}\n");
            restore_body.push_str("    comp.enter_args = d.enter_args or {}\n");
            restore_body.push_str("    comp.exit_args = d.exit_args or {}\n");
            restore_body.push_str("    comp.forward_event = d.forward_event\n");
            restore_body
                .push_str("    comp.parent_compartment = deserialize_comp(d.parent_compartment)\n");
            restore_body.push_str("    return comp\n");
            restore_body.push_str("end\n");
            restore_body.push_str(&format!("local instance = {{}}\n"));
            restore_body.push_str(&format!(
                "setmetatable(instance, {{__index = {}}})\n",
                system.name
            ));
            restore_body.push_str("instance.__compartment = deserialize_comp(data._compartment)\n");
            restore_body.push_str("instance.__next_compartment = nil\n");
            restore_body.push_str("instance._state_stack = {}\n");
            // _context_stack is pushed/popped per interface call; a restored
            // instance starts with an empty context just like a fresh one.
            restore_body.push_str("instance._context_stack = {}\n");
            restore_body.push_str("if data._state_stack then\n");
            restore_body.push_str("    for _, c in ipairs(data._state_stack) do\n");
            restore_body.push_str(
                "        instance._state_stack[#instance._state_stack + 1] = deserialize_comp(c)\n",
            );
            restore_body.push_str("    end\n");
            restore_body.push_str("end\n");
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    restore_body.push_str(&format!(
                        "if data.{0} ~= nil then instance.{0} = {1}.restore_state(json.encode(data.{0})) else instance.{0} = nil end\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!("instance.{} = data.{}\n", var.name, var.name));
                }
            }
            restore_body.push_str("return instance");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json_str").with_type("string")],
                return_type: Some(system.name.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::GDScript => {
            // GDScript: JSON-based persistence using serialize_comp helper
            let compartment_type = format!("{}Compartment", system.name);

            // save_state method - iterative serialization (GDScript lambdas can't recurse)
            let mut save_body = String::new();
            // Serialize a compartment chain iteratively: collect into array, then build dicts bottom-up
            save_body.push_str("# Serialize compartment chain iteratively\n");
            save_body.push_str("var _ser_chain = func(comp):\n");
            save_body.push_str("    var chain = []\n");
            save_body.push_str("    var cur = comp\n");
            save_body.push_str("    while cur != null:\n");
            save_body.push_str("        chain.append(cur)\n");
            save_body.push_str("        cur = cur.parent_compartment\n");
            save_body.push_str("    chain.reverse()\n");
            save_body.push_str("    var result = null\n");
            save_body.push_str("    for c in chain:\n");
            save_body.push_str("        var d = {}\n");
            save_body.push_str("        d[\"state\"] = c.state\n");
            save_body.push_str("        d[\"state_args\"] = c.state_args.duplicate()\n");
            save_body.push_str("        d[\"state_vars\"] = c.state_vars.duplicate()\n");
            save_body.push_str("        d[\"enter_args\"] = c.enter_args.duplicate()\n");
            save_body.push_str("        d[\"exit_args\"] = c.exit_args.duplicate()\n");
            save_body.push_str("        d[\"parent_compartment\"] = result\n");
            save_body.push_str("        result = d\n");
            save_body.push_str("    return result\n");
            save_body.push_str("var state_data = {}\n");
            save_body
                .push_str("state_data[\"_compartment\"] = _ser_chain.call(self.__compartment)\n");
            save_body.push_str("var stack_arr = []\n");
            save_body.push_str("for c in self._state_stack:\n");
            save_body.push_str("    stack_arr.append(_ser_chain.call(c))\n");
            save_body.push_str("state_data[\"_state_stack\"] = stack_arr\n");

            // Add domain variables. Nested @@SystemName() instances
            // round-trip via child save_state/restore_state.
            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if extract_tagged_system_name(init).is_some() {
                    save_body.push_str(&format!(
                        "state_data[\"{0}\"] = bytes_to_var(self.{0}.save_state()) if self.{0} != null else null\n",
                        var.name
                    ));
                } else {
                    save_body.push_str(&format!(
                        "state_data[\"{}\"] = self.{}\n",
                        var.name, var.name
                    ));
                }
            }

            save_body.push_str("return var_to_bytes(state_data)");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: Some("PackedByteArray".to_string()),
                body: vec![CodegenNode::NativeBlock {
                    code: save_body,
                    span: None,
                }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restore_state static method - iterative deserialization
            let mut restore_body = String::new();
            restore_body.push_str("var state_data = bytes_to_var(data)\n");
            // Deserialize compartment chain iteratively
            restore_body.push_str("var _deser_chain = func(d):\n");
            restore_body.push_str("    if d == null:\n");
            restore_body.push_str("        return null\n");
            restore_body.push_str("    # Collect chain into array (child first)\n");
            restore_body.push_str("    var chain = []\n");
            restore_body.push_str("    var cur = d\n");
            restore_body.push_str("    while cur != null:\n");
            restore_body.push_str("        chain.append(cur)\n");
            restore_body.push_str("        cur = cur.get(\"parent_compartment\", null)\n");
            restore_body.push_str("    chain.reverse()\n");
            restore_body.push_str("    var result = null\n");
            restore_body.push_str("    for cd in chain:\n");
            restore_body.push_str(&format!(
                "        var comp = {}.new(cd[\"state\"])\n",
                compartment_type
            ));
            restore_body.push_str("        comp.state_args = cd.get(\"state_args\", {})\n");
            restore_body.push_str("        comp.state_vars = cd.get(\"state_vars\", {})\n");
            restore_body.push_str("        comp.enter_args = cd.get(\"enter_args\", {})\n");
            restore_body.push_str("        comp.exit_args = cd.get(\"exit_args\", {})\n");
            restore_body.push_str("        comp.parent_compartment = result\n");
            restore_body.push_str("        result = comp\n");
            restore_body.push_str("    return result\n");

            // Set the class-static __skipInitialEnter before constructing
            // so the initial-state $>() handler is not re-fired on a
            // restored instance. See Swift comment for the pattern.
            restore_body.push_str(&format!("{}.__skipInitialEnter = true\n", system.name));
            restore_body.push_str(&format!("var instance = {}.new()\n", system.name));
            restore_body.push_str(&format!("{}.__skipInitialEnter = false\n", system.name));
            restore_body.push_str(
                "instance.__compartment = _deser_chain.call(state_data[\"_compartment\"])\n",
            );
            restore_body.push_str("instance.__next_compartment = null\n");
            restore_body.push_str("instance._state_stack = []\n");
            restore_body.push_str("for c in state_data.get(\"_state_stack\", []):\n");
            restore_body.push_str("    instance._state_stack.append(_deser_chain.call(c))\n");
            restore_body.push_str("instance._context_stack = []\n");

            for var in &system.domain {
                let init = var.initializer_text.as_deref().unwrap_or("");
                if let Some(child_sys) = extract_tagged_system_name(init) {
                    restore_body.push_str(&format!(
                        "var __raw_{0} = state_data.get(\"{0}\", null)\n",
                        var.name
                    ));
                    restore_body.push_str(&format!(
                        "instance.{0} = {1}.restore_state(var_to_bytes(__raw_{0})) if __raw_{0} != null else null\n",
                        var.name, child_sys
                    ));
                } else {
                    restore_body.push_str(&format!(
                        "instance.{} = state_data.get(\"{}\", null)\n",
                        var.name, var.name
                    ));
                }
            }

            restore_body.push_str("return instance");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("data").with_type("PackedByteArray")],
                return_type: Some(system.name.clone()),
                body: vec![CodegenNode::NativeBlock {
                    code: restore_body,
                    span: None,
                }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Graphviz => unreachable!(),
    }

    methods
}

// ============================================================================
// Erlang gen_statem code generation
// ============================================================================
