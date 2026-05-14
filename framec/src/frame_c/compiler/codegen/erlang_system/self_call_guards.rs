//! Transition-guard wrapping for `@@:self.method()` dispatch sites.
//!
//! In Frame, calling `@@:self.method()` inside a handler invokes the
//! system's own dispatch path — and that callee may transition the
//! system to a new state. After such a call, the rest of the
//! caller's body should be suppressed and the handler should return
//! a `{next_state, NewState, NewData, [{reply, From, undefined}]}`
//! tuple to gen_statem (the dynamic-language backends get this for
//! free via a mutable `_transitioned` flag and an early `return`).
//!
//! This module wraps the body processor's linear output: every
//! `= frame_dispatch__(` line is followed by a `case
//! DataN#data.frame_current_state of` split, with the enclosing
//! state's own atom as the "no transition" arm (re-runs the
//! caller's tail), and a `_` arm that produces the transitioned
//! reply tuple. Nested @@:self calls recurse naturally.

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
pub(super) fn erlang_wrap_self_call_guards(lines: &[String], state_atom: &str) -> Vec<String> {
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
            ind = ind,
            dv = data_var
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
                    || t.starts_with("frame_forward_transition__(")
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
                || t.starts_with("frame_forward_transition__(")
        });
        let arm_final_data: String = inner_wrapped
            .iter()
            .rev()
            .find_map(|l| {
                let t = l.trim();
                if t.starts_with("Data") {
                    let rest = &t[4..];
                    let n: usize = rest.chars().take_while(|c| c.is_ascii_digit()).count();
                    if n > 0 && rest[n..].trim_start().starts_with('=') {
                        return Some(t[..4 + n].to_string());
                    }
                }
                None
            })
            .unwrap_or_else(|| data_var.clone());
        // arm_reply: the reply value used in BOTH the matched arm
        // (when no transition occurred) and the `_ ->` arm (when one
        // did). The `_ ->` arm has a hard constraint: only variables
        // visible at OUTER scope can be referenced. Variables bound
        // by direct assignment inside the matched arm body
        // (e.g. `__ReturnVal_1 = 5 + __SelfResult_1`) are arm-local
        // and would be unbound in the `_ ->` arm.
        //
        // Strategy: scan the body's terminal `[{reply, From, X}]`
        // tuple to extract X. Then verify X is bound at outer scope
        // (tuple-destructure pattern `{...DataN, X}` — pre-case
        // visible) and NOT by a direct `X = expr` line (arm-local).
        // Fall back to `ok` if X isn't outer-scope visible.
        let arm_reply: String = {
            // Find the terminal reply value (X in `[{reply, From, X}]`).
            let mut terminal_var: Option<String> = None;
            for l in inner_wrapped.iter().rev() {
                let t = l.trim();
                if let Some(pos) = t.find("[{reply, From, ") {
                    let after = &t[pos + "[{reply, From, ".len()..];
                    if let Some(close) = after.find('}') {
                        let val = after[..close].trim().to_string();
                        if !val.is_empty() {
                            terminal_var = Some(val);
                            break;
                        }
                    }
                }
            }
            match terminal_var {
                Some(val) => {
                    // Verify val is bound at outer scope. Direct
                    // assignment (`X = expr`) inside the body becomes
                    // arm-local once wrapped; tuple-destructure
                    // (`{Data, X} = ...`) is pre-case visible.
                    let direct_assign_prefix = format!("{} = ", val);
                    let tuple_bind_substr = format!(", {}}}", val);
                    let has_direct_assign = inner_wrapped
                        .iter()
                        .any(|x| x.trim().starts_with(&direct_assign_prefix));
                    let has_tuple_bind =
                        inner_wrapped.iter().any(|x| x.contains(&tuple_bind_substr));
                    if has_tuple_bind && !has_direct_assign {
                        val
                    } else {
                        "ok".to_string()
                    }
                }
                None => "ok".to_string(),
            }
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

        // Bare `_ ->` form: the previous arm body already has a
        // trailing `;` from the wrap (line ~1654). Doubling the
        // separator (`;` after body PLUS `; _ ->`) is an Erlang
        // syntax error. The analyzer in `analyze_case_arms` was
        // extended to recognise bare-pattern arm headers so the
        // wrap-emitted case is correctly classified.
        //
        // Reply value: must match the matching arm's reply
        // (`arm_reply`), not hardcoded `undefined`. A dispatch
        // that caused a transition still produces a return value
        // — discarding it across the transition was wrong and
        // surfaced by Phase 14 P8 (child overrides compute,
        // drive forwards, parent calls @@:self.compute()
        // — the dispatch's __ReturnVal_1 must reach the reply
        // tuple regardless of whether a transition happened).
        result.push(format!("{ind}    _ ->", ind = ind));
        result.push(format!(
            "{ind}        {{next_state, {dv}#data.frame_current_state, {dv}, [{{reply, From, {rv}}}]}}",
            ind = ind, dv = data_var, rv = arm_reply
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
