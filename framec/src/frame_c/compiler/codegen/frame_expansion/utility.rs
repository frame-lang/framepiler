//! Stateless string / expression helpers shared across the Frame
//! statement expanders.
//!
//! Grouped here so the reader doesn't have to scroll past several
//! hundred lines of paren-walking and indent-arithmetic to find the
//! per-statement expansion logic in `frame_expansion.rs`. Three
//! rough categories:
//!
//! - **Target-specific marshalling** — `c_return_assign` (C's
//!   `void*` `_return` slot with the float bit-pun fallback),
//!   `cpp_wrap_string_literal` (`std::any` round-trip), and the
//!   PHP `$`-prefix sigil rewriter `php_prefix_params`.
//! - **Source-shape extractors** — `extract_dot_key`,
//!   `extract_state_var_name`, `strip_outer_parens`, and
//!   `split_transition_return` pull substrings out of Frame
//!   source text in a way that's identical across every backend.
//! - **Indent / formatting helpers** — `paren_wrap_if_multiline`
//!   restores implicit line-continuation for indent-sensitive
//!   targets; `strip_java_unreachable` drops dead code after a
//!   terminal `return;` to keep `javac` quiet;
//!   `normalize_indentation` deletes the common leading
//!   whitespace from spliced handler text so generated code lines
//!   up at column 0 before re-indentation downstream.

/// C `_return` assignment with double-aware marshalling.
///
/// The `_return` slot is `void*`. Ints/bools/pointers travel via
/// `(void*)(intptr_t)(val)` cleanly. Doubles don't — `(intptr_t)(42.0)`
/// truncates the fractional part. When the handler's declared return
/// type is `float`/`double`, pack via a memcpy helper the runtime emits
/// (`Sys_pack_double`).
pub(super) fn c_return_assign(
    system_name: &str,
    expanded_expr: &str,
    return_type: &Option<String>,
) -> String {
    let is_dbl = return_type
        .as_deref()
        .map(|t| {
            let t = t.trim();
            t == "float" || t == "double"
        })
        .unwrap_or(false);
    if is_dbl {
        format!(
            "{sys}_CTX(self)->_return = {sys}_pack_double({expr});",
            sys = system_name,
            expr = expanded_expr,
        )
    } else {
        format!(
            "{}_CTX(self)->_return = (void*)(intptr_t)({});",
            system_name, expanded_expr
        )
    }
}

/// Strip the outer parentheses from `(inner)` → `inner`.
///
/// Preconditions (checked by the caller): `s` is non-empty and wrapped
/// in a matching outer `(…)` pair. The three @@:self.method expansion
/// sites use this to unwrap `raw_args_with_parens` before splicing the
/// arg list into a target's native call form (e.g. C's free-function
/// dispatch `Sys_method(self, <inner>)`).
pub(super) fn strip_outer_parens(s: &str) -> &str {
    debug_assert!(
        s.len() >= 2 && s.starts_with('(') && s.ends_with(')'),
        "strip_outer_parens called on non-paren-wrapped input: {:?}",
        s
    );
    &s[1..s.len() - 1]
}

/// Wrap a C++ expression in std::string() if it's a string literal.
/// Prevents std::bad_any_cast when storing in std::any (const char* vs std::string).
pub(super) fn cpp_wrap_string_literal(expr: &str) -> String {
    let trimmed = expr.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') {
        format!("std::string({})", trimmed)
    } else {
        expr.to_string()
    }
}

/// Wrap an expression in `(...)` when it spans multiple source lines.
///
/// Frame's `@@:(<expr>)` and `@@:return = <expr>` sigils carry parens
/// that serve as syntactic markers for the return-value form. The
/// codegen consumes those markers and emits the inner expression as
/// the RHS of an assignment to the context-stack `_return` slot. On
/// indent-sensitive targets (Python, GDScript) a multi-line RHS
/// without grouping parens hits a parse error: the assignment closes
/// at the first newline and the continuation line ("    and ...")
/// becomes an unexpected `Indent`.
///
/// Re-introducing parens around a multi-line RHS restores the
/// implicit line-continuation that those targets require. For
/// curly-brace targets the parens are redundant but harmless. Single-
/// line expressions skip the wrap so the common case stays
/// paren-free.
pub(super) fn paren_wrap_if_multiline(expr: &str) -> String {
    if expr.contains('\n') {
        format!("({})", expr)
    } else {
        expr.to_string()
    }
}

/// Strip code after a terminal `return;` until the enclosing block closes.
/// Java treats code after `return;` as a compile error, unlike TypeScript/C++ which ignore it.
pub(crate) fn strip_java_unreachable(text: &str) -> String {
    let mut result = Vec::new();
    let mut skip = false;
    for line in text.lines() {
        if skip {
            // Stop skipping when we hit a closing brace or another control structure
            let trimmed = line.trim();
            if trimmed.starts_with('}') || trimmed.is_empty() {
                skip = false;
                // Don't include the empty lines that were between return and next code
                if !trimmed.is_empty() {
                    result.push(line.to_string());
                }
            }
            // Skip all other lines (unreachable code)
            continue;
        }
        result.push(line.to_string());
        // Check if this line ends with a terminal return
        let trimmed = line.trim();
        if trimmed == "return;" || trimmed == "return" {
            skip = true;
        }
    }
    result.join("\n")
}

/// Normalize indentation by removing common leading whitespace from all lines
pub(crate) fn normalize_indentation(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    // Find minimum indentation (ignoring empty lines)
    let min_indent = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    // Strip the common indentation from all lines
    lines
        .iter()
        .map(|line| {
            if line.len() >= min_indent {
                &line[min_indent..]
            } else {
                line.trim()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Extract bracketed key from syntax like "@@:data[key]" or "@@:params[key]"
/// Returns the raw content between [ and ] — including any user-supplied quotes.
/// For languages that need a bare key (C, Rust), call .trim_matches on the result.
/// Extract the key from a dot-accessor: `@@:params.key` → `key`
pub(crate) fn extract_dot_key(text: &str, prefix: &str) -> String {
    if let Some(rest) = text.strip_prefix(prefix) {
        if let Some(rest) = rest.strip_prefix('.') {
            // Extract only the identifier (alphanumeric + underscore)
            let key: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            return key;
        }
    }
    "".to_string()
}

/// Extract state variable name from "$.varName"
pub(crate) fn extract_state_var_name(text: &str) -> String {
    // Skip "$." prefix and get identifier
    if text.starts_with("$.") {
        let after_prefix = &text[2..];
        let end = after_prefix
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(after_prefix.len());
        after_prefix[..end].to_string()
    } else {
        "unknown".to_string()
    }
}

/// Split a transition expansion into `(body, trailing_return)`.
///
/// Transition expansions always end with `return` or `return;` to exit
/// the handler after the state change. The orchestrator needs these
/// separated so it can insert a return-expr between the body and the
/// return when `-> $State` is followed by `@@:(expr)` in the same scope.
pub(super) fn split_transition_return(expansion: &str) -> (&str, &str) {
    let trimmed = expansion.trim_end();
    if trimmed.ends_with("return;") {
        (trimmed[..trimmed.len() - 7].trim_end(), "return;")
    } else if trimmed.ends_with("return") {
        (trimmed[..trimmed.len() - 6].trim_end(), "return")
    } else {
        // Expansion doesn't end with return (e.g., Rust uses different
        // control flow, or Graphviz). Emit as-is.
        (trimmed, "")
    }
}

/// PHP requires a `$` sigil on every variable reference. Handler param names
/// like `count` appear as bare words in user-authored Frame expressions; in
/// generated PHP where it's interpreted as an undefined constant. This walks
/// the expression outside of string literals and rewrites bare-word occurrences
/// of known handler params to `$param`.
///
/// Safe under:
///   - string literals (skipped)
///   - already-prefixed `$foo` (not doubled)
///   - method/property access `->foo`, `::foo` (not prefixed — `foo` is a
///     member name, not a variable)
///   - member calls on `$this` (already has `$`)
pub(crate) fn php_prefix_params(expr: &str, params: &[String]) -> String {
    if params.is_empty() {
        return expr.to_string();
    }
    // Delegate string-literal and comment skipping to the PHP skipper
    // rather than re-implementing quote tracking inline. Only the
    // identifier-walking + param-matching logic below is PHP-specific
    // to this transformation.
    let skipper = crate::frame_c::compiler::native_region_scanner::create_skipper(
        crate::frame_c::visitors::TargetLanguage::Php,
    );
    let bytes = expr.as_bytes();
    let end = bytes.len();
    let mut out = String::with_capacity(expr.len() + 4);
    let mut i = 0;
    while i < end {
        if let Some(next) = skipper.skip_string(bytes, i, end) {
            out.push_str(&expr[i..next]);
            i = next;
            continue;
        }
        if let Some(next) = skipper.skip_comment(bytes, i, end) {
            out.push_str(&expr[i..next]);
            i = next;
            continue;
        }
        let c = bytes[i];
        // Identifier start: lowercase alpha or underscore, not already
        // preceded by `$`, `->`, `::`, or an ident char (i.e. part of a
        // larger token).
        let is_ident_start = (c.is_ascii_lowercase() || c == b'_')
            && !(i > 0 && (bytes[i - 1] == b'$' || is_ident_char(bytes[i - 1])))
            && !(i >= 2 && bytes[i - 1] == b'>' && bytes[i - 2] == b'-')
            && !(i >= 2 && bytes[i - 1] == b':' && bytes[i - 2] == b':');
        if is_ident_start {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let ident = &expr[start..i];
            // Skip PHP keywords
            let is_keyword = matches!(
                ident,
                "true"
                    | "false"
                    | "null"
                    | "and"
                    | "or"
                    | "xor"
                    | "new"
                    | "return"
                    | "if"
                    | "else"
                    | "elseif"
                    | "while"
                    | "for"
                    | "foreach"
                    | "do"
                    | "switch"
                    | "case"
                    | "break"
                    | "continue"
                    | "function"
                    | "class"
                    | "public"
                    | "private"
                    | "protected"
                    | "static"
                    | "use"
                    | "namespace"
                    | "as"
                    | "throw"
                    | "try"
                    | "catch"
                    | "finally"
                    | "instanceof"
            );
            // Next non-space char: if it's `(`, this is a function call,
            // not a variable reference — leave it alone.
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let followed_by_call = j < bytes.len() && bytes[j] == b'(';
            if !is_keyword && !followed_by_call && params.iter().any(|p| p == ident) {
                out.push('$');
            }
            out.push_str(ident);
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}
