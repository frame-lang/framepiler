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
                if name == "__transition" {
                    continue;
                }
                // Skip __prepareEnter / __prepareExit — pure data
                // operations on Compartment objects, no event dispatch.
                // Keeping them sync lets the constructor (which can't be
                // async in JS/TS) call __prepareEnter directly.
                if name == "__prepareEnter"
                    || name == "__prepareExit"
                    || name == "__hsm_chain"
                {
                    continue;
                }
                *is_async = true;
                // Add `await` to internal dispatch calls in NativeBlock strings
                add_await_to_dispatch_calls(body, lang);
                // C++ coroutines: per-handler methods (`_s_<…>_hdl_<…>`) may
                // lack a terminating co_await / co_return / co_yield (e.g.
                // a lifecycle enter that just runs native code). For those,
                // append `co_return;` so the function is actually a
                // coroutine — otherwise `FrameTask<void>` is returned by
                // value without a backing promise, and the caller's
                // `co_await` crashes.
                if matches!(lang, TargetLanguage::Cpp)
                    && (name.starts_with("_s_") || name.starts_with("_state_"))
                {
                    ensure_cpp_coroutine_terminator(body);
                }
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

/// Ensure a C++ method body contains at least one coroutine keyword
/// (`co_await`, `co_return`, `co_yield`). If none is present, append
/// `co_return;` to the body's last NativeBlock. Required because
/// `FrameTask<T>` is only constructed as a coroutine when the function
/// body contains a coroutine keyword — otherwise the declared return
/// type is a default-constructed `FrameTask<T>` with an empty handle,
/// which crashes on `co_await`.
fn ensure_cpp_coroutine_terminator(body: &mut Vec<CodegenNode>) {
    let has_coroutine_keyword = body.iter().any(|node| {
        if let CodegenNode::NativeBlock { code, .. } = node {
            code.contains("co_await") || code.contains("co_return") || code.contains("co_yield")
        } else {
            false
        }
    });
    if has_coroutine_keyword {
        return;
    }
    if let Some(CodegenNode::NativeBlock { code, .. }) = body.iter_mut().rev().find(|n| {
        matches!(n, CodegenNode::NativeBlock { .. })
    }) {
        if !code.ends_with('\n') {
            code.push('\n');
        }
        code.push_str("co_return;\n");
    } else {
        body.push(CodegenNode::NativeBlock {
            code: "co_return;\n".to_string(),
            span: None,
        });
    }
}

/// Remove kernel call from constructor body (for async two-phase init).
fn remove_kernel_call_from_body(body: &mut Vec<CodegenNode>) {
    body.retain(|node| {
        if let CodegenNode::NativeBlock { code, .. } = node {
            // Remove lines that call into the dispatch chain from the
            // constructor — kernel, cascade helpers, and transition-loop
            // drains all need to be replaced by an explicit `await init()`
            // call from user code in async systems.
            !code.contains("__kernel(")
                && !code.contains("_kernel(self,")
                && !code.contains("__fire_enter_cascade")
                && !code.contains("__fire_exit_cascade")
                && !code.contains("__process_transition_loop")
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
            || trimmed.starts_with("self.__fire_")
            || trimmed.starts_with("self.__route_to_state(")
            || trimmed.starts_with("self.__process_transition_loop(")
            || (trimmed.starts_with("self._state_") && !trimmed.starts_with("self._state_stack"))
            || trimmed.starts_with("self._s_")
            || trimmed.starts_with("handler(")         // Python dynamic dispatch
            || trimmed.starts_with("handler.call(")   // TypeScript dynamic dispatch
            || trimmed.starts_with("this.__kernel(")
            || trimmed.starts_with("this.__router(")
            || trimmed.starts_with("this.__fire_")
            || trimmed.starts_with("this.__route_to_state(")
            || trimmed.starts_with("this.__process_transition_loop(")
            || trimmed.starts_with("this._state_")
            || trimmed.starts_with("this._s_")
            || trimmed.starts_with("this.#kernel(")
            || trimmed.starts_with("this.#router(")
            || trimmed.starts_with("$this->__kernel(")
            || trimmed.starts_with("$this->__router(")
            || trimmed.starts_with("$this->__fire_")
            || trimmed.starts_with("$this->__route_to_state(")
            || trimmed.starts_with("$this->_state_")
            // C++: `this->` uses `->` deref, not `.` like TS/Java/etc.
            || trimmed.starts_with("this->__kernel(")
            || trimmed.starts_with("this->__router(")
            || trimmed.starts_with("this->__fire_")
            || trimmed.starts_with("this->__route_to_state(")
            || trimmed.starts_with("this->_state_")
            || trimmed.starts_with("this->_s_")
            // Bare references (no `self.`/`this.` prefix): Swift uses bare
            // for same-instance method calls.
            || trimmed.starts_with("__kernel(")
            || trimmed.starts_with("__router(")
            || trimmed.starts_with("__fire_")
            || trimmed.starts_with("__route_to_state(")
            || trimmed.starts_with("__process_transition_loop(")
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
                    // C: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // state_args / enter_args flow through the helper.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let mut init_code = String::new();
                    // Build temporary FrameVecs for state_args and enter_args.
                    if state_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __sa = NULL;\n",
                            sys = system.name
                        ));
                    } else {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __sa = {sys}_FrameVec_new();\n",
                            sys = system.name
                        ));
                        for n in &state_args_vec {
                            init_code.push_str(&format!(
                                "{sys}_FrameVec_push(__sa, (void*)(intptr_t)({n}));\n",
                                sys = system.name,
                                n = n
                            ));
                        }
                    }
                    if enter_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __ea = NULL;\n",
                            sys = system.name
                        ));
                    } else {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec* __ea = {sys}_FrameVec_new();\n",
                            sys = system.name
                        ));
                        for n in &enter_args_vec {
                            init_code.push_str(&format!(
                                "{sys}_FrameVec_push(__ea, (void*)(intptr_t)({n}));\n",
                                sys = system.name,
                                n = n
                            ));
                        }
                    }
                    init_code.push_str(&format!(
                        "self->__compartment = {sys}_prepareEnter(self, \"{leaf}\", __sa, __ea);\n",
                        sys = system.name,
                        leaf = first_state.name
                    ));
                    init_code.push_str("self->__next_compartment = NULL;\n");
                    if !state_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec_destroy(__sa);\n",
                            sys = system.name
                        ));
                    }
                    if !enter_args_vec.is_empty() {
                        init_code.push_str(&format!(
                            "{sys}_FrameVec_destroy(__ea);",
                            sys = system.name
                        ));
                    }
                    body.push(CodegenNode::NativeBlock {
                        code: init_code,
                        span: None,
                    });
                }
                TargetLanguage::Python3 => {
                    // Python: build the start state's compartment chain
                    // via __prepareEnter, the same helper used by all
                    // transitions. System header params flow into the
                    // start state's state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = None",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    // TS/JS: build start chain via __prepareEnter, the same
                    // helper used by all transitions. System header params
                    // flow into state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = this.__prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Php => {
                    // PHP: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| format!("${}", p.name))
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| format!("${}", p.name))
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "$this->__compartment = $this->__prepareEnter(\"{}\", {}, {});\n$this->__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Ruby => {
                    // Ruby: build start chain via __prepareEnter, the same
                    // helper used by all transitions. System header params
                    // flow into state_args / enter_args channels.
                    let mut state_args_vec: Vec<String> = Vec::new();
                    let mut enter_args_vec: Vec<String> = Vec::new();
                    for p in &system.params {
                        match p.kind {
                            crate::frame_c::compiler::frame_ast::ParamKind::StateArg => {
                                state_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::EnterArg => {
                                enter_args_vec.push(p.name.clone());
                            }
                            crate::frame_c::compiler::frame_ast::ParamKind::Domain => {}
                        }
                    }
                    let state_args_lit = format!("[{}]", state_args_vec.join(", "));
                    let enter_args_lit = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "@__compartment = __prepareEnter(\"{}\", {}, {})\n@__next_compartment = nil",
                            first_state.name, state_args_lit, enter_args_lit
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Cpp => {
                    // C++: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // state_args / enter_args flow through the helper.
                    // Values are wrapped in std::any so the dispatch
                    // reader (which uses std::any_cast<Type>) round-trips
                    // them correctly.
                    let _ = compartment_class;
                    let state_args_wrapped: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| format!("std::any({})", cpp_wrap_any_arg(&p.name)))
                        .collect();
                    let enter_args_wrapped: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| format!("std::any({})", cpp_wrap_any_arg(&p.name)))
                        .collect();
                    let state_arg = format!(
                        "std::vector<std::any>{{{}}}",
                        state_args_wrapped.join(", ")
                    );
                    let enter_arg = format!(
                        "std::vector<std::any>{{{}}}",
                        enter_args_wrapped.join(", ")
                    );
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "__compartment = __prepareEnter(\"{}\", {}, {});",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Java => {
                    // Java: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "new ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new ArrayList<>(java.util.Arrays.asList({}))",
                            state_args_vec.join(", ")
                        )
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "new ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new ArrayList<>(java.util.Arrays.asList({}))",
                            enter_args_vec.join(", ")
                        )
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Kotlin => {
                    // Kotlin: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {})\nthis.__next_compartment = null",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Swift => {
                    // Swift: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = {}.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = nil",
                            system.name, first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::CSharp => {
                    // C#: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!(
                            "new List<object> {{ {} }}",
                            state_args_vec.join(", ")
                        )
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!(
                            "new List<object> {{ {} }}",
                            enter_args_vec.join(", ")
                        )
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = __prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Go => {
                    // Go: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "[]any{}".to_string()
                    } else {
                        format!("[]any{{{}}}", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "[]any{}".to_string()
                    } else {
                        format!("[]any{{{}}}", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "s.__compartment = s.__prepareEnter(\"{}\", {}, {})\ns.__next_compartment = nil",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::Dart => {
                    // Dart: build start chain via __prepareEnter, the same
                    // helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let _ = syntax;
                    let _ = compartment_class;
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "this.__compartment = this.__prepareEnter(\"{}\", {}, {});\nthis.__next_compartment = null;",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                TargetLanguage::GDScript => {
                    // GDScript: build start chain via __prepareEnter, the
                    // same helper used by every transition. System header
                    // params flow into state_args / enter_args.
                    let state_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::StateArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system
                        .params
                        .iter()
                        .filter(|p| {
                            matches!(
                                p.kind,
                                crate::frame_c::compiler::frame_ast::ParamKind::EnterArg
                            )
                        })
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = format!("[{}]", state_args_vec.join(", "));
                    let enter_arg = format!("[{}]", enter_args_vec.join(", "));
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self.__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = null",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                // Lua: build start chain via __prepareEnter, the same
                // helper used by all transitions. System header params
                // flow into state_args / enter_args channels. Uses
                // table.pack(...) / nil instead of `{}` literals
                // (block-transformer workaround).
                TargetLanguage::Lua => {
                    let state_args_vec: Vec<String> = system.params.iter()
                        .filter(|p| matches!(p.kind, crate::frame_c::compiler::frame_ast::ParamKind::StateArg))
                        .map(|p| p.name.clone())
                        .collect();
                    let enter_args_vec: Vec<String> = system.params.iter()
                        .filter(|p| matches!(p.kind, crate::frame_c::compiler::frame_ast::ParamKind::EnterArg))
                        .map(|p| p.name.clone())
                        .collect();
                    let state_arg = if state_args_vec.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", state_args_vec.join(", "))
                    };
                    let enter_arg = if enter_args_vec.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", enter_args_vec.join(", "))
                    };
                    body.push(CodegenNode::NativeBlock {
                        code: format!(
                            "self.__compartment = self:__prepareEnter(\"{}\", {}, {})\nself.__next_compartment = nil",
                            first_state.name, state_arg, enter_arg
                        ),
                        span: None,
                    });
                }
                // Dynamic languages and remaining: New expression
                // (Erlang, Kotlin — all routed here)
                TargetLanguage::Python3
                | TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Php
                | TargetLanguage::Ruby
                | TargetLanguage::Erlang
                | TargetLanguage::Kotlin => {
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
                TargetLanguage::Python3 => {
                    let _ = event_class;
                    "self.__fire_enter_cascade()\nself.__process_transition_loop()".to_string()
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    let _ = event_class;
                    "this.__fire_enter_cascade();\nthis.__process_transition_loop();".to_string()
                }
                TargetLanguage::Rust => format!(
                    r#"let __frame_event = {}::new("$>");
let __ctx = {}FrameContext::new(__frame_event, None);
self._context_stack.push(__ctx);
self.__kernel();
self._context_stack.pop();"#,
                    event_class, system.name
                ),
                TargetLanguage::C => {
                    let _ = start_state_has_enter_params;
                    format!(
                        "{sys}_fire_enter_cascade(self);\n{sys}_process_transition_loop(self);",
                        sys = system.name
                    )
                },
                TargetLanguage::Cpp => format!(
                    "if (!__skipInitialEnter) {{\n    __fire_enter_cascade();\n    __process_transition_loop();\n}}"
                ),
                TargetLanguage::Java => {
                    let _ = (event_class, &system.name);
                    "if (!__skipInitialEnter) {\n    __fire_enter_cascade();\n    __process_transition_loop();\n}".to_string()
                }
                TargetLanguage::CSharp => {
                    let _ = (event_class, &system.name);
                    "__fire_enter_cascade();\n__process_transition_loop();".to_string()
                }
                TargetLanguage::Go => {
                    let _ = (event_class, &system.name);
                    "s.__fire_enter_cascade()\ns.__process_transition_loop()".to_string()
                }
                TargetLanguage::Kotlin => {
                    let _ = (event_class, &system.name);
                    "if (!__skipInitialEnter) {\n    __fire_enter_cascade()\n    __process_transition_loop()\n}".to_string()
                }
                TargetLanguage::Swift => format!(
                    "if !{}.__skipInitialEnter {{\n    __fire_enter_cascade()\n    __process_transition_loop()\n}}",
                    system.name
                ),
                TargetLanguage::Php => {
                    let _ = (event_class, &system.name);
                    "$this->__fire_enter_cascade();\n$this->__process_transition_loop();".to_string()
                }
                TargetLanguage::Ruby => {
                    let _ = (event_class, &system.name);
                    "__fire_enter_cascade\n__process_transition_loop".to_string()
                }
                TargetLanguage::Lua => {
                    let _ = (event_class, &system.name);
                    "self:__fire_enter_cascade()\nself:__process_transition_loop()".to_string()
                }
                TargetLanguage::Dart => {
                    let _ = (event_class, &system.name);
                    "__fire_enter_cascade();\n__process_transition_loop();".to_string()
                }
                TargetLanguage::GDScript => {
                    let _ = (event_class, &system.name);
                    "if not __skipInitialEnter:\n    self.__fire_enter_cascade()\n    self.__process_transition_loop()".to_string()
                }
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

/// Compute the HSM topology chain for each state in the machine.
///
/// Returns a list of (leaf_state_name, ancestor_chain) pairs, where
/// `ancestor_chain` is ordered root-to-leaf (with the leaf as the last
/// entry). For flat states, the chain has length 1 — just the state
/// itself. For HSM children, the chain walks up via each state's
/// declared `parent` field until a state with no parent is reached.
///
/// This is the source of truth for `_HSM_CHAIN` table emission and for
/// `__prepareEnter` / persistence-restore chain reconstruction.
pub(crate) fn compute_hsm_chains(system: &SystemAst) -> Vec<(String, Vec<String>)> {
    let machine = match &system.machine {
        Some(m) => m,
        None => return Vec::new(),
    };
    let parent_lookup: std::collections::HashMap<String, Option<String>> = machine
        .states
        .iter()
        .map(|s| (s.name.clone(), s.parent.clone()))
        .collect();
    let mut result = Vec::new();
    for state in &machine.states {
        let mut chain: Vec<String> = Vec::new();
        let mut cursor = Some(state.name.clone());
        while let Some(name) = cursor {
            chain.push(name.clone());
            cursor = parent_lookup.get(&name).cloned().flatten();
        }
        chain.reverse();
        result.push((state.name.clone(), chain));
    }
    result
}

/// Generate Frame machinery methods (__kernel, __router, __transition,
/// __prepareEnter, __prepareExit, __fire_enter_cascade,
/// __fire_exit_cascade, __route_to_state) for all target languages.
pub(crate) fn generate_frame_machinery(
    system: &SystemAst,
    syntax: &super::backend::ClassSyntax,
    lang: TargetLanguage,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let compartment_class = format!("{}Compartment", system.name);
    let event_class = format!("{}FrameEvent", system.name);

    match lang {
        TargetLanguage::Python3 => methods.extend(generate_python3_machinery(system, &event_class, &compartment_class)),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => methods.extend(
            generate_javascript_machinery(system, lang, &event_class, &compartment_class),
        ),
        TargetLanguage::Php => methods.extend(generate_php_machinery(system, &event_class, &compartment_class)),
        TargetLanguage::Ruby => methods.extend(generate_ruby_machinery(system, &event_class, &compartment_class)),
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
        TargetLanguage::Lua => methods.extend(generate_lua_machinery(system, &event_class, &compartment_class)),
        TargetLanguage::Dart => methods.extend(generate_dart_machinery(
            system,
            &event_class,
            &compartment_class,
        )),
        TargetLanguage::GDScript => methods.extend(generate_gdscript_machinery(system, &event_class, &compartment_class)),
        TargetLanguage::Graphviz => unreachable!(),
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

fn generate_python3_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let chains = compute_hsm_chains(system);

    // _HSM_CHAIN class attribute — static topology table mapping each
    // state name to its root-to-leaf ancestor chain. Used by
    // __prepareEnter to construct destination compartment chains and by
    // restore_state() to reconstruct chains from a save blob.
    let mut chain_lines = String::from("_HSM_CHAIN = {\n");
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_lines.push_str(&format!("    \"{}\": [{}],\n", leaf, chain_str));
    }
    chain_lines.push('}');
    methods.push(CodegenNode::NativeBlock {
        code: chain_lines,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain for a
    // transition. Each compartment in the chain receives its own copy of
    // state_args / enter_args; ancestors and the leaf get the same
    // values under the signature-match rule (see docs/frame_runtime.md
    // § "Uniform Parameter Propagation").
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf"),
            Param::new("state_args"),
            Param::new("enter_args"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"comp = None
for name in self._HSM_CHAIN[leaf]:
    new_comp = {}(name)
    new_comp.state_args = list(state_args)
    new_comp.enter_args = list(enter_args)
    new_comp.parent_compartment = comp
    comp = new_comp
return comp"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every compartment in the
    // current source chain before the kernel's exit cascade fires.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"comp = self.__compartment
while comp is not None:
    comp.exit_args = list(exit_args)
    comp = comp.parent_compartment"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router. Routes an event to a specific
    // state's dispatcher with a specific compartment, instead of using
    // self.__compartment. Used by the enter/exit cascades to dispatch to
    // every layer of the HSM chain.
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name"),
            Param::new("__e"),
            Param::new("compartment"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"handler_name = f"_state_{state_name}"
handler = getattr(self, handler_name, None)
if handler:
    handler(__e, compartment)"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — fires <$ on every layer of the current
    // chain, walking from leaf upward (bottom-up).
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"comp = self.__compartment
while comp is not None:
    exit_event = {}("<$", comp.exit_args)
    self.__route_to_state(comp.state, exit_event, comp)
    comp = comp.parent_compartment"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — fires $> on every layer of the new chain,
    // walking from root downward (top-down).
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"chain = []
comp = self.__compartment
while comp is not None:
    chain.append(comp)
    comp = comp.parent_compartment
for comp in reversed(chain):
    enter_event = {}("$>", comp.enter_args)
    self.__route_to_state(comp.state, enter_event, comp)"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains any pending transitions
    // queued by handlers (or by the initial enter cascade in the
    // constructor). Extracted from __kernel so the constructor can
    // also call it after firing the start state's enter cascade.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while self.__next_compartment is not None:
    next_compartment = self.__next_compartment
    self.__next_compartment = None
    # Exit current chain (bottom-up)
    self.__fire_exit_cascade()
    # Switch to new compartment
    self.__compartment = next_compartment
    # Enter new chain (top-down) or forward event
    if next_compartment.forward_event is None:
        self.__fire_enter_cascade()
    else:
        forward_event = next_compartment.forward_event
        next_compartment.forward_event = None
        self.__fire_enter_cascade()
        if forward_event._message != "$>":
            self.__router(forward_event)
    # Mark all stacked contexts as transitioned
    for ctx in self._context_stack:
        ctx._transitioned = True"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel method - the main event processing loop. Routes the
    // event to the current state's dispatcher, then drains any pending
    // transitions via __process_transition_loop.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"# Route event to current state
self.__router(__e)
# Process any pending transition
self.__process_transition_loop()"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — dispatches events to the current state's dispatcher
    // method. Always uses self.__compartment.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"self.__route_to_state(self.__compartment.state, __e, self.__compartment)"#
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
    system: &SystemAst,
    lang: TargetLanguage,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let chains = compute_hsm_chains(system);
    let is_ts = matches!(lang, TargetLanguage::TypeScript);

    // _HSM_CHAIN — static topology table mapping each leaf state name
    // to its root-to-leaf ancestor chain. Used by __prepareEnter and
    // restore_state(). Emitted as a class-level const.
    let mut chain_lines = String::new();
    if is_ts {
        chain_lines.push_str("static readonly _HSM_CHAIN: Record<string, string[]> = {\n");
    } else {
        chain_lines.push_str("static _HSM_CHAIN = {\n");
    }
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_lines.push_str(&format!("    \"{}\": [{}],\n", leaf, chain_str));
    }
    chain_lines.push_str("};");
    methods.push(CodegenNode::NativeBlock {
        code: chain_lines,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain for a transition.
    let prepare_enter_params = if is_ts {
        vec![
            Param::new("leaf").with_type("string"),
            Param::new("state_args").with_type("any[]"),
            Param::new("enter_args").with_type("any[]"),
        ]
    } else {
        vec![
            Param::new("leaf"),
            Param::new("state_args"),
            Param::new("enter_args"),
        ]
    };
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: prepare_enter_params,
        return_type: if is_ts { Some(compartment_class.to_string()) } else { None },
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"let comp{cast} = null;
for (const name of {sys}._HSM_CHAIN[leaf]) {{
    const new_comp = new {comp}(name);
    new_comp.state_args = [...state_args];
    new_comp.enter_args = [...enter_args];
    new_comp.parent_compartment = comp;
    comp = new_comp;
}}
return comp{nonnull};"#,
                cast = if is_ts { format!(": {} | null", compartment_class) } else { String::new() },
                sys = system.name,
                comp = compartment_class,
                nonnull = if is_ts { "!" } else { "" }
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer of the source chain.
    let prepare_exit_params = if is_ts {
        vec![Param::new("exit_args").with_type("any[]")]
    } else {
        vec![Param::new("exit_args")]
    };
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: prepare_exit_params,
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"let comp{cast} = this.__compartment;
while (comp !== null) {{
    comp.exit_args = [...exit_args];
    comp = comp.parent_compartment;
}}"#,
                cast = if is_ts { format!(": {} | null", compartment_class) } else { String::new() },
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router. Routes to a specific state's
    // dispatcher with a specific compartment.
    let dispatch_call = if is_ts {
        "(this as any)[handler_name]".to_string()
    } else {
        "this[handler_name]".to_string()
    };
    let route_params = if is_ts {
        vec![
            Param::new("state_name").with_type("string"),
            Param::new("__e").with_type(event_class),
            Param::new("compartment").with_type(compartment_class),
        ]
    } else {
        vec![
            Param::new("state_name"),
            Param::new("__e"),
            Param::new("compartment"),
        ]
    };
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: route_params,
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"const handler_name = `_state_${{state_name}}`;
const handler = {dispatch};
if (handler) {{
    handler.call(this, __e, compartment);
}}"#,
                dispatch = dispatch_call
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — fires <$ on every layer, walking bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"let comp{cast} = this.__compartment;
while (comp !== null) {{
    const exit_event = new {evt}("<$", comp.exit_args);
    this.__route_to_state(comp.state, exit_event, comp);
    comp = comp.parent_compartment;
}}"#,
                cast = if is_ts { format!(": {} | null", compartment_class) } else { String::new() },
                evt = event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — fires $> on every layer, walking top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"const chain{cast} = [];
let comp{cast2} = this.__compartment;
while (comp !== null) {{
    chain.push(comp);
    comp = comp.parent_compartment;
}}
for (let i = chain.length - 1; i >= 0; i--) {{
    const layer = chain[i];
    const enter_event = new {evt}("$>", layer.enter_args);
    this.__route_to_state(layer.state, enter_event, layer);
}}"#,
                cast = if is_ts { format!(": {}[]", compartment_class) } else { String::new() },
                cast2 = if is_ts { format!(": {} | null", compartment_class) } else { String::new() },
                evt = event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains pending transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while (this.__next_compartment !== null) {
    const next_compartment = this.__next_compartment;
    this.__next_compartment = null;
    // Exit current chain (bottom-up)
    this.__fire_exit_cascade();
    // Switch compartment
    this.__compartment = next_compartment;
    // Enter new chain (top-down) or forward event
    if (next_compartment.forward_event === null) {
        this.__fire_enter_cascade();
    } else {
        const forward_event = next_compartment.forward_event;
        next_compartment.forward_event = null;
        this.__fire_enter_cascade();
        if (forward_event._message !== "$>") {
            this.__router(forward_event);
        }
    }
    // Mark all stacked contexts as transitioned
    for (const ctx of this._context_stack) {
        ctx._transitioned = true;
    }
}"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains pending transitions.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"// Route event to current state
this.__router(__e);
// Process any pending transition
this.__process_transition_loop();"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state for the active compartment.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "this.__route_to_state(this.__compartment.state, __e, this.__compartment);".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition — caches next compartment (deferred transition).
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

fn generate_php_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let chains = compute_hsm_chains(system);

    // hsm_chain — instance method returning the topology table.
    let mut chain_method = String::from("public function hsm_chain() {\n    return [\n");
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!("        \"{}\" => [{}],\n", leaf, chain_str));
    }
    chain_method.push_str("    ];\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf"),
            Param::new("state_args"),
            Param::new("enter_args"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"$comp = null;
foreach ($this->hsm_chain()[$leaf] as $name) {{
    $new_comp = new {}($name);
    $new_comp->state_args = $state_args;
    $new_comp->enter_args = $enter_args;
    $new_comp->parent_compartment = $comp;
    $comp = $new_comp;
}}
return $comp;"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"$comp = $this->__compartment;
while ($comp !== null) {
    $comp->exit_args = $exit_args;
    $comp = $comp->parent_compartment;
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name"),
            Param::new("__e"),
            Param::new("compartment"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"$handler_name = "_state_" . $state_name;
if (method_exists($this, $handler_name)) {
    $this->$handler_name($__e, $compartment);
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"$comp = $this->__compartment;
while ($comp !== null) {{
    $exit_event = new {}("<$", $comp->exit_args);
    $this->__route_to_state($comp->state, $exit_event, $comp);
    $comp = $comp->parent_compartment;
}}"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"$chain = [];
$comp = $this->__compartment;
while ($comp !== null) {{
    $chain[] = $comp;
    $comp = $comp->parent_compartment;
}}
for ($i = count($chain) - 1; $i >= 0; $i--) {{
    $layer = $chain[$i];
    $enter_event = new {}("$>", $layer->enter_args);
    $this->__route_to_state($layer->state, $enter_event, $layer);
}}"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while ($this->__next_compartment !== null) {
    $next_compartment = $this->__next_compartment;
    $this->__next_compartment = null;
    $this->__fire_exit_cascade();
    $this->__compartment = $next_compartment;
    if ($next_compartment->forward_event === null) {
        $this->__fire_enter_cascade();
    } else {
        $forward_event = $next_compartment->forward_event;
        $next_compartment->forward_event = null;
        $this->__fire_enter_cascade();
        if ($forward_event->_message !== "$>") {
            $this->__router($forward_event);
        }
    }
    foreach ($this->_context_stack as $ctx) {
        $ctx->_transitioned = true;
    }
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "// Route event to current state\n$this->__router($__e);\n// Process any pending transition\n$this->__process_transition_loop();".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "$this->__route_to_state($this->__compartment->state, $__e, $this->__compartment);".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __transition — caches next compartment.
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

fn generate_ruby_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let chains = compute_hsm_chains(system);

    // hsm_chain — class method returning the topology table. Emitted
    // as a method (not a constant) because Ruby's TestRunner loads
    // multiple test files into the same process. A constant
    // collision warning gets captured as test output and
    // misclassified as unrecognized. Memoization via class instance
    // variable also collides across test files (same class name,
    // different topology), so we recreate the hash on each call —
    // it's small, allocation cost is negligible vs. dispatch.
    let mut chain_method = String::from("def self.hsm_chain\n    {\n");
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!("        \"{}\" => [{}],\n", leaf, chain_str));
    }
    chain_method.push_str("    }\nend");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain for a transition.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf"),
            Param::new("state_args"),
            Param::new("enter_args"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"comp = nil
self.class.hsm_chain[leaf].each do |name|
    new_comp = {}.new(name)
    new_comp.state_args = state_args.dup
    new_comp.enter_args = enter_args.dup
    new_comp.parent_compartment = comp
    comp = new_comp
end
comp"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer of the source chain.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"comp = @__compartment
while comp != nil
    comp.exit_args = exit_args.dup
    comp = comp.parent_compartment
end"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router. Routes to a specific state's
    // dispatcher with a specific compartment.
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name"),
            Param::new("__e"),
            Param::new("compartment"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"handler_name = "_state_#{state_name}"
if respond_to?(handler_name, true)
    send(handler_name, __e, compartment)
end"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — fires <$ on every layer, walking bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"comp = @__compartment
while comp != nil
    exit_event = {}.new("<$", comp.exit_args)
    __route_to_state(comp.state, exit_event, comp)
    comp = comp.parent_compartment
end"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — fires $> on every layer, walking top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"chain = []
comp = @__compartment
while comp != nil
    chain.push(comp)
    comp = comp.parent_compartment
end
chain.reverse_each do |layer|
    enter_event = {}.new("$>", layer.enter_args)
    __route_to_state(layer.state, enter_event, layer)
end"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains pending transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while @__next_compartment != nil
    next_compartment = @__next_compartment
    @__next_compartment = nil
    __fire_exit_cascade
    @__compartment = next_compartment
    if next_compartment.forward_event == nil
        __fire_enter_cascade
    else
        forward_event = next_compartment.forward_event
        next_compartment.forward_event = nil
        __fire_enter_cascade
        if forward_event._message != "$>"
            __router(forward_event)
        end
    end
    @_context_stack.each { |ctx| ctx._transitioned = true }
end"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains pending transitions.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "# Route event to current state\n__router(__e)\n# Process any pending transition\n__process_transition_loop".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state for the active compartment.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__route_to_state(@__compartment.state, __e, @__compartment)".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

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
    let sys = &system.name;
    let chains = compute_hsm_chains(system);

    // hsm_chain — function that fills out *out_chain with const char*
    // pointers and returns the chain length. C has no map literal,
    // so we use a switch on the leaf name.
    let mut chain_body = String::from("if (false) { (void)0; }\n");
    for (leaf, chain) in &chains {
        chain_body.push_str(&format!(
            "    else if (strcmp(leaf, \"{}\") == 0) {{\n        static const char* __chain[] = {{ ",
            leaf
        ));
        for (i, name) in chain.iter().enumerate() {
            if i > 0 {
                chain_body.push_str(", ");
            }
            chain_body.push_str(&format!("\"{}\"", name));
        }
        chain_body.push_str(&format!(
            " }};\n        *out_chain = __chain;\n        return {};\n    }}\n",
            chain.len()
        ));
    }
    chain_body.push_str("    *out_chain = NULL;\n    return 0;");
    methods.push(CodegenNode::Method {
        name: "__hsm_chain".to_string(),
        params: vec![
            Param::new("leaf").with_type("const char*"),
            Param::new("out_chain").with_type("const char***"),
        ],
        return_type: Some("int".to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: chain_body,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareEnter — builds the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("const char*"),
            Param::new("state_args").with_type(&format!("{}_FrameVec*", sys)),
            Param::new("enter_args").with_type(&format!("{}_FrameVec*", sys)),
        ],
        return_type: Some(format!("{}_Compartment*", sys)),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"const char** chain = NULL;
int n = {sys}_hsm_chain(self, leaf, &chain);
{sys}_Compartment* comp = NULL;
for (int i = 0; i < n; i++) {{
    {sys}_Compartment* nc = {sys}_Compartment_new(chain[i]);
    if (state_args) {{
        for (int j = 0; j < state_args->size; j++) {sys}_FrameVec_push(nc->state_args, state_args->items[j]);
    }}
    if (enter_args) {{
        for (int j = 0; j < enter_args->size; j++) {sys}_FrameVec_push(nc->enter_args, enter_args->items[j]);
    }}
    nc->parent_compartment = comp;  // adopts ref
    comp = nc;
}}
return comp;"#,
                sys = sys
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer of the
    // current chain.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type(&format!("{}_FrameVec*", sys))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{sys}_Compartment* comp = self->__compartment;
while (comp != NULL) {{
    // Clear any prior exit_args before copying the new ones in.
    while (comp->exit_args->size > 0) comp->exit_args->size--;
    if (exit_args) {{
        for (int j = 0; j < exit_args->size; j++) {sys}_FrameVec_push(comp->exit_args, exit_args->items[j]);
    }}
    comp = comp->parent_compartment;
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

    // __route_to_state — cascade router. Same dispatch logic as
    // __router but takes an explicit state name and compartment.
    let route_code = generate_c_route_to_state_dispatch(system);
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("const char*"),
            Param::new("__e").with_type(&format!("{}_FrameEvent*", sys)),
            Param::new("compartment").with_type(&format!("{}_Compartment*", sys)),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{sys}_Compartment* comp = self->__compartment;
while (comp != NULL) {{
    {sys}_FrameEvent* exit_event = {sys}_FrameEvent_new("<$", comp->exit_args, 0);
    {sys}_route_to_state(self, comp->state, exit_event, comp);
    {sys}_FrameEvent_destroy(exit_event);
    comp = comp->parent_compartment;
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

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"// Build chain bottom-up into a stack-allocated array (reasonable
// upper bound: HSM depth is ~16 in practice).
{sys}_Compartment* chain[64];
int depth = 0;
{sys}_Compartment* comp = self->__compartment;
while (comp != NULL && depth < 64) {{
    chain[depth++] = comp;
    comp = comp->parent_compartment;
}}
for (int i = depth - 1; i >= 0; i--) {{
    {sys}_Compartment* layer = chain[i];
    {sys}_FrameEvent* enter_event = {sys}_FrameEvent_new("$>", layer->enter_args, 0);
    {sys}_route_to_state(self, layer->state, enter_event, layer);
    {sys}_FrameEvent_destroy(enter_event);
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

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"while (self->__next_compartment != NULL) {{
    {sys}_Compartment* next_compartment = self->__next_compartment;
    self->__next_compartment = NULL;
    {sys}_fire_exit_cascade(self);
    {sys}_Compartment_unref(self->__compartment);
    self->__compartment = next_compartment;
    if (next_compartment->forward_event == NULL) {{
        {sys}_fire_enter_cascade(self);
    }} else {{
        {sys}_FrameEvent* forward_event = next_compartment->forward_event;
        next_compartment->forward_event = NULL;
        {sys}_fire_enter_cascade(self);
        if (strcmp(forward_event->_message, "$>") != 0) {{
            {sys}_router(self, forward_event);
        }}
    }}
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

    // __kernel method - thin: route then drain.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(&format!("{}_FrameEvent*", sys))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                "{sys}_router(self, __e);\n{sys}_process_transition_loop(self);",
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
    let chains = compute_hsm_chains(system);
    let comp_ptr = format!("std::shared_ptr<{}>", compartment_class);

    // Class-static flag.
    methods.push(CodegenNode::NativeBlock {
        code: "inline static bool __skipInitialEnter = false;".to_string(),
        span: None,
    });

    // hsm_chain — instance method returning the topology table.
    let mut chain_method = String::from(
        "std::unordered_map<std::string, std::vector<std::string>> hsm_chain() {\n    return {\n",
    );
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!(
            "        {{\"{}\", {{{}}} }},\n",
            leaf, chain_str
        ));
    }
    chain_method.push_str("    };\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("const std::string&"),
            Param::new("state_args").with_type("std::vector<std::any>"),
            Param::new("enter_args").with_type("std::vector<std::any>"),
        ],
        return_type: Some(comp_ptr.clone()),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                // Assign hsm_chain() to a local before iterating — the
                // method returns by value, so a range-for over the
                // temporary's [] result would dangle.
                r#"{0} comp = nullptr;
auto chain_table = hsm_chain();
for (const auto& name : chain_table[leaf]) {{
    auto new_comp = std::make_shared<{1}>(name);
    new_comp->state_args = state_args;
    new_comp->enter_args = enter_args;
    new_comp->parent_compartment = comp;
    comp = new_comp;
}}
return comp;"#,
                comp_ptr, compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("std::vector<std::any>")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{} comp = __compartment;
while (comp) {{
    comp->exit_args = exit_args;
    comp = comp->parent_compartment;
}}"#,
                comp_ptr
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();
    let mut route_code = String::new();
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        route_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
        route_code.push_str(&format!("    _state_{}(__e, compartment);\n", state));
    }
    if !states.is_empty() {
        route_code.push_str("}");
    }
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("const std::string&"),
            Param::new("__e").with_type(&format!("{}&", event_class)),
            Param::new("compartment").with_type(&comp_ptr),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{0} comp = __compartment;
while (comp) {{
    {1} exit_event("<$", comp->exit_args);
    __route_to_state(comp->state, exit_event, comp);
    comp = comp->parent_compartment;
}}"#,
                comp_ptr, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"std::vector<{0}> chain;
{0} comp = __compartment;
while (comp) {{
    chain.push_back(comp);
    comp = comp->parent_compartment;
}}
for (auto it = chain.rbegin(); it != chain.rend(); ++it) {{
    auto& layer = *it;
    {1} enter_event("$>", layer->enter_args);
    __route_to_state(layer->state, enter_event, layer);
}}"#,
                comp_ptr, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while (__next_compartment) {
    auto next_compartment = std::move(__next_compartment);
    __fire_exit_cascade();
    __compartment = std::move(next_compartment);
    if (!__compartment->forward_event) {
        __fire_enter_cascade();
    } else {
        auto forward_event = std::move(__compartment->forward_event);
        __fire_enter_cascade();
        if (forward_event->_message != "$>") {
            __router(*forward_event);
        }
    }
    for (auto& ctx : _context_stack) {
        ctx._transitioned = true;
    }
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__router(__e);\n__process_transition_loop();".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(&format!("{}&", event_class))],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__route_to_state(__compartment->state, __e, __compartment);".to_string(),
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
        params: vec![Param::new("next").with_type(&comp_ptr)],
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
    let chains = compute_hsm_chains(system);

    // Class-static flag gating the ctor's ENTER dispatch.
    methods.push(CodegenNode::NativeBlock {
        code: "private static boolean __skipInitialEnter = false;".to_string(),
        span: None,
    });

    // hsm_chain — instance method returning the topology table.
    let mut chain_method = String::from(
        "private HashMap<String, ArrayList<String>> hsm_chain() {\n    HashMap<String, ArrayList<String>> m = new HashMap<>();\n",
    );
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!(
            "    m.put(\"{}\", new ArrayList<>(java.util.Arrays.asList({})));\n",
            leaf, chain_str
        ));
    }
    chain_method.push_str("    return m;\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("String"),
            Param::new("state_args").with_type("ArrayList<Object>"),
            Param::new("enter_args").with_type("ArrayList<Object>"),
        ],
        return_type: Some(compartment_class.to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{0} comp = null;
for (String name : hsm_chain().get(leaf)) {{
    {0} new_comp = new {0}(name);
    new_comp.state_args = new ArrayList<>(state_args);
    new_comp.enter_args = new ArrayList<>(enter_args);
    new_comp.parent_compartment = comp;
    comp = new_comp;
}}
return comp;"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("ArrayList<Object>")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{} comp = __compartment;
while (comp != null) {{
    comp.exit_args = new ArrayList<>(exit_args);
    comp = comp.parent_compartment;
}}"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router. Same dispatch table as
    // __router but takes an explicit state name.
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();
    let mut route_code = String::new();
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        route_code.push_str(&format!(
            "{} (state_name.equals(\"{}\")) {{\n",
            prefix, state
        ));
        route_code.push_str(&format!("    _state_{}(__e, compartment);\n", state));
    }
    if !states.is_empty() {
        route_code.push_str("}");
    }
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("String"),
            Param::new("__e").with_type(event_class),
            Param::new("compartment").with_type(compartment_class),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{0} comp = __compartment;
while (comp != null) {{
    {1} exit_event = new {1}("<$", comp.exit_args);
    __route_to_state(comp.state, exit_event, comp);
    comp = comp.parent_compartment;
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"ArrayList<{0}> chain = new ArrayList<>();
{0} comp = __compartment;
while (comp != null) {{
    chain.add(comp);
    comp = comp.parent_compartment;
}}
for (int i = chain.size() - 1; i >= 0; i--) {{
    {0} layer = chain.get(i);
    {1} enter_event = new {1}("$>", layer.enter_args);
    __route_to_state(layer.state, enter_event, layer);
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"while (__next_compartment != null) {{
    {0} next_compartment = __next_compartment;
    __next_compartment = null;
    __fire_exit_cascade();
    __compartment = next_compartment;
    if (next_compartment.forward_event == null) {{
        __fire_enter_cascade();
    }} else {{
        {1} forward_event = next_compartment.forward_event;
        next_compartment.forward_event = null;
        __fire_enter_cascade();
        if (!forward_event._message.equals("$>")) {{
            __router(forward_event);
        }}
    }}
    for ({2}FrameContext ctx : _context_stack) {{
        ctx._transitioned = true;
    }}
}}"#,
                compartment_class, event_class, system.name
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__router(__e);\n__process_transition_loop();".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__route_to_state(__compartment.state, __e, __compartment);".to_string(),
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
    let chains = compute_hsm_chains(system);

    // Companion-object flag gating the ctor's ENTER dispatch.
    methods.push(CodegenNode::NativeBlock {
        code: "@JvmStatic var __skipInitialEnter: Boolean = false".to_string(),
        span: None,
    });

    // hsm_chain — instance method returning the topology table.
    let mut chain_method = String::from(
        "private fun hsm_chain(): Map<String, List<String>> {\n    return mapOf(\n",
    );
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!(
            "        \"{}\" to listOf({}),\n",
            leaf, chain_str
        ));
    }
    chain_method.push_str("    )\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("String"),
            Param::new("state_args").with_type("MutableList<Any?>"),
            Param::new("enter_args").with_type("MutableList<Any?>"),
        ],
        return_type: Some(compartment_class.to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp: {0}? = null
for (name in hsm_chain()[leaf]!!) {{
    val new_comp = {0}(name)
    new_comp.state_args.addAll(state_args)
    new_comp.enter_args.addAll(enter_args)
    new_comp.parent_compartment = comp
    comp = new_comp
}}
return comp!!"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("MutableList<Any?>")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp: {}? = __compartment
while (comp != null) {{
    comp.exit_args.clear()
    comp.exit_args.addAll(exit_args)
    comp = comp.parent_compartment
}}"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();
    let mut route_code = String::new();
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        route_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
        route_code.push_str(&format!("    _state_{}(__e, compartment)\n", state));
    }
    if !states.is_empty() {
        route_code.push_str("}");
    }
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("String"),
            Param::new("__e").with_type(event_class),
            Param::new("compartment").with_type(compartment_class),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp: {0}? = __compartment
while (comp != null) {{
    val exit_event = {1}("<$", comp.exit_args)
    __route_to_state(comp.state, exit_event, comp)
    comp = comp.parent_compartment
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"val chain = mutableListOf<{0}>()
var comp: {0}? = __compartment
while (comp != null) {{
    chain.add(comp)
    comp = comp.parent_compartment
}}
for (i in chain.size - 1 downTo 0) {{
    val layer = chain[i]
    val enter_event = {1}("$>", layer.enter_args)
    __route_to_state(layer.state, enter_event, layer)
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while (__next_compartment != null) {
    val next_compartment = __next_compartment!!
    __next_compartment = null
    __fire_exit_cascade()
    __compartment = next_compartment
    if (next_compartment.forward_event == null) {
        __fire_enter_cascade()
    } else {
        val forward_event = next_compartment.forward_event!!
        next_compartment.forward_event = null
        __fire_enter_cascade()
        if (forward_event._message != "$>") {
            __router(forward_event)
        }
    }
    for (ctx in _context_stack) {
        ctx._transitioned = true
    }
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__router(__e)\n__process_transition_loop()".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__route_to_state(__compartment.state, __e, __compartment)".to_string(),
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
    let chains = compute_hsm_chains(system);

    // Class-level static flag.
    methods.push(CodegenNode::NativeBlock {
        code: "static var __skipInitialEnter: Bool = false".to_string(),
        span: None,
    });

    // hsm_chain — class method (so it's callable from init before all
    // stored properties are initialized; Swift forbids instance-method
    // calls on `self` until that point).
    let mut chain_method = String::from(
        "private static func hsm_chain() -> [String: [String]] {\n    return [\n",
    );
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!("        \"{}\": [{}],\n", leaf, chain_str));
    }
    chain_method.push_str("    ]\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — class method (must be callable from init before
    // stored properties are initialized; doesn't touch instance state
    // anyway). Constructs the destination HSM chain.
    methods.push(CodegenNode::NativeBlock {
        code: format!(
            r#"private static func __prepareEnter(_ leaf: String, _ state_args: [Any], _ enter_args: [Any]) -> {0} {{
    var comp: {0}? = nil
    for name in {1}.hsm_chain()[leaf]! {{
        let new_comp = {0}(state: name)
        new_comp.state_args = state_args
        new_comp.enter_args = enter_args
        new_comp.parent_compartment = comp
        comp = new_comp
    }}
    return comp!
}}"#,
            compartment_class, system.name
        ),
        span: None,
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("[Any]")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp: {}? = __compartment
while comp != nil {{
    comp!.exit_args = exit_args
    comp = comp!.parent_compartment
}}"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();
    let mut route_code = String::new();
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        route_code.push_str(&format!("{} state_name == \"{}\" {{\n", prefix, state));
        route_code.push_str(&format!("    _state_{}(__e, compartment)\n", state));
    }
    if !states.is_empty() {
        route_code.push_str("}");
    }
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("String"),
            Param::new("__e").with_type(event_class),
            Param::new("compartment").with_type(compartment_class),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp: {0}? = __compartment
while comp != nil {{
    let exit_event = {1}(message: "<$", parameters: comp!.exit_args)
    __route_to_state(comp!.state, exit_event, comp!)
    comp = comp!.parent_compartment
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var chain: [{0}] = []
var comp: {0}? = __compartment
while comp != nil {{
    chain.append(comp!)
    comp = comp!.parent_compartment
}}
for i in stride(from: chain.count - 1, through: 0, by: -1) {{
    let layer = chain[i]
    let enter_event = {1}(message: "$>", parameters: layer.enter_args)
    __route_to_state(layer.state, enter_event, layer)
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while __next_compartment != nil {
    let next_compartment = __next_compartment!
    __next_compartment = nil
    __fire_exit_cascade()
    __compartment = next_compartment
    if next_compartment.forward_event == nil {
        __fire_enter_cascade()
    } else {
        let forward_event = next_compartment.forward_event!
        next_compartment.forward_event = nil
        __fire_enter_cascade()
        if forward_event._message != "$>" {
            __router(forward_event)
        }
    }
    for i in 0..<_context_stack.count {
        _context_stack[i]._transitioned = true
    }
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__router(__e)\n__process_transition_loop()".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__route_to_state(__compartment.state, __e, __compartment)".to_string(),
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
    let chains = compute_hsm_chains(system);

    // hsm_chain — instance method returning the topology table.
    let mut chain_method = String::from(
        "private Dictionary<string, List<string>> hsm_chain() {\n    return new Dictionary<string, List<string>> {\n",
    );
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!(
            "        {{ \"{}\", new List<string> {{ {} }} }},\n",
            leaf, chain_str
        ));
    }
    chain_method.push_str("    };\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("string"),
            Param::new("state_args").with_type("List<object>"),
            Param::new("enter_args").with_type("List<object>"),
        ],
        return_type: Some(compartment_class.to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{0}? comp = null;
foreach (string name in hsm_chain()[leaf]) {{
    {0} new_comp = new {0}(name);
    new_comp.state_args = new List<object>(state_args);
    new_comp.enter_args = new List<object>(enter_args);
    new_comp.parent_compartment = comp;
    comp = new_comp;
}}
return comp!;"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("List<object>")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{}? comp = __compartment;
while (comp != null) {{
    comp.exit_args = new List<object>(exit_args);
    comp = comp.parent_compartment;
}}"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();
    let mut route_code = String::new();
    for (i, state) in states.iter().enumerate() {
        let prefix = if i == 0 { "if" } else { "} else if" };
        route_code.push_str(&format!("{} (state_name == \"{}\") {{\n", prefix, state));
        route_code.push_str(&format!("    _state_{}(__e, compartment);\n", state));
    }
    if !states.is_empty() {
        route_code.push_str("}");
    }
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("string"),
            Param::new("__e").with_type(event_class),
            Param::new("compartment").with_type(compartment_class),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{0}? comp = __compartment;
while (comp != null) {{
    {1} exit_event = new {1}("<$", comp.exit_args);
    __route_to_state(comp.state, exit_event, comp);
    comp = comp.parent_compartment;
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"List<{0}> chain = new List<{0}>();
{0}? comp = __compartment;
while (comp != null) {{
    chain.Add(comp);
    comp = comp.parent_compartment;
}}
for (int i = chain.Count - 1; i >= 0; i--) {{
    {0} layer = chain[i];
    {1} enter_event = new {1}("$>", layer.enter_args);
    __route_to_state(layer.state, enter_event, layer);
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"while (__next_compartment != null) {{
    {0} next_compartment = __next_compartment;
    __next_compartment = null;
    __fire_exit_cascade();
    __compartment = next_compartment;
    if (next_compartment.forward_event == null) {{
        __fire_enter_cascade();
    }} else {{
        {1} forward_event = next_compartment.forward_event;
        next_compartment.forward_event = null;
        __fire_enter_cascade();
        if (forward_event._message != "$>") {{
            __router(forward_event);
        }}
    }}
    foreach (var ctx in _context_stack) {{
        ctx._transitioned = true;
    }}
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__router(__e);\n__process_transition_loop();".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__route_to_state(__compartment.state, __e, __compartment);".to_string(),
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
    let chains = compute_hsm_chains(system);
    let event_type = format!("*{}FrameEvent", system.name);
    let comp_type = format!("*{}Compartment", system.name);

    // hsm_chain — instance method returning the topology table.
    let mut chain_method = format!(
        "func (s *{}) hsm_chain() map[string][]string {{\n    return map[string][]string{{\n",
        system.name
    );
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!(
            "        \"{}\": {{{}}},\n",
            leaf, chain_str
        ));
    }
    chain_method.push_str("    }\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("string"),
            Param::new("state_args").with_type("[]any"),
            Param::new("enter_args").with_type("[]any"),
        ],
        return_type: Some(comp_type.clone()),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp {0} = nil
for _, name := range s.hsm_chain()[leaf] {{
    new_comp := new{1}Compartment(name)
    new_comp.stateArgs = append([]any{{}}, state_args...)
    new_comp.enterArgs = append([]any{{}}, enter_args...)
    new_comp.parentCompartment = comp
    comp = new_comp
}}
return comp"#,
                comp_type, system.name
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("[]any")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"comp := s.__compartment
for comp != nil {{
    comp.exitArgs = append([]any{{}}, exit_args...)
    comp = comp.parentCompartment
}}
_ = comp"#
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();
    let mut route_code = String::from("switch state_name {\n");
    for state in &states {
        route_code.push_str(&format!("case \"{}\":\n", state));
        route_code.push_str(&format!("    s._state_{}(__e, compartment)\n", state));
    }
    route_code.push_str("}");
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("string"),
            Param::new("__e").with_type(&event_type),
            Param::new("compartment").with_type(&comp_type),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"comp := s.__compartment
for comp != nil {{
    exit_event := &{}FrameEvent{{_message: "<$", _parameters: comp.exitArgs}}
    s.__route_to_state(comp.state, exit_event, comp)
    comp = comp.parentCompartment
}}"#,
                system.name
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"chain := []{0}{{}}
comp := s.__compartment
for comp != nil {{
    chain = append(chain, comp)
    comp = comp.parentCompartment
}}
for i := len(chain) - 1; i >= 0; i-- {{
    layer := chain[i]
    enter_event := &{1}FrameEvent{{_message: "$>", _parameters: layer.enterArgs}}
    s.__route_to_state(layer.state, enter_event, layer)
}}"#,
                comp_type, system.name
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"for s.__next_compartment != nil {
    next_compartment := s.__next_compartment
    s.__next_compartment = nil
    s.__fire_exit_cascade()
    s.__compartment = next_compartment
    if next_compartment.forwardEvent == nil {
        s.__fire_enter_cascade()
    } else {
        forward_event := next_compartment.forwardEvent
        next_compartment.forwardEvent = nil
        s.__fire_enter_cascade()
        if forward_event._message != "$>" {
            s.__router(forward_event)
        }
    }
    for i := range s._context_stack {
        s._context_stack[i]._transitioned = true
    }
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(&event_type)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "s.__router(__e)\ns.__process_transition_loop()".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(&event_type)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "s.__route_to_state(s.__compartment.state, __e, s.__compartment)".to_string(),
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
        params: vec![Param::new("next").with_type(&comp_type)],
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

fn generate_lua_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let chains = compute_hsm_chains(system);

    // hsm_chain — class method returning the topology table.
    // Built via sequential assignments rather than a literal table
    // expression: the Lua block transformer treats `{ … }` on
    // multiple lines as a Frame block (matching `if`/`while`
    // bodies) and rewrites it incorrectly. Sequential assignments
    // avoid the multi-line literal entirely.
    let mut chain_method = String::from("function ");
    chain_method.push_str(&system.name);
    chain_method.push_str(":hsm_chain()\n    local t = {}\n");
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!("    t[\"{}\"] = {{{}}}\n", leaf, chain_str));
    }
    chain_method.push_str("    return t\nend");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain. Accepts
    // nil for empty args lists so transition sites can call
    // `self:__prepareEnter("X", nil, nil)` without emitting `{}`
    // literals (the Lua block transformer mishandles `{}` inside
    // if/else bodies — see transition emission notes).
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf"),
            Param::new("state_args"),
            Param::new("enter_args"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"state_args = state_args or {{}}
enter_args = enter_args or {{}}
local comp = nil
local chain = self:hsm_chain()[leaf]
for i = 1, #chain do
    local new_comp = {}.new(chain[i])
    new_comp.state_args = {{}}
    for j = 1, #state_args do new_comp.state_args[j] = state_args[j] end
    new_comp.enter_args = {{}}
    for j = 1, #enter_args do new_comp.enter_args[j] = enter_args[j] end
    new_comp.parent_compartment = comp
    comp = new_comp
end
return comp"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"local comp = self.__compartment
while comp ~= nil do
    comp.exit_args = {}
    for j = 1, #exit_args do comp.exit_args[j] = exit_args[j] end
    comp = comp.parent_compartment
end"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name"),
            Param::new("__e"),
            Param::new("compartment"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"local handler = self["_state_" .. state_name]
if handler then
    handler(self, __e, compartment)
end"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"local comp = self.__compartment
while comp ~= nil do
    local exit_event = {}.new("<$", comp.exit_args)
    self:__route_to_state(comp.state, exit_event, comp)
    comp = comp.parent_compartment
end"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"local chain = {{}}
local comp = self.__compartment
while comp ~= nil do
    chain[#chain + 1] = comp
    comp = comp.parent_compartment
end
for i = #chain, 1, -1 do
    local layer = chain[i]
    local enter_event = {}.new("$>", layer.enter_args)
    self:__route_to_state(layer.state, enter_event, layer)
end"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains pending transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while self.__next_compartment ~= nil do
    local next_compartment = self.__next_compartment
    self.__next_compartment = nil
    self:__fire_exit_cascade()
    self.__compartment = next_compartment
    if next_compartment.forward_event == nil then
        self:__fire_enter_cascade()
    else
        local forward_event = next_compartment.forward_event
        next_compartment.forward_event = nil
        self:__fire_enter_cascade()
        if forward_event._message ~= "$>" then
            self:__router(forward_event)
        end
    end
    for _, ctx in ipairs(self._context_stack) do
        ctx._transitioned = true
    end
end"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains transitions.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "-- Route event to current state\nself:__router(__e)\n-- Process any pending transition\nself:__process_transition_loop()".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state for the active compartment.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "self:__route_to_state(self.__compartment.state, __e, self.__compartment)".to_string(),
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
    let chains = compute_hsm_chains(system);

    // hsm_chain — instance method returning the topology table.
    let mut chain_method = String::from(
        "Map<String, List<String>> hsm_chain() {\n    return {\n",
    );
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!("        \"{}\": [{}],\n", leaf, chain_str));
    }
    chain_method.push_str("    };\n}");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf").with_type("String"),
            Param::new("state_args").with_type("List<dynamic>"),
            Param::new("enter_args").with_type("List<dynamic>"),
        ],
        return_type: Some(compartment_class.to_string()),
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{0}? comp = null;
for (final name in hsm_chain()[leaf]!) {{
    final new_comp = {0}(name);
    new_comp.state_args = List<dynamic>.from(state_args);
    new_comp.enter_args = List<dynamic>.from(enter_args);
    new_comp.parent_compartment = comp;
    comp = new_comp;
}}
return comp!;"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args").with_type("List<dynamic>")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{}? comp = __compartment;
while (comp != null) {{
    comp.exit_args = List<dynamic>.from(exit_args);
    comp = comp.parent_compartment;
}}"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router. Uses the same switch-on-state
    // pattern as __router but takes an explicit state name.
    let mut route_code = String::from("switch (state_name) {\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            route_code.push_str(&format!("    case \"{}\":\n", state.name));
            route_code.push_str(&format!(
                "        _state_{}(__e, compartment);\n",
                state.name
            ));
            route_code.push_str("        break;\n");
        }
    }
    route_code.push_str("}");
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name").with_type("String"),
            Param::new("__e").with_type(event_class),
            Param::new("compartment").with_type(compartment_class),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: route_code,
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"{0}? comp = __compartment;
while (comp != null) {{
    final exit_event = {1}("<\$", comp.exit_args);
    __route_to_state(comp.state, exit_event, comp);
    comp = comp.parent_compartment;
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"final List<{0}> chain = [];
{0}? comp = __compartment;
while (comp != null) {{
    chain.add(comp);
    comp = comp.parent_compartment;
}}
for (int i = chain.length - 1; i >= 0; i--) {{
    final layer = chain[i];
    final enter_event = {1}("\$>", layer.enter_args);
    __route_to_state(layer.state, enter_event, layer);
}}"#,
                compartment_class, event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains queued transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while (__next_compartment != null) {
    final next_compartment = __next_compartment!;
    __next_compartment = null;
    __fire_exit_cascade();
    __compartment = next_compartment;
    if (next_compartment.forward_event == null) {
        __fire_enter_cascade();
    } else {
        final forward_event = next_compartment.forward_event!;
        next_compartment.forward_event = null;
        __fire_enter_cascade();
        if (forward_event._message != "\$>") {
            __router(forward_event);
        }
    }
    for (final ctx in _context_stack) {
        ctx._transitioned = true;
    }
}"#
            .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "// Route event to current state\n__router(__e);\n// Process any pending transition\n__process_transition_loop();".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e").with_type(event_class)],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "__route_to_state(__compartment.state, __e, __compartment);".to_string(),
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

fn generate_gdscript_machinery(
    system: &SystemAst,
    event_class: &str,
    compartment_class: &str,
) -> Vec<CodegenNode> {
    let mut methods = Vec::new();
    let chains = compute_hsm_chains(system);

    // Class-static flag gating the ctor's ENTER dispatch.
    methods.push(CodegenNode::NativeBlock {
        code: "static var __skipInitialEnter: bool = false".to_string(),
        span: None,
    });

    // hsm_chain — class method returning the topology table.
    let mut chain_method = String::from("func hsm_chain() -> Dictionary:\n    return {\n");
    for (leaf, chain) in &chains {
        let chain_str = chain
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", ");
        chain_method.push_str(&format!("        \"{}\": [{}],\n", leaf, chain_str));
    }
    chain_method.push_str("    }");
    methods.push(CodegenNode::NativeBlock {
        code: chain_method,
        span: None,
    });

    // __prepareEnter — constructs the destination HSM chain.
    methods.push(CodegenNode::Method {
        name: "__prepareEnter".to_string(),
        params: vec![
            Param::new("leaf"),
            Param::new("state_args"),
            Param::new("enter_args"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp = null
for name in self.hsm_chain()[leaf]:
    var new_comp = {}.new(name)
    new_comp.state_args = state_args.duplicate()
    new_comp.enter_args = enter_args.duplicate()
    new_comp.parent_compartment = comp
    comp = new_comp
return comp"#,
                compartment_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __prepareExit — populates exit_args on every layer.
    methods.push(CodegenNode::Method {
        name: "__prepareExit".to_string(),
        params: vec![Param::new("exit_args")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"var comp = self.__compartment
while comp != null:
    comp.exit_args = exit_args.duplicate()
    comp = comp.parent_compartment"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __route_to_state — cascade router.
    methods.push(CodegenNode::Method {
        name: "__route_to_state".to_string(),
        params: vec![
            Param::new("state_name"),
            Param::new("__e"),
            Param::new("compartment"),
        ],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"var handler_name = "_state_" + state_name
if self.has_method(handler_name):
    self.call(handler_name, __e, compartment)"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_exit_cascade — bottom-up.
    methods.push(CodegenNode::Method {
        name: "__fire_exit_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var comp = self.__compartment
while comp != null:
    var exit_event = {}.new("<$", comp.exit_args)
    self.__route_to_state(comp.state, exit_event, comp)
    comp = comp.parent_compartment"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __fire_enter_cascade — top-down.
    methods.push(CodegenNode::Method {
        name: "__fire_enter_cascade".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: format!(
                r#"var chain = []
var comp = self.__compartment
while comp != null:
    chain.append(comp)
    comp = comp.parent_compartment
for i in range(chain.size() - 1, -1, -1):
    var layer = chain[i]
    var enter_event = {}.new("$>", layer.enter_args)
    self.__route_to_state(layer.state, enter_event, layer)"#,
                event_class
            ),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __process_transition_loop — drains pending transitions.
    methods.push(CodegenNode::Method {
        name: "__process_transition_loop".to_string(),
        params: vec![],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: r#"while self.__next_compartment != null:
    var next_compartment = self.__next_compartment
    self.__next_compartment = null
    self.__fire_exit_cascade()
    self.__compartment = next_compartment
    if next_compartment.forward_event == null:
        self.__fire_enter_cascade()
    else:
        var forward_event = next_compartment.forward_event
        next_compartment.forward_event = null
        self.__fire_enter_cascade()
        if forward_event._message != "$>":
            self.__router(forward_event)
    for ctx in self._context_stack:
        ctx._transitioned = true"#
                .to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __kernel — routes event then drains.
    methods.push(CodegenNode::Method {
        name: "__kernel".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "# Route event to current state\nself.__router(__e)\n# Process any pending transition\nself.__process_transition_loop()".to_string(),
            span: None,
        }],
        is_async: false,
        is_static: false,
        visibility: Visibility::Private,
        decorators: vec![],
    });

    // __router — delegates to __route_to_state.
    methods.push(CodegenNode::Method {
        name: "__router".to_string(),
        params: vec![Param::new("__e")],
        return_type: None,
        body: vec![CodegenNode::NativeBlock {
            code: "self.__route_to_state(self.__compartment.state, __e, self.__compartment)".to_string(),
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
    // Thin __router that delegates to __route_to_state with the
    // active compartment.
    let sys = &system.name;
    format!(
        "{sys}_route_to_state(self, self->__compartment->state, __e, self->__compartment);"
    )
}

fn generate_c_route_to_state_dispatch(system: &SystemAst) -> String {
    // Per-handler architecture: routes by state name to a specific
    // state's dispatcher with a specific compartment (see
    // docs/frame_runtime.md § "Dispatch Model"). Used by the cascade
    // helpers in addition to the __router thin wrapper.
    let sys = &system.name;
    let mut code = String::new();
    if let Some(ref machine) = system.machine {
        for (i, state) in machine.states.iter().enumerate() {
            let cond = if i == 0 { "if" } else { "} else if" };
            code.push_str(&format!(
                "{} (strcmp(state_name, \"{}\") == 0) {{\n    {}_state_{}(self, __e, compartment);\n",
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
