//! Erlang gen_statem code generation.
//!
//! This module generates complete Erlang/OTP gen_statem modules from Frame systems.
//! It bypasses the standard class-based CodegenNode pipeline entirely, producing
//! raw Erlang source text with proper gen_statem callbacks, -record(data, {}),
//! and Frame infrastructure (frame_transition__, frame_dispatch__, etc.).

use super::ast::CodegenNode;
use super::codegen_utils::{
    convert_expression, convert_literal, expression_to_string, is_bool_type, is_float_type,
    is_int_type, is_string_type, replace_outside_strings_and_comments, to_snake_case,
    type_to_string, HandlerContext,
};
use super::frame_expansion::emit_handler_body_via_statements;
use crate::frame_c::compiler::arcanum::Arcanum;
use crate::frame_c::compiler::frame_ast::{Expression, Literal, SystemAst, Type};
use crate::frame_c::compiler::native_region_scanner::erlang::NativeRegionScannerErlang;
use crate::frame_c::visitors::TargetLanguage;

/// Generate a complete Erlang gen_statem module from a Frame system.
/// This bypasses the standard class-based pipeline entirely.
/// Result of rewriting a line of native code for Erlang
enum ErlangRewrite {
    /// A Data-modifying action call: needs `DataN = action(DataPrev)`
    ActionCall(String), // The action call expression
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

/// Rewrite a line of native code for Erlang, classifying the result
fn erlang_rewrite_native_classified(
    line: &str,
    action_names: &[String],
    data_var: &str,
) -> ErlangRewrite {
    erlang_rewrite_native_classified_full(line, action_names, &[], data_var)
}

fn erlang_rewrite_native_classified_full(
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
                let lhs_is_simple_field = !lhs_field.is_empty()
                    && !lhs_field.contains('.')
                    && !lhs_field.contains('(');
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
                let rhs = l[5 + eq_pos + 1..]
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                if rhs.contains(&call_pattern) {
                    let action_lc = erlang_op_name(action);
                    let rewritten_call = rhs
                        .replace(&call_pattern, &format!("{}({}, ", action_lc, data_var))
                        .replace(
                            &format!("({}, )", data_var),
                            &format!("({})", data_var),
                        );
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
            let rhs = rest[eq_pos + 1..]
                .trim()
                .trim_end_matches(';')
                .trim();
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

/// Word-boundary string substitution. Replaces `needle` with `replacement`
/// only when `needle` appears as a complete identifier (surrounded by
/// non-word chars or string boundaries). Used to substitute Frame param
/// names with their capitalized Erlang variable names in domain field
/// initializer expressions.
fn replace_word(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    let n = bytes.len();
    let m = needle_bytes.len();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = String::with_capacity(n);
    let mut i = 0;
    while i < n {
        if i + m <= n && bytes[i..i + m] == *needle_bytes {
            let prev_ok = i == 0 || !is_word(bytes[i - 1]);
            let next_ok = i + m == n || !is_word(bytes[i + m]);
            if prev_ok && next_ok {
                out.push_str(replacement);
                i += m;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Rewrite an operation/action name to a valid Erlang function atom.
/// Erlang function names MUST start with a lowercase letter. Names
/// already starting lowercase pass through unchanged — preserving
/// idioms like `addOffset`, `my_action` that hand-authored Erlang
/// Frame sources use. PascalCase names (`Op`, `OpOuter`, `Bump`)
/// have their leading capital lowercased to satisfy Erlang's atom
/// rules without snake-casing the interior (which would be more
/// disruptive than necessary).
fn erlang_op_name(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(c) if c.is_ascii_uppercase() => {
            c.to_ascii_lowercase().to_string() + chars.as_str()
        }
        Some(_) => name.to_string(),
    }
}

/// Word-boundary substring search. Returns true iff `needle` appears in
/// `haystack` as a complete identifier (surrounded by non-word chars or
/// string boundaries). Used to detect whether a domain field's raw
/// initializer references a system param by name.
fn raw_contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    let n = bytes.len();
    let m = needle_bytes.len();
    if m > n {
        return false;
    }
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i + m <= n {
        if bytes[i..i + m] == *needle_bytes {
            let prev_ok = i == 0 || !is_word(bytes[i - 1]);
            let next_ok = i + m == n || !is_word(bytes[i + m]);
            if prev_ok && next_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Capitalize a parameter name for Erlang, avoiding collisions with gen_statem reserved names.
/// "data" → "Data_Arg" (not "Data" which collides with the gen_statem state data variable)
/// "from" → "From_Arg" (not "From" which collides with the gen_statem caller reference)
fn erlang_safe_capitalize(name: &str) -> String {
    let capitalized = {
        let mut chars = name.chars();
        match chars.next() {
            None => String::new(),
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        }
    };
    // Reserved gen_statem variable names
    if matches!(
        capitalized.as_str(),
        "Data" | "From" | "State" | "OldState" | "Pid"
    ) {
        format!("{}_Arg", capitalized)
    } else {
        capitalized
    }
}

/// Capitalize handler parameter names in a line of code.
/// Erlang variables must start with uppercase — `n` → `N`, `name` → `Name`
fn erlang_capitalize_params(line: &str, param_names: &[(&str, String)]) -> String {
    let mut result = line.to_string();
    // Replace longest names first to avoid partial matches
    let mut sorted_params: Vec<_> = param_names.to_vec();
    sorted_params.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    for (original, capitalized) in &sorted_params {
        // Word-boundary replacement: only replace standalone identifiers
        let mut new_result = String::new();
        let mut chars = result.chars().peekable();
        let mut i = 0;
        let orig_len = original.len();
        while i < result.len() {
            if result[i..].starts_with(original) {
                // Check word boundaries
                // Don't capitalize identifiers inside record access patterns (#record.field)
                let prev_byte = result.as_bytes()[i.saturating_sub(1)];
                let before_ok = i == 0
                    || !prev_byte.is_ascii_alphanumeric()
                        && prev_byte != b'_'
                        && prev_byte != b'#'
                        && prev_byte != b'.';
                let after_ok = i + orig_len >= result.len()
                    || !result.as_bytes()[i + orig_len].is_ascii_alphanumeric()
                        && result.as_bytes()[i + orig_len] != b'_';
                if before_ok && after_ok {
                    new_result.push_str(capitalized);
                    i += orig_len;
                    continue;
                }
            }
            new_result.push(result.as_bytes()[i] as char);
            i += 1;
        }
        result = new_result;
    }
    result
}

/// Process a sequence of native lines, threading Data through modifications.
/// Returns (processed_lines, final_data_var) where final_data_var tracks
/// the most recent Data binding (Data, Data1, Data2, etc.)
/// Processed handler body, its final Data variable, and the final
/// `__ReturnVal` generation name. Callers use `final_rv_name` in the
/// gen_statem reply tuple rather than hardcoding `__ReturnVal` — the
/// body processor may have SSA-renamed earlier writes to avoid
/// Erlang's single-assignment collision, so the last write's actual
/// name is the authoritative one. If no writes happened,
/// `final_rv_name` is `"ok"` (the gen_statem default reply value).
type ErlangBodyResult = (Vec<String>, String, String);

fn erlang_process_body_lines(
    lines: &[&str],
    action_names: &[String],
    initial_data: &str,
) -> ErlangBodyResult {
    erlang_process_body_lines_with_params(lines, action_names, initial_data, &[])
}

fn erlang_process_body_lines_with_params(
    lines: &[&str],
    action_names: &[String],
    initial_data: &str,
    param_names: &[(&str, String)],
) -> ErlangBodyResult {
    erlang_process_body_lines_full(lines, action_names, &[], initial_data, param_names)
}

/// Tracks per-arm data-threading state so each case's arms can converge
/// on a single `DataN` binding that downstream code can reference.
///
/// # The bug this structure exists to prevent
///
/// Frame's Erlang backend threads the record-typed compartment through
/// statements by binding a fresh `DataN` each time a field is updated:
///
/// ```text
/// Data1 = Data0#compartment{...},
/// Data2 = Data1#compartment{...},
/// ```
///
/// The counter is the `data_gen` field on this module's outer loop. When
/// a `case ... of true -> ...; false -> ... end` block appears, each arm
/// is emitted with its own run of updates. If the two arms emit different
/// numbers of updates, the arms end with *different* names:
///
/// ```text
/// case X of
///   true  -> Data1 = ..., Data2 = ...;   % ends at Data2
///   false -> Data1 = ...                 % ends at Data1
/// end,
/// %% What's bound after the case? Erlang says: whichever arm ran.
/// %% But our caller wants ONE name to reference — and whichever we
/// %% pick is wrong on the other arm's path.
/// ```
///
/// Before this fix, the outer loop simply popped the saved state at
/// `end` and kept using the LAST arm's final gen, so the short arm left
/// its binding unreferenced (dead) while callers wrote code that only
/// worked on the long-arm path.
///
/// # Why rebind-based unification (not case-as-expression)
///
/// The idiomatic Erlang fix is to make `case` produce the compartment
/// as its expression value:
///
/// ```text
/// Data2 = case X of
///   true  -> Data1a = ..., Data1a#compartment{...};
///   false -> Data0#compartment{...}
/// end,
/// ```
///
/// That would require the backend to know, at `case` entry, that
/// whatever follows will want a compartment binding — and to re-parent
/// the arms' last expressions accordingly. That transformation touches
/// many code paths (enter/exit handlers, transitions, returns) and is
/// hard to audit for correctness across all of them.
///
/// Instead we rebind: after both arms emit, splice `DataMax = DataN` into
/// the shorter arm so every path ends at the same name. This is exactly
/// what a compiler targeting SSA form would do at a join point — it's
/// the standard way to unify divergent values in scoped-binding targets.
/// The trade-off is slightly less idiomatic generated code (an extra
/// `Data2 = Data1` rebind), but correctness is local to this function
/// and easy to prove.
///
/// # Terminal arms
///
/// Arms ending in `frame_transition__(...)`, `{keep_state,...}`,
/// `{next_state,...}`, or `{stop,...}` return their own value — control
/// leaves the enclosing function, so the post-case `DataN` is never
/// observed on that path. Padding those arms would put dead code after
/// the return statement. We detect and skip them.
///
/// # How the fields are used
///
/// * `saved_var` / `saved_gen`: captured at `true ->`. Restored each
///   time we enter a fresh arm (`; false ->`, `; _ ->`) so every arm
///   starts threading from the same `DataN`.
/// * `arms`: populated as each arm closes (at `; false ->`, `; _ ->`,
///   or at `end`). Each entry is `(result_idx, final_gen)` — the index
///   of the arm's last emitted statement and the arm's final data_gen.
///   At `end` we scan this list to pick the unifying `DataMax`, then
///   splice rebind statements into each shorter arm.
struct CaseFrame {
    /// data_var at case entry (restored at each new arm so arms start
    /// threading from the same point, not cumulatively).
    saved_var: String,
    /// data_gen at case entry (paired with `saved_var`).
    saved_gen: usize,
    /// Closed arms' `(start_idx, last_result_idx, final_data_gen)`.
    /// Populated at `; false ->` / `; _ ->` / `end`. Used at `end` to
    /// compute the join-point gen and splice padding rebinds into
    /// shorter arms. `start_idx` is the first line of the arm in
    /// `result` — used to scan that arm's lines for `__FwdNextN`
    /// bindings so absent vars can be padded with `= undefined`.
    arms: Vec<(usize, usize, usize)>,
    /// First line index of the currently-accumulating arm.
    current_arm_start: usize,
}

fn erlang_process_body_lines_full(
    lines: &[&str],
    action_names: &[String],
    interface_names: &[String],
    initial_data: &str,
    param_names: &[(&str, String)],
) -> ErlangBodyResult {
    let mut result: Vec<String> = Vec::new();
    let mut data_var = initial_data.to_string();
    let mut data_gen = 0;
    let mut case_data_stack: Vec<CaseFrame> = Vec::new();

    // Pre-process: split lines with inline % comments so the comment
    // can't eat trailing syntax (commas, semicolons, record close braces).
    // "code  % comment" → ["code", "% comment"]
    let preprocessed: Vec<String> = lines
        .iter()
        .flat_map(|line| {
            let l = line.trim();
            // Find % that's not inside a string
            let mut in_string = false;
            let mut escape = false;
            for (i, c) in l.char_indices() {
                if escape {
                    escape = false;
                    continue;
                }
                if c == '\\' {
                    escape = true;
                    continue;
                }
                if c == '"' {
                    in_string = !in_string;
                    continue;
                }
                if c == '%' && !in_string && i > 0 {
                    let code = l[..i].trim_end();
                    let comment = &l[i..];
                    if !code.is_empty() {
                        return vec![code.to_string(), comment.to_string()];
                    }
                }
            }
            vec![l.to_string()]
        })
        .collect();
    let lines = preprocessed
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<&str>>();

    for line in &lines {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }

        // Capitalize params — but for self.field = expr, only capitalize the expr part
        let l = if param_names.is_empty() {
            l.to_string()
        } else if l.starts_with("self.") && l.contains('=') {
            // Record update: capitalize only the value part after =
            if let Some(eq_pos) = l.find('=') {
                let field_part = &l[..eq_pos + 1];
                let value_part = &l[eq_pos + 1..];
                format!(
                    "{}{}",
                    field_part,
                    erlang_capitalize_params(value_part, param_names)
                )
            } else {
                erlang_capitalize_params(l, param_names)
            }
        } else {
            erlang_capitalize_params(l, param_names)
        };

        // Pass through Erlang structural lines (case/of/end, return tuples)
        // Check if this is a parent forward call (parent_name({call, From}, ...))
        let is_forward_call =
            l.contains("({call, From},") && !l.starts_with("case") && !l.starts_with("{");

        let is_structural = l.starts_with("case ")
            || l.starts_with("case(")
            || l.starts_with("true ->")
            || l.starts_with("; false")
            || l == "end"
            || l == "end,"
            || l.starts_with("{next_state,")
            || l.starts_with("{keep_state,")
            || l.starts_with("{stop,")
            || l.starts_with("[__Popped")
            || l.starts_with("frame_transition__(")
            || is_forward_call;
        if is_structural {
            // Case-arm unification — see CaseFrame doc for the full rationale.
            //
            //   true ->       push a CaseFrame snapshotting saved_var/gen
            //   ; false ->    record true arm's tail gen; restore saved gen
            //   ; _ ->        same as `; false ->`
            //   end / end,    record final arm's tail gen; pop the frame;
            //                 pad shorter arms with `DataMax = DataN` so
            //                 all non-terminal paths converge on DataMax.
            if l.starts_with("true ->") {
                case_data_stack.push(CaseFrame {
                    saved_var: data_var.clone(),
                    saved_gen: data_gen,
                    arms: Vec::new(),
                    // The `true ->` header goes into `result` below;
                    // the arm body starts on the NEXT line. Defer the
                    // start_idx assignment to right after the push —
                    // but we need the index *after* the `true ->`
                    // header lands in result, which is result.len() + 1
                    // at this point (since the header hasn't been
                    // pushed yet). The next line added to result will
                    // be `    true ->` (with an indent prefix). Use
                    // result.len() + 1 to account for it.
                    current_arm_start: result.len() + 1,
                });
            } else if l.starts_with("; false") || l.starts_with("; _") {
                if let Some(frame) = case_data_stack.last_mut() {
                    // Record the true arm's final position: index of last
                    // emitted line (which came before this `; false ->`).
                    let last_idx = result.len().saturating_sub(1);
                    let start = frame.current_arm_start;
                    frame.arms.push((start, last_idx, data_gen));
                    // The separator itself lands on the next push; the
                    // new arm's body begins after that.
                    frame.current_arm_start = result.len() + 1;
                    data_var = frame.saved_var.clone();
                    data_gen = frame.saved_gen;
                }
            } else if l == "end" || l == "end," {
                if let Some(frame) = case_data_stack.last_mut() {
                    let last_idx = result.len().saturating_sub(1);
                    let start = frame.current_arm_start;
                    frame.arms.push((start, last_idx, data_gen));
                }
                if let Some(frame) = case_data_stack.pop() {
                    // Compute max gen across *non-terminal* arms only.
                    // Arms ending in `frame_transition__(...)`, `{keep_state,...}`,
                    // or `{next_state,...}` return their own value and
                    // don't participate in the post-case data-var flow —
                    // padding them with a trailing rebind would put dead
                    // code after the return, which erlc accepts but the
                    // Erlang runtime treats as unreachable.
                    let is_terminal_line = |l: &str| -> bool {
                        let t = l.trim();
                        t.starts_with("frame_transition__(")
                            || t.starts_with("{next_state,")
                            || t.starts_with("{keep_state,")
                            || t.starts_with("{stop,")
                    };
                    let arm_is_terminal = |idx: usize| -> bool {
                        idx < result.len() && is_terminal_line(&result[idx])
                    };
                    let max_gen = frame
                        .arms
                        .iter()
                        .filter(|(_, last, _)| !arm_is_terminal(*last))
                        .map(|a| a.2)
                        .max()
                        .unwrap_or(frame.saved_gen);

                    // Scan each arm for `__FwdNextN` bindings so we can
                    // pad arms that don't bind them (Erlang rejects an
                    // "unsafe" var in subsequent code if any arm fails
                    // to bind it). Forward binds look like
                    // `{DataK, __FwdNextK} = frame_unwrap_forward__(...)`.
                    let extract_fwd_vars_from_arm = |start: usize, last: usize| -> Vec<String> {
                        let mut vars = Vec::new();
                        if start > last || start >= result.len() {
                            return vars;
                        }
                        let upper = last.min(result.len().saturating_sub(1));
                        for i in start..=upper {
                            if let Some(idx) = result[i].find("__FwdNext") {
                                // Must be a bind (not a bare read). The bind shape
                                // is `... __FwdNextN} = frame_unwrap_forward__`.
                                if result[i].contains("frame_unwrap_forward__(") {
                                    let rest = &result[i][idx..];
                                    let name: String = rest
                                        .chars()
                                        .take_while(|c| {
                                            c.is_ascii_alphanumeric() || *c == '_'
                                        })
                                        .collect();
                                    if !name.is_empty() && !vars.contains(&name) {
                                        vars.push(name);
                                    }
                                }
                            }
                        }
                        vars
                    };
                    let per_arm_fwds: Vec<Vec<String>> = frame
                        .arms
                        .iter()
                        .map(|(s, e, _)| extract_fwd_vars_from_arm(*s, *e))
                        .collect();
                    let all_fwds: Vec<String> = {
                        let mut seen = Vec::new();
                        for arm_fwds in &per_arm_fwds {
                            for v in arm_fwds {
                                if !seen.contains(v) {
                                    seen.push(v.clone());
                                }
                            }
                        }
                        seen
                    };

                    let need_data_pad = max_gen > frame.saved_gen;
                    let need_fwd_pad = !all_fwds.is_empty();
                    if need_data_pad || need_fwd_pad {
                        let max_name = format!("Data{}", max_gen);
                        let mut pads: Vec<(usize, Vec<String>)> = Vec::new();
                        for ((_, last_idx, gen), arm_fwds) in
                            frame.arms.iter().rev().zip(per_arm_fwds.iter().rev())
                        {
                            if arm_is_terminal(*last_idx) {
                                continue;
                            }
                            let mut arm_pads: Vec<String> = Vec::new();
                            if need_data_pad && *gen < max_gen {
                                let src = if *gen == frame.saved_gen {
                                    frame.saved_var.clone()
                                } else {
                                    format!("Data{}", gen)
                                };
                                arm_pads.push(format!("    {} = {}", max_name, src));
                            }
                            if need_fwd_pad {
                                for v in &all_fwds {
                                    if !arm_fwds.contains(v) {
                                        arm_pads.push(format!("    {} = undefined", v));
                                    }
                                }
                            }
                            if !arm_pads.is_empty() {
                                pads.push((*last_idx, arm_pads));
                            }
                        }
                        for (idx, arm_pads) in pads {
                            if idx < result.len() {
                                let trimmed = result[idx].trim_end();
                                if !trimmed.ends_with(',')
                                    && !trimmed.ends_with(';')
                                {
                                    result[idx] = format!("{},", result[idx]);
                                }
                            }
                            // Insert arm_pads in sequence. Each but the last
                            // needs a trailing `,` comma to chain into the
                            // next statement / the post-case line.
                            let insert_at = idx + 1;
                            let n = arm_pads.len();
                            for (k, pad_line) in arm_pads.into_iter().enumerate() {
                                let line = if k + 1 < n {
                                    format!("{},", pad_line)
                                } else {
                                    pad_line
                                };
                                if insert_at + k <= result.len() {
                                    result.insert(insert_at + k, line);
                                } else {
                                    result.push(line);
                                }
                            }
                        }
                        if need_data_pad {
                            data_var = max_name;
                            data_gen = max_gen;
                        }
                    }
                }
            }

            // Rewrite self.action() calls and self.field access in structural lines
            let mut rewritten = l.clone();
            let mut action_extracted = false;
            for action in action_names {
                let pattern = format!("self.{}(", action);
                if rewritten.contains(&pattern) {
                    // Check if this is a case header with an action call in the condition
                    // e.g., "case (self.validate(self.item)) of" → extract action call
                    if rewritten.starts_with("case ") && rewritten.ends_with(" of") {
                        let call_replaced =
                            rewritten.replace(&pattern, &format!("{}({}, ", action, data_var));
                        let call_replaced = call_replaced
                            .replace(&format!("({}, )", data_var), &format!("({})", data_var));
                        // Extract the action call from "case (action_call) of"
                        if let Some(paren_start) = call_replaced.find('(') {
                            if let Some(of_pos) = call_replaced.rfind(") of") {
                                let extracted = &call_replaced[paren_start + 1..of_pos];
                                let data_access = format!("{}#data.", data_var);
                                let action_expr = replace_outside_strings_and_comments(
                                    extracted,
                                    TargetLanguage::Erlang,
                                    &[("self.", data_access.as_str())],
                                );
                                // Emit the action call as a separate line, bind result
                                data_gen += 1;
                                let new_var = format!("Data{}", data_gen);
                                let result_var = format!("__ActionResult{}", data_gen);
                                result.push(format!(
                                    "    {{{}, {}}} = {}",
                                    new_var, result_var, action_expr
                                ));
                                data_var = new_var;
                                // Replace case condition with the result variable
                                rewritten = format!("case ({}) of", result_var);
                                action_extracted = true;
                                break;
                            }
                        }
                    }
                    if !action_extracted {
                        rewritten =
                            rewritten.replace(&pattern, &format!("{}({}, ", action, data_var));
                        rewritten = rewritten
                            .replace(&format!("({}, )", data_var), &format!("({})", data_var));
                    }
                }
            }
            // Rewrite `self.` → `<data_var>#data.` everywhere in the
            // line, skipping string literals and comments so a user
            // string like `"self.x"` isn't mangled.
            let data_access = format!("{}#data.", data_var);
            rewritten = replace_outside_strings_and_comments(
                &rewritten,
                TargetLanguage::Erlang,
                &[("self.", data_access.as_str())],
            );

            // Replace Data with current data_var in return tuples,
            // expressions, and forward calls. Grouped into a single
            // string-aware pass so patterns like `, Data,` inside a
            // user string literal stay intact.
            if data_var != "Data" {
                let data_comma = format!(", {},", data_var);
                let data_brace = format!(", {}}}", data_var);
                let data_paren = format!(", {})", data_var);
                let data_access2 = format!("{}#data.", data_var);
                rewritten = replace_outside_strings_and_comments(
                    &rewritten,
                    TargetLanguage::Erlang,
                    &[
                        (", Data,", data_comma.as_str()),
                        (", Data}", data_brace.as_str()),
                        (", Data)", data_paren.as_str()),
                        ("Data#data.", data_access2.as_str()),
                    ],
                );
            }
            if is_forward_call {
                // Rewrite forward call into a bind so post-forward statements
                // can run. Parent's handler may transition; frame_unwrap_forward__
                // extracts the updated Data and either `undefined` (parent
                // kept state) or the next-state atom (parent transitioned).
                // The caller's final clause then emits a conditional tuple
                // that honors whichever the parent returned — matching the
                // 16-backend consensus that post-forward code always runs.
                let stripped = rewritten
                    .trim_end_matches([',', ';'])
                    .trim()
                    .to_string();
                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                let fwd_var = format!("__FwdNext{}", data_gen);
                result.push(format!(
                    "    {{{}, {}}} = frame_unwrap_forward__({})",
                    new_var, fwd_var, stripped
                ));
                data_var = new_var;
            } else {
                result.push(format!("    {}", rewritten));
            }
            continue;
        }

        // Suppress bare "return" — this is a Frame-generated artifact that has no meaning
        // in Erlang gen_statem (the __ReturnVal mechanism handles returns)
        if l == "return" {
            continue;
        }

        // `return <expr>;` from portable C-family Frame source: Erlang
        // has no `return` keyword, and the last expression in an action
        // or operation body IS the returned value. Strip the prefix so
        // the rest of the line flows through the classifier normally.
        let l = if l.starts_with("return ") {
            l.trim_start_matches("return ").trim().to_string()
        } else {
            l.clone()
        };

        match erlang_rewrite_native_classified_full(&l, action_names, interface_names, &data_var) {
            ErlangRewrite::ActionCall(call) => {
                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                // Actions return {Data, ReturnValue} — destructure the tuple
                result.push(format!(
                    "    {{{}, __ActionResult{}}} = {}",
                    new_var, data_gen, call
                ));
                data_var = new_var;
            }
            ErlangRewrite::ActionCallWithBind { field, call } => {
                // `self.<field> = self.<action>(args)` — emit the call-
                // bind first, then apply the result to the record field
                // in a follow-on DataN+1 binding. Two data_gen bumps.
                data_gen += 1;
                let call_var = format!("Data{}", data_gen);
                let result_name = format!("__ActionResult{}", data_gen);
                result.push(format!(
                    "    {{{}, {}}} = {}",
                    call_var, result_name, call
                ));
                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                result.push(format!(
                    "    {} = {}#data{{{} = {}}}",
                    new_var, call_var, field, result_name
                ));
                data_var = new_var;
            }
            ErlangRewrite::RecordUpdate { field, value } => {
                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                // Erlang string concat is ++ not + — fix when adjacent to string literals.
                // Also strip any trailing `,` / `.` / whitespace: the native
                // text arrives with the user's statement separator attached
                // (e.g. `self.count = self.count + 1,`), and including it
                // inside the record-update braces is a parse error
                // (`Data#data{count = ... ,}`).
                let value = value
                    .trim_end_matches(|c: char| c == ',' || c == '.' || c.is_whitespace())
                    .replace("\" + \"", "\" ++ \"")
                    .replace("\" + ", "\" ++ ")
                    .replace(" + \"", " ++ \"");
                result.push(format!(
                    "    {} = {}#data{{{} = {}}}",
                    new_var, data_var, field, value
                ));
                data_var = new_var;
            }
            ErlangRewrite::InterfaceCallWithBind {
                field,
                method,
                args,
            } => {
                // `self.<field> = self.<iface>(args)` — emit the
                // dispatch bind, then a record update that writes the
                // dispatch's return value into the field.
                //
                // Recursively classify nested `self.<iface>(…)` in
                // args (same logic as the bare InterfaceCall branch
                // below — extract, emit a prior dispatch bind,
                // replace the arg with the bind's result var). Without
                // this the inner `self.echo(…)` would pass through to
                // Erlang as invalid dot-access on the `self` atom.
                let mut args_rewritten = args.clone();
                let mut iter_guard = 0;
                while iter_guard < 16 {
                    iter_guard += 1;
                    // Process INNERMOST call first: use rfind to
                    // pick the LAST `self.<iface>(` occurrence in
                    // the string. In `self.a(self.b(self.c(X)))`
                    // that's `self.c(X)` — its args have no further
                    // `self.` patterns, so the emitted bind is clean.
                    // Repeat until no `self.<iface>(` remains.
                    let mut matched = None;
                    let mut best_pos = 0usize;
                    for iface in interface_names {
                        let pat = format!("self.{}(", iface);
                        if let Some(start) = args_rewritten.rfind(&pat) {
                            if matched.is_none() || start >= best_pos {
                                let open = start + pat.len() - 1;
                                let bytes = args_rewritten.as_bytes();
                                let mut depth = 0i32;
                                let mut end = open;
                                for i in open..bytes.len() {
                                    match bytes[i] {
                                        b'(' => depth += 1,
                                        b')' => {
                                            depth -= 1;
                                            if depth == 0 {
                                                end = i;
                                                break;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                if end > open {
                                    let inner_args =
                                        args_rewritten[open + 1..end].to_string();
                                    matched = Some((
                                        iface.clone(),
                                        start,
                                        end + 1,
                                        inner_args,
                                    ));
                                    best_pos = start;
                                }
                            }
                        }
                    }
                    match matched {
                        None => break,
                        Some((iface, start, after_end, inner_args)) => {
                            data_gen += 1;
                            let bind_data = format!("Data{}", data_gen);
                            let nested_result = format!("__NestedResult{}", data_gen);
                            let nested_args = if inner_args.trim().is_empty() {
                                "[]".to_string()
                            } else {
                                format!("[{}]", inner_args.trim())
                            };
                            let nested_method = to_snake_case(&iface);
                            result.push(format!(
                                "    {{{}, {}}} = frame_dispatch__({}, {}, {})",
                                bind_data, nested_result, nested_method, nested_args, data_var
                            ));
                            data_var = bind_data;
                            args_rewritten = format!(
                                "{}{}{}",
                                &args_rewritten[..start],
                                nested_result,
                                &args_rewritten[after_end..]
                            );
                        }
                    }
                }
                // Rewrite remaining `self.<field>` reads in args.
                let data_access = format!("{}#data.", data_var);
                let args_rewritten = replace_outside_strings_and_comments(
                    &args_rewritten,
                    TargetLanguage::Erlang,
                    &[("self.", data_access.as_str())],
                );

                data_gen += 1;
                let call_data = format!("Data{}", data_gen);
                let result_name = format!("__IfaceResult{}", data_gen);
                let args_list = if args_rewritten.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", args_rewritten)
                };
                result.push(format!(
                    "    {{{}, {}}} = frame_dispatch__({}, {}, {})",
                    call_data, result_name, method, args_list, data_var
                ));
                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                result.push(format!(
                    "    {} = {}#data{{{} = {}}}",
                    new_var, call_data, field, result_name
                ));
                data_var = new_var;
            }
            ErlangRewrite::InterfaceCall {
                method,
                args,
                result_var,
            } => {
                // Internal dispatch: {DataN, Result} = frame_dispatch__(method, [args], DataPrev)
                //
                // If `args` contains nested `self.<iface>(…)` patterns
                // (Erlang expansion of nested `@@:self.method(…)`),
                // classify each as its own dispatch call FIRST, emit
                // its bind, and rewrite the outer arg to reference the
                // nested bind's result var. Without this the outer
                // dispatch would pass raw `self.echo(X)` text through
                // to Erlang, which parses it as invalid dot-access on
                // the atom `self`.
                let mut args_rewritten = args.clone();
                let mut iter_guard = 0;
                while iter_guard < 16 {
                    iter_guard += 1;
                    // Process INNERMOST call first via rfind — in
                    // `self.a(self.b(self.c(X)))` the last-starting
                    // `self.` pattern is `self.c(X)`, whose args have
                    // no further nested calls. Emitting the innermost
                    // bind FIRST avoids leaving unresolved nested
                    // patterns inside an already-emitted bind's args.
                    let mut matched = None;
                    let mut best_pos = 0usize;
                    for iface in interface_names {
                        let pat = format!("self.{}(", iface);
                        if let Some(start) = args_rewritten.rfind(&pat) {
                            if matched.is_none() || start >= best_pos {
                                let open = start + pat.len() - 1;
                                let bytes = args_rewritten.as_bytes();
                                let mut depth = 0i32;
                                let mut end = open;
                                for i in open..bytes.len() {
                                    match bytes[i] {
                                        b'(' => depth += 1,
                                        b')' => {
                                            depth -= 1;
                                            if depth == 0 {
                                                end = i;
                                                break;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                if end > open {
                                    let inner_args =
                                        args_rewritten[open + 1..end].to_string();
                                    matched = Some((
                                        iface.clone(),
                                        start,
                                        end + 1,
                                        inner_args,
                                    ));
                                    best_pos = start;
                                }
                            }
                        }
                    }
                    match matched {
                        None => break,
                        Some((iface, start, after_end, inner_args)) => {
                            data_gen += 1;
                            let bind_data = format!("Data{}", data_gen);
                            let nested_result = format!("__NestedResult{}", data_gen);
                            let nested_args = if inner_args.trim().is_empty() {
                                "[]".to_string()
                            } else {
                                format!("[{}]", inner_args.trim())
                            };
                            let nested_method = to_snake_case(&iface);
                            result.push(format!(
                                "    {{{}, {}}} = frame_dispatch__({}, {}, {})",
                                bind_data, nested_result, nested_method, nested_args, data_var
                            ));
                            data_var = bind_data;
                            args_rewritten = format!(
                                "{}{}{}",
                                &args_rewritten[..start],
                                nested_result,
                                &args_rewritten[after_end..]
                            );
                        }
                    }
                }

                // Any remaining `self.<field>` reads in the args
                // (state-var reads, domain reads) must be rewritten
                // with the current `data_var` prefix — same as the
                // Plain path does on ordinary lines.
                let data_access = format!("{}#data.", data_var);
                let args_rewritten = replace_outside_strings_and_comments(
                    &args_rewritten,
                    TargetLanguage::Erlang,
                    &[("self.", data_access.as_str())],
                );

                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                let args_list = if args_rewritten.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", args_rewritten)
                };
                let result_name = if result_var == "_" {
                    "_".to_string()
                } else {
                    let mut chars = result_var.chars();
                    match chars.next() {
                        None => "_".to_string(),
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                };
                result.push(format!(
                    "    {{{}, {}}} = frame_dispatch__({}, {}, {})",
                    new_var, result_name, method, args_list, data_var
                ));
                data_var = new_var;
            }
            ErlangRewrite::Plain(text) => {
                if text.is_empty() {
                    continue;
                }
                // Erlang string concat is ++ not + — fix when adjacent to string literals
                let text = text
                    .replace("\" + \"", "\" ++ \"")
                    .replace("\" + ", "\" ++ ")
                    .replace(" + \"", " ++ \"");
                result.push(format!("    {}", text));
            }
            ErlangRewrite::Reply(expr) => {
                result.push(format!(
                    "    {{keep_state, {}, [{{reply, From, {}}}]}}",
                    data_var, expr
                ));
            }
        }
    }

    // SSA-rename `__ReturnVal` so repeat writes don't collide with
    // Erlang's single-assignment rule. Each top-level write becomes
    // `__ReturnVal_K` for K = 1, 2, 3… Reads between writes K and
    // K+1 bind to `__ReturnVal_K`. Returns the LAST write's name
    // (or `"ok"` if no writes), which the handler emitter uses in
    // its terminal reply tuple — no hardcoded name at the emit site.
    // Only top-level writes are renamed; case-arm writes have their
    // own per-arm unification.
    let final_rv_name: String;
    {
        fn is_direct_write(t: &str) -> bool {
            t.starts_with("__ReturnVal = ")
        }
        fn is_tuple_bind(t: &str) -> bool {
            t.starts_with('{')
                && t.contains(", __ReturnVal}")
                && t.contains(" = ")
        }
        // Pass 1: collect top-level write indices.
        let mut depth: i32 = 0;
        let mut writes: Vec<usize> = Vec::new();
        for (i, line) in result.iter().enumerate() {
            let t = line.trim();
            let opens = (t.starts_with("case ") || t.starts_with("case("))
                && (t.ends_with(" of") || t.ends_with(" of,"));
            let closes = t == "end" || t == "end," || t == "end;";
            if opens {
                depth += 1;
                continue;
            }
            if closes {
                depth = (depth - 1).max(0);
                continue;
            }
            if depth == 0 && (is_direct_write(t) || is_tuple_bind(t)) {
                writes.push(i);
            }
        }
        // Determine the final return-value name for the caller's
        // terminal tuple: `__ReturnVal_K` where K is the number of
        // writes. If no writes occurred in this body, fall back to
        // `"ok"` — the gen_statem convention for "no reply value".
        final_rv_name = if writes.is_empty() {
            "ok".to_string()
        } else {
            format!("__ReturnVal_{}", writes.len())
        };
        // Pass 2: rename every write to `__ReturnVal_K` and rewrite
        // reads between write K and write K+1 to `__ReturnVal_K`.
        // Reads BEFORE the first write keep `__ReturnVal` (no prior
        // value — the slot's default; shouldn't appear in
        // well-formed source).
        if !writes.is_empty() {
            let total = writes.len();
            let mut depth2: i32 = 0;
            let mut write_idx = 0usize; // how many writes we've passed (0..total)
            for (i, line) in result.iter_mut().enumerate() {
                let t: String = line.trim().to_string();
                let opens = (t.starts_with("case ") || t.starts_with("case("))
                    && (t.ends_with(" of") || t.ends_with(" of,"));
                let closes = t == "end" || t == "end," || t == "end;";
                if opens {
                    depth2 += 1;
                    continue;
                }
                if closes {
                    depth2 = (depth2 - 1).max(0);
                    continue;
                }
                // Is this line one of the tracked writes?
                let is_tracked_write = write_idx < total && writes[write_idx] == i;
                if is_tracked_write {
                    // Rewrite RHS reads of __ReturnVal to the
                    // previous write's name (if any).
                    if write_idx > 0 {
                        let prev_name = format!("__ReturnVal_{}", write_idx);
                        // Direct write: only rewrite after `=`; tuple bind:
                        // hide the LHS __ReturnVal with a sentinel, rewrite,
                        // restore.
                        if is_direct_write(&t) {
                            if let Some(eq_pos) = line.find('=') {
                                let (lhs, rhs) = line.split_at(eq_pos + 1);
                                let rhs_new = replace_outside_strings_and_comments(
                                    rhs,
                                    TargetLanguage::Erlang,
                                    &[("__ReturnVal", prev_name.as_str())],
                                );
                                *line = format!("{}{}", lhs, rhs_new);
                            }
                        } else {
                            let sentinel = "__RVLhsSentinel";
                            let hidden = line.replacen(
                                ", __ReturnVal}",
                                &format!(", {}}}", sentinel),
                                1,
                            );
                            let rewritten = replace_outside_strings_and_comments(
                                &hidden,
                                TargetLanguage::Erlang,
                                &[("__ReturnVal", prev_name.as_str())],
                            );
                            *line = rewritten.replace(
                                &format!(", {}}}", sentinel),
                                ", __ReturnVal}",
                            );
                        }
                    }
                    // Rename the LHS to `__ReturnVal_{write_idx+1}`.
                    // Every write gets a fresh name — including the
                    // last, which is exposed via `final_rv_name` to
                    // the handler emitter. No hardcoded terminal at
                    // the emit site.
                    let new_name = format!("__ReturnVal_{}", write_idx + 1);
                    if is_direct_write(&t) {
                        *line = line.replacen("__ReturnVal", &new_name, 1);
                    } else {
                        *line = line.replacen(
                            ", __ReturnVal}",
                            &format!(", {}}}", new_name),
                            1,
                        );
                    }
                    write_idx += 1;
                } else {
                    // Non-write line — rewrite reads to the most
                    // recent write's name (`__ReturnVal_{write_idx}`).
                    // Before the first write, no rename: the slot
                    // still holds its default.
                    if write_idx > 0 {
                        let name = format!("__ReturnVal_{}", write_idx);
                        *line = replace_outside_strings_and_comments(
                            line,
                            TargetLanguage::Erlang,
                            &[("__ReturnVal", name.as_str())],
                        );
                    }
                }
            }
        }
    }

    (result, data_var, final_rv_name)
}

/// Wrap a state handler body in transition-guard `case` expressions after
/// every `@@:self.method()` call. Preserves semantics: if the called
/// handler transitioned the system to a new state, the rest of the
/// caller's body is suppressed and the handler returns `{next_state,
/// NewState, NewData, [{reply, From, undefined}]}` to gen_statem — same
/// behavior Python/TS/etc. get for free via the mutable `_transitioned`
/// context flag plus early `return`.
///
/// Input: the linear output of `erlang_process_body_lines_full` (each
/// line comma-suffixed, terminal tuple has no separator). Lines containing
/// `= frame_dispatch__(` are the InterfaceCall sites the classifier
/// emitted for @@:self.
///
/// Output: same lines, but each dispatch-call site is followed by a case
/// split on the returned Data's `frame_current_state`. The state's own
/// atom (snake_case) is the "no transition" arm; any other state is the
/// "transitioned" early-return arm.
///
/// Recurses on the tail so nested @@:self calls produce nested cases
/// without needing an explicit counter.
///
/// `state_atom` is the snake_case atom for the handler's enclosing state
/// (e.g., `active`, `logged_out`). `reply_expr` is whatever the handler
/// would return in the transition arm (use the latest `__ReturnVal`
/// binding if one was set before the @@:self, else `undefined`).
fn erlang_wrap_self_call_guards(
    lines: &[String],
    state_atom: &str,
) -> Vec<String> {
    fn is_dispatch_call(line: &str) -> bool {
        line.contains("= frame_dispatch__(")
    }

    // Pull the `DataN` new-binding variable name out of a line like
    //   "    {Data2, _} = frame_dispatch__(method, [args], Data1),"
    fn extract_new_data_var(line: &str) -> Option<&str> {
        let open = line.find('{')?;
        let rest = &line[open + 1..];
        let comma = rest.find(',')?;
        Some(rest[..comma].trim())
    }

    // Recursion: wrap from the first dispatch onward in a case, then
    // recurse on the tail so nested dispatches get nested cases.
    //
    // CRITICAL: if the dispatch sits inside an arm of an enclosing
    // case block (e.g., the `; false ->` arm of an `if/else`), the
    // tail we wrap must NOT extend past the arm's closing delimiter
    // — otherwise we'd suck the outer case's `end` or sibling arm
    // header into the guard case's atom arm, producing malformed
    // nested-case output. Scan for the first boundary (`; false ->`,
    // `; _ ->`, or a standalone `end` / `end,` / `end;` at shallow
    // depth) and cap the tail there.
    for (idx, line) in lines.iter().enumerate() {
        if !is_dispatch_call(line) {
            continue;
        }
        let data_var = match extract_new_data_var(line) {
            Some(v) => v.to_string(),
            None => continue,
        };

        // Find the tail's upper bound. Track case depth from the
        // dispatch onward: opens on `case … of`, closes on `end`.
        // The first arm-separator (`; false` / `; _`) OR the first
        // `end` at depth 0 signals the tail ends.
        let mut depth: i32 = 0;
        let mut tail_end = lines.len();
        for (j, l) in lines.iter().enumerate().skip(idx + 1) {
            let t = l.trim();
            let opens = (t.starts_with("case ") || t.starts_with("case("))
                && (t.ends_with(" of") || t.ends_with(" of,"));
            let closes = t == "end" || t == "end," || t == "end;";
            if opens {
                depth += 1;
                continue;
            }
            if depth == 0 && (t.starts_with("; false") || t.starts_with("; _")) {
                tail_end = j;
                break;
            }
            if closes {
                if depth == 0 {
                    tail_end = j;
                    break;
                }
                depth -= 1;
            }
        }
        let tail_slice = &lines[idx + 1..tail_end];
        let inner_wrapped = erlang_wrap_self_call_guards(tail_slice, state_atom);
        if inner_wrapped.is_empty() {
            // No tail — the dispatch was the last statement. No guard
            // needed because there's nothing to suppress; the outer
            // handler's terminal tuple will emit from the caller.
            continue;
        }

        let mut result: Vec<String> = lines[..=idx].to_vec();
        let ind = "    ";
        result.push(format!(
            "{ind}case {dv}#data.frame_current_state of",
            ind = ind, dv = data_var
        ));
        result.push(format!("{ind}    {atom} ->", ind = ind, atom = state_atom));

        // Arm body sits two indent levels deeper than the outer `    case`
        // (i.e. 12 spaces here, but — and this matters for nested guards —
        // inner lines that ALREADY contain deeper structure must preserve
        // their relative indent. So re-indent by prefixing 8 spaces rather
        // than resetting.
        //
        // Atom-arm terminal: if the inner body does NOT end with a
        // gen_statem-shaped tuple (`{keep_state,…}`/`{next_state,…}` or
        // a `frame_transition__(…)` call), the arm's final expression
        // is something like `__ReturnVal = …` or a bare value. gen_statem
        // requires a tuple. Inject `{keep_state, <final_data>, [{reply,
        // From, __ReturnVal|ok}]}` as the arm's terminator.
        let last_expr_is_terminal = inner_wrapped
            .last()
            .map(|l| {
                let t = l.trim();
                t.starts_with("{keep_state,")
                    || t.starts_with("{next_state,")
                    || t.starts_with("frame_transition__(")
                    || t == "end"
                    || t == "end,"
                    || t == "end;"
            })
            .unwrap_or(false);
        let arm_has_terminal_tuple = inner_wrapped.iter().any(|l| {
            let t = l.trim();
            t.starts_with("{keep_state,")
                || t.starts_with("{next_state,")
                || t.starts_with("frame_transition__(")
        });
        let arm_final_data: String = inner_wrapped
            .iter()
            .rev()
            .find_map(|l| {
                let t = l.trim();
                if t.starts_with("Data") {
                    let rest = &t[4..];
                    let n: usize = rest
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .count();
                    if n > 0 && rest[n..].trim_start().starts_with('=') {
                        return Some(t[..4 + n].to_string());
                    }
                }
                None
            })
            .unwrap_or_else(|| data_var.clone());
        let arm_reply = if inner_wrapped
            .iter()
            .any(|l| l.trim().starts_with("__ReturnVal = "))
        {
            "__ReturnVal".to_string()
        } else {
            "ok".to_string()
        };
        let inject_terminal = !last_expr_is_terminal && !arm_has_terminal_tuple;

        let last = inner_wrapped.len().saturating_sub(1);
        for (i, l) in inner_wrapped.iter().enumerate() {
            // Prepend 8 spaces to whatever relative indent the line has.
            let re_indent = format!("        {}", l);
            if i == last && !inject_terminal {
                // Arms are separated by `;`. The last statement of the
                // `atom ->` arm needs a trailing `;` (not `,`); strip any
                // pre-existing comma first.
                let trimmed = re_indent
                    .trim_end_matches(|c: char| c == ',' || c.is_whitespace())
                    .to_string();
                result.push(format!("{};", trimmed));
            } else {
                result.push(re_indent);
            }
        }
        if inject_terminal {
            result.push(format!(
                "        {{keep_state, {}, [{{reply, From, {}}}]}};",
                arm_final_data, arm_reply
            ));
        }

        result.push(format!("{ind}    _ ->", ind = ind));
        result.push(format!(
            "{ind}        {{next_state, {dv}#data.frame_current_state, {dv}, [{{reply, From, undefined}}]}}",
            ind = ind, dv = data_var
        ));
        result.push(format!("{ind}end", ind = ind));
        // Anything past tail_end (the enclosing case's arm boundary or
        // `end`) belongs to the outer structure, not the guard. Append
        // it verbatim so the outer case's closing delimiter and sibling
        // arms are preserved.
        for l in &lines[tail_end..] {
            result.push(l.clone());
        }
        return result;
    }
    lines.to_vec()
}

/// Simple rewrite for contexts where Data threading isn't needed (expressions only)
fn erlang_rewrite_expr(line: &str, action_names: &[String]) -> String {
    let l = line.trim();
    for action in action_names {
        let pattern = format!("self.{}(", action);
        if l.contains(&pattern) {
            let replaced = l.replace(&pattern, &format!("{}(Data, ", action));
            return replaced.replace("(Data, )", "(Data)");
        }
    }
    replace_outside_strings_and_comments(
        l,
        TargetLanguage::Erlang,
        &[("self.", "Data#data.")],
    )
}

/// Transform C-family `if/else { }` block syntax to Erlang `case/of/end`.
///
/// Join processed Erlang lines with proper comma/newline separators.
/// In Erlang, all expressions in a function clause are comma-separated except:
/// - Inside case blocks: branches are separated by `;`, values by comma only within a branch
/// - After `case ... of`, `true ->`, `; false ->` (structural, no comma)
/// - Before `end`, `; false`, `true ->` (structural, no comma)
/// - Lines already ending with `,` or `;` get a newline only
fn erlang_smart_join(lines: &[String], code: &mut String) {
    let mut case_depth = 0i32;

    // Filter out comment-only lines — they contribute nothing to Erlang syntax
    // and break comma/semicolon placement logic when between code lines.
    let non_comment_lines: Vec<&String> = lines
        .iter()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with('%') || t.is_empty()
        })
        .collect();

    for (idx, line) in non_comment_lines.iter().enumerate() {
        if idx > 0 {
            let lt = line.trim();
            let pt_full = non_comment_lines[idx - 1].trim();
            // Strip trailing % comment to get the code portion for punctuation checks
            let pt = {
                let mut in_string = false;
                let mut escape = false;
                let mut code_end = pt_full.len();
                for (i, c) in pt_full.char_indices() {
                    if escape {
                        escape = false;
                        continue;
                    }
                    if c == '\\' {
                        escape = true;
                        continue;
                    }
                    if c == '"' {
                        in_string = !in_string;
                        continue;
                    }
                    if c == '%' && !in_string {
                        code_end = i;
                        break;
                    }
                }
                pt_full[..code_end].trim_end()
            };

            // Track case depth BEFORE deciding separator
            if lt.starts_with("case ") || lt.starts_with("case(") {
                // case_depth will be incremented below
            }

            // Previous line already ends with comma or semicolon — just newline
            let prev_ends_punctuated = pt.ends_with(',') || pt.ends_with(';');

            // No comma after structural introducers
            let prev_is_case_head = pt.ends_with(" of");
            let prev_is_branch =
                pt.ends_with("->") || pt.starts_with("; false") || pt.starts_with("; true");

            // No comma before structural closers or branch starts
            let curr_is_end =
                lt == "end" || lt == "end," || lt.starts_with("end;") || lt.starts_with("end.");
            let curr_is_branch =
                lt.starts_with("true ->") || lt.starts_with("; false") || lt.starts_with("; true");

            // Inside case blocks: suppress commas only at structural boundaries
            // (between branches, after case/of, before end). Expressions within
            // a branch still need commas between them.
            let prev_is_structural_case = prev_is_case_head || prev_is_branch;
            let curr_is_structural_case = curr_is_end || curr_is_branch;

            if prev_ends_punctuated || prev_is_structural_case || curr_is_structural_case {
                code.push('\n');
            } else {
                code.push_str(",\n");
            }
        }

        let lt = line.trim();
        if (lt.starts_with("case ") || lt.contains(" case ") || lt.starts_with("case("))
            && lt.ends_with(" of")
        {
            case_depth += 1;
        }
        if lt == "end" || lt == "end," || lt.starts_with("end;") || lt.starts_with("end.") {
            case_depth = (case_depth - 1).max(0);
        }

        code.push_str(line);
    }
}

/// Runs on the spliced handler body text AFTER Frame statements have been expanded.
/// Only converts `{` that follows `if`/`else if`/`else` keywords.
/// Leaves other `{` alone (maps, tuples, records, gen_statem return tuples).
fn erlang_transform_blocks(text: &str) -> String {
    let mut result = String::new();
    // Track block contexts: ("if", has_else), ("elif", _)
    let mut block_depth: Vec<(&str, bool)> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        let indent = &line[..line.len() - trimmed.len()];

        // Skip empty lines
        if trimmed.is_empty() {
            result.push('\n');
            continue;
        }

        // `if condition {` → `case (condition) of true ->`
        if trimmed.starts_with("if ") && trimmed.ends_with('{') {
            let condition = trimmed[3..trimmed.len() - 1].trim();
            result.push_str(&format!(
                "{}case ({}) of\n{}    true ->",
                indent, condition, indent
            ));
            block_depth.push(("if", false));
            result.push('\n');
            continue;
        }

        // `} else if condition {` → `; false -> case (condition) of true ->`
        if (trimmed.starts_with("} else if ") || trimmed.starts_with("}else if "))
            && trimmed.ends_with('{')
        {
            let rest = if trimmed.starts_with("} else if ") {
                &trimmed[10..trimmed.len() - 1]
            } else {
                &trimmed[9..trimmed.len() - 1]
            };
            let condition = rest.trim();
            // Pop current if, push new nested case
            if !block_depth.is_empty() {
                block_depth.pop();
            }
            result.push_str(&format!(
                "{}    ; false ->\n{}        case ({}) of\n{}            true ->",
                indent, indent, condition, indent
            ));
            block_depth.push(("elif", false));
            block_depth.push(("if", false));
            result.push('\n');
            continue;
        }

        // `} else {` → `; false ->`
        if trimmed == "} else {" || trimmed == "}else{" || trimmed == "} else{" {
            // Mark the current if-block as having an else clause
            if let Some(last) = block_depth.last_mut() {
                last.1 = true;
            }
            result.push_str(&format!("{}    ; false ->", indent));
            result.push('\n');
            continue;
        }

        // `}` that closes an if block → `end`
        if trimmed == "}" && !block_depth.is_empty() {
            let (ctx, has_else) = match block_depth.pop() {
                Some(v) => v,
                None => continue,
            };
            // If no else clause, add a default false branch
            if !has_else && ctx == "if" {
                result.push_str(&format!("{}    ; false -> ok\n", indent));
            }
            // If the last branch was empty (e.g., empty else {}), add ok
            let result_trimmed = result.trim_end();
            if result_trimmed.ends_with("->") {
                result.push_str("    ok\n");
            }
            result.push_str(&format!("{}end", indent));
            // If this was an elif, we also need to close the outer case.
            // And for an `if ... else if ... else {}` chain, the terminal
            // `}` pops the innermost "if" with ctx="if" (has_else=true)
            // but the enclosing "elif" wrappers are still on the stack —
            // drain each one, emitting an `end` per wrapper so the nested
            // case structure closes fully.
            if ctx == "elif" {
                if !block_depth.is_empty() {
                    block_depth.pop();
                    result.push_str(&format!("\n{}end", indent));
                }
            } else if ctx == "if" && has_else {
                // Drain any "elif" entries stacked underneath this if —
                // each represents an outer case that opened at the
                // matching `} else if` and needs its own closing `end`.
                while let Some(&(outer_ctx, _)) = block_depth.last() {
                    if outer_ctx != "elif" {
                        break;
                    }
                    block_depth.pop();
                    // An "elif" entry is always preceded by an "if" that
                    // tracks the inner case block. It's already been closed
                    // by this time, but drain defensively in case the shape
                    // diverges (nested mixed patterns).
                    if let Some(&(c, _)) = block_depth.last() {
                        if c == "if" {
                            block_depth.pop();
                        }
                    }
                    result.push_str(&format!("\n{}end", indent));
                }
            }
            result.push('\n');
            continue;
        }

        // Everything else passes through
        result.push_str(line);
        result.push('\n');
    }

    // Second pass: nest sequential if-without-else blocks (early-exit pattern)
    //
    // Converts linear early-exit chains into right-nested case blocks:
    //   case (A) of true -> X; false -> ok end
    //   case (B) of true -> Y; false -> ok end
    //   Z
    // Becomes:
    //   case (A) of true -> X; false ->
    //     case (B) of true -> Y; false ->
    //       Z
    //     end
    //   end
    let result_lines: Vec<&str> = result.lines().collect();
    let pass2 = erlang_nest_early_exits(&result_lines);

    // Third pass: add commas after `end` when followed by another expression
    let mut final_result = String::new();
    let pass2_lines: Vec<&str> = pass2.lines().collect();
    for (i, line) in pass2_lines.iter().enumerate() {
        final_result.push_str(line);
        if line.trim() == "end" && i + 1 < pass2_lines.len() {
            let next = pass2_lines[i + 1..].iter().find(|l| !l.trim().is_empty());
            if let Some(next_line) = next {
                let nt = next_line.trim();
                if !nt.starts_with("end") && !nt.starts_with(";") && !nt.is_empty() {
                    final_result.push(',');
                }
            }
        }
        final_result.push('\n');
    }

    final_result
}

/// Nest sequential if-without-else blocks into right-nested case expressions.
/// This converts the early-exit pattern (common in Frame handlers) into valid
/// Erlang where each function clause returns exactly one value.
fn erlang_nest_early_exits(lines: &[&str]) -> String {
    // Find blocks: sequences of (case...of true->X; false->ok end) followed by trailing expr
    // Strategy: work backwards from the end, folding each "; false -> ok\nend" into
    // "; false ->\n  <rest of handler>"

    let mut output_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();

    // Iterate from end to start, looking for "; false -> ok" patterns
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < output_lines.len() {
            let is_false_ok = output_lines[i].trim() == "; false -> ok";

            if is_false_ok {
                // Find the matching "end" line after this
                let mut j = i + 1;
                while j < output_lines.len() && output_lines[j].trim().is_empty() {
                    j += 1;
                }

                let is_end = j < output_lines.len() && {
                    let t = output_lines[j].trim().to_string();
                    t == "end" || t == "end,"
                };
                if is_end {
                    // Collect everything after "end" until end of lines
                    let remaining_start = j + 1;
                    let mut remaining: Vec<String> = Vec::new();
                    for k in remaining_start..output_lines.len() {
                        if !output_lines[k].trim().is_empty() {
                            remaining.push(output_lines[k].clone());
                        }
                    }

                    // Only nest if remaining code has real expressions (not just structural case lines)
                    let has_real_code = remaining.iter().any(|r| {
                        let t = r.trim();
                        !t.is_empty()
                            && t != "end"
                            && t != "end,"
                            && !t.starts_with("; false -> ok")
                            && !t.starts_with("; false ->")
                    });
                    if has_real_code {
                        let indent_len = output_lines[i].len() - output_lines[i].trim_start().len();
                        let indent = " ".repeat(indent_len);
                        // Replace "; false -> ok" with "; false ->"
                        output_lines[i] = format!("{}; false ->", indent);
                        // Replace "end" with the remaining lines + "end"
                        let mut new_section: Vec<String> = Vec::new();
                        for r in &remaining {
                            new_section.push(format!("{}    {}", indent, r.trim()));
                        }
                        new_section.push(format!("{}end", indent));

                        // Remove old end + remaining lines
                        output_lines.drain(j..);
                        // Insert new section after the "; false ->" line
                        let insert_pos = i + 1;
                        for (idx, new_line) in new_section.into_iter().enumerate() {
                            output_lines.insert(insert_pos + idx, new_line);
                        }

                        changed = true;
                        break; // Restart from the beginning
                    }
                }
            }
            i += 1;
        }
    }

    output_lines.join("\n")
}

/// Expand @@SystemName() in Erlang domain initializers
fn expand_tagged_in_domain_erlang(text: &str) -> String {
    // Simple pattern: @@Name(args) → name:start_link(args)
    let mut result = text.to_string();
    while let Some(pos) = result.find("@@") {
        let after = pos + 2;
        if after < result.len() && result.as_bytes()[after].is_ascii_uppercase() {
            let name_end = result[after..]
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .map(|p| after + p)
                .unwrap_or(result.len());
            let name = &result[after..name_end];
            let snake = to_snake_case(name);
            if name_end < result.len() && result.as_bytes()[name_end] == b'(' {
                result = format!(
                    "{}{}:start_link({}",
                    &result[..pos],
                    snake,
                    &result[name_end + 1..]
                );
            } else {
                result = format!("{}{}{}", &result[..pos], snake, &result[name_end..]);
            }
        } else {
            break;
        }
    }
    result
}

// ============================================================================
// Case Arm Analysis — structural classification for mixed conditional handlers
// ============================================================================

/// Information about a single arm in a case...end block
struct CaseArmInfo {
    /// Index of the arm header line (e.g., "true ->" or "; false ->") in processed lines
    header_idx: usize,
    /// Line indices of body content (after header, before next arm or end)
    body_start: usize,
    body_end: usize,
    /// Whether this arm contains a frame_transition__() call
    has_transition: bool,
    /// The __ReturnVal expression if one was assigned in this arm
    return_val: Option<String>,
    /// The last DataN variable in this arm (for {keep_state, DataN, ...})
    final_data_var: Option<String>,
}

/// Classification of a case block's arm behaviors
enum CaseBlockClassification {
    /// All arms have frame_transition__() — case is terminal, use as handler return
    AllTerminal,
    /// No arms have frame_transition__() — hoist __ReturnVal, append {keep_state,...}
    NoTerminal,
    /// Mixed: some arms transition, some don't — per-arm rewrite needed
    Mixed,
}

/// Analyze a case block in processed handler lines.
/// Returns (classification, arms, case_start_line, case_end_line).
/// Only analyzes the first top-level case block found.
fn analyze_case_arms(
    processed: &[String],
) -> Option<(CaseBlockClassification, Vec<CaseArmInfo>, usize, usize)> {
    let mut case_start = None;
    let mut case_end = None;
    let mut depth = 0i32;
    let mut arms: Vec<CaseArmInfo> = Vec::new();
    let mut current_arm: Option<CaseArmInfo> = None;

    for (idx, line) in processed.iter().enumerate() {
        let t = line.trim();

        // Track case block depth
        if (t.starts_with("case ") || t.starts_with("case(")) && t.ends_with(" of") {
            depth += 1;
            if depth == 1 && case_start.is_none() {
                case_start = Some(idx);
            }
            continue;
        }

        if t == "end" || t == "end," || t == "end;" {
            if depth == 1 {
                // Close current arm
                if let Some(mut arm) = current_arm.take() {
                    arm.body_end = idx;
                    arms.push(arm);
                }
                case_end = Some(idx);
            }
            depth = (depth - 1).max(0);
            continue;
        }

        // Only analyze top-level arms (depth == 1)
        if depth != 1 {
            // Still track content for current arm at nested depths
            if let Some(ref mut arm) = current_arm {
                if t.starts_with("frame_transition__(") {
                    arm.has_transition = true;
                }
                if t.starts_with("__ReturnVal = ") {
                    let val = t
                        .trim_start_matches("__ReturnVal = ")
                        .trim_end_matches(',')
                        .to_string();
                    arm.return_val = Some(val);
                }
                // Track DataN variable assignments
                if t.starts_with("Data") && t.contains(" = ") && !t.contains("#data") {
                    if let Some(eq_pos) = t.find(" = ") {
                        let var = t[..eq_pos].trim().to_string();
                        if var.starts_with("Data") && var[4..].chars().all(|c| c.is_ascii_digit()) {
                            arm.final_data_var = Some(var);
                        }
                    }
                }
            }
            continue;
        }

        // Arm boundary detection at depth 1
        let is_arm_header =
            t.starts_with("true ->") || t.starts_with("; false") || t.starts_with("; _");
        if is_arm_header {
            // Close previous arm
            if let Some(mut arm) = current_arm.take() {
                arm.body_end = idx;
                arms.push(arm);
            }
            // Start new arm
            current_arm = Some(CaseArmInfo {
                header_idx: idx,
                body_start: idx + 1,
                body_end: idx + 1, // updated when arm closes
                has_transition: false,
                return_val: None,
                final_data_var: None,
            });
            continue;
        }

        // Content within current arm
        if let Some(ref mut arm) = current_arm {
            if t.starts_with("frame_transition__(") {
                arm.has_transition = true;
            }
            if t.starts_with("__ReturnVal = ") {
                let val = t
                    .trim_start_matches("__ReturnVal = ")
                    .trim_end_matches(',')
                    .to_string();
                arm.return_val = Some(val);
            }
            if t.starts_with("Data") && t.contains(" = ") && !t.contains("#data") {
                if let Some(eq_pos) = t.find(" = ") {
                    let var = t[..eq_pos].trim().to_string();
                    if var.starts_with("Data") && var[4..].chars().all(|c| c.is_ascii_digit()) {
                        arm.final_data_var = Some(var);
                    }
                }
            }
        }
    }

    let case_start = case_start?;
    let case_end = case_end?;
    if arms.is_empty() {
        return None;
    }

    // Classify
    let all_terminal = arms.iter().all(|a| a.has_transition);
    let none_terminal = arms.iter().all(|a| !a.has_transition);
    let classification = if all_terminal {
        CaseBlockClassification::AllTerminal
    } else if none_terminal {
        CaseBlockClassification::NoTerminal
    } else {
        CaseBlockClassification::Mixed
    };

    Some((classification, arms, case_start, case_end))
}

/// Rewrite a case block with mixed arms so each arm produces a gen_statem return tuple.
fn rewrite_mixed_case_arms(
    processed: &[String],
    arms: &[CaseArmInfo],
    case_start: usize,
    case_end: usize,
    default_data: &str,
) -> Vec<String> {
    let mut result = Vec::new();

    // Emit lines before case block
    for i in 0..case_start {
        result.push(processed[i].clone());
    }

    // Emit case header
    result.push(processed[case_start].clone());

    // Emit each arm
    for arm in arms {
        // Emit arm header — strip any inline content after "->"
        // (e.g., "; false -> ok" becomes "; false ->")
        let header = &processed[arm.header_idx];
        let clean_header = if let Some(arrow_pos) = header.find("->") {
            let after_arrow = &header[arrow_pos + 2..].trim();
            if after_arrow.is_empty() {
                header.clone()
            } else {
                header[..arrow_pos + 2].to_string()
            }
        } else {
            header.clone()
        };
        result.push(clean_header);

        // Emit arm body lines, filtering as needed. Track nested case
        // depth so we only strip `__ReturnVal = ...` at depth 0 (this arm's
        // top level). `__ReturnVal` inside nested cases belongs to the inner
        // case's own arm — the depth-≥2 injector already turned those into
        // complete reply tuples and stripping them here would leak an
        // unbound `__ReturnVal` reference.
        let mut nested_depth = 0i32;
        for i in arm.body_start..arm.body_end {
            let t = processed[i].trim();

            let opens = (t.starts_with("case ") || t.starts_with("case("))
                && (t.ends_with(" of") || t.ends_with(" of,"));
            let closes = t == "end" || t == "end," || t == "end;";

            if opens {
                result.push(processed[i].clone());
                nested_depth += 1;
                continue;
            }

            if t.starts_with("__ReturnVal = ") && nested_depth == 0 {
                // Top-level of this arm: drop. Captured via arm.return_val
                // and re-emitted in the injected reply tuple below.
                continue;
            }

            result.push(processed[i].clone());

            if closes {
                nested_depth = (nested_depth - 1).max(0);
            }
        }

        // For non-transition arms, inject the gen_statem return tuple.
        // Skip if the arm body already contains a reply tuple (the depth-≥2
        // injector may have planted one at a nested leaf that's the only
        // exit of this arm).
        if !arm.has_transition {
            let arm_has_reply = processed[arm.body_start..arm.body_end]
                .iter()
                .any(|l| {
                    let t = l.trim();
                    t.starts_with("{keep_state,") || t.starts_with("{next_state,")
                });
            if !arm_has_reply {
                let data = arm.final_data_var.as_deref().unwrap_or(default_data);
                let reply = arm.return_val.as_deref().unwrap_or("ok");
                result.push(format!(
                    "        {{keep_state, {}, [{{reply, From, {}}}]}}",
                    data, reply
                ));
            }
        }
    }

    // Emit end
    result.push("    end".to_string());

    result
}

/// Post-process emitted handler lines to inject gen_statem reply tuples at
/// orphan `__ReturnVal = "..."` leaves in **nested** case blocks (depth > 1).
///
/// `rewrite_mixed_case_arms` already handles the outermost case — for each
/// top-level arm that doesn't transition, it injects a reply tuple. But it
/// only descends one level deep. When a handler uses nested `if/else`
/// (producing nested case blocks), the inner else branches that just set
/// `__ReturnVal` without transitioning escape the rewriter and leak bare
/// values into the gen_statem return, crashing with
/// `bad_return_from_state_function`.
///
/// This pass handles the inner cases: finds every `__ReturnVal = <expr>`
/// that sits at case nesting depth ≥ 2 AND is the final statement of its
/// arm (followed by `end`, `; false ->`, or `; _ ->` with no transition or
/// reply tuple in between), and rewrites it to:
///     __ReturnVal = <expr>,
///     {keep_state, <Data>, [{reply, From, __ReturnVal}]}
///
/// Depth-1 orphans are left alone because `rewrite_mixed_case_arms` handles
/// them via its top-level arm-boundary injection.
fn erlang_inject_orphan_reply_tuples(lines: &[String], default_data: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::with_capacity(lines.len());
    let mut depth: i32 = 0;

    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();

        // Track case nesting depth.
        let opens_case = (t.starts_with("case ") || t.starts_with("case("))
            && (t.ends_with(" of") || t.ends_with(" of,"));

        // `end` / `end,` / `end;` closes the innermost case. We bump depth
        // down AFTER emitting so the `end` line is attributed to its case.
        let closes_case = t == "end" || t == "end," || t == "end;";

        if opens_case {
            depth += 1;
        }

        let is_orphan_candidate = t.starts_with("__ReturnVal = ")
            && !t.ends_with(',')
            && !t.ends_with(';');

        if !is_orphan_candidate || depth < 2 {
            result.push(line.clone());
            if closes_case {
                depth = (depth - 1).max(0);
            }
            continue;
        }

        // Look ahead: next non-blank line.
        let mut j = i + 1;
        while j < lines.len() && lines[j].trim().is_empty() {
            j += 1;
        }
        let next_trimmed = lines.get(j).map(|s| s.trim()).unwrap_or("");
        let arm_closes = next_trimmed == "end"
            || next_trimmed == "end,"
            || next_trimmed == "end;"
            || next_trimmed.starts_with("; false")
            || next_trimmed.starts_with("; _");

        let already_has_reply = next_trimmed.starts_with("{keep_state,")
            || next_trimmed.starts_with("{next_state,")
            || next_trimmed.starts_with("frame_transition__(");

        if arm_closes && !already_has_reply {
            let lead_len = line.len() - line.trim_start().len();
            let indent = &line[..lead_len];
            result.push(format!("{}{},", indent, t));
            result.push(format!(
                "{}{{keep_state, {}, [{{reply, From, __ReturnVal}}]}}",
                indent, default_data
            ));
        } else {
            result.push(line.clone());
        }
    }

    result
}

// ============================================================================

pub(crate) fn generate_erlang_system(
    system: &SystemAst,
    _arcanum: &Arcanum,
    source: &[u8],
) -> CodegenNode {
    let sys = &system.name;
    let module_name = to_snake_case(sys);
    let mut code = String::new();

    // Collect action + operation names for native code rewriting
    // (both are module-level functions that get self.X() → X(Data) rewriting)
    let mut action_names: Vec<String> = system.actions.iter().map(|a| a.name.clone()).collect();
    action_names.extend(system.operations.iter().map(|o| o.name.clone()));

    // Collect interface method names for internal dispatch
    // (self.method() → frame_dispatch__(method, [args], Data))
    let interface_names: Vec<String> = system.interface.iter().map(|m| m.name.clone()).collect();

    // Module header
    code.push_str(&format!("-module({}).\n", module_name));
    code.push_str("-behaviour(gen_statem).\n\n");

    // Collect state names
    let states: Vec<&str> = system
        .machine
        .as_ref()
        .map(|m| m.states.iter().map(|s| s.name.as_str()).collect())
        .unwrap_or_default();

    let first_state = states
        .first()
        .map(|s| to_snake_case(s))
        .unwrap_or_else(|| "init_state".to_string());

    // State name conversion: $MyState -> my_state
    let state_atom = |name: &str| -> String { to_snake_case(name) };

    // System params (header parameters): used to thread constructor
    // arguments through start_link/N → init/1 → the #data{} record
    // literal so domain fields can reference parameters by name and
    // state params land in frame_state_args.
    let sys_params = &system.params;
    let sys_param_arity = sys_params.len();
    let sys_param_vars: Vec<String> = sys_params
        .iter()
        .map(|p| erlang_safe_capitalize(&p.name))
        .collect();

    // Exports — API functions
    let mut api_exports = Vec::new();
    api_exports.push(format!("start_link/{}", sys_param_arity));
    for method in &system.interface {
        let arity = method.params.len() + 1; // +1 for Pid
        api_exports.push(format!("{}/{}", to_snake_case(&method.name), arity));
    }
    // Non-static operations are callable externally (Pid-based) AND
    // internally (Data-based). Each non-static op emits a two-clause
    // function guarded by `is_pid/1`: if the first arg is a Pid the
    // external clause dispatches through gen_statem; otherwise the
    // internal clause runs the body. Same arity in both clauses, so
    // one export covers both. See the op-function emitter below.
    for op in &system.operations {
        if op.is_static {
            continue;
        }
        let arity = op.params.len() + 1; // +1 for Pid (external) / Data (internal)
        api_exports.push(format!("{}/{}", erlang_op_name(&op.name), arity));
    }
    code.push_str(&format!("-export([{}]).\n", api_exports.join(", ")));

    // Exports — gen_statem callbacks
    code.push_str("-export([callback_mode/0, init/1]).\n");

    // Exports — state functions
    let state_exports: Vec<String> = states
        .iter()
        .map(|s| format!("{}/3", state_atom(s)))
        .collect();
    if !state_exports.is_empty() {
        code.push_str(&format!("-export([{}]).\n", state_exports.join(", ")));
    }

    // Record for domain variables + state variables
    code.push_str("\n-record(data, {\n");
    let mut all_fields: Vec<String> = Vec::new();

    // Helper: does this raw domain initializer reference any system param?
    // If so, the record default must be neutral (`undefined`) and the real
    // value is bound in init/N — record defaults can't see init/N's variables.
    let raw_references_param = |raw: &str| -> bool {
        for p in sys_params {
            if raw_contains_word(raw, &p.name) {
                return true;
            }
        }
        false
    };

    // Domain vars — emit Erlang record fields from the structured
    // (name, var_type, initializer_text) slots populated by the new
    // domain_native parser. Erlang ignores the var_type entirely
    // (record fields are dynamically typed in Erlang). The initializer
    // text becomes the record field default — except when it references
    // a system param, in which case we emit `undefined` and let init/N
    // populate the real value via the record literal (Erlang record
    // defaults are evaluated at compile time and can't see init/N's
    // variables).
    for var in &system.domain {
        let init_for_record = match &var.initializer_text {
            Some(init) if raw_references_param(init) => "undefined".to_string(),
            Some(init) => expand_tagged_in_domain_erlang(init),
            None => "undefined".to_string(),
        };
        all_fields.push(format!("    {} = {}", var.name, init_for_record));
    }

    // State variables — prefixed with sv_StateName_ to avoid collisions
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            let state_prefix = to_snake_case(&state.name);
            for sv in &state.state_vars {
                let field_name = format!("sv_{}_{}", state_prefix, sv.name);
                let init_val = if let Some(ref init) = sv.init {
                    expression_to_string(init, TargetLanguage::Erlang)
                } else {
                    "undefined".to_string()
                };
                all_fields.push(format!("    {} = {}", field_name, init_val));
            }
        }
    }

    // Frame infrastructure (Path D hybrid)
    all_fields.push("    frame_stack = []".to_string());
    all_fields.push(format!("    frame_current_state = {}", first_state));
    all_fields.push("    frame_enter_args = #{}".to_string());
    all_fields.push("    frame_exit_args = #{}".to_string());
    all_fields.push("    frame_state_args = #{}".to_string());
    all_fields.push("    frame_context_stack = []".to_string());
    all_fields.push("    frame_return_val = undefined".to_string());

    code.push_str(&all_fields.join(",\n"));
    code.push('\n');
    code.push_str("}).\n\n");

    // start_link/N — system params become positional args, threaded
    // through to init/1 as a list.
    let start_link_args = sys_param_vars.join(", ");
    let start_link_list = if sys_param_vars.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", sys_param_vars.join(", "))
    };
    code.push_str(&format!(
        "start_link({}) ->\n    gen_statem:start_link(?MODULE, {}, []).\n\n",
        start_link_args, start_link_list
    ));

    // Interface functions — public API
    for method in &system.interface {
        let params: Vec<String> = method
            .params
            .iter()
            .map(|p| erlang_safe_capitalize(&p.name))
            .collect();
        let all_params = {
            let mut p = vec!["Pid".to_string()];
            p.extend(params.clone());
            p
        };
        let method_snake = to_snake_case(&method.name);
        let call_args = if params.is_empty() {
            method_snake.clone()
        } else {
            format!("{{{}, {}}}", method_snake, params.join(", "))
        };
        code.push_str(&format!(
            "{}({}) ->\n    gen_statem:call(Pid, {}).\n\n",
            method_snake,
            all_params.join(", "),
            call_args
        ));
    }

    // callback_mode/0
    code.push_str("callback_mode() -> [state_functions, state_enter].\n\n");

    // init/1 — receive system params via the list passed to gen_statem,
    // bind them as Erlang variables, then build the #data{} record literal
    // overriding fields that reference params and populating frame_state_args
    // for any $(...) state params declared in the system header.
    let init_pattern = if sys_param_vars.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", sys_param_vars.join(", "))
    };
    let mut record_overrides: Vec<String> = Vec::new();
    // Domain field overrides for fields whose initializer references a
    // header param.
    for var in &system.domain {
        if let Some(init_expr) = &var.initializer_text {
            if raw_references_param(init_expr) {
                // Substitute bare param identifiers with their
                // capitalized Erlang variable names, then emit the
                // record override.
                let mut substituted = init_expr.clone();
                for p in sys_params {
                    let cap = erlang_safe_capitalize(&p.name);
                    substituted = replace_word(&substituted, &p.name, &cap);
                }
                record_overrides.push(format!("{} = {}", var.name, substituted));
            }
        }
    }
    // State-param overrides go into frame_state_args as a binary-keyed map,
    // and enter-param overrides go into frame_enter_args the same way.
    use crate::frame_c::compiler::frame_ast::ParamKind;
    let state_param_entries: Vec<String> = sys_params
        .iter()
        .filter(|p| matches!(p.kind, ParamKind::StateArg))
        .map(|p| {
            let cap = erlang_safe_capitalize(&p.name);
            format!("<<\"{}\">> => {}", p.name, cap)
        })
        .collect();
    if !state_param_entries.is_empty() {
        record_overrides.push(format!(
            "frame_state_args = #{{{}}}",
            state_param_entries.join(", ")
        ));
    }
    let enter_param_entries: Vec<String> = sys_params
        .iter()
        .filter(|p| matches!(p.kind, ParamKind::EnterArg))
        .map(|p| {
            let cap = erlang_safe_capitalize(&p.name);
            format!("<<\"{}\">> => {}", p.name, cap)
        })
        .collect();
    if !enter_param_entries.is_empty() {
        record_overrides.push(format!(
            "frame_enter_args = #{{{}}}",
            enter_param_entries.join(", ")
        ));
    }
    let record_literal = if record_overrides.is_empty() {
        "#data{}".to_string()
    } else {
        format!("#data{{{}}}", record_overrides.join(", "))
    };
    code.push_str(&format!(
        "init({}) ->\n    {{ok, {}, {}}}.\n\n",
        init_pattern, first_state, record_literal
    ));

    // State functions — one per state
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            let state_name = state_atom(&state.name);

            // Enter handler
            code.push_str(&format!("{}(enter, _OldState, Data) ->\n", state_name));
            if let Some(ref enter) = state.enter {
                // Extract enter params from frame_enter_args
                for (i, p) in enter.params.iter().enumerate() {
                    let var_name = erlang_safe_capitalize(&p.name);
                    code.push_str(&format!(
                        "    {} = maps:get(<<\"{}\">>, Data#data.frame_enter_args, undefined),\n",
                        var_name, i
                    ));
                }
                // Use splicer for proper $.var expansion
                let enter_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: "$>".to_string(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    current_return_type: None,
                };
                let enter_span = crate::frame_c::compiler::ast::Span {
                    start: enter.body.span.start,
                    end: enter.body.span.end,
                };
                let raw_enter = emit_handler_body_via_statements(
                    &enter_span,
                    source,
                    TargetLanguage::Erlang,
                    &enter_ctx,
                );
                let enter_body = erlang_transform_blocks(&raw_enter);

                if !enter_body.trim().is_empty() {
                    let enter_params: Vec<(&str, String)> = enter
                        .params
                        .iter()
                        .map(|p| {
                            let cap = erlang_safe_capitalize(&p.name);
                            (p.name.as_str(), cap)
                        })
                        .collect();
                    let lines: Vec<&str> = enter_body.lines().collect();
                    let (processed, final_data, _final_rv) = erlang_process_body_lines_with_params(
                        &lines,
                        &action_names,
                        "Data",
                        &enter_params,
                    );
                    if !processed.is_empty() {
                        // Check if enter handler contains a transition
                        let has_enter_transition = processed
                            .iter()
                            .any(|l| l.trim().starts_with("frame_transition__("));
                        if has_enter_transition {
                            // Enter handlers can't return {next_state,...} in gen_statem state_enter mode,
                            // and {next_event,...} actions are forbidden from a state enter call.
                            // Defer the transition via a zero-delay state_timeout, which IS allowed
                            // from enter callbacks and is dispatched as a normal event afterward:
                            //   {keep_state, Data, [{state_timeout, 0, {frame_enter_transition, Target}}]}
                            let mut enter_lines = Vec::new();
                            for line in &processed {
                                let t = line.trim();
                                if t.starts_with("frame_transition__(") {
                                    let inner = t
                                        .trim_start_matches("frame_transition__(")
                                        .trim_end_matches(')');
                                    let parts: Vec<&str> =
                                        inner.split(',').map(|s| s.trim()).collect();
                                    if !parts.is_empty() {
                                        let target = parts[0];
                                        enter_lines.push(format!(
                                            "    {{keep_state, {}, [{{state_timeout, 0, {{frame_enter_transition, {}}}}}]}}",
                                            final_data, target
                                        ));
                                    }
                                } else {
                                    enter_lines.push(line.clone());
                                }
                            }
                            erlang_smart_join(&enter_lines, &mut code);
                        } else {
                            erlang_smart_join(&processed, &mut code);
                            code.push_str(",\n");
                            code.push_str(&format!("    {{keep_state, {}}}", final_data));
                        }
                    } else {
                        code.push_str(&format!("    {{keep_state, {}}}", final_data));
                    }
                    code.push_str(";\n");
                } else {
                    code.push_str("    {keep_state, Data};\n");
                }
            } else if !state.state_vars.is_empty() {
                // No explicit enter handler, but state has state vars — auto-init them
                let state_prefix = to_snake_case(&state.name);
                let mut data_var = "Data".to_string();
                let mut gen = 0;
                for sv in &state.state_vars {
                    let field_name = format!("sv_{}_{}", state_prefix, sv.name);
                    let init_val = if let Some(ref init) = sv.init {
                        expression_to_string(init, TargetLanguage::Erlang)
                    } else {
                        "undefined".to_string()
                    };
                    gen += 1;
                    let new_var = format!("Data{}", gen);
                    code.push_str(&format!(
                        "    {} = {}#data{{{} = {}}},\n",
                        new_var, data_var, field_name, init_val
                    ));
                    data_var = new_var;
                }
                code.push_str(&format!("    {{keep_state, {}}};\n", data_var));
            } else {
                code.push_str("    {keep_state, Data};\n");
            }

            // Event handlers
            for handler in &state.handlers {
                if handler.event == "$>"
                    || handler.event == "enter"
                    || handler.event == "<$"
                    || handler.event == "exit"
                {
                    continue; // Skip lifecycle handlers
                }

                let event_atom = to_snake_case(&handler.event);

                // Build parameter pattern for gen_statem call
                let call_pattern = if handler.params.is_empty() {
                    event_atom.clone()
                } else {
                    let param_names: Vec<String> = handler
                        .params
                        .iter()
                        .map(|p| erlang_safe_capitalize(&p.name))
                        .collect();
                    format!("{{{}, {}}}", event_atom, param_names.join(", "))
                };

                code.push_str(&format!(
                    "{}({{call, From}}, {}, Data) ->\n",
                    state_name, call_pattern
                ));

                // State params: bind frame_state_args[name] to a local
                // Erlang variable so handler bodies can read state params
                // by their declared name. Mirrors the Python dispatch
                // preamble that prepends `name = compartment.state_args[name]`.
                for sp in &state.params {
                    let cap = erlang_safe_capitalize(&sp.name);
                    code.push_str(&format!(
                        "    {} = maps:get(<<\"{}\">>, Data#data.frame_state_args, undefined),\n",
                        cap, sp.name
                    ));
                }

                // Use emit_handler_body_via_statements for proper Frame statement expansion
                let handler_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: handler.event.clone(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    current_return_type: None,
                };
                // Convert frame_ast::Span to ast::Span
                let body_span = crate::frame_c::compiler::ast::Span {
                    start: handler.body.span.start,
                    end: handler.body.span.end,
                };
                let raw_spliced = emit_handler_body_via_statements(
                    &body_span,
                    source,
                    TargetLanguage::Erlang,
                    &handler_ctx,
                );

                // Transform if/else { } blocks to Erlang case/of/end
                let spliced_body = erlang_transform_blocks(&raw_spliced);

                // Post-process: rewrite self.X, capitalize params, thread Data.
                // Include both handler params AND state params (declared via
                // `$Start(x: int)`) so the body can reference state-args
                // bound at the top of the clause by their declared name.
                let handler_params: Vec<(&str, String)> = handler
                    .params
                    .iter()
                    .map(|p| {
                        let capitalized = erlang_safe_capitalize(&p.name);
                        (p.name.as_str(), capitalized)
                    })
                    .chain(state.params.iter().map(|sp| {
                        let capitalized = erlang_safe_capitalize(&sp.name);
                        (sp.name.as_str(), capitalized)
                    }))
                    .collect();

                // Check if the spliced body contains a gen_statem return tuple, forward, or frame_transition
                let has_forward_call = spliced_body.contains("({call, From},");
                let has_frame_transition = spliced_body.contains("frame_transition__(");
                let has_return_tuple = spliced_body.contains("{next_state,")
                    || spliced_body.contains("{keep_state,")
                    || has_forward_call
                    || has_frame_transition;
                let has_case_block =
                    spliced_body.contains("case (") || spliced_body.contains("case(");

                if has_return_tuple {
                    // Exit handler is now handled by __frame_transition — no inlining needed

                    // Process through Data threading (handles both simple and case-block bodies)
                    let lines: Vec<&str> = spliced_body.lines().collect();
                    let (processed, _final_data, final_rv) = erlang_process_body_lines_full(
                        &lines,
                        &action_names,
                        &interface_names,
                        "Data",
                        &handler_params,
                    );
                    // Wrap any @@:self dispatch sites in transition-guard
                    // `case` expressions so a state change inside the called
                    // handler short-circuits the rest of the caller's body.
                    // No-op if the body has no `= frame_dispatch__(` lines.
                    let processed = erlang_wrap_self_call_guards(
                        &processed,
                        &to_snake_case(&state.name),
                    );
                    // Inject gen_statem reply tuples at any orphan
                    // `__ReturnVal = "..."` leaves (innermost else branches
                    // of nested if/else that don't transition). Fixes the
                    // "bad_return_from_state_function" crash when a handler
                    // has a non-transitioning terminal case arm.
                    let processed =
                        erlang_inject_orphan_reply_tuples(&processed, &_final_data);
                    if !processed.is_empty() {
                        // Use structured case arm analysis when a case block exists
                        if let Some((classification, arms, case_start, case_end)) =
                            analyze_case_arms(&processed)
                        {
                            match classification {
                                CaseBlockClassification::AllTerminal => {
                                    // All arms have transitions — case is terminal, use as handler return
                                    let emit_lines = &processed[..=case_end];
                                    erlang_smart_join(emit_lines, &mut code);
                                    if code.trim_end().ends_with("end,") {
                                        if let Some(pos) = code.rfind("end,") {
                                            code.replace_range(pos + 3..pos + 4, "");
                                        }
                                    }
                                }
                                CaseBlockClassification::Mixed => {
                                    // Some arms transition, some don't — per-arm rewrite
                                    let rewritten = rewrite_mixed_case_arms(
                                        &processed,
                                        &arms,
                                        case_start,
                                        case_end,
                                        &_final_data,
                                    );
                                    erlang_smart_join(&rewritten, &mut code);
                                }
                                CaseBlockClassification::NoTerminal => {
                                    // Case arms have no transitions of their own, but the body
                                    // leading into the case DID contain a forward (otherwise we
                                    // wouldn't be in has_return_tuple). The forward bind produces
                                    // `__FwdNextN`; the case arms only thread Data. Emit the
                                    // case block as-is, then append a conditional terminal tuple
                                    // that honors whichever transition (if any) the parent
                                    // performed — mirroring the no-case-block path below.
                                    erlang_smart_join(&processed, &mut code);
                                    let last_fwd_var: Option<String> = processed
                                        .iter()
                                        .rev()
                                        .find_map(|line| {
                                            line.find("__FwdNext").map(|i| {
                                                let rest = &line[i..];
                                                rest.chars()
                                                    .take_while(|c| {
                                                        c.is_ascii_alphanumeric() || *c == '_'
                                                    })
                                                    .collect::<String>()
                                            })
                                        });
                                    let reply_val = final_rv.as_str();
                                    if let Some(fwd) = last_fwd_var {
                                        code.push_str(",\n");
                                        code.push_str(&format!(
                                            "    case {} of\n        undefined -> {{keep_state, {}, [{{reply, From, {}}}]}};\n        _ -> {{next_state, {}, {}, [{{reply, From, {}}}]}}\n    end",
                                            fwd, _final_data, reply_val, fwd, _final_data, reply_val
                                        ));
                                    } else {
                                        code.push_str(",\n");
                                        code.push_str(&format!(
                                            "    {{keep_state, {}, [{{reply, From, {}}}]}}",
                                            _final_data, reply_val
                                        ));
                                    }
                                }
                            }
                        } else {
                            // No case block — use existing terminal detection for linear handlers.
                            // A raw forward tail-call (`p({call,From},...)`) IS terminal. A forward
                            // *bind* emitted by the body processor (`{Data1, __FwdNext1} = frame_unwrap_forward__(...)`)
                            // is not — post-forward code follows and must run. The leading `{` guard
                            // distinguishes them.
                            let is_terminal = |l: &str| -> bool {
                                let t = l.trim();
                                (t.contains("({call, From},") && !t.starts_with("{"))
                                    || t.starts_with("frame_transition__(")
                                    || t.starts_with("{next_state,")
                                    || t.starts_with("{keep_state,")
                            };
                            let mut terminal_idx: Option<usize> = None;
                            for (idx, line) in processed.iter().enumerate() {
                                if is_terminal(line.trim()) {
                                    terminal_idx = Some(idx);
                                    break;
                                }
                            }
                            if let Some(tidx) = terminal_idx {
                                erlang_smart_join(&processed[..=tidx], &mut code);
                            } else {
                                // No inline terminal. If a forward was rewritten to a bind, emit
                                // a conditional tuple that propagates the parent's transition
                                // (if any) after post-forward statements have run.
                                erlang_smart_join(&processed, &mut code);
                                let last_fwd_var: Option<String> = processed
                                    .iter()
                                    .rev()
                                    .find_map(|line| {
                                        line.find("__FwdNext").map(|i| {
                                            let rest = &line[i..];
                                            rest.chars()
                                                .take_while(|c| {
                                                    c.is_ascii_alphanumeric() || *c == '_'
                                                })
                                                .collect::<String>()
                                        })
                                    });
                                let reply_val = final_rv.as_str();
                                if let Some(fwd) = last_fwd_var {
                                    code.push_str(",\n");
                                    code.push_str(&format!(
                                        "    case {} of\n        undefined -> {{keep_state, {}, [{{reply, From, {}}}]}};\n        _ -> {{next_state, {}, {}, [{{reply, From, {}}}]}}\n    end",
                                        fwd, _final_data, reply_val, fwd, _final_data, reply_val
                                    ));
                                } else {
                                    code.push_str(",\n");
                                    code.push_str(&format!(
                                        "    {{keep_state, {}, [{{reply, From, {}}}]}}",
                                        _final_data, reply_val
                                    ));
                                }
                            }
                        }
                    }
                    // Ensure clause terminator is on its own line (not hidden by % comment)
                    if !code.ends_with('\n') {
                        code.push('\n');
                    }
                    code.push_str(";\n");
                } else {
                    // No return tuple — process through Data threading and add return
                    let lines: Vec<&str> = spliced_body.lines().collect();
                    let (processed, final_data, final_rv_nr) = erlang_process_body_lines_full(
                        &lines,
                        &action_names,
                        &interface_names,
                        "Data",
                        &handler_params,
                    );
                    if processed.is_empty() {
                        code.push_str("    {keep_state, Data, [{reply, From, ok}]};\n");
                    } else {
                        // Use the body processor's authoritative
                        // final return-value name. `"ok"` when no
                        // `@@:return` writes happened in the body.
                        let has_return_val = final_rv_nr != "ok";
                        let has_transition = processed
                            .iter()
                            .any(|l| l.trim().starts_with("frame_transition__("));
                        let has_case = processed
                            .iter()
                            .any(|l| l.trim().starts_with("case ") || l.contains(" case "));
                        let reply_val: &str = final_rv_nr.as_str();

                        if has_case && has_transition {
                            // Case block with transitions in some arms.
                            // Each arm must evaluate to a gen_statem return tuple:
                            //   - Arms with frame_transition__() already produce {next_state,...}
                            //   - Arms without need {keep_state, Data, [{reply, From, ReturnVal}]}
                            // The case expression IS the handler return — no trailing {keep_state,...}
                            let mut rewritten = Vec::new();
                            let mut in_case = false;
                            let mut arm_has_transition = false;
                            let mut arm_return_val: Option<String> = None;

                            for line in &processed {
                                let trimmed = line.trim();

                                if trimmed.starts_with("case ") {
                                    in_case = true;
                                    arm_has_transition = false;
                                    arm_return_val = None;
                                    rewritten.push(line.clone());
                                    continue;
                                }

                                if in_case
                                    && (trimmed.starts_with("true ->")
                                        || trimmed.starts_with("; false")
                                        || trimmed.starts_with("; _"))
                                {
                                    // Entering a new arm — flush previous arm's keep_state if needed
                                    if trimmed.starts_with("; ") && !arm_has_transition {
                                        // Previous arm had no transition — inject keep_state
                                        let rv = arm_return_val.as_deref().unwrap_or("ok");
                                        rewritten.push(format!(
                                            "        {{keep_state, {}, [{{reply, From, {}}}]}}",
                                            final_data, rv
                                        ));
                                    }
                                    arm_has_transition = false;
                                    arm_return_val = None;
                                    rewritten.push(line.clone());
                                    continue;
                                }

                                if in_case && trimmed.starts_with("__ReturnVal = ") {
                                    let val = trimmed
                                        .trim_start_matches("__ReturnVal = ")
                                        .trim_end_matches(',');
                                    arm_return_val = Some(val.to_string());
                                    // Don't emit the assignment — embed the value in the reply tuple
                                    continue;
                                }

                                if in_case && trimmed.starts_with("frame_transition__(") {
                                    arm_has_transition = true;
                                    // Emit the transition call — it produces the arm's return tuple
                                    rewritten.push(line.clone());
                                    continue;
                                }

                                if in_case && (trimmed == "end" || trimmed == "end,") {
                                    // Last arm ending — inject keep_state if no transition
                                    if !arm_has_transition {
                                        let rv = arm_return_val.as_deref().unwrap_or("ok");
                                        rewritten.push(format!(
                                            "        {{keep_state, {}, [{{reply, From, {}}}]}}",
                                            final_data, rv
                                        ));
                                    }
                                    rewritten.push(format!("    end"));
                                    in_case = false;
                                    continue;
                                }

                                rewritten.push(line.clone());
                            }

                            erlang_smart_join(&rewritten, &mut code);
                            // The case expression is the handler return — just terminate the clause
                            code.push_str(";\n");
                        } else if has_return_val && has_case {
                            // Case block with __ReturnVal but no transitions — hoist assignment
                            let mut rewritten = Vec::new();
                            let mut in_case = false;
                            let mut hoisted = false;
                            for line in &processed {
                                let trimmed = line.trim();
                                if trimmed.starts_with("case ") && !hoisted {
                                    rewritten.push(format!("    __ReturnVal = {}", trimmed));
                                    in_case = true;
                                    hoisted = true;
                                } else if in_case && trimmed.starts_with("__ReturnVal = ") {
                                    let val = trimmed.trim_start_matches("__ReturnVal = ");
                                    rewritten.push(format!("    {}", val));
                                } else if in_case && (trimmed == "end" || trimmed == "end,") {
                                    rewritten.push(line.clone());
                                    in_case = false;
                                } else {
                                    rewritten.push(line.clone());
                                }
                            }
                            erlang_smart_join(&rewritten, &mut code);
                            code.push_str(",\n");
                            code.push_str(&format!(
                                "    {{keep_state, {}, [{{reply, From, {}}}]}};\n",
                                final_data, reply_val
                            ));
                        } else {
                            // Build the full body — processed body + keep_state terminal —
                            // then wrap any `@@:self` dispatch sites in transition-guard
                            // cases so a state change inside the called handler
                            // short-circuits the rest of the caller and propagates the
                            // new state back through gen_statem.
                            let mut full = processed.clone();
                            full.push(format!(
                                "    {{keep_state, {}, [{{reply, From, {}}}]}}",
                                final_data, reply_val
                            ));
                            let wrapped = erlang_wrap_self_call_guards(
                                &full,
                                &to_snake_case(&state.name),
                            );
                            erlang_smart_join(&wrapped, &mut code);
                            code.push_str(";\n");
                        }
                    }
                }
            }

            // `frame_op_call` dispatch — routes external op calls into
            // the server process. Each non-static op's external wrapper
            // emits `gen_statem:call(Pid, {frame_op_call, <op>, [Args]})`;
            // here we match that message, invoke the internal op (same
            // function name, arity-1 Data clause), destructure the
            // `{UpdatedData, Result}` tuple, and reply with Result while
            // keeping the updated Data in the gen_statem state. This
            // clause is emitted in every state so ops are callable
            // regardless of the machine's current state.
            for op in &system.operations {
                if op.is_static {
                    continue;
                }
                let op_lc = erlang_op_name(&op.name);
                let n = op.params.len();
                let arg_vars: Vec<String> =
                    (0..n).map(|i| format!("A{}", i + 1)).collect();
                let pattern_args = if arg_vars.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", arg_vars.join(", "))
                };
                let call_args = if arg_vars.is_empty() {
                    "Data".to_string()
                } else {
                    format!("Data, {}", arg_vars.join(", "))
                };
                code.push_str(&format!(
                    "{}({{call, From}}, {{frame_op_call, {}, {}}}, Data) ->\n    {{NewData, __Result}} = {}({}),\n    {{keep_state, NewData, [{{reply, From, __Result}}]}};\n",
                    state_name, op_lc, pattern_args, op_lc, call_args
                ));
            }

            // State-timeout handler for deferred enter-handler transitions.
            // When an enter handler calls -> $State, we defer via:
            //   {keep_state, Data, [{state_timeout, 0, {frame_enter_transition, Target}}]}
            // This clause processes the resulting state_timeout event.
            code.push_str(&format!(
                "{}(state_timeout, {{frame_enter_transition, Target}}, Data) ->\n    {{next_state, Target, Data}};\n",
                state_name
            ));

            // Default catch-all for unhandled events in this state
            // HSM: if state has a parent, forward unhandled call events to parent
            if let Some(ref parent) = state.parent {
                let parent_atom = state_atom(parent);
                code.push_str(&format!("{}({{call, From}}, __Event, Data) ->\n    {}({{call, From}}, __Event, Data);\n", state_name, parent_atom));
                code.push_str(&format!(
                    "{}(_EventType, _Event, Data) ->\n    {{keep_state, Data}}.\n\n",
                    state_name
                ));
            } else {
                // Catch-all for call events — must reply to avoid caller deadlock
                code.push_str(&format!("{}({{call, From}}, _Event, Data) ->\n    {{keep_state, Data, [{{reply, From, ok}}]}};\n", state_name));
                code.push_str(&format!(
                    "{}(_EventType, _Event, Data) ->\n    {{keep_state, Data}}.\n\n",
                    state_name
                ));
            }
        }
    }

    // Frame transition helper — orchestrates exit → arg passing → gen_statem transition
    code.push_str(
        "frame_transition__(TargetState, Data, ExitArgs, EnterArgs, StateArgs, From) ->\n",
    );
    code.push_str("    Data1 = Data#data{frame_exit_args = ExitArgs},\n");
    code.push_str("    Data2 = frame_exit_dispatch__(Data1),\n");
    code.push_str("    Data3 = Data2#data{frame_enter_args = EnterArgs, frame_state_args = StateArgs, frame_current_state = TargetState},\n");
    code.push_str("    {next_state, TargetState, Data3, [{reply, From, ok}]}.\n\n");

    // HSM parent-forward unwrap. When a child's `=> $^` has post-forward code
    // (e.g., `=> $^; self.x = self.x + 1`), we can't emit the parent call as
    // a tail call — the post-forward statements would be lost. The body
    // processor instead binds: `{DataN, __FwdNextN} = frame_unwrap_forward__(ParentCall)`.
    // This helper flattens the parent's gen_statem return tuple into
    // `{UpdatedData, NextStateOrUndefined}` so the child can continue threading
    // Data through its remaining statements and finally emit a single return
    // tuple that honors whatever transition (if any) the parent performed.
    // Matches the 16-backend consensus that post-forward code always runs.
    code.push_str("frame_unwrap_forward__({keep_state, D, _}) -> {D, undefined};\n");
    code.push_str("frame_unwrap_forward__({keep_state, D}) -> {D, undefined};\n");
    code.push_str("frame_unwrap_forward__({next_state, NS, D, _}) -> {D, NS};\n");
    code.push_str("frame_unwrap_forward__({next_state, NS, D}) -> {D, NS}.\n\n");

    // Exit handler dispatch — routes to per-state exit function
    code.push_str("frame_exit_dispatch__(Data) ->\n");
    code.push_str("    case Data#data.frame_current_state of\n");
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if state.exit.is_some() {
                let sname = state_atom(&state.name);
                code.push_str(&format!(
                    "        {} -> frame_exit__{}(Data);\n",
                    sname, sname
                ));
            }
        }
    }
    code.push_str("        _ -> Data\n");
    code.push_str("    end.\n\n");

    // Per-state exit handler functions
    if let Some(ref machine) = system.machine {
        for state in &machine.states {
            if let Some(ref exit) = state.exit {
                let sname = state_atom(&state.name);
                code.push_str(&format!("frame_exit__{}(Data) ->\n", sname));

                // Extract exit params
                for (i, p) in exit.params.iter().enumerate() {
                    let var_name = erlang_safe_capitalize(&p.name);
                    code.push_str(&format!(
                        "    {} = maps:get(<<\"{}\">>, Data#data.frame_exit_args, undefined),\n",
                        var_name, i
                    ));
                }

                // Exit handler body via splicer
                let exit_ctx = HandlerContext {
                    system_name: sys.to_string(),
                    state_name: state.name.clone(),
                    event_name: "<$".to_string(),
                    parent_state: state.parent.clone(),
                    defined_systems: std::collections::HashSet::from([sys.to_string()]),
                    use_sv_comp: false,
                    state_var_types: std::collections::HashMap::new(),
                    state_param_names: std::collections::HashMap::new(),
                    state_enter_param_names: std::collections::HashMap::new(),
                    state_exit_param_names: std::collections::HashMap::new(),
                    event_param_names: std::collections::HashMap::new(),
                    current_return_type: None,
                };
                let exit_span = crate::frame_c::compiler::ast::Span {
                    start: exit.body.span.start,
                    end: exit.body.span.end,
                };
                let raw_exit = emit_handler_body_via_statements(
                    &exit_span,
                    source,
                    TargetLanguage::Erlang,
                    &exit_ctx,
                );

                let exit_params: Vec<(&str, String)> = exit
                    .params
                    .iter()
                    .map(|p| {
                        let cap = erlang_safe_capitalize(&p.name);
                        (p.name.as_str(), cap)
                    })
                    .collect();
                let lines: Vec<&str> = raw_exit.lines().collect();
                let (processed, final_data, _final_rv) = erlang_process_body_lines_with_params(
                    &lines,
                    &action_names,
                    "Data",
                    &exit_params,
                );

                if !processed.is_empty() {
                    erlang_smart_join(&processed, &mut code);
                    code.push_str(",\n");
                }
                code.push_str(&format!("    {}.\n\n", final_data));
            }
        }
    }

    // Frame internal dispatch — for self.method() calls within handlers
    // Calls the state handler directly (avoids gen_statem:call deadlock)
    // Pushes/pops context stack for reentrancy
    code.push_str("frame_dispatch__(EventName, Args, Data) ->\n");
    code.push_str("    Ctx = #{return_val => undefined, data => #{}},\n");
    code.push_str(
        "    Data1 = Data#data{frame_context_stack = [Ctx | Data#data.frame_context_stack]},\n",
    );
    code.push_str("    Msg = case Args of\n");
    code.push_str("        [] -> EventName;\n");
    code.push_str("        _ -> list_to_tuple([EventName | Args])\n");
    code.push_str("    end,\n");
    code.push_str("    State = Data1#data.frame_current_state,\n");
    code.push_str("    FakeFrom = {self(), make_ref()},\n");
    code.push_str("    Result = ?MODULE:State({call, FakeFrom}, Msg, Data1),\n");
    code.push_str("    case Result of\n");
    code.push_str("        {keep_state, Data2, Actions} ->\n");
    code.push_str("            RetVal = case [V || {reply, _, V} <- Actions] of\n");
    code.push_str("                [V | _] -> V;\n");
    code.push_str("                [] -> undefined\n");
    code.push_str("            end,\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack)},\n");
    code.push_str("            {Data3, RetVal};\n");
    code.push_str("        {keep_state, Data2} ->\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack)},\n");
    code.push_str("            {Data3, undefined};\n");
    code.push_str("        {next_state, NewState, Data2, Actions} ->\n");
    code.push_str("            RetVal = case [V || {reply, _, V} <- Actions] of\n");
    code.push_str("                [V | _] -> V;\n");
    code.push_str("                [] -> undefined\n");
    code.push_str("            end,\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack), frame_current_state = NewState},\n");
    code.push_str("            {Data3, RetVal};\n");
    code.push_str("        {next_state, NewState, Data2} ->\n");
    code.push_str("            Data3 = Data2#data{frame_context_stack = tl(Data2#data.frame_context_stack), frame_current_state = NewState},\n");
    code.push_str("            {Data3, undefined}\n");
    code.push_str("    end.\n\n");

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
                trimmed[1..trimmed.len() - 1].trim()
            } else {
                trimmed
            };
            // Run `if {...} else {...}` → `case X of true -> ...; false -> ... end`
            // conversion over the action body before the classifier pipeline.
            // Handler bodies get this via erlang_transform_blocks at their
            // emission sites; action bodies previously skipped it, causing
            // malformed output for any action containing control flow.
            let transformed = erlang_transform_blocks(inner_raw);
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
                        num_end > 0
                            && rest[num_end..]
                                .trim_start()
                                .starts_with('=')
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

                if last_is_value && processed.len() > 0 {
                    // Last expression is the return value — emit body up to last line,
                    // then return {FinalData, LastExpr}
                    let body_lines = &processed[..processed.len() - 1];
                    if !body_lines.is_empty() {
                        erlang_smart_join(body_lines, &mut code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {{{}, {}}}", final_data, last_line));
                } else if let Some(result_var) = action_result_var {
                    // Tail action-call: preserve its result as the action's
                    // return value.
                    erlang_smart_join(&processed, &mut code);
                    code.push_str(",\n");
                    code.push_str(&format!("    {{{}, {}}}", final_data, result_var));
                } else {
                    erlang_smart_join(&processed, &mut code);
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
                super::frame_expansion::expand_system_state_in_code(inner, TargetLanguage::Erlang);
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
                    for iface in &interface_names {
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
                                    Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
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
                    for action in &action_names {
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
                            let rhs = l[eq_pos + 3..].trim().trim_end_matches(|c: char| c == ',' || c == '.');
                            data_bind_counter += 1;
                            let prev = if data_bind_counter == 1 {
                                "Data".to_string()
                            } else {
                                format!("Data{}", data_bind_counter - 1)
                            };
                            format!("Data{} = {}#data{{{} = {}}}", data_bind_counter, prev, field, rhs)
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
                        num_end > 0
                            && rest[num_end..]
                                .trim_start()
                                .starts_with('=')
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
                        erlang_smart_join(body_lines, &mut code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {{{}, {}}}", final_data, last_line));
                } else {
                    erlang_smart_join(&processed_lines, &mut code);
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
                        num_end > 0
                            && rest[num_end..]
                                .trim_start()
                                .starts_with('=')
                    } else {
                        false
                    }
                };
                let is_tail_tuple_call = action_names.iter().any(|a| {
                    let lc = erlang_op_name(a.as_str());
                    let pat_no_args = format!("{}(Data)", lc);
                    let pat_with_args = format!("{}(Data,", lc);
                    last_line == pat_no_args
                        || last_line.starts_with(&pat_with_args)
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
                        erlang_smart_join(body_lines, &mut code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {{Data, {}}}", last_line));
                } else if is_tail_tuple_call {
                    // Forward another tuple-returning call's result
                    // straight through — preserves Data threading.
                    erlang_smart_join(&processed_lines, &mut code);
                } else {
                    erlang_smart_join(&processed_lines, &mut code);
                }
            } else {
                // Static op: pure function, no Data threading.
                erlang_smart_join(&processed_lines, &mut code);
            }
        }
        code.push_str(".\n\n");
    }

    // Persistence methods (when @@persist is present)
    if system.persist_attr.is_some() {
        // Collect all record field names for serialization
        let mut field_names: Vec<String> = Vec::new();
        for var in &system.domain {
            field_names.push(var.name.clone());
        }
        if let Some(ref machine) = system.machine {
            for state in &machine.states {
                let state_prefix = to_snake_case(&state.name);
                for sv in &state.state_vars {
                    field_names.push(format!("sv_{}_{}", state_prefix, sv.name));
                }
            }
        }
        field_names.push("frame_stack".to_string());

        // save_state/1 — serializes current state + Data to a map
        code.push_str("save_state(Pid) ->\n");
        code.push_str("    {State, Data} = sys:get_state(Pid),\n");
        code.push_str("    #{state => State,\n");
        for (i, field) in field_names.iter().enumerate() {
            let comma = if i < field_names.len() - 1 { "," } else { "" };
            code.push_str(&format!(
                "      {} => Data#data.{}{}\n",
                field, field, comma
            ));
        }
        code.push_str("    }.\n\n");

        // load_state/1 — deserializes map and starts a new gen_statem
        code.push_str("load_state(Map) ->\n");
        code.push_str("    State = maps:get(state, Map),\n");
        code.push_str("    Data = #data{\n");
        for (i, field) in field_names.iter().enumerate() {
            let comma = if i < field_names.len() - 1 { "," } else { "" };
            code.push_str(&format!(
                "        {} = maps:get({}, Map, undefined){}\n",
                field, field, comma
            ));
        }
        code.push_str("    },\n");
        code.push_str("    {ok, Pid} = gen_statem:start_link(?MODULE, [], []),\n");
        code.push_str("    sys:replace_state(Pid, fun(_) -> {State, Data} end),\n");
        code.push_str("    {ok, Pid}.\n\n");

        // Add save_state/load_state to exports
        // Need to insert into the export list — prepend to code
        let save_export = format!("-export([save_state/1, load_state/1]).\n");
        // Find position after the last -export line
        if let Some(pos) = code.rfind("-export([callback_mode/0") {
            if let Some(newline) = code[pos..].find('\n') {
                let insert_pos = pos + newline + 1;
                code.insert_str(insert_pos, &save_export);
            }
        }
    }

    // Wrap in a NativeBlock — the assembler will stitch prolog + this + epilog
    CodegenNode::NativeBlock { code, span: None }
}
