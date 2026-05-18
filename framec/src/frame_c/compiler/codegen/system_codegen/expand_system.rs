//! Domain-initializer `@@SystemName(args)` expansion.
//!
//! When a system has another system embedded in its domain (e.g.
//! `domain { var inner: Counter = @@Counter(7) }`), the codegen has
//! to rewrite the `@@Counter(7)` into the right factory-call
//! spelling per backend. This is the SAME lowering the assembler
//! performs for source-level `@@Name(args)` — delegating keeps the
//! two paths in sync so future per-backend spelling changes only
//! touch one site (the assembler's `generate_constructor`).
//!
//! C++ is the one wrinkle: domain fields holding system instances
//! are `std::shared_ptr<T>`, but the factory returns `T` by value.
//! Wrapping with `std::make_shared<T>(...)` move-constructs the
//! returned temporary into a heap-managed instance.

use crate::frame_c::compiler::native_region_scanner::create_skipper;
use crate::frame_c::visitors::TargetLanguage;

/// Expand `@@SystemName(args)` in domain variable initializers to
/// native constructor / factory calls. Mirrors the assembler's
/// system-instantiation expansion, but for domain code emitted
/// during codegen (before the assembler runs).
pub(crate) fn expand_system_instantiation_in_domain(
    raw_code: &str,
    lang: TargetLanguage,
) -> String {
    // Delegate string/comment skipping at the outer level to the target
    // language's skipper so `@@Foo(...)` appearing inside a string or
    // comment isn't mistakenly rewritten into a constructor call.
    let skipper = create_skipper(lang);
    let bytes = raw_code.as_bytes();
    let end = bytes.len();
    let mut result = String::new();
    let mut i = 0;

    while i < end {
        if let Some(next) = skipper.skip_string(bytes, i, end) {
            result.push_str(&raw_code[i..next]);
            i = next;
            continue;
        }
        if let Some(next) = skipper.skip_comment(bytes, i, end) {
            result.push_str(&raw_code[i..next]);
            i = next;
            continue;
        }
        // Look for @@ followed by uppercase letter
        if i + 2 < bytes.len()
            && bytes[i] == b'@'
            && bytes[i + 1] == b'@'
            && i + 2 < bytes.len()
            && bytes[i + 2].is_ascii_uppercase()
        {
            let start = i;
            i += 2;
            let name_start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let name = std::str::from_utf8(&bytes[name_start..i]).unwrap_or("");

            if i < bytes.len() && bytes[i] == b'(' {
                // Use the target language's skipper to find the matching
                // close paren — handles string literals, escapes, and
                // nested parens per-language rather than re-implementing
                // them here.
                let skipper = create_skipper(lang);
                let args_start = i + 1;
                let after_close = match skipper.balanced_paren_end(bytes, i, bytes.len()) {
                    Some(pos) => pos,
                    None => {
                        // Unbalanced — emit the bare `@@Name` and move on.
                        result.push_str(&raw_code[start..i]);
                        continue;
                    }
                };
                let close_paren = after_close - 1;
                let args = std::str::from_utf8(&bytes[args_start..close_paren]).unwrap_or("");
                i = after_close;
                // RFC-0017: domain-initializer `@@Name(args)` must
                // lower to the same factory call the assembler emits
                // for source-level `@@Name(args)`. Delegating keeps
                // the two paths in sync so a future per-backend
                // spelling change only updates one site.
                let constructor = match lang {
                    TargetLanguage::Graphviz => name.to_string(),
                    // C++ system-typed domain fields are `shared_ptr<T>`;
                    // the factory returns `T` by value (so drivers stay
                    // value-semantics), so wrap it — move-constructing the
                    // returned temporary into a heap-managed instance.
                    TargetLanguage::Cpp => {
                        let factory = crate::frame_c::compiler::assembler::generate_constructor(
                            name, args, lang,
                        );
                        format!("std::make_shared<{}>({})", name, factory)
                    }
                    _ => {
                        crate::frame_c::compiler::assembler::generate_constructor(name, args, lang)
                    }
                };
                result.push_str(&constructor);
                continue;
            }
            // Not a valid pattern — copy original
            for b in &bytes[start..i] {
                result.push(*b as char);
            }
            continue;
        }

        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC-0017 regression: `@@SystemName(args)` inside a domain
    // initializer must lower to the factory call the assembler emits
    // for source-level `@@SystemName(args)`. Before the fix, this
    // expander emitted the bare-ctor spelling, which broke Godot
    // ("Too many arguments for new()") and silently passed extra
    // args to the no-arg framework constructor on typed backends.
    #[test]
    fn test_domain_init_uses_rfc0017_factory() {
        let cases = &[
            (TargetLanguage::Python3, "Counter._create(7)"),
            (TargetLanguage::GDScript, "Counter._create(7)"),
            (TargetLanguage::Java, "Counter.__create(7)"),
            (TargetLanguage::Kotlin, "Counter.__create(7)"),
            (TargetLanguage::Swift, "Counter.__create(7)"),
            (TargetLanguage::CSharp, "Counter.__create(7)"),
            (
                TargetLanguage::Cpp,
                "std::make_shared<Counter>(Counter::__create(7))",
            ),
            (TargetLanguage::Rust, "Counter::__create(7)"),
            (TargetLanguage::C, "Counter_create(7)"),
            (TargetLanguage::Go, "CreateCounter(7)"),
            (TargetLanguage::Dart, "Counter._create(7)"),
            (TargetLanguage::JavaScript, "Counter._create(7)"),
            (TargetLanguage::TypeScript, "Counter._create(7)"),
            (TargetLanguage::Ruby, "Counter._create(7)"),
            (TargetLanguage::Lua, "Counter._create(7)"),
            (TargetLanguage::Php, "Counter::_create(7)"),
        ];
        for (lang, expected) in cases {
            let actual = expand_system_instantiation_in_domain("@@Counter(7)", *lang);
            assert_eq!(
                actual, *expected,
                "{:?} domain-init expansion mismatch",
                lang
            );
        }
    }
}
