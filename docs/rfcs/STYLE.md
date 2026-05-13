# Writing Frame RFCs — Style Guide

This guide defines how RFC documents in `docs/rfcs/` are written. It exists
because, without one, RFCs drift: they accumulate project-management residue
(phase numbers, internal decision codes, line-of-code counts), reference files
that only exist in someone's working tree, and interleave "what we decided" with
"how the compiler does it" until a reader who lacks the project's internal
context can't follow them.

An RFC is a **design document for users and future maintainers**. It explains
*why* a change exists and *how the language behaves* — not how the compiler is
built, and not when which piece shipped.

## Conventions this follows

This guide adapts well-established RFC/specification practice:

- **The Rust RFC process** (`rust-lang/rfcs`) — the section skeleton (Summary →
  Motivation → Guide-level / Reference-level explanation → Drawbacks →
  Rationale and alternatives → Unresolved questions) and the framing that an RFC
  is a *proposal and rationale*, not a changelog.
- **IETF RFC 2119** — the requirement vocabulary. When an RFC states a normative
  rule, use **MUST** / **MUST NOT** / **SHOULD** / **SHOULD NOT** / **MAY** with
  their RFC 2119 meanings, and say so.
- **IETF RFC 7322** ("RFC Style Guide") — the readability rules: define terms
  before using them, avoid jargon and undefined abbreviations, write for a
  reader outside the working group.
- **Python PEP 1** ("PEP Purpose and Guidelines") and **PEP 12** (the PEP
  template) — the front-matter discipline (a small fixed metadata block) and the
  idea of a canonical, copy-paste template.

You don't need to have read those documents to write a Frame RFC — this guide
distills what matters. They're cited so the conventions here are anchored to
something stable rather than invented.

## What belongs in an RFC

- **Motivation** — the problem, described in prose. State the problem so a reader
  who has never seen the bug report can understand it. (Do *not* cite it as
  "Issue #2" or by a tracker path — describe what went wrong.)
- **The contract** — the user-visible behavior and syntax the change introduces
  or changes. This is the heart of the document.
- **Examples** — worked examples in Frame source (see *Code samples* below).
- **Alternatives** — the designs considered and rejected, with the reason for
  each rejection. This is some of the most valuable content in an RFC; keep it.
- **Migration** — what existing code or users have to do, if anything. Mention a
  codemod or tool by name in one sentence; don't enumerate how many fixtures it
  touched.
- **References** — links to related RFCs, to the [language reference](../frame_language.md),
  to the [glossary](../glossary.md), and to `CHANGELOG.md`. Nothing else.

## What does NOT belong in an RFC

- **Implementation phase / wave / stage labels** ("Phase A0", "Wave 3").
- **Internal decision codes** ("D1", "D7") used as if the reader knows them.
- **Line-of-code counts and fixture/test counts** ("−150 LOC", "4,781/4,781").
- **References to files outside the published repository** — gitignored scratch
  notes (`_scratch/…`), sibling repositories (`frame-arcade/…`), internal
  tracking docs.
- **Names of internal compiler functions, structs, or modules** — the reader of
  an RFC is not reading the codegen source.
- **Commit hashes and dated shipping logs.**

Where does that information go? The **CHANGELOG** records what shipped when. The
**git history** records the implementation detail and who changed what. An RFC's
*status* is **one line** in its front-matter — `Status: Shipped in framec 4.1.0`
or `Status: Draft` — and nothing more.

## Terminology

- Every term that isn't standard programming vocabulary **MUST** have an entry
  in the [glossary](../glossary.md). An RFC links the glossary on a term's first
  use and does **not** redefine it.
- New terms get a glossary entry *before or with* the RFC that introduces them —
  never after.
- Use the canonical name. When a term has been renamed, an RFC uses the current
  name; the glossary carries the parenthetical note about the old one.

## Structure

Every RFC follows this skeleton. Omit a section only if it genuinely has no
content (a non-breaking change has no Migration section, for instance — say so
in one line rather than padding it).

Write the front-matter as a bulleted list — single newlines between plain
`**Field:** value` lines collapse into one paragraph when rendered, so the list
markers are what keep each field on its own line.

```markdown
# RFC-NNNN: <Short Title>

- **Status:** <Draft | Accepted | Shipped in framec X.Y.Z — see CHANGELOG | Superseded by RFC-MMMM>
- **Author:** <name> <email>
- **Created:** <YYYY-MM-DD>
- **Builds on:** RFC-AAAA, RFC-BBBB        (optional)
- **Supersedes:** RFC-CCCC                  (optional)
- **Superseded in part by:** RFC-DDDD       (optional; name which part)

## Summary

One short paragraph: what the change is, in plain language.

## Motivation

The problem, described so it stands on its own. What's wrong today; why it
matters; what a fix needs to achieve.

## The contract

The user-visible behavior and syntax. Subsections as needed. This is the
normative core — if you use RFC 2119 keywords, say "The key words MUST, SHOULD,
… are to be interpreted as in RFC 2119" once, here.

## Examples

Worked Frame-source examples illustrating the contract.

## Alternatives

The designs considered and rejected, each with its rejection reason.

## Migration

What existing code/users must do. If nothing: "Source-additive; no breaking
change."

## References

- RFC-XXXX — <one-line description>
- [Frame language reference](../frame_language.md)
- [Glossary](../glossary.md)
- `CHANGELOG.md`
```

## Code samples

Fenced ` ```frame ` blocks in `docs/` (including RFCs) are checked by the
pre-commit hook, which runs `scripts/validate_doc_samples.py`. The validator
**executes** a block only if it contains *both*:

1. a target directive — `@@[target("python_3")]` (or `@@target python_3`), and
2. a Python entry guard — `if __name__ == "__main__":`.

A block that omits either of those is treated as **illustrative** — it is
neither compiled nor run. So:

- A small syntax illustration: omit the target directive. It's clearly
  pseudo-Frame.
- A full, real example you want kept honest: include the target directive *and*
  the entry guard, and make sure it compiles and runs.

Keep runnable examples runnable. Don't let an illustrative block silently rot
into something that wouldn't compile — if it's worth showing, it's worth being
either real or obviously schematic.

## Lifecycle

An RFC moves through:

1. **Draft** — under discussion; design may change.
2. **Accepted** — design agreed; not yet (fully) implemented.
3. **Shipped in framec X.Y.Z** — implemented and released. The front-matter
   names the release; the CHANGELOG has the detail.
4. **Superseded by RFC-MMMM** — a later RFC replaces it, in whole or in part.
   The front-matter says which; the body keeps a one-line pointer at the top of
   any section that was superseded ("The per-backend mechanism here was
   superseded by RFC-MMMM — see there.").

RFC files are named `rfc-NNNN.md`, zero-padded, in `docs/rfcs/`. A *sub-RFC* —
one that defines or amends one specific piece of an existing RFC, rather than a
new feature — uses `rfc-NNNN-M.md` (e.g. `rfc-0016-1.md` defines the
`@@[no_persist]` attribute that RFC-0016 only mentions in passing) and carries
a `**Companion to:** RFC-NNNN` (or `**Builds on:**`) line in its front-matter.
Prefer a fresh top-level number for anything that's really a new feature; reserve
the sub-RFC form for "this thing was buried in / underspecified by RFC-NNNN".
This style guide and the [glossary](../glossary.md) are *not* RFCs.

## When to write an RFC vs. a doc edit

Write an RFC for a change to the **language** — new syntax, a changed
construct's behavior, a new attribute, a contract change. Don't write an RFC for
a bug fix that brings the implementation into line with an already-documented
contract; that's a CHANGELOG entry and, if the docs were unclear, a doc edit.

## Reviewing an RFC against this guide — checklist

- [ ] Status is one line; no phase/wave log, no LOC/fixture counts.
- [ ] No references to `_scratch/…`, sibling repos, or internal compiler
      functions/modules.
- [ ] Every non-standard term links to the glossary on first use; no term is
      used undefined.
- [ ] The Motivation describes the problem in prose — no "Issue #N" / tracker
      citation.
- [ ] If superseded, the front-matter and the affected section(s) say so and
      link the superseding RFC.
- [ ] Runnable code samples compile and run; illustrative ones are clearly
      schematic.
- [ ] The document reads top to bottom for someone with no project-internal
      context.
