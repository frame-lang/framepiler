//! Whole-word matching utilities used by domain-initializer
//! analysis.
//!
//! These helpers exist because Frame's V4 codegen needs to answer two
//! questions about user-written initializer text without parsing it:
//!
//! 1. **Does the initializer reference a constructor parameter?**
//!    `balance: int = balance` is a self-reference; we have to detect
//!    the rebind to avoid emitting a name-collision at init.
//! 2. **Which identifiers in a PHP initializer are param names that
//!    need a `$` sigil?** PHP's lexical rules require `$` on every
//!    variable read, so domain initializers written in language-
//!    agnostic Frame text have to be rewritten on the PHP path.
//!
//! Both questions reduce to "find every whole-word occurrence of
//! `name` in this string". The `extra_leading` parameter to
//! `is_whole_word_at` lets the PHP path treat `$` as already-sigiled
//! (so `$balance` doesn't double-prefix) and lets the param-detection
//! path treat `.` as a valid leading boundary (so `obj.balance`
//! doesn't trigger on `balance`).

/// Check if a word appears at a whole-word boundary in text.
/// Leading boundary excludes alphanumeric, underscore, and any chars
/// in `extra_leading`. Trailing boundary excludes alphanumeric and
/// underscore.
pub(super) fn is_whole_word_at(
    bytes: &[u8],
    start: usize,
    end: usize,
    extra_leading: &[u8],
) -> bool {
    let prev_ok = start == 0 || {
        let b = bytes[start - 1];
        !(b.is_ascii_alphanumeric() || b == b'_' || extra_leading.contains(&b))
    };
    let next_ok = end >= bytes.len() || {
        let b = bytes[end];
        !(b.is_ascii_alphanumeric() || b == b'_')
    };
    prev_ok && next_ok
}

/// Find all whole-word occurrences of `word` in `text`, calling
/// `callback` for each. The callback receives `(start, end)` byte
/// positions and returns `true` to continue, `false` to stop.
pub(super) fn find_whole_words(
    text: &[u8],
    word: &[u8],
    extra_leading: &[u8],
    mut callback: impl FnMut(usize, usize) -> bool,
) {
    let mut i = 0;
    while i + word.len() <= text.len() {
        if let Some(found) = text[i..].windows(word.len()).position(|w| w == word) {
            let start = i + found;
            let end = start + word.len();
            if is_whole_word_at(text, start, end, extra_leading) {
                if !callback(start, end) {
                    return;
                }
            }
            i = end;
        } else {
            break;
        }
    }
}

/// True iff the init expression text contains any of the supplied
/// param names as a whole word. Used to detect `balance: int = balance`
/// where a domain field initializer references a constructor
/// parameter.
pub(crate) fn init_references_param(init_text: &str, params: &[String]) -> bool {
    if params.is_empty() || init_text.is_empty() {
        return false;
    }
    let bytes = init_text.as_bytes();
    for p in params {
        if p.is_empty() {
            continue;
        }
        let mut found = false;
        find_whole_words(bytes, p.as_bytes(), b".", |_, _| {
            found = true;
            false
        });
        if found {
            return true;
        }
    }
    false
}

/// Prefix `$` to identifiers in `text` that match system param names.
/// Used for PHP domain initializer expressions (e.g.
/// `initial_balance` → `$initial_balance`).
pub(super) fn prefix_php_vars(text: &str, params: &[String]) -> String {
    let mut result = text.to_string();
    for p in params {
        if p.is_empty() {
            continue;
        }
        let mut new_result = String::new();
        let bytes = result.as_bytes();
        let pb = p.as_bytes();
        let mut i = 0usize;
        while i + pb.len() <= bytes.len() {
            if let Some(found) = bytes[i..].windows(pb.len()).position(|w| w == pb) {
                let start = i + found;
                let end = start + pb.len();
                new_result.push_str(&result[i..start]);
                if is_whole_word_at(bytes, start, end, b"$") {
                    new_result.push('$');
                }
                new_result.push_str(p);
                i = end;
            } else {
                new_result.push_str(&result[i..]);
                i = bytes.len();
            }
        }
        if i < result.len() {
            new_result.push_str(&result[i..]);
        }
        result = new_result;
    }
    result
}
