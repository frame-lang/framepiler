//! Interface wrapper, action, operation, and persistence code generation.
//!
//! Generates:
//! - Interface method wrappers (public API → kernel dispatch)
//! - Action method bodies (native code with self.X rewriting)
//! - Operation method bodies (static/class methods)
//! - Persistence serialization/deserialization methods

use crate::frame_c::visitors::TargetLanguage;
use crate::frame_c::compiler::frame_ast::{SystemAst, ActionAst, OperationAst, Type, InterfaceMethod, MethodParam, Span};
use super::ast::{CodegenNode, Param, Visibility};
use super::codegen_utils::{
    HandlerContext, expression_to_string, type_to_string, to_snake_case,
    cpp_map_type, cpp_wrap_any_arg, java_map_type, kotlin_map_type,
    swift_map_type, csharp_map_type, go_map_type, type_to_cpp_string,
    extract_type_from_raw_domain, is_int_type, is_float_type, is_bool_type, is_string_type,
};



/// Generate interface wrapper methods
///
/// For Python/TypeScript: Create FrameEvent and call __kernel
/// For Rust: Use match-based dispatch directly
///
/// If no explicit interface is defined, auto-generate interface methods from
/// unique event handlers found in the machine states (excluding lifecycle events).
pub(crate) fn generate_interface_wrappers(system: &SystemAst, syntax: &super::backend::ClassSyntax) -> Vec<CodegenNode> {
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
        let mut method_info: std::collections::HashMap<String, (Vec<MethodParam>, Option<Type>)> = std::collections::HashMap::new();

        if let Some(ref machine) = system.machine {
            for state in &machine.states {
                for handler in &state.handlers {
                    // Skip lifecycle events
                    if handler.event == "$>" || handler.event == "<$" || handler.event == "$>|" || handler.event == "<$|" {
                        continue;
                    }
                    if events.insert(handler.event.clone()) {
                        // First time seeing this event - capture its params and return type
                        let params: Vec<MethodParam> = handler.params.iter().map(|p| {
                            MethodParam {
                                name: p.name.clone(),
                                param_type: p.param_type.clone(),
                                default: None,
                                span: Span::new(0, 0),
                            }
                        }).collect();
                        method_info.insert(handler.event.clone(), (params, handler.return_type.clone()));
                    }
                }
            }
        }

        events.into_iter().map(|event| {
            let (params, return_type) = method_info.get(&event).cloned().unwrap_or_default();
            InterfaceMethod {
                name: event,
                params,
                return_type,
                return_init: None,
                is_async: false,
                span: Span::new(0, 0),
            }
        }).collect()
    };

    interface_methods.iter().map(|method| {
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
                // Rust: Use context stack pattern per V4 spec
                // 1. Create FrameEvent with parameters
                // 2. Create FrameContext with event and default return
                // 3. Push context to _context_stack
                // 4. Call __kernel
                // 5. Pop context and return _return (with downcast)
                let context_class = format!("{}FrameContext", system.name);

                // Build parameters HashMap insertion code
                let params_code = if method.params.is_empty() {
                    String::new()
                } else {
                    // Clone parameters before boxing since we also pass them directly to handlers
                    method.params.iter()
                        .map(|p| format!("__e.parameters.insert(\"{}\".to_string(), Box::new({}.clone()) as Box<dyn std::any::Any>);", p.name, p.name))
                        .collect::<Vec<_>>()
                        .join("\n")
                };

                let mut match_code = format!("let mut __e = {}::new(\"{}\");\n", event_class, method.name);
                if !params_code.is_empty() {
                    match_code.push_str(&params_code);
                    match_code.push('\n');
                }

                // Create context with event (move, not clone) and push to stack
                match_code.push_str(&format!("let mut __ctx = {}::new(__e, None);\n", context_class));
                if let Some(ref init_expr) = method.return_init {
                    match_code.push_str(&format!("__ctx._return = Some(Box::new({}) as Box<dyn std::any::Any>);\n", init_expr));
                }
                match_code.push_str("self._context_stack.push(__ctx);\n");

                // Call kernel to route event and process transitions
                // Kernel gets event from context stack
                match_code.push_str("self.__kernel();\n");

                // Pop context and return
                if let Some(ref rt) = method.return_type {
                    let return_type = type_to_string(rt);
                    match_code.push_str(&format!(
                        r#"let __ctx = self._context_stack.pop().unwrap();
if let Some(ret) = __ctx._return {{
    *ret.downcast::<{}>().unwrap()
}} else {{
    Default::default()
}}"#, return_type));
                } else {
                    match_code.push_str("self._context_stack.pop();");
                }

                CodegenNode::NativeBlock {
                    code: match_code,
                    span: None,
                }
            }
            TargetLanguage::Python3 => {
                // Python: Create FrameEvent + FrameContext, push context, call __kernel, pop and return
                // Parameters are passed as a dict with parameter names as keys for @@ syntax access
                let context_class = format!("{}FrameContext", system.name);
                let params_code = if method.params.is_empty() {
                    "None".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\": {}", p.name, p.name))
                        .collect();
                    format!("{{{}}}", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                let has_return = method.return_type.is_some() || method.return_init.is_some();
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
                // Parameters are passed as a dict with parameter names as keys for @@ syntax access
                let params_code = if method.params.is_empty() {
                    "null".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\": {}", p.name, p.name))
                        .collect();
                    format!("{{{}}}", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {};", init_expr)
                } else {
                    String::new()
                };

                // TypeScript uses ! (non-null assertion), JavaScript doesn't
                let pop_suffix = if matches!(lang, TargetLanguage::TypeScript) { "!" } else { "" };

                if method.return_type.is_some() || method.return_init.is_some() {
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
                    "null".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\" => ${}", p.name, p.name))
                        .collect();
                    format!("[{}]", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n$__ctx->_return = {};", init_expr)
                } else {
                    String::new()
                };

                if method.return_type.is_some() || method.return_init.is_some() {
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
                    "{}".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\" => {}", p.name, p.name))
                        .collect();
                    format!("{{{}}}", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                if method.return_type.is_some() || method.return_init.is_some() {
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

                // Build parameters dict creation (with semicolon)
                let params_code = if method.params.is_empty() {
                    format!("{}_FrameEvent* __e = {}_FrameEvent_new(\"{}\", NULL);", sys, sys, method.name)
                } else {
                    let mut code = format!("{}_FrameDict* __params = {}_FrameDict_new();\n", sys, sys);
                    for p in &method.params {
                        code.push_str(&format!("{}_FrameDict_set(__params, \"{}\", (void*)(intptr_t){});\n", sys, p.name, p.name));
                    }
                    code.push_str(&format!("{}_FrameEvent* __e = {}_FrameEvent_new(\"{}\", __params);", sys, sys, method.name));
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

                // Set default return value after context creation
                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx->_return = (void*)(intptr_t)({});", init_expr)
                } else {
                    String::new()
                };

                if let (true, Some(return_type_str)) = (has_return_value, return_type_str) {
                    let cast = match return_type_str.as_str() {
                        "bool" | "int" => "(intptr_t)",
                        _ => "",
                    };
                    CodegenNode::NativeBlock {
                        code: format!(
                            r#"{}
{}_FrameContext* __ctx = {}_FrameContext_new(__e, NULL);{}
{}_FrameVec_push(self->_context_stack, __ctx);
{}_kernel(self, __e);
{}_FrameContext* __result_ctx = ({}_FrameContext*){}_FrameVec_pop(self->_context_stack);
{} __result = ({}){}__result_ctx->_return;
{}_FrameContext_destroy(__result_ctx);
{}_FrameEvent_destroy(__e);
return __result;"#,
                            params_code, sys, sys, default_init, sys, sys, sys, sys, sys,
                            return_type_str, return_type_str, cast, sys, sys
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

                let mut code = String::new();

                // Build params map
                if !method.params.is_empty() {
                    code.push_str("std::unordered_map<std::string, std::any> __params;\n");
                    for param in &method.params {
                        code.push_str(&format!("__params[\"{}\"] = {};\n", param.name, param.name));
                    }
                    code.push_str(&format!("{} __e(\"{}\", std::move(__params));\n", event_class, method.name));
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
                code.push_str("__kernel(_context_stack.back()._event);\n");

                if returns_value {
                    code.push_str(&format!("auto __result = std::any_cast<{}>(std::move(_context_stack.back()._return));\n", return_type_str));
                    code.push_str("_context_stack.pop_back();\n");
                    code.push_str("return __result;");
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

                let mut code = String::new();

                // Build params map
                if !method.params.is_empty() {
                    code.push_str("HashMap<String, Object> __params = new HashMap<>();\n");
                    for param in &method.params {
                        code.push_str(&format!("__params.put(\"{}\", {});\n", param.name, param.name));
                    }
                    code.push_str(&format!("{} __e = new {}(\"{}\", __params);\n", event_class, event_class, method.name));
                } else {
                    code.push_str(&format!("{} __e = new {}(\"{}\");\n", event_class, event_class, method.name));
                }

                // Create context with default return
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("{} __ctx = new {}(__e, {});\n", context_class, context_class, init));
                } else if has_return && return_type_str != "void" {
                    code.push_str(&format!("{} __ctx = new {}(__e, null);\n", context_class, context_class));
                } else {
                    code.push_str(&format!("{} __ctx = new {}(__e, null);\n", context_class, context_class));
                }

                code.push_str("_context_stack.add(__ctx);\n");
                code.push_str("__kernel(_context_stack.get(_context_stack.size() - 1)._event);\n");

                if has_return && return_type_str != "void" && return_type_str != "Any" && return_type_str != "Object" {
                    let java_type = java_map_type(&return_type_str);
                    code.push_str(&format!("var __result = ({}) _context_stack.get(_context_stack.size() - 1)._return;\n", java_type));
                    code.push_str("_context_stack.remove(_context_stack.size() - 1);\n");
                    code.push_str("return __result;");
                } else {
                    code.push_str("_context_stack.remove(_context_stack.size() - 1);");
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

                // Build params map — Kotlin: no new, no semicolons, mutableMapOf
                if !method.params.is_empty() {
                    code.push_str("val __params = mutableMapOf<String, Any?>(");
                    let entries: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\" to {}", p.name, p.name))
                        .collect();
                    code.push_str(&entries.join(", "));
                    code.push_str(")\n");
                    code.push_str(&format!("val __e = {}(\"{}\", __params)\n", event_class, method.name));
                } else {
                    code.push_str(&format!("val __e = {}(\"{}\")\n", event_class, method.name));
                }

                // Create context with default return
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("val __ctx = {}(__e, {})\n", context_class, init));
                } else if has_return && return_type_str != "void" {
                    code.push_str(&format!("val __ctx = {}(__e, null)\n", context_class));
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

                // Build params map — Swift: [String: Any] dictionary literal
                if !method.params.is_empty() {
                    code.push_str("let __params: [String: Any] = [");
                    let entries: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\": {}", p.name, p.name))
                        .collect();
                    code.push_str(&entries.join(", "));
                    code.push_str("]\n");
                    code.push_str(&format!("let __e = {}(message: \"{}\", parameters: __params)\n", event_class, method.name));
                } else {
                    code.push_str(&format!("let __e = {}(message: \"{}\")\n", event_class, method.name));
                }

                // Create context with default return
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("let __ctx = {}(event: __e, defaultReturn: {})\n", context_class, init));
                } else if has_return && return_type_str != "void" {
                    code.push_str(&format!("let __ctx = {}(event: __e)\n", context_class));
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

                // Build params map
                if !method.params.is_empty() {
                    code.push_str("Dictionary<string, object> __params = new Dictionary<string, object>();\n");
                    for param in &method.params {
                        code.push_str(&format!("__params[\"{}\"] = {};\n", param.name, param.name));
                    }
                    code.push_str(&format!("{} __e = new {}(\"{}\", __params);\n", event_class, event_class, method.name));
                } else {
                    code.push_str(&format!("{} __e = new {}(\"{}\");\n", event_class, event_class, method.name));
                }

                // Create context with default return
                if let Some(ref init) = method.return_init {
                    code.push_str(&format!("{} __ctx = new {}(__e, {});\n", context_class, context_class, init));
                } else if has_return && return_type_str != "void" {
                    code.push_str(&format!("{} __ctx = new {}(__e, null);\n", context_class, context_class));
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

                // Build params map
                if !method.params.is_empty() {
                    code.push_str("__params := map[string]any{\n");
                    for param in &method.params {
                        code.push_str(&format!("    \"{}\": {},\n", param.name, param.name));
                    }
                    code.push_str("}\n");
                    code.push_str(&format!("__e := {}FrameEvent{{_message: \"{}\", _parameters: __params}}\n", system.name, method.name));
                } else {
                    code.push_str(&format!("__e := {}FrameEvent{{_message: \"{}\"}}\n", system.name, method.name));
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
                    "nil".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("[\"{}\"] = {}", p.name, p.name))
                        .collect();
                    format!("{{{}}}", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                if method.return_type.is_some() || method.return_init.is_some() {
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
                    "null".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\": {}", p.name, p.name))
                        .collect();
                    format!("{{{}}}", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {};", init_expr)
                } else {
                    String::new()
                };

                if method.return_type.is_some() || method.return_init.is_some() {
                    CodegenNode::NativeBlock {
                        code: format!(
                            "final __e = {}(\"{}\", {});\nfinal __ctx = {}(__e, null);{}\n_context_stack.add(__ctx);\n__kernel(__e);\nreturn _context_stack.removeLast()._return;",
                            event_class, method.name, params_code, context_class, default_init
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
                    "null".to_string()
                } else {
                    let param_items: Vec<String> = method.params.iter()
                        .map(|p| format!("\"{}\": {}", p.name, p.name))
                        .collect();
                    format!("{{{}}}", param_items.join(", "))
                };

                let default_init = if let Some(ref init_expr) = method.return_init {
                    format!("\n__ctx._return = {}", init_expr)
                } else {
                    String::new()
                };

                let has_return = method.return_type.is_some() || method.return_init.is_some();
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

        CodegenNode::Method {
            name: method.name.clone(),
            params,
            return_type: method.return_type.as_ref().map(|t| type_to_string(t)),
            body: vec![body_stmt],
            is_async: false,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        }
    }).collect()
}

/// Generate action method
///
/// Extracts native code from source using the body span
pub(crate) fn generate_action(action: &ActionAst, _syntax: &super::backend::ClassSyntax, source: &[u8]) -> CodegenNode {
    let params: Vec<Param> = action.params.iter().map(|p| {
        let type_str = type_to_string(&p.param_type);
        Param::new(&p.name).with_type(&type_str)
    }).collect();

    // Extract native code from source using span (oceans model)
    let code = extract_body_content(source, &action.body.span);

    CodegenNode::Method {
        name: action.name.clone(),
        params,
        return_type: None,  // Actions don't have explicit return types
        body: vec![CodegenNode::NativeBlock {
            code,
            span: Some(action.body.span.clone()),
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    }
}

/// Generate operation method
///
/// Extracts native code from source using the body span
pub(crate) fn generate_operation(operation: &OperationAst, _syntax: &super::backend::ClassSyntax, source: &[u8]) -> CodegenNode {
    let params: Vec<Param> = operation.params.iter().map(|p| {
        let type_str = type_to_string(&p.param_type);
        Param::new(&p.name).with_type(&type_str)
    }).collect();

    // Extract native code from source using span (oceans model)
    let code = extract_body_content(source, &operation.body.span);

    // Backend handles is_static flag for @staticmethod decorator
    CodegenNode::Method {
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
    }
}

/// Extract body content from source using span
///
/// Strips the outer braces and extracts the inner content while preserving
/// consistent line-by-line indentation for proper re-indentation by backends.
pub(crate) fn extract_body_content(source: &[u8], span: &crate::frame_c::compiler::frame_ast::Span) -> String {
    let bytes = &source[span.start..span.end];
    let content = String::from_utf8_lossy(bytes).to_string();

    // Strip outer braces if present
    let trimmed = content.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        // Extract content between braces
        let inner = &trimmed[1..trimmed.len()-1];

        // Split into lines, preserving structure
        let lines: Vec<&str> = inner.lines().collect();

        // Skip leading and trailing empty lines, but preserve internal structure
        let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
        let end = lines.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(lines.len());

        if start >= end {
            return String::new();
        }

        // Return lines with preserved indentation - let NativeBlock emitter normalize
        lines[start..end].join("\n")
    } else {
        trimmed.to_string()
    }
}

/// Generate persistence methods (save_state, restore_state) for @@persist
pub(crate) fn generate_persistence_methods(system: &SystemAst, syntax: &super::backend::ClassSyntax) -> Vec<CodegenNode> {
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
            save_body.push_str("        parent_compartment: serializeComp(c.parent_compartment),\n");
            save_body.push_str("    };\n");
            save_body.push_str("};\n");
            save_body.push_str("return JSON.stringify({\n");
            save_body.push_str("    _compartment: serializeComp(this.__compartment),\n");
            // Stack stores compartment objects - serialize each with its parent chain
            if is_ts {
                save_body.push_str("    _state_stack: this._state_stack.map((c: any) => serializeComp(c)),\n");
            } else {
                save_body.push_str("    _state_stack: this._state_stack.map((c) => serializeComp(c)),\n");
            }

            // Add domain variables
            for var in &system.domain {
                save_body.push_str(&format!("    {}: this.{},\n", var.name, var.name));
            }

            save_body.push_str("});\n");

            methods.push(CodegenNode::Method {
                name: "saveState".to_string(),
                params: vec![],
                return_type: Some("string".to_string()),  // Returns JSON string
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
                restore_body.push_str(&format!("const deserializeComp = (data: any): {}Compartment | null => {{\n", system.name));
            } else {
                restore_body.push_str("const deserializeComp = (data) => {\n");
            }
            restore_body.push_str("    if (!data) return null;\n");
            restore_body.push_str(&format!("    const comp = new {}Compartment(data.state);\n", system.name));
            restore_body.push_str("    comp.state_args = {...(data.state_args || {})};\n");
            restore_body.push_str("    comp.state_vars = {...(data.state_vars || {})};\n");
            restore_body.push_str("    comp.enter_args = {...(data.enter_args || {})};\n");
            restore_body.push_str("    comp.exit_args = {...(data.exit_args || {})};\n");
            restore_body.push_str("    comp.forward_event = data.forward_event;\n");
            restore_body.push_str("    comp.parent_compartment = deserializeComp(data.parent_compartment);\n");
            restore_body.push_str("    return comp;\n");
            restore_body.push_str("};\n");
            restore_body.push_str("const data = JSON.parse(json);\n");
            restore_body.push_str(&format!("const instance = Object.create({}.prototype);\n", system.name));
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

            // Restore domain variables
            for var in &system.domain {
                restore_body.push_str(&format!("instance.{} = data.{};\n", var.name, var.name));
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
            // Rust uses serde_json (requires serde_json in Cargo.toml)
            // HSM persistence: serialize entire compartment chain including parent_compartment
            // State vars live on compartment.state_context (StateContext enum)

            {
                // Generate save_state that recursively serializes compartment chain
                let mut save_body = String::new();

                // Helper: serialize state_context enum to JSON
                save_body.push_str(&format!("fn serialize_state_context(ctx: &{}StateContext) -> serde_json::Value {{\n", system.name));
                save_body.push_str("    match ctx {\n");
                if let Some(ref machine) = system.machine {
                    for state in &machine.states {
                        if state.state_vars.is_empty() {
                            save_body.push_str(&format!(
                                "        {}StateContext::{} => serde_json::json!({{}}),\n",
                                system.name, state.name
                            ));
                        } else {
                            save_body.push_str(&format!(
                                "        {}StateContext::{}(ctx) => serde_json::json!({{\n",
                                system.name, state.name
                            ));
                            for var in &state.state_vars {
                                save_body.push_str(&format!(
                                    "            \"{}\": ctx.{},\n",
                                    var.name, var.name
                                ));
                            }
                            save_body.push_str("        }),\n");
                        }
                    }
                }
                save_body.push_str(&format!("        {}StateContext::Empty => serde_json::json!({{}}),\n", system.name));
                save_body.push_str("    }\n");
                save_body.push_str("}\n");

                // Helper function to serialize a compartment and its parent chain
                save_body.push_str(&format!("fn serialize_comp(comp: &{}Compartment) -> serde_json::Value {{\n", system.name));
                save_body.push_str("    let parent = match &comp.parent_compartment {\n");
                save_body.push_str("        Some(p) => serialize_comp(p),\n");
                save_body.push_str("        None => serde_json::Value::Null,\n");
                save_body.push_str("    };\n");
                save_body.push_str("    serde_json::json!({\n");
                save_body.push_str("        \"state\": comp.state,\n");
                save_body.push_str("        \"state_context\": serialize_state_context(&comp.state_context),\n");
                save_body.push_str("        \"parent_compartment\": parent,\n");
                save_body.push_str("    })\n");
                save_body.push_str("}\n");

                save_body.push_str("let compartment_data = serialize_comp(&self.__compartment);\n");
                save_body.push_str("let stack_data: Vec<serde_json::Value> = self._state_stack.iter()\n");
                save_body.push_str("    .map(|comp| serialize_comp(comp))\n");
                save_body.push_str("    .collect();\n");
                save_body.push_str("serde_json::json!({\n");
                save_body.push_str("    \"_compartment\": compartment_data,\n");
                save_body.push_str("    \"_state_stack\": stack_data,\n");

                // Add domain variables
                for var in &system.domain {
                    save_body.push_str(&format!("    \"{}\": self.{},\n", var.name, var.name));
                }

                save_body.push_str("}).to_string()");

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

                // Generate restore_state that recursively deserializes compartment chain
                let mut restore_body = String::new();
                restore_body.push_str("let data: serde_json::Value = serde_json::from_str(json).unwrap();\n");

                // Helper: deserialize state_context from JSON based on state name
                restore_body.push_str(&format!("fn deserialize_state_context(state: &str, data: &serde_json::Value) -> {}StateContext {{\n", system.name));
                restore_body.push_str("    match state {\n");
                if let Some(ref machine) = system.machine {
                    for state in &machine.states {
                        if state.state_vars.is_empty() {
                            restore_body.push_str(&format!(
                                "        \"{}\" => {}StateContext::{},\n",
                                state.name, system.name, state.name
                            ));
                        } else {
                            restore_body.push_str(&format!(
                                "        \"{}\" => {}StateContext::{}({}Context {{\n",
                                state.name, system.name, state.name, state.name
                            ));
                            for var in &state.state_vars {
                                let json_extract = match &var.var_type {
                                    Type::Custom(name) => {
                                        match name.to_lowercase().as_str() {
                                            "int" | "i32" => format!("data[\"{}\"].as_i64().unwrap_or(0) as i32", var.name),
                                            "i64" => format!("data[\"{}\"].as_i64().unwrap_or(0)", var.name),
                                            "float" | "f32" | "f64" => format!("data[\"{}\"].as_f64().unwrap_or(0.0)", var.name),
                                            "bool" => format!("data[\"{}\"].as_bool().unwrap_or(false)", var.name),
                                            "str" | "string" => format!("data[\"{}\"].as_str().unwrap_or(\"\").to_string()", var.name),
                                            _ => format!("serde_json::from_value(data[\"{}\"].clone()).unwrap_or_default()", var.name),
                                        }
                                    }
                                    _ => format!("serde_json::from_value(data[\"{}\"].clone()).unwrap_or_default()", var.name),
                                };
                                restore_body.push_str(&format!("            {}: {},\n", var.name, json_extract));
                            }
                            restore_body.push_str("        }),\n");
                        }
                    }
                }
                restore_body.push_str(&format!("        _ => {}StateContext::Empty,\n", system.name));
                restore_body.push_str("    }\n");
                restore_body.push_str("}\n");

                // Helper function to deserialize a compartment and its parent chain
                restore_body.push_str(&format!("fn deserialize_comp(data: &serde_json::Value) -> {}Compartment {{\n", system.name));
                restore_body.push_str(&format!("    let state = data[\"state\"].as_str().unwrap();\n"));
                restore_body.push_str(&format!("    let mut comp = {}Compartment::new(state);\n", system.name));
                restore_body.push_str("    let ctx_data = &data[\"state_context\"];\n");
                restore_body.push_str("    if !ctx_data.is_null() {\n");
                restore_body.push_str(&format!("        comp.state_context = deserialize_state_context(state, ctx_data);\n"));
                restore_body.push_str("    }\n");
                restore_body.push_str("    if !data[\"parent_compartment\"].is_null() {\n");
                restore_body.push_str("        comp.parent_compartment = Some(Box::new(deserialize_comp(&data[\"parent_compartment\"])));\n");
                restore_body.push_str("    }\n");
                restore_body.push_str("    comp\n");
                restore_body.push_str("}\n");

                // Restore stack as Vec<Compartment>
                restore_body.push_str(&format!("let stack: Vec<{}Compartment> = data[\"_state_stack\"].as_array()\n", system.name));
                restore_body.push_str("    .map(|arr| arr.iter()\n");
                restore_body.push_str("        .map(|v| deserialize_comp(v))\n");
                restore_body.push_str("        .collect())\n");
                restore_body.push_str("    .unwrap_or_default();\n");

                // Deserialize compartment
                restore_body.push_str("let compartment = deserialize_comp(&data[\"_compartment\"]);\n");

                restore_body.push_str(&format!("let instance = {} {{\n", system.name));
                restore_body.push_str("    _state_stack: stack,\n");
                restore_body.push_str("    _context_stack: vec![],\n");
                restore_body.push_str("    __compartment: compartment,\n");
                restore_body.push_str("    __next_compartment: None,\n");

                // Restore domain variables
                for var in &system.domain {
                    let _type_str = type_to_string(&var.var_type);
                    let json_extract = match &var.var_type {
                        Type::Custom(name) => {
                            match name.to_lowercase().as_str() {
                                "int" | "i32" => format!("data[\"{}\"].as_i64().unwrap() as i32", var.name),
                                "i64" => format!("data[\"{}\"].as_i64().unwrap()", var.name),
                                "float" | "f32" | "f64" => format!("data[\"{}\"].as_f64().unwrap()", var.name),
                                "bool" => format!("data[\"{}\"].as_bool().unwrap()", var.name),
                                "str" | "string" => format!("data[\"{}\"].as_str().unwrap().to_string()", var.name),
                                _ => format!("serde_json::from_value(data[\"{}\"].clone()).unwrap()", var.name),
                            }
                        }
                        _ => format!("serde_json::from_value(data[\"{}\"].clone()).unwrap()", var.name),
                    };
                    restore_body.push_str(&format!("    {}: {},\n", var.name, json_extract));
                }

                restore_body.push_str("};\n");
                restore_body.push_str("instance");

                methods.push(CodegenNode::Method {
                    name: "restore_state".to_string(),
                    params: vec![Param::new("json").with_type("&str")],
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
        }
        TargetLanguage::C => {
            // C uses cJSON library (requires cJSON.h/cJSON.c or -lcjson)
            // HSM persistence: serialize entire compartment chain including parent_compartment

            // First, generate helper functions for compartment serialization/deserialization
            // These will be generated as static functions before save_state/restore_state

            // Generate serialize_compartment helper
            let mut serialize_helper = String::new();
            serialize_helper.push_str(&format!("static cJSON* {}_serialize_compartment({}_Compartment* comp) {{\n", system.name, system.name));
            serialize_helper.push_str("    if (!comp) return cJSON_CreateNull();\n");
            serialize_helper.push_str("    cJSON* obj = cJSON_CreateObject();\n");
            serialize_helper.push_str("    cJSON_AddStringToObject(obj, \"state\", comp->state);\n");
            // Serialize state_vars (iterate over bucket-based linked list)
            serialize_helper.push_str("    cJSON* vars = cJSON_CreateObject();\n");
            serialize_helper.push_str(&format!("    {}_FrameDict* sv = comp->state_vars;\n", system.name));
            serialize_helper.push_str("    if (sv) {\n");
            serialize_helper.push_str("        for (int i = 0; i < sv->bucket_count; i++) {\n");
            serialize_helper.push_str(&format!("            {}_FrameDictEntry* entry = sv->buckets[i];\n", system.name));
            serialize_helper.push_str("            while (entry) {\n");
            serialize_helper.push_str("                cJSON_AddNumberToObject(vars, entry->key, (double)(intptr_t)entry->value);\n");
            serialize_helper.push_str("                entry = entry->next;\n");
            serialize_helper.push_str("            }\n");
            serialize_helper.push_str("        }\n");
            serialize_helper.push_str("    }\n");
            serialize_helper.push_str("    cJSON_AddItemToObject(obj, \"state_vars\", vars);\n");
            // Recursively serialize parent
            serialize_helper.push_str(&format!("    cJSON_AddItemToObject(obj, \"parent_compartment\", {}_serialize_compartment(comp->parent_compartment));\n", system.name));
            serialize_helper.push_str("    return obj;\n");
            serialize_helper.push_str("}\n\n");

            // Generate deserialize_compartment helper
            let mut deserialize_helper = String::new();
            deserialize_helper.push_str(&format!("static {}_Compartment* {}_deserialize_compartment(cJSON* data) {{\n", system.name, system.name));
            deserialize_helper.push_str("    if (!data || cJSON_IsNull(data)) return NULL;\n");
            deserialize_helper.push_str("    cJSON* state_item = cJSON_GetObjectItem(data, \"state\");\n");
            // strdup the state string since cJSON memory will be freed
            deserialize_helper.push_str(&format!("    {}_Compartment* comp = {}_Compartment_new(strdup(state_item->valuestring));\n", system.name, system.name));
            // Deserialize state_vars
            deserialize_helper.push_str("    cJSON* vars = cJSON_GetObjectItem(data, \"state_vars\");\n");
            deserialize_helper.push_str("    if (vars) {\n");
            deserialize_helper.push_str("        cJSON* var_item;\n");
            deserialize_helper.push_str("        cJSON_ArrayForEach(var_item, vars) {\n");
            deserialize_helper.push_str(&format!("            {}_FrameDict_set(comp->state_vars, var_item->string, (void*)(intptr_t)(int)var_item->valuedouble);\n", system.name));
            deserialize_helper.push_str("        }\n");
            deserialize_helper.push_str("    }\n");
            // Recursively deserialize parent
            deserialize_helper.push_str("    cJSON* parent = cJSON_GetObjectItem(data, \"parent_compartment\");\n");
            deserialize_helper.push_str(&format!("    comp->parent_compartment = {}_deserialize_compartment(parent);\n", system.name));
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

            // Serialize state stack (simplified - just states for now)
            save_body.push_str("cJSON* stack_arr = cJSON_CreateArray();\n");
            save_body.push_str(&format!("for (int i = 0; i < {}_FrameVec_size(self->_state_stack); i++) {{\n", system.name));
            save_body.push_str(&format!("    {}_Compartment* comp = ({}_Compartment*){}_FrameVec_get(self->_state_stack, i);\n",
                system.name, system.name, system.name));
            save_body.push_str("    cJSON* stack_obj = cJSON_CreateObject();\n");
            save_body.push_str("    cJSON_AddStringToObject(stack_obj, \"state\", comp->state);\n");
            save_body.push_str("    cJSON_AddItemToArray(stack_arr, stack_obj);\n");
            save_body.push_str("}\n");
            save_body.push_str("cJSON_AddItemToObject(root, \"_state_stack\", stack_arr);\n");

            // Serialize domain variables
            for var in &system.domain {
                let type_str = extract_type_from_raw_domain(&var.raw_code, &var.name);

                let json_add = if is_int_type(&type_str) {
                    format!("cJSON_AddNumberToObject(root, \"{}\", (double)self->{});\n", var.name, var.name)
                } else if is_float_type(&type_str) {
                    format!("cJSON_AddNumberToObject(root, \"{}\", self->{});\n", var.name, var.name)
                } else if is_bool_type(&type_str) {
                    format!("cJSON_AddBoolToObject(root, \"{}\", self->{});\n", var.name, var.name)
                } else if is_string_type(&type_str) {
                    format!("cJSON_AddStringToObject(root, \"{}\", self->{});\n", var.name, var.name)
                } else {
                    format!("cJSON_AddNumberToObject(root, \"{}\", (double)(intptr_t)self->{});\n", var.name, var.name)
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

            restore_body.push_str(&format!("{}* instance = malloc(sizeof({}));\n", system.name, system.name));
            restore_body.push_str(&format!("instance->_state_stack = {}_FrameVec_new();\n", system.name));
            restore_body.push_str(&format!("instance->_context_stack = {}_FrameVec_new();\n", system.name));
            restore_body.push_str("instance->__next_compartment = NULL;\n\n");

            // Restore entire compartment chain
            restore_body.push_str("cJSON* comp_data = cJSON_GetObjectItem(root, \"_compartment\");\n");
            restore_body.push_str(&format!("instance->__compartment = {}_deserialize_compartment(comp_data);\n\n", system.name));

            // Restore state stack
            restore_body.push_str("cJSON* stack_arr = cJSON_GetObjectItem(root, \"_state_stack\");\n");
            restore_body.push_str("if (stack_arr) {\n");
            restore_body.push_str("    cJSON* stack_item;\n");
            restore_body.push_str("    cJSON_ArrayForEach(stack_item, stack_arr) {\n");
            restore_body.push_str("        cJSON* state_obj = cJSON_GetObjectItem(stack_item, \"state\");\n");
            restore_body.push_str(&format!("        {}_Compartment* comp = {}_Compartment_new(strdup(state_obj->valuestring));\n",
                system.name, system.name));
            restore_body.push_str(&format!("        {}_FrameVec_push(instance->_state_stack, comp);\n", system.name));
            restore_body.push_str("    }\n");
            restore_body.push_str("}\n\n");

            // Restore domain variables
            for var in &system.domain {
                let type_str = extract_type_from_raw_domain(&var.raw_code, &var.name);

                let json_get = if is_int_type(&type_str) {
                    format!("instance->{} = (int)cJSON_GetObjectItem(root, \"{}\")->valuedouble;\n", var.name, var.name)
                } else if is_float_type(&type_str) {
                    format!("instance->{} = cJSON_GetObjectItem(root, \"{}\")->valuedouble;\n", var.name, var.name)
                } else if is_bool_type(&type_str) {
                    format!("instance->{} = cJSON_IsTrue(cJSON_GetObjectItem(root, \"{}\"));\n", var.name, var.name)
                } else if is_string_type(&type_str) {
                    format!("instance->{} = strdup(cJSON_GetObjectItem(root, \"{}\")->valuestring);\n", var.name, var.name)
                } else {
                    format!("instance->{} = (int)cJSON_GetObjectItem(root, \"{}\")->valuedouble;\n", var.name, var.name)
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
            let all_state_vars: Vec<(&str, &str, &str)> = system.machine.as_ref()
                .map(|m| m.states.iter().flat_map(|s| {
                    s.state_vars.iter().map(move |sv| {
                        let type_str = match &sv.var_type {
                            crate::frame_c::compiler::frame_ast::Type::Custom(t) => t.as_str(),
                            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
                        };
                        (s.name.as_str(), sv.name.as_str(), type_str)
                    })
                }).collect())
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
            save_body.push_str("    __cj[\"parent\"] = __ser(c->parent_compartment.get());\n");
            save_body.push_str("    return __cj;\n");
            save_body.push_str("};\n");

            save_body.push_str("nlohmann::json __j;\n");
            save_body.push_str("__j[\"_compartment\"] = __ser(__compartment.get());\n");

            // Serialize state stack
            save_body.push_str("nlohmann::json __stack = nlohmann::json::array();\n");
            save_body.push_str("for (auto& c : _state_stack) { __stack.push_back(__ser(c.get())); }\n");
            save_body.push_str("__j[\"_state_stack\"] = __stack;\n");

            // Serialize domain vars
            for var in &system.domain {
                save_body.push_str(&format!("__j[\"{}\"] = {};\n", var.name, var.name));
            }

            save_body.push_str("return __j.dump();");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: Some("std::string".to_string()),
                body: vec![CodegenNode::NativeBlock { code: save_body, span: None }],
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
            restore_body.push_str("    if (d.contains(\"parent\") && !d[\"parent\"].is_null()) {\n");
            restore_body.push_str("        c->parent_compartment = __deser(d[\"parent\"]);\n");
            restore_body.push_str("    }\n");
            restore_body.push_str("    return c;\n");
            restore_body.push_str("};\n");

            restore_body.push_str("auto __j = nlohmann::json::parse(json);\n");
            restore_body.push_str(&format!("{} __instance;\n", sys));
            restore_body.push_str("__instance.__compartment = __deser(__j[\"_compartment\"]);\n");

            // Restore state stack
            restore_body.push_str("if (__j.contains(\"_state_stack\")) {\n");
            restore_body.push_str("    for (auto& __sc : __j[\"_state_stack\"]) {\n");
            restore_body.push_str("        __instance._state_stack.push_back(__deser(__sc));\n");
            restore_body.push_str("    }\n");
            restore_body.push_str("}\n");

            // Restore domain vars
            for var in &system.domain {
                restore_body.push_str(&format!(
                    "if (__j.contains(\"{0}\")) {{ __j[\"{0}\"].get_to(__instance.{0}); }}\n",
                    var.name
                ));
            }

            restore_body.push_str("return __instance;");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json").with_type("const std::string&")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock { code: restore_body, span: None }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Java => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Collect state vars with types
            let all_state_vars: Vec<(&str, &str, &str)> = system.machine.as_ref()
                .map(|m| m.states.iter().flat_map(|s| {
                    s.state_vars.iter().map(move |sv| {
                        let type_str = match &sv.var_type {
                            crate::frame_c::compiler::frame_ast::Type::Custom(t) => t.as_str(),
                            crate::frame_c::compiler::frame_ast::Type::Unknown => "int",
                        };
                        (s.name.as_str(), sv.name.as_str(), type_str)
                    })
                }).collect())
                .unwrap_or_default();

            // Private helper method for recursive compartment serialization
            let mut ser_body = String::new();
            ser_body.push_str(&format!("if (comp == null) return null;\n"));
            ser_body.push_str("var j = new org.json.JSONObject();\n");
            ser_body.push_str("j.put(\"state\", comp.state);\n");
            ser_body.push_str("var sv = new org.json.JSONObject();\n");
            ser_body.push_str("for (var e : comp.state_vars.entrySet()) { sv.put(e.getKey(), e.getValue()); }\n");
            ser_body.push_str("j.put(\"state_vars\", sv);\n");
            ser_body.push_str("j.put(\"parent\", __serComp(comp.parent_compartment));\n");
            ser_body.push_str("return j;");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp").with_type(&compartment_class)],
                return_type: Some("org.json.JSONObject".to_string()),
                body: vec![CodegenNode::NativeBlock { code: ser_body, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // Private helper for deserialization
            let mut deser_body = String::new();
            deser_body.push_str("if (obj == null || obj.equals(org.json.JSONObject.NULL)) return null;\n");
            deser_body.push_str("var d = (org.json.JSONObject) obj;\n");
            deser_body.push_str(&format!("var c = new {}(d.getString(\"state\"));\n", compartment_class));
            deser_body.push_str("if (d.has(\"state_vars\")) {\n");
            deser_body.push_str("    var sv = d.getJSONObject(\"state_vars\");\n");
            deser_body.push_str("    for (var k : sv.keySet()) { c.state_vars.put(k, sv.get(k)); }\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if (d.has(\"parent\") && !d.isNull(\"parent\")) {\n");
            deser_body.push_str("    c.parent_compartment = __deserComp(d.get(\"parent\"));\n");
            deser_body.push_str("}\n");
            deser_body.push_str("return c;");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![Param::new("obj").with_type("Object")],
                return_type: Some(compartment_class.clone()),
                body: vec![CodegenNode::NativeBlock { code: deser_body, span: None }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("var __j = new org.json.JSONObject();\n");
            save_body.push_str("__j.put(\"_compartment\", __serComp(__compartment));\n");
            save_body.push_str("var __stack = new org.json.JSONArray();\n");
            save_body.push_str("for (var c : _state_stack) { __stack.put(__serComp(c)); }\n");
            save_body.push_str("__j.put(\"_state_stack\", __stack);\n");

            for var in &system.domain {
                save_body.push_str(&format!("__j.put(\"{}\", {});\n", var.name, var.name));
            }

            save_body.push_str("return __j.toString();");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: Some("String".to_string()),
                body: vec![CodegenNode::NativeBlock { code: save_body, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restore_state(json) — static method, uses static __deserComp
            let mut restore_body = String::new();
            restore_body.push_str("var __j = new org.json.JSONObject(json);\n");
            restore_body.push_str(&format!("var __instance = new {}();\n", sys));
            restore_body.push_str("__instance.__compartment = __deserComp(__j.get(\"_compartment\"));\n");
            restore_body.push_str("if (__j.has(\"_state_stack\")) {\n");
            restore_body.push_str("    var __stack = __j.getJSONArray(\"_state_stack\");\n");
            restore_body.push_str("    __instance._state_stack = new ArrayList<>();\n");
            restore_body.push_str("    for (int i = 0; i < __stack.length(); i++) { __instance._state_stack.add(__deserComp(__stack.get(i))); }\n");
            restore_body.push_str("}\n");

            // Restore domain vars — detect type from raw_code if available
            for var in &system.domain {
                // Try to determine Java type from raw_code (e.g., "int x = 0" → "int")
                let java_type = if let Some(ref raw) = var.raw_code {
                    let trimmed = raw.trim();
                    if trimmed.starts_with("int ") { "int" }
                    else if trimmed.starts_with("double ") || trimmed.starts_with("float ") { "double" }
                    else if trimmed.starts_with("boolean ") { "boolean" }
                    else if trimmed.starts_with("String ") { "String" }
                    else if trimmed.starts_with("long ") { "long" }
                    else { "Object" }
                } else {
                    match &var.var_type {
                        crate::frame_c::compiler::frame_ast::Type::Custom(t) => java_map_type(t).leak(),
                        _ => "Object",
                    }
                };
                match java_type {
                    "int" => restore_body.push_str(&format!("if (__j.has(\"{0}\")) {{ __instance.{0} = __j.getInt(\"{0}\"); }}\n", var.name)),
                    "double" | "float" => restore_body.push_str(&format!("if (__j.has(\"{0}\")) {{ __instance.{0} = __j.getDouble(\"{0}\"); }}\n", var.name)),
                    "boolean" => restore_body.push_str(&format!("if (__j.has(\"{0}\")) {{ __instance.{0} = __j.getBoolean(\"{0}\"); }}\n", var.name)),
                    "String" => restore_body.push_str(&format!("if (__j.has(\"{0}\")) {{ __instance.{0} = __j.getString(\"{0}\"); }}\n", var.name)),
                    _ => restore_body.push_str(&format!("if (__j.has(\"{0}\")) {{ __instance.{0} = __j.get(\"{0}\"); }}\n", var.name)),
                }
            }

            restore_body.push_str("return __instance;");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json").with_type("String")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock { code: restore_body, span: None }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::CSharp => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Private helper method for recursive compartment serialization
            let mut ser_body = String::new();
            ser_body.push_str("if (comp == null) return null;\n");
            ser_body.push_str("var j = new Dictionary<string, object>();\n");
            ser_body.push_str("j[\"state\"] = comp.state;\n");
            ser_body.push_str("var sv = new Dictionary<string, object>(comp.state_vars);\n");
            ser_body.push_str("j[\"state_vars\"] = sv;\n");
            ser_body.push_str("j[\"parent\"] = __SerComp(comp.parent_compartment);\n");
            ser_body.push_str("return j;");

            methods.push(CodegenNode::Method {
                name: "__SerComp".to_string(),
                params: vec![Param::new("comp").with_type(&compartment_class)],
                return_type: Some("object".to_string()),
                body: vec![CodegenNode::NativeBlock { code: ser_body, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // Private helper for deserialization — uses JsonElement
            let mut deser_body = String::new();
            deser_body.push_str("if (el.ValueKind == System.Text.Json.JsonValueKind.Null) return null;\n");
            deser_body.push_str(&format!("var c = new {}(el.GetProperty(\"state\").GetString());\n", compartment_class));
            deser_body.push_str("if (el.TryGetProperty(\"state_vars\", out var sv) && sv.ValueKind == System.Text.Json.JsonValueKind.Object) {\n");
            deser_body.push_str("    foreach (var kv in sv.EnumerateObject()) {\n");
            deser_body.push_str("        if (kv.Value.ValueKind == System.Text.Json.JsonValueKind.Number) c.state_vars[kv.Name] = kv.Value.GetInt32();\n");
            deser_body.push_str("        else if (kv.Value.ValueKind == System.Text.Json.JsonValueKind.String) c.state_vars[kv.Name] = kv.Value.GetString();\n");
            deser_body.push_str("        else c.state_vars[kv.Name] = kv.Value.ToString();\n");
            deser_body.push_str("    }\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if (el.TryGetProperty(\"parent\", out var p) && p.ValueKind != System.Text.Json.JsonValueKind.Null) {\n");
            deser_body.push_str("    c.parent_compartment = __DeserComp(p);\n");
            deser_body.push_str("}\n");
            deser_body.push_str("return c;");

            methods.push(CodegenNode::Method {
                name: "__DeserComp".to_string(),
                params: vec![Param::new("el").with_type("System.Text.Json.JsonElement")],
                return_type: Some(compartment_class.clone()),
                body: vec![CodegenNode::NativeBlock { code: deser_body, span: None }],
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
                save_body.push_str(&format!("__j[\"{}\"] = {};\n", var.name, var.name));
            }

            save_body.push_str("var __opts = new System.Text.Json.JsonSerializerOptions { TypeInfoResolver = new System.Text.Json.Serialization.Metadata.DefaultJsonTypeInfoResolver() };\n");
            save_body.push_str("return System.Text.Json.JsonSerializer.Serialize(__j, __opts);");

            methods.push(CodegenNode::Method {
                name: "SaveState".to_string(),
                params: vec![],
                return_type: Some("string".to_string()),
                body: vec![CodegenNode::NativeBlock { code: save_body, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // RestoreState(json) — static method
            let mut restore_body = String::new();
            restore_body.push_str("var __doc = System.Text.Json.JsonDocument.Parse(json);\n");
            restore_body.push_str("var __root = __doc.RootElement;\n");
            restore_body.push_str(&format!("var __instance = new {}();\n", sys));
            restore_body.push_str("__instance.__compartment = __DeserComp(__root.GetProperty(\"_compartment\"));\n");
            restore_body.push_str("if (__root.TryGetProperty(\"_state_stack\", out var __stack)) {\n");
            restore_body.push_str(&format!("    __instance._state_stack = new List<{}>();\n", compartment_class));
            restore_body.push_str("    foreach (var item in __stack.EnumerateArray()) { __instance._state_stack.Add(__DeserComp(item)); }\n");
            restore_body.push_str("}\n");

            // Restore domain vars via JsonElement
            for var in &system.domain {
                let cs_type = if let Some(ref raw) = var.raw_code {
                    let trimmed = raw.trim();
                    if trimmed.starts_with("int ") { "int" }
                    else if trimmed.starts_with("double ") || trimmed.starts_with("float ") { "double" }
                    else if trimmed.starts_with("bool ") { "bool" }
                    else if trimmed.starts_with("string ") || trimmed.starts_with("String ") { "string" }
                    else { "object" }
                } else { "object" };
                match cs_type {
                    "int" => restore_body.push_str(&format!(
                        "if (__root.TryGetProperty(\"{0}\", out var __{0})) {{ __instance.{0} = __{0}.GetInt32(); }}\n", var.name)),
                    "double" => restore_body.push_str(&format!(
                        "if (__root.TryGetProperty(\"{0}\", out var __{0})) {{ __instance.{0} = __{0}.GetDouble(); }}\n", var.name)),
                    "bool" => restore_body.push_str(&format!(
                        "if (__root.TryGetProperty(\"{0}\", out var __{0})) {{ __instance.{0} = __{0}.GetBoolean(); }}\n", var.name)),
                    "string" => restore_body.push_str(&format!(
                        "if (__root.TryGetProperty(\"{0}\", out var __{0})) {{ __instance.{0} = __{0}.GetString(); }}\n", var.name)),
                    _ => restore_body.push_str(&format!(
                        "if (__root.TryGetProperty(\"{0}\", out var __{0})) {{ __instance.{0} = __{0}.ToString(); }}\n", var.name)),
                }
            }

            restore_body.push_str("return __instance;");

            methods.push(CodegenNode::Method {
                name: "RestoreState".to_string(),
                params: vec![Param::new("json").with_type("string")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock { code: restore_body, span: None }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Php => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Private helper for recursive compartment serialization
            let mut ser_body = String::new();
            ser_body.push_str("if ($comp === null) return null;\n");
            ser_body.push_str("$j = ['state' => $comp->state, 'state_vars' => $comp->state_vars];\n");
            ser_body.push_str("$j['parent'] = $this->__serComp($comp->parent_compartment);\n");
            ser_body.push_str("return $j;");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: ser_body, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Private,
                decorators: vec![],
            });

            // Private helper for deserialization
            let mut deser_body = String::new();
            deser_body.push_str("if ($data === null) return null;\n");
            deser_body.push_str(&format!("$c = new {}($data['state']);\n", compartment_class));
            deser_body.push_str("if (isset($data['state_vars'])) $c->state_vars = $data['state_vars'];\n");
            deser_body.push_str("if (isset($data['parent'])) $c->parent_compartment = self::__deserComp($data['parent']);\n");
            deser_body.push_str("return $c;");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![Param::new("data")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: deser_body, span: None }],
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
            save_body.push_str("foreach ($this->_state_stack as $c) { $stack[] = $this->__serComp($c); }\n");
            save_body.push_str("$j['_state_stack'] = $stack;\n");
            for var in &system.domain {
                save_body.push_str(&format!("$j['{}'] = $this->{};\n", var.name, var.name));
            }
            save_body.push_str("return json_encode($j);");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: save_body, span: None }],
                is_async: false,
                is_static: false,
                visibility: Visibility::Public,
                decorators: vec![],
            });

            // restore_state($json) — static
            let mut restore_body = String::new();
            restore_body.push_str("$j = json_decode($json, true);\n");
            restore_body.push_str(&format!("$instance = new {}();\n", sys));
            restore_body.push_str("$instance->__compartment = self::__deserComp($j['_compartment']);\n");
            restore_body.push_str("if (isset($j['_state_stack'])) {\n");
            restore_body.push_str("    $instance->_state_stack = [];\n");
            restore_body.push_str("    foreach ($j['_state_stack'] as $sc) { $instance->_state_stack[] = self::__deserComp($sc); }\n");
            restore_body.push_str("}\n");
            for var in &system.domain {
                restore_body.push_str(&format!("if (isset($j['{}'])) $instance->{} = $j['{}'];\n", var.name, var.name, var.name));
            }
            restore_body.push_str("return $instance;");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: restore_body, span: None }],
                is_async: false,
                is_static: true,
                visibility: Visibility::Public,
                decorators: vec![],
            });
        }
        TargetLanguage::Kotlin => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Kotlin can use Java's org.json on JVM — same pattern as Java
            // Private helpers as class methods
            let mut ser_body = String::new();
            ser_body.push_str("if (comp == null) return null\n");
            ser_body.push_str("val j = org.json.JSONObject()\n");
            ser_body.push_str("j.put(\"state\", comp.state)\n");
            ser_body.push_str("val sv = org.json.JSONObject()\n");
            ser_body.push_str("for ((k, v) in comp.state_vars) { sv.put(k, v) }\n");
            ser_body.push_str("j.put(\"state_vars\", sv)\n");
            ser_body.push_str("j.put(\"parent\", __serComp(comp.parent_compartment))\n");
            ser_body.push_str("return j");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp").with_type(&format!("{}?", compartment_class))],
                return_type: Some("org.json.JSONObject?".to_string()),
                body: vec![CodegenNode::NativeBlock { code: ser_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![],
            });

            let mut deser_body = String::new();
            deser_body.push_str("if (obj == null || obj == org.json.JSONObject.NULL) return null\n");
            deser_body.push_str("val d = obj as org.json.JSONObject\n");
            deser_body.push_str(&format!("val c = {}(d.getString(\"state\"))\n", compartment_class));
            deser_body.push_str("if (d.has(\"state_vars\")) {\n");
            deser_body.push_str("    val sv = d.getJSONObject(\"state_vars\")\n");
            deser_body.push_str("    for (k in sv.keys()) { c.state_vars[k] = sv.get(k) }\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if (d.has(\"parent\") && !d.isNull(\"parent\")) {\n");
            deser_body.push_str("    c.parent_compartment = __deserComp(d.get(\"parent\"))\n");
            deser_body.push_str("}\n");
            deser_body.push_str("return c");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![Param::new("obj").with_type("Any?")],
                return_type: Some(format!("{}?", compartment_class)),
                body: vec![CodegenNode::NativeBlock { code: deser_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("val j = org.json.JSONObject()\n");
            save_body.push_str("j.put(\"_compartment\", __serComp(__compartment))\n");
            save_body.push_str("val stack = org.json.JSONArray()\n");
            save_body.push_str("for (c in _state_stack) { stack.put(__serComp(c)) }\n");
            save_body.push_str("j.put(\"_state_stack\", stack)\n");
            for var in &system.domain {
                save_body.push_str(&format!("j.put(\"{}\", {})\n", var.name, var.name));
            }
            save_body.push_str("return j.toString()");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![], return_type: Some("String".to_string()),
                body: vec![CodegenNode::NativeBlock { code: save_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Public, decorators: vec![],
            });

            // restore_state — companion object static method
            // For Kotlin, static methods go in companion object, but for simplicity
            // emit as a top-level function or use companion object in the class
            // Actually, emit as a regular method and the test will call it on an instance
            // OR use companion object pattern
            let mut restore_body = String::new();
            restore_body.push_str("val j = org.json.JSONObject(json)\n");
            restore_body.push_str(&format!("val instance = {}()\n", sys));
            restore_body.push_str("instance.__compartment = instance.__deserComp(j.get(\"_compartment\"))!!\n");
            restore_body.push_str("if (j.has(\"_state_stack\")) {\n");
            restore_body.push_str("    val stack = j.getJSONArray(\"_state_stack\")\n");
            restore_body.push_str("    instance._state_stack = mutableListOf()\n");
            restore_body.push_str(&format!("    for (i in 0 until stack.length()) {{ instance._state_stack.add(instance.__deserComp(stack.get(i))!!) }}\n"));
            restore_body.push_str("}\n");
            for var in &system.domain {
                let kt_type = if let Some(ref raw) = var.raw_code {
                    let t = raw.trim();
                    // Check for numeric integer initialization (= 0, = 42, = -1, etc.)
                    let is_int = t.contains("Int") || {
                        if let Some(eq_pos) = t.find('=') {
                            let val = t[eq_pos+1..].trim();
                            val.parse::<i64>().is_ok()
                        } else {
                            false
                        }
                    };
                    if is_int { "Int" }
                    else if t.contains("String") || t.contains("= \"") { "String" }
                    else if t.contains("Boolean") || t.contains("= true") || t.contains("= false") { "Boolean" }
                    else if t.contains("Double") || {
                        if let Some(eq_pos) = t.find('=') {
                            let val = t[eq_pos+1..].trim();
                            val.contains('.') && val.parse::<f64>().is_ok()
                        } else {
                            false
                        }
                    } { "Double" }
                    else { "Any" }
                } else { "Any" };
                match kt_type {
                    "Int" => restore_body.push_str(&format!("if (j.has(\"{0}\")) instance.{0} = j.getInt(\"{0}\")\n", var.name)),
                    "String" => restore_body.push_str(&format!("if (j.has(\"{0}\")) instance.{0} = j.getString(\"{0}\")\n", var.name)),
                    "Boolean" => restore_body.push_str(&format!("if (j.has(\"{0}\")) instance.{0} = j.getBoolean(\"{0}\")\n", var.name)),
                    "Double" => restore_body.push_str(&format!("if (j.has(\"{0}\")) instance.{0} = j.getDouble(\"{0}\")\n", var.name)),
                    _ => restore_body.push_str(&format!("if (j.has(\"{0}\")) instance.{0} = j.get(\"{0}\")\n", var.name)),
                }
            }
            restore_body.push_str("return instance");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json").with_type("String")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock { code: restore_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Public, decorators: vec![],
            });
        }
        TargetLanguage::Swift => {
            let sys = &system.name;
            let compartment_class = format!("{}Compartment", sys);

            // Swift uses Foundation JSONSerialization — dict-based serialization
            // Private helpers as class methods
            let mut ser_body = String::new();
            ser_body.push_str("if comp == nil { return nil }\n");
            ser_body.push_str("var j: [String: Any] = [:]\n");
            ser_body.push_str("j[\"state\"] = comp!.state\n");
            ser_body.push_str("var sv: [String: Any] = [:]\n");
            ser_body.push_str("for (k, v) in comp!.state_vars { sv[k] = v }\n");
            ser_body.push_str("j[\"state_vars\"] = sv\n");
            ser_body.push_str("j[\"parent\"] = __serComp(comp!.parent_compartment) as Any\n");
            ser_body.push_str("return j");

            methods.push(CodegenNode::Method {
                name: "__serComp".to_string(),
                params: vec![Param::new("comp").with_type(&format!("{}?", compartment_class))],
                return_type: Some("[String: Any]?".to_string()),
                body: vec![CodegenNode::NativeBlock { code: ser_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![],
            });

            let mut deser_body = String::new();
            deser_body.push_str("guard let d = dict else { return nil }\n");
            deser_body.push_str("guard let state = d[\"state\"] as? String else { return nil }\n");
            deser_body.push_str(&format!("let c = {}(state: state)\n", compartment_class));
            deser_body.push_str("if let sv = d[\"state_vars\"] as? [String: Any] {\n");
            deser_body.push_str("    for (k, v) in sv { c.state_vars[k] = v }\n");
            deser_body.push_str("}\n");
            deser_body.push_str("if let parent = d[\"parent\"] as? [String: Any] {\n");
            deser_body.push_str("    c.parent_compartment = __deserComp(parent)\n");
            deser_body.push_str("}\n");
            deser_body.push_str("return c");

            methods.push(CodegenNode::Method {
                name: "__deserComp".to_string(),
                params: vec![Param::new("dict").with_type("[String: Any]?")],
                return_type: Some(format!("{}?", compartment_class)),
                body: vec![CodegenNode::NativeBlock { code: deser_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("var j: [String: Any] = [:]\n");
            save_body.push_str("j[\"_compartment\"] = __serComp(__compartment) as Any\n");
            save_body.push_str("var stack: [[String: Any]] = []\n");
            save_body.push_str("for c in _state_stack { if let s = __serComp(c) { stack.append(s) } }\n");
            save_body.push_str("j[\"_state_stack\"] = stack\n");
            for var in &system.domain {
                save_body.push_str(&format!("j[\"{}\"] = {}\n", var.name, var.name));
            }
            save_body.push_str("let data = try! JSONSerialization.data(withJSONObject: j)\n");
            save_body.push_str("return String(data: data, encoding: .utf8)!");

            methods.push(CodegenNode::Method {
                name: "saveState".to_string(),
                params: vec![], return_type: Some("String".to_string()),
                body: vec![CodegenNode::NativeBlock { code: save_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Public, decorators: vec![],
            });

            // restoreState — static method
            let mut restore_body = String::new();
            restore_body.push_str("let data = json.data(using: .utf8)!\n");
            restore_body.push_str("let j = try! JSONSerialization.jsonObject(with: data) as! [String: Any]\n");
            restore_body.push_str(&format!("let instance = {}()\n", sys));
            restore_body.push_str("instance.__compartment = instance.__deserComp(j[\"_compartment\"] as? [String: Any])!\n");
            restore_body.push_str("if let stack = j[\"_state_stack\"] as? [[String: Any]] {\n");
            restore_body.push_str("    instance._state_stack = []\n");
            restore_body.push_str("    for sc in stack { if let c = instance.__deserComp(sc) { instance._state_stack.append(c) } }\n");
            restore_body.push_str("}\n");
            for var in &system.domain {
                let swift_type = if let Some(ref raw) = var.raw_code {
                    let t = raw.trim();
                    // Check for array types first (e.g. string[], number[], Int[], String[])
                    let is_array = t.contains("[]") || t.contains("= []");
                    if is_array {
                        // Determine element type
                        if t.contains("string[]") || t.contains("String[]") { "[String]" }
                        else if t.contains("number[]") || t.contains("Int[]") { "[Int]" }
                        else if t.contains("bool[]") || t.contains("Bool[]") { "[Bool]" }
                        else if t.contains("Double[]") || t.contains("float[]") { "[Double]" }
                        else { "[Any]" }
                    } else {
                        let is_int = t.contains("Int") || t.contains("number") || {
                            if let Some(eq_pos) = t.find('=') {
                                let val = t[eq_pos+1..].trim();
                                val.parse::<i64>().is_ok()
                            } else {
                                false
                            }
                        };
                        if is_int { "Int" }
                        else if t.contains("String") || t.contains("string") || t.contains("= \"") { "String" }
                        else if t.contains("Bool") || t.contains("bool") || t.contains("= true") || t.contains("= false") { "Bool" }
                        else if t.contains("Double") || t.contains("float") || t.contains("double") || {
                            if let Some(eq_pos) = t.find('=') {
                                let val = t[eq_pos+1..].trim();
                                val.contains('.') && val.parse::<f64>().is_ok()
                            } else {
                                false
                            }
                        } { "Double" }
                        else { "Any" }
                    }
                } else { "Any" };
                match swift_type {
                    "Int" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? Int {{ instance.{0} = v }}\n", var.name)),
                    "String" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? String {{ instance.{0} = v }}\n", var.name)),
                    "Bool" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? Bool {{ instance.{0} = v }}\n", var.name)),
                    "Double" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? Double {{ instance.{0} = v }}\n", var.name)),
                    "[String]" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? [String] {{ instance.{0} = v }}\n", var.name)),
                    "[Int]" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? [Int] {{ instance.{0} = v }}\n", var.name)),
                    "[Bool]" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? [Bool] {{ instance.{0} = v }}\n", var.name)),
                    "[Double]" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? [Double] {{ instance.{0} = v }}\n", var.name)),
                    "[Any]" => restore_body.push_str(&format!("if let v = j[\"{0}\"] as? [Any] {{ instance.{0} = v }}\n", var.name)),
                    _ => restore_body.push_str(&format!("if let v = j[\"{0}\"] {{ instance.{0} = v }}\n", var.name)),
                }
            }
            restore_body.push_str("return instance");

            methods.push(CodegenNode::Method {
                name: "restoreState".to_string(),
                params: vec![Param::new("json").with_type("String")],
                return_type: Some(sys.clone()),
                body: vec![CodegenNode::NativeBlock { code: restore_body, span: None }],
                is_async: false, is_static: true, visibility: Visibility::Public, decorators: vec![],
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
            ser_body.push_str("j[\"parent\"] = __ser_comp(comp.parent_compartment)\n");
            ser_body.push_str("j");

            methods.push(CodegenNode::Method {
                name: "__ser_comp".to_string(),
                params: vec![Param::new("comp")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: ser_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![],
            });

            // Private helper: deserialize compartment chain
            let mut deser_body = String::new();
            deser_body.push_str("return nil if data.nil?\n");
            deser_body.push_str(&format!("c = {}.new(data[\"state\"])\n", compartment_class));
            deser_body.push_str("if data[\"state_vars\"]\n");
            deser_body.push_str("  data[\"state_vars\"].each { |k, v| c.state_vars[k] = v }\n");
            deser_body.push_str("end\n");
            deser_body.push_str("if data[\"parent\"]\n");
            deser_body.push_str("  c.parent_compartment = __deser_comp(data[\"parent\"])\n");
            deser_body.push_str("end\n");
            deser_body.push_str("c");

            methods.push(CodegenNode::Method {
                name: "__deser_comp".to_string(),
                params: vec![Param::new("data")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: deser_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![],
            });

            // save_state()
            let mut save_body = String::new();
            save_body.push_str("j = {}\n");
            save_body.push_str("j[\"_compartment\"] = __ser_comp(@__compartment)\n");
            save_body.push_str("stack = []\n");
            save_body.push_str("@_state_stack.each { |c| stack.push(__ser_comp(c)) }\n");
            save_body.push_str("j[\"_state_stack\"] = stack\n");
            for var in &system.domain {
                save_body.push_str(&format!("j[\"{}\"] = @{}\n", var.name, var.name));
            }
            save_body.push_str("JSON.generate(j)");

            methods.push(CodegenNode::Method {
                name: "save_state".to_string(),
                params: vec![],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: save_body, span: None }],
                is_async: false, is_static: false, visibility: Visibility::Public, decorators: vec![],
            });

            // restore_state(json) — class method (static)
            let mut restore_body = String::new();
            restore_body.push_str("j = JSON.parse(json)\n");
            restore_body.push_str(&format!("instance = {}.new\n", sys));
            restore_body.push_str("instance.instance_variable_set(:@__compartment, instance.send(:__deser_comp, j[\"_compartment\"]))\n");
            restore_body.push_str("if j[\"_state_stack\"]\n");
            restore_body.push_str("  instance.instance_variable_set(:@_state_stack, j[\"_state_stack\"].map { |sc| instance.send(:__deser_comp, sc) })\n");
            restore_body.push_str("end\n");
            for var in &system.domain {
                restore_body.push_str(&format!("instance.{} = j[\"{}\"] if j.key?(\"{}\")\n", var.name, var.name, var.name));
            }
            restore_body.push_str("instance");

            methods.push(CodegenNode::Method {
                name: "restore_state".to_string(),
                params: vec![Param::new("json")],
                return_type: None,
                body: vec![CodegenNode::NativeBlock { code: restore_body, span: None }],
                is_async: false, is_static: true, visibility: Visibility::Public, decorators: vec![],
            });
        }
        TargetLanguage::Go => {
            // Go persistence not yet implemented
        }
        TargetLanguage::Erlang => {
            // Erlang persistence not yet implemented
        }
        TargetLanguage::Lua | TargetLanguage::Dart => {
            // TODO: Lua/Dart persistence not yet implemented
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
            save_body.push_str("state_data[\"_compartment\"] = _ser_chain.call(self.__compartment)\n");
            save_body.push_str("var stack_arr = []\n");
            save_body.push_str("for c in self._state_stack:\n");
            save_body.push_str("    stack_arr.append(_ser_chain.call(c))\n");
            save_body.push_str("state_data[\"_state_stack\"] = stack_arr\n");

            // Add domain variables
            for var in &system.domain {
                save_body.push_str(&format!("state_data[\"{}\"] = self.{}\n", var.name, var.name));
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
            restore_body.push_str(&format!("        var comp = {}.new(cd[\"state\"])\n", compartment_type));
            restore_body.push_str("        comp.state_args = cd.get(\"state_args\", {})\n");
            restore_body.push_str("        comp.state_vars = cd.get(\"state_vars\", {})\n");
            restore_body.push_str("        comp.enter_args = cd.get(\"enter_args\", {})\n");
            restore_body.push_str("        comp.exit_args = cd.get(\"exit_args\", {})\n");
            restore_body.push_str("        comp.parent_compartment = result\n");
            restore_body.push_str("        result = comp\n");
            restore_body.push_str("    return result\n");

            restore_body.push_str(&format!("var instance = {}.new()\n", system.name));
            restore_body.push_str("instance.__compartment = _deser_chain.call(state_data[\"_compartment\"])\n");
            restore_body.push_str("instance.__next_compartment = null\n");
            restore_body.push_str("instance._state_stack = []\n");
            restore_body.push_str("for c in state_data.get(\"_state_stack\", []):\n");
            restore_body.push_str("    instance._state_stack.append(_deser_chain.call(c))\n");
            restore_body.push_str("instance._context_stack = []\n");

            for var in &system.domain {
                restore_body.push_str(&format!("instance.{} = state_data.get(\"{}\", null)\n", var.name, var.name));
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

