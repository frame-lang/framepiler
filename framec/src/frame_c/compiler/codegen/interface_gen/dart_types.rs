//! Dart type-tree helpers used by the persist-restore emitter.
//!
//! Dart is the only backend that needs a proper type tree at codegen
//! time. Reason: Dart has reified generics (collections carry their
//! element type at runtime) AND a `dynamic`-shaped JSON path. To
//! cross that boundary without per-access casts, the restore emitter
//! has to construct typed-comprehension expressions (`<T>[for ... ]`
//! / `<K,V>{for ... }`) — Dart's only construct that reliably
//! builds a genuinely-typed collection from a dynamic source.
//!
//! Every other backend either has erased generics (Java, Kotlin),
//! relies on whole-object deserializers (Rust serde, JVM Jackson, C++
//! nlohmann), or has no static types to honor (Python, JS, Lua,
//! GDScript). Dart sits alone.
//!
//! Architectural commitment: this module is **type-ignorant** at the
//! shape level. It parses two structural shapes — `List<...>` and
//! `Map<...,...>` — and treats every other declaration as
//! `Primitive(name)`. The downstream cast emitter handles the
//! per-primitive translation (`str` → `String`, `float` → `double`,
//! `num.toInt()` for `int`, etc.); we never add a per-user-type
//! branch here.

/// Dart type-tree node. Two structural shapes plus a primitive leaf.
/// Anything that doesn't pattern-match `List<...>` / `Map<...,...>`
/// becomes a `Primitive(name)` and is emitted as `value as <name>`.
pub(super) enum DartTypeNode {
    Primitive(String),
    List(Box<DartTypeNode>),
    Map(Box<DartTypeNode>, Box<DartTypeNode>),
}

pub(super) fn parse_dart_type(s: &str) -> DartTypeNode {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("List<").and_then(|x| x.strip_suffix('>')) {
        return DartTypeNode::List(Box::new(parse_dart_type(inner)));
    }
    if let Some(inner) = s.strip_prefix("Map<").and_then(|x| x.strip_suffix('>')) {
        // Find top-level comma (not nested in <>).
        let mut depth = 0i32;
        let mut comma_pos: Option<usize> = None;
        for (i, c) in inner.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth == 0 => {
                    comma_pos = Some(i);
                    break;
                }
                _ => {}
            }
        }
        if let Some(p) = comma_pos {
            let k = parse_dart_type(&inner[..p]);
            let v = parse_dart_type(&inner[p + 1..]);
            return DartTypeNode::Map(Box::new(k), Box::new(v));
        }
    }
    // Normalize Frame keyword types to Dart's actual primitive names
    // so the downstream cast emitter doesn't produce `as str` /
    // `as float` (Dart errors). `int` and `bool` already match.
    let normalized = match s {
        "str" | "string" => "String",
        "float" => "double",
        other => other,
    };
    DartTypeNode::Primitive(normalized.to_string())
}

pub(super) fn render_dart_type(t: &DartTypeNode) -> String {
    match t {
        DartTypeNode::Primitive(s) => s.clone(),
        DartTypeNode::List(inner) => format!("List<{}>", render_dart_type(inner)),
        DartTypeNode::Map(k, v) => {
            format!("Map<{}, {}>", render_dart_type(k), render_dart_type(v))
        }
    }
}

/// Emit a Dart expression that converts `input` (type `dynamic`) to
/// the typed shape described by `t`. Uses comprehensions
/// (`<T>[for ...]` / `<K,V>{for ...}`) to produce genuinely-typed
/// collections — the only Dart construct that reliably bridges
/// `dynamic`-shaped JSON output to reified-generic typed fields
/// without per-access casts.
///
/// Variable names carry a depth suffix so nested comprehensions
/// don't shadow each other (`__e1`, `__me2`, etc.).
pub(super) fn dart_conv_expr(t: &DartTypeNode, input: &str) -> String {
    dart_conv_expr_at(t, input, 0)
}

fn dart_conv_expr_at(t: &DartTypeNode, input: &str, depth: usize) -> String {
    match t {
        DartTypeNode::Primitive(name) => match name.as_str() {
            "int" => format!("({input} as num).toInt()"),
            "double" => format!("({input} as num).toDouble()"),
            "num" => format!("{input} as num"),
            "String" => format!("{input} as String"),
            "bool" => format!("{input} as bool"),
            "dynamic" | "Object" | "Object?" => input.to_string(),
            other => format!("{input} as {other}"),
        },
        DartTypeNode::List(inner) => {
            let var = format!("__e{}", depth);
            let elem = dart_conv_expr_at(inner, &var, depth + 1);
            let inner_t = render_dart_type(inner);
            format!("<{inner_t}>[for (var {var} in ({input} as List)) {elem}]")
        }
        DartTypeNode::Map(k, v) => {
            let var = format!("__me{}", depth);
            let k_expr = dart_conv_expr_at(k, &format!("{var}.key"), depth + 1);
            let v_expr = dart_conv_expr_at(v, &format!("{var}.value"), depth + 1);
            let k_t = render_dart_type(k);
            let v_t = render_dart_type(v);
            format!(
                "<{k_t}, {v_t}>{{for (var {var} in ({input} as Map).entries) {k_expr}: {v_expr}}}"
            )
        }
    }
}
