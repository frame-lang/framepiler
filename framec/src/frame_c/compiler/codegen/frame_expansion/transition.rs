//! `-> $State` transition expansion across the 17 backends.
//!
//! This is the largest single Frame segment kind by code volume.
//! Each backend needs its own emission shape because the
//! transition mechanic — write enter_args / state_args, set the
//! new compartment, call `__transition`, return — uses different
//! container types, ownership flavors, and dispatch idioms per
//! target. The whole arm comprises:
//!
//! - **Pop short-circuit** (`-> pop$`) — delegates to
//!   `pop_transition::generate_pop_transition` (RFC-0008
//!   decorations included).
//! - **Forward short-circuit** (`-> => $State`) — sets the new
//!   compartment's `forward_event` field so the destination
//!   re-dispatches the in-flight event after enter completes.
//! - **Regular transition** — the bulk: per-target compartment
//!   construction, HSM ancestor chain walking
//!   (`__prepareEnter`-style for the dynamic backends; eager
//!   `parent_compartment` field threading for the static
//!   backends), state_args / enter_args positional writes, and
//!   the `__transition()` call.
//!
//! All language-specific emission stays here — no per-target
//! crate-level helpers escape from this arm beyond what
//! rust_system already publishes (the Rust transition has a
//! large dedicated module emitter; this arm just calls it).

use super::super::codegen_utils::{cpp_wrap_any_arg, to_snake_case, HandlerContext};
use super::expand_expression;
use super::generate_pop_transition;
use super::utility::php_prefix_params;
use crate::frame_c::compiler::native_region_scanner::{RegionSpan, SegmentMetadata};
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn expand_transition(
    body_bytes: &[u8],
    span: &RegionSpan,
    indent: usize,
    lang: TargetLanguage,
    ctx: &HandlerContext,
    metadata: &SegmentMetadata,
) -> String {
    let segment_text = String::from_utf8_lossy(&body_bytes[span.start..span.end]);
    let indent_str = " ".repeat(indent);

    // Parse transition: (exit_args)? -> (enter_args)? $State(state_args)?
    // For Python/TypeScript: Create compartment and call __transition()
    // For Rust: Use simpler _transition() approach

    // Check for forward-transition: -> => $State
    let is_forward = if let SegmentMetadata::Transition { is_forward, .. } = metadata {
        *is_forward
    } else {
        false
    };

    // Check for pop-transition: -> pop$
    let is_pop = if let SegmentMetadata::Transition { is_pop, .. } = metadata {
        *is_pop
    } else {
        segment_text.contains("pop$")
    };
    if is_pop {
        // Pop-transition with optional decorations (RFC-0008):
        // 1. Write exit_args to current compartment (if present)
        // 2. Pop from stack
        // 3. If enter_args present: clear + write fresh values
        // 4. If is_forward: set forward_event
        // 5. __transition + return
        let (exit_str, enter_str) = match metadata {
            SegmentMetadata::Transition {
                exit_args,
                enter_args,
                ..
            } => (exit_args.clone(), enter_args.clone()),
            _ => (None, None),
        };
        generate_pop_transition(&indent_str, ctx, lang, &exit_str, &enter_str, is_forward)
    } else if is_forward {
        // Forward-transition: -> => $State
        // Create compartment, set forward_event to current event,
        // call __transition, return.
        let target = match metadata {
            SegmentMetadata::Transition { target_state, .. } => target_state.clone(),
            _ => "Unknown".to_string(),
        };
        // Build the target state's HSM ancestry outer-in. Used
        // below by the per-handler targets (Python/TS/JS/GDScript/
        // Ruby/Lua) to construct the parent_compartment chain
        // eagerly, never duplicating the transition-source
        // compartment (see
        // _scratch/bug_parent_compartment_hsm_walk.md).
        let mut ancestors: Vec<String> = Vec::new();
        let mut cursor = target.clone();
        while let Some(parent) = ctx.state_hsm_parents.get(&cursor) {
            ancestors.push(parent.clone());
            cursor = parent.clone();
        }
        ancestors.reverse();

        match lang {
            TargetLanguage::Python3 => {
                // Forward transition: same chain construction as
                // a regular transition (via __prepareEnter), plus
                // forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}__compartment = self.__prepareEnter(\"{}\", [], [])\n",
                    indent_str, target
                ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e\n",
                    indent_str
                ));
                code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                code.push_str(&format!("{}return", indent_str));
                code
            }
            TargetLanguage::GDScript => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}var __compartment = self.__prepareEnter(\"{}\", [], [])\n",
                    indent_str, target
                ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e\n",
                    indent_str
                ));
                code.push_str(&format!("{}self.__transition(__compartment)\n", indent_str));
                code.push_str(&format!("{}return", indent_str));
                code
            }
            TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}const __compartment = this.__prepareEnter(\"{}\", [], []);\n",
                    indent_str, target
                ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e;\n",
                    indent_str
                ));
                code.push_str(&format!(
                    "{}this.__transition(__compartment);\n",
                    indent_str
                ));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::Dart => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}final __compartment = this.__prepareEnter(\"{}\", [], []);\n",
                    indent_str, target
                ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e;\n",
                    indent_str
                ));
                code.push_str(&format!(
                    "{}this.__transition(__compartment);\n",
                    indent_str
                ));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::Rust => {
                super::super::rust_system::rust_expand_forward_transition(&indent_str, ctx, &target)
            }
            TargetLanguage::C => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                            "{}{}_Compartment* __compartment = {}_prepareEnter(self, \"{}\", NULL, NULL);\n",
                            indent_str, ctx.system_name, ctx.system_name, target
                        ));
                code.push_str(&format!(
                    "{}__compartment->forward_event = __e;\n",
                    indent_str
                ));
                code.push_str(&format!(
                    "{}{}_transition(self, __compartment);\n",
                    indent_str, ctx.system_name
                ));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::Cpp => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                            "{}auto __compartment = __prepareEnter(\"{}\", std::vector<std::any>{{}}, std::vector<std::any>{{}});\n",
                            indent_str, target
                        ));
                code.push_str(&format!(
                    "{}__compartment->forward_event = std::make_unique<{}FrameEvent>(__e);\n",
                    indent_str, ctx.system_name
                ));
                code.push_str(&format!(
                    "{}__transition(std::move(__compartment));\n",
                    indent_str
                ));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::Java => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                            "{}{}Compartment __compartment = __prepareEnter(\"{}\", new ArrayList<>(), new ArrayList<>());\n",
                            indent_str, ctx.system_name, target
                        ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e;\n",
                    indent_str
                ));
                code.push_str(&format!("{}__transition(__compartment);\n", indent_str));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::Kotlin => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                            "{}val __compartment = __prepareEnter(\"{}\", mutableListOf<Any?>(), mutableListOf<Any?>())\n",
                            indent_str, target
                        ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e\n",
                    indent_str
                ));
                code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                code.push_str(&format!("{}return", indent_str));
                code
            }
            TargetLanguage::Swift => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}let __compartment = {}.__prepareEnter(\"{}\", [], [])\n",
                    indent_str, ctx.system_name, target
                ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e\n",
                    indent_str
                ));
                code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                code.push_str(&format!("{}return", indent_str));
                code
            }
            TargetLanguage::Php => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}$__compartment = $this->__prepareEnter(\"{}\", [], []);\n",
                    indent_str, target
                ));
                code.push_str(&format!(
                    "{}$__compartment->forward_event = $__e;\n",
                    indent_str
                ));
                code.push_str(&format!(
                    "{}$this->__transition($__compartment);\n",
                    indent_str
                ));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::CSharp => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf. Local
                // is `__next` (not `__compartment`) — see C# regular
                // transition for why. Wrapped in `{ ... }` block
                // for the same reason.
                let mut code = String::new();
                code.push_str(&format!(
                            "{}{{ {}Compartment __next = __prepareEnter(\"{}\", new List<object>(), new List<object>());\n",
                            indent_str, ctx.system_name, target
                        ));
                code.push_str(&format!("{}__next.forward_event = __e;\n", indent_str));
                code.push_str(&format!("{}__transition(__next); }}\n", indent_str));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::Go => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}__compartment := s.__prepareEnter(\"{}\", []any{{}}, []any{{}})\n",
                    indent_str, target
                ));
                code.push_str(&format!("{}__compartment.forwardEvent = __e\n", indent_str));
                code.push_str(&format!("{}s.__transition(__compartment)\n", indent_str));
                code.push_str(&format!("{}return", indent_str));
                code
            }
            TargetLanguage::Ruby => {
                // Forward transition: same chain via __prepareEnter,
                // plus forward_event field set on the leaf.
                let mut code = String::new();
                code.push_str(&format!(
                    "{}__compartment = __prepareEnter(\"{}\", [], [])\n",
                    indent_str, target
                ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e\n",
                    indent_str
                ));
                code.push_str(&format!("{}__transition(__compartment)\n", indent_str));
                code.push_str(&format!("{}return", indent_str));
                code
            }
            TargetLanguage::Lua => {
                // Forward transition. nil for empty args lists
                // (block-transformer workaround — see regular-
                // transition Lua case).
                let mut code = String::new();
                code.push_str(&format!(
                    "{}local __compartment = self:__prepareEnter(\"{}\", nil, nil)\n",
                    indent_str, target
                ));
                code.push_str(&format!(
                    "{}__compartment.forward_event = __e\n",
                    indent_str
                ));
                code.push_str(&format!("{}self:__transition(__compartment)\n", indent_str));
                code.push_str(&format!("{}return", indent_str));
                code
            }
            TargetLanguage::Erlang => {
                // Forward transition: cascade exit/enter (same
                // shape as `frame_transition__`) plus a
                // `next_event` action that re-dispatches the
                // originating event (`__Event`) to the new
                // leaf after gen_statem fires its `state_enter`
                // callback there. `__Event` is bound by the
                // handler clause's pattern (see
                // erlang_system.rs handler emission). Forward
                // transitions can't carry their own
                // exit/enter/state args at the Frame level
                // (the syntax is just `-> => $State`), so all
                // three arg maps are empty.
                let erlang_state = to_snake_case(&target);
                format!(
                    "{}frame_forward_transition__({}, __Event, Data, [], [], [], From)",
                    indent_str, erlang_state
                )
            }
            TargetLanguage::Graphviz => unreachable!(),
        }
    } else {
        // Normal transition: -> $State with exit/enter/state args
        // Transition metadata is always populated by the scanner.
        let (target, exit_args, enter_args, state_args) = match metadata {
            SegmentMetadata::Transition {
                target_state,
                exit_args,
                enter_args,
                state_args,
                ..
            } => (
                target_state.clone(),
                exit_args.clone(),
                enter_args.clone(),
                state_args.clone(),
            ),
            _ => unreachable!(
                "Transition kind segment without Transition metadata: {:?}",
                metadata
            ),
        };

        // Expand state variable references in arguments
        let exit_str = exit_args.map(|a| expand_expression(&a, lang, ctx));
        let enter_str = enter_args.map(|a| expand_expression(&a, lang, ctx));
        let state_str = state_args.map(|a| expand_expression(&a, lang, ctx));

        // Get compartment class name from system name
        let _compartment_class = format!("{}Compartment", ctx.system_name);

        match lang {
            TargetLanguage::Python3 => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                //   __prepareExit(exit_args) — populates
                //     exit_args on every layer of the source chain.
                //   __prepareEnter(leaf, state_args, enter_args) —
                //     constructs the destination chain via the
                //     static _HSM_CHAIN topology table; every
                //     layer gets independent copies of the args
                //     (uniform parameter propagation).
                //   __transition(comp) — caches destination for
                //     the kernel to process.
                let mut code = String::new();

                // Build state_args list literal.
                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                // Build enter_args list literal.
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                // Populate exit_args on the source chain (omitted
                // when there are no exit_args).
                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}self.__prepareExit([{}])\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                // Construct destination chain via the helper.
                code.push_str(&format!(
                    "{}__compartment = self.__prepareEnter(\"{}\", {}, {})\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                // Cache and return.
                code.push_str(&format!(
                    "{}self.__transition(__compartment)\n{}return",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::GDScript => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}self.__prepareExit([{}])\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}var __compartment = self.__prepareEnter(\"{}\", {}, {})\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}self.__transition(__compartment)\n{}return",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::TypeScript | TargetLanguage::JavaScript => {
                // Per-handler architecture with helpers (see
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareExit / __prepareEnter / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}this.__prepareExit([{}]);\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}const __compartment = this.__prepareEnter(\"{}\", {}, {});\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}this.__transition(__compartment);\n{}return;",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Dart => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}this.__prepareExit([{}]);\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}final __compartment = this.__prepareEnter(\"{}\", {}, {});\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}this.__transition(__compartment);\n{}return;",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Rust => super::super::rust_system::rust_expand_transition(
                &indent_str,
                ctx,
                &target,
                &exit_str,
                &state_str,
                &enter_str,
            ),
            TargetLanguage::C => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();
                let sys = &ctx.system_name;

                // exit_args via __prepareExit if any provided.
                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}{{ {}_FrameVec* __ea = {}_FrameVec_new();\n",
                            indent_str, sys, sys
                        ));
                        for v in &vals {
                            code.push_str(&format!(
                                "{}{}_FrameVec_push(__ea, (void*)(intptr_t)({}));\n",
                                indent_str, sys, v
                            ));
                        }
                        code.push_str(&format!("{}{}_prepareExit(self, __ea);\n", indent_str, sys));
                        code.push_str(&format!(
                            "{}{}_FrameVec_destroy(__ea); }}\n",
                            indent_str, sys
                        ));
                    }
                }

                // Build state_args / enter_args FrameVecs, call __prepareEnter.
                let state_vals: Vec<String> = if let Some(ref state) = state_str {
                    state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim().to_string()
                            } else {
                                arg.to_string()
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let enter_vals: Vec<String> = if let Some(ref enter) = enter_str {
                    enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|s| s.to_string())
                        .collect()
                } else {
                    Vec::new()
                };

                // Open block scope so locals don't collide with
                // sibling transitions in the same handler (e.g.
                // separate `if` branches).
                // Look up target's state-arg names so each value
                // can be packed using its declared type. Float /
                // double survive only via the memcpy bit-pun
                // helper `Sys_pack_double`; (intptr_t) truncates.
                let target_param_names: Vec<String> = ctx
                    .state_param_names
                    .get(&target)
                    .cloned()
                    .unwrap_or_default();
                let push_arg_for_target = |idx: usize, value_expr: &str| -> String {
                    let frame_type = target_param_names
                        .get(idx)
                        .and_then(|name| {
                            ctx.state_param_types
                                .get(&(target.clone(), name.clone()))
                                .cloned()
                        })
                        .unwrap_or_default();
                    match frame_type.trim() {
                        "float" | "double" | "f32" | "f64" => {
                            format!("{}_pack_double({})", sys, value_expr)
                        }
                        _ => format!("(void*)(intptr_t)({})", value_expr),
                    }
                };
                code.push_str(&format!("{}{{\n", indent_str));
                if state_vals.is_empty() {
                    code.push_str(&format!(
                        "{}    {}_FrameVec* __sa = NULL;\n",
                        indent_str, sys
                    ));
                } else {
                    code.push_str(&format!(
                        "{}    {}_FrameVec* __sa = {}_FrameVec_new();\n",
                        indent_str, sys, sys
                    ));
                    for (i, v) in state_vals.iter().enumerate() {
                        let push_arg = push_arg_for_target(i, v);
                        code.push_str(&format!(
                            "{}    {}_FrameVec_push(__sa, {});\n",
                            indent_str, sys, push_arg
                        ));
                    }
                }
                if enter_vals.is_empty() {
                    code.push_str(&format!(
                        "{}    {}_FrameVec* __ea = NULL;\n",
                        indent_str, sys
                    ));
                } else {
                    code.push_str(&format!(
                        "{}    {}_FrameVec* __ea = {}_FrameVec_new();\n",
                        indent_str, sys, sys
                    ));
                    for v in &enter_vals {
                        code.push_str(&format!(
                            "{}    {}_FrameVec_push(__ea, (void*)(intptr_t)({}));\n",
                            indent_str, sys, v
                        ));
                    }
                }
                code.push_str(&format!(
                            "{}    {}_Compartment* __compartment = {}_prepareEnter(self, \"{}\", __sa, __ea);\n",
                            indent_str, sys, sys, target
                        ));
                if !state_vals.is_empty() {
                    code.push_str(&format!(
                        "{}    {}_FrameVec_destroy(__sa);\n",
                        indent_str, sys
                    ));
                }
                if !enter_vals.is_empty() {
                    code.push_str(&format!(
                        "{}    {}_FrameVec_destroy(__ea);\n",
                        indent_str, sys
                    ));
                }
                code.push_str(&format!(
                    "{}    {}_transition(self, __compartment);\n",
                    indent_str, sys
                ));
                code.push_str(&format!("{}}}\n", indent_str));
                code.push_str(&format!("{}return;", indent_str));
                code
            }
            TargetLanguage::Cpp => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<String> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            let raw = if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            };
                            format!("std::any({})", cpp_wrap_any_arg(raw))
                        })
                        .collect();
                    format!("std::vector<std::any>{{{}}}", vals.join(", "))
                } else {
                    "std::vector<std::any>{}".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<String> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|a| format!("std::any({})", cpp_wrap_any_arg(a)))
                        .collect();
                    format!("std::vector<std::any>{{{}}}", vals.join(", "))
                } else {
                    "std::vector<std::any>{}".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<String> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|a| format!("std::any({})", cpp_wrap_any_arg(a)))
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}__prepareExit(std::vector<std::any>{{{}}});\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}auto __next = __prepareEnter(\"{}\", {}, {});\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}__transition(std::move(__next));\n{}return;",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Java => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    if vals.is_empty() {
                        "new java.util.ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new java.util.ArrayList<>(java.util.Arrays.asList({}))",
                            vals.join(", ")
                        )
                    }
                } else {
                    "new java.util.ArrayList<>()".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if vals.is_empty() {
                        "new java.util.ArrayList<>()".to_string()
                    } else {
                        format!(
                            "new java.util.ArrayList<>(java.util.Arrays.asList({}))",
                            vals.join(", ")
                        )
                    }
                } else {
                    "new java.util.ArrayList<>()".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                                    "{}__prepareExit(new java.util.ArrayList<>(java.util.Arrays.asList({})));\n",
                                    indent_str,
                                    vals.join(", ")
                                ));
                    }
                }

                code.push_str(&format!(
                    "{}{}Compartment __compartment = __prepareEnter(\"{}\", {}, {});\n",
                    indent_str, ctx.system_name, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}__transition(__compartment);\n{}return;",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Kotlin => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    if vals.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", vals.join(", "))
                    }
                } else {
                    "mutableListOf<Any?>()".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if vals.is_empty() {
                        "mutableListOf<Any?>()".to_string()
                    } else {
                        format!("mutableListOf<Any?>({})", vals.join(", "))
                    }
                } else {
                    "mutableListOf<Any?>()".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}__prepareExit(mutableListOf<Any?>({}))\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}val __compartment = __prepareEnter(\"{}\", {}, {})\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}__transition(__compartment)\n{}return",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Swift => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}__prepareExit([{}])\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}let __compartment = {}.__prepareEnter(\"{}\", {}, {})\n",
                    indent_str, ctx.system_name, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}__transition(__compartment)\n{}return",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::CSharp => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                // Note: local var is named `__next` (not
                // `__compartment`) to avoid shadowing the field
                // in stack-push handlers that reference the field
                // earlier in the same block — C# rejects that
                // even when the local is declared later.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    if vals.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!("new List<object> {{ {} }}", vals.join(", "))
                    }
                } else {
                    "new List<object>()".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if vals.is_empty() {
                        "new List<object>()".to_string()
                    } else {
                        format!("new List<object> {{ {} }}", vals.join(", "))
                    }
                } else {
                    "new List<object>()".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}__prepareExit(new List<object> {{ {} }});\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                // Wrap in `{ ... }` block scope so multiple
                // transitions in the same handler (e.g. inside
                // separate `if` branches) don't trigger C#
                // CS0136 (same name used in enclosing scope).
                code.push_str(&format!(
                    "{}{{ {}Compartment __next = __prepareEnter(\"{}\", {}, {});\n",
                    indent_str, ctx.system_name, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}__transition(__next); }}\n{}return;",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Go => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[]any{{{}}}", vals.join(", "))
                } else {
                    "[]any{}".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    format!("[]any{{{}}}", vals.join(", "))
                } else {
                    "[]any{}".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}s.__prepareExit([]any{{{}}})\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}__compartment := s.__prepareEnter(\"{}\", {}, {})\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}s.__transition(__compartment)\n{}return",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Php => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();
                let current_params = ctx
                    .event_param_names
                    .get(&ctx.event_name)
                    .cloned()
                    .unwrap_or_default();
                let php_fix = |expr: &str| php_prefix_params(expr, &current_params);

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<String> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            let raw = if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            };
                            php_fix(raw)
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<String> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            let raw = if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            };
                            php_fix(raw)
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<String> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|a| php_fix(a))
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}$this->__prepareExit([{}]);\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}$__compartment = $this->__prepareEnter(\"{}\", {}, {});\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}$this->__transition($__compartment);\n{}return;",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Ruby => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+):
                // __prepareEnter / __prepareExit / __transition.
                let mut code = String::new();

                let state_args_list = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };
                let enter_args_list = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    format!("[{}]", vals.join(", "))
                } else {
                    "[]".to_string()
                };

                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}__prepareExit([{}])\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}__compartment = __prepareEnter(\"{}\", {}, {})\n",
                    indent_str, target, state_args_list, enter_args_list
                ));

                code.push_str(&format!(
                    "{}__transition(__compartment)\n{}return",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Lua => {
                // Per-handler architecture with helpers (per
                // docs/frame_runtime_introduction.md Step 21+).
                // Uses table.pack(...) instead of `{}` literals
                // because the Lua block transformer mishandles
                // `{}` table literals inside if/else bodies
                // (sees them as nested block braces). nil is
                // accepted by __prepareEnter / __prepareExit
                // when there are no args.
                let mut code = String::new();

                // state_args
                let state_arg = if let Some(ref state) = state_str {
                    let vals: Vec<&str> = state
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    if vals.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", vals.join(", "))
                    }
                } else {
                    "nil".to_string()
                };

                // enter_args
                let enter_arg = if let Some(ref enter) = enter_str {
                    let vals: Vec<&str> = enter
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .map(|arg| {
                            if let Some(eq_pos) = arg.find('=') {
                                arg[eq_pos + 1..].trim()
                            } else {
                                arg
                            }
                        })
                        .collect();
                    if vals.is_empty() {
                        "nil".to_string()
                    } else {
                        format!("table.pack({})", vals.join(", "))
                    }
                } else {
                    "nil".to_string()
                };

                // exit_args (only emitted when present)
                if let Some(ref exit) = exit_str {
                    let vals: Vec<&str> = exit
                        .split(',')
                        .map(|x| x.trim())
                        .filter(|x| !x.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        code.push_str(&format!(
                            "{}self:__prepareExit(table.pack({}))\n",
                            indent_str,
                            vals.join(", ")
                        ));
                    }
                }

                code.push_str(&format!(
                    "{}local __compartment = self:__prepareEnter(\"{}\", {}, {})\n",
                    indent_str, target, state_arg, enter_arg
                ));

                code.push_str(&format!(
                    "{}self:__transition(__compartment)\n{}return",
                    indent_str, indent_str
                ));
                code
            }
            TargetLanguage::Erlang => {
                // Path D: use frame_transition__ for full lifecycle.
                //
                // Args are emitted as positional Erlang lists (per the
                // HashMap→Vec migration). For `name=value` entries
                // the name is dropped and only `value` is included —
                // the codegen relies on Frame source authors providing
                // args in declaration order, same convention as
                // Python/TS/Rust/etc.
                let erlang_state = to_snake_case(&target);
                let mut code = String::new();

                let to_list_lit = |s: &Option<String>| -> String {
                    match s {
                        Some(joined) => {
                            let vals: Vec<&str> = joined
                                .split(',')
                                .map(|x| x.trim())
                                .filter(|x| !x.is_empty())
                                .map(|arg| {
                                    if let Some(eq_pos) = arg.find('=') {
                                        arg[eq_pos + 1..].trim()
                                    } else {
                                        arg
                                    }
                                })
                                .collect();
                            format!("[{}]", vals.join(", "))
                        }
                        None => "[]".to_string(),
                    }
                };
                let exit_list = to_list_lit(&exit_str);
                let enter_list = to_list_lit(&enter_str);
                let state_list = to_list_lit(&state_str);

                // 7th arg: the @@:return value (the SSA-renamed
                // `__ReturnVal_K` from the body's most recent
                // `@@:return` write). Emit the literal token here;
                // the body processor's SSA pass rewrites it to
                // the correct SSA name when a write precedes the
                // transition. `erlang_finalize_transition_replies`
                // (run after the SSA pass) replaces any remaining
                // unresolved `__ReturnVal` token in this position
                // with `ok` — the gen_statem default reply value
                // for handlers that didn't set one.
                // Quote the state atom to immunize it from
                // erlang_capitalize_params: when a transition target
                // collides with a param name (state `$Active` and
                // param `active`) the capitalize pass would otherwise
                // turn the atom into the param's variable form.
                // `'active'` and `active` are equivalent Erlang atoms.
                code.push_str(&format!(
                    "{}frame_transition__('{}', Data, {}, {}, {}, From, __ReturnVal)",
                    indent_str, erlang_state, exit_list, enter_list, state_list
                ));
                code
            }
            TargetLanguage::Graphviz => unreachable!(),
        }
    }
}
