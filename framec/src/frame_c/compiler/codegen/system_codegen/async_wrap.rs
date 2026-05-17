//! Async dispatch-chain transformer.
//!
//! When any interface method on a Frame system is declared `async`,
//! the entire generated dispatch chain (interface methods, kernel,
//! router, state dispatch, handlers) has to be async too. This
//! module post-processes the `CodegenNode::Class` tree to enforce
//! that contract — flipping `is_async: true` on the relevant
//! methods, injecting `await` / `co_await` / `.await` into internal
//! dispatch call sites, and emitting an `init()` method that fires
//! the start-state's `$>` event.
//!
//! ## Per-language quirks worth flagging
//!
//! - **Java** is special-cased: it has no native async/await. The
//!   `make_java_interface_async` path wraps async-declared interface
//!   methods in `CompletableFuture<T>` and keeps the internal
//!   dispatch chain synchronous. An `init()` method is still emitted
//!   for cross-language API parity, but its body is a no-op.
//! - **Swift** reserves `init` for constructors — `func init() async`
//!   is a parse error. We rename to `initAsync` on Swift so tests can
//!   write `await w.initAsync()`.
//! - **C++** uses `co_await` (not `await`). Per-handler methods that
//!   don't actually `co_await` anything need a trailing `co_return;`
//!   so `FrameTask<T>` is constructed as a real coroutine — otherwise
//!   the caller's `co_await` crashes on the empty handle.
//! - **Rust** uses postfix `.await` rather than the `await <expr>`
//!   prefix used everywhere else. The `insert_rust_await` helper
//!   finds the closing paren of the dispatch call and splices
//!   `.await` after it.
//! - **Persist machinery stays sync.** `save_state` / `restore_state`
//!   and their recursive helpers (`__serComp`, `__deserComp`,
//!   `__convertJsonValue` and case variants) are pure data
//!   operations — they don't dispatch events. Marking them async
//!   triggered defects D16 (Swift's `await`-pattern check missed the
//!   serializer-internal calls) and D17 (C++ rewrote `return` to
//!   `co_return` inside the embedded `__ser` lambda whose return
//!   type isn't a coroutine type).
//!
//! Only `make_system_async` is part of this module's pub API; the
//! rest are private helpers.

use super::{CodegenNode, SystemAst, TargetLanguage, Visibility};

/// Java-specific async handling. Java has no native async/await; the
/// approximation is:
///
/// - Public interface methods declared `async` get `is_async = true`,
///   which the Java backend honors by wrapping the return type in
///   `CompletableFuture<T>` and the body in
///   `CompletableFuture.completedFuture(...)`.
/// - The internal dispatch chain (`__kernel`, `__router`, state
///   functions, transitions, cascades) stays synchronous. Users pay
///   `.get()` only at the interface boundary; deep
///   `.get()`-everywhere chains would be noisy and would not buy
///   concurrency since `CompletableFuture.completedFuture` is
///   already-resolved by construction.
/// - The constructor fires the start-state's `$>` synchronously (the
///   default Java emission). Other async backends defer this to a
///   separate `init()` so the caller can `await` it; Java's sync
///   internals make that two-phase split unnecessary.
/// - An `init()` method is still emitted for API parity with the
///   other async backends — so a user can write
///   `system.init().get()` portably across languages — but its body
///   is a no-op (returns an already-completed future). The
///   constructor has already done the work.
pub(super) fn make_java_interface_async(class_node: &mut CodegenNode, system: &SystemAst) {
    let async_names: std::collections::HashSet<String> = system
        .interface
        .iter()
        .filter(|m| m.is_async)
        .map(|m| m.name.clone())
        .collect();
    if let CodegenNode::Class {
        ref mut methods, ..
    } = class_node
    {
        for method in methods.iter_mut() {
            if let CodegenNode::Method { is_async, name, .. } = method {
                if async_names.contains(name) {
                    *is_async = true;
                }
            }
        }

        // Emit `init()` as an API-parity no-op. The constructor
        // already drove the start-state cascade.
        let _ = system;
        methods.push(CodegenNode::Method {
            name: "init".to_string(),
            params: vec![],
            return_type: None,
            body: vec![CodegenNode::NativeBlock {
                code: "return java.util.concurrent.CompletableFuture.completedFuture(null);"
                    .to_string(),
                span: None,
            }],
            is_async: true,
            is_static: false,
            visibility: Visibility::Public,
            decorators: vec![],
        });
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
    // Java path is structurally different — interface methods only, sync
    // internals, no-op init().
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
                if name == "__prepareEnter" || name == "__prepareExit" || name == "__hsm_chain" {
                    continue;
                }
                // Skip persist machinery — save_state / restore_state and
                // the recursive helpers (__serComp / __deserComp /
                // __convertJsonValue and case-variant aliases) are pure
                // data operations on the compartment chain. They don't
                // dispatch events. Defects D16 (Swift) and D17 (cpp_23)
                // surfaced here: marking save_state async on Swift caused
                // its calls to __serComp/__deserComp to lack the matching
                // `await` (those calls don't match the dispatch-call
                // pattern in `add_await_to_string`); on cpp_23, marking
                // save_state as a coroutine triggered the
                // `rewrite_return_to_co_return` pass to also rewrite
                // `return` statements inside the embedded `__ser` lambda,
                // breaking compile because the lambda's nlohmann::json
                // return type isn't a coroutine type. Keeping persist
                // sync resolves both.
                if name == "save_state"
                    || name == "saveState"
                    || name == "SaveState"
                    || name == "restore_state"
                    || name == "restoreState"
                    || name == "RestoreState"
                    || name == "__serComp"
                    || name == "__SerComp"
                    || name == "__deserComp"
                    || name == "__DeserComp"
                    || name == "__convertJsonValue"
                    || name == "__convertJsonArray"
                    || name == "__convertJsonObject"
                    || name == "_restore"
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
                // RFC-0020: __kernel takes &Rc<FrameEvent>; FrameContext
                // holds an Rc-wrapped event. Wrap the synthesized $>
                // before pushing the context and dispatching.
                // RFC-0025 Track B.1: $> is the FrameEnter variant
                // (lifecycle args are empty for the no-args async case).
                r#"let __e = std::rc::Rc::new({s}FrameEvent::FrameEnter {{ args: Vec::new() }});
let __ctx = {s}FrameContext::new(std::rc::Rc::clone(&__e), None);
self._context_stack.push(__ctx);
self.__kernel(&__e).await;
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
    if let Some(CodegenNode::NativeBlock { code, .. }) = body
        .iter_mut()
        .rev()
        .find(|n| matches!(n, CodegenNode::NativeBlock { .. }))
    {
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
        let trimmed_semi = line.trim_end_matches(';');
        format!("{}.await;", trimmed_semi)
    }
}
