//! `@@:event` / `@@:data` / `@@:params` / `@@:system` context-
//! sigil expansion (the bare-access forms; `@@:return` lives
//! in `context_return.rs`, `@@:self` in `context_self.rs`).
//!
//! Each arm emits the per-target accessor for one piece of
//! ambient handler context:
//!
//! - `expand_context_event` (`@@:event`) — the current event
//!   message identifier.
//! - `expand_context_data` (`@@:data` / `@@:data.key`) —
//!   user-attached event payload map / dict lookup.
//! - `expand_context_data_assign` (`@@:data = expr`) — writes
//!   the event payload (used by handlers that re-emit).
//! - `expand_context_params` (`@@:params` / `@@:params.key`) —
//!   handler-parameter dict lookup.
//! - `expand_context_system_bare` (`@@:system`) — the running
//!   system instance reference.

use super::expand_expression;
use super::super::codegen_utils::{to_snake_case, HandlerContext};
use super::utility::extract_dot_key;
use crate::frame_c::compiler::native_region_scanner::{RegionSpan, SegmentMetadata};
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn expand_context_event(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:event - interface event name
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    "self._context_stack[-1].event._message".to_string()
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    "this._context_stack[this._context_stack.length - 1].event._message".to_string()
                }
                TargetLanguage::C => format!("{}_CTX(self)->event->_message", ctx.system_name),
                // Rust: handlers receive __e as parameter, use it directly to avoid borrow conflicts
                TargetLanguage::Rust => super::super::rust_system::rust_event_message(),
                TargetLanguage::Cpp => "_context_stack.back()._event._message".to_string(),
                TargetLanguage::Java => {
                    "_context_stack.get(_context_stack.size() - 1)._event._message".to_string()
                }
                TargetLanguage::Kotlin => {
                    "_context_stack[_context_stack.size - 1]._event._message".to_string()
                }
                TargetLanguage::Swift => {
                    "_context_stack[_context_stack.count - 1]._event._message".to_string()
                }
                TargetLanguage::CSharp => {
                    "_context_stack[_context_stack.Count - 1]._event._message".to_string()
                }
                TargetLanguage::Go => {
                    "s._context_stack[len(s._context_stack)-1]._event._message".to_string()
                }
                TargetLanguage::Php => {
                    "$this->_context_stack[count($this->_context_stack) - 1]->_event->_message"
                        .to_string()
                }
                TargetLanguage::Ruby => {
                    "@_context_stack[@_context_stack.length - 1]._event._message".to_string()
                }
                TargetLanguage::Lua => {
                    "self._context_stack[#self._context_stack]._event._message".to_string()
                }
                TargetLanguage::Erlang => {
                    let event_atom = to_snake_case(&ctx.event_name);
                    format!("{}", event_atom)
                }
                TargetLanguage::Graphviz => unreachable!(),
            }
}

pub(super) fn expand_context_data(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:data.key - call-scoped data (read)
            let key = if let SegmentMetadata::ContextData { key, .. } = metadata {
                key.clone()
            } else {
                extract_dot_key(&segment_text, "@@:data") // fallback
            };
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!("self._context_stack[-1]._data[\"{}\"]", key)
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    format!(
                        "this._context_stack[this._context_stack.length - 1]._data[\"{}\"]",
                        key
                    )
                }
                TargetLanguage::C => format!("{}_DATA(self, \"{}\")", ctx.system_name, key),
                TargetLanguage::Rust => {
                    super::super::rust_system::rust_context_data_get(&key, &ctx.system_name)
                }
                TargetLanguage::Cpp => format!("_context_stack.back()._data[\"{}\"]", key),
                TargetLanguage::Java => format!(
                    "_context_stack.get(_context_stack.size() - 1)._data.get(\"{}\")",
                    key
                ),
                TargetLanguage::Kotlin => {
                    format!("_context_stack[_context_stack.size - 1]._data[\"{}\"]", key)
                }
                TargetLanguage::Swift => format!(
                    "_context_stack[_context_stack.count - 1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::CSharp => format!(
                    "_context_stack[_context_stack.Count - 1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Go => format!(
                    "s._context_stack[len(s._context_stack)-1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Php => format!(
                    "$this->_context_stack[count($this->_context_stack) - 1]->_data[\"{}\"]",
                    key
                ),
                TargetLanguage::Ruby => format!(
                    "@_context_stack[@_context_stack.length - 1]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Lua => format!(
                    "self._context_stack[#self._context_stack]._data[\"{}\"]",
                    key
                ),
                TargetLanguage::Erlang => "undefined".to_string(), // gen_statem has no context data
                TargetLanguage::Graphviz => unreachable!(),
            }
}

pub(super) fn expand_context_data_assign(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:data[key] = expr - call-scoped data (assignment)
            // Extract key and value from "@@:data.key = expr;"
            let key = if let SegmentMetadata::ContextData { key, .. } = metadata {
                key.clone()
            } else {
                extract_dot_key(&segment_text, "@@:data") // fallback
            };
            // Find the = and extract the expression
            let trimmed = segment_text.trim();
            let eq_pos = trimmed.find('=').unwrap_or(trimmed.len());
            let expr = trimmed[eq_pos + 1..].trim().trim_end_matches(';').trim();
            let expanded_expr = expand_expression(expr, lang, ctx);
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => format!("{}self._context_stack[-1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => format!("{}this._context_stack[this._context_stack.length - 1]._data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::C => format!("{}{}_DATA_SET(self, \"{}\", {});", indent_str, ctx.system_name, key, expanded_expr),
                TargetLanguage::Rust => super::super::rust_system::rust_expand_context_data_write(
                    &indent_str, &key, &expanded_expr, &ctx.system_name,
                ),
                TargetLanguage::Cpp => format!("{}_context_stack.back()._data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::Java => format!("{}_context_stack.get(_context_stack.size() - 1)._data.put(\"{}\", {});", indent_str, key, expanded_expr),
                TargetLanguage::Kotlin => format!("{}_context_stack[_context_stack.size - 1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Swift => format!("{}_context_stack[_context_stack.count - 1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::CSharp => format!("{}_context_stack[_context_stack.Count - 1]._data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::Go => format!("{}s._context_stack[len(s._context_stack)-1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Php => format!("{}$this->_context_stack[count($this->_context_stack) - 1]->_data[\"{}\"] = {};", indent_str, key, expanded_expr),
                TargetLanguage::Ruby => format!("{}@_context_stack[@_context_stack.length - 1]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Lua => format!("{}self._context_stack[#self._context_stack]._data[\"{}\"] = {}", indent_str, key, expanded_expr),
                TargetLanguage::Erlang => format!("{}ok", indent_str), // gen_statem has no context data
                TargetLanguage::Graphviz => unreachable!(),
            }
}

pub(super) fn expand_context_params(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // @@:params.<key> — handler-interface parameter access.
            //
            // Every target's state-dispatch prologue binds the declared
            // params as TYPED locals at the top of the handler body
            // (e.g., `x := __e._parameters[0].(int)` for Go,
            // `let x = __e.parameters[0] as! Int` for Swift, etc.).
            // Emitting `@@:params.x` as the raw `_parameters[idx]`
            // access (what this branch used to do) dropped the type
            // info and failed in typed targets as soon as the value
            // hit an arithmetic operator or a typed-method arg —
            // especially when nested inside another Frame construct
            // like `@@:self.typed_method(@@:params.x)`.
            //
            // The correct translation is the declared param name
            // itself: it is the already-typed local the prologue
            // bound. Dynamic targets (Python, JS, Ruby, …) see the
            // same name — just an ordinary local variable.
            //
            // Erlang's handler dispatch binds params with the
            // capitalized variant (`X` = the param `x`), matching
            // Erlang's variable-identifier rule.
            let key = if let SegmentMetadata::ContextParams { key } = metadata {
                key.clone()
            } else {
                extract_dot_key(&segment_text, "@@:params") // fallback
            };
            match lang {
                TargetLanguage::Erlang => {
                    // Erlang bindings use the capitalized form (framec's
                    // dispatch prologue rebinds `x` as `X = maps:get(...)`).
                    let mut chars = key.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                }
                // PHP identifies locals with a `$` prefix. The handler
                // prologue binds the param as `$x = $__e->_parameters[0]`.
                TargetLanguage::Php => format!("${}", key),
                TargetLanguage::Graphviz => unreachable!(),
                _ => key,
            }
}

pub(super) fn expand_context_system_bare(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

            // Bare @@:system — should have been caught by validator (E604)
            "/* ERROR: bare @@:system */".to_string()
}
