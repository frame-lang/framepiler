//! `push$` / `pop$` modal-stack frame segment expansion.
//!
//! Two arms:
//!
//! - `expand_stack_push` — RFC-0008 `push$` (with optional
//!   transition target). Saves a REFERENCE to the current
//!   compartment on the state stack; the GC backends do a
//!   direct assignment, C/C++ do ref-count increments, Rust
//!   uses `clone` for bare push or `mem::replace` for
//!   push-with-transition.
//! - `expand_stack_pop` — bare `pop$` (no transition). Just
//!   pops the top of the stack and discards. The `-> pop$`
//!   form (pop-with-transition) is handled by
//!   `pop_transition::generate_pop_transition`.

use super::super::codegen_utils::{to_snake_case, HandlerContext};
use crate::frame_c::compiler::native_region_scanner::{RegionSpan, SegmentMetadata};
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn expand_stack_push(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

    let target = match metadata {
        SegmentMetadata::StackPush {
            transition_target: Some(t),
        } => t.clone(),
        _ => String::new(),
    };

    // push$ saves a REFERENCE to the current compartment on the
    // state stack — not a copy. In GC languages this is a direct
    // assignment. In C it's a pointer save (ownership transfers to
    // stack on push-with-transition). In C++ it's a shared_ptr
    // copy (ref count increment). In Rust, clone is required for
    // bare push$ (ownership model) but push-with-transition uses
    // mem::replace (ownership transfer). pop$ restores the saved
    // reference as the current compartment.
    match lang {
        TargetLanguage::Python3 => {
            let push_code = format!("{}self._state_stack.append(self.__compartment)", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}self._transition(\"{}\", None, None)",
                    push_code, indent_str, target
                )
            } else {
                push_code
            }
        }
        TargetLanguage::GDScript => {
            let push_code = format!("{}self._state_stack.append(self.__compartment)", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}self._transition(\"{}\", null, null)",
                    push_code, indent_str, target
                )
            } else {
                push_code
            }
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
            let push_code = format!("{}this._state_stack.push(this.__compartment);", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}this._transition(\"{}\", null, null);",
                    push_code, indent_str, target
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Dart => {
            let push_code = format!("{}this._state_stack.add(this.__compartment);", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}this.__transition({}Compartment(\"{}\"));\n{}return;",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Rust => {
            if !target.is_empty() {
                super::super::rust_system::rust_push_transition(&indent_str, ctx, &target)
            } else {
                super::super::rust_system::rust_bare_push(&indent_str)
            }
        }
        TargetLanguage::C => {
            // C: save reference via ref count increment. The stack
            // holds a ref'd pointer. The kernel's _unref on
            // transition won't free it while the stack holds a ref.
            let push_code = format!(
                "{}{}_FrameVec_push(self->_state_stack, {}_Compartment_ref(self->__compartment));",
                indent_str, ctx.system_name, ctx.system_name
            );
            if !target.is_empty() {
                format!(
                    "{}\n{}{}_transition(self, {}_Compartment_new(\"{}\"));",
                    push_code, indent_str, ctx.system_name, ctx.system_name, target
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Cpp => {
            // C++: shared_ptr reference save (ref count increment).
            let push_code = format!("{}_state_stack.push_back(__compartment);", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}__transition(std::make_shared<{}Compartment>(\"{}\"));\n{}return;",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Java => {
            let push_code = format!("{}_state_stack.add(__compartment);", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}__transition(new {}Compartment(\"{}\"));\n{}return;",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Kotlin => {
            let push_code = format!("{}_state_stack.add(__compartment)", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}__transition({}Compartment(\"{}\"))\n{}return",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Swift => {
            let push_code = format!("{}_state_stack.append(__compartment)", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}__transition({}Compartment(state: \"{}\"))\n{}return",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Go => {
            let push_code = format!(
                "{}s._state_stack = append(s._state_stack, s.__compartment)",
                indent_str
            );
            if !target.is_empty() {
                format!(
                    "{}\n{}s.__transition(new{}Compartment(\"{}\"))\n{}return",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::CSharp => {
            let push_code = format!("{}_state_stack.Add(__compartment);", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}__transition(new {}Compartment(\"{}\"));\n{}return;",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Php => {
            let push_code = format!(
                "{}$this->_state_stack[] = $this->__compartment;",
                indent_str
            );
            if !target.is_empty() {
                format!(
                    "{}\n{}$this->__transition(new {}Compartment(\"{}\"));\n{}return;",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Ruby => {
            let push_code = format!("{}@_state_stack.push(@__compartment)", indent_str);
            if !target.is_empty() {
                format!(
                    "{}\n{}__transition({}Compartment.new(\"{}\"))\n{}return",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Lua => {
            let push_code = format!(
                "{}self._state_stack[#self._state_stack + 1] = self.__compartment",
                indent_str
            );
            if !target.is_empty() {
                format!(
                    "{}\n{}self:__transition({}Compartment.new(\"{}\"))\n{}return",
                    push_code, indent_str, ctx.system_name, target, indent_str
                )
            } else {
                push_code
            }
        }
        TargetLanguage::Erlang => {
            // Save the current compartment context (state name +
            // state-args + enter-args) as a 3-tuple onto
            // frame_stack. The two args fields here MUST stay
            // in lockstep with `ERLANG_COMPARTMENT_CONTEXT_FIELDS`
            // in `erlang_system.rs` — that constant is the
            // canonical list of context fields and persist's
            // save_state/load_state iterates the same list.
            // If you add a context field to the Data record,
            // update both spots.
            //
            // Pre-state-args codegen saved only the state atom,
            // discarding the compartment's positional args; a
            // `-> pop$` back to a state declared `(x: int)`
            // saw undefined args. Surfaced by Phase 19 wave 3
            // P7 (state_args_round_trip).
            let state_atom = to_snake_case(&ctx.state_name);
            if !target.is_empty() {
                let target_atom = to_snake_case(&target);
                format!(
                            "{}self.frame_stack = [{{{}, self.frame_state_args, self.frame_enter_args}} | self.frame_stack]\n{}{{next_state, {}, Data, [{{reply, From, ok}}]}}",
                            indent_str, state_atom, indent_str, target_atom
                        )
            } else {
                format!(
                            "{}self.frame_stack = [{{{}, self.frame_state_args, self.frame_enter_args}} | self.frame_stack]",
                            indent_str, state_atom
                        )
            }
        }
        TargetLanguage::Graphviz => unreachable!(),
    }
}

pub(super) fn expand_stack_pop(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

    // Standalone pop$ — pop the top of the stack and discard it.
    // No transition. For transitioning to the popped state, use -> pop$.
    match lang {
        TargetLanguage::Python3 => format!("{}self._state_stack.pop()", indent_str),
        TargetLanguage::GDScript => format!("{}self._state_stack.pop_back()", indent_str),
        TargetLanguage::TypeScript => format!("{}this._state_stack.pop();", indent_str),
        TargetLanguage::JavaScript => format!("{}this._state_stack.pop();", indent_str),
        TargetLanguage::Dart => format!("{}this._state_stack.removeLast();", indent_str),
        TargetLanguage::Rust => super::super::rust_system::rust_bare_pop(&indent_str),
        TargetLanguage::C => format!(
            "{}{}_FrameVec_pop(self->_state_stack);",
            indent_str, ctx.system_name
        ),
        TargetLanguage::Cpp => format!("{}_state_stack.pop_back();", indent_str),
        TargetLanguage::Java => format!(
            "{}_state_stack.remove(_state_stack.size() - 1);",
            indent_str
        ),
        TargetLanguage::Kotlin => {
            format!("{}_state_stack.removeAt(_state_stack.size - 1)", indent_str)
        }
        TargetLanguage::Swift => format!("{}_state_stack.removeLast()", indent_str),
        TargetLanguage::CSharp => format!(
            "{}_state_stack.RemoveAt(_state_stack.Count - 1);",
            indent_str
        ),
        TargetLanguage::Go => format!(
            "{}s._state_stack = s._state_stack[:len(s._state_stack)-1]",
            indent_str
        ),
        TargetLanguage::Php => format!("{}array_pop($this->_state_stack);", indent_str),
        TargetLanguage::Ruby => format!("{}@_state_stack.pop", indent_str),
        TargetLanguage::Lua => format!("{}table.remove(self._state_stack)", indent_str),
        TargetLanguage::Erlang => {
            // Standalone `pop$` (no transition) discards the
            // saved compartment. Match the 3-tuple shape that
            // push$ emits so the pattern bind succeeds.
            format!(
                        "{}[{{_, _, _}} | __RestStack] = self.frame_stack,\n{}self.frame_stack = __RestStack",
                        indent_str, indent_str
                    )
        }
        TargetLanguage::Graphviz => unreachable!(),
    }
}
