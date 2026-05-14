//! `@@:self.method()` transition-guard emission.
//!
//! When a handler calls `@@:self.method(args)` it re-enters
//! dispatch on the system, which may itself transition. After the
//! call returns, the calling handler must bail if a transition
//! happened — otherwise subsequent statements would run in the
//! wrong state. The dynamic-language backends set a
//! `_transitioned` flag on the top FrameContext; this helper
//! emits the per-target read+early-return at the call site.
//!
//! One subtlety: a `$>` / `<$` lifecycle handler runs inside the
//! enter/exit cascade with NO FrameContext on the stack (the
//! cascade's pre-handler push is skipped for lifecycle invokes —
//! see `__frame_init`). The guard's first check is therefore
//! "stack non-empty" — otherwise a `@@:self.x()` from a `$>`
//! body would read past `[-1]` and crash construction.

use crate::frame_c::visitors::TargetLanguage;

/// Generate the transition guard check for a self-call.
/// Emitted by the orchestrator AFTER the line containing the self-call
/// expression, on its own line at the given indentation.
///
/// The guard reads the top FrameContext's `_transitioned` flag to bail the
/// calling handler if the re-entrant `@@:self.method(...)` dispatch transitioned
/// the machine. Every backend's guard first checks the context stack is
/// non-empty: an event handler always runs inside its own FrameContext, but a
/// `$>`/`<$` lifecycle handler runs inside the enter/exit cascade with **no**
/// FrameContext pushed (e.g. the initial `__frame_init` cascade), so an
/// unconditional `_context_stack[-1]` read is out of bounds there → crash at
/// `@@System()` construction. The non-empty check makes the guard a safe no-op
/// in that case (a transition queued from a lifecycle handler is drained by the
/// cascade's own `__process_transition_loop`, not by this guard). See RFC-0018
/// for the open question of what re-entrant interface dispatch from a lifecycle
/// handler *should* mean.
pub(crate) fn generate_self_call_guard(
    indent: usize,
    lang: TargetLanguage,
    system_name: &str,
) -> String {
    let ind = " ".repeat(indent);
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => format!(
            "{}if self._context_stack and self._context_stack[-1]._transitioned:\n{}    return",
            ind, ind
        ),
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => format!(
            "{}if (this._context_stack.length > 0 && this._context_stack[this._context_stack.length - 1]._transitioned) return;",
            ind
        ),
        TargetLanguage::Dart => format!(
            "{}if (this._context_stack.isNotEmpty && this._context_stack[this._context_stack.length - 1]._transitioned) return;",
            ind
        ),
        TargetLanguage::Rust => format!(
            "{}if self._context_stack.last().map_or(false, |ctx| ctx._transitioned) {{ return; }}",
            ind
        ),
        TargetLanguage::C => format!(
            "{}if ({}_FrameVec_size(self->_context_stack) > 0 && {}_CTX(self)->_transitioned) return;",
            ind, system_name, system_name
        ),
        TargetLanguage::Cpp => format!(
            "{}if (!_context_stack.empty() && _context_stack.back()._transitioned) return;",
            ind
        ),
        TargetLanguage::Java => format!(
            "{}if (!_context_stack.isEmpty() && _context_stack.get(_context_stack.size() - 1)._transitioned) return;",
            ind
        ),
        TargetLanguage::Kotlin => format!(
            "{}if (_context_stack.isNotEmpty() && _context_stack[_context_stack.size - 1]._transitioned) return",
            ind
        ),
        TargetLanguage::Swift => format!(
            "{}if !_context_stack.isEmpty && _context_stack[_context_stack.count - 1]._transitioned {{ return }}",
            ind
        ),
        TargetLanguage::CSharp => format!(
            "{}if (_context_stack.Count > 0 && _context_stack[_context_stack.Count - 1]._transitioned) return;",
            ind
        ),
        TargetLanguage::Go => format!(
            "{}if len(s._context_stack) > 0 && s._context_stack[len(s._context_stack)-1]._transitioned {{ return }}",
            ind
        ),
        TargetLanguage::Php => format!(
            "{}if (count($this->_context_stack) > 0 && $this->_context_stack[count($this->_context_stack) - 1]->_transitioned) return;",
            ind
        ),
        TargetLanguage::Ruby => format!(
            "{}return if !@_context_stack.empty? && @_context_stack[@_context_stack.length - 1]._transitioned",
            ind
        ),
        TargetLanguage::Lua => format!(
            "{}if #self._context_stack > 0 and self._context_stack[#self._context_stack]._transitioned then return end",
            ind
        ),
        TargetLanguage::Erlang | TargetLanguage::Graphviz => String::new(),
    }
}
