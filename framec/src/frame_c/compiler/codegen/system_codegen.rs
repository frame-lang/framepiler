//! System Code Generation from Frame AST
//!
//! This module transforms Frame AST (SystemAst) into CodegenNode for emission
//! by language-specific backends.
//!
//! Uses the "oceans model" - native code is preserved exactly, Frame segments
//! are replaced with generated code using the splicer.

use super::ast::*;
use super::backend::get_backend;
use super::codegen_utils::{
    convert_expression, convert_literal, cpp_map_type, cpp_wrap_any_arg, csharp_map_type,
    expression_to_string, go_map_type, is_bool_type, is_float_type, is_int_type, is_string_type,
    java_map_type, kotlin_map_type, state_var_init_value, swift_map_type, to_snake_case,
    type_to_cpp_string, type_to_string, HandlerContext,
};
use super::frame_expansion::{generate_frame_expansion, get_native_scanner, normalize_indentation};
use super::interface_gen::{
    generate_action, generate_interface_wrappers, generate_operation, generate_persistence_methods,
};
use super::state_dispatch::{generate_handler_from_arcanum, generate_state_handlers_via_arcanum};
use crate::frame_c::compiler::arcanum::{Arcanum, HandlerEntry};
use crate::frame_c::compiler::frame_ast::{
    ActionAst, BinaryOp, Expression, InterfaceMethod, Literal, MachineAst, MethodParam,
    OperationAst, Span, StateVarAst, SystemAst, Type, UnaryOp,
};
use crate::frame_c::compiler::native_region_scanner::{
    c::NativeRegionScannerC, cpp::NativeRegionScannerCpp, csharp::NativeRegionScannerCs,
    dart::NativeRegionScannerDart, erlang::NativeRegionScannerErlang,
    gdscript::NativeRegionScannerGDScript, go::NativeRegionScannerGo,
    java::NativeRegionScannerJava, javascript::NativeRegionScannerJs,
    kotlin::NativeRegionScannerKotlin, lua::NativeRegionScannerLua, php::NativeRegionScannerPhp,
    python::NativeRegionScannerPy, ruby::NativeRegionScannerRuby, rust::NativeRegionScannerRust,
    swift::NativeRegionScannerSwift, typescript::NativeRegionScannerTs, FrameSegmentKind,
    NativeRegionScanner, Region,
};
use crate::frame_c::visitors::TargetLanguage;

/// Check if a word appears at a whole-word boundary in text.
/// Leading boundary excludes alphanumeric, underscore, and any chars in `extra_leading`.
/// Trailing boundary excludes alphanumeric and underscore.
fn is_whole_word_at(bytes: &[u8], start: usize, end: usize, extra_leading: &[u8]) -> bool {
    let prev_ok = start == 0 || {
        let b = bytes[start - 1];
        !(b.is_ascii_alphanumeric() || b == b'_' || extra_leading.contains(&b))
    };
    let next_ok = end >= bytes.len() || {
        let b = bytes[end];
        !(b.is_ascii_alphanumeric() || b == b'_')
    };
    prev_ok && next_ok
}

/// Find all whole-word occurrences of `word` in `text`, calling `callback` for each.
/// The callback receives (start, end) byte positions.
fn find_whole_words(
    text: &[u8],
    word: &[u8],
    extra_leading: &[u8],
    mut callback: impl FnMut(usize, usize) -> bool,
) {
    let mut i = 0;
    while i + word.len() <= text.len() {
        if let Some(found) = text[i..].windows(word.len()).position(|w| w == word) {
            let start = i + found;
            let end = start + word.len();
            if is_whole_word_at(text, start, end, extra_leading) {
                if !callback(start, end) {
                    return; // callback returned false = stop searching
                }
            }
            i = end;
        } else {
            break;
        }
    }
}

/// True iff the init expression text contains any of the supplied param
/// names as a whole word. Used to detect `balance: int = balance` where
/// a domain field initializer references a constructor parameter.
pub(crate) fn init_references_param(init_text: &str, params: &[String]) -> bool {
    if params.is_empty() || init_text.is_empty() {
        return false;
    }
    let bytes = init_text.as_bytes();
    for p in params {
        if p.is_empty() {
            continue;
        }
        let mut found = false;
        find_whole_words(bytes, p.as_bytes(), b".", |_, _| {
            found = true;
            false
        });
        if found {
            return true;
        }
    }
    false
}

/// Prefix `$` to identifiers in `text` that match system param names.
/// Used for PHP domain initializer expressions (e.g. `initial_balance` → `$initial_balance`).
fn prefix_php_vars(text: &str, params: &[String]) -> String {
    let mut result = text.to_string();
    for p in params {
        if p.is_empty() {
            continue;
        }
        let mut new_result = String::new();
        let bytes = result.as_bytes();
        let pb = p.as_bytes();
        let mut i = 0usize;
        while i + pb.len() <= bytes.len() {
            if let Some(found) = bytes[i..].windows(pb.len()).position(|w| w == pb) {
                let start = i + found;
                let end = start + pb.len();
                new_result.push_str(&result[i..start]);
                if is_whole_word_at(bytes, start, end, b"$") {
                    new_result.push('$');
                }
                new_result.push_str(p);
                i = end;
            } else {
                new_result.push_str(&result[i..]);
                i = bytes.len(); // done
            }
        }
        if i < result.len() {
            new_result.push_str(&result[i..]);
        }
        result = new_result;
    }
    result
}

/// Generate a complete CodegenNode for a Frame system
///
/// # Arguments
/// * `system` - The parsed Frame system AST
/// * `arcanum` - Symbol table for the system (used for handler info and validation)
/// * `lang` - Target language for code generation
/// * `source` - Original source bytes (used to extract native code via spans)
pub fn generate_system(
    system: &SystemAst,
    arcanum: &Arcanum,
    lang: TargetLanguage,
    source: &[u8],
) -> CodegenNode {
    // Erlang: gen_statem native — completely different codegen path
    if lang == TargetLanguage::Erlang {
        return super::erlang_system::generate_erlang_system(system, arcanum, source);
    }
    // Rust: dedicated codegen with Rc<RefCell<Compartment>> ownership
    if lang == TargetLanguage::Rust {
        return super::rust_system::generate_rust_system(system, arcanum, source);
    }

    generate_system_shared(system, arcanum, lang, source)
}

/// Shared system codegen for class-based languages (15 of 17).
/// Rust and Erlang have dedicated pipelines in `rust_system.rs` and
/// `erlang_system.rs`; they never reach this function.
pub fn generate_system_shared(
    system: &SystemAst,
    arcanum: &Arcanum,
    lang: TargetLanguage,
    source: &[u8],
) -> CodegenNode {
    debug_assert!(
        !matches!(lang, TargetLanguage::Rust | TargetLanguage::Erlang),
        "Rust and Erlang have dedicated pipelines — should not reach generate_system_shared"
    );
    let backend = get_backend(lang);
    let syntax = backend.class_syntax();

    // Determine if this system needs async dispatch chain
    let needs_async = system.interface.iter().any(|m| m.is_async);

    // Generate fields
    let fields = generate_fields(system, &syntax);

    // Generate methods
    let mut methods = Vec::new();

    // Constructor (for async systems, skips $> kernel call)
    methods.push(generate_constructor(system, &syntax));

    // Frame machinery (transition, state management)
    methods.extend(generate_frame_machinery(system, &syntax, lang));

    // Interface wrappers
    methods.extend(generate_interface_wrappers(system, &syntax));

    // Check if system has states with state variables (for Rust compartment-based push/pop)
    let has_state_vars = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().any(|s| !s.state_vars.is_empty()))
        .unwrap_or(false);

    // State handlers - use enhanced Arcanum for clean iteration
    if let Some(ref machine) = system.machine {
        methods.extend(generate_state_handlers_via_arcanum(
            &system.name,
            machine,
            arcanum,
            source,
            lang,
            has_state_vars,
        ));
    }

    // Actions - extract native code from source using spans
    for action in &system.actions {
        methods.push(generate_action(action, &syntax, source));
    }

    // Operations - extract native code from source using spans
    for operation in &system.operations {
        methods.push(generate_operation(operation, &syntax, source));
    }

    // Persistence methods (when @@persist is present)
    if system.persist_attr.is_some() {
        methods.extend(generate_persistence_methods(system, &syntax));
    }

    let mut class_node = CodegenNode::Class {
        name: system.name.clone(),
        fields,
        methods,
        base_classes: system.bases.clone(),
        is_abstract: false,
        derives: vec![], // Derives not used - we manually build JSON
        visibility: if system.visibility.as_deref() == Some("private") {
            Visibility::Private
        } else {
            Visibility::Public
        },
    };

    // Post-process: make dispatch chain async if any interface method is async.
    // Java is excluded — it has no native async/await. Instead, Java
    // interface methods return CompletableFuture<T> (see `make_java_interface_async`)
    // while the internal dispatch chain stays synchronous.
    if needs_async {
        if matches!(lang, TargetLanguage::Java) {
            make_java_interface_async(&mut class_node, system);
        } else {
            make_system_async(&mut class_node, &system.name, lang);
        }
    }

    class_node
}

/// Java-specific async handling: only the public interface methods get
/// `is_async = true` (triggering `CompletableFuture<T>` return-type wrapping
/// in the Java backend). The internal dispatch chain (__kernel, __router,
/// _state_X, __transition, init) stays synchronous — callers would have to
/// `.get()` each internal call otherwise, and deep chains become noisy
/// without buying real concurrency. Users call `worker.get_status().get()`
/// to await an interface result.
fn make_java_interface_async(class_node: &mut CodegenNode, system: &SystemAst) {
    let async_names: std::collections::HashSet<String> = system
        .interface
        .iter()
        .filter(|m| m.is_async)
        .map(|m| m.name.clone())
        .collect();
    if let CodegenNode::Class { ref mut methods, .. } = class_node {
        for method in methods.iter_mut() {
            if let CodegenNode::Method {
                is_async, name, ..
            } = method
            {
                if async_names.contains(name) {
                    *is_async = true;
                }
            }
        }
    }
}

/// Transform a generated system class to use async dispatch.
///
/// When any interface method is declared `async`, the entire dispatch chain
/// (interface methods, kernel, router, state dispatch, handlers) must be async.
/// This post-processes the CodegenNode tree to:
/// 1. Set `is_async: true` on all non-static, non-constructor methods
/// 2. The backends handle `is_async` to emit `async def` / `async` / etc.
///
/// Note: `await` on internal dispatch calls is handled by the backends
/// recognizing Await nodes in the method bodies, or via NativeBlock code
/// that already contains the dispatch calls.
pub(crate) fn make_system_async(
    class_node: &mut CodegenNode,
    _system_name: &str,
    lang: TargetLanguage,
) {
    if let CodegenNode::Class {
        ref mut methods,
        ref name,
        ..
    } = class_node
    {
        let system_name = name.clone();
        for method in methods.iter_mut() {
            if let CodegenNode::Method {
                is_async,
                is_static,
                name,
                body,
                ..
            } = method
            {
                // Skip static methods and constructors
                if *is_static || name == "__init__" || name == "new" {
                    continue;
                }
                // Skip __transition (synchronous compartment swap, no dispatch)
                // But NOT __push_transition (has dispatch calls that need await)
                if name == "__transition" {
                    continue;
                }
                *is_async = true;
                // Add `await` to internal dispatch calls in NativeBlock strings
                add_await_to_dispatch_calls(body, lang);
            }
            if let CodegenNode::Constructor { body, .. } = method {
                // Constructor stays sync — remove kernel call for async systems
                // (user calls `await system.init()` instead)
                remove_kernel_call_from_body(body);
            }
        }

        // Add async init() method — fires $> enter event
        let init_code = match lang {
            TargetLanguage::Python3 => format!(
                r#"__e = {s}FrameEvent("$>", None)
__ctx = {s}FrameContext(__e, None)
self._context_stack.append(__ctx)
await self.__kernel(__e)
self._context_stack.pop()"#,
                s = system_name
            ),
            TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!(
                r#"const __e = new {s}FrameEvent("$>", null);
const __ctx = new {s}FrameContext(__e, null);
this._context_stack.push(__ctx);
await this.__kernel(__e);
this._context_stack.pop();"#,
                s = system_name
            ),
            TargetLanguage::Rust => format!(
                r#"let __frame_event = {s}FrameEvent::new("$>");
let __ctx = {s}FrameContext::new(__frame_event, None);
self._context_stack.push(__ctx);
self.__kernel().await;
self._context_stack.pop();"#,
                s = system_name
            ),
            // Async not supported for these targets — emit a comment placeholder
            TargetLanguage::C => format!("// async not supported for C"),
            TargetLanguage::Cpp => format!(
                r#"{s}FrameEvent __e("$>");
{s}FrameContext __ctx(std::move(__e));
_context_stack.push_back(std::move(__ctx));
co_await __kernel(_context_stack.back()._event);
_context_stack.pop_back();
co_return;"#,
                s = system_name
            ),
            TargetLanguage::Dart => format!(
                r#"final __e = {s}FrameEvent("\$>", []);
final __ctx = {s}FrameContext(__e, null);
_context_stack.add(__ctx);
await __kernel(__e);
_context_stack.removeLast();"#,
                s = system_name
            ),
            TargetLanguage::GDScript => format!(
                r#"var __e = {s}FrameEvent.new("$>", [])
var __ctx = {s}FrameContext.new(__e, null)
self._context_stack.append(__ctx)
await self.__kernel(__e)
self._context_stack.pop_back()"#,
                s = system_name
            ),
            TargetLanguage::Kotlin => format!(
                r#"val __e = {s}FrameEvent("$>", mutableListOf<Any?>())
val __ctx = {s}FrameContext(__e, null)
_context_stack.add(__ctx)
__kernel(__e)
_context_stack.removeLast()"#,
                s = system_name
            ),
            TargetLanguage::Swift => format!(
                r#"let __e = {s}FrameEvent(message: "$>", parameters: [])
let __ctx = {s}FrameContext(event: __e)
_context_stack.append(__ctx)
await __kernel(__e)
_context_stack.removeLast()"#,
                s = system_name
            ),
            TargetLanguage::CSharp => format!(
                r#"var __e = new {s}FrameEvent("$>", new List<object>());
var __ctx = new {s}FrameContext(__e, null);
_context_stack.Add(__ctx);
await __kernel(__e);
_context_stack.RemoveAt(_context_stack.Count - 1);"#,
                s = system_name
            ),
            // Languages with async that haven't been implemented yet
            TargetLanguage::Java
            | TargetLanguage::Go
            | TargetLanguage::Php
            | TargetLanguage::Ruby
            | TargetLanguage::Lua => {
                format!("// async init not yet implemented for {:?}", lang)
            }
            TargetLanguage::Erlang => String::new(), // gen_statem: handled natively by erlang_system.rs
            TargetLanguage::Graphviz => unreachable!(),
        };
        let init_body = vec![CodegenNode::NativeBlock {
            code: init_code,
            span: None,
        }];

        // Swift: `init` is reserved for constructors — `func init() async`
        // is a parse error. Rename just for Swift so tests call
        // `await w.initAsync()` instead of `await w.init()`.
        let init_name = match lang {
            TargetLanguage::Swift => "initAsync".to_string(),
            _ => "init".to_string(),
        };

        methods.push(CodegenNode::Method {
            name: init_name,
            params: vec![],
            return_type: None,
            body: init_body,
            is_async: true,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        });
    }
}

/// Remove kernel call from constructor body (for async two-phase init).
fn remove_kernel_call_from_body(body: &mut Vec<CodegenNode>) {
    body.retain(|node| {
        if let CodegenNode::NativeBlock { code, .. } = node {
            // Remove lines that call __kernel in constructor
            !code.contains("__kernel(") && !code.contains("_kernel(self,")
        } else {
            true
        }
    });
}

/// Walk method body and add `await` to dispatch calls in NativeBlock strings.
/// Dispatch calls are: self.__kernel(...), self.__router(...), self._state_*(...),
/// self._s_*(...) — all internal Frame dispatch methods.
fn add_await_to_dispatch_calls(body: &mut Vec<CodegenNode>, lang: TargetLanguage) {
    for node in body.iter_mut() {
        match node {
            CodegenNode::NativeBlock { code, .. } => {
                *code = add_await_to_string(code, lang);
            }
            CodegenNode::If {
                then_block,
                else_block,
                ..
            } => {
                add_await_to_dispatch_calls(then_block, lang);
                if let Some(els) = else_block {
                    add_await_to_dispatch_calls(els, lang);
                }
            }
            CodegenNode::While {
                body: while_body, ..
            } => {
                add_await_to_dispatch_calls(while_body, lang);
            }
            _ => {}
        }
    }
}

/// Add `await` to Frame dispatch calls in a string.
/// Python/TypeScript: prefix `await` (e.g., `await self.__kernel()`)
/// Rust: postfix `.await` (e.g., `self.__kernel().await`)
fn add_await_to_string(code: &str, lang: TargetLanguage) -> String {
    let mut result = String::with_capacity(code.len() + 100);
    for line in code.lines() {
        let trimmed = line.trim();
        // Match dispatch call patterns that need await.
        // Swift/Kotlin/C#/Dart (and some branches of others) emit bare
        // references without a `self.`/`this.` prefix — match those too.
        // These names are framec-generated (`__kernel`, `__router`,
        // `_state_<Name>`, `_s_<...>`) so bare matching is safe.
        let needs_await = trimmed.starts_with("self.__kernel(")
            || trimmed.starts_with("self.__router(")
            || (trimmed.starts_with("self._state_") && !trimmed.starts_with("self._state_stack"))
            || trimmed.starts_with("self._s_")
            || trimmed.starts_with("handler(")         // Python dynamic dispatch
            || trimmed.starts_with("handler.call(")   // TypeScript dynamic dispatch
            || trimmed.starts_with("this.__kernel(")
            || trimmed.starts_with("this.__router(")
            || trimmed.starts_with("this._state_")
            || trimmed.starts_with("this._s_")
            || trimmed.starts_with("this.#kernel(")
            || trimmed.starts_with("this.#router(")
            || trimmed.starts_with("$this->__kernel(")
            || trimmed.starts_with("$this->__router(")
            || trimmed.starts_with("$this->_state_")
            // Bare references (no `self.`/`this.` prefix): Swift uses bare
            // for same-instance method calls.
            || trimmed.starts_with("__kernel(")
            || trimmed.starts_with("__router(")
            || (trimmed.starts_with("_state_") && !trimmed.starts_with("_state_stack"))
            // Rust match arms and braced dispatch:
            // "StateName" => self._state_X(__e),
            // { self._s_Ready_fetch(__e, key); }
            || (trimmed.contains("self._state_") && !trimmed.contains("_state_stack") && !trimmed.starts_with("fn ") && !trimmed.starts_with("async fn "))
            || (trimmed.contains("self._s_") && !trimmed.starts_with("fn ") && !trimmed.starts_with("async fn "));
        if needs_await {
            let indent = &line[..line.len() - trimmed.len()];
            match lang {
                TargetLanguage::Rust => {
                    // Rust: postfix .await — insert before ); or ); } or ), etc.
                    if !trimmed.contains(".await") {
                        result.push_str(indent);
                        // Find the call's closing paren `)` and insert `.await` after it
                        let modified = insert_rust_await(trimmed);
                        result.push_str(&modified);
                    } else {
                        result.push_str(line);
                    }
                }
                // Kotlin: `suspend fun` → `suspend fun` calls are bare,
                // no `await` keyword. Emit the line unchanged.
                TargetLanguage::Kotlin => {
                    result.push_str(line);
                }
                // C++: coroutines use `co_await` keyword, not `await`.
                TargetLanguage::Cpp => {
                    if !trimmed.starts_with("co_await ") {
                        result.push_str(indent);
                        result.push_str("co_await ");
                        result.push_str(trimmed);
                    } else {
                        result.push_str(line);
                    }
                }
                // All other async-capable languages use prefix `await`
                TargetLanguage::Python3
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::CSharp
                | TargetLanguage::Swift
                | TargetLanguage::Java
                | TargetLanguage::Go
                | TargetLanguage::C
                | TargetLanguage::Php
                | TargetLanguage::Ruby
                | TargetLanguage::Erlang
                | TargetLanguage::Lua
                | TargetLanguage::Dart
                | TargetLanguage::GDScript => {
                    // Python/TypeScript/C#/etc: prefix await
                    if !trimmed.starts_with("await ") {
                        result.push_str(indent);
                        result.push_str("await ");
                        result.push_str(trimmed);
                    } else {
                        result.push_str(line);
                    }
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    // Remove trailing newline if original didn't have one
    if !code.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Insert `.await` after the closing paren of a Rust function call.
/// Handles patterns like:
///   `self.__kernel();`          → `self.__kernel().await;`
///   `self._state_Ready(__e),`   → `self._state_Ready(__e).await,`
///   `{ self._s_X(__e, k); }`   → `{ self._s_X(__e, k).await; }`
fn insert_rust_await(line: &str) -> String {
    // Find the last `)` that's part of a function call, insert `.await` after it
    let bytes = line.as_bytes();
    let mut last_close_paren = None;
    let mut depth = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'(' {
            depth += 1;
        }
        if b == b')' {
            depth -= 1;
            if depth == 0 {
                last_close_paren = Some(i);
            }
        }
    }
    if let Some(pos) = last_close_paren {
        let mut result = String::with_capacity(line.len() + 6);
        result.push_str(&line[..pos + 1]);
        result.push_str(".await");
        result.push_str(&line[pos + 1..]);
        result
    } else {
        // No paren found — just append .await before semicolon
        let trimmed_semi = line.trim_end_matches(';');
        format!("{}.await;", trimmed_semi)
    }
}

/// Generate class fields for the system
pub(crate) fn generate_fields(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> Vec<Field> {
    let mut fields = Vec::new();
    let compartment_type = format!("{}Compartment", system.name);

    // State stack - for push/pop state operations
    let stack_type = match syntax.language {
        TargetLanguage::Rust => format!("Vec<{}Compartment>", system.name),
        TargetLanguage::Cpp => format!("std::vector<std::shared_ptr<{}Compartment>>", system.name),
        TargetLanguage::Java => format!("ArrayList<{}Compartment>", system.name),
        TargetLanguage::Kotlin => format!("MutableList<{}Compartment>", system.name),
        TargetLanguage::Dart => format!("List<{}Compartment>", system.name),
        TargetLanguage::Swift => format!("[{}Compartment]", system.name),
        TargetLanguage::CSharp => format!("List<{}Compartment>", system.name),
        TargetLanguage::Go => format!("[]*{}Compartment", system.name),
        // Dynamic languages: untyped lists — type annotation is for documentation only
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Lua
        | TargetLanguage::GDScript => "List".to_string(),
        TargetLanguage::C => "List".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(
        Field::new("_state_stack")
            .with_visibility(Visibility::Private)
            .with_type(&stack_type),
    );

    // Compartment field - canonical compartment architecture for ALL languages
    let (comp_field_type, nullable_comp_type) = match syntax.language {
        TargetLanguage::Rust => (
            compartment_type.clone(),
            format!("Option<{}>", compartment_type),
        ),
        TargetLanguage::Cpp => (
            format!("std::shared_ptr<{}>", compartment_type),
            format!("std::shared_ptr<{}>", compartment_type),
        ),
        TargetLanguage::Java => (compartment_type.clone(), compartment_type.clone()),
        TargetLanguage::CSharp => (compartment_type.clone(), format!("{}?", compartment_type)),
        TargetLanguage::Kotlin | TargetLanguage::Swift | TargetLanguage::Dart => {
            (compartment_type.clone(), format!("{}?", compartment_type))
        }
        TargetLanguage::Go => (
            format!("*{}", compartment_type),
            format!("*{}", compartment_type),
        ),
        // Dynamic languages: nullable via language convention (None/null/nil)
        TargetLanguage::Python3
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Lua
        | TargetLanguage::GDScript => (compartment_type.clone(), compartment_type.clone()),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => (
            compartment_type.clone(),
            format!("{} | null", compartment_type),
        ),
        TargetLanguage::Php => (compartment_type.clone(), format!("?{}", compartment_type)),
        TargetLanguage::C => (
            format!("{}*", compartment_type),
            format!("{}*", compartment_type),
        ),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(
        Field::new("__compartment")
            .with_visibility(Visibility::Private)
            .with_type(&comp_field_type),
    );

    // Next compartment field - for deferred transition caching in __kernel
    fields.push(
        Field::new("__next_compartment")
            .with_visibility(Visibility::Private)
            .with_type(&nullable_comp_type),
    );

    // Context stack for reentrancy - holds FrameContext objects
    let context_stack_type = match syntax.language {
        TargetLanguage::Rust => format!("Vec<{}FrameContext>", system.name),
        TargetLanguage::Cpp => format!("std::vector<{}FrameContext>", system.name),
        TargetLanguage::Java => format!("ArrayList<{}FrameContext>", system.name),
        TargetLanguage::Kotlin => format!("MutableList<{}FrameContext>", system.name),
        TargetLanguage::Dart => format!("List<{}FrameContext>", system.name),
        TargetLanguage::Swift => format!("[{}FrameContext]", system.name),
        TargetLanguage::CSharp => format!("List<{}FrameContext>", system.name),
        TargetLanguage::Go => format!("[]{}FrameContext", system.name),
        // Dynamic languages: untyped lists
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Lua
        | TargetLanguage::GDScript => "List".to_string(),
        TargetLanguage::C => "List".to_string(),
        TargetLanguage::Graphviz => unreachable!(),
    };
    fields.push(
        Field::new("_context_stack")
            .with_visibility(Visibility::Private)
            .with_type(&context_stack_type),
    );

    // Domain variables — build a structured `Field` for each. Backends
    // consume the structured slots (name, type_annotation, initializer,
    // is_const) directly via their own `emit_field` helpers; nothing
    // re-parses a synthesized declaration string anymore.
    for domain_var in &system.domain {
        // Structured type slot: keep as None when the user wrote no
        // type (Type::Unknown), so backends can omit the type
        // annotation entirely and let the target language infer.
        let type_str_opt = match &domain_var.var_type {
            Type::Custom(s) => Some(s.clone()),
            Type::Unknown => None,
        };
        let sys_param_names: Vec<String> = system.params.iter().map(|p| p.name.clone()).collect();

        let mut field = Field::new(&domain_var.name).with_visibility(Visibility::Public);
        if let Some(ref t) = type_str_opt {
            field = field.with_type(t);
        }
        field.is_const = domain_var.is_const;

        // Populate the structured initializer slot from init text —
        // but ONLY when the init wasn't stripped from the field declaration.
        // If the init references a system param, the assignment belongs in
        // the constructor body, not the field declaration.
        // Apply expand_tagged_in_domain so backends consuming this slot get
        // the fully-expanded init text (@@SystemName → native constructor).
        let init_text_str = domain_var.initializer_text.as_deref().unwrap_or("");
        let strip_unconditionally =
            matches!(syntax.language, TargetLanguage::Go | TargetLanguage::C);
        let strip_collision = init_references_param(init_text_str, &sys_param_names);
        if !(strip_unconditionally || strip_collision) {
            if let Some(ref init_text) = &domain_var.initializer_text {
                let expanded_init = expand_tagged_in_domain(init_text, syntax.language);
                field = field.with_initializer(CodegenNode::Ident(expanded_init));
            }
        }

        fields.push(field);
    }

    // Rust state vars now live on compartment.state_context (StateContext enum)
    // No _sv_* struct fields needed

    // Rust system header state/enter params: stash on the system struct
    // as `__sys_<name>` typed fields. The constructor receives the
    // system params via its signature (system_codegen.rs:1752 — params
    // from system.params), assigns them into these synthetic fields,
    // and the per-state dispatch reads them into bare locals via the
    // binding preamble inserted by `generate_handler_from_arcanum`.
    //
    // Domain params are handled by the existing domain field path
    // (the domain field IS the storage), so we skip them here.
    //
    // This is the Rust equivalent of the HashMap<String, Any>
    // `state_args`/`enter_args` dict approach used by the dynamic
    // backends — typed fields keep idiomatic Rust without inventing
    // a new typed-enum variant per state.
    if matches!(syntax.language, TargetLanguage::Rust) {
        for p in &system.params {
            match p.kind {
                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                | crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                    let type_str = type_to_string(&p.param_type);
                    fields.push(
                        Field::new(&format!("__sys_{}", p.name))
                            .with_visibility(Visibility::Private)
                            .with_type(&type_str),
                    );
                }
                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
            }
        }
    }

    fields
}

/// Element type carried by one of the kernel's two stacks. C# and Go
/// format the element type into the init expression; every other language
/// either uses a default-constructed empty container or doesn't care
/// about the element type at all.
enum StackElementKind {
    /// `_state_stack` — Compartment values; pointer-typed in Go.
    Compartment,
    /// `_context_stack` — FrameContext values; never pointer-typed.
    FrameContext,
}

/// Emit the per-language initializer for a kernel stack field
/// (`_state_stack` or `_context_stack`). Returns `None` for languages
/// where no init is needed (C++: vectors default-construct empty;
/// Graphviz: unreachable in this code path).
fn init_collection_stack(
    field_name: &str,
    element_kind: StackElementKind,
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> Option<CodegenNode> {
    match syntax.language {
        TargetLanguage::C => Some(CodegenNode::assign(
            CodegenNode::field(CodegenNode::self_ref(), field_name),
            CodegenNode::Ident(format!("{}_FrameVec_new()", system.name)),
        )),
        // C++ vectors default-construct as empty; no init needed.
        TargetLanguage::Cpp => None,
        TargetLanguage::Java => Some(CodegenNode::NativeBlock {
            code: format!("{} = new ArrayList<>();", field_name),
            span: None,
        }),
        TargetLanguage::Kotlin => Some(CodegenNode::NativeBlock {
            code: format!("{} = mutableListOf()", field_name),
            span: None,
        }),
        TargetLanguage::Swift => Some(CodegenNode::NativeBlock {
            code: format!("{} = []", field_name),
            span: None,
        }),
        TargetLanguage::CSharp => {
            let elem = match element_kind {
                StackElementKind::Compartment => format!("{}Compartment", system.name),
                StackElementKind::FrameContext => format!("{}FrameContext", system.name),
            };
            Some(CodegenNode::NativeBlock {
                code: format!("{} = new List<{}>();", field_name, elem),
                span: None,
            })
        }
        TargetLanguage::Go => {
            // Go state-stack stores Compartment pointers; context-stack stores values.
            let elem = match element_kind {
                StackElementKind::Compartment => format!("*{}Compartment", system.name),
                StackElementKind::FrameContext => format!("{}FrameContext", system.name),
            };
            Some(CodegenNode::NativeBlock {
                code: format!("s.{} = make([]{}, 0)", field_name, elem),
                span: None,
            })
        }
        // Dynamic / array-literal languages.
        TargetLanguage::Python3
        | TargetLanguage::TypeScript
        | TargetLanguage::JavaScript
        | TargetLanguage::Php
        | TargetLanguage::Ruby
        | TargetLanguage::Erlang
        | TargetLanguage::Rust
        | TargetLanguage::Lua
        | TargetLanguage::Dart
        | TargetLanguage::GDScript => Some(CodegenNode::assign(
            CodegenNode::field(CodegenNode::self_ref(), field_name),
            CodegenNode::Array(vec![]),
        )),
        TargetLanguage::Graphviz => unreachable!(),
    }
}

/// Decide whether the constructor body should emit a domain-field init
/// statement, given the language and the field's properties.
///
/// Three groups:
/// - **Always emit** (C, Go, Python, Ruby, Lua, Rust): no field-level
///   init is available (C/Go) or none is generated (dynamic langs go
///   straight to the constructor body).
/// - **Emit only on collision** (Cpp, Java, Swift, C#, Dart, GDScript,
///   TS, JS, PHP): the field has a literal init at declaration scope,
///   except when that init references a system param — then the init
///   moves into the constructor body to avoid the name-collision.
/// - **Kotlin**: same as the OO group, but const fields with collisions
///   are suppressed entirely (they're declared in the primary
///   constructor as `val name: Type`).
/// - **Erlang / Graphviz**: never go through this code path.
fn should_emit_constructor_body_init(
    lang: TargetLanguage,
    is_const: bool,
    init_refs_param: bool,
) -> bool {
    use TargetLanguage::*;
    match lang {
        C | Go | Python3 | Ruby | Lua | Rust => true,
        Cpp | Java | Swift | CSharp | Dart | GDScript | TypeScript | JavaScript | Php => {
            init_refs_param
        }
        Kotlin => init_refs_param && !is_const,
        Erlang | Graphviz => false,
    }
}

/// Format `field = init_value` using the per-language self-access form
/// and statement terminator. Used to build the constructor body's
/// domain-field init lines for every language EXCEPT Rust (which uses
/// the structured `CodegenNode::assign` instead) and the C++ const-init
/// case (which uses a member initializer list).
fn format_field_assignment(lang: TargetLanguage, field_name: &str, init_value: &str) -> String {
    use TargetLanguage::*;
    match lang {
        C => format!("self->{} = {};", field_name, init_value),
        Cpp => format!("this->{} = {};", field_name, init_value),
        Go => format!("s.{} = {}", field_name, init_value),
        Java | CSharp | Dart | TypeScript | JavaScript => {
            format!("this.{} = {};", field_name, init_value)
        }
        Swift | GDScript | Python3 | Lua => format!("self.{} = {}", field_name, init_value),
        Kotlin => format!("this.{} = {}", field_name, init_value),
        Php => format!("$this->{} = {};", field_name, init_value),
        Ruby => format!("@{} = {}", field_name, init_value),
        Rust | Erlang | Graphviz => unreachable!(
            "format_field_assignment called for {:?} (Rust uses structured assign; \
             Erlang/Graphviz never reach this code path)",
            lang
        ),
    }
}

/// Generate the constructor
pub(crate) fn generate_constructor(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
) -> CodegenNode {
    let mut body = Vec::new();
    // C++ member initializer list entries: `field(value), field2(value2)`
    // Collected during domain init loop for const fields whose init was stripped.
    let mut cpp_init_list: Vec<String> = Vec::new();

    // Initialize state stack and context stack — same shape per language,
    // only the field name and element-type formatting differ.
    if let Some(node) = init_collection_stack(
        "_state_stack",
        StackElementKind::Compartment,
        system,
        syntax,
    ) {
        body.push(node);
    }
    if let Some(node) = init_collection_stack(
        "_context_stack",
        StackElementKind::FrameContext,
        system,
        syntax,
    ) {
        body.push(node);
    }

    // Initialize domain variables.
    //
    // Three concerns interleave per language:
    //   1. WHETHER to emit a body init at all (see
    //      `should_emit_constructor_body_init`).
    //   2. HOW to spell `self.field = value` for the target
    //      (see `format_field_assignment`).
    //   3. Per-language adjustments to the init expression itself
    //      (PHP `$`-prefix on param refs, Lua `[]` → `{}`, Rust
    //      Domain-param override, C++ const → member init list).
    //
    // Rust is the structural odd-one-out: it uses `CodegenNode::assign`
    // instead of a `NativeBlock` and must always init every field
    // (including the no-init case, handled up front).
    let sys_param_names_for_init: Vec<String> =
        system.params.iter().map(|p| p.name.clone()).collect();
    for domain_var in &system.domain {
        // Rust requires all fields initialized; handle the no-init
        // case up front before the regular path.
        if matches!(syntax.language, TargetLanguage::Rust) && domain_var.initializer_text.is_none()
        {
            let rust_init = if system.params.iter().any(|p| {
                p.name == domain_var.name
                    && matches!(
                        p.kind,
                        crate::frame_c::compiler::frame_ast::ParamKind::Domain
                    )
            }) {
                domain_var.name.clone()
            } else {
                "Default::default()".to_string()
            };
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                CodegenNode::Ident(rust_init),
            ));
            continue;
        }

        let init_text = match &domain_var.initializer_text {
            Some(t) => t,
            None => continue,
        };
        let init_refs_param = init_references_param(init_text, &sys_param_names_for_init);

        if !should_emit_constructor_body_init(syntax.language, domain_var.is_const, init_refs_param)
        {
            continue;
        }

        let init_expanded = expand_tagged_in_domain(init_text, syntax.language);

        // C++ const fields whose init references a constructor param
        // must use the member initializer list — `const T x;` cannot
        // be reassigned in the body. The list is rendered before the
        // constructor body as `: x(value)`.
        if matches!(syntax.language, TargetLanguage::Cpp) && domain_var.is_const {
            cpp_init_list.push(format!("{}({})", domain_var.name, init_expanded));
            continue;
        }

        // Per-language init-expression adjustments.
        let final_init = match syntax.language {
            // PHP rejects bare names in init expressions; system params
            // need a `$` prefix to be valid PHP.
            TargetLanguage::Php => prefix_php_vars(&init_expanded, &sys_param_names_for_init),
            // Lua uses `{}` for empty tables, not `[]`.
            TargetLanguage::Lua if init_expanded.trim() == "[]" => "{}".to_string(),
            // Rust: a Domain-kind system param of the same name overrides
            // the literal default.
            TargetLanguage::Rust
                if system.params.iter().any(|p| {
                    p.name == domain_var.name
                        && matches!(
                            p.kind,
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain
                        )
                }) =>
            {
                domain_var.name.clone()
            }
            _ => init_expanded,
        };

        if matches!(syntax.language, TargetLanguage::Rust) {
            body.push(CodegenNode::assign(
                CodegenNode::field(CodegenNode::self_ref(), &domain_var.name),
                CodegenNode::Ident(final_init),
            ));
        } else {
            body.push(CodegenNode::NativeBlock {
                code: format_field_assignment(syntax.language, &domain_var.name, &final_init),
                span: None,
            });
        }
    }

    // Domain-kind system params override domain field defaults.
    // The domain init uses the literal default (e.g., `self.inventory = {}`).
    // We must then assign the constructor arg: `self.inventory = inventory`.
    // Note: the param name always matches the domain field name for Domain params.
    for p in &system.params {
        if matches!(
            p.kind,
            crate::frame_c::compiler::frame_ast::ParamKind::Domain
        ) {
            // Check that this param name actually matches a domain field
            let matching_field = system.domain.iter().find(|d| d.name == p.name);
            if matching_field.is_none() {
                continue; // Skip — no matching domain field
            }
            // Skip if the domain field's init already references this param
            // (avoids double assignment, which breaks final/readonly/const)
            let field_init = matching_field
                .unwrap()
                .initializer_text
                .as_deref()
                .unwrap_or("");
            if field_init.trim() == p.name {
                continue; // Domain init already assigns from this param
            }
            let assign_code = match syntax.language {
                TargetLanguage::Python3 | TargetLanguage::GDScript | TargetLanguage::Lua => {
                    format!("self.{} = {}", p.name, p.name)
                }
                TargetLanguage::Ruby => {
                    format!("@{} = {}", p.name, p.name)
                }
                TargetLanguage::Php => {
                    format!("$this->{} = ${};", p.name, p.name)
                }
                TargetLanguage::C => {
                    format!("self->{} = {};", p.name, p.name)
                }
                TargetLanguage::Go => {
                    format!("s.{} = {}", p.name, p.name)
                }
                TargetLanguage::Rust => {
                    // Rust handles this in the struct literal — skip here
                    continue;
                }
                TargetLanguage::Cpp => {
                    format!("this->{} = {};", p.name, p.name)
                }
                TargetLanguage::Swift => {
                    format!("self.{} = {}", p.name, p.name)
                }
                TargetLanguage::Erlang => {
                    continue; // Erlang handles domain differently
                }
                _ => {
                    // Java, C#, Kotlin, Dart, TypeScript, JavaScript
                    format!("this.{} = {};", p.name, p.name)
                }
            };
            body.push(CodegenNode::NativeBlock {
                code: assign_code,
                span: None,
            });
        }
    }

    // Rust state vars now live on compartment.state_context — no _sv_ field init needed.
    // State vars are initialized when compartments are created (in transition codegen).

    // Rust system header state/enter param fields: assign each
    // synthetic `__sys_<name>` field from the constructor parameter
    // of the same name. The Rust constructor emitter (rust.rs) sees
    // these as `self.field = value` assignments and folds them into
    // the struct literal.
    if matches!(syntax.language, TargetLanguage::Rust) {
        for p in &system.params {
            match p.kind {
                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                | crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), &format!("__sys_{}", p.name)),
                        CodegenNode::Ident(p.name.clone()),
                    ));
                }
                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
            }
        }
    }

    // Set initial state (first state in machine)
    // All languages now use the kernel/router/compartment pattern
    if let Some(ref machine) = system.machine {
        if let Some(first_state) = machine.states.first() {
            let compartment_class = format!("{}Compartment", system.name);
            let event_class = format!("{}FrameEvent", system.name);

            // HSM: Build ancestor chain if start state has a parent
            // We need to create compartments for all ancestors and link them via parent_compartment
            let has_hsm_parent = first_state.parent.is_some();

            // Build ancestor chain from root to leaf (reversed order for creation)
            let mut ancestor_chain: Vec<&crate::frame_c::compiler::frame_ast::StateAst> =
                Vec::new();
            if has_hsm_parent {
                let mut current_parent = first_state.parent.as_ref();
                while let Some(parent_name) = current_parent {
                    if let Some(parent_state) =
                        machine.states.iter().find(|s| &s.name == parent_name)
                    {
                        ancestor_chain.push(parent_state);
                        current_parent = parent_state.parent.as_ref();
                    } else {
                        break;
                    }
                }
                // Reverse so we start from root (topmost parent)
                ancestor_chain.reverse();
            }

            // Initialize __compartment with initial state
            match syntax.language {
                TargetLanguage::Rust => {
                    // Rust: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        // For Rust, use a block expression inside struct literal
                        // This creates parent chain and returns the child compartment
                        let mut block_expr = String::new();
                        block_expr.push_str("{\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "None".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            block_expr.push_str(&format!(
                                "let mut {} = {}Compartment::new(\"{}\");\n",
                                comp_var, system.name, ancestor.name
                            ));
                            block_expr.push_str(&format!(
                                "{}.parent_compartment = {};\n",
                                comp_var, prev_comp_var
                            ));
                            // state_context is auto-set by Compartment::new()
                            prev_comp_var = format!("Some(Box::new({}))", comp_var);
                        }
                        // Create the start state compartment with parent link
                        // state_context is auto-set by Compartment::new()
                        block_expr.push_str(&format!(
                            "let mut __child = {}Compartment::new(\"{}\");\n",
                            system.name, first_state.name
                        ));
                        block_expr.push_str(&format!(
                            "__child.parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        block_expr.push_str("__child\n}");

                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(block_expr),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::Ident("None".to_string()),
                        ));
                    } else {
                        // No HSM parent - simple compartment creation
                        // state_context is auto-set by Compartment::new()
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(format!(
                                "{}Compartment::new(\"{}\")",
                                system.name, first_state.name
                            )),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::Ident("None".to_string()),
                        ));
                    }
                }
                TargetLanguage::C => {
                    // C: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "NULL".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{}_Compartment* {} = {}_Compartment_new(\"{}\");\n",
                                system.name, comp_var, system.name, ancestor.name
                            ));
                            hsm_init_code.push_str(&format!(
                                "{}->parent_compartment = {};\n",
                                comp_var, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::C)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::C)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}_FrameDict_set({}->state_vars, \"{}\", (void*)(intptr_t){});\n",
                                    system.name, comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "self->__compartment = {}_Compartment_new(\"{}\");\n",
                            system.name, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "self->__compartment->parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("self->__next_compartment = NULL;");

                        // System state and enter params: bind into start state's
                        // state_args / enter_args. Mirrors the Python branch — every
                        // ParamKind::StateArg lands in `state_args[name]` and every
                        // ParamKind::EnterArg lands in `enter_args[name]`. The cast
                        // `(void*)(intptr_t)(name)` matches the rest of the C
                        // codegen's intptr-tagged void-pointer convention so the
                        // dispatch reader (also intptr-cast) round-trips correctly.
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n{}_FrameVec_push(self->__compartment->state_args, (void*)(intptr_t)({}));",
                                        system.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n{}_FrameVec_push(self->__compartment->enter_args, (void*)(intptr_t)({}));",
                                        system.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::Ident(format!(
                                "{}_Compartment_new(\"{}\")",
                                system.name, first_state.name
                            )),
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "{}_FrameVec_push(self->__compartment->state_args, (void*)(intptr_t)({}));",
                                        system.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "{}_FrameVec_push(self->__compartment->enter_args, (void*)(intptr_t)({}));",
                                        system.name, p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Python3 => {
                    // Python: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("# HSM: Create parent compartment chain\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "None".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} = {}(\"{}\", parent_compartment={})\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Python3)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Python3)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "self.__compartment = {}(\"{}\", parent_compartment={})\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("self.__next_compartment = None");

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));
                        // System state and enter params: bind into start state's
                        // state_args / enter_args. The constructor receives params
                        // via system.params; each StateArg/EnterArg entry writes
                        // into the corresponding compartment dict so the state's
                        // dispatch (state_args) and enter handler (enter_args) can
                        // read them. Domain params are handled by the domain field
                        // initializer loop above and aren't touched here.
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.state_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.enter_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    // TypeScript: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        // Create compartments from root to leaf
                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "const {} = new {}(\"{}\", {});\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, syntax.language)
                                } else {
                                    state_var_init_value(&var.var_type, syntax.language)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = new {}(\"{}\", {});\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args.push({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args.push({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.state_args.push({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.enter_args.push({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Php => {
                    // PHP: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("$__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} = new {}(\"{}\", {});\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Php)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Php)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}->state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "$this->__compartment = new {}(\"{}\", {});\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("$this->__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n$this->__compartment->state_args[] = ${};",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n$this->__compartment->enter_args[] = ${};",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "$this->__compartment->state_args[] = ${};",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "$this->__compartment->enter_args[] = ${};",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Ruby => {
                    // Ruby: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("# HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "nil".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} = {}.new(\"{}\", {})\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Ruby)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Ruby)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "@__compartment = {}.new(\"{}\", {})\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("@__next_compartment = nil");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n@__compartment.state_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\n@__compartment.enter_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "@__compartment.state_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "@__compartment.enter_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Cpp => {
                    // C++: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_expr = "nullptr".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "auto {} = std::make_shared<{}>(\"{}\");\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_expr != "nullptr" {
                                hsm_init_code.push_str(&format!(
                                    "{}->parent_compartment = std::move({});\n",
                                    comp_var, prev_comp_expr
                                ));
                            }
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    cpp_wrap_any_arg(&expression_to_string(
                                        init,
                                        TargetLanguage::Cpp,
                                    ))
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Cpp)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}->state_vars[\"{}\"] = std::any({});\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_expr = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "__compartment = std::make_shared<{}>(\"{}\");\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "__compartment->parent_compartment = std::move({});\n",
                            prev_comp_expr
                        ));

                        // System state and enter params: positional append into
                        // the start state's state_args / enter_args vectors.
                        // Values are wrapped in std::any so the dispatch
                        // reader (which uses std::any_cast<Type>) round-trips
                        // them correctly. cpp_wrap_any_arg promotes string
                        // literals to std::string so any_cast<std::string>
                        // works on the read side.
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    hsm_init_code.push_str(&format!(
                                        "__compartment->state_args.push_back(std::any({}));\n",
                                        wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    hsm_init_code.push_str(&format!(
                                        "__compartment->enter_args.push_back(std::any({}));\n",
                                        wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = std::make_shared<{}>(\"{}\");",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    compartment_inits.push(format!(
                                        "__compartment->state_args.push_back(std::any({}));",
                                        wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    let wrapped = cpp_wrap_any_arg(&p.name);
                                    compartment_inits.push(format!(
                                        "__compartment->enter_args.push_back(std::any({}));",
                                        wrapped
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Java => {
                    // Java: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} {} = new {}(\"{}\");\n",
                                compartment_class, comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "null" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {};\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Java)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Java)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars.put(\"{}\", {});\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = new {}(\"{}\");\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "this.__compartment.parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = new {}(\"{}\");\n__next_compartment = null;",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Kotlin => {
                    // Kotlin: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "val {} = {}(\"{}\")\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "null" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {}\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Kotlin)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Kotlin)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}[\"{}\"] = {}\n",
                                    format!("{}.state_vars", comp_var),
                                    var.name,
                                    init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = {}(\"{}\")\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "this.__compartment.parent_compartment = {}\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args.add({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args.add({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = {}(\"{}\")\n__next_compartment = null",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args.add({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args.add({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Swift => {
                    // Swift: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "nil".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "let {} = {}(state: \"{}\")\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "nil" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {}\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Swift)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Swift)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "self.__compartment = {}(state: \"{}\")\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "self.__compartment.parent_compartment = {}\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("self.__next_compartment = nil");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.state_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.enter_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = {}(state: \"{}\")\n__next_compartment = nil",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::CSharp => {
                    // C#: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "var {} = new {}(\"{}\");\n",
                                comp_var, compartment_class, ancestor.name
                            ));
                            if prev_comp_var != "null" {
                                hsm_init_code.push_str(&format!(
                                    "{}.parent_compartment = {};\n",
                                    comp_var, prev_comp_var
                                ));
                            }
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::CSharp)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::CSharp)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = new {}(\"{}\");\n",
                            compartment_class, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "this.__compartment.parent_compartment = {};\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args.Add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args.Add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "__compartment = new {}(\"{}\");\n__next_compartment = null;",
                                compartment_class, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.state_args.Add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "__compartment.enter_args.Add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Go => {
                    // Go: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "nil".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "{} := new{}Compartment(\"{}\")\n",
                                comp_var, system.name, ancestor.name
                            ));
                            hsm_init_code.push_str(&format!(
                                "{}.parentCompartment = {}\n",
                                comp_var, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::Go)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::Go)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.stateVars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "s.__compartment = new{}Compartment(\"{}\")\n",
                            system.name, first_state.name
                        ));
                        hsm_init_code.push_str(&format!(
                            "s.__compartment.parentCompartment = {}\n",
                            prev_comp_var
                        ));
                        hsm_init_code.push_str("s.__next_compartment = nil");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\ns.__compartment.stateArgs = append(s.__compartment.stateArgs, {})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\ns.__compartment.enterArgs = append(s.__compartment.enterArgs, {})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::NativeBlock {
                            code: format!(
                                "s.__compartment = new{}Compartment(\"{}\")\ns.__next_compartment = nil",
                                system.name, first_state.name
                            ),
                            span: None,
                        });

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "s.__compartment.stateArgs = append(s.__compartment.stateArgs, {})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "s.__compartment.enterArgs = append(s.__compartment.enterArgs, {})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Dart => {
                    // Dart: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("// HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "final {} = {}(\"{}\", {});\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, syntax.language)
                                } else {
                                    state_var_init_value(&var.var_type, syntax.language)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {};\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        // Create the start state compartment with parent link
                        hsm_init_code.push_str(&format!(
                            "this.__compartment = {}(\"{}\", {});\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("this.__next_compartment = null;");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.state_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nthis.__compartment.enter_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        // No HSM parent - simple compartment creation
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.state_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "this.__compartment.enter_args.add({});",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::GDScript => {
                    // GDScript: Create compartment chain for HSM if start state has parent
                    if !ancestor_chain.is_empty() {
                        let mut hsm_init_code = String::new();
                        hsm_init_code.push_str("# HSM: Create parent compartment chain\n");

                        let mut prev_comp_var = "null".to_string();
                        for (i, ancestor) in ancestor_chain.iter().enumerate() {
                            let comp_var = format!("__parent_comp_{}", i);
                            hsm_init_code.push_str(&format!(
                                "var {} = {}.new(\"{}\", {})\n",
                                comp_var, compartment_class, ancestor.name, prev_comp_var
                            ));
                            // Initialize state vars for this ancestor
                            for var in &ancestor.state_vars {
                                let init_val = if let Some(ref init) = var.init {
                                    expression_to_string(init, TargetLanguage::GDScript)
                                } else {
                                    state_var_init_value(&var.var_type, TargetLanguage::GDScript)
                                };
                                hsm_init_code.push_str(&format!(
                                    "{}.state_vars[\"{}\"] = {}\n",
                                    comp_var, var.name, init_val
                                ));
                            }
                            prev_comp_var = comp_var;
                        }
                        hsm_init_code.push_str(&format!(
                            "self.__compartment = {}.new(\"{}\", {})\n",
                            compartment_class, first_state.name, prev_comp_var
                        ));
                        hsm_init_code.push_str("self.__next_compartment = null");

                        // System state and enter params (HSM path).
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.state_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    hsm_init_code.push_str(&format!(
                                        "\nself.__compartment.enter_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }

                        body.push(CodegenNode::NativeBlock {
                            code: hsm_init_code,
                            span: None,
                        });
                    } else {
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                            CodegenNode::New {
                                class: compartment_class.clone(),
                                args: vec![CodegenNode::string(&first_state.name)],
                            },
                        ));
                        body.push(CodegenNode::assign(
                            CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                            CodegenNode::null(),
                        ));

                        // System state and enter params (non-HSM path).
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.state_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "self.__compartment.enter_args.append({})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                // Dynamic languages and remaining: New expression
                // (Lua, Erlang, Kotlin — all routed here)
                TargetLanguage::Python3
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Php
                | TargetLanguage::Ruby
                | TargetLanguage::Erlang
                | TargetLanguage::Kotlin
                | TargetLanguage::Lua => {
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), "__compartment"),
                        CodegenNode::New {
                            class: compartment_class.clone(),
                            args: vec![CodegenNode::string(&first_state.name)],
                        },
                    ));
                    body.push(CodegenNode::assign(
                        CodegenNode::field(CodegenNode::self_ref(), "__next_compartment"),
                        CodegenNode::null(),
                    ));

                    // System state and enter params (Lua path — Lua's
                    // compartment has state_args/enter_args dicts the same
                    // way Python does). Erlang and Kotlin have their own
                    // dispatch routes upstream (Erlang via gen_statem,
                    // Kotlin via the explicit branch above), so for these
                    // the population is a no-op.
                    if matches!(syntax.language, TargetLanguage::Lua) {
                        let mut compartment_inits: Vec<String> = Vec::new();
                        for p in &system.params {
                            match p.kind {
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                    compartment_inits.push(format!(
                                        "table.insert(self.__compartment.state_args, {})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                    compartment_inits.push(format!(
                                        "table.insert(self.__compartment.enter_args, {})",
                                        p.name
                                    ));
                                }
                                crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                            }
                        }
                        if !compartment_inits.is_empty() {
                            body.push(CodegenNode::NativeBlock {
                                code: compartment_inits.join("\n"),
                                span: None,
                            });
                        }
                    }
                }
                TargetLanguage::Graphviz => unreachable!(),
            }

            // Send $> (enter) event via __kernel - language-specific.
            // The enter_args of the start state's compartment carry any
            // header-declared enter params; pass them through so the
            // start state's $>(name: type) handler can read them.
            //
            // C optimization: if the start state has no declared enter
            // params, pass NULL for the event's `_parameters` (the
            // dispatch generates no `FrameDict_get` calls in that case).
            let start_state_has_enter_params = first_state
                .enter
                .as_ref()
                .map(|e| !e.params.is_empty())
                .unwrap_or(false);
            let init_event_code = match syntax.language {
                TargetLanguage::Python3 => format!(
                    r#"__frame_event = {}("$>", self.__compartment.enter_args)
self.__kernel(__frame_event)"#,
                    event_class
                ),
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!(
                    r#"const __frame_event = new {}("$>", this.__compartment.enter_args);
this.__kernel(__frame_event);"#,
                    event_class
                ),
                TargetLanguage::Rust => format!(
                    r#"let __frame_event = {}::new("$>");
let __ctx = {}FrameContext::new(__frame_event, None);
self._context_stack.push(__ctx);
self.__kernel();
self._context_stack.pop();"#,
                    event_class, system.name
                ),
                TargetLanguage::C => {
                    // Pass the start state's enter_args dict as the event
                    // _parameters so the start state's `$>(name: type)`
                    // enter handler can read header-declared enter params
                    // by name. For systems WITHOUT declared enter params,
                    // pass NULL — the dispatch generates no FrameDict_get
                    // calls so the NULL is never dereferenced, and the
                    // generated code is one indirection lighter at start
                    // time. Behavior is unchanged for systems with enter
                    // params.
                    let parameters_arg = if start_state_has_enter_params {
                        "self->__compartment->enter_args"
                    } else {
                        "NULL"
                    };
                    format!(
                        r#"{}_FrameEvent* __frame_event = {}_FrameEvent_new("$>", {}, 0);
{}_kernel(self, __frame_event);
{}_FrameEvent_destroy(__frame_event);"#,
                        system.name, system.name, parameters_arg, system.name, system.name
                    )
                },
                TargetLanguage::Cpp => format!(
                    // Class-static `__skipInitialEnter` — see Swift comment.
                    r#"if (!__skipInitialEnter) {{
    {0}FrameEvent __frame_event("$>");
    {0}FrameContext __ctx(std::move(__frame_event));
    _context_stack.push_back(std::move(__ctx));
    __kernel(_context_stack.back()._event);
    _context_stack.pop_back();
}}"#,
                    system.name
                ),
                TargetLanguage::Java => format!(
                    // `__skipInitialEnter` is a class-level static flag that
                    // restore_state sets true before invoking the ctor so
                    // the initial-state $>() handler does NOT fire on a
                    // restored instance. See Swift comment for rationale.
                    r#"if (!__skipInitialEnter) {{
    {0}FrameEvent __frame_event = new {0}FrameEvent("$>");
    {0}FrameContext __ctx = new {0}FrameContext(__frame_event, null);
    _context_stack.add(__ctx);
    __kernel(_context_stack.get(_context_stack.size() - 1)._event);
    _context_stack.remove(_context_stack.size() - 1);
}}"#,
                    system.name
                ),
                TargetLanguage::CSharp => format!(
                    r#"{}FrameEvent __frame_event = new {}FrameEvent("$>");
{}FrameContext __ctx = new {}FrameContext(__frame_event, null);
_context_stack.Add(__ctx);
__kernel(_context_stack[_context_stack.Count - 1]._event);
_context_stack.RemoveAt(_context_stack.Count - 1);"#,
                    system.name, system.name, system.name, system.name
                ),
                TargetLanguage::Go => format!(
                    r#"__frame_event := {}FrameEvent{{_message: "$>", _parameters: nil}}
__ctx := {}FrameContext{{_event: __frame_event, _data: make(map[string]any)}}
s._context_stack = append(s._context_stack, __ctx)
s.__kernel(&s._context_stack[len(s._context_stack)-1]._event)
s._context_stack = s._context_stack[:len(s._context_stack)-1]"#,
                    system.name, system.name
                ),
                TargetLanguage::Kotlin => format!(
                    // Companion-object static flag; see Swift comment for
                    // rationale.
                    r#"if (!__skipInitialEnter) {{
    val __frame_event = {0}FrameEvent("$>")
    val __ctx = {0}FrameContext(__frame_event, null)
    _context_stack.add(__ctx)
    __kernel(_context_stack[_context_stack.size - 1]._event)
    _context_stack.removeAt(_context_stack.size - 1)
}}"#,
                    system.name
                ),
                TargetLanguage::Swift => format!(
                    // `__skipInitialEnter` is a class-level static flag that
                    // restoreState sets to true before invoking init() so the
                    // initial-state `$>()` enter handler does not fire on a
                    // restored instance. Without this, every Swift @@persist
                    // system would leak enter side effects from state `$A`
                    // on top of its restored compartment. The flag is always
                    // emitted (false by default for non-persist systems too)
                    // so the class has one init signature regardless of
                    // whether @@persist is declared — simplifies call sites.
                    r#"if !{0}.__skipInitialEnter {{
    let __frame_event = {0}FrameEvent(message: "$>")
    let __ctx = {0}FrameContext(event: __frame_event)
    _context_stack.append(__ctx)
    __kernel(_context_stack[_context_stack.count - 1]._event)
    _context_stack.removeLast()
}}"#,
                    system.name
                ),
                TargetLanguage::Php => format!(
                    r#"$__frame_event = new {}("$>", $this->__compartment->enter_args);
$__ctx = new {}FrameContext($__frame_event, null);
$this->_context_stack[] = $__ctx;
$this->__kernel($this->_context_stack[count($this->_context_stack) - 1]->_event);
array_pop($this->_context_stack);"#,
                    event_class, system.name
                ),
                TargetLanguage::Ruby => format!(
                    r#"__frame_event = {}.new("$>", @__compartment.enter_args)
__ctx = {}FrameContext.new(__frame_event, nil)
@_context_stack.push(__ctx)
__kernel(@_context_stack[@_context_stack.length - 1]._event)
@_context_stack.pop"#,
                    event_class, system.name
                ),
                TargetLanguage::Lua => format!(
                    r#"local __frame_event = {}.new("$>", self.__compartment.enter_args)
local __ctx = {}FrameContext.new(__frame_event, nil)
self._context_stack[#self._context_stack + 1] = __ctx
self:__kernel(self._context_stack[#self._context_stack]._event)
self._context_stack[#self._context_stack] = nil"#,
                    event_class, system.name
                ),
                TargetLanguage::Dart => format!(
                    "final __frame_event = {}(\"\\$>\", this.__compartment.enter_args);\nfinal __ctx = {}FrameContext(__frame_event, null);\n_context_stack.add(__ctx);\n__kernel(_context_stack[_context_stack.length - 1].event);\n_context_stack.removeLast();",
                    event_class, system.name
                ),
                TargetLanguage::GDScript => format!(
                    // Class-static `__skipInitialEnter` — see Swift comment.
                    r#"if not __skipInitialEnter:
    var __frame_event = {0}.new("$>", self.__compartment.enter_args)
    var __ctx = {1}FrameContext.new(__frame_event, null)
    self._context_stack.append(__ctx)
    self.__kernel(self._context_stack[self._context_stack.size() - 1].event)
    self._context_stack.pop_back()"#,
                    event_class, system.name
                ),
                TargetLanguage::Erlang => String::new(), // gen_statem: handled natively by erlang_system.rs
                TargetLanguage::Graphviz => unreachable!(),
            };
            body.push(CodegenNode::NativeBlock {
                code: init_event_code,
                span: None,
            });
        }
    }

    // Params from system params
    let params: Vec<Param> = system
        .params
        .iter()
        .map(|p| {
            let type_str = type_to_string(&p.param_type);
            let mut param = Param::new(&p.name).with_type(&type_str);
            if let Some(ref def) = p.default {
                param = param.with_default(CodegenNode::Ident(def.clone()));
            }
            param
        })
        .collect();

    // For C++: emit const fields via member initializer list.
    // The C++ backend formats super_call as ` : {expr}` before the body.
    let super_call = if !cpp_init_list.is_empty() && matches!(syntax.language, TargetLanguage::Cpp)
    {
        Some(Box::new(CodegenNode::Ident(cpp_init_list.join(", "))))
    } else {
        None
    };

    CodegenNode::Constructor {
        params,
        body,
        super_call,
    }
}

/// Generate Frame machinery methods (__kernel, __router, __transition)
/// for all target languages.
pub(crate) fn generate_frame_machinery(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
    lang: TargetLanguage,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let compartment_class = format!("{}Compartment", system.name);
    let event_class = format!("{}FrameEvent", system.name);

    match lang {
        TargetLanguage::Python3 => methods.extend(generate_python3_machinery(&event_class)),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => methods.extend(
            generate_javascript_machinery(lang, &event_class, &compartment_class),
        ),
        TargetLanguage::Php => methods.extend(generate_php_machinery(&event_class)),
        TargetLanguage::Ruby => methods.extend(generate_ruby_machinery(&event_class)),
        TargetLanguage::Rust => methods.extend(super::rust_system::generate_rust_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::C => methods.extend(generate_c_machinery(system)),
        TargetLanguage::Cpp => methods.extend(generate_cpp_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::Java => methods.extend(generate_java_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::Kotlin => methods.extend(generate_kotlin_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::Swift => methods.extend(generate_swift_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::CSharp => methods.extend(generate_csharp_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::Go => methods.extend(generate_go_machinery(system)),
        TargetLanguage::Erlang => {
            // gen_statem: kernel/router/transition are built into OTP — no custom methods needed
        }
        TargetLanguage::Lua => methods.extend(generate_lua_machinery(&event_class)),
        TargetLanguage::Dart => methods.extend(generate_dart_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::GDScript => methods.extend(generate_gdscript_machinery(&event_class)),
        TargetLanguage::Graphviz => unreachable!(),
    }

    // Generate __push_transition method for Rust when there's a machine
    // Uses mem::replace to move the current compartment to the stack (no clone)
    if matches!(lang, TargetLanguage::Rust) && system.machine.is_some() {
        methods.push(super::rust_system::generate_rust_push_transition(system));
    }

    methods
}

// =====================================================================
// Per-language frame-machinery helpers
// =====================================================================
//
// Each `generate_<lang>_machinery` returns the kernel/router/transition
// methods for one target language. They were previously inlined as
// match arms inside `generate_frame_machinery`, which had grown to
// ~1374 lines (one giant match across 17 backends). Splitting them
// out gives each language an isolated, navigable function while
// keeping codegen entirely in this file.

fn generate_python3_machinery(event_class: &str) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    // __kernel method - the main event processing loop
    // Routes event to current state, then processes any pending transition
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"# Route event to current state
self.__router(__e)
# Process any pending transition
while self.__next_compartment is not None:
    next_compartment = self.__next_compartment
    self.__next_compartment = None
    # Exit current state
    exit_event = {}("<$", self.__compartment.exit_args)
    self.__router(exit_event)
    # Switch to new compartment
    self.__compartment = next_compartment
    # Enter new state (or forward event)
    if next_compartment.forward_event is None:
        enter_event = {}("$>", self.__compartment.enter_args)
        self.__router(enter_event)
    else:
        # Forward event to new state
        forward_event = next_compartment.forward_event
        next_compartment.forward_event = None
        if forward_event._message == "$>":
            # Forwarding enter event - just send it
            self.__router(forward_event)
        else:
            # Forwarding other event - send $> first, then forward
            enter_event = {}("$>", self.__compartment.enter_args)
            self.__router(enter_event)
            self.__router(forward_event)
    # Mark all stacked contexts as transitioned
    for ctx in self._context_stack:
        ctx._transitioned = True"#,
                event_class, event_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router method - dispatches events to state methods
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"state_name = self.__compartment.state
handler_name = f"_state_{state_name}"
handler = getattr(self, handler_name, None)
if handler:
    handler(__e)"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition method - caches next compartment (deferred transition)
    // Does NOT execute transition - __kernel does that after handler returns
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "self.__next_compartment = next_compartment".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_javascript_machinery(
    lang: TargetLanguage,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    // __kernel method - the main event processing loop
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"// Route event to current state
this.__router(__e);
// Process any pending transition
while (this.__next_compartment !== null) {{
    const next_compartment = this.__next_compartment;
    this.__next_compartment = null;
    // Exit current state
    const exit_event = new {}("<$", this.__compartment.exit_args);
    this.__router(exit_event);
    // Switch to new compartment
    this.__compartment = next_compartment;
    // Enter new state (or forward event)
    if (next_compartment.forward_event === null) {{
        const enter_event = new {}("$>", this.__compartment.enter_args);
        this.__router(enter_event);
    }} else {{
        // Forward event to new state
        const forward_event = next_compartment.forward_event;
        next_compartment.forward_event = null;
        if (forward_event._message === "$>") {{
            // Forwarding enter event - just send it
            this.__router(forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            const enter_event = new {}("$>", this.__compartment.enter_args);
            this.__router(enter_event);
            this.__router(forward_event);
        }}
    }}
    // Mark all stacked contexts as transitioned
    for (const ctx of this._context_stack) {{
        ctx._transitioned = true;
    }}
}}"#,
                event_class, event_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router method - dispatches events to state methods
    // TypeScript uses `(this as any)` for dynamic dispatch; JS uses `this` directly
    let router_code = if matches!(lang, TargetLanguage::TypeScript) {
        r#"const state_name = this.__compartment.state;
const handler_name = `_state_${state_name}`;
const handler = (this as any)[handler_name];
if (handler) {
    handler.call(this, __e);
}"#
        .to_string()
    } else {
        r#"const state_name = this.__compartment.state;
const handler_name = `_state_${state_name}`;
const handler = this[handler_name];
if (handler) {
    handler.call(this, __e);
}"#
        .to_string()
    };
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition method - caches next compartment (deferred transition)
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment").with_type(compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "this.__next_compartment = next_compartment;".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_php_machinery(event_class: &str) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    // PHP: __kernel method - the main event processing loop
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"// Route event to current state
$this->__router($__e);
// Process any pending transition
while ($this->__next_compartment !== null) {{
    $next_compartment = $this->__next_compartment;
    $this->__next_compartment = null;
    // Exit current state
    $exit_event = new {}("<$", $this->__compartment->exit_args);
    $this->__router($exit_event);
    // Switch to new compartment
    $this->__compartment = $next_compartment;
    // Enter new state (or forward event)
    if ($next_compartment->forward_event === null) {{
        $enter_event = new {}("$>", $this->__compartment->enter_args);
        $this->__router($enter_event);
    }} else {{
        // Forward event to new state
        $forward_event = $next_compartment->forward_event;
        $next_compartment->forward_event = null;
        if ($forward_event->_message === "$>") {{
            // Forwarding enter event - just send it
            $this->__router($forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            $enter_event = new {}("$>", $this->__compartment->enter_args);
            $this->__router($enter_event);
            $this->__router($forward_event);
        }}
    }}
    // Mark all stacked contexts as transitioned
    foreach ($this->_context_stack as $ctx) {{
        $ctx->_transitioned = true;
    }}
}}"#,
                event_class, event_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // PHP: __router method - dispatches events to state methods
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"$state_name = $this->__compartment->state;
$handler_name = "_state_" . $state_name;
if (method_exists($this, $handler_name)) {
    $this->$handler_name($__e);
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // PHP: __transition method - caches next compartment (deferred transition)
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "$this->__next_compartment = $next_compartment;".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_ruby_machinery(event_class: &str) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    methods.push(CodegenNode::Method { name: "__kernel".to_string(), params: vec![Param::new("__e")], return_type: None, body: vec![CodegenNode::NativeBlock { code: format!("# Route event to current state\n__router(__e)\nwhile @__next_compartment != nil\n    next_compartment = @__next_compartment\n    @__next_compartment = nil\n    exit_event = {0}.new(\"<$\", @__compartment.exit_args)\n    __router(exit_event)\n    @__compartment = next_compartment\n    if next_compartment.forward_event == nil\n        enter_event = {0}.new(\"$>\", @__compartment.enter_args)\n        __router(enter_event)\n    else\n        forward_event = next_compartment.forward_event\n        next_compartment.forward_event = nil\n        if forward_event._message == \"$>\"\n            __router(forward_event)\n        else\n            enter_event = {0}.new(\"$>\", @__compartment.enter_args)\n            __router(enter_event)\n            __router(forward_event)\n        end\n    end\n    # Mark all stacked contexts as transitioned\n    @_context_stack.each {{ |ctx| ctx._transitioned = true }}\nend", event_class), span: None }], is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![] });
    methods.push(CodegenNode::Method { name: "__router".to_string(), params: vec![Param::new("__e")], return_type: None, body: vec![CodegenNode::NativeBlock { code: "state_name = @__compartment.state\nhandler_name = \"_state_#{state_name}\"\nif respond_to?(handler_name, true)\n    send(handler_name, __e)\nend".to_string(), span: None }], is_async: false, is_static: false, visibility: Visibility::Private, decorators: vec![] });
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "@__next_compartment = next_compartment".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });
    methods
}

fn generate_c_machinery(system: &SystemAst) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    // C: Full kernel/router/transition pattern with string comparison dispatch
    let sys = &system.name;

    // __kernel method - the main event processing loop
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"// Route event to current state
{sys}_router(self, __e);
// Process any pending transition
while (self->__next_compartment != NULL) {{
    {sys}_Compartment* next_compartment = self->__next_compartment;
    self->__next_compartment = NULL;
    // Exit current state (with exit_args from current compartment)
    {sys}_FrameEvent* exit_event = {sys}_FrameEvent_new("<$", self->__compartment->exit_args, 0);
    {sys}_router(self, exit_event);
    {sys}_FrameEvent_destroy(exit_event);
    // Switch to new compartment
    {sys}_Compartment_unref(self->__compartment);
    self->__compartment = next_compartment;
    // Enter new state (or forward event)
    if (next_compartment->forward_event == NULL) {{
        {sys}_FrameEvent* enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
        {sys}_router(self, enter_event);
        {sys}_FrameEvent_destroy(enter_event);
    }} else {{
        // Forward event to new state
        // Note: forward_event is a borrowed pointer to the caller's __e, do NOT destroy it
        {sys}_FrameEvent* forward_event = next_compartment->forward_event;
        next_compartment->forward_event = NULL;
        if (strcmp(forward_event->_message, "$>") == 0) {{
            // Forwarding enter event - just send it
            {sys}_router(self, forward_event);
        }} else {{
            // Forwarding other event - send $> first, then forward
            {sys}_FrameEvent* enter_event = {sys}_FrameEvent_new("$>", self->__compartment->enter_args, 0);
            {sys}_router(self, enter_event);
            {sys}_FrameEvent_destroy(enter_event);
            {sys}_router(self, forward_event);
        }}
        // Do NOT destroy forward_event - it's owned by the interface method caller
    }}
    // Mark all stacked contexts as transitioned
    for (int __i = 0; __i < self->_context_stack->size; __i++) {{
        (({sys}_FrameContext*)self->_context_stack->items[__i])->_transitioned = 1;
    }}
}}"#,
                sys = sys
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router method - dispatches events to state handler functions
    let router_code = generate_c_router_dispatch(system);
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition method - caches next compartment (deferred transition)
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment").with_type(&format!("{}_Compartment*", sys))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "self->__next_compartment = next_compartment;".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // destroy method - cleanup system resources
    methods.push(CodegenNode::Method {
        name: "destroy".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"// Unref current compartment (may free if not on stack)
if (self->__compartment) {sys}_Compartment_unref(self->__compartment);
if (self->__next_compartment) {sys}_Compartment_unref(self->__next_compartment);
// Unref all state stack entries
if (self->_state_stack) {{
    for (int __i = 0; __i < self->_state_stack->size; __i++) {{
        {sys}_Compartment_unref(({sys}_Compartment*)self->_state_stack->items[__i]);
    }}
    {sys}_FrameVec_destroy(self->_state_stack);
}}
if (self->_context_stack) {sys}_FrameVec_destroy(self->_context_stack);
free(self);"#,
                sys = sys
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Public,
        decorators: vec![],
    });

    methods
}

fn generate_cpp_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // Class-static flag used by restore_state to suppress the initial-
    // state $>() dispatch on a restored instance. Inline initialization
    // requires C++17+. See Swift comment for rationale — same class of
    // bug across JVM + C++ + GDScript + Objective-C-style dynamic langs.
    methods.push(CodegenNode::NativeBlock {
        code: "inline static bool __skipInitialEnter = false;".to_string(),
        span: None,
    });

    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    // __kernel
    let mut kernel_code = String::new();
    kernel_code.push_str("__router(__e);\n");
    kernel_code.push_str("while (__next_compartment) {\n");
    kernel_code.push_str("    auto next_compartment = std::move(__next_compartment);\n");
    kernel_code.push_str(&format!("    {} exit_event(\"<$\");\n", event_class));
    kernel_code.push_str("    __router(exit_event);\n");
    kernel_code.push_str("    __compartment = std::move(next_compartment);\n");
    kernel_code.push_str("    if (!__compartment->forward_event) {\n");
    kernel_code.push_str(&format!("        {} enter_event(\"$>\");\n", event_class));
    kernel_code.push_str("        __router(enter_event);\n");
    kernel_code.push_str("    } else {\n");
    kernel_code.push_str("        auto forward_event = std::move(__compartment->forward_event);\n");
    kernel_code.push_str("        if (forward_event->_message == \"$>\") {\n");
    kernel_code.push_str("            __router(*forward_event);\n");
    kernel_code.push_str("        } else {\n");
    kernel_code.push_str(&format!(
        "            {} enter_event(\"$>\");\n",
        event_class
    ));
    kernel_code.push_str("            __router(enter_event);\n");
    kernel_code.push_str("            __router(*forward_event);\n");
    kernel_code.push_str("        }\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("    // Mark all stacked contexts as transitioned\n");
    kernel_code.push_str("    for (auto& ctx : _context_stack) {\n");
    kernel_code.push_str("        ctx._transitioned = true;\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("}");

    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: kernel_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router
    let mut router_code = String::new();
    router_code.push_str("const auto& state_name = __compartment->state;\n");
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        router_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
        router_code.push_str(&format!("    _state_{}(__e);\n", state));
    }
    if !states.is_empty() {
        router_code.push_str("}");
    }

    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![
            Param::new("next").with_type(&format!("std::shared_ptr<{}>", compartment_class))
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__next_compartment = next;".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_java_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // Class-static flag gating the ctor's ENTER dispatch. restore_state
    // sets it true, calls the ctor, resets it false, then overwrites the
    // compartment. See Swift comment in generate_swift_machinery for the
    // rationale — same class of bug across JVM + C++ + GDScript.
    methods.push(CodegenNode::NativeBlock {
        code: "private static boolean __skipInitialEnter = false;".to_string(),
        span: None,
    });

    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    // __kernel
    let mut kernel_code = String::new();
    kernel_code.push_str("__router(__e);\n");
    kernel_code.push_str("while (__next_compartment != null) {\n");
    kernel_code.push_str(&format!(
        "    {} next_compartment = __next_compartment;\n",
        compartment_class
    ));
    kernel_code.push_str("    __next_compartment = null;\n");
    kernel_code.push_str(&format!(
        "    {} exit_event = new {}(\"<$\");\n",
        event_class, event_class
    ));
    kernel_code.push_str("    __router(exit_event);\n");
    kernel_code.push_str("    __compartment = next_compartment;\n");
    kernel_code.push_str("    if (__compartment.forward_event == null) {\n");
    kernel_code.push_str(&format!(
        "        {} enter_event = new {}(\"$>\");\n",
        event_class, event_class
    ));
    kernel_code.push_str("        __router(enter_event);\n");
    kernel_code.push_str("    } else {\n");
    kernel_code.push_str(&format!(
        "        {} forward_event = __compartment.forward_event;\n",
        event_class
    ));
    kernel_code.push_str("        __compartment.forward_event = null;\n");
    kernel_code.push_str("        if (forward_event._message.equals(\"$>\")) {\n");
    kernel_code.push_str("            __router(forward_event);\n");
    kernel_code.push_str("        } else {\n");
    kernel_code.push_str(&format!(
        "            {} enter_event = new {}(\"$>\");\n",
        event_class, event_class
    ));
    kernel_code.push_str("            __router(enter_event);\n");
    kernel_code.push_str("            __router(forward_event);\n");
    kernel_code.push_str("        }\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("    // Mark all stacked contexts as transitioned\n");
    kernel_code.push_str(&format!(
        "    for ({}FrameContext ctx : _context_stack) {{\n",
        system.name
    ));
    kernel_code.push_str("        ctx._transitioned = true;\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("}");

    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: kernel_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router - if/else if chain with .equals()
    let mut router_code = String::new();
    router_code.push_str("String state_name = __compartment.state;\n");
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        router_code.push_str(&format!(
            "{} (state_name.equals(\"{}\")) {{\n",
            prefix, state
        ));
        router_code.push_str(&format!("    _state_{}(__e);\n", state));
    }
    if !states.is_empty() {
        router_code.push_str("}");
    }

    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next").with_type(compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__next_compartment = next;".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_kotlin_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // Companion-object flag gating the ctor's ENTER dispatch. restore_state
    // sets it true, calls Canary(), resets false. The raw `@JvmStatic var`
    // declaration below is emitted inside the class's sole companion
    // object — the Kotlin backend recognizes NativeBlock items in the
    // methods vec and places them alongside static methods so only one
    // companion object is produced (Kotlin permits at most one per class).
    methods.push(CodegenNode::NativeBlock {
        code: "@JvmStatic var __skipInitialEnter: Boolean = false".to_string(),
        span: None,
    });

    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    // __kernel — Kotlin: no semicolons, no `new`, `==` instead of `.equals()`
    let mut kernel_code = String::new();
    kernel_code.push_str("__router(__e)\n");
    kernel_code.push_str("while (__next_compartment != null) {\n");
    kernel_code.push_str("    val next_compartment = __next_compartment!!\n");
    kernel_code.push_str("    __next_compartment = null\n");
    kernel_code.push_str(&format!("    val exit_event = {}(\"<$\")\n", event_class));
    kernel_code.push_str("    __router(exit_event)\n");
    kernel_code.push_str("    __compartment = next_compartment\n");
    kernel_code.push_str("    if (__compartment.forward_event == null) {\n");
    kernel_code.push_str(&format!(
        "        val enter_event = {}(\"$>\")\n",
        event_class
    ));
    kernel_code.push_str("        __router(enter_event)\n");
    kernel_code.push_str("    } else {\n");
    kernel_code.push_str("        val forward_event = __compartment.forward_event!!\n");
    kernel_code.push_str("        __compartment.forward_event = null\n");
    kernel_code.push_str("        if (forward_event._message == \"$>\") {\n");
    kernel_code.push_str("            __router(forward_event)\n");
    kernel_code.push_str("        } else {\n");
    kernel_code.push_str(&format!(
        "            val enter_event = {}(\"$>\")\n",
        event_class
    ));
    kernel_code.push_str("            __router(enter_event)\n");
    kernel_code.push_str("            __router(forward_event)\n");
    kernel_code.push_str("        }\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("    // Mark all stacked contexts as transitioned\n");
    kernel_code.push_str("    for (ctx in _context_stack) {\n");
    kernel_code.push_str("        ctx._transitioned = true\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("}");

    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: kernel_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — Kotlin: when expression with == comparison
    let mut router_code = String::new();
    router_code.push_str("val state_name = __compartment.state\n");
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        router_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
        router_code.push_str(&format!("    _state_{}(__e)\n", state));
    }
    if !states.is_empty() {
        router_code.push_str("}");
    }

    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next").with_type(compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__next_compartment = next".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_swift_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // Class-level static flag: restoreState sets this before calling init
    // to suppress the initial-state $>() dispatch. Default false means all
    // non-restore instantiations fire ENTER as before. Always emitted so
    // init()'s generated body can reference it regardless of @@persist.
    methods.push(CodegenNode::NativeBlock {
        code: "static var __skipInitialEnter: Bool = false".to_string(),
        span: None,
    });

    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    // __kernel — Swift: no semicolons, no `new`, `!=` for comparison
    let mut kernel_code = String::new();
    kernel_code.push_str("__router(__e)\n");
    kernel_code.push_str("while __next_compartment != nil {\n");
    kernel_code.push_str("    let next_compartment = __next_compartment!\n");
    kernel_code.push_str("    __next_compartment = nil\n");
    kernel_code.push_str(&format!(
        "    let exit_event = {}(message: \"<$\")\n",
        event_class
    ));
    kernel_code.push_str("    __router(exit_event)\n");
    kernel_code.push_str("    __compartment = next_compartment\n");
    kernel_code.push_str("    if __compartment.forward_event == nil {\n");
    kernel_code.push_str(&format!(
        "        let enter_event = {}(message: \"$>\")\n",
        event_class
    ));
    kernel_code.push_str("        __router(enter_event)\n");
    kernel_code.push_str("    } else {\n");
    kernel_code.push_str("        let forward_event = __compartment.forward_event!\n");
    kernel_code.push_str("        __compartment.forward_event = nil\n");
    kernel_code.push_str("        if forward_event._message == \"$>\" {\n");
    kernel_code.push_str("            __router(forward_event)\n");
    kernel_code.push_str("        } else {\n");
    kernel_code.push_str(&format!(
        "            let enter_event = {}(message: \"$>\")\n",
        event_class
    ));
    kernel_code.push_str("            __router(enter_event)\n");
    kernel_code.push_str("            __router(forward_event)\n");
    kernel_code.push_str("        }\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("    // Mark all stacked contexts as transitioned\n");
    kernel_code.push_str("    for i in 0..<_context_stack.count {\n");
    kernel_code.push_str("        _context_stack[i]._transitioned = true\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("}");

    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: kernel_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — Swift: if/else chain with == comparison
    let mut router_code = String::new();
    router_code.push_str("let state_name = __compartment.state\n");
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        router_code.push_str(&format!("{} state_name == \"{}\" {{\n", prefix, state));
        router_code.push_str(&format!("    _state_{}(__e)\n", state));
    }
    if !states.is_empty() {
        router_code.push_str("}");
    }

    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next").with_type(compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__next_compartment = next".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_csharp_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    // __kernel
    let mut kernel_code = String::new();
    kernel_code.push_str("__router(__e);\n");
    kernel_code.push_str("while (__next_compartment != null) {\n");
    kernel_code.push_str(&format!(
        "    {} next_compartment = __next_compartment;\n",
        compartment_class
    ));
    kernel_code.push_str("    __next_compartment = null;\n");
    kernel_code.push_str(&format!(
        "    {} exit_event = new {}(\"<$\");\n",
        event_class, event_class
    ));
    kernel_code.push_str("    __router(exit_event);\n");
    kernel_code.push_str("    __compartment = next_compartment;\n");
    kernel_code.push_str("    if (__compartment.forward_event == null) {\n");
    kernel_code.push_str(&format!(
        "        {} enter_event = new {}(\"$>\");\n",
        event_class, event_class
    ));
    kernel_code.push_str("        __router(enter_event);\n");
    kernel_code.push_str("    } else {\n");
    kernel_code.push_str(&format!(
        "        {} forward_event = __compartment.forward_event;\n",
        event_class
    ));
    kernel_code.push_str("        __compartment.forward_event = null;\n");
    kernel_code.push_str("        if (forward_event._message == \"$>\") {\n");
    kernel_code.push_str("            __router(forward_event);\n");
    kernel_code.push_str("        } else {\n");
    kernel_code.push_str(&format!(
        "            {} enter_event = new {}(\"$>\");\n",
        event_class, event_class
    ));
    kernel_code.push_str("            __router(enter_event);\n");
    kernel_code.push_str("            __router(forward_event);\n");
    kernel_code.push_str("        }\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("    // Mark all stacked contexts as transitioned\n");
    kernel_code.push_str("    foreach (var ctx in _context_stack) {\n");
    kernel_code.push_str("        ctx._transitioned = true;\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("}");

    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: kernel_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router - if/else if chain with ==
    let mut router_code = String::new();
    router_code.push_str("string state_name = __compartment.state;\n");
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        router_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
        router_code.push_str(&format!("    _state_{}(__e);\n", state));
    }
    if !states.is_empty() {
        router_code.push_str("}");
    }

    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next").with_type(compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__next_compartment = next;".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_go_machinery(system: &SystemAst) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    // __kernel
    let mut kernel_code = String::new();
    kernel_code.push_str("s.__router(__e)\n");
    kernel_code.push_str("for s.__next_compartment != nil {\n");
    kernel_code.push_str("    next_compartment := s.__next_compartment\n");
    kernel_code.push_str("    s.__next_compartment = nil\n");
    kernel_code.push_str(&format!("    exit_event := &{}FrameEvent{{_message: \"<$\", _parameters: s.__compartment.exitArgs}}\n", system.name));
    kernel_code.push_str("    s.__router(exit_event)\n");
    kernel_code.push_str("    s.__compartment = next_compartment\n");
    kernel_code.push_str("    if s.__compartment.forwardEvent == nil {\n");
    kernel_code.push_str(&format!("        enter_event := &{}FrameEvent{{_message: \"$>\", _parameters: s.__compartment.enterArgs}}\n", system.name));
    kernel_code.push_str("        s.__router(enter_event)\n");
    kernel_code.push_str("    } else {\n");
    kernel_code.push_str("        forward_event := s.__compartment.forwardEvent\n");
    kernel_code.push_str("        s.__compartment.forwardEvent = nil\n");
    kernel_code.push_str("        if forward_event._message == \"$>\" {\n");
    kernel_code.push_str("            s.__router(forward_event)\n");
    kernel_code.push_str("        } else {\n");
    kernel_code.push_str(&format!("            enter_event := &{}FrameEvent{{_message: \"$>\", _parameters: s.__compartment.enterArgs}}\n", system.name));
    kernel_code.push_str("            s.__router(enter_event)\n");
    kernel_code.push_str("            s.__router(forward_event)\n");
    kernel_code.push_str("        }\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("    // Mark all stacked contexts as transitioned\n");
    kernel_code.push_str("    for i := range s._context_stack {\n");
    kernel_code.push_str("        s._context_stack[i]._transitioned = true\n");
    kernel_code.push_str("    }\n");
    kernel_code.push_str("}");

    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(&format!("*{}FrameEvent", system.name))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: kernel_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router - switch on state string
    let mut router_code = String::new();
    router_code.push_str("switch s.__compartment.state {\n");
    for state in &states {
        router_code.push_str(&format!("case \"{}\":\n", state));
        router_code.push_str(&format!("    s._state_{}(__e)\n", state));
    }
    router_code.push_str("}");

    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(&format!("*{}FrameEvent", system.name))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next").with_type(&format!("*{}Compartment", system.name))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "s.__next_compartment = next".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_lua_machinery(event_class: &str) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    // __kernel method
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"-- Route event to current state
self:__router(__e)
-- Process any pending transition
while self.__next_compartment ~= nil do
    local next_compartment = self.__next_compartment
    self.__next_compartment = nil
    -- Exit current state
    local exit_event = {}.new("<$", self.__compartment.exit_args)
    self:__router(exit_event)
    -- Switch to new compartment
    self.__compartment = next_compartment
    -- Enter new state (or forward event)
    if next_compartment.forward_event == nil then
        local enter_event = {}.new("$>", self.__compartment.enter_args)
        self:__router(enter_event)
    else
        local forward_event = next_compartment.forward_event
        next_compartment.forward_event = nil
        if forward_event._message == "$>" then
            self:__router(forward_event)
        else
            local enter_event = {}.new("$>", self.__compartment.enter_args)
            self:__router(enter_event)
            self:__router(forward_event)
        end
    end
    -- Mark all stacked contexts as transitioned
    for _, ctx in ipairs(self._context_stack) do
        ctx._transitioned = true
    end
end"#,
                event_class, event_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router method — dispatches to state handler methods
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"local state_name = self.__compartment.state
local handler = self["_state_" .. state_name]
if handler then
    handler(self, __e)
end"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition method — caches next compartment (deferred)
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "self.__next_compartment = next_compartment".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_dart_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    // __kernel method
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"// Route event to current state
__router(__e);
// Process any pending transition
while (__next_compartment != null) {{
    final next_compartment = __next_compartment!;
    __next_compartment = null;
    // Exit current state
    final exit_event = {}("<\$", __compartment.exit_args);
    __router(exit_event);
    // Switch to new compartment
    __compartment = next_compartment;
    // Enter new state (or forward event)
    if (next_compartment.forward_event == null) {{
        final enter_event = {}("\$>", __compartment.enter_args);
        __router(enter_event);
    }} else {{
        final forward_event = next_compartment.forward_event!;
        next_compartment.forward_event = null;
        if (forward_event._message == "\$>") {{
            __router(forward_event);
        }} else {{
            final enter_event = {}("\$>", __compartment.enter_args);
            __router(enter_event);
            __router(forward_event);
        }}
    }}
    // Mark all stacked contexts as transitioned
    for (final ctx in _context_stack) {{
        ctx._transitioned = true;
    }}
}}"#,
                event_class, event_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router method — use switch dispatch on state name
    let mut router_code = String::from("switch (__compartment.state) {\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            router_code.push_str(&format!("    case \"{}\":\n", state.name));
            router_code.push_str(&format!("        _state_{}(__e);\n", state.name));
            router_code.push_str("        break;\n");
        }
    }
    router_code.push_str("}");
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: router_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition method
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment").with_type(compartment_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__next_compartment = next_compartment;".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

fn generate_gdscript_machinery(event_class: &str) -> Vec<CodegenNode> {
    let mut methods = Vec::new();

    // Class-static flag gating the ctor's ENTER dispatch. GDScript 4.1+
    // supports `static var`. See Swift comment for the rationale — same
    // class of bug across every language whose restore path uses the
    // default-construction idiom.
    methods.push(CodegenNode::NativeBlock {
        code: "static var __skipInitialEnter: bool = false".to_string(),
        span: None,
    });

    // __kernel method
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"# Route event to current state
self.__router(__e)
# Process any pending transition
while self.__next_compartment != null:
    var next_compartment = self.__next_compartment
    self.__next_compartment = null
    # Exit current state
    var exit_event = {}.new("<$", self.__compartment.exit_args)
    self.__router(exit_event)
    # Switch to new compartment
    self.__compartment = next_compartment
    # Enter new state (or forward event)
    if next_compartment.forward_event == null:
        var enter_event = {}.new("$>", self.__compartment.enter_args)
        self.__router(enter_event)
    else:
        # Forward event to new state
        var forward_event = next_compartment.forward_event
        next_compartment.forward_event = null
        if forward_event._message == "$>":
            self.__router(forward_event)
        else:
            var enter_event = {}.new("$>", self.__compartment.enter_args)
            self.__router(enter_event)
            self.__router(forward_event)
    # Mark all stacked contexts as transitioned
    for ctx in self._context_stack:
        ctx._transitioned = true"#,
                event_class, event_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router method
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"var state_name = self.__compartment.state
var handler_name = "_state_" + state_name
if self.has_method(handler_name):
    self.call(handler_name, __e)"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition method
    methods.push(CodegenNode::Method {
        name: "__transition".to_string(),
        params: vec![Param::new("next_compartment")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "self.__next_compartment = next_compartment".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    methods
}

/// Generate C router dispatch using if-else chain with strcmp
fn generate_c_router_dispatch(system: &SystemAst) -> String {
    let sys = &system.name;
    let mut code = String::new();
    code.push_str("const char* state_name = self->__compartment->state;\n");

    if let Some(ref machine) = system.machine {
        for (i, state) in machine.states.iter().enumerate() {
            let cond = if i == 0 { "if" } else { "} else if" };
            code.push_str(&format!(
                "{} (strcmp(state_name, \"{}\") == 0) {{\n    {}_state_{}(self, __e);\n",
                cond, state.name, sys, state.name
            ));
        }
        if !machine.states.is_empty() {
            code.push_str("}");
        }
    }

    code
}

/// Expand `@@SystemName(args)` in domain variable initializers to native constructors.
/// This handles the same pattern as the assembler's tagged instantiation expansion,
/// but for domain code that is emitted during codegen (before the assembler runs).
pub(crate) fn expand_tagged_in_domain(raw_code: &str, lang: TargetLanguage) -> String {
    // Delegate string/comment skipping at the outer level to the target
    // language's skipper so `@@Foo(...)` appearing inside a string or
    // comment isn't mistakenly rewritten into a constructor call.
    let skipper = crate::frame_c::compiler::native_region_scanner::create_skipper(lang);
    let bytes = raw_code.as_bytes();
    let end = bytes.len();
    let mut result = String::new();
    let mut i = 0;

    while i < end {
        if let Some(next) = skipper.skip_string(bytes, i, end) {
            result.push_str(&raw_code[i..next]);
            i = next;
            continue;
        }
        if let Some(next) = skipper.skip_comment(bytes, i, end) {
            result.push_str(&raw_code[i..next]);
            i = next;
            continue;
        }
        // Look for @@ followed by uppercase letter
        if i + 2 < bytes.len()
            && bytes[i] == b'@'
            && bytes[i + 1] == b'@'
            && i + 2 < bytes.len()
            && bytes[i + 2].is_ascii_uppercase()
        {
            let start = i;
            i += 2;
            let name_start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = std::str::from_utf8(&bytes[name_start..i]).unwrap_or("");

            if i < bytes.len() && bytes[i] == b'(' {
                // Use the target language's skipper to find the matching
                // close paren — handles string literals, escapes, and
                // nested parens per-language rather than re-implementing
                // them here.
                let skipper = crate::frame_c::compiler::native_region_scanner::create_skipper(lang);
                let args_start = i + 1;
                let after_close = match skipper.balanced_paren_end(bytes, i, bytes.len()) {
                    Some(pos) => pos,
                    None => {
                        // Unbalanced — emit the bare `@@Name` and move on.
                        result.push_str(&raw_code[start..i]);
                        continue;
                    }
                };
                let close_paren = after_close - 1;
                let args = std::str::from_utf8(&bytes[args_start..close_paren]).unwrap_or("");
                i = after_close;
                // Generate native constructor per target language.
                let constructor = match lang {
                    TargetLanguage::Python3 => {
                        format!("{}({})", name, args)
                    }
                    // GDScript instantiation is `Class.new(...)`; bare
                    // `Class(...)` parses as a function call at runtime
                    // and fails with "Cannot call non-static function
                    // 'Logger()' on a null instance."
                    TargetLanguage::GDScript => {
                        format!("{}.new({})", name, args)
                    }
                    TargetLanguage::TypeScript
                    | TargetLanguage::JavaScript
                    | TargetLanguage::Cpp
                    | TargetLanguage::Java
                    | TargetLanguage::CSharp
                    | TargetLanguage::Dart
                    | TargetLanguage::Kotlin
                    | TargetLanguage::Php => format!("new {}({})", name, args),
                    TargetLanguage::Rust => format!("{}::new({})", name, args),
                    TargetLanguage::C => format!("{}_new({})", name, args),
                    TargetLanguage::Go => format!("New{}({})", name, args),
                    TargetLanguage::Swift => format!("{}({})", name, args),
                    TargetLanguage::Ruby => format!("{}.new({})", name, args),
                    TargetLanguage::Lua => format!("{}.new({})", name, args),
                    TargetLanguage::Erlang => {
                        format!("{}:start_link({})", to_snake_case_simple(name), args)
                    }
                    TargetLanguage::Graphviz => name.to_string(),
                };
                result.push_str(&constructor);
                continue;
            }
            // Not a valid pattern — copy original
            for b in &bytes[start..i] {
                result.push(*b as char);
            }
            continue;
        }

        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Simple snake_case conversion for Erlang module names
fn to_snake_case_simple(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::codegen::codegen_utils::{
        convert_expression, convert_literal, cpp_map_type, cpp_wrap_any_arg, csharp_map_type,
        expression_to_string, go_map_type, is_bool_type, is_float_type, is_int_type,
        is_string_type, java_map_type, kotlin_map_type, state_var_init_value, swift_map_type,
        to_snake_case, type_to_cpp_string, type_to_string, HandlerContext,
    };
    use crate::frame_c::compiler::frame_ast::{
        DomainVar, Expression, Literal, Span, SystemAst, Type,
    };
    use crate::frame_c::visitors::TargetLanguage;

    fn create_test_system() -> SystemAst {
        SystemAst::new("TestSystem".to_string(), Span::new(0, 0))
    }

    #[test]
    fn test_generate_simple_system() {
        let system = create_test_system();
        let arcanum = Arcanum::new();
        // Empty source since test system has no actions/operations with native code
        let source = b"";
        let node = generate_system(&system, &arcanum, TargetLanguage::Python3, source);

        match node {
            CodegenNode::Class { name, .. } => {
                assert_eq!(name, "TestSystem");
            }
            _ => panic!("Expected Class node"),
        }
    }

    #[test]
    fn test_generate_fields() {
        let mut system = create_test_system();
        system.domain.push(DomainVar {
            name: "counter".to_string(),
            var_type: Type::Custom("int".into()),
            initializer_text: Some("0".to_string()),
            is_const: false,
            span: Span::new(0, 0),
        });

        let syntax = super::super::backend::ClassSyntax::python();
        let fields = generate_fields(&system, &syntax);

        // Should have _state, _state_stack, _state_context, and counter
        assert!(fields.len() >= 4);
        assert!(fields.iter().any(|f| f.name == "counter"));
    }

    #[test]
    fn test_generate_constructor() {
        let system = create_test_system();
        let syntax = super::super::backend::ClassSyntax::python();
        let constructor = generate_constructor(&system, &syntax);

        match constructor {
            CodegenNode::Constructor { body, .. } => {
                assert!(!body.is_empty());
            }
            _ => panic!("Expected Constructor node"),
        }
    }

    #[test]
    fn test_init_references_param_exact_match() {
        assert!(init_references_param("balance", &["balance".into()]));
    }

    #[test]
    fn test_init_references_param_in_expression() {
        assert!(init_references_param("balance + 10", &["balance".into()]));
    }

    #[test]
    fn test_init_references_param_no_match() {
        assert!(!init_references_param("0", &["balance".into()]));
        assert!(!init_references_param(
            "mutableListOf<>()",
            &["balance".into()]
        ));
    }

    #[test]
    fn test_init_references_param_member_access_not_matched() {
        // Defaults.count should NOT match param "count" — it's a member access
        assert!(!init_references_param("Defaults.count", &["count".into()]));
        assert!(!init_references_param("obj.balance", &["balance".into()]));
        assert!(!init_references_param(
            "Config.DEFAULT_VALUE",
            &["DEFAULT_VALUE".into()]
        ));
    }

    #[test]
    fn test_init_references_param_method_on_param() {
        // count.toString() SHOULD match — it's a method call on the param
        assert!(init_references_param("count.toString()", &["count".into()]));
    }

    #[test]
    fn test_init_references_param_substring_not_matched() {
        // "rebalance" should NOT match param "balance"
        assert!(!init_references_param("rebalance", &["balance".into()]));
        assert!(!init_references_param("balanced_tree", &["balance".into()]));
    }

    #[test]
    fn test_init_references_param_empty() {
        assert!(!init_references_param("", &["balance".into()]));
        assert!(!init_references_param("balance", &[]));
    }
}
