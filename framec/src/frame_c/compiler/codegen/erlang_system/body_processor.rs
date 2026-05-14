//! Per-handler body processor — the core of Frame's Erlang code
//! generation.
//!
//! Each Frame state handler body is a sequence of native lines:
//! field writes (`self.x = expr`), action calls (`self.method()`),
//! interface dispatches (`self.iface()`), C-family control flow
//! (`if/else { }`), and so on. Erlang doesn't have any of those
//! shapes directly — it has single-assignment variables, record
//! updates (`Data#data{field = ...}`), and pattern-matched
//! `case ... of` blocks.
//!
//! This module walks the line stream and emits the equivalent
//! Erlang. The key invariant is **Data threading**: each statement
//! that updates the system's record-shaped compartment binds a
//! fresh `DataN` (`Data1`, `Data2`, ...), and subsequent
//! statements reference `DataN` instead of the implicit `self`.
//!
//! The three public entry points are progressive specializations:
//!
//! - `erlang_process_body_lines(lines, action_names, initial_data)` —
//!   the simplest form, no interface dispatch / no param renames.
//! - `erlang_process_body_lines_with_params(..., param_names)` —
//!   adds Erlang's lowercase-to-uppercase variable rename for handler
//!   parameters.
//! - `erlang_process_body_lines_full(..., interface_names, ...)` —
//!   adds `self.<iface>()` → `frame_dispatch__` recognition for
//!   interface-method dispatch.
//!
//! All three return `(Vec<String>, String, String)` — the emitted
//! Erlang lines, the final `DataN` binding (so the caller can plug
//! it into a `{keep_state, DataN, ...}` tuple), and the final
//! `__ReturnVal` name (so the caller's reply tuple uses the
//! SSA-renamed write rather than a stale `__ReturnVal`).
//!
//! `CaseFrame` is a per-case-arm bookkeeping struct that handles
//! the case-as-statement-not-expression problem (see its doc).
//! `erlang_capitalize_params` is exposed for the lifecycle-helper
//! emitter in the parent module, which calls it directly.

use super::native_rewrite::{erlang_rewrite_native_classified_full, ErlangRewrite};
use super::super::codegen_utils::{replace_outside_strings_and_comments, to_snake_case};
use crate::frame_c::visitors::TargetLanguage;

/// Capitalize handler parameter names in a line of code.
/// Erlang variables must start with uppercase — `n` → `N`, `name` → `Name`
pub(super) fn erlang_capitalize_params(line: &str, param_names: &[(&str, String)]) -> String {
    let mut result = line.to_string();
    // Replace longest names first to avoid partial matches
    let mut sorted_params: Vec<_> = param_names.to_vec();
    sorted_params.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
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
                // or inside quoted atoms ('foo'): `'` flanks an atom, not a
                // variable, so a name collision between a state atom and a
                // param name (test 67: state $Active + param `active`) keeps
                // the atom form when the codegen quotes it.
                let prev_byte = result.as_bytes()[i.saturating_sub(1)];
                let before_ok = i == 0
                    || !prev_byte.is_ascii_alphanumeric()
                        && prev_byte != b'_'
                        && prev_byte != b'#'
                        && prev_byte != b'.'
                        && prev_byte != b'\'';
                let after_ok = i + orig_len >= result.len()
                    || !result.as_bytes()[i + orig_len].is_ascii_alphanumeric()
                        && result.as_bytes()[i + orig_len] != b'_'
                        && result.as_bytes()[i + orig_len] != b'\'';
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
pub(super) type ErlangBodyResult = (Vec<String>, String, String);

pub(super) fn erlang_process_body_lines(
    lines: &[&str],
    action_names: &[String],
    initial_data: &str,
) -> ErlangBodyResult {
    erlang_process_body_lines_with_params(lines, action_names, &[], initial_data, &[])
}

pub(super) fn erlang_process_body_lines_with_params(
    lines: &[&str],
    action_names: &[String],
    interface_names: &[String],
    initial_data: &str,
    param_names: &[(&str, String)],
) -> ErlangBodyResult {
    erlang_process_body_lines_full(lines, action_names, interface_names, initial_data, param_names)
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

pub(super) fn erlang_process_body_lines_full(
    lines: &[&str],
    action_names: &[String],
    interface_names: &[String],
    initial_data: &str,
    param_names: &[(&str, String)],
) -> ErlangBodyResult {
    let mut result: Vec<String> = Vec::new();
    let mut data_var = initial_data.to_string();
    // Derive the starting generation counter from the initial Data
    // variable name. When the caller passes `Data` (the default), gen
    // starts at 0 so the first emitted binding is `Data1`. When the
    // caller passes `Data1` (e.g., from a leaf clause's HSM enter
    // cascade that already produced `Data1`), gen must start at 1 so
    // the first emitted binding is `Data2` — otherwise we'd rebind
    // the existing `Data1` and Erlang's pattern match would fail.
    let mut data_gen: usize = initial_data
        .strip_prefix("Data")
        .and_then(|tail| tail.parse::<usize>().ok())
        .unwrap_or(0);
    let mut case_data_stack: Vec<CaseFrame> = Vec::new();

    // Pre-process step 1: translate Frame `#` comments to Erlang `%`
    // comments. Left as-is, `# foo` is an Erlang syntax error. But `#`
    // is *also* Erlang record/map syntax — `Var#rec{...}`, `Var#rec.f`,
    // `#{k => v}`, `Map#{k => v}` — and by this point some body lines
    // already carry it (a lowered `if ($.x) {...}` becomes
    // `case Data#data.sv_... of`; dict literals become `#{...}`). A
    // record `#` is flanked by identifier chars on both sides; a map
    // `#` is followed by `{`. A comment `#` is neither.
    let is_ident_byte = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let comment_translated: Vec<String> = lines
        .iter()
        .map(|line| {
            let l: &str = line;
            let bytes = l.as_bytes();
            let mut in_string = false;
            let mut escape = false;
            for (i, c) in l.char_indices() {
                if escape {
                    escape = false;
                    continue;
                }
                match c {
                    '\\' => escape = true,
                    '"' => in_string = !in_string,
                    '#' if !in_string => {
                        let next = bytes.get(i + 1).copied();
                        let next_is_map = next == Some(b'{');
                        let next_ident = next.map(is_ident_byte).unwrap_or(false);
                        let prev_ident = i > 0 && is_ident_byte(bytes[i - 1]);
                        let is_record_or_map = next_is_map || (prev_ident && next_ident);
                        if !is_record_or_map {
                            return format!("{}%{}", &l[..i], &l[i + 1..]);
                        }
                    }
                    _ => {}
                }
            }
            l.to_string()
        })
        .collect();
    let lines: Vec<&str> = comment_translated.iter().map(|s| s.as_str()).collect();

    // Pre-process step 2: split lines with inline % comments so the comment
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

    // Normalize pattern-match case-arm headers. The body processor's
    // existing arm-frame logic recognizes the framec-generated
    // boolean-case shape: `true ->` (first arm) and `; false ->` /
    // `; _ ->` (subsequent arms). User-written Erlang `case ... of`
    // blocks use bare patterns separated by trailing `;` on each
    // arm body, e.g.
    //
    //     case X of
    //         {true, V} -> body1;
    //         false -> body2
    //     end
    //
    // The body processor needs to see the canonical "next-arm
    // separator at line start" form to drive its CaseFrame
    // SSA-reset logic per arm. Walk the lines tracking case
    // nesting depth and a `first_arm_pending` flag per nesting
    // level; on a non-first arm header, prepend `; ` so it
    // matches the existing detection. Function clauses (lines
    // containing `({call, From},`) are not arm headers and
    // pass through untouched.
    let preprocessed: Vec<String> = {
        let mut out: Vec<String> = Vec::with_capacity(preprocessed.len());
        let mut case_depth: i32 = 0;
        let mut first_arm_pending: Vec<bool> = Vec::new();
        for line in preprocessed.iter() {
            let t = line.trim();
            let is_case_open = (t.starts_with("case ") || t.starts_with("case("))
                && (t.ends_with(" of") || t.ends_with(" of,"));
            let is_case_close = t == "end" || t == "end," || t == "end;";
            let looks_like_arm_header = case_depth > 0
                && (t.ends_with(" ->") || t.ends_with("->"))
                && !t.starts_with("case ")
                && !t.starts_with("case(")
                && !t.starts_with(";")
                && !t.contains("({call, From},")
                // The framec-generated boolean form already has `true ->`
                // as the first arm; leave it untouched. Subsequent
                // boolean arms (`; false ->` / `; _ ->`) start with `;`
                // so they're already filtered above.
                && t != "true ->";
            if is_case_open {
                out.push(line.clone());
                case_depth += 1;
                first_arm_pending.push(true);
                continue;
            }
            if is_case_close {
                if case_depth > 0 {
                    case_depth -= 1;
                    first_arm_pending.pop();
                }
                out.push(line.clone());
                continue;
            }
            if looks_like_arm_header {
                let is_first = first_arm_pending.last().copied().unwrap_or(true);
                if is_first {
                    if let Some(slot) = first_arm_pending.last_mut() {
                        *slot = false;
                    }
                    out.push(line.clone());
                } else {
                    // Prepend `; ` to make this arm header match the
                    // body processor's `; <pattern> ->` recognition.
                    let leading_ws_len = line.len() - line.trim_start().len();
                    let indent: String = line[..leading_ws_len].to_string();
                    out.push(format!("{}; {}", indent, t));
                }
                continue;
            }
            out.push(line.clone());
        }
        out
    };

    // Pre-process: split lines of the form `LHS = <prefix> self.<iface>(args) <suffix>`
    // into a temp-bind + an assignment whenever the call is embedded
    // inside a larger expression on the RHS. Without this the
    // InterfaceCall classifier captures only the LHS and call args,
    // silently dropping the surrounding arithmetic. For
    // `__ReturnVal = self.compute() + 1` we'd lose the `+ 1` and the
    // reply value would be wrong. Iterates so cases like
    // `LHS = self.a() + self.b()` get fully decomposed.
    //
    // Skipped when the LHS starts with `self.` — those are
    // record-update forms handled by `InterfaceCallWithBind` /
    // `ActionCallWithBind`, which already split correctly.
    let preprocessed: Vec<String> = if interface_names.is_empty() {
        preprocessed
    } else {
        let mut out: Vec<String> = preprocessed;
        let mut tmp_idx: usize = 0;
        let mut iter_guard = 0;
        loop {
            iter_guard += 1;
            if iter_guard > 64 {
                break;
            }
            let mut next: Vec<String> = Vec::with_capacity(out.len());
            let mut changed = false;
            for line in out.iter() {
                let leading_ws_len = line.len() - line.trim_start().len();
                let indent = line[..leading_ws_len].to_string();
                let raw = line[leading_ws_len..].to_string();
                // Strip trailing terminator for analysis, preserve to re-attach.
                let trail_start = raw
                    .rfind(|c: char| c != ',' && c != ';' && !c.is_whitespace())
                    .map(|p| p + raw[p..].chars().next().unwrap().len_utf8())
                    .unwrap_or(raw.len());
                let analyze = raw[..trail_start].to_string();
                let trailing = raw[trail_start..].to_string();

                // `self.<field> = self.<iface>(<args>)` — bare-call form
                // — is handled by InterfaceCallWithBind. Skip those.
                // Mixed expressions like `self.<field> = self.f() <op> X`
                // need the pre-pass because InterfaceCallWithBind uses
                // rfind(')') and would grab the wrong closing paren when
                // the RHS has multiple call/paren tokens.
                //
                // Bare-call detection is via paren-balance: find the
                // first `self.<iface>(`, walk forward matching parens,
                // and the bare form requires the matching `)` to be the
                // last meaningful char in the RHS. Surface checks like
                // `rhs.ends_with(")")` are too loose because
                // `self.f() + self.g()` also ends with `)`.
                if analyze.starts_with("self.") {
                    if let Some(eq_pos) = analyze.find('=') {
                        let lhs = analyze[..eq_pos].trim();
                        let rhs = analyze[eq_pos + 1..].trim();
                        let lhs_is_simple_field = lhs
                            .strip_prefix("self.")
                            .map(|f| !f.is_empty() && !f.contains('.') && !f.contains('('))
                            .unwrap_or(false);
                        let rhs_is_bare_iface_call = if !lhs_is_simple_field {
                            false
                        } else {
                            interface_names.iter().any(|iface| {
                                let pat = format!("self.{}(", iface);
                                if !rhs.starts_with(&pat) {
                                    return false;
                                }
                                let bytes = rhs.as_bytes();
                                let open_idx = pat.len() - 1;
                                let mut depth = 0i32;
                                let mut close = open_idx;
                                for i in open_idx..bytes.len() {
                                    match bytes[i] {
                                        b'(' => depth += 1,
                                        b')' => {
                                            depth -= 1;
                                            if depth == 0 {
                                                close = i;
                                                break;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                // Bare form: the matching `)` is the
                                // last char of the RHS (no operator,
                                // no second call after it).
                                close == bytes.len() - 1
                            })
                        };
                        if lhs_is_simple_field && rhs_is_bare_iface_call {
                            next.push(line.clone());
                            continue;
                        }
                    }
                    // Otherwise fall through — the pre-pass below will
                    // hoist any embedded self-calls out of the RHS so
                    // the final RHS is bare and the line classifies
                    // as RecordUpdate.
                }

                // Must be an assignment with `=` (not `==`).
                let eq_pos = match analyze.find('=') {
                    Some(p) if !analyze[p..].starts_with("==") => p,
                    _ => {
                        next.push(line.clone());
                        continue;
                    }
                };
                let rhs_start = eq_pos + 1;

                // Find leftmost `self.<iface>(` in RHS.
                let mut earliest: Option<(usize, usize)> = None;
                for iface in interface_names {
                    let pat = format!("self.{}(", iface);
                    if let Some(rel) = analyze[rhs_start..].find(&pat) {
                        let abs = rhs_start + rel;
                        if earliest.map_or(true, |(prev, _)| abs < prev) {
                            earliest = Some((abs, pat.len()));
                        }
                    }
                }
                let (pat_start, pat_len) = match earliest {
                    Some(v) => v,
                    None => {
                        next.push(line.clone());
                        continue;
                    }
                };

                // Match the closing paren via depth counting.
                let open_paren_idx = pat_start + pat_len - 1;
                let bytes = analyze.as_bytes();
                let mut depth = 0i32;
                let mut close = open_paren_idx;
                for i in open_paren_idx..bytes.len() {
                    match bytes[i] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                close = i;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if close == open_paren_idx {
                    next.push(line.clone());
                    continue;
                }

                let prefix = analyze[rhs_start..pat_start].trim();
                let suffix = analyze[close + 1..].trim();
                if prefix.is_empty() && suffix.is_empty() {
                    // `LHS = self.method(args)` — bare form, classifier handles.
                    next.push(line.clone());
                    continue;
                }

                let lhs = analyze[..eq_pos].trim();
                let call_text = &analyze[pat_start..=close];
                tmp_idx += 1;
                let temp_var = format!("__SelfResult_{}", tmp_idx);
                next.push(format!("{}{} = {}", indent, temp_var, call_text));
                let mut combined = String::new();
                if !prefix.is_empty() {
                    combined.push_str(prefix);
                    combined.push(' ');
                }
                combined.push_str(&temp_var);
                if !suffix.is_empty() {
                    combined.push(' ');
                    combined.push_str(suffix);
                }
                next.push(format!("{}{} = {}{}", indent, lhs, combined, trailing));
                changed = true;
            }
            out = next;
            if !changed {
                break;
            }
        }
        out
    };

    let lines = preprocessed
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<&str>>();

    for line in &lines {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }

        // Erlang `%` comments aren't expressions — emit them verbatim,
        // no separator handling / param-capitalization / classification.
        // (Frame `#` comments were translated to `%` in the pre-process
        // above; an inline one was split onto its own line there.)
        if l.starts_with('%') {
            result.push(format!("    {}", l));
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

        // Subsequent arm of any pattern-match case: `; <pattern> ->`.
        // The pre-processing pass above normalizes user-written
        // `<pattern> ->` second-arms to this canonical form so the
        // existing CaseFrame logic (which already handles `; false`
        // / `; _`) can drive the SSA reset.
        let is_general_subsequent_arm =
            l.starts_with(';') && (l.ends_with(" ->") || l.ends_with("->"));

        // First arm of a pattern-match case: `<pattern> ->` with no
        // leading `;`. The pattern can be anything Erlang accepts —
        // an atom (`other ->`), tuple (`{tag, V} ->`), list, var.
        // Detected by trailing ` ->` and not matching any of the
        // other structural shapes. `{next_state,…}` / `{keep_state,…}`
        // tuples don't end with ` ->`, so they're naturally excluded.
        let is_general_first_arm = !l.starts_with(';')
            && !l.starts_with("case ")
            && !l.starts_with("case(")
            && !is_forward_call
            && (l.ends_with(" ->") || l.ends_with("->"));

        let is_structural = l.starts_with("case ")
            || l.starts_with("case(")
            || l.starts_with("true ->")
            || l.starts_with("; false")
            || is_general_subsequent_arm
            || is_general_first_arm
            || l == "end"
            || l == "end,"
            || l.starts_with("{next_state,")
            || l.starts_with("{keep_state,")
            || l.starts_with("{stop,")
            || l.starts_with("[__Popped")
            || l.starts_with("frame_transition__(")
            || l.starts_with("frame_forward_transition__(")
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
            if l.starts_with("true ->") || is_general_first_arm {
                case_data_stack.push(CaseFrame {
                    saved_var: data_var.clone(),
                    saved_gen: data_gen,
                    arms: Vec::new(),
                    // The arm header goes into `result` below; the
                    // arm body starts on the NEXT line. result.len()+1
                    // accounts for the about-to-be-pushed header.
                    current_arm_start: result.len() + 1,
                });
            } else if l.starts_with("; false") || l.starts_with("; _") || is_general_subsequent_arm
            {
                if let Some(frame) = case_data_stack.last_mut() {
                    // Record the previous arm's final position: index
                    // of last emitted line (which came before this
                    // `; <pattern> ->`).
                    let last_idx = result.len().saturating_sub(1);
                    let start = frame.current_arm_start;
                    frame.arms.push((start, last_idx, data_gen));
                    // Strip a trailing `,` from the last emitted line —
                    // Erlang case-arm bodies are comma-separated
                    // expressions, but the LAST expression in an arm
                    // is followed by `;` (next arm) or `end`, NOT by
                    // another `,`. The body processor's per-statement
                    // emit appends `,` to assignment lines; on an arm
                    // boundary we drop that trailing comma.
                    if let Some(last_line) = result.last_mut() {
                        if last_line.trim_end().ends_with(',') {
                            let trimmed_end = last_line.trim_end();
                            let new_len = trimmed_end.len() - 1;
                            *last_line = trimmed_end[..new_len].to_string();
                        }
                    }
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
                // Strip trailing `,` from the last arm's last expression
                // — same rationale as on `; <pattern> ->` boundaries.
                if let Some(last_line) = result.last_mut() {
                    if last_line.trim_end().ends_with(',') {
                        let trimmed_end = last_line.trim_end();
                        let new_len = trimmed_end.len() - 1;
                        *last_line = trimmed_end[..new_len].to_string();
                    }
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
                            || t.starts_with("frame_forward_transition__(")
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
                                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
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
                                if !trimmed.ends_with(',') && !trimmed.ends_with(';') {
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
            // Hoist `self.<iface>(...)` calls embedded inside
            // `frame_transition__(...)` / `frame_forward_transition__(...)`
            // arg lists into preceding `frame_dispatch__` binds so the
            // call result reaches the transition as a bound variable
            // instead of falling through to the blanket `self.` →
            // `Data#data.` substitution (which would emit invalid
            // record-field-call syntax `Data#data.method()`). Innermost
            // calls are hoisted first via `rfind`; the loop iterates
            // until no `self.<iface>(` remains.
            if !interface_names.is_empty()
                && (rewritten.starts_with("frame_transition__(")
                    || rewritten.starts_with("frame_forward_transition__("))
            {
                let mut iter_guard = 0;
                while iter_guard < 16 {
                    iter_guard += 1;
                    let mut matched: Option<(String, usize, usize, String)> = None;
                    for iface in interface_names {
                        let pat = format!("self.{}(", iface);
                        if let Some(start) = rewritten.rfind(&pat) {
                            let open = start + pat.len() - 1;
                            let bytes = rewritten.as_bytes();
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
                                let inner_args = rewritten[open + 1..end].to_string();
                                if matched.is_none() {
                                    matched = Some((iface.clone(), start, end + 1, inner_args));
                                }
                            }
                        }
                    }
                    if let Some((iface, start, after_end, inner_args)) = matched {
                        let prev_data = data_var.clone();
                        let data_access_prev = format!("{}#data.", prev_data);
                        let args_subst = replace_outside_strings_and_comments(
                            &inner_args,
                            TargetLanguage::Erlang,
                            &[("self.", data_access_prev.as_str())],
                        );
                        let args_list = if args_subst.trim().is_empty() {
                            "[]".to_string()
                        } else {
                            format!("[{}]", args_subst)
                        };
                        data_gen += 1;
                        let new_var = format!("Data{}", data_gen);
                        let result_name = format!("__SelfResult_{}", data_gen);
                        let method_snake = to_snake_case(&iface);
                        result.push(format!(
                            "    {{{}, {}}} = frame_dispatch__({}, {}, {})",
                            new_var, result_name, method_snake, args_list, prev_data
                        ));
                        rewritten = format!(
                            "{}{}{}",
                            &rewritten[..start],
                            result_name,
                            &rewritten[after_end..]
                        );
                        data_var = new_var;
                    } else {
                        break;
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
                let data_update = format!("{}#data{{", data_var);
                rewritten = replace_outside_strings_and_comments(
                    &rewritten,
                    TargetLanguage::Erlang,
                    &[
                        (", Data,", data_comma.as_str()),
                        (", Data}", data_brace.as_str()),
                        (", Data)", data_paren.as_str()),
                        ("Data#data.", data_access2.as_str()),
                        // Record-update syntax `Data#data{field = val}`.
                        // Without this substitution, a structural line
                        // that emits a record update (e.g. `pop$` transit
                        // emission) would read the pre-handler Data
                        // snapshot, discarding any `self.x = ...` writes
                        // that bumped `data_var` before this line.
                        ("Data#data{", data_update.as_str()),
                    ],
                );
            }
            if is_forward_call {
                // Rewrite forward call into a bind so post-forward statements
                // can run. Parent's handler may transition; frame_unwrap_forward__
                // extracts the updated Data, either `undefined` (parent kept
                // state) or the next-state atom (parent transitioned), and
                // the parent's reply value (from `[{reply, From, V}]`).
                // Capture the reply as a `__ReturnVal` write so the existing
                // return-value SSA pass picks it up as the child's reply
                // (otherwise the child would hardcode `ok` and drop the
                // parent's `@@:return` value across the forward).
                let stripped = rewritten.trim_end_matches([',', ';']).trim().to_string();
                data_gen += 1;
                let new_var = format!("Data{}", data_gen);
                let fwd_var = format!("__FwdNext{}", data_gen);
                let reply_var = format!("__FwdReply{}", data_gen);
                result.push(format!(
                    "    {{{}, {}, {}}} = frame_unwrap_forward__({})",
                    new_var, fwd_var, reply_var, stripped
                ));
                // Treat the captured reply as a `@@:return` write so the
                // SSA pass renames this to `__ReturnVal_K` and the handler's
                // terminal reply uses the parent's value, not `ok`.
                result.push(format!("    __ReturnVal = {}", reply_var));
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

        // RFC-0019: `=> $^` inside a `$>` / `<$` handler lowered to a
        // call to the parent's lifecycle helper (`frame_enter__<P>(Data)
        // -> Data` / `frame_exit__<P>(Data) -> Data`). Bind the returned
        // record into a fresh `DataN` and thread it forward so any
        // post-forward statements in the handler see the parent's
        // updates. The helper's single argument is the literal `Data`
        // (emitted by frame_expansion.rs) — point it at the current
        // `data_var`.
        if l.starts_with("frame_enter__") || l.starts_with("frame_exit__") {
            let mut call = l.trim_end_matches([',', ';']).trim().to_string();
            if data_var != "Data" {
                if let Some(prefix) = call.strip_suffix("(Data)") {
                    call = format!("{}({})", prefix, data_var);
                }
            }
            data_gen += 1;
            let new_var = format!("Data{}", data_gen);
            result.push(format!("    {} = {}", new_var, call));
            data_var = new_var;
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
            ErlangRewrite::ActionCallWithBind { field, call } => {
                // `self.<field> = self.<action>(args)` — emit the call-
                // bind first, then apply the result to the record field
                // in a follow-on DataN+1 binding. Two data_gen bumps.
                data_gen += 1;
                let call_var = format!("Data{}", data_gen);
                let result_name = format!("__ActionResult{}", data_gen);
                result.push(format!("    {{{}, {}}} = {}", call_var, result_name, call));
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
                                    let inner_args = args_rewritten[open + 1..end].to_string();
                                    matched = Some((iface.clone(), start, end + 1, inner_args));
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
                                    let inner_args = args_rewritten[open + 1..end].to_string();
                                    matched = Some((iface.clone(), start, end + 1, inner_args));
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
            t.starts_with('{') && t.contains(", __ReturnVal}") && t.contains(" = ")
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
                            let hidden =
                                line.replacen(", __ReturnVal}", &format!(", {}}}", sentinel), 1);
                            let rewritten = replace_outside_strings_and_comments(
                                &hidden,
                                TargetLanguage::Erlang,
                                &[("__ReturnVal", prev_name.as_str())],
                            );
                            *line =
                                rewritten.replace(&format!(", {}}}", sentinel), ", __ReturnVal}");
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
                        *line = line.replacen(", __ReturnVal}", &format!(", {}}}", new_name), 1);
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

    // Final pass: any remaining literal `__ReturnVal` token in a
    // `frame_transition__(...)` argument position is unresolved
    // (no preceding @@:return write reached it via SSA rename).
    // Substitute `ok` so the call has a valid reply value. This
    // closes the case where the handler transitions without ever
    // setting a return value — the gen_statem default.
    for line in result.iter_mut() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("frame_transition__(") && line.contains(", __ReturnVal)") {
            *line = line.replace(", __ReturnVal)", ", ok)");
        }
    }

    (result, data_var, final_rv_name)
}
