//! Call-site argument parser for `@@SystemName(args)` tagged instantiations.
//!
//! When the user writes `@@Robot($(10), $>(80), "R2D2")` in native code,
//! Frame's assembler intercepts the tagged instantiation, parses the
//! args via this module, validates them against the system's declared
//! params, substitutes any omitted defaults, and emits a canonical
//! positional argument list to the target language.
//!
//! ## Two surface forms
//!
//! Each call must use exactly one form for all of its args. Mixing
//! positional and named within a single call is rejected.
//!
//! ### Positional sigil form
//!
//! ```text
//! @@Robot($(10), $>(80), "R2D2")
//! ```
//!
//! - `$(value)`        — state arg, positional
//! - `$>(value)`       — enter arg, positional
//! - bare `value`      — domain arg, positional
//!
//! Order at the call site must match the declaration order in the
//! system header. The validator catches mismatches.
//!
//! ### Named form
//!
//! ```text
//! @@Robot($(x=10), $>(battery=80), name="R2D2")
//! ```
//!
//! - `$(name=value)`   — state arg, named
//! - `$>(name=value)`  — enter arg, named
//! - `name=value`      — domain arg, named
//!
//! Order is irrelevant; the resolver matches each arg to the declared
//! param by name. Defaults can be omitted.
//!
//! ## What this module does NOT do
//!
//! - It does not know about target languages. Output is a canonical
//!   positional `Vec<String>` of value expressions in declaration
//!   order, ready to be wrapped in target syntax by `generate_constructor`.
//! - It does not interpret the value expressions. Each value is captured
//!   as raw target-language text from the user's source.
//! - It does not handle the `@@SystemName(` outer wrapping — that's the
//!   assembler's job. This module receives just the inside of the parens.

use crate::frame_c::compiler::frame_ast::{ParamKind, SystemParam};

// ============================================================================
// Parsed call-site arguments
// ============================================================================

/// Which group an argument targets at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallArgGroup {
    /// `$(...)` — state arg (compartment.state_args)
    State,
    /// `$>(...)` — enter arg (compartment.enter_args)
    Enter,
    /// bare value or `name=value` — domain arg (constructor parameter)
    Domain,
}

/// A single parsed call-site argument.
///
/// `value` is the raw target-language text of the value expression
/// (e.g. `"42"`, `"\"hello\""`, `"compute(x, 2)"`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCallArg {
    pub group: CallArgGroup,
    /// `Some(name)` for named form, `None` for positional form.
    pub name: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallArgsError {
    /// The args text is malformed at the parser level (unbalanced
    /// parens, unexpected token, unterminated string, etc.).
    ParseError { message: String, position: usize },
    /// The user mixed positional and named forms inside a single call.
    MixedForms { message: String },
    /// The system has state or enter params but the call uses bare
    /// positional values (no sigils) instead of `$(...)` / `$>(...)`.
    SigilsRequired { message: String },
    /// Positional form but the count or order doesn't match the
    /// declaration.
    PositionalMismatch { message: String },
    /// Named form but a referenced name doesn't exist on the system.
    UnknownNamedArg { name: String },
    /// A required (no-default) param wasn't provided.
    MissingArg { name: String },
    /// More args supplied than the system declares.
    ExtraArgs { count: usize },
    /// A duplicate name in the named form.
    DuplicateNamedArg { name: String },
}

// ============================================================================
// Single-pass parser
// ============================================================================

/// Parse the contents of `@@SystemName(...)`. The input is the text
/// between the outer parens, exclusive. Returns a flat list of parsed
/// args; the resolver later validates them against the system's
/// declared params.
pub fn parse_call_args(args_text: &str) -> Result<Vec<ParsedCallArg>, CallArgsError> {
    let bytes = args_text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    let mut out = Vec::new();

    loop {
        // Skip whitespace and commas between args
        while i < n
            && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
        {
            i += 1;
        }
        if i >= n {
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }

        let arg_start = i;
        let arg = parse_one_arg(args_text, bytes, &mut i, n)?;
        out.push(arg);

        // After an arg, expect either a comma or end-of-input
        while i < n
            && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
        {
            i += 1;
        }
        if i >= n {
            break;
        }
        if bytes[i] != b',' {
            return Err(CallArgsError::ParseError {
                message: format!(
                    "expected ',' or end of args after argument starting at position {}, got '{}'",
                    arg_start, bytes[i] as char
                ),
                position: i,
            });
        }
    }

    Ok(out)
}

/// Parse one argument starting at `*cursor`. Recognizes `$(...)`,
/// `$>(...)`, and bare-value forms. For each, recognizes both the
/// positional `value` form and the named `name=value` form.
fn parse_one_arg(
    text: &str,
    bytes: &[u8],
    cursor: &mut usize,
    end: usize,
) -> Result<ParsedCallArg, CallArgsError> {
    // Sigil branch — `$(...)` or `$>(...)`
    if bytes[*cursor] == b'$' {
        let is_enter = *cursor + 1 < end && bytes[*cursor + 1] == b'>';
        let open_paren_pos = if is_enter { *cursor + 2 } else { *cursor + 1 };
        if open_paren_pos >= end || bytes[open_paren_pos] != b'(' {
            return Err(CallArgsError::ParseError {
                message: format!("expected '(' after '{}'", if is_enter { "$>" } else { "$" }),
                position: *cursor,
            });
        }
        *cursor = open_paren_pos + 1; // past `$(` or `$>(`

        // Find the matching close paren, capturing the body bytes.
        let body_start = *cursor;
        let body_end = find_matching_close_paren(bytes, *cursor, end)?;
        let body = text[body_start..body_end].trim().to_string();
        *cursor = body_end + 1; // past `)`

        // Parse the body as either `name=value` or bare `value`.
        let (name, value) = split_named_or_positional(&body);

        return Ok(ParsedCallArg {
            group: if is_enter {
                CallArgGroup::Enter
            } else {
                CallArgGroup::State
            },
            name,
            value,
        });
    }

    // Bare branch — `value` or `name=value`. Read the arg up to the
    // top-level comma or end of input. The body is whatever the user
    // wrote, with respect for nested brackets and string literals so a
    // value like `make_thing(1, 2)` doesn't get split on the inner
    // comma.
    let body_start = *cursor;
    let body_end = find_top_level_comma_or_end(bytes, *cursor, end);
    let body = text[body_start..body_end].trim().to_string();
    *cursor = body_end;

    if body.is_empty() {
        return Err(CallArgsError::ParseError {
            message: "empty argument".to_string(),
            position: body_start,
        });
    }

    let (name, value) = split_named_or_positional(&body);
    Ok(ParsedCallArg {
        group: CallArgGroup::Domain,
        name,
        value,
    })
}

/// Inside an arg body (already isolated), detect whether it's a named
/// form (`name=value`) or a positional form (just `value`). For the
/// named form to be recognized, the body must start with a bare
/// identifier followed by an `=` at depth zero (not part of `==`).
fn split_named_or_positional(body: &str) -> (Option<String>, String) {
    let bytes = body.as_bytes();
    let n = bytes.len();
    if n == 0 {
        return (None, String::new());
    }
    let is_ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let is_ident_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

    // Walk leading identifier
    if !is_ident_start(bytes[0]) {
        return (None, body.to_string());
    }
    let mut p = 0;
    while p < n && is_ident_cont(bytes[p]) {
        p += 1;
    }
    let name_end = p;

    // Skip whitespace after the identifier
    while p < n && (bytes[p] == b' ' || bytes[p] == b'\t') {
        p += 1;
    }
    // Need a `=` that is NOT followed by another `=` (i.e., not `==`)
    if p >= n || bytes[p] != b'=' {
        return (None, body.to_string());
    }
    if p + 1 < n && bytes[p + 1] == b'=' {
        return (None, body.to_string());
    }
    let eq_pos = p;

    let name = body[..name_end].to_string();
    let value = body[eq_pos + 1..].trim().to_string();
    (Some(name), value)
}

/// Find the byte index of the matching close paren for an open paren
/// that has already been consumed. Tracks nested parens, brackets,
/// braces, and string literals so it doesn't fool itself.
fn find_matching_close_paren(
    bytes: &[u8],
    start: usize,
    end: usize,
) -> Result<usize, CallArgsError> {
    let mut depth: i32 = 1;
    let mut i = start;
    while i < end {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 1 && bytes[i] == b')' {
                    return Ok(i);
                }
                depth -= 1;
            }
            b'"' => {
                i += 1;
                while i < end && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < end {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                i += 1;
                while i < end && bytes[i] != b'\'' {
                    if bytes[i] == b'\\' && i + 1 < end {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err(CallArgsError::ParseError {
        message: "unbalanced parentheses in call args".to_string(),
        position: start,
    })
}

/// Find the byte index of a top-level comma (depth zero outside of
/// brackets and string literals) or the end of input. Returns the
/// position of the comma, or `end` if no comma found.
fn find_top_level_comma_or_end(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut depth: i32 = 0;
    let mut i = start;
    while i < end {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => return i,
            b'"' => {
                i += 1;
                while i < end && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < end {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            }
            b'\'' => {
                i += 1;
                while i < end && bytes[i] != b'\'' {
                    if bytes[i] == b'\\' && i + 1 < end {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    end
}

// ============================================================================
// Resolver: parsed args + system params → canonical positional list
// ============================================================================

/// Resolve a parsed call against a system's declared params. Validates
/// the call (no mixing, sigils where required, name matching, all
/// required args present) and substitutes Frame-declared defaults for
/// any omitted params.
///
/// Returns a `Vec<String>` of value expressions in **declaration order**:
/// state args first, then enter args, then domain args. The caller can
/// flatten this directly into a target-language constructor call.
pub fn resolve_call(
    parsed: &[ParsedCallArg],
    sys_params: &[SystemParam],
) -> Result<Vec<String>, CallArgsError> {
    // 1. If the system has no params at all, the call must also have
    //    no args. (`@@Empty()` is the only valid form.)
    if sys_params.is_empty() {
        if parsed.is_empty() {
            return Ok(Vec::new());
        }
        return Err(CallArgsError::ExtraArgs {
            count: parsed.len(),
        });
    }

    // 2. Determine whether the system has state or enter params (which
    //    means sigils are required at the call site).
    let needs_sigils = sys_params
        .iter()
        .any(|p| matches!(p.kind, ParamKind::StateArg | ParamKind::EnterArg));

    // 3. Detect mixed forms and overall form.
    let any_named = parsed.iter().any(|a| a.name.is_some());
    let any_positional = parsed.iter().any(|a| a.name.is_none());
    if any_named && any_positional {
        return Err(CallArgsError::MixedForms {
            message: "call mixes positional and named arguments — use one form for the entire call"
                .to_string(),
        });
    }

    // 4. If sigils are required, check that bare-positional values
    //    don't appear where state or enter args belong. We allow bare
    //    domain args (positional or named) regardless.
    if needs_sigils {
        // Walk the parsed args. For each, the user MUST have used a
        // sigil if it targets a state or enter slot. Bare values are
        // only allowed for domain slots.
        // Since the parser sets `group: Domain` for any bare arg, we
        // check by counting: a bare-domain arg in a system that has
        // state/enter params is only allowed if every state and enter
        // slot has been provided via sigils.
        let domain_bare_count = parsed
            .iter()
            .filter(|a| a.group == CallArgGroup::Domain && a.name.is_none())
            .count();
        let state_count = parsed
            .iter()
            .filter(|a| a.group == CallArgGroup::State)
            .count();
        let enter_count = parsed
            .iter()
            .filter(|a| a.group == CallArgGroup::Enter)
            .count();
        let declared_state = sys_params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::StateArg))
            .count();
        let declared_enter = sys_params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::EnterArg))
            .count();
        // For positional form: every declared state/enter slot must be
        // covered by a sigil. We can't mix bare-positional and sigils
        // because positional bare values would be ambiguous about which
        // slot they target.
        if any_positional {
            if state_count != declared_state || enter_count != declared_enter {
                return Err(CallArgsError::SigilsRequired {
                    message: format!(
                        "system has {} state arg(s) and {} enter arg(s) — call must use $(...) and $>(...) sigils for them",
                        declared_state, declared_enter
                    ),
                });
            }
            let _ = domain_bare_count;
        }
        // Named form is OK as long as every state and enter named arg
        // uses a sigil — the parser already encodes that in `group`.
    }

    // 5. Resolve. Two paths: positional or named.
    if any_named {
        resolve_named(parsed, sys_params)
    } else {
        resolve_positional(parsed, sys_params)
    }
}

fn resolve_positional(
    parsed: &[ParsedCallArg],
    sys_params: &[SystemParam],
) -> Result<Vec<String>, CallArgsError> {
    // Walk sys_params in declaration order. For each declared param,
    // the next parsed arg with the matching group is its value.
    // Sigil-tagged args must appear in the same group order as the
    // declaration. Bare domain args fill the trailing domain slots.
    //
    // We use index cursors per group to track which parsed arg is next.
    let parsed_state: Vec<&ParsedCallArg> = parsed
        .iter()
        .filter(|a| a.group == CallArgGroup::State)
        .collect();
    let parsed_enter: Vec<&ParsedCallArg> = parsed
        .iter()
        .filter(|a| a.group == CallArgGroup::Enter)
        .collect();
    let parsed_domain: Vec<&ParsedCallArg> = parsed
        .iter()
        .filter(|a| a.group == CallArgGroup::Domain)
        .collect();

    let mut state_idx = 0;
    let mut enter_idx = 0;
    let mut domain_idx = 0;
    let mut result = Vec::with_capacity(sys_params.len());

    for sp in sys_params {
        let value = match sp.kind {
            ParamKind::StateArg => {
                if let Some(arg) = parsed_state.get(state_idx) {
                    state_idx += 1;
                    arg.value.clone()
                } else if let Some(def) = &sp.default {
                    def.clone()
                } else {
                    return Err(CallArgsError::MissingArg {
                        name: sp.name.clone(),
                    });
                }
            }
            ParamKind::EnterArg => {
                if let Some(arg) = parsed_enter.get(enter_idx) {
                    enter_idx += 1;
                    arg.value.clone()
                } else if let Some(def) = &sp.default {
                    def.clone()
                } else {
                    return Err(CallArgsError::MissingArg {
                        name: sp.name.clone(),
                    });
                }
            }
            ParamKind::Domain => {
                if let Some(arg) = parsed_domain.get(domain_idx) {
                    domain_idx += 1;
                    arg.value.clone()
                } else if let Some(def) = &sp.default {
                    def.clone()
                } else {
                    return Err(CallArgsError::MissingArg {
                        name: sp.name.clone(),
                    });
                }
            }
        };
        result.push(value);
    }

    // Reject extras
    if state_idx < parsed_state.len()
        || enter_idx < parsed_enter.len()
        || domain_idx < parsed_domain.len()
    {
        return Err(CallArgsError::ExtraArgs {
            count: (parsed_state.len() - state_idx)
                + (parsed_enter.len() - enter_idx)
                + (parsed_domain.len() - domain_idx),
        });
    }

    Ok(result)
}

fn resolve_named(
    parsed: &[ParsedCallArg],
    sys_params: &[SystemParam],
) -> Result<Vec<String>, CallArgsError> {
    // Build a name → value lookup, rejecting duplicates.
    use std::collections::HashMap;
    let mut by_name: HashMap<&str, &str> = HashMap::new();
    for arg in parsed {
        let name = match &arg.name {
            Some(n) => n.as_str(),
            None => unreachable!("resolve_named called with positional arg"),
        };
        if by_name.contains_key(name) {
            return Err(CallArgsError::DuplicateNamedArg {
                name: name.to_string(),
            });
        }
        by_name.insert(name, arg.value.as_str());
    }

    // Validate every named arg matches a declared param of the right
    // group. We also check the group: a `$(x=10)` arg can only target
    // a declared state param named `x`, etc.
    for arg in parsed {
        let name = arg.name.as_deref().unwrap();
        let declared = sys_params.iter().find(|p| p.name == name);
        match declared {
            None => {
                return Err(CallArgsError::UnknownNamedArg {
                    name: name.to_string(),
                });
            }
            Some(sp) => {
                let expected_group = match sp.kind {
                    ParamKind::StateArg => CallArgGroup::State,
                    ParamKind::EnterArg => CallArgGroup::Enter,
                    ParamKind::Domain => CallArgGroup::Domain,
                };
                if arg.group != expected_group {
                    return Err(CallArgsError::ParseError {
                        message: format!(
                            "named arg '{}' uses wrong sigil for its declared group",
                            name
                        ),
                        position: 0,
                    });
                }
            }
        }
    }

    // Walk sys_params in declaration order, look up each by name.
    let mut result = Vec::with_capacity(sys_params.len());
    for sp in sys_params {
        let value = match by_name.get(sp.name.as_str()) {
            Some(v) => (*v).to_string(),
            None => match &sp.default {
                Some(d) => d.clone(),
                None => {
                    return Err(CallArgsError::MissingArg {
                        name: sp.name.clone(),
                    });
                }
            },
        };
        result.push(value);
    }

    Ok(result)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::frame_ast::{Span, Type};

    fn make_param(name: &str, kind: ParamKind, default: Option<&str>) -> SystemParam {
        SystemParam {
            name: name.to_string(),
            param_type: Type::Custom("int".to_string()),
            default: default.map(String::from),
            kind,
            span: Span::new(0, 0),
        }
    }

    // ----- Parser tests -----

    #[test]
    fn parses_empty() {
        let parsed = parse_call_args("").unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parses_single_bare_value() {
        let parsed = parse_call_args("42").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].group, CallArgGroup::Domain);
        assert_eq!(parsed[0].name, None);
        assert_eq!(parsed[0].value, "42");
    }

    #[test]
    fn parses_single_state_arg() {
        let parsed = parse_call_args("$(42)").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].group, CallArgGroup::State);
        assert_eq!(parsed[0].name, None);
        assert_eq!(parsed[0].value, "42");
    }

    #[test]
    fn parses_single_enter_arg() {
        let parsed = parse_call_args("$>(80)").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].group, CallArgGroup::Enter);
        assert_eq!(parsed[0].value, "80");
    }

    #[test]
    fn parses_mixed_groups_positional() {
        let parsed = parse_call_args("$(10), $>(80), \"R2D2\"").unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].group, CallArgGroup::State);
        assert_eq!(parsed[0].value, "10");
        assert_eq!(parsed[1].group, CallArgGroup::Enter);
        assert_eq!(parsed[1].value, "80");
        assert_eq!(parsed[2].group, CallArgGroup::Domain);
        assert_eq!(parsed[2].value, "\"R2D2\"");
    }

    #[test]
    fn parses_named_state_arg() {
        let parsed = parse_call_args("$(x=10)").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].group, CallArgGroup::State);
        assert_eq!(parsed[0].name.as_deref(), Some("x"));
        assert_eq!(parsed[0].value, "10");
    }

    #[test]
    fn parses_named_domain_arg() {
        let parsed = parse_call_args("name=\"alice\"").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].group, CallArgGroup::Domain);
        assert_eq!(parsed[0].name.as_deref(), Some("name"));
        assert_eq!(parsed[0].value, "\"alice\"");
    }

    #[test]
    fn parses_named_with_function_call_value() {
        let parsed = parse_call_args("name=make_default(1, 2)").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name.as_deref(), Some("name"));
        assert_eq!(parsed[0].value, "make_default(1, 2)");
    }

    #[test]
    fn parses_value_with_eq_inside_doesnt_become_named() {
        // `(a == b)` has `==`, not a top-level `=`. The body should be
        // treated as a positional value.
        let parsed = parse_call_args("(a == b)").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, None);
        assert_eq!(parsed[0].value, "(a == b)");
    }

    #[test]
    fn parses_string_with_comma_inside() {
        let parsed = parse_call_args("\"hello, world\"").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].value, "\"hello, world\"");
    }

    #[test]
    fn parses_function_call_with_inner_comma() {
        let parsed = parse_call_args("compute(1, 2), other").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].value, "compute(1, 2)");
        assert_eq!(parsed[1].value, "other");
    }

    #[test]
    fn rejects_unbalanced_paren_in_sigil() {
        let err = parse_call_args("$(42").unwrap_err();
        match err {
            CallArgsError::ParseError { .. } => {}
            _ => panic!("expected ParseError"),
        }
    }

    // ----- Resolver tests -----

    #[test]
    fn resolves_pure_domain_positional() {
        let parsed = parse_call_args("10, \"R2D2\"").unwrap();
        let sys = vec![
            make_param("count", ParamKind::Domain, None),
            make_param("name", ParamKind::Domain, None),
        ];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(resolved, vec!["10".to_string(), "\"R2D2\"".to_string()]);
    }

    #[test]
    fn resolves_pure_domain_with_default() {
        let parsed = parse_call_args("").unwrap();
        let sys = vec![make_param("count", ParamKind::Domain, Some("5"))];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(resolved, vec!["5".to_string()]);
    }

    #[test]
    fn resolves_state_arg_with_sigil() {
        let parsed = parse_call_args("$(42)").unwrap();
        let sys = vec![make_param("x", ParamKind::StateArg, None)];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(resolved, vec!["42".to_string()]);
    }

    #[test]
    fn resolves_mixed_groups_positional_in_declaration_order() {
        let parsed = parse_call_args("$(10), $>(80), \"R2D2\"").unwrap();
        let sys = vec![
            make_param("x", ParamKind::StateArg, None),
            make_param("battery", ParamKind::EnterArg, None),
            make_param("name", ParamKind::Domain, None),
        ];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(
            resolved,
            vec!["10".to_string(), "80".to_string(), "\"R2D2\"".to_string()]
        );
    }

    #[test]
    fn resolves_named_form() {
        let parsed = parse_call_args("$(x=10), $>(battery=80), name=\"R2D2\"").unwrap();
        let sys = vec![
            make_param("x", ParamKind::StateArg, None),
            make_param("battery", ParamKind::EnterArg, None),
            make_param("name", ParamKind::Domain, None),
        ];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(
            resolved,
            vec!["10".to_string(), "80".to_string(), "\"R2D2\"".to_string()]
        );
    }

    #[test]
    fn resolves_named_form_out_of_order() {
        // Named args can be in any order; resolver matches by name.
        let parsed = parse_call_args("name=\"R2D2\", $>(battery=80), $(x=10)").unwrap();
        let sys = vec![
            make_param("x", ParamKind::StateArg, None),
            make_param("battery", ParamKind::EnterArg, None),
            make_param("name", ParamKind::Domain, None),
        ];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        // Resolved is ALWAYS in declaration order regardless of call order
        assert_eq!(
            resolved,
            vec!["10".to_string(), "80".to_string(), "\"R2D2\"".to_string()]
        );
    }

    #[test]
    fn resolves_named_with_omitted_default() {
        let parsed = parse_call_args("name=\"alice\"").unwrap();
        let sys = vec![
            make_param("count", ParamKind::Domain, Some("5")),
            make_param("name", ParamKind::Domain, None),
        ];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(resolved, vec!["5".to_string(), "\"alice\"".to_string()]);
    }

    #[test]
    fn rejects_mixed_forms() {
        let parsed = parse_call_args("10, name=\"alice\"").unwrap();
        let sys = vec![
            make_param("count", ParamKind::Domain, None),
            make_param("name", ParamKind::Domain, None),
        ];
        let err = resolve_call(&parsed, &sys).unwrap_err();
        assert!(matches!(err, CallArgsError::MixedForms { .. }));
    }

    #[test]
    fn rejects_missing_required_arg() {
        let parsed = parse_call_args("").unwrap();
        let sys = vec![make_param("required", ParamKind::Domain, None)];
        let err = resolve_call(&parsed, &sys).unwrap_err();
        assert!(matches!(err, CallArgsError::MissingArg { .. }));
    }

    #[test]
    fn rejects_extra_args() {
        let parsed = parse_call_args("1, 2, 3").unwrap();
        let sys = vec![make_param("only", ParamKind::Domain, None)];
        let err = resolve_call(&parsed, &sys).unwrap_err();
        assert!(matches!(err, CallArgsError::ExtraArgs { .. }));
    }

    #[test]
    fn rejects_unknown_named_arg() {
        let parsed = parse_call_args("nonexistent=42").unwrap();
        let sys = vec![make_param("only", ParamKind::Domain, None)];
        let err = resolve_call(&parsed, &sys).unwrap_err();
        assert!(matches!(err, CallArgsError::UnknownNamedArg { .. }));
    }

    #[test]
    fn rejects_duplicate_named_arg() {
        let parsed = parse_call_args("x=1, x=2").unwrap();
        let sys = vec![make_param("x", ParamKind::Domain, None)];
        let err = resolve_call(&parsed, &sys).unwrap_err();
        assert!(matches!(err, CallArgsError::DuplicateNamedArg { .. }));
    }

    #[test]
    fn rejects_bare_positional_when_state_arg_required() {
        // System has a state arg; user must use $(...) sigil.
        let parsed = parse_call_args("42").unwrap();
        let sys = vec![make_param("x", ParamKind::StateArg, None)];
        let err = resolve_call(&parsed, &sys).unwrap_err();
        assert!(matches!(err, CallArgsError::SigilsRequired { .. }));
    }

    #[test]
    fn pure_empty_call() {
        let parsed = parse_call_args("").unwrap();
        let sys: Vec<SystemParam> = vec![];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert!(resolved.is_empty());
    }

    #[test]
    fn pure_empty_call_with_extra_args_rejected() {
        let parsed = parse_call_args("42").unwrap();
        let sys: Vec<SystemParam> = vec![];
        let err = resolve_call(&parsed, &sys).unwrap_err();
        assert!(matches!(err, CallArgsError::ExtraArgs { .. }));
    }

    #[test]
    fn frame_default_substitution_at_call_site() {
        // Q9: Frame substitutes the declared default at the
        // tagged-instantiation expansion site. The user calls
        // @@Counter() and Frame supplies the default `5`.
        let parsed = parse_call_args("").unwrap();
        let sys = vec![make_param("initial", ParamKind::Domain, Some("5"))];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(resolved, vec!["5".to_string()]);
    }

    #[test]
    fn frame_default_substitution_with_some_provided() {
        // User provides one of two domain params; the second falls
        // back to the declared default.
        let parsed = parse_call_args("\"alice\"").unwrap();
        let sys = vec![
            make_param("name", ParamKind::Domain, None),
            make_param("age", ParamKind::Domain, Some("0")),
        ];
        let resolved = resolve_call(&parsed, &sys).unwrap();
        assert_eq!(resolved, vec!["\"alice\"".to_string(), "0".to_string()]);
    }
}
