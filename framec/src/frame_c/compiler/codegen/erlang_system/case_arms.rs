//! Case-arm structural classification + per-arm gen_statem reply
//! injection.
//!
//! Erlang's gen_statem callbacks must return one of a closed set of
//! tuples — `{keep_state, Data, Actions}` / `{next_state, ..., ...}`
//! / `frame_transition__(...)`. The Frame source's portable
//! conditional shape (`if/else if/else`) lowers to a `case ... of`
//! whose arms may or may not transition. This module classifies
//! each top-level case arm and rewrites the case so every arm
//! produces a valid gen_statem return:
//!
//! - **AllTerminal** — every arm transitions; the case is itself
//!   the handler's terminal return.
//! - **NoTerminal** — no arm transitions; the case body hoists
//!   `__ReturnVal` and the outer handler emits a single
//!   `{keep_state, Data, [{reply, From, __ReturnVal}]}`.
//! - **Mixed** — some arms transition, some don't. Per-arm rewrite:
//!   transition arms keep their `frame_transition__(...)` shape
//!   (with the arm-local `@@:return` spliced into the reply slot);
//!   non-transition arms get a `{keep_state, Data, [{reply, From,
//!   <return val>}]}` tuple injected.
//!
//! `erlang_inject_orphan_reply_tuples` is the depth>1 sibling pass —
//! `rewrite_mixed_case_arms` only descends one level, so nested
//! cases (produced by nested `if/else`) need separate orphan
//! handling for their leaf `__ReturnVal = ...` writes that would
//! otherwise leak bare values into the gen_statem return.

/// Information about a single arm in a case...end block
pub(super) struct CaseArmInfo {
    /// Index of the arm header line (e.g., "true ->" or "; false ->") in processed lines
    pub header_idx: usize,
    /// Line indices of body content (after header, before next arm or end)
    pub body_start: usize,
    pub body_end: usize,
    /// Whether this arm contains a frame_transition__() call
    pub has_transition: bool,
    /// The __ReturnVal expression if one was assigned in this arm
    pub return_val: Option<String>,
    /// The last DataN variable in this arm (for {keep_state, DataN, ...})
    pub final_data_var: Option<String>,
}

/// Classification of a case block's arm behaviors
pub(super) enum CaseBlockClassification {
    /// All arms have frame_transition__() — case is terminal, use as handler return
    AllTerminal,
    /// No arms have frame_transition__() — hoist __ReturnVal, append {keep_state,...}
    NoTerminal,
    /// Mixed: some arms transition, some don't — per-arm rewrite needed
    Mixed,
}

/// Analyze a case block in processed handler lines.
/// Returns (classification, arms, case_start_line, case_end_line).
///
/// When a handler body contains MULTIPLE top-level case blocks
/// (e.g., consecutive `if` statements), this analyzes the LAST
/// one. That's the case which typically contains the handler's
/// terminal — its transition arm or final return value — while
/// earlier cases are just intermediate logic. The rewriter emits
/// pre-case lines verbatim, rewrites the last case in place, and
/// trailing-line emission handles anything after.
pub(super) fn analyze_case_arms(
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
            if depth == 1 {
                // New top-level case begins. If we've already
                // analyzed an earlier sibling case, drop it — only
                // the LAST top-level case is the terminal.
                case_start = Some(idx);
                case_end = None;
                arms.clear();
                current_arm = None;
            }
            continue;
        }

        if t == "end" || t == "end," || t == "end;" {
            if depth == 1 {
                // Close current arm + record case end. Don't break:
                // a sibling case may follow, in which case we'll
                // discard the just-closed analysis on its first
                // header line above.
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
                if t.starts_with("frame_transition__(")
                    || t.starts_with("frame_forward_transition__(")
                {
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

        // Arm boundary detection at depth 1.
        //
        // Recognise both the canonical boolean-case shape (`true ->`,
        // `; false ->`, `; _ ->`) AND bare-pattern arms emitted by
        // `erlang_wrap_self_call_guards` (e.g. `s0 ->`, `_ ->`).
        // The wrap function runs AFTER the body processor's pre-pass
        // that normalises user-written subsequent arms to `; <pat> ->`,
        // so its emitted case can have either bare or `;`-prefixed
        // arms. Both must be detected here or the wrap-emitted case
        // gets misclassified and `rewrite_mixed_case_arms` drops the
        // `; _ ->` / `end` tail.
        let is_canonical_header =
            t.starts_with("true ->") || t.starts_with("; false") || t.starts_with("; _");
        let is_bare_pattern_header = !t.starts_with("case ")
            && !t.starts_with("case(")
            && !t.contains("({call, From},")
            && (t.ends_with(" ->") || t.ends_with("->"))
            && !t.starts_with("{");
        let is_general_semicolon_header =
            t.starts_with("; ") && (t.ends_with(" ->") || t.ends_with("->"));
        let is_arm_header =
            is_canonical_header || is_general_semicolon_header || is_bare_pattern_header;
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
            if t.starts_with("frame_transition__(") || t.starts_with("frame_forward_transition__(")
            {
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
///
/// `default_reply` is the fallback value for non-transition arms that don't
/// have an in-arm `__ReturnVal = …` write — typically the SSA-renamed
/// `__ReturnVal_K` name from a top-level `@@:return` written *before* the
/// case block. Falls back to `"ok"` when no top-level return value exists.
pub(super) fn rewrite_mixed_case_arms(
    processed: &[String],
    arms: &[CaseArmInfo],
    case_start: usize,
    case_end: usize,
    default_data: &str,
    default_reply: &str,
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

            // Splice the arm's captured @@:return value into a
            // transition call's reply slot. The frame_expansion site
            // emits `frame_transition__(..., From, __ReturnVal)`; the
            // SSA pass + transition-finalize fallback resolved that
            // to a top-level SSA name or `ok`. But arm-local
            // `@@:return = X` writes don't reach the SSA pass (they
            // live at depth>0), so the transition call still has
            // `ok`. Substitute the arm-captured value here so a
            // transitioning arm with an in-arm @@:return preserves
            // the value through the gen_statem reply.
            if (t.starts_with("frame_transition__(")
                || t.starts_with("frame_forward_transition__("))
                && nested_depth == 0
            {
                if let Some(rv) = arm.return_val.as_deref() {
                    let line = &processed[i];
                    if line.contains(", ok)") {
                        let rewritten = line.replacen(", ok)", &format!(", {})", rv), 1);
                        result.push(rewritten);
                        continue;
                    }
                }
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
            let arm_has_reply = processed[arm.body_start..arm.body_end].iter().any(|l| {
                let t = l.trim();
                t.starts_with("{keep_state,") || t.starts_with("{next_state,")
            });
            if !arm_has_reply {
                let data = arm.final_data_var.as_deref().unwrap_or(default_data);
                let reply = arm.return_val.as_deref().unwrap_or(default_reply);
                result.push(format!(
                    "        {{keep_state, {}, [{{reply, From, {}}}]}}",
                    data, reply
                ));
            }
        }
    }

    // Emit end
    result.push("    end".to_string());

    // Emit any lines AFTER the case block. A handler with a sibling
    // `if`/`case` after this one (e.g., a non-transitioning `if` with
    // a follow-up transitioning `if`) needs those tail lines preserved
    // — without this, the analyzer's `break` at first-`end` truncates
    // the rewrite output. The original lines have already had their
    // `__ReturnVal` SSA-renamed by the body processor.
    for i in (case_end + 1)..processed.len() {
        result.push(processed[i].clone());
    }

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
pub(super) fn erlang_inject_orphan_reply_tuples(
    lines: &[String],
    default_data: &str,
) -> Vec<String> {
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

        let is_orphan_candidate =
            t.starts_with("__ReturnVal = ") && !t.ends_with(',') && !t.ends_with(';');

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
            || next_trimmed.starts_with("frame_transition__(")
            || next_trimmed.starts_with("frame_forward_transition__(");

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
