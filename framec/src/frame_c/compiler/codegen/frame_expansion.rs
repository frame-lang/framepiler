//! Frame statement expansion and handler body splicing.
//!
//! This module handles the core Frame-to-native-code transformation:
//! - Splicing handler bodies: scanning for Frame statements in native code
//!   and replacing them with target-language expansions
//! - Frame statement expansion: converting -> $State, => $^, push$, pop$,
//!   return sugar, $.var, @@:return, etc. to target language code
//! - Helper functions for extracting transition targets, args, state vars

mod expression;
mod handler_body;
mod no_init;
mod pop_transition;
mod scanner_dispatch;
mod self_call_guard;
mod state_var;
mod transition;
mod utility;

use expression::expand_expression;
use handler_body::context_return_read_typed;
use pop_transition::generate_pop_transition;
use utility::{
    c_return_assign, cpp_wrap_string_literal, is_ident_char, paren_wrap_if_multiline,
    split_transition_return, strip_outer_parens,
};

pub(crate) use handler_body::{emit_handler_body_via_statements, resolve_state_arg_key};
pub(crate) use no_init::generate_no_initialization;
pub(crate) use scanner_dispatch::{
    expand_system_state, expand_system_state_in_code, get_native_scanner,
};
pub(crate) use self_call_guard::generate_self_call_guard;
pub(crate) use utility::{
    extract_dot_key, extract_state_var_name, normalize_indentation, php_prefix_params,
    strip_java_unreachable,
};

use super::codegen_utils::{
    cpp_map_type, cpp_wrap_any_arg, csharp_map_type, expression_to_string, go_map_type,
    java_map_type, kotlin_map_type, replace_outside_strings_and_comments, state_var_init_value,
    swift_map_type, to_snake_case, type_to_cpp_string, HandlerContext,
};
use crate::frame_c::compiler::frame_ast::Type;
use crate::frame_c::compiler::native_region_scanner::{
    c::NativeRegionScannerC, cpp::NativeRegionScannerCpp, csharp::NativeRegionScannerCs,
    dart::NativeRegionScannerDart, erlang::NativeRegionScannerErlang,
    gdscript::NativeRegionScannerGDScript, go::NativeRegionScannerGo,
    java::NativeRegionScannerJava, javascript::NativeRegionScannerJs,
    kotlin::NativeRegionScannerKotlin, lua::NativeRegionScannerLua, php::NativeRegionScannerPhp,
    python::NativeRegionScannerPy, ruby::NativeRegionScannerRuby, rust::NativeRegionScannerRust,
    swift::NativeRegionScannerSwift, typescript::NativeRegionScannerTs, FrameSegmentKind,
    NativeRegionScanner, Region, SegmentMetadata,
};
use crate::frame_c::compiler::splice::Splicer;
use crate::frame_c::visitors::TargetLanguage;



/// Generate code expansion for a Frame segment
///
/// NOTE: The scanner leaves a gap between NativeText and FrameSegment where leading
/// whitespace lives. Since the splicer doesn't copy this gap, we MUST include the
/// indentation in the expansion to preserve proper code structure.
pub(crate) fn generate_frame_expansion(
    body_bytes: &[u8],
    span: &crate::frame_c::compiler::native_region_scanner::RegionSpan,
    kind: FrameSegmentKind,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

    match kind {
        FrameSegmentKind::Transition => {
            transition::expand_transition(body_bytes, span, indent, lang, ctx, metadata)
        }
        FrameSegmentKind::Forward => {
            // HSM forward: call parent state's handler for the same event
            if let Some(ref parent) = ctx.parent_state {
                match lang {
                    // Python/TypeScript: call _state_Parent(__e) to dispatch via unified state method
                    TargetLanguage::Python3 | TargetLanguage::GDScript => {
                        if ctx.per_handler {
                            // Per-handler architecture: shift compartment up one
                            // level at the forward site so the parent dispatcher
                            // sees its own compartment as the param.
                            format!(
                                "{}self._state_{}(__e, compartment.parent_compartment)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}self._state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::TypeScript
                    | TargetLanguage::Dart
                    | TargetLanguage::JavaScript => {
                        if ctx.per_handler {
                            // Dart and TypeScript are null-safe; assert
                            // non-null with `!`. JavaScript ignores the
                            // postfix annotation in runtime semantics —
                            // keep it bare.
                            let bang = if matches!(
                                lang,
                                TargetLanguage::Dart | TargetLanguage::TypeScript
                            ) {
                                "!"
                            } else {
                                ""
                            };
                            format!(
                                "{}this._state_{}(__e, compartment.parent_compartment{});",
                                indent_str, parent, bang
                            )
                        } else {
                            format!("{}this._state_{}(__e);", indent_str, parent)
                        }
                    }
                    // Rust: call parent state router (not specific handler) to dispatch via match
                    TargetLanguage::Rust => {
                        super::rust_system::rust_parent_forward(&indent_str, parent)
                    }
                    // C: call System_state_Parent(self, __e, parent_compartment)
                    // — per-handler architecture shifts the compartment up
                    // one level at the forward site.
                    TargetLanguage::C => {
                        if ctx.per_handler {
                            format!(
                                "{}{}_state_{}(self, __e, compartment->parent_compartment);",
                                indent_str, ctx.system_name, parent
                            )
                        } else {
                            format!(
                                "{}{}_state_{}(self, __e);",
                                indent_str, ctx.system_name, parent
                            )
                        }
                    }
                    // C++: call _state_Parent(__e, compartment->parent_compartment)
                    // — forward is not terminal, no return.
                    TargetLanguage::Cpp => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment->parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e);", indent_str, parent)
                        }
                    }
                    // Java: per-handler architecture passes compartment as
                    // second arg; shift up one level at the forward site.
                    TargetLanguage::Java => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e);", indent_str, parent)
                        }
                    }
                    TargetLanguage::CSharp => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e);", indent_str, parent)
                        }
                    }
                    TargetLanguage::Kotlin => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment!!)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Swift => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment!)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e)", indent_str, parent)
                        }
                    }
                    // Go: call s._state_Parent(__e) — forward is not terminal, no return
                    TargetLanguage::Go => {
                        if ctx.per_handler {
                            format!(
                                "{}s._state_{}(__e, compartment.parentCompartment)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}s._state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Php => {
                        if ctx.per_handler {
                            format!(
                                "{}$this->_state_{}($__e, $compartment->parent_compartment);",
                                indent_str, parent
                            )
                        } else {
                            format!("{}$this->_state_{}($__e);", indent_str, parent)
                        }
                    }
                    TargetLanguage::Ruby => {
                        if ctx.per_handler {
                            format!(
                                "{}_state_{}(__e, compartment.parent_compartment)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}_state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Lua => {
                        if ctx.per_handler {
                            format!(
                                "{}self:_state_{}(__e, compartment.parent_compartment)",
                                indent_str, parent
                            )
                        } else {
                            format!("{}self:_state_{}(__e)", indent_str, parent)
                        }
                    }
                    TargetLanguage::Erlang => {
                        let parent_atom = to_snake_case(parent);
                        match ctx.event_name.as_str() {
                            // RFC-0019: `=> $^` inside a `$>` / `<$` handler
                            // runs the parent's lifecycle code by calling its
                            // `frame_enter__<P>(Data) -> Data` /
                            // `frame_exit__<P>(Data) -> Data` helper. The body
                            // processor threads the returned Data record. (The
                            // `($>`/`<$` callbacks in gen_statem are `enter` /
                            // `frame_exit_dispatch__`, not regular events — so
                            // there's no `{call, From}` to re-dispatch here.)
                            "$>" => format!("{}frame_enter__{}(Data)", indent_str, parent_atom),
                            "<$" => format!("{}frame_exit__{}(Data)", indent_str, parent_atom),
                            // Ordinary event handler: delegate to the parent
                            // state function with the same `{call, From}` so
                            // the parent's reply reaches the original caller.
                            other => {
                                let event_atom = to_snake_case(other);
                                format!(
                                    "{}{}({{call, From}}, {}, Data)",
                                    indent_str, parent_atom, event_atom
                                )
                            }
                        }
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                // No parent state - just return (shouldn't happen in valid HSM)
                match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => {
                        format!("{}return  # Forward to parent (no parent)", indent_str)
                    }
                    TargetLanguage::Ruby
                    | TargetLanguage::Kotlin
                    | TargetLanguage::Swift
                    | TargetLanguage::Lua => {
                        format!("{}return // Forward to parent (no parent)", indent_str)
                    }
                    TargetLanguage::TypeScript
                    | TargetLanguage::JavaScript
                    | TargetLanguage::Rust
                    | TargetLanguage::Dart
                    | TargetLanguage::C
                    | TargetLanguage::Cpp
                    | TargetLanguage::Java
                    | TargetLanguage::CSharp
                    | TargetLanguage::Go
                    | TargetLanguage::Php => {
                        format!("{}return; // Forward to parent (no parent)", indent_str)
                    }
                    TargetLanguage::Erlang => {
                        format!("{}{{keep_state, Data}}", indent_str)
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            }
        }
        FrameSegmentKind::StackPush => {
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
                    let push_code =
                        format!("{}self._state_stack.append(self.__compartment)", indent_str);
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
                    let push_code =
                        format!("{}self._state_stack.append(self.__compartment)", indent_str);
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
                    let push_code =
                        format!("{}this._state_stack.push(this.__compartment);", indent_str);
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
                    let push_code =
                        format!("{}this._state_stack.add(this.__compartment);", indent_str);
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
                        super::rust_system::rust_push_transition(&indent_str, ctx, &target)
                    } else {
                        super::rust_system::rust_bare_push(&indent_str)
                    }
                }
                TargetLanguage::C => {
                    // C: save reference via ref count increment. The stack
                    // holds a ref'd pointer. The kernel's _unref on
                    // transition won't free it while the stack holds a ref.
                    let push_code = format!("{}{}_FrameVec_push(self->_state_stack, {}_Compartment_ref(self->__compartment));",
                        indent_str, ctx.system_name, ctx.system_name);
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
                        format!("{}\n{}__transition(std::make_shared<{}Compartment>(\"{}\"));\n{}return;",
                            push_code, indent_str, ctx.system_name, target, indent_str)
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
        FrameSegmentKind::StackPop => {
            // Standalone pop$ — pop the top of the stack and discard it.
            // No transition. For transitioning to the popped state, use -> pop$.
            match lang {
                TargetLanguage::Python3 => format!("{}self._state_stack.pop()", indent_str),
                TargetLanguage::GDScript => format!("{}self._state_stack.pop_back()", indent_str),
                TargetLanguage::TypeScript => format!("{}this._state_stack.pop();", indent_str),
                TargetLanguage::JavaScript => format!("{}this._state_stack.pop();", indent_str),
                TargetLanguage::Dart => format!("{}this._state_stack.removeLast();", indent_str),
                TargetLanguage::Rust => super::rust_system::rust_bare_pop(&indent_str),
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
        FrameSegmentKind::StateVar => {
            state_var::expand_state_var(body_bytes, span, indent, lang, ctx, metadata)
        }
        FrameSegmentKind::StateVarAssign => {
            state_var::expand_state_var_assign(body_bytes, span, indent, lang, ctx, metadata)
        }
        FrameSegmentKind::ContextReturn => {
            // @@:return - return value slot (assignment or read)
            // Determine if this is assignment or read from metadata (preferred) or text
            let is_assignment = if let SegmentMetadata::ContextReturn { assign_expr } = metadata {
                assign_expr.is_some()
            } else {
                let t = segment_text.trim();
                t.contains('=') && !t.contains("==")
            };
            let trimmed = segment_text.trim();
            if is_assignment {
                // Assignment: @@:return = expr
                let expr = if let SegmentMetadata::ContextReturn {
                    assign_expr: Some(e),
                } = metadata
                {
                    e.as_str()
                } else {
                    let eq_pos = trimmed.find('=').unwrap();
                    trimmed[eq_pos + 1..].trim().trim_end_matches(';').trim()
                };
                let expanded_expr = paren_wrap_if_multiline(&expand_expression(expr, lang, ctx));
                match lang {
                    TargetLanguage::Python3 | TargetLanguage::GDScript => format!(
                        "{}self._context_stack[-1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::TypeScript
                    | TargetLanguage::Dart
                    | TargetLanguage::JavaScript => format!(
                        "{}this._context_stack[this._context_stack.length - 1]._return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::C => format!(
                        "{}{}",
                        indent_str,
                        c_return_assign(&ctx.system_name, &expanded_expr, &ctx.current_return_type),
                    ),
                    TargetLanguage::Rust => super::rust_system::rust_expand_box_return(
                        &indent_str,
                        &expanded_expr,
                        &ctx.current_return_type,
                    ),
                    TargetLanguage::Cpp => {
                        let wrapped = cpp_wrap_string_literal(&expanded_expr);
                        format!(
                            "{}_context_stack.back()._return = std::any({});",
                            indent_str, wrapped
                        )
                    }
                    TargetLanguage::Java => format!(
                        "{}_context_stack.get(_context_stack.size() - 1)._return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Kotlin => format!(
                        "{}_context_stack[_context_stack.size - 1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Swift => format!(
                        "{}_context_stack[_context_stack.count - 1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::CSharp => format!(
                        "{}_context_stack[_context_stack.Count - 1]._return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Go => {
                        // Go's generated methods use `s` as the receiver
                        // name, not `self`. Rewrite `self.` → `s.` via the
                        // string-literal-aware helper so a `self.` that
                        // happens to appear inside a string literal or
                        // comment isn't mangled.
                        let go_expr = replace_outside_strings_and_comments(
                            &expanded_expr,
                            TargetLanguage::Go,
                            &[("self.", "s.")],
                        );
                        format!(
                            "{}s._context_stack[len(s._context_stack)-1]._return = {}",
                            indent_str, go_expr
                        )
                    }
                    TargetLanguage::Php => format!(
                        "{}$this->_context_stack[count($this->_context_stack) - 1]->_return = {};",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Ruby => format!(
                        "{}@_context_stack[@_context_stack.length - 1]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Lua => format!(
                        "{}self._context_stack[#self._context_stack]._return = {}",
                        indent_str, expanded_expr
                    ),
                    TargetLanguage::Erlang => {
                        // Leave `self.` in the expression untouched —
                        // the Erlang body processor classifies this as
                        // Plain and substitutes `self.` with the CURRENT
                        // `DataN#data.` using the live data_gen. A
                        // hardcoded `Data#data.` here would bind to the
                        // pre-handler Data and miss updates made earlier
                        // in the handler body (e.g., by `self.x = ...`
                        // or a preceding `@@:self` dispatch).
                        format!("{}__ReturnVal = {}", indent_str, expanded_expr)
                    }
                    TargetLanguage::Graphviz => unreachable!(),
                }
            } else {
                // Read: @@:return.
                //
                // The context-stack slot is an untyped `Any` / `Object`
                // / `void*` / `std::any` / `Option<Box<dyn Any>>` in
                // every typed target. Reading `@@:return` as that raw
                // slot fails as soon as the value hits an arithmetic
                // operator or a typed-method argument. Emit a
                // target-native downcast based on the handler's
                // declared return type (`ctx.current_return_type`)
                // so the read evaluates to a typed rvalue.
                //
                // Dynamic-typed targets (Python, JS, Ruby, Lua, PHP,
                // Dart, GDScript) need no cast.
                let rt = ctx.current_return_type.as_deref().unwrap_or("");
                context_return_read_typed(lang, rt, &ctx.system_name)
            }
        }
        FrameSegmentKind::ContextReturnExpr => {
            // @@:(expr) - set context return value (concise form).
            // The scanner extends the segment span to consume any
            // trailing whitespace + `return` + `;` on the same source
            // line, so when `@@:(expr) return;` appears as a single
            // line in the source, this expansion emits BOTH the
            // assignment to the return slot AND the native return
            // statement on separate lines, properly indented.
            //
            // Detect whether the scanner consumed a trailing `return`
            // by looking for the bare `return` keyword in segment_text
            // outside of the `@@:(...)` expression.
            // Extract expression from metadata (preferred) or raw text (fallback)
            let trimmed = segment_text.trim();
            let (expr, has_native_return) = if let SegmentMetadata::ReturnExpr { expr } = metadata {
                // Check for trailing `return` in the segment text
                // (the metadata only has the expression, not the trailing keyword)
                let has_ret = if let Some(close_pos) = trimmed.rfind(')') {
                    let tail = trimmed[close_pos + 1..].trim();
                    tail.starts_with("return")
                        && (tail.len() == 6
                            || tail
                                .as_bytes()
                                .get(6)
                                .map_or(true, |b| b.is_ascii_whitespace() || *b == b';'))
                } else {
                    false
                };
                (expr.clone(), has_ret)
            } else {
                // Fallback: parse from raw text
                if let Some(start) = trimmed.find("@@:(") {
                    let after_open = start + 4;
                    let bytes = trimmed.as_bytes();
                    let mut depth = 1i32;
                    let mut p = after_open;
                    while p < bytes.len() && depth > 0 {
                        match bytes[p] {
                            b'(' => depth += 1,
                            b')' => depth -= 1,
                            _ => {}
                        }
                        if depth > 0 {
                            p += 1;
                        }
                    }
                    let expr_str = trimmed[after_open..p].to_string();
                    let after_close = if p < bytes.len() { p + 1 } else { p };
                    let tail = trimmed[after_close..].trim();
                    let has_ret = tail.starts_with("return")
                        && (tail.len() == 6
                            || tail.as_bytes()[6].is_ascii_whitespace()
                            || tail.as_bytes()[6] == b';');
                    (expr_str, has_ret)
                } else {
                    (trimmed.to_string(), false)
                }
            };
            let expanded_expr = paren_wrap_if_multiline(&expand_expression(expr.trim(), lang, ctx));
            // Standalone @@ constructs include indent_str on all lines.
            // The scanner trims trailing whitespace from preceding native
            // text for standalone constructs (computed_indent > 0), so
            // indent_str reconstructs the correct indentation.
            let assignment = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!(
                        "{}self._context_stack[-1]._return = {}",
                        indent_str, expanded_expr
                    )
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    format!(
                        "this._context_stack[this._context_stack.length - 1]._return = {};",
                        expanded_expr
                    )
                }
                TargetLanguage::C => {
                    c_return_assign(&ctx.system_name, &expanded_expr, &ctx.current_return_type)
                }
                TargetLanguage::Rust => super::rust_system::rust_expand_box_return_bare(
                    &indent_str,
                    &expanded_expr,
                    &ctx.current_return_type,
                ),
                TargetLanguage::Cpp => {
                    let wrapped = cpp_wrap_string_literal(&expanded_expr);
                    format!("_context_stack.back()._return = std::any({});", wrapped)
                }
                TargetLanguage::Java => format!(
                    "_context_stack.get(_context_stack.size() - 1)._return = {};",
                    expanded_expr
                ),
                TargetLanguage::Kotlin => format!(
                    "_context_stack[_context_stack.size - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Swift => format!(
                    "_context_stack[_context_stack.count - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::CSharp => format!(
                    "_context_stack[_context_stack.Count - 1]._return = {};",
                    expanded_expr
                ),
                TargetLanguage::Go => {
                    // String-literal-aware rewrite of `self.` → `s.` —
                    // same fix as the `@@:return = expr` Go branch above.
                    let go_expr = replace_outside_strings_and_comments(
                        &expanded_expr,
                        TargetLanguage::Go,
                        &[("self.", "s.")],
                    );
                    format!(
                        "s._context_stack[len(s._context_stack)-1]._return = {}",
                        go_expr
                    )
                }
                TargetLanguage::Php => format!(
                    "$this->_context_stack[count($this->_context_stack) - 1]->_return = {};",
                    expanded_expr
                ),
                TargetLanguage::Ruby => format!(
                    "@_context_stack[@_context_stack.length - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Lua => format!(
                    "self._context_stack[#self._context_stack]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Erlang => {
                    // Leave `self.` intact so the body processor binds
                    // it to the live data_gen — see the sibling fix in
                    // `FrameSegmentKind::ContextReturn` above.
                    format!("__ReturnVal = {}", expanded_expr)
                }
                TargetLanguage::Graphviz => unreachable!(),
            };
            if has_native_return {
                // Append a `return` statement on its own line at the
                // same indent as the assignment. The indent comes from
                // the segment's `indent` field, which the scanner sets
                // to the column position of the segment in the source.
                // The newline puts us at column 0, then indent_str
                // fills in the source's leading whitespace.
                let ret_line = match lang {
                    TargetLanguage::Python3
                    | TargetLanguage::GDScript
                    | TargetLanguage::Lua
                    | TargetLanguage::Ruby => format!("{}return", indent_str),
                    TargetLanguage::Erlang => String::new(), // Erlang has no native return statement
                    _ => format!("{}return;", indent_str),
                };
                if ret_line.is_empty() {
                    assignment
                } else {
                    format!("{}\n{}", assignment, ret_line)
                }
            } else {
                assignment
            }
        }
        FrameSegmentKind::ContextEvent => {
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
                TargetLanguage::Rust => super::rust_system::rust_event_message(),
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
        FrameSegmentKind::ContextData => {
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
                TargetLanguage::Rust => super::rust_system::rust_context_data_get(&key),
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
        FrameSegmentKind::ContextDataAssign => {
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
                TargetLanguage::Rust => super::rust_system::rust_expand_context_data_write(
                    &indent_str, &key, &expanded_expr,
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
        FrameSegmentKind::ContextParams => {
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
        FrameSegmentKind::SystemInstantiation => {
            // `@@SystemName(args)` (Factory): emitted VERBATIM here so the
            // assembler's `expand_system_instantiations` post-pass rewrites
            // to the per-language constructor call. (Phase 5 of RFC-0015 D7
            // will migrate Factory rewriting into this arm and remove the
            // post-pass; for now we keep the existing behavior.)
            //
            // `@@!SystemName()` (NoInitialization, RFC-0015 D7): emitted
            // here directly as the per-language no-initialization allocation. The
            // assembler's post-pass doesn't recognize `@@!` (the trailing
            // `!` after `@@` short-circuits its uppercase check), so we
            // MUST emit the final form here.
            use crate::frame_c::compiler::frame_ast::InstantiationKind;
            if let SegmentMetadata::SystemInstantiation {
                system_name,
                kind: InstantiationKind::NoInitialization,
                ..
            } = metadata
            {
                generate_no_initialization(system_name, lang)
            } else {
                segment_text.to_string()
            }
        }
        FrameSegmentKind::ReturnCall => {
            // @@:return(expr) — set context return value AND exit handler.
            // This is the "set + return" one-liner. The segment text is
            // `@@:return(expr)` — extract the expression between parens.
            let trimmed = segment_text.trim();
            let expr_owned;
            let expr = if let SegmentMetadata::ReturnCall { expr } = metadata {
                expr.as_str()
            } else {
                // Fallback: parse from raw text
                expr_owned = if let Some(start) = trimmed.find('(') {
                    let inner = &trimmed[start + 1..];
                    if let Some(end) = inner.rfind(')') {
                        inner[..end].trim().to_string()
                    } else {
                        inner.trim().to_string()
                    }
                } else {
                    String::new()
                };
                &expr_owned
            };
            let expanded_expr = paren_wrap_if_multiline(&expand_expression(expr, lang, ctx));

            // Standalone @@ constructs include indent_str on all lines.
            let set_code = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!(
                        "{}self._context_stack[-1]._return = {}",
                        indent_str, expanded_expr
                    )
                }
                TargetLanguage::TypeScript | TargetLanguage::Dart | TargetLanguage::JavaScript => {
                    format!(
                        "this._context_stack[this._context_stack.length - 1]._return = {};",
                        expanded_expr
                    )
                }
                TargetLanguage::C => {
                    c_return_assign(&ctx.system_name, &expanded_expr, &ctx.current_return_type)
                }
                TargetLanguage::Rust => super::rust_system::rust_expand_box_return_bare(
                    &indent_str,
                    &expanded_expr,
                    &ctx.current_return_type,
                ),
                TargetLanguage::Cpp => {
                    let wrapped = cpp_wrap_string_literal(&expanded_expr);
                    format!("_context_stack.back()._return = std::any({});", wrapped)
                }
                TargetLanguage::Java => format!(
                    "_context_stack.get(_context_stack.size() - 1)._return = {};",
                    expanded_expr
                ),
                TargetLanguage::Kotlin => format!(
                    "_context_stack[_context_stack.size - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Swift => format!(
                    "_context_stack[_context_stack.count - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::CSharp => format!(
                    "_context_stack[_context_stack.Count - 1]._return = {};",
                    expanded_expr
                ),
                TargetLanguage::Go => {
                    // String-literal-aware rewrite of `self.` → `s.` —
                    // same fix as the `@@:return = expr` Go branch above.
                    let go_expr = replace_outside_strings_and_comments(
                        &expanded_expr,
                        TargetLanguage::Go,
                        &[("self.", "s.")],
                    );
                    format!(
                        "s._context_stack[len(s._context_stack)-1]._return = {}",
                        go_expr
                    )
                }
                TargetLanguage::Php => format!(
                    "$this->_context_stack[count($this->_context_stack) - 1]->_return = {};",
                    expanded_expr
                ),
                TargetLanguage::Ruby => format!(
                    "@_context_stack[@_context_stack.length - 1]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Lua => format!(
                    "self._context_stack[#self._context_stack]._return = {}",
                    expanded_expr
                ),
                TargetLanguage::Erlang => {
                    // Leave `self.` intact so the body processor binds
                    // it to the live data_gen — see the sibling fix in
                    // `FrameSegmentKind::ContextReturn` above.
                    format!("__ReturnVal = {}", expanded_expr)
                }
                TargetLanguage::Graphviz => unreachable!(),
            };

            // Append native return on a new line with proper indent
            let ret_code = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript | TargetLanguage::Ruby => {
                    format!("\n{}return", indent_str)
                }
                TargetLanguage::Lua => format!("\n{}return", indent_str),
                TargetLanguage::Erlang => String::new(),
                _ => format!("\n{}return;", indent_str),
            };

            format!("{}{}", set_code, ret_code)
        }
        FrameSegmentKind::ContextSystemBare => {
            // Bare @@:system — should have been caught by validator (E604)
            "/* ERROR: bare @@:system */".to_string()
        }
        FrameSegmentKind::ContextSystemState => expand_system_state(lang),
        FrameSegmentKind::ContextSelf => {
            // @@:self — bare system instance reference
            match lang {
                TargetLanguage::Python3
                | TargetLanguage::GDScript
                | TargetLanguage::Ruby
                | TargetLanguage::Lua
                | TargetLanguage::Swift => "self".to_string(),
                TargetLanguage::TypeScript
                | TargetLanguage::JavaScript
                | TargetLanguage::Java
                | TargetLanguage::Kotlin
                | TargetLanguage::CSharp
                | TargetLanguage::Dart => "this".to_string(),
                TargetLanguage::Cpp => "this".to_string(),
                TargetLanguage::C => "self".to_string(),
                TargetLanguage::Go => "s".to_string(),
                TargetLanguage::Php => "$this".to_string(),
                TargetLanguage::Rust => super::rust_system::rust_self_ref().to_string(),
                TargetLanguage::Erlang => "self".to_string(),
                TargetLanguage::Graphviz => unreachable!(),
            }
        }
        FrameSegmentKind::ContextSelfCall => {
            // @@:self.method(args) — reentrant interface call with transition guard
            // Extract method name and args from segment text: @@:self.method(args)
            let trimmed = segment_text.trim();
            let (method_name, raw_args_with_parens) =
                if let SegmentMetadata::SelfCall { method, args } = metadata {
                    (method.as_str(), args.as_str())
                } else {
                    let after_self = trimmed.strip_prefix("@@:self.").unwrap_or(trimmed);
                    let paren_pos = after_self.find('(').unwrap_or(after_self.len());
                    (&after_self[..paren_pos], &after_self[paren_pos..])
                };
            // Recursively expand Frame syntax nested inside the args —
            // e.g. `@@:self.foo(@@:return)`, `@@:self.foo(@@:params.x)`,
            // `@@:self.foo(self.op())`, etc. Without this the inner
            // segment would leak verbatim into target source and fail
            // to parse (e.g. literal `@@:return` in Python output).
            let expanded_args = if raw_args_with_parens.len() >= 2
                && raw_args_with_parens.starts_with('(')
                && raw_args_with_parens.ends_with(')')
            {
                let inner = strip_outer_parens(raw_args_with_parens);
                if inner.is_empty() {
                    raw_args_with_parens.to_string()
                } else {
                    format!("({})", expand_expression(inner, lang, ctx))
                }
            } else {
                raw_args_with_parens.to_string()
            };
            let args_with_parens = expanded_args.as_str();

            // Generate the native self-call
            let call_expr = match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
                    format!("this.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Rust => {
                    // Rust's borrow checker rejects `self.foo(self.bar(x))`
                    // because both calls take `&mut self` at the same time.
                    // When the already-expanded args contain another
                    // `self.<method>(` pattern, hoist the inner call into
                    // a let-binding inside a block expression:
                    //   { let __rs_tmpN = self.bar(x); self.foo(__rs_tmpN) }
                    // Sequential `let` bindings in a block are two
                    // separate borrows — not simultaneous — so the
                    // checker accepts.
                    if args_with_parens.contains("self.") {
                        let inner = strip_outer_parens(args_with_parens);
                        format!(
                            "{{ let __rs_tmp_arg = {}; self.{}(__rs_tmp_arg) }}",
                            inner, method_name
                        )
                    } else {
                        format!("self.{}{}", method_name, args_with_parens)
                    }
                }
                TargetLanguage::Swift => {
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Cpp => format!("this->{}{}", method_name, args_with_parens),
                TargetLanguage::C => {
                    if args_with_parens == "()" {
                        format!("{}_{}(self)", ctx.system_name, method_name)
                    } else {
                        let inner_args = strip_outer_parens(args_with_parens);
                        format!("{}_{}(self, {})", ctx.system_name, method_name, inner_args)
                    }
                }
                TargetLanguage::Java | TargetLanguage::Kotlin | TargetLanguage::CSharp => {
                    format!("this.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Go => {
                    let go_method =
                        format!("{}{}", method_name[..1].to_uppercase(), &method_name[1..]);
                    format!("s.{}{}", go_method, args_with_parens)
                }
                TargetLanguage::Php => format!("$this->{}{}", method_name, args_with_parens),
                TargetLanguage::Ruby => format!("self.{}{}", method_name, args_with_parens),
                TargetLanguage::Lua => format!("self:{}{}", method_name, args_with_parens),
                TargetLanguage::Erlang => {
                    // Emit bare `self.method(args)` and let the Erlang
                    // handler post-pass (erlang_system.rs::
                    // erlang_rewrite_native_classified_full) recognize the
                    // pattern as an `InterfaceCall` and rewrite it to
                    // `{DataN, Result} = frame_dispatch__(method, [args],
                    // DataPrev)`. That pass threads NewData forward
                    // through the rest of the handler body via
                    // `data_gen`/`data_var` — so `self.field` reads and
                    // `-> $State` transitions after a @@:self call
                    // correctly see the state changes the called
                    // handler made.
                    format!("self.{}{}", method_name, args_with_parens)
                }
                TargetLanguage::Graphviz => unreachable!(),
            };

            // @@:self.method() — check if standalone (only whitespace before @@:
            // in the source) or inline (preceded by native code like `x = `).
            // The scanner trims trailing whitespace from native text for
            // standalone constructs, so we must provide indent_str. For
            // inline, native text provides the indent.
            //
            // We detect this from the segment_text position: if the segment
            // starts at a position where the preceding byte is whitespace or
            // newline, it's standalone. The scanner always sets indent > 0
            // for self-calls (line's leading whitespace for the guard), so
            // we can't use indent == 0 as the inline signal.
            //
            // Instead, check if the raw output ends with whitespace (inline
            // context: native text like "baseline = " precedes us) or with
            // a newline (standalone: previous line ended, we start fresh).
            //
            // Actually, the simplest correct approach: the expansion is
            // always just the call expression. The orchestrator adds the
            // guard. For standalone, the scanner trimmed the whitespace so
            // indent_str fills the gap. For inline, the scanner kept the
            // native text. In BOTH cases, indent_str is correct:
            //   standalone: trimmed ws (16 sp) + indent_str (16 sp) call = 16 sp call ✓
            //   inline: native "baseline = " + indent_str (16 sp) call = broken!
            //
            // So we DO need to distinguish. Use the preceding native text:
            // if it was trimmed (standalone), the segment immediately follows
            // a newline in the output. If not trimmed (inline), it follows
            // non-newline content. But we don't have access to `out` here.
            //
            // Cleanest: just return call_expr. The standalone case needs
            // indent_str, which the orchestrator can add based on indent > 0
            // and whether the expansion doesn't already start with whitespace.
            call_expr
        }
        FrameSegmentKind::ReturnStatement => {
            // Native return keyword detected in handler body.
            // Extract expression after "return" (if any).
            let after_return = segment_text
                .trim()
                .strip_prefix("return")
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .trim();

            if after_return.is_empty() {
                // Bare `return` — valid, exits the handler. Pass through as native.
                format!("{}return", indent_str)
            } else if after_return.starts_with("@@:") || after_return.starts_with("@@(") {
                // E408: `return @@:<anything>` — combining native return with Frame context
                eprintln!(
                    "E408: Cannot combine `return` with Frame context syntax `{}`. \
                    Use `@@:(expr)` to set the return value, then `return` on a separate line.",
                    after_return
                );
                String::new()
            } else {
                // W415: `return <expr>` in event handler — value is silently lost
                eprintln!(
                    "W415: `return {}` in event handler '{}' — the return value is lost. \
                    Use `@@:({})` to set the return value, or bare `return` to exit.",
                    after_return, ctx.event_name, after_return
                );
                // Pass through as native — it compiles but doesn't do what the user expects
                format!("{}{}", indent_str, segment_text.trim())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::native_region_scanner::FrameSegmentKind;
    use crate::frame_c::visitors::TargetLanguage;

    fn make_ctx(state_var_types: Vec<(&str, &str)>) -> HandlerContext {
        HandlerContext {
            system_name: "TestSys".to_string(),
            state_name: "S1".to_string(),
            event_name: "foo".to_string(),
            parent_state: None,
            defined_systems: std::collections::HashSet::new(),
            use_sv_comp: false,
            per_handler: false,
            state_var_types: state_var_types
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            state_param_names: std::collections::HashMap::new(),
            state_enter_param_names: std::collections::HashMap::new(),
            state_exit_param_names: std::collections::HashMap::new(),
            event_param_names: std::collections::HashMap::new(),
            state_hsm_parents: std::collections::HashMap::new(),
            current_return_type: None,
            state_param_types: std::collections::HashMap::new(),
        }
    }

    /// Helper: call generate_frame_expansion with text as bytes + span
    fn expand(
        kind: FrameSegmentKind,
        text: &str,
        lang: TargetLanguage,
        ctx: &HandlerContext,
    ) -> String {
        let bytes = text.as_bytes();
        let span = crate::frame_c::compiler::native_region_scanner::RegionSpan {
            start: 0,
            end: bytes.len(),
        };
        generate_frame_expansion(bytes, &span, kind, 0, lang, ctx, &SegmentMetadata::None)
    }

    // =========================================================
    // Rust @@:(expr) — string literals wrapped with String::from
    // =========================================================

    #[test]
    fn test_context_return_expr_rust_string_wraps() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(\"green\")",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            result.contains("String::from(\"green\")"),
            "Rust @@:(\"green\") should wrap with String::from, got: {}",
            result
        );
    }

    #[test]
    fn test_context_return_expr_rust_int_no_wrap() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(42)",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            result.contains("Box::new(42)"),
            "Rust @@:(42) should NOT wrap with String::from, got: {}",
            result
        );
        assert!(
            !result.contains("String::from"),
            "Integer should not get String::from wrapping, got: {}",
            result
        );
    }

    #[test]
    fn test_context_return_expr_python_no_wrap() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(\"green\")",
            TargetLanguage::Python3,
            &ctx,
        );
        assert!(
            !result.contains("String::from"),
            "Python should NOT wrap string literals, got: {}",
            result
        );
        assert!(
            result.contains("\"green\""),
            "Python should pass through the literal, got: {}",
            result
        );
    }

    // =========================================================
    // Rust @@:return = expr — same wrapping
    // =========================================================

    #[test]
    fn test_context_return_assign_rust_string_wraps() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturn,
            "@@:return = \"hello\"",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            result.contains("String::from(\"hello\")"),
            "Rust @@:return = \"hello\" should wrap, got: {}",
            result
        );
    }

    #[test]
    fn test_context_return_assign_rust_int_no_wrap() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturn,
            "@@:return = 42",
            TargetLanguage::Rust,
            &ctx,
        );
        assert!(
            !result.contains("String::from"),
            "Rust @@:return = 42 should NOT wrap, got: {}",
            result
        );
    }

    // =========================================================
    // paren_wrap_if_multiline — multi-line @@:(expr) assignments
    // need re-wrapping in `(...)` so indent-sensitive targets
    // (Python, GDScript) parse the continuation lines as part of
    // the expression. Single-line expressions stay unwrapped.
    // =========================================================

    #[test]
    fn paren_wrap_if_multiline_singleline_unchanged() {
        assert_eq!(paren_wrap_if_multiline("self.x"), "self.x");
        assert_eq!(paren_wrap_if_multiline("a + b"), "a + b");
        assert_eq!(paren_wrap_if_multiline(""), "");
    }

    #[test]
    fn paren_wrap_if_multiline_wraps_multiline() {
        let inp = "self.timer >= self.threshold\n    and self.count < self.limit";
        let want = "(self.timer >= self.threshold\n    and self.count < self.limit)";
        assert_eq!(paren_wrap_if_multiline(inp), want);
    }

    #[test]
    fn context_return_expr_gdscript_multiline_wraps() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(self.timer >= self.threshold\n    and self.count < self.limit)",
            TargetLanguage::GDScript,
            &ctx,
        );
        assert!(
            result.contains(
                "_return = (self.timer >= self.threshold\n    and self.count < self.limit)"
            ),
            "GDScript multi-line @@:() should re-wrap in parens, got:\n{}",
            result
        );
    }

    #[test]
    fn context_return_expr_python_multiline_wraps() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(self.timer >= self.threshold\n    and self.count < self.limit)",
            TargetLanguage::Python3,
            &ctx,
        );
        assert!(
            result.contains(
                "_return = (self.timer >= self.threshold\n    and self.count < self.limit)"
            ),
            "Python multi-line @@:() should re-wrap in parens, got:\n{}",
            result
        );
    }

    #[test]
    fn context_return_expr_singleline_no_wrap() {
        let ctx = make_ctx(vec![]);
        let result = expand(
            FrameSegmentKind::ContextReturnExpr,
            "@@:(true)",
            TargetLanguage::GDScript,
            &ctx,
        );
        assert!(
            result.contains("_return = true"),
            "single-line @@:() should NOT add parens, got:\n{}",
            result
        );
        assert!(
            !result.contains("_return = (true)"),
            "single-line @@:() must not gain parens, got:\n{}",
            result
        );
    }

    // =========================================================
    // Rust state var READ — .clone() for non-Copy types only
    // =========================================================

    #[test]
    fn test_state_var_read_rust_string_clones() {
        let ctx = make_ctx(vec![("name", "String")]);
        let result = expand_expression("$.name", TargetLanguage::Rust, &ctx);
        assert!(
            result.contains(".clone()"),
            "String state var read should add .clone(), got: {}",
            result
        );
    }

    #[test]
    fn test_state_var_read_rust_int_no_clone() {
        let ctx = make_ctx(vec![("count", "i32")]);
        let result = expand_expression("$.count", TargetLanguage::Rust, &ctx);
        assert!(
            !result.contains(".clone()"),
            "i32 state var read should NOT add .clone(), got: {}",
            result
        );
    }

    #[test]
    fn test_state_var_read_rust_bool_no_clone() {
        let ctx = make_ctx(vec![("flag", "bool")]);
        let result = expand_expression("$.flag", TargetLanguage::Rust, &ctx);
        assert!(
            !result.contains(".clone()"),
            "bool state var read should NOT add .clone(), got: {}",
            result
        );
    }

    #[test]
    fn test_state_var_read_rust_unknown_type_clones() {
        let ctx = make_ctx(vec![]);
        let result = expand_expression("$.mystery", TargetLanguage::Rust, &ctx);
        assert!(
            result.contains(".clone()"),
            "Unknown-type state var should clone for safety, got: {}",
            result
        );
    }
}
