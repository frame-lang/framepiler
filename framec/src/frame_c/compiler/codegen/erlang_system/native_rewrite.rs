//! Per-line native-code rewriting for Erlang body emission.
//!
//! When the body processor (`erlang_process_body_lines` and friends
//! in `erlang_system.rs`) walks the spliced handler text, each line
//! that isn't already structural Erlang (`case`/`end`/`->` arms,
//! `frame_transition__` calls, etc.) gets classified here. The
//! classifier rewrites the line for Erlang syntax and returns an
//! `ErlangRewrite` variant that tells the body processor which
//! Data-threading shape to emit:
//!
//! - `ActionCall` — wrap in `DataN = action(DataPrev, ...)` so the
//!   action's Data return chains forward.
//! - `ActionCallWithBind` — `self.<field> = self.<action>(args)`,
//!   needs two emissions: the action bind, then a record-update of
//!   that bind into the field.
//! - `InterfaceCall` — wrap in `{DataN, Result} =
//!   frame_dispatch__(method, [args], DataPrev)`.
//! - `InterfaceCallWithBind` — `self.<field> = self.<iface>(args)`,
//!   the interface-call variant of ActionCallWithBind.
//! - `RecordUpdate` — `self.<field> = expr` → `DataN =
//!   DataPrev#data{field = expr}`.
//! - `Plain` — no Data write; emit as-is (after `self.field` →
//!   `DataN#data.field` read substitution).
//! - `Reply` — set the trailing return-value (gen_statem reply
//!   tuple); constructed elsewhere in `erlang_system.rs`.
//!
//! The two entry points differ only in interface-call awareness:
//! `_full` accepts an `interface_names` slice so it can recognise
//! `self.<iface>(...)` and emit the dispatch path. The shorter
//! arity wraps `_full` with an empty interface list for call sites
//! that haven't been threaded yet.

use super::super::codegen_utils::{replace_outside_strings_and_comments, to_snake_case};
use super::lexical::erlang_op_name;
use crate::frame_c::visitors::TargetLanguage;

/// Result of rewriting a line of native code for Erlang.
pub(super) enum ErlangRewrite {
    /// A Data-modifying action call: needs `DataN = action(DataPrev)`
    ActionCall(String),
    /// Source pattern: `self.<field> = self.<action>(args)`. Emits two
    /// statements — `{DataN, __ActionResultN} = <call>` then
    /// `DataN+1 = DataN#data{<field> = __ActionResultN}`. Keeps record-
    /// update and action-bind composable when the action returns a
    /// value the caller wants stored in domain.
    ActionCallWithBind { field: String, call: String },
    /// Source pattern: `self.<field> = self.<interface>(args)`. Emits
    /// the interface dispatch as a two-tuple bind and then writes the
    /// result into the record field. Parallel to `ActionCallWithBind`
    /// but for interface calls (which go through `frame_dispatch__`
    /// rather than a direct action function).
    InterfaceCallWithBind {
        field: String,
        method: String,
        args: String,
    },
    /// A Data record update: needs `DataN = DataPrev#data{field = value}`
    RecordUpdate { field: String, value: String },
    /// An interface dispatch call: `{DataN, Result} = frame_dispatch__(method, [args], DataPrev)`
    InterfaceCall {
        method: String,
        args: String,
        result_var: String,
    },
    /// A plain expression (no Data modification)
    Plain(String),
    /// A return-value reply
    Reply(String),
}

/// Rewrite a line of native code for Erlang, classifying the result.
pub(super) fn erlang_rewrite_native_classified(
    line: &str,
    action_names: &[String],
    data_var: &str,
) -> ErlangRewrite {
    erlang_rewrite_native_classified_full(line, action_names, &[], data_var)
}

pub(super) fn erlang_rewrite_native_classified_full(
    line: &str,
    action_names: &[String],
    interface_names: &[String],
    data_var: &str,
) -> ErlangRewrite {
    let l = line.trim();

    // `self.<field> = self.<iface>(args)` — StateVar/domain write whose
    // RHS is an interface call. Must be checked BEFORE the bare
    // InterfaceCall branch, otherwise the generic path captures
    // `result_var = "self.<field>"` which emits as an invalid Erlang
    // pattern (`Self.<field>`). Splits into the dispatch bind + a
    // record update chained through DataN.
    for iface in interface_names {
        let call_pat = format!("self.{}(", iface);
        if l.starts_with("self.") && l.contains(" = ") && l.contains(&call_pat) {
            if let Some(eq_pos) = l.find(" = ") {
                let lhs = l[..eq_pos].trim();
                let rhs = l[eq_pos + 3..].trim().trim_end_matches(';').trim();
                // Only match when the LHS is a bare `self.<field>` (no
                // further dots / calls) — a domain or state-var write.
                let lhs_field = lhs.strip_prefix("self.").unwrap_or("");
                let lhs_is_simple_field =
                    !lhs_field.is_empty() && !lhs_field.contains('.') && !lhs_field.contains('(');
                if lhs_is_simple_field && rhs.starts_with(&call_pat) {
                    // rhs = `self.<iface>(<args>)` — strip the wrapper
                    // to get just `<args>`.
                    let inner_start = call_pat.len();
                    let inner_end = rhs.rfind(')').unwrap_or(rhs.len());
                    let args = rhs[inner_start..inner_end].trim().to_string();
                    return ErlangRewrite::InterfaceCallWithBind {
                        field: lhs_field.to_string(),
                        method: to_snake_case(iface),
                        args,
                    };
                }
            }
        }
    }

    // self.method(args) → interface dispatch (for interface methods).
    //
    // When the line contains multiple `self.<iface>(` patterns (e.g.
    // `self.dbl(self.echo(X))`), pick the OUTERMOST call — the one
    // whose opening paren appears EARLIEST in the line. Iteration
    // order over interface_names is not meaningful for that choice.
    // Inner calls nested in the args are re-classified recursively by
    // the body-processor's InterfaceCall handler.
    let mut best: Option<(&String, usize)> = None;
    for iface in interface_names {
        let pattern = format!("self.{}(", iface);
        if let Some(pos) = l.find(&pattern) {
            match best {
                None => best = Some((iface, pos)),
                Some((_, cur_pos)) if pos < cur_pos => best = Some((iface, pos)),
                _ => {}
            }
        }
    }
    if let Some((iface, open_pos)) = best {
        // Parenthesis-match to find the matching close for the
        // call's opening paren. Using `rfind(')')` could land on a
        // paren from an ENCLOSING / following expression (e.g.
        // `... self.echo(X) + 1)` would cut `X) + 1`).
        let pattern = format!("self.{}(", iface);
        let call_start = open_pos + pattern.len();
        let open_paren_idx = call_start - 1;
        let bytes = l.as_bytes();
        let mut depth = 0i32;
        let mut call_end = l.len();
        for i in open_paren_idx..bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        call_end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let method_snake = to_snake_case(iface);
        let args = l[call_start..call_end].trim().to_string();
        if let Some(eq_pos) = l.find('=') {
            // Only treat as `lhs = self.method(...)` when the `=` is
            // BEFORE the call — otherwise it's a `==` / `>=` / an
            // equality inside the args, not an assignment.
            if eq_pos < open_pos {
                let result_var = l[..eq_pos].trim().to_string();
                return ErlangRewrite::InterfaceCall {
                    method: method_snake,
                    args,
                    result_var,
                };
            }
        }
        return ErlangRewrite::InterfaceCall {
            method: method_snake,
            args,
            result_var: "_".to_string(),
        };
    }

    // self.field = self.method(args) — record-update whose RHS is an
    // action/op call. Split into two statements:
    //   1. {DataN, __ActionResultN} = <action>(Data, args)
    //   2. DataN+1 = DataN#data{field = __ActionResultN}
    // Done here rather than in the body-processor dispatch because
    // the classifier sees the full original line and can decide in
    // one pass without re-parsing. Without this branch the prior
    // `ActionCall` path would prepend its destructure to the whole
    // line including the `self.field =` prefix — emitting invalid
    // Erlang like `{Data1, __ActionResult1} = self.field = op(Data)`.
    //
    // Erlang function names must be lowercase atoms, so the emitted
    // call uses `to_snake_case(action)` not the source-side name.
    for action in action_names {
        let call_pattern = format!("self.{}(", action);
        if l.starts_with("self.") && l.contains('=') && l.contains(&call_pattern) {
            if let Some(eq_pos) = l[5..].find('=') {
                let field = l[5..5 + eq_pos].trim().to_string();
                let rhs = l[5 + eq_pos + 1..].trim().trim_end_matches(';').trim();
                if rhs.contains(&call_pattern) {
                    let action_lc = erlang_op_name(action);
                    let rewritten_call = rhs
                        .replace(&call_pattern, &format!("{}({}, ", action_lc, data_var))
                        .replace(&format!("({}, )", data_var), &format!("({})", data_var));
                    return ErlangRewrite::ActionCallWithBind {
                        field,
                        call: rewritten_call,
                    };
                }
            }
        }
    }

    // self.method(args) → action call that modifies Data. Strip any
    // trailing C-family `;` terminator — Erlang uses `,` between
    // statements in a body and the ActionCall wrapper supplies it.
    for action in action_names {
        let pattern = format!("self.{}(", action);
        if l.contains(&pattern) {
            let action_lc = erlang_op_name(action);
            let replaced = l.replace(&pattern, &format!("{}({}, ", action_lc, data_var));
            let fixed = replaced
                .replace(&format!("({}, )", data_var), &format!("({})", data_var))
                .trim_end_matches(';')
                .trim()
                .to_string();
            return ErlangRewrite::ActionCall(fixed);
        }
    }

    // self.field = expr → record update.
    // String-aware replacement on the RHS so a `self.` appearing inside
    // a string literal in the expression isn't mangled. A trailing
    // semicolon (`self.x = v;`) is stripped because C-family Frame
    // bodies use `;` as a statement terminator but Erlang's record
    // update `Data#data{x = v}` can't carry it — `,`/`.` are the only
    // separators Erlang accepts at that position.
    if l.starts_with("self.") && l.contains('=') {
        let rest = &l[5..]; // skip "self."
        if let Some(eq_pos) = rest.find('=') {
            let field = rest[..eq_pos].trim().to_string();
            let rhs = rest[eq_pos + 1..].trim().trim_end_matches(';').trim();
            let replacement = format!("{}#data.", data_var);
            let value = replace_outside_strings_and_comments(
                rhs,
                TargetLanguage::Erlang,
                &[("self.", replacement.as_str())],
            );
            return ErlangRewrite::RecordUpdate { field, value };
        }
    }

    // self.field → DataVar#data.field (access). String-aware so a
    // `self.x` inside a quoted string in the line is preserved.
    let replacement = format!("{}#data.", data_var);
    ErlangRewrite::Plain(replace_outside_strings_and_comments(
        l,
        TargetLanguage::Erlang,
        &[("self.", replacement.as_str())],
    ))
}
