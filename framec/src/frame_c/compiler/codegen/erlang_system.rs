//! Erlang gen_statem code generation.
//!
//! This module generates complete Erlang/OTP gen_statem modules from Frame systems.
//! It bypasses the standard class-based CodegenNode pipeline entirely, producing
//! raw Erlang source text with proper gen_statem callbacks, -record(data, {}),
//! and Frame infrastructure (frame_transition__, frame_dispatch__, etc.).

use super::ast::CodegenNode;
use super::codegen_utils::{
    convert_expression, convert_literal, expression_to_string, is_bool_type, is_float_type,
    is_int_type, is_string_type, to_snake_case, type_to_string, HandlerContext,
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

    // self.method(args) → interface dispatch (for interface methods)
    for iface in interface_names {
        let pattern = format!("self.{}(", iface);
        if l.contains(&pattern) {
            // Extract: result_var = self.method(args)
            let method_snake = to_snake_case(iface);
            if let Some(eq_pos) = l.find('=') {
                let result_var = l[..eq_pos].trim().to_string();
                let call_start = l.find(&pattern).map(|p| p + pattern.len()).unwrap_or(0);
                let call_end = l.rfind(')').unwrap_or(l.len());
                let args = l[call_start..call_end].trim().to_string();
                return ErlangRewrite::InterfaceCall {
                    method: method_snake,
                    args,
                    result_var,
                };
            } else {
                // No assignment — just call
                let call_start = l.find(&pattern).map(|p| p + pattern.len()).unwrap_or(0);
                let call_end = l.rfind(')').unwrap_or(l.len());
                let args = l[call_start..call_end].trim().to_string();
                return ErlangRewrite::InterfaceCall {
                    method: method_snake,
                    args,
                    result_var: "_".to_string(),
                };
            }
        }
    }

    // self.method(args) → action call that modifies Data
    for action in action_names {
        let pattern = format!("self.{}(", action);
        if l.contains(&pattern) {
            let replaced = l.replace(&pattern, &format!("{}({}, ", action, data_var));
            let fixed = replaced.replace(&format!("({}, )", data_var), &format!("({})", data_var));
            return ErlangRewrite::ActionCall(fixed);
        }
    }

    // self.field = expr → record update
    if l.starts_with("self.") && l.contains('=') {
        let rest = &l[5..]; // skip "self."
        if let Some(eq_pos) = rest.find('=') {
            let field = rest[..eq_pos].trim().to_string();
            let value = rest[eq_pos + 1..]
                .trim()
                .replace("self.", &format!("{}#data.", data_var));
            return ErlangRewrite::RecordUpdate { field, value };
        }
    }

    // self.field → DataVar#data.field (access)
    ErlangRewrite::Plain(l.replace("self.", &format!("{}#data.", data_var)))
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
fn erlang_process_body_lines(
    lines: &[&str],
    action_names: &[String],
    initial_data: &str,
) -> (Vec<String>, String) {
    erlang_process_body_lines_with_params(lines, action_names, initial_data, &[])
}

fn erlang_process_body_lines_with_params(
    lines: &[&str],
    action_names: &[String],
    initial_data: &str,
    param_names: &[(&str, String)],
) -> (Vec<String>, String) {
    erlang_process_body_lines_full(lines, action_names, &[], initial_data, param_names)
}

fn erlang_process_body_lines_full(
    lines: &[&str],
    action_names: &[String],
    interface_names: &[String],
    initial_data: &str,
    param_names: &[(&str, String)],
) -> (Vec<String>, String) {
    let mut result = Vec::new();
    let mut data_var = initial_data.to_string();
    let mut data_gen = 0;
    // Stack to save data_var at case branch boundaries
    let mut case_data_stack: Vec<(String, usize)> = Vec::new();

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
            // Track case branch boundaries for Data variable scoping
            // Both data_var AND data_gen are saved/restored so that both arms
            // produce the same DataN sequence (e.g., both arms end with Data2).
            if l.starts_with("true ->") {
                // Entering true branch — save current data_var and data_gen
                case_data_stack.push((data_var.clone(), data_gen));
            } else if l.starts_with("; false") || l.starts_with("; _") {
                // Entering alternate branch — restore data_var and data_gen from before true branch
                if let Some(&(ref saved_var, saved_gen)) = case_data_stack.last() {
                    data_var = saved_var.clone();
                    data_gen = saved_gen;
                }
            } else if l == "end" || l == "end," {
                // Exiting case block — pop the saved state
                case_data_stack.pop();
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
                                let action_expr = call_replaced[paren_start + 1..of_pos]
                                    .replace("self.", &format!("{}#data.", data_var));
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
            rewritten = rewritten.replace("self.", &format!("{}#data.", data_var));

            // Replace Data with current data_var in return tuples, expressions, and forward calls
            if data_var != "Data" {
                rewritten = rewritten
                    .replace(", Data,", &format!(", {},", data_var))
                    .replace(", Data}", &format!(", {}}}", data_var))
                    .replace(", Data)", &format!(", {})", data_var))
                    .replace("Data#data.", &format!("{}#data.", data_var));
            }
            result.push(format!("    {}", rewritten));
            continue;
        }

        // Suppress bare "return" — this is a Frame-generated artifact that has no meaning
        // in Erlang gen_statem (the __ReturnVal mechanism handles returns)
        if l == "return" {
            continue;
        }

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
            ErlangRewrite::InterfaceCall {
                method,
                args,
                result_var,
            } => {
                // Internal dispatch: {DataN, Result} = frame_dispatch__(method, [args], DataPrev)
                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                let args_list = if args.is_empty() {
                    "[]".to_string()
                } else {
                    format!("[{}]", args)
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

    (result, data_var)
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
    for (idx, line) in lines.iter().enumerate() {
        if !is_dispatch_call(line) {
            continue;
        }
        let data_var = match extract_new_data_var(line) {
            Some(v) => v.to_string(),
            None => continue,
        };

        let inner_wrapped = erlang_wrap_self_call_guards(&lines[idx + 1..], state_atom);
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
        let last = inner_wrapped.len().saturating_sub(1);
        for (i, l) in inner_wrapped.iter().enumerate() {
            // Prepend 8 spaces to whatever relative indent the line has.
            let re_indent = format!("        {}", l);
            if i == last {
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

        result.push(format!("{ind}    _ ->", ind = ind));
        result.push(format!(
            "{ind}        {{next_state, {dv}#data.frame_current_state, {dv}, [{{reply, From, undefined}}]}}",
            ind = ind, dv = data_var
        ));
        result.push(format!("{ind}end", ind = ind));
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
    l.replace("self.", "Data#data.").to_string()
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
            // If this was an elif, we also need to close the outer case
            if ctx == "elif" {
                if !block_depth.is_empty() {
                    block_depth.pop();
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

        if t == "end" || t == "end," {
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

        // Emit arm body lines, filtering as needed
        for i in arm.body_start..arm.body_end {
            let t = processed[i].trim();

            if t.starts_with("__ReturnVal = ") {
                // In transition arms, drop (transition replies ok)
                // In non-transition arms, capture but don't emit (used in injected tuple)
                continue;
            }

            result.push(processed[i].clone());
        }

        // For non-transition arms, inject the gen_statem return tuple
        if !arm.has_transition {
            let data = arm.final_data_var.as_deref().unwrap_or(default_data);
            let reply = arm.return_val.as_deref().unwrap_or("ok");
            result.push(format!(
                "        {{keep_state, {}, [{{reply, From, {}}}]}}",
                data, reply
            ));
        }
    }

    // Emit end
    result.push("    end".to_string());

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
                    let (processed, final_data) = erlang_process_body_lines_with_params(
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
                    let (processed, _final_data) = erlang_process_body_lines_full(
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
                                    // No transitions in case — shouldn't be in has_return_tuple branch
                                    // but handle gracefully: emit all lines
                                    erlang_smart_join(&processed, &mut code);
                                }
                            }
                        } else {
                            // No case block — use existing terminal detection for linear handlers
                            let is_terminal = |l: &str| -> bool {
                                let t = l.trim();
                                t.contains("({call, From},")
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
                            let emit_lines = if let Some(tidx) = terminal_idx {
                                &processed[..=tidx]
                            } else {
                                &processed[..]
                            };
                            erlang_smart_join(emit_lines, &mut code);
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
                    let (processed, final_data) = erlang_process_body_lines_full(
                        &lines,
                        &action_names,
                        &interface_names,
                        "Data",
                        &handler_params,
                    );
                    if processed.is_empty() {
                        code.push_str("    {keep_state, Data, [{reply, From, ok}]};\n");
                    } else {
                        // Check if @@:return was used (sets __ReturnVal)
                        let has_return_val = processed.iter().any(|l| l.contains("__ReturnVal"));
                        let has_transition = processed
                            .iter()
                            .any(|l| l.trim().starts_with("frame_transition__("));
                        let has_case = processed
                            .iter()
                            .any(|l| l.trim().starts_with("case ") || l.contains(" case "));
                        let reply_val = if has_return_val { "__ReturnVal" } else { "ok" };

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
                let (processed, final_data) = erlang_process_body_lines_with_params(
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

    // Action functions
    for action in &system.actions {
        code.push_str(&format!("{}(", action.name));
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
            let inner = if trimmed.starts_with('{') && trimmed.ends_with('}') {
                trimmed[1..trimmed.len() - 1].trim()
            } else {
                trimmed
            };
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
            let (processed, final_data) = erlang_process_body_lines_full(
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
                let last_is_value = !last_line.starts_with("Data")
                    && !last_line.starts_with("__")
                    && !last_line.is_empty()
                    && !last_line.starts_with("{")
                    && !last_line.starts_with("ok");

                if last_is_value && processed.len() > 0 {
                    // Last expression is the return value — emit body up to last line,
                    // then return {FinalData, LastExpr}
                    let body_lines = &processed[..processed.len() - 1];
                    if !body_lines.is_empty() {
                        erlang_smart_join(body_lines, &mut code);
                        code.push_str(",\n");
                    }
                    code.push_str(&format!("    {{{}, {}}}", final_data, last_line));
                } else {
                    erlang_smart_join(&processed, &mut code);
                    code.push_str(",\n");
                    code.push_str(&format!("    {{{}, ok}}", final_data));
                }
            }
        }
        code.push_str(".\n\n");
    }

    // Operations
    for op in &system.operations {
        code.push_str(&format!("{}(", op.name));
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
                let l = l.replace("self.", "Data#data.");
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
                            l.replace("Data#data.", &format!("Data{}#data.", data_bind_counter))
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
            } else {
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
