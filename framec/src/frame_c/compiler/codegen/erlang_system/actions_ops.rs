//! Action and operation function emission for Erlang gen_statem
//! modules.
//!
//! Frame's `actions:` and `operations:` blocks both produce
//! module-level Erlang functions, but with subtly different
//! shapes:
//!
//! - **Actions** receive `Data` as their first arg and return
//!   `{Data, RetVal}` (or `{Data, ok}` for void actions). They
//!   thread state through the body via the body processor's
//!   `DataN`-binding chain. Called from handlers as
//!   `name(Data, args)`; classifier rewrites turn `self.name()`
//!   into that shape.
//! - **Non-static operations** get a two-clause function: one
//!   guarded by `is_pid/1` that routes external callers through
//!   `gen_statem:call`, and one that runs the same body as
//!   internal callers (action/handler rewrites). The body shape
//!   is the same as actions.
//! - **Static operations** are pure functions: no `Data` thread,
//!   pure expression body. Reserved-by-attribute ops
//!   (`@@[save]`, `@@[load]`) are skipped here; the persist
//!   module emits them.
//!
//! The `@@:(<expr>)` system-state sigil is lowered upstream by
//! the operation pipeline; we re-run it on action bodies because
//! the action emission path used to bypass that lowering.
//!
//! All call sites are reachable from `generate_erlang_system`
//! after `runtime_helpers::emit_runtime_helpers` and the
//! per-state callback emission; placing them here means
//! actions/operations appear AFTER state callbacks but BEFORE
//! persist methods in the generated module — same order Erlang
//! Frame programmers expect for hand-rolled modules.

use super::blocks::{erlang_smart_join, erlang_transform_blocks};
use super::body_processor::{erlang_capitalize_params, erlang_process_body_lines_full};
use super::lexical::{erlang_op_name, erlang_safe_capitalize};
use super::super::codegen_utils::{replace_outside_strings_and_comments, to_snake_case};
use crate::frame_c::compiler::frame_ast::SystemAst;
use crate::frame_c::visitors::TargetLanguage;

pub(super) fn emit_actions_and_operations(
    code: &mut String,
    system: &SystemAst,
    source: &[u8],
    action_names: &[String],
    interface_names: &[String],
) {
    // Action functions. Same naming rule as operations — Erlang
    // function names must be lowercase-leading atoms.
    for action in &system.actions {
        code.push_str(&format!("{}(", erlang_op_name(&action.name)));
        let params: Vec<String> = action
            .params
            .iter()
            .map(|p| erlang_safe_capitalize(&p.name))
            .collect();
        // Actions receive Data as first param
        let mut all_params = vec!["Data".to_string()];
        all_params.extend(params);
        code.push_str(&all_params.join(", "));
        code.push_str(") ->\n");
        {
            let body_span = &action.body.span;
            let body_bytes = &source[body_span.start..body_span.end];
            let body_text = std::str::from_utf8(body_bytes).unwrap_or("    ok");
            let trimmed = body_text.trim();
            let inner_raw = if trimmed.starts_with('{') && trimmed.ends_with('}') {
                trimmed[1..trimmed.len() - 1].trim().to_string()
            } else {
                trimmed.to_string()
            };
            // Lower `@@:(<expr>)` to bare `<expr>` (Erlang's last
            // expression is the return value). Operations get this
            // upstream; actions did not, causing the literal sigil to
            // leak into the generated source.
            let inner_raw = super::super::frame_expansion::expand_system_state_in_code(
                &inner_raw,
                TargetLanguage::Erlang,
            );
            // Run `if {...} else {...}` → `case X of true -> ...; false -> ... end`
            // conversion over the action body before the classifier pipeline.
            // Handler bodies get this via erlang_transform_blocks at their
            // emission sites; action bodies previously skipped it, causing
            // malformed output for any action containing control flow.
            let transformed = erlang_transform_blocks(&inner_raw);
            let inner = transformed.as_str();
            // Build param name mappings for capitalization
            let act_params: Vec<(&str, String)> = action
                .params
                .iter()
                .map(|p| {
                    let cap = erlang_safe_capitalize(&p.name);
                    (p.name.as_str(), cap)
                })
                .collect();
            let lines: Vec<&str> = inner.lines().collect();
            // Pass `interface_names` so `@@:self.method(args)` calls inside
            // an action body route through the classifier's `InterfaceCall`
            // rewrite (`{DataN, Result} = frame_dispatch__(method, [args],
            // DataPrev)`) rather than collapsing to `Data#data.method(args)`.
            // Transition-guard wrapping is intentionally *not* applied at
            // the action level — actions return `{Data, RetVal}` not a
            // gen_statem tuple, and the calling handler's own guard picks
            // up the new state after the action returns with the transitioned
            // Data.
            let (processed, final_data, _final_rv) = erlang_process_body_lines_full(
                &lines,
                &action_names,
                &interface_names,
                "Data",
                &act_params,
            );
            if processed.is_empty() {
                // No body — return {Data, ok}
                code.push_str("    {Data, ok}");
            } else {
                // Check if last processed line is a value expression (not a Data assignment)
                let last_line = processed
                    .last()
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default();
                // `Data#data.<field>` is a domain READ (a value);
                // `Data<N> = ...` is a rebind (not a value). The
                // prior `starts_with("Data")` check lumped both
                // together and silently dropped legitimate value
                // returns shaped as field reads.
                let is_data_rebind = {
                    let t = last_line.as_str();
                    if t.starts_with("Data") {
                        let rest = &t[4..];
                        let num_end = rest
                            .char_indices()
                            .take_while(|(_, c)| c.is_ascii_digit())
                            .last()
                            .map(|(i, _)| i + 1)
                            .unwrap_or(0);
                        num_end > 0 && rest[num_end..].trim_start().starts_with('=')
                    } else {
                        false
                    }
                };
                let last_is_value = !is_data_rebind
                    && !last_line.starts_with("__")
                    && !last_line.is_empty()
                    && !last_line.starts_with("{")
                    && !last_line.starts_with("ok")
                    // A trailing case-block or control-flow closer is a
                    // statement, not a value expression. Emit `{Data, ok}`
                    // rather than trying to return `end` as a value.
                    && last_line != "end"
                    && last_line != "end,"
                    && last_line != "end;"
                    && !last_line.starts_with("; false")
                    && !last_line.starts_with("; _")
                    && !last_line.starts_with("true ->");

                // If the last line is an ActionCall binding shaped
                // `{DataN, __ActionResultN} = ...`, the action's return
                // value IS `__ActionResultN`. Extract it so the tuple
                // wrap returns the called op's result instead of `ok`.
                // Without this, `return self.Op();` at the action's tail
                // resolves to `{Data, ok}` — dropping the op's value.
                let action_result_var: Option<String> = {
                    let t = last_line.trim();
                    if t.starts_with('{') && t.contains("__ActionResult") {
                        t.find("__ActionResult").map(|i| {
                            let rest = &t[i..];
                            rest.chars()
                                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect::<String>()
                        })
                    } else {
                        None
                    }
                };

                // Detect a multi-line trailing `case ... of … end` or
                // `if ... -> … end` block as the action's return
                // value. Without this, an action whose body is a
                // multi-line case-as-value lowers to `{Data, ok}`,
                // dropping the user's intended return.
                let trailing_block_start: Option<usize> =
                    if matches!(last_line.as_str(), "end" | "end," | "end.") {
                        let mut depth = 1i32;
                        let mut start: Option<usize> = None;
                        for i in (0..processed.len() - 1).rev() {
                            let lt = processed[i].trim();
                            if lt == "end" || lt == "end," || lt == "end." {
                                depth += 1;
                            } else if (lt.starts_with("case ") || lt.starts_with("case("))
                                && (lt.ends_with(" of") || lt.ends_with(" of,"))
                            {
                                depth -= 1;
                                if depth == 0 {
                                    start = Some(i);
                                    break;
                                }
                            } else if lt.starts_with("if ") && lt.ends_with(" ->") {
                                depth -= 1;
                                if depth == 0 {
                                    start = Some(i);
                                    break;
                                }
                            }
                        }
                        start
                    } else {
                        None
                    };

                if let Some(block_start) = trailing_block_start {
                    // Emit pre-block lines (if any), bind the block to
                    // a fresh result var, return `{Data, var}`.
                    if block_start > 0 {
                        erlang_smart_join(&processed[..block_start], code);
                        code.push_str(",\n");
                    }
                    code.push_str("    __ActionRetVal__ = ");
                    let block_lines = &processed[block_start..];
                    // The block's own internal `,` separators between
                    // arms are case-statement structural; smart_join
                    // already handles them correctly.
                    let mut block_buf = String::new();
                    erlang_smart_join(block_lines, &mut block_buf);
                    // Strip the leading "    " from the first line —
                    // the binding adds its own indent.
                    let block_buf = block_buf.trim_start_matches(' ').to_string();
                    code.push_str(&block_buf);
                    code.push_str(",\n");
                    code.push_str(&format!("    {{{}, __ActionRetVal__}}", final_data));
                } else if last_is_value && processed.len() > 0 {
                    // Last expression is the return value — emit body up to last line,
                    // then return {FinalData, LastExpr}
                    let body_lines = &processed[..processed.len() - 1];
                    if !body_lines.is_empty() {
                        erlang_smart_join(body_lines, code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {{{}, {}}}", final_data, last_line));
                } else if let Some(result_var) = action_result_var {
                    // Tail action-call: preserve its result as the action's
                    // return value.
                    erlang_smart_join(&processed, code);
                    code.push_str(",\n");
                    code.push_str(&format!("    {{{}, {}}}", final_data, result_var));
                } else {
                    erlang_smart_join(&processed, code);
                    code.push_str(",\n");
                    code.push_str(&format!("    {{{}, ok}}", final_data));
                }
            }
        }
        code.push_str(".\n\n");
    }

    // Operations. Erlang function names must be lowercase atoms;
    // Frame source names (which may be PascalCase for portability
    // across targets) are snake_cased at declaration and at all
    // call sites (see classifier in `erlang_rewrite_native_classified_full`).
    //
    // Non-static ops get a two-clause shape:
    //   op(Pid, Args...) when is_pid(Pid) -> gen_statem:call(Pid, {frame_op_call, op, [Args...]});
    //   op(Data, Args...) -> <body>.
    // First clause routes external callers through gen_statem so the
    // op runs in the server process with the live Data. Second clause
    // is the same body internal callers (from action/handler rewrites)
    // already use. `is_pid/1` is a guard — safe, builtin, compile-time
    // verified.
    for op in &system.operations {
        // RFC-0012 amendment: framework-managed ops are emitted as
        // module-level functions in the persistence block.
        if op
            .attributes
            .iter()
            .any(|a| a.name == "save" || a.name == "load")
        {
            continue;
        }
        if !op.is_static {
            let op_snake = erlang_op_name(&op.name);
            let param_vars: Vec<String> = op
                .params
                .iter()
                .map(|p| erlang_safe_capitalize(&p.name))
                .collect();
            let args_list = if param_vars.is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", param_vars.join(", "))
            };
            let head_params_ext = if param_vars.is_empty() {
                "Pid".to_string()
            } else {
                format!("Pid, {}", param_vars.join(", "))
            };
            code.push_str(&format!(
                "{}({}) when is_pid(Pid) ->\n    gen_statem:call(Pid, {{frame_op_call, {}, {}}});\n",
                op_snake, head_params_ext, op_snake, args_list
            ));
        }
        code.push_str(&format!("{}(", erlang_op_name(&op.name)));
        let params: Vec<String> = op
            .params
            .iter()
            .map(|p| erlang_safe_capitalize(&p.name))
            .collect();
        if !op.is_static {
            let mut all_params = vec!["Data".to_string()];
            all_params.extend(params);
            code.push_str(&all_params.join(", "));
        } else {
            code.push_str(&params.join(", "));
        }
        code.push_str(") ->\n");
        {
            let body_span = &op.body.span;
            let body_bytes = &source[body_span.start..body_span.end];
            let body_text = std::str::from_utf8(body_bytes).unwrap_or("    ok");
            let trimmed = body_text.trim();
            let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') {
                trimmed[1..trimmed.len() - 1].trim()
            } else {
                trimmed
            };
            // Build param name mappings for capitalization
            let op_params: Vec<(&str, String)> = op
                .params
                .iter()
                .map(|p| {
                    let cap = erlang_safe_capitalize(&p.name);
                    (p.name.as_str(), cap)
                })
                .collect();

            // Expand @@:system.state and @@:(expr) in operation bodies
            let inner =
                super::super::frame_expansion::expand_system_state_in_code(inner, TargetLanguage::Erlang);
            let inner = inner.as_str();

            // Process lines: strip return keyword, capitalize params, rewrite self
            let mut processed_lines: Vec<String> = Vec::new();
            let mut data_bind_counter: usize = 0;
            for line in inner.lines() {
                let l = line.trim();
                if l.is_empty() {
                    continue;
                }
                let l = if l.starts_with("return ") {
                    l.trim_start_matches("return ").trim().to_string()
                } else if l == "return" {
                    "ok".to_string()
                } else {
                    l.to_string()
                };
                // C-family `;` terminators from portable Frame source
                // are not valid inside an Erlang return-expression slot.
                // Strip a trailing `;` so the last-expression emitter
                // can wrap the value cleanly (e.g., `42` not `42;`).
                let l = l.trim_end_matches(';').trim().to_string();
                // `@@:self.method(args)` inside a non-static operation body
                // expands (in frame_expansion.rs) to bare `self.method(args)`.
                // Catch that shape BEFORE the blanket `self. → Data#data.`
                // substitution below — otherwise `self.method(args)` would
                // collapse to `Data#data.method(args)` (record-field access)
                // and lose the dispatch. Routes through `frame_dispatch__`
                // with Data-threading, matching the handler-level semantics.
                // Static operations have no Data parameter and therefore
                // no `@@:self` semantics — this branch only fires when
                // `op.is_static == false`.
                let l = if !op.is_static {
                    let mut out = None;
                    for iface in interface_names {
                        let pattern = format!("self.{}(", iface);
                        if l.contains(&pattern) {
                            data_bind_counter += 1;
                            let prev = if data_bind_counter == 1 {
                                "Data".to_string()
                            } else {
                                format!("Data{}", data_bind_counter - 1)
                            };
                            let new_var = format!("Data{}", data_bind_counter);
                            let method_snake = to_snake_case(iface);
                            let call_start = l.find(&pattern).unwrap() + pattern.len();
                            let call_end = l.rfind(')').unwrap_or(l.len());
                            let args = l[call_start..call_end].trim();
                            let args_list = if args.is_empty() {
                                "[]".to_string()
                            } else {
                                format!("[{}]", args)
                            };
                            let lhs = if let Some(eq_pos) = l.find(" = ") {
                                let raw = l[..eq_pos].trim();
                                let mut chars = raw.chars();
                                match chars.next() {
                                    None => "_".to_string(),
                                    Some(c) => {
                                        c.to_uppercase().collect::<String>() + chars.as_str()
                                    }
                                }
                            } else {
                                "_".to_string()
                            };
                            out = Some(format!(
                                "{{{}, {}}} = frame_dispatch__({}, {}, {})",
                                new_var, lhs, method_snake, args_list, prev
                            ));
                            break;
                        }
                    }
                    out.unwrap_or(l)
                } else {
                    l
                };
                // `self.<action_or_op>(args)` — direct call into a
                // sibling operation or action. Rewrite to lowercase
                // function call with Data threaded as first arg BEFORE
                // the blanket `self. → Data#data.` step below (which
                // would otherwise collapse it to a record-field read).
                // Non-static ops only — static ops have no Data.
                let l = if !op.is_static {
                    let mut out = l.clone();
                    for action in action_names {
                        let pattern = format!("self.{}(", action);
                        if out.contains(&pattern) {
                            let action_lc = erlang_op_name(action.as_str());
                            let rep = format!("{}(Data, ", action_lc);
                            out = out.replace(&pattern, &rep);
                            out = out.replace(
                                &format!("{}(Data, )", action_lc),
                                &format!("{}(Data)", action_lc),
                            );
                        }
                    }
                    out
                } else {
                    l
                };
                let l = replace_outside_strings_and_comments(
                    &l,
                    TargetLanguage::Erlang,
                    &[("self.", "Data#data.")],
                );
                // Detect domain field assignment: Data#data.field = value
                // Rewrite to Erlang record update with sequential bindings:
                //   Data1 = Data#data{field = Value}
                //   Data2 = Data1#data{field2 = Value2}
                // Erlang is single-assignment — can't rebind Data.
                let l = {
                    if let Some(eq_pos) = l.find(" = ") {
                        let lhs = l[..eq_pos].trim();
                        if lhs.starts_with("Data#data.") {
                            let field = &lhs["Data#data.".len()..];
                            let rhs = l[eq_pos + 3..]
                                .trim()
                                .trim_end_matches(|c: char| c == ',' || c == '.');
                            data_bind_counter += 1;
                            let prev = if data_bind_counter == 1 {
                                "Data".to_string()
                            } else {
                                format!("Data{}", data_bind_counter - 1)
                            };
                            format!(
                                "Data{} = {}#data{{{} = {}}}",
                                data_bind_counter, prev, field, rhs
                            )
                        } else {
                            l
                        }
                    } else {
                        // Replace reads of Data#data. with latest binding
                        if data_bind_counter > 0 {
                            let binding = format!("Data{}#data.", data_bind_counter);
                            replace_outside_strings_and_comments(
                                &l,
                                TargetLanguage::Erlang,
                                &[("Data#data.", binding.as_str())],
                            )
                        } else {
                            l
                        }
                    }
                };
                let l = if op_params.is_empty() {
                    l
                } else {
                    erlang_capitalize_params(&l, &op_params)
                };
                processed_lines.push(format!("    {}", l));
            }
            if processed_lines.is_empty() {
                code.push_str("    ok");
            } else if !op.is_static && data_bind_counter > 0 {
                // Non-static op that mutated Data (`self.x = ...` emitted
                // record updates into Data1, Data2, ...). Callers expect
                // a `{UpdatedData, Value}` tuple so updates are visible
                // at the call site. Wrap the body's last value expression
                // with the latest Data binding — mirroring the action-
                // body tuple-wrap at line ~2625.
                let last_line = processed_lines
                    .last()
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default();
                // A `Data#data.<field>` READ is a value; a `DataN = ...`
                // REBIND is not. Distinguish precisely.
                let is_data_rebind = {
                    let t = last_line.as_str();
                    if t.starts_with("Data") {
                        let rest = &t[4..];
                        let num_end = rest
                            .char_indices()
                            .take_while(|(_, c)| c.is_ascii_digit())
                            .last()
                            .map(|(i, _)| i + 1)
                            .unwrap_or(0);
                        num_end > 0 && rest[num_end..].trim_start().starts_with('=')
                    } else {
                        false
                    }
                };
                let last_is_value = !is_data_rebind
                    && !last_line.starts_with("__")
                    && !last_line.is_empty()
                    && !last_line.starts_with("{")
                    && last_line != "ok"
                    && last_line != "end"
                    && last_line != "end,"
                    && last_line != "end;";
                let final_data = format!("Data{}", data_bind_counter);
                if last_is_value {
                    let body_lines = &processed_lines[..processed_lines.len() - 1];
                    if !body_lines.is_empty() {
                        erlang_smart_join(body_lines, code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {{{}, {}}}", final_data, last_line));
                } else {
                    erlang_smart_join(&processed_lines, code);
                    code.push_str(",\n");
                    code.push_str(&format!("    {{{}, ok}}", final_data));
                }
            } else if !op.is_static {
                // Non-static op with no Data mutations — Data passes
                // through unchanged; wrap the last expression as
                // `{Data, Value}`. An expression using `Data#data.<f>`
                // is a domain READ (still a value), not a Data rebind.
                // A line starting with `Data<digit> = ` IS a rebind
                // and must not be wrapped. A tail call to ANOTHER
                // non-static op/action — recognizable as `<name>(Data)`
                // or `<name>(Data, …)` for a name in `action_names` —
                // already returns a `{Data, Value}` tuple and must
                // pass through verbatim (otherwise we double-wrap).
                let last_line = processed_lines
                    .last()
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default();
                let is_data_rebind = {
                    let t = last_line.as_str();
                    if t.starts_with("Data") {
                        let rest = &t[4..];
                        let num_end = rest
                            .char_indices()
                            .take_while(|(_, c)| c.is_ascii_digit())
                            .last()
                            .map(|(i, _)| i + 1)
                            .unwrap_or(0);
                        num_end > 0 && rest[num_end..].trim_start().starts_with('=')
                    } else {
                        false
                    }
                };
                let is_tail_tuple_call = action_names.iter().any(|a| {
                    let lc = erlang_op_name(a.as_str());
                    let pat_no_args = format!("{}(Data)", lc);
                    let pat_with_args = format!("{}(Data,", lc);
                    last_line == pat_no_args || last_line.starts_with(&pat_with_args)
                });
                let last_is_value = !is_data_rebind
                    && !is_tail_tuple_call
                    && !last_line.starts_with("__")
                    && !last_line.is_empty()
                    && !last_line.starts_with("{")
                    && last_line != "ok"
                    && last_line != "end"
                    && last_line != "end,"
                    && last_line != "end;";
                if last_is_value && processed_lines.len() >= 1 {
                    let body_lines = &processed_lines[..processed_lines.len() - 1];
                    if !body_lines.is_empty() {
                        erlang_smart_join(body_lines, code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {{Data, {}}}", last_line));
                } else if is_tail_tuple_call {
                    // Forward another tuple-returning call's result
                    // straight through — preserves Data threading.
                    erlang_smart_join(&processed_lines, code);
                } else {
                    erlang_smart_join(&processed_lines, code);
                }
            } else {
                // Static op: pure function, no Data threading.
                erlang_smart_join(&processed_lines, code);
            }
        }
        code.push_str(".\n\n");
    }
}
