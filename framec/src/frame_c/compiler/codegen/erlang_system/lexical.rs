//! Lexical / text-rewriting helpers for the Erlang code generator.
//!
//! Stateless string utilities consumed by the per-handler body
//! processors and the smart-join emitter. Grouped together so the
//! reader doesn't have to scroll past 200 LOC of low-level string
//! pawing to find the per-handler logic in `erlang_system.rs`.
//!
//! Three rough categories:
//!
//! - **Word-boundary search/replace** (`replace_word`,
//!   `replace_whole_word`, `raw_contains_word`) — match an
//!   identifier only when surrounded by non-word characters.
//! - **Identifier rewrites** (`erlang_op_name`,
//!   `erlang_safe_capitalize`) — translate Frame names into
//!   Erlang's atom/variable grammar, dodging gen_statem reserved
//!   names.
//! - **Line analysis** (`paren_balance_unclosed`,
//!   `ends_with_binary_op`, `split_top_level_commas`) — answer
//!   structural questions about a processed line so the smart-join
//!   emitter inserts the right separator (comma vs newline-only).
//!
//! Plus `expand_system_instantiation_in_domain_erlang` — domain
//! initializer expansion for nested `@@SystemName(args)` references,
//! mirroring the cross-target `expand_system_instantiation_in_domain`
//! in `system_codegen` but lowering to Erlang's `name:create(args)`
//! shape (RFC-0017 Phase A6 factory call).

use super::super::codegen_utils::to_snake_case;

/// Word-boundary string substitution. Replaces `needle` with `replacement`
/// only when `needle` appears as a complete identifier (surrounded by
/// non-word chars or string boundaries). Used to substitute Frame param
/// names with their capitalized Erlang variable names in domain field
/// initializer expressions.
pub(super) fn replace_word(haystack: &str, needle: &str, replacement: &str) -> String {
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
pub(super) fn erlang_op_name(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        None => String::new(),
        Some(c) if c.is_ascii_uppercase() => c.to_ascii_lowercase().to_string() + chars.as_str(),
        // Frame allows `_<name>` for "private/internal" actions and
        // operations (a Python/JS convention). Erlang's bare-atom
        // grammar rejects identifiers that begin with `_` — those
        // are reserved for ignored bindings. Quote the atom so the
        // user's name is preserved verbatim. Both the function
        // declaration and call sites route through this helper, so
        // the quoted form propagates uniformly.
        Some('_') => format!("'{}'", name),
        Some(_) => name.to_string(),
    }
}

/// Word-boundary substring search. Returns true iff `needle` appears in
/// `haystack` as a complete identifier (surrounded by non-word chars or
/// string boundaries). Used to detect whether a domain field's raw
/// initializer references a system param by name.
pub(super) fn raw_contains_word(haystack: &str, needle: &str) -> bool {
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

/// Replace whole-word occurrences of `word` with `replacement` in `s`.
/// Word boundaries are non-alphanumeric / non-underscore. Used by the
/// chained-transition emitter to capitalize state-arg references
/// (`x` → `X`) inside Erlang transition-arg expressions.
pub(super) fn replace_whole_word(s: &str, word: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let word_bytes = word.as_bytes();
    let is_word_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < bytes.len() {
        if i + word_bytes.len() <= bytes.len()
            && &bytes[i..i + word_bytes.len()] == word_bytes
            && (i == 0 || !is_word_char(bytes[i - 1]))
            && (i + word_bytes.len() == bytes.len() || !is_word_char(bytes[i + word_bytes.len()]))
        {
            out.push_str(replacement);
            i += word_bytes.len();
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Split a string on top-level commas, respecting nesting in
/// `(`/`)`, `[`/`]`, and `{`/`}`. Used by the chained-transition
/// emitter to parse a `frame_transition__(target, Data, exits,
/// enters, state_args, From, ReplyVal)` argument list when the
/// state_args list itself contains commas (multi-arg patterns
/// like `[X, Y]`).
pub(super) fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                buf.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                buf.push(c);
            }
            ',' if depth == 0 => {
                out.push(buf.trim().to_string());
                buf.clear();
            }
            _ => buf.push(c),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

/// Capitalize a parameter name for Erlang, avoiding collisions with
/// gen_statem reserved names. `"data"` → `"Data_Arg"` (not `"Data"`
/// which collides with the gen_statem state data variable); `"from"`
/// → `"From_Arg"` (not `"From"` which collides with the gen_statem
/// caller reference).
pub(super) fn erlang_safe_capitalize(name: &str) -> String {
    let capitalized = {
        let mut chars = name.chars();
        match chars.next() {
            None => String::new(),
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        }
    };
    if matches!(
        capitalized.as_str(),
        "Data" | "From" | "State" | "OldState" | "Pid"
    ) {
        format!("{}_Arg", capitalized)
    } else {
        capitalized
    }
}

/// True if `line` has more open parens/brackets/braces than closes —
/// meaning the line is in the middle of an expression that continues
/// on subsequent lines. String/atom-quote and comment regions are
/// excluded from the count.
pub(super) fn paren_balance_unclosed(line: &str) -> bool {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut in_atom = false;
    let mut escape = false;
    for c in line.chars() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && (in_string || in_atom) {
            escape = true;
            continue;
        }
        if c == '"' && !in_atom {
            in_string = !in_string;
            continue;
        }
        if c == '\'' && !in_string {
            in_atom = !in_atom;
            continue;
        }
        if in_string || in_atom {
            continue;
        }
        if c == '%' {
            break;
        }
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

/// True if `line` ends with an Erlang binary operator that requires a
/// right operand on a subsequent line. Such a line is mid-expression;
/// the next line is the operand and must NOT be separated by `,`.
pub(super) fn ends_with_binary_op(line: &str) -> bool {
    let t = line.trim_end();
    // Word operators: must be preceded by whitespace (or start-of-line)
    // to avoid matching identifier suffixes like `foo_andalso`.
    for op in &[
        "andalso", "orelse", "and", "or", "xor", "not", "div", "rem", "band", "bor", "bxor",
        "bnot", "bsl", "bsr",
    ] {
        if t.ends_with(op) {
            let before_op_len = t.len() - op.len();
            if before_op_len == 0 {
                return true;
            }
            let preceding = t.as_bytes()[before_op_len - 1];
            if preceding == b' ' || preceding == b'\t' {
                return true;
            }
        }
    }
    for op in &[
        "+", "-", "*", "/", "++", "--", "=:=", "=/=", "==", "/=", "<", ">", "=<", ">=", "->", "=>",
        "|", "||", "::",
    ] {
        if t.ends_with(op) {
            // A `>` that closes a binary construction `<<...>>` is NOT a
            // binary op. Filter that out.
            if *op == ">" && t.ends_with(">>") {
                continue;
            }
            return true;
        }
    }
    false
}

/// Lower `@@Name(args)` references inside domain initializer text to
/// Erlang's RFC-0017 factory call `name:create(args)` (returns bare
/// Pid). Bare `@@Name` (no args) lowers to the snake-cased module
/// atom only.
pub(super) fn expand_system_instantiation_in_domain_erlang(text: &str) -> String {
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
                // Find matching `)` so we wrap exactly the
                // `name:start_link(...)` expression in the
                // `element(2, ...)` unwrap. Walk paren depth
                // because user args may themselves contain parens.
                let bytes = result.as_bytes();
                let mut depth: i32 = 1;
                let mut p = name_end + 1;
                while p < bytes.len() && depth > 0 {
                    match bytes[p] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    p += 1;
                }
                let args = &result[name_end + 1..p.saturating_sub(1)];
                let tail = &result[p..];
                // RFC-0017 Phase A6: factory call lowers to `name:create(args)`
                // which returns a bare Pid directly.
                result = format!("{}{}:create({}){}", &result[..pos], snake, args, tail);
            } else {
                result = format!("{}{}{}", &result[..pos], snake, &result[name_end..]);
            }
        } else {
            break;
        }
    }
    result
}
