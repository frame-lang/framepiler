#!/usr/bin/env python3
"""
RFC-0015 codemod: migrate RFC-0012 operation-attribute persist form
to RFC-0015 system-level form.

Before (RFC-0012, accepted by framec 4.0.x):

    @@[persist]
    @@system Foo {
        operations:
            @@[save]
            save_state(): bytes {}

            @@[load]
            restore_state(data: bytes) {}

        interface:
            ...
    }

After (RFC-0015, required by framec 4.1.0+):

    @@[persist(bytes)]
    @@[save(save_state)]
    @@[load(restore_state)]
    @@system Foo {
        interface:
            ...
    }

Behaviour:

  1. Recognises any `@@[persist]` system whose operations block
     contains `@@[save] X(): T {}` and/or `@@[load] Y(data: T) {}`.
  2. Lifts the names to system-level `@@[save(X)]` / `@@[load(Y)]`
     attributes inserted just before `@@system`.
  3. Lifts the type T to `@@[persist(T)]` (if `@@[persist]` is bare).
  4. Strips the lifted op declarations from `operations:`.
  5. Removes `operations:` block if it becomes empty.
  6. Leaves systems without persist untouched.
  7. Idempotent — files already in RFC-0015 form pass through
     unchanged.

Usage:
    migrate_rfc0015.py <file>...
    migrate_rfc0015.py --dry-run <file>...
"""

import argparse
import re
import sys
from pathlib import Path


# Attribute markers we lift. The operation's body is permitted to be
# `{}` (empty — framework-generated) or `{ ... }` with whitespace.
RE_PERSIST_BARE = re.compile(r"@@\[persist\](?!\s*\()")
RE_PERSIST_WITH_ARG = re.compile(r"@@\[persist\(([^)]+)\)\]")
RE_SAVE_AT_SYSTEM = re.compile(r"@@\[save\s*\(\s*\w+\s*\)\]")
RE_LOAD_AT_SYSTEM = re.compile(r"@@\[load\s*\(\s*\w+\s*\)\]")
# Allow an optional visibility modifier (`private`, `public`,
# `internal`) between `@@system` and the system name — Java fixtures
# use `@@system private L5 {` to keep nested classes file-local.
RE_SYSTEM_HEADER = re.compile(
    r"^(\s*)(@@system\s+(?:private\s+|public\s+|internal\s+)?\w+\s*\{)",
    re.MULTILINE,
)


def find_block(text, start_idx):
    """Find the matching `}` for a `{` at `start_idx`. Returns the
    index of the matching `}`, or None if unbalanced."""
    assert text[start_idx] == "{"
    depth = 1
    i = start_idx + 1
    while i < len(text) and depth > 0:
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return None


def extract_op_attribute(operations_text, attr_name):
    """Find and extract a `@@[<attr_name>] <name>(...): T {}` (or
    `@@[<attr_name>] <name>(data: T) {}` for load) declaration in
    operations_text. Returns (op_name, type_str, span_start, span_end)
    or None.

    span_start/span_end mark the slice that includes the @@[attr],
    the op declaration, and any trailing blank line — so the caller
    can excise it cleanly.
    """
    pattern = re.compile(
        r"(?P<lead>^[ \t]*)@@\[" + attr_name + r"\][ \t]*\n"
        r"[ \t]*(?P<name>\w+)\s*\((?P<params>[^)]*)\)\s*"
        r"(?::\s*(?P<rtype>[\w\s\*<>:&\[\],]+?))?\s*"
        r"\{\s*\}\s*",
        re.MULTILINE,
    )
    m = pattern.search(operations_text)
    if m is None:
        return None
    name = m.group("name")
    params = m.group("params").strip()
    rtype = (m.group("rtype") or "").strip()

    # Type comes from save's return type or load's data-param type.
    if attr_name == "save":
        type_str = rtype
    else:
        # load: expect `data: T`
        if ":" in params:
            type_str = params.split(":", 1)[1].strip()
        else:
            type_str = ""

    span_start = m.start()
    # Consume any blank line(s) immediately following the op's `}`.
    span_end = m.end()
    while span_end < len(operations_text) and operations_text[span_end] in " \t":
        span_end += 1
    if span_end < len(operations_text) and operations_text[span_end] == "\n":
        span_end += 1
        # Plus an optional fully blank trailing line.
        while span_end < len(operations_text) and operations_text[span_end] in " \t":
            span_end += 1
        if span_end < len(operations_text) and operations_text[span_end] == "\n":
            span_end += 1

    return (name, type_str, span_start, span_end)


def operations_block_is_empty(text):
    """Return True iff `text` (the body between `operations:` and the
    next top-level keyword) contains only whitespace."""
    return text.strip() == ""


def find_operations_block(system_body):
    """Within a system body, find the `operations:` block. Returns
    (header_start, header_end, body_start, body_end) where:
      - header_start..header_end covers `    operations:\\n`
      - body_start..body_end covers the contents up to the next
        top-level keyword (`interface:`, `machine:`, `domain:`,
        `actions:`) or end of system body.
    Returns None if no `operations:` block is present.
    """
    m = re.search(r"^([ \t]*)operations:\s*\n", system_body, re.MULTILINE)
    if m is None:
        return None
    header_start = m.start()
    header_end = m.end()
    body_start = header_end
    # The block ends at the next top-level keyword OR end-of-body.
    next_kw = re.search(
        r"^[ \t]*(interface|machine|domain|actions):\s*\n",
        system_body[body_start:],
        re.MULTILINE,
    )
    if next_kw is None:
        body_end = len(system_body)
    else:
        body_end = body_start + next_kw.start()
    return (header_start, header_end, body_start, body_end)


def migrate(text):
    """Apply the RFC-0015 migration to `text`. Returns the migrated
    text (or `text` unchanged if no migration was needed)."""
    # Walk every `@@system Name {` header. For each, locate the
    # preceding attribute lines, the system body, and rewrite if
    # needed.
    out_parts = []
    cursor = 0
    changed = False

    while True:
        m = RE_SYSTEM_HEADER.search(text, cursor)
        if m is None:
            out_parts.append(text[cursor:])
            break

        # Collect the attribute lines immediately above the @@system.
        # Walk backward from the system header line, collecting lines
        # that look like `@@[...]` attributes (allowing blank lines
        # between, but stopping at non-attribute content).
        sys_line_start = m.start(2)
        # Scan backward to find the start of the attribute prelude.
        attr_block_start = sys_line_start
        i = sys_line_start - 1
        # Skip the newline immediately preceding the system header.
        while i >= cursor:
            # Find the start of this line.
            line_start = text.rfind("\n", cursor, i)
            line_start = cursor if line_start == -1 else line_start + 1
            line = text[line_start:i + 1]
            stripped = line.strip()
            if stripped == "":
                # Blank line — keep walking back.
                attr_block_start = line_start
                i = line_start - 1
                continue
            if stripped.startswith("@@["):
                attr_block_start = line_start
                i = line_start - 1
                continue
            # Anything else: stop walking.
            break

        attr_block = text[attr_block_start:sys_line_start]

        # Locate the system body: from `{` to matching `}`.
        brace_idx = text.index("{", m.start(2))
        body_close = find_block(text, brace_idx)
        if body_close is None:
            # Unbalanced — bail on this system, copy through.
            out_parts.append(text[cursor:m.end()])
            cursor = m.end()
            continue

        system_body = text[brace_idx + 1:body_close]

        # A system needs migration if it has `@@[persist]` (with or
        # without type arg), OR if it has `@@[save]` / `@@[load]` ops
        # in its operations block. The latter case covers nested
        # systems used inside a parent's domain — codegen treats those
        # ops as lifecycle methods even without an explicit
        # `@@[persist]` attribute. RFC-0015 promotes them to
        # system-level attributes which need to be added.
        ops_loc = find_operations_block(system_body)
        if ops_loc is None:
            out_parts.append(text[cursor:body_close + 1])
            cursor = body_close + 1
            continue
        h_start, h_end, b_start, b_end = ops_loc
        ops_body = system_body[b_start:b_end]

        save_match = extract_op_attribute(ops_body, "save")
        load_match = extract_op_attribute(ops_body, "load")
        has_persist = bool(RE_PERSIST_BARE.search(attr_block) or RE_PERSIST_WITH_ARG.search(attr_block))
        if save_match is None and load_match is None:
            # Nothing to lift.
            out_parts.append(text[cursor:body_close + 1])
            cursor = body_close + 1
            continue

        # Build the new attribute prelude and updated system body.
        save_name = save_match[0] if save_match else None
        load_name = load_match[0] if load_match else None
        # Type comes from save's return; fall back to load's param.
        type_str = ""
        if save_match and save_match[1]:
            type_str = save_match[1]
        elif load_match and load_match[1]:
            type_str = load_match[1]

        new_attr_block = attr_block

        # Lift persist arg if persist is bare and we have a type.
        if RE_PERSIST_BARE.search(new_attr_block) and type_str:
            new_attr_block = RE_PERSIST_BARE.sub(f"@@[persist({type_str})]", new_attr_block, count=1)

        # If `@@[persist]` is absent but the system has lifecycle ops
        # (nested-system case), synthesise a `@@[persist(T)]` line at
        # the top of the attr block. This matches the RFC-0015
        # contract: every system whose codegen generates save/load
        # methods declares `@@[persist]` explicitly.
        if not has_persist and type_str:
            # Match the indentation of the first existing attribute,
            # or default to no leading whitespace (most attribute
            # preludes are flush-left).
            first_attr = re.search(r"^([ \t]*)@@\[", new_attr_block, re.MULTILINE)
            indent = first_attr.group(1) if first_attr else ""
            new_attr_block = f"{indent}@@[persist({type_str})]\n" + new_attr_block

        # Insert @@[save(<name>)] and @@[load(<name>)] lines if not
        # already present.
        # Match the indentation of @@[persist].
        persist_indent_match = re.search(r"^([ \t]*)@@\[persist", new_attr_block, re.MULTILINE)
        indent = persist_indent_match.group(1) if persist_indent_match else ""

        attr_lines_to_add = []
        if save_name and not RE_SAVE_AT_SYSTEM.search(new_attr_block):
            attr_lines_to_add.append(f"{indent}@@[save({save_name})]\n")
        if load_name and not RE_LOAD_AT_SYSTEM.search(new_attr_block):
            attr_lines_to_add.append(f"{indent}@@[load({load_name})]\n")

        if attr_lines_to_add:
            # Insert just after the @@[persist] line.
            persist_line_match = re.search(
                r"^[ \t]*@@\[persist[^\n]*\n",
                new_attr_block,
                re.MULTILINE,
            )
            if persist_line_match:
                insert_pos = persist_line_match.end()
                new_attr_block = (
                    new_attr_block[:insert_pos]
                    + "".join(attr_lines_to_add)
                    + new_attr_block[insert_pos:]
                )

        # Remove the lifted op declarations from operations body.
        # extract_op_attribute returned spans relative to ops_body.
        spans_to_remove = []
        if save_match:
            spans_to_remove.append((save_match[2], save_match[3]))
        if load_match:
            spans_to_remove.append((load_match[2], load_match[3]))
        spans_to_remove.sort()
        new_ops_body_parts = []
        prev = 0
        for s, e in spans_to_remove:
            new_ops_body_parts.append(ops_body[prev:s])
            prev = e
        new_ops_body_parts.append(ops_body[prev:])
        new_ops_body = "".join(new_ops_body_parts)

        # If operations body is now empty, drop the whole block
        # (header + body).
        if operations_block_is_empty(new_ops_body):
            new_system_body = (
                system_body[:h_start] + system_body[b_end:]
            )
        else:
            new_system_body = (
                system_body[:b_start] + new_ops_body + system_body[b_end:]
            )

        # Reassemble.
        out_parts.append(text[cursor:attr_block_start])
        out_parts.append(new_attr_block)
        out_parts.append(text[sys_line_start:brace_idx + 1])
        out_parts.append(new_system_body)
        out_parts.append(text[body_close:body_close + 1])
        cursor = body_close + 1
        changed = True

    return "".join(out_parts), changed


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("files", nargs="+", type=Path)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    n_changed = 0
    n_unchanged = 0
    for fp in args.files:
        text = fp.read_text()
        new_text, changed = migrate(text)
        if changed:
            if args.dry_run:
                print(f"WOULD CHANGE: {fp}")
            else:
                fp.write_text(new_text)
                print(f"CHANGED: {fp}")
            n_changed += 1
        else:
            n_unchanged += 1
    print(
        f"\nSummary: {n_changed} changed, {n_unchanged} unchanged, "
        f"{n_changed + n_unchanged} total"
    )


if __name__ == "__main__":
    main()
