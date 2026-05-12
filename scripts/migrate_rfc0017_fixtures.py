#!/usr/bin/env python3
"""
RFC-0017 fixture codemod: migrate test-driver construction of Frame
systems from the host-language native constructor to the `@@Foo(args)`
sigil.

Why: under RFC-0017 (init decoupling), the host-language constructor
(`new Foo()`, `Foo::new()`, `NewFoo()`, `Foo.new()`, bare `Foo()`,
`Foo_new()`, `foo:start_link()`) is the *bare, no-initialization*
constructor — it runs no `$>` enter handler. Construction goes through
the auto-generated factory, which `@@Foo(args)` lowers to per backend
(`Foo._create(args)` / `Foo.__create(args)` / `CreateFoo(args)` / ...).
So test-driver code that constructed systems the old way now either
fails to compile (native ctor *with arguments* — the bare ctor takes
none) or compiles but produces an under-initialized instance (native
ctor with no args, when the system has a side-effecting `$>`).

This codemod rewrites, *only for system names declared in the same
file*, the native-ctor construction forms to `@@<Name>(...)`:

    Java/C#/PHP/JS/TS/C++   new Foo(args)        -> @@Foo(args)
    Rust                    Foo::new(args)       -> @@Foo(args)
    Go                      NewFoo(args)         -> @@Foo(args)
    C                       Foo_new(args)        -> @@Foo(args)
    Ruby/Lua/GDScript       Foo.new(args)        -> @@Foo(args)
    Erlang                  foo:start_link(args) -> @@Foo(args)
    Python/Kotlin/Swift/Dart   Foo(args)         -> @@Foo(args)   (bare name at a value position)

It does NOT touch `@@Foo(...)` or `@@!Foo(...)` (already migrated /
deliberately no-init), nor anything inside an `@@system { ... }` block
(domain initializers already use `@@Foo()`, which the codegen handles).
Idempotent.

Usage:
    migrate_rfc0017_fixtures.py [--dry-run] <file-or-dir>...
"""

import argparse
import os
import re
import sys

# Map fixture extension -> target language family.
EXT_LANG = {
    ".fpy": "python", ".frs": "rust", ".fjava": "java", ".fkt": "kotlin",
    ".fswift": "swift", ".fcs": "csharp", ".fgo": "go", ".fc": "c",
    ".fcpp": "cpp", ".fdart": "dart", ".fphp": "php", ".frb": "ruby",
    ".flua": "lua", ".fgd": "gdscript", ".ferl": "erlang",
    ".fjs": "javascript", ".fts": "typescript",
}

# Languages whose construction syntax is a bare `Foo(...)` (no `new` /
# `::new` / etc.). These need a value-position guard so we don't rewrite
# unrelated function calls.
BARE_NAME_LANGS = {"python", "kotlin", "swift", "dart"}

# `new Foo(...)` languages.
NEW_KEYWORD_LANGS = {"java", "csharp", "php", "javascript", "typescript"}


def snake(name: str) -> str:
    """CamelCase -> snake_case (for Erlang module names)."""
    out = []
    for i, ch in enumerate(name):
        if ch.isupper() and i > 0:
            out.append("_")
        out.append(ch.lower())
    return "".join(out)


def find_system_names(src: str) -> list[str]:
    """Names of systems declared with `@@system [private] <Name> ...`."""
    names = []
    for m in re.finditer(r"@@system\s+(?:private\s+|public\s+|internal\s+)?([A-Za-z_][A-Za-z0-9_]*)", src):
        names.append(m.group(1))
    # Dedup, longest-first so e.g. `Outer` doesn't shadow `OuterChild`.
    return sorted(set(names), key=len, reverse=True)


def split_off_system_blocks(src: str) -> list[tuple[bool, str]]:
    """Split `src` into [(is_system_block, text), ...].

    A 'system block' is `@@system ... { ... }` with brace matching.
    We don't rewrite inside these (domain initializers already use
    `@@Foo()`, handled by the codegen)."""
    parts = []
    i = 0
    n = len(src)
    while i < n:
        m = re.search(r"@@system\b", src[i:])
        if not m:
            parts.append((False, src[i:]))
            break
        start = i + m.start()
        parts.append((False, src[i:start]))
        # find the opening brace of the system body
        j = src.find("{", start)
        if j == -1:
            parts.append((False, src[start:]))
            break
        depth = 0
        k = j
        while k < n:
            c = src[k]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    k += 1
                    break
            k += 1
        parts.append((True, src[start:k]))
        i = k
    return parts


def rewrite_driver(text: str, names: list[str], lang: str) -> str:
    """Rewrite native-ctor construction of `names` in driver text."""
    for name in names:
        n = re.escape(name)
        if lang in NEW_KEYWORD_LANGS:
            text = re.sub(rf"\bnew\s+{n}\s*\(", f"@@{name}(", text)
        if lang == "rust":
            text = re.sub(rf"\b{n}::new\s*\(", f"@@{name}(", text)
        if lang == "go":
            text = re.sub(rf"\bNew{n}\s*\(", f"@@{name}(", text)
        if lang == "c":
            text = re.sub(rf"\b{n}_new\s*\(", f"@@{name}(", text)
        if lang in ("ruby", "lua", "gdscript"):
            # `Foo.new(args)` -> `@@Foo(args)`. Ruby also permits the
            # parenless `Foo.new`; rewrite that to `@@Foo()`. The two
            # lookaheads keep `Foo.new_thing` / `Foo.newX` (next char a
            # word char) and `Foo.new(...)` (handled above) from
            # matching, while still allowing a newline / statement to
            # follow `Foo.new`.
            text = re.sub(rf"\b{n}\.new\s*\(", f"@@{name}(", text)
            text = re.sub(rf"\b{n}\.new(?!\w)(?!\s*\()", f"@@{name}()", text)
        if lang == "lua":
            # Lua's colon-call idiom: `Foo:new(args)` -> `@@Foo(args)`
            # (the implicit `self` arg the colon passes is the now-no-arg
            # bare ctor's, so it was being ignored anyway).
            text = re.sub(rf"\b{n}:new\s*\(", f"@@{name}(", text)
        if lang == "erlang":
            text = re.sub(rf"\b{re.escape(snake(name))}\s*:\s*start_link\s*\(", f"@@{name}(", text)
        if lang in BARE_NAME_LANGS:
            # Bare `Foo(` at a value position: preceded by something
            # that isn't an identifier char, `.`, or `@` (so we don't
            # match `obj.Foo(`, `barFoo(`, `@@Foo(`, `@@!Foo(`).
            text = re.sub(rf"(?<![\w.@]){n}\s*\(", f"@@{name}(", text)
        if lang == "cpp":
            # C++ has several construction syntaxes; under RFC-0017 the
            # bare `Foo` constructor is the *no-init* one, so each must
            # route through the factory (`@@Foo(args)` -> `Foo::__create`,
            # which returns by value, so the call sites stay value-form):
            #   new Foo(args)            -> @@Foo(args)
            #   auto x = Foo(args)       -> auto x = @@Foo(args)
            #   Foo x(args);             -> auto x = @@Foo(args);
            #   Foo x;                   -> auto x = @@Foo();
            # Function parameters (`const Foo& x`, `Foo* x`) and forward
            # declarations (`class Foo;`) don't match these — `&`/`*`/the
            # `class` keyword break the patterns. Order matters: the
            # `Foo x(args);` rewrite must precede `Foo x;`.
            text = re.sub(rf"\bnew\s+{n}\s*\(", f"@@{name}(", text)
            text = re.sub(rf"\bauto(\s+\w+\s*=\s*){n}\s*\(", rf"auto\1@@{name}(", text)
            text = re.sub(
                rf"\b{n}\s+(\w+)\s*\(([^;{{}}]*)\)\s*;",
                rf"auto \1 = @@{name}(\2);",
                text,
            )
            text = re.sub(rf"\b{n}\s+(\w+)\s*;", rf"auto \1 = @@{name}();", text)
    # Collapse any accidental double-prefix from re-running.
    text = text.replace("@@@@", "@@")
    return text


def migrate(src: str, lang: str) -> str:
    names = find_system_names(src)
    if not names:
        return src
    out = []
    for is_block, chunk in split_off_system_blocks(src):
        out.append(chunk if is_block else rewrite_driver(chunk, names, lang))
    return "".join(out)


# Negative-test corpora are deliberately malformed; rewriting their
# construction sites could change which error they trigger, so leave
# them alone.
SKIP_DIR_PARTS = ("compile-error", "transpile-error")


def process_file(path: str, dry_run: bool) -> bool:
    if any(part in SKIP_DIR_PARTS for part in path.split(os.sep)):
        return False
    ext = os.path.splitext(path)[1]
    lang = EXT_LANG.get(ext)
    if lang is None:
        return False
    with open(path, "r", encoding="utf-8") as f:
        src = f.read()
    new = migrate(src, lang)
    if new == src:
        return False
    if dry_run:
        print(f"would change: {path}")
    else:
        with open(path, "w", encoding="utf-8") as f:
            f.write(new)
        print(f"changed: {path}")
    return True


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("paths", nargs="+")
    args = ap.parse_args()
    changed = 0
    for p in args.paths:
        if os.path.isdir(p):
            for root, _dirs, files in os.walk(p):
                for fn in files:
                    if os.path.splitext(fn)[1] in EXT_LANG:
                        changed += process_file(os.path.join(root, fn), args.dry_run)
        else:
            changed += process_file(p, args.dry_run)
    print(f"\n{'would change' if args.dry_run else 'changed'} {changed} file(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
