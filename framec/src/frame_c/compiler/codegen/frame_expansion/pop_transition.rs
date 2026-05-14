//! Pop-transition lowering — RFC-0008 `pop$` with optional
//! `(exit_args)`, `(enter_args)`, and `=>` forward decorations.
//!
//! Frame's `pop$` (and its decorated forms `-> (exit_args) pop$`,
//! `-> () (enter_args) pop$`, and `-> => pop$`) pops the modal
//! stack and transitions to the popped state. The codegen emits
//! five sequential phases per backend:
//!
//! 1. `exit_args` writes on the current compartment (positional append).
//! 2. Pop from the state stack into a local `__saved` reference.
//! 3. Clear-and-write `enter_args` on `__saved` (RFC-0008 replace
//!    semantics, positional append). The args expressions go through
//!    `expand_expression` so Frame sigils like `\$.x` /
//!    `@@:params.name` resolve to the language-specific accessor
//!    before they're spliced.
//! 4. Set `forward_event = __e` on `__saved` if this is a `-> =>`
//!    forward transition.
//! 5. Call `__transition(__saved)` (or the backend's variant) and
//!    return from the handler.
//!
//! Erlang uses a different shape — there's no compartment object;
//! the popped state context is unpacked from the `Data#data.frame_stack`
//! head and passed as a `{next_state, ...}` reply tuple. The Erlang
//! branch short-circuits the five-phase emission after step 2.

use super::super::codegen_utils::HandlerContext;
use super::expand_expression;
use crate::frame_c::visitors::TargetLanguage;

/// Generate a pop-transition with optional RFC-0008 decorations
/// (exit_args, enter_args, is_forward). Each backend emits:
///   1. exit_args writes on current compartment (if present)
///   2. Pop from stack into __saved
///   3. Clear + write enter_args on __saved (if present)
///   4. Set forward_event on __saved (if is_forward)
///   5. __transition(__saved) + return
pub(super) fn generate_pop_transition(
    indent: &str,
    ctx: &HandlerContext,
    lang: TargetLanguage,
    exit_args: &Option<String>,
    enter_args: &Option<String>,
    is_forward: bool,
) -> String {
    let mut code = String::new();

    // Helper: emit exit_args writes on current compartment (positional append)
    if let Some(ref exit) = exit_args {
        for arg in exit.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
            let value = if let Some(eq_pos) = arg.find('=') {
                arg[eq_pos + 1..].trim()
            } else {
                arg
            };
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    code.push_str(&format!(
                        "{}self.__compartment.exit_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    code.push_str(&format!(
                        "{}this.__compartment.exit_args.push({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Dart => {
                    code.push_str(&format!(
                        "{}this.__compartment.exit_args.add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Rust => {
                    code.push_str(&super::super::rust_system::rust_pop_exit_arg(indent, value));
                }
                TargetLanguage::C => {
                    code.push_str(&format!("{}{}_FrameVec_push(self->__compartment->exit_args, (void*)(intptr_t)({}));\n", indent, ctx.system_name, value));
                }
                TargetLanguage::Cpp => {
                    code.push_str(&format!(
                        "{}__compartment->exit_args.push_back(std::any({}));\n",
                        indent, value
                    ));
                }
                TargetLanguage::Java => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Kotlin => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.add({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Swift => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::CSharp => {
                    code.push_str(&format!(
                        "{}__compartment.exit_args.Add({});\n",
                        indent, value
                    ));
                }
                TargetLanguage::Go => {
                    code.push_str(&format!(
                        "{}s.__compartment.exitArgs = append(s.__compartment.exitArgs, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Php => {
                    code.push_str(&format!(
                        "{}$this->__compartment->exit_args[] = {};\n",
                        indent, value
                    ));
                }
                TargetLanguage::Ruby => {
                    code.push_str(&format!(
                        "{}@__compartment.exit_args.append({})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Lua => {
                    code.push_str(&format!(
                        "{}table.insert(self.__compartment.exit_args, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
            }
        }
    }

    // Pop from stack
    match lang {
        TargetLanguage::Python3 => code.push_str(&format!("{}__saved = self._state_stack.pop()\n", indent)),
        TargetLanguage::GDScript => code.push_str(&format!("{}var __saved = self._state_stack.pop_back()\n", indent)),
        TargetLanguage::TypeScript => code.push_str(&format!("{}const __saved = this._state_stack.pop()!;\n", indent)),
        TargetLanguage::Dart => code.push_str(&format!("{}final __saved = this._state_stack.removeLast();\n", indent)),
        TargetLanguage::JavaScript => code.push_str(&format!("{}const __saved = this._state_stack.pop();\n", indent)),
        TargetLanguage::Rust => code.push_str(&super::super::rust_system::rust_pop_stack(indent)),
        TargetLanguage::C => code.push_str(&format!("{}{}_Compartment* __saved = ({}_Compartment*){}_FrameVec_pop(self->_state_stack);\n", indent, ctx.system_name, ctx.system_name, ctx.system_name)),
        TargetLanguage::Cpp => code.push_str(&format!("{}auto __saved = std::move(_state_stack.back()); _state_stack.pop_back();\n", indent)),
        TargetLanguage::Java => code.push_str(&format!("{}var __saved = _state_stack.remove(_state_stack.size() - 1);\n", indent)),
        TargetLanguage::Kotlin => code.push_str(&format!("{}val __saved = _state_stack.removeAt(_state_stack.size - 1)\n", indent)),
        TargetLanguage::Swift => code.push_str(&format!("{}let __saved = _state_stack.removeLast()\n", indent)),
        TargetLanguage::CSharp => code.push_str(&format!("{}var __saved = _state_stack[_state_stack.Count - 1]; _state_stack.RemoveAt(_state_stack.Count - 1);\n", indent)),
        TargetLanguage::Go => {
            code.push_str(&format!("{}__saved := s._state_stack[len(s._state_stack)-1]\n", indent));
            code.push_str(&format!("{}s._state_stack = s._state_stack[:len(s._state_stack)-1]\n", indent));
        }
        TargetLanguage::Php => code.push_str(&format!("{}$__saved = array_pop($this->_state_stack);\n", indent)),
        TargetLanguage::Ruby => code.push_str(&format!("{}__saved = @_state_stack.pop\n", indent)),
        TargetLanguage::Lua => code.push_str(&format!("{}local __saved = table.remove(self._state_stack)\n", indent)),
        TargetLanguage::Erlang => {
            // Pop the saved compartment context: a 3-tuple of state
            // atom + frame_state_args + frame_enter_args (push side
            // emits the same shape). Restoring all three fields on
            // the popped Data record fixes a defect surfaced by
            // Phase 19 wave 3 P7 — without args restoration, popping
            // back to a state with `(x: int)` left state_args at the
            // PUSHED state's value (or undefined), so subsequent
            // reads of `$.x` returned the wrong context.
            code.push_str(&format!("{}[{{__PoppedState, __PoppedStateArgs, __PoppedEnterArgs}} | __RestStack] = Data#data.frame_stack,\n", indent));
            code.push_str(&format!(
                "{}{{next_state, __PoppedState, Data#data{{frame_stack = __RestStack, frame_state_args = __PoppedStateArgs, frame_enter_args = __PoppedEnterArgs}}, [{{reply, From, ok}}]}}",
                indent
            ));
            return code;
        }
        TargetLanguage::Graphviz => unreachable!(),
    }

    // Fresh enter_args: clear + write (RFC-0008 replace semantics, positional append).
    // The arg expression arrives straight from the Frame source — `$.items`,
    // `self.field`, `@@:params.name` and friends are Frame sigils that the
    // standard expression expander resolves to language-specific accessors
    // (e.g. `$.items` → `__sv_comp.state_vars["items"]` in Python). Without
    // this expansion pop-args like `-> ($.items) pop$` would emit the raw
    // sigil into native code and blow up at parse time.
    if let Some(ref enter) = enter_args {
        for arg in enter.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
            let raw_value = if let Some(eq_pos) = arg.find('=') {
                arg[eq_pos + 1..].trim()
            } else {
                arg
            };
            let value_owned = expand_expression(raw_value, lang, ctx);
            let value = value_owned.as_str();
            match lang {
                TargetLanguage::Python3 | TargetLanguage::GDScript => {
                    code.push_str(&format!("{}__saved.enter_args.append({})\n", indent, value));
                }
                TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                    code.push_str(&format!("{}__saved.enter_args.push({});\n", indent, value));
                }
                TargetLanguage::Dart => {
                    code.push_str(&format!("{}__saved.enter_args.add({});\n", indent, value));
                }
                TargetLanguage::Rust => {
                    code.push_str(&super::super::rust_system::rust_pop_enter_arg(indent, value));
                }
                TargetLanguage::C => {
                    code.push_str(&format!(
                        "{}{}_FrameVec_push(__saved->enter_args, (void*)(intptr_t)({}));\n",
                        indent, ctx.system_name, value
                    ));
                }
                TargetLanguage::Cpp => {
                    code.push_str(&format!(
                        "{}__saved->enter_args.push_back(std::any({}));\n",
                        indent, value
                    ));
                }
                TargetLanguage::Java => {
                    code.push_str(&format!("{}__saved.enter_args.add({});\n", indent, value));
                }
                TargetLanguage::Kotlin => {
                    code.push_str(&format!("{}__saved.enter_args.add({})\n", indent, value));
                }
                TargetLanguage::Swift => {
                    code.push_str(&format!("{}__saved.enter_args.append({})\n", indent, value));
                }
                TargetLanguage::CSharp => {
                    code.push_str(&format!("{}__saved.enter_args.Add({});\n", indent, value));
                }
                TargetLanguage::Go => {
                    code.push_str(&format!(
                        "{}__saved.enterArgs = append(__saved.enterArgs, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Php => {
                    code.push_str(&format!("{}$__saved->enter_args[] = {};\n", indent, value));
                }
                TargetLanguage::Ruby => {
                    code.push_str(&format!("{}__saved.enter_args.append({})\n", indent, value));
                }
                TargetLanguage::Lua => {
                    code.push_str(&format!(
                        "{}table.insert(__saved.enter_args, {})\n",
                        indent, value
                    ));
                }
                TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
            }
        }
    }

    // Forward event (RFC-0008: -> => pop$)
    if is_forward {
        match lang {
            TargetLanguage::Python3 | TargetLanguage::GDScript => {
                code.push_str(&format!("{}__saved.forward_event = __e\n", indent));
            }
            TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
                code.push_str(&format!("{}__saved.forward_event = __e;\n", indent));
            }
            TargetLanguage::Rust => {
                code.push_str(&super::super::rust_system::rust_pop_forward(indent));
            }
            TargetLanguage::C => {
                code.push_str(&format!("{}__saved->forward_event = __e;\n", indent));
            }
            TargetLanguage::Cpp => {
                code.push_str(&format!(
                    "{}__saved->forward_event = std::make_unique<{}FrameEvent>(__e);\n",
                    indent, ctx.system_name
                ));
            }
            TargetLanguage::Java
            | TargetLanguage::Kotlin
            | TargetLanguage::Swift
            | TargetLanguage::CSharp => {
                code.push_str(&format!("{}__saved.forward_event = __e;\n", indent));
            }
            TargetLanguage::Go => {
                code.push_str(&format!("{}__saved.forwardEvent = __e\n", indent));
            }
            TargetLanguage::Php => {
                code.push_str(&format!("{}$__saved->forward_event = $__e;\n", indent));
            }
            TargetLanguage::Ruby => {
                code.push_str(&format!("{}__saved.forward_event = __e\n", indent));
            }
            TargetLanguage::Lua => {
                code.push_str(&format!("{}__saved.forward_event = __e\n", indent));
            }
            TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
        }
    }

    // Transition + return
    let var = if matches!(lang, TargetLanguage::Rust) {
        super::super::rust_system::rust_pop_var_name()
    } else {
        "__saved"
    };
    match lang {
        TargetLanguage::Python3 | TargetLanguage::GDScript => {
            code.push_str(&format!(
                "{}self.__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::TypeScript | TargetLanguage::JavaScript | TargetLanguage::Dart => {
            code.push_str(&format!(
                "{}this.__transition({});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Rust => {
            code.push_str(&super::super::rust_system::rust_pop_transition(indent));
        }
        TargetLanguage::C => {
            code.push_str(&format!(
                "{}{}_transition(self, {});\n{}return;",
                indent, ctx.system_name, var, indent
            ));
        }
        TargetLanguage::Cpp => {
            code.push_str(&format!(
                "{}__transition(std::move({}));\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Java => {
            code.push_str(&format!(
                "{}__transition({});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Kotlin => {
            code.push_str(&format!(
                "{}__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Swift => {
            code.push_str(&format!(
                "{}__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::CSharp => {
            code.push_str(&format!(
                "{}__transition({});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Go => {
            code.push_str(&format!(
                "{}s.__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Php => {
            code.push_str(&format!(
                "{}$this->__transition(${});\n{}return;",
                indent, var, indent
            ));
        }
        TargetLanguage::Ruby => {
            code.push_str(&format!(
                "{}__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Lua => {
            code.push_str(&format!(
                "{}self:__transition({})\n{}return",
                indent, var, indent
            ));
        }
        TargetLanguage::Erlang | TargetLanguage::Graphviz => {}
    }

    code
}
