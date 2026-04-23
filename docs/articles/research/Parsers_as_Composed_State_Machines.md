# Parsers as Composed State Machines: A Frame Architecture

Parsers are among the oldest well-understood problems in computer science. The tools for building them — lex and yacc, flex and bison, ANTLR, Ragel, parser combinator libraries, hand-rolled recursive descent, and more recently tree-sitter — have been refined over more than five decades. A new parsing tool has to justify itself against that inheritance. Any claim that state machines deserve a role in parsing runs immediately into the observation that state machines have always had a role in parsing, in the form of finite automata for lexing. The question is whether there is anything left to say above the lexical layer, where the work of recognizing structure has traditionally been the domain of pushdown automata implemented as either recursive descent procedures or as tables driven by a bottom-up algorithm.

This essay proposes that there is. Not because state machines are a superior replacement for recursive descent — they aren't, for the reasons this essay will examine — but because a sufficiently expressive state machine language enables a parser architecture that neither recursive descent nor parser generators express cleanly: a composition of specialist machines, each solving a bounded sub-problem, coordinated by a high-level machine that treats them as oracles. The resulting parser is not state machines all the way down. It is state machines where state machines help and conventional code where conventional code helps, composed through a uniform interface.

The architecture described here is, as of this writing, proposed rather than documented. No production parser is currently built this way. The cookbook's recipes #53 and #54 — a byte scanner and a bracket-matching pushdown parser — are proofs of capability for the primitives the architecture requires, not instances of the architecture itself. This essay argues for the design on the strength of the primitives and the structural fit, and names the empirical gap explicitly.

---

## The Scanning Half Is Uncontroversial

Lexical analysis has been a state-machine workload since before the term "state machine" was common in software engineering vocabulary. A lexer sits in a mode — scanning an identifier, scanning a number, scanning inside a string literal — consumes an input byte, and either stays in its current mode, emits a completed token and returns to the start mode, or transitions to a different mode. This is the definition of a finite automaton.

Frame handles this natively, and the framepiler itself is the proof. The compiler uses 44 Frame-generated state machines (`.frs` → `.gen.rs`) for its scanning infrastructure: 15 SyntaxSkipper FSMs for per-language comment and string skipping, 15 BodyCloser FSMs for brace matching, one scope scanner for Erlang `fun...end` closures, and three sub-machines for expression scanning, context parsing, and state-variable parsing. These are the components that handle the "states within states" of recognising lexical structure across 17 target languages. The framepiler's stage-2 grammar parser is hand-written Rust recursive descent. Its stage-0 and stage-1 scanning work is Frame. This division is itself evidence for the composition thesis: the places where state machines genuinely help — bounded lexical sub-problems — get expressed in Frame; the place where they don't — the grammar-structural recursive descent — stays in Rust. The division reflects an empirical judgment by the people who know the tool best.

A minimal scanner in Frame (cookbook #53) looks like this:

```frame
@@system Scanner {
    interface:
        feed(ch: str)
        eof()
        tokens(): list

    machine:
        $Start {
            feed(ch: str) {
                if ch.isalpha():
                    self.buf = ch
                    -> $InIdent
                elif ch.isdigit():
                    self.buf = ch
                    -> $InNumber
                elif ch == '"':
                    -> $InString
                elif not ch.isspace():
                    self.emit("PUNCT", ch)
            }
        }
        $InIdent {
            feed(ch: str) {
                if ch.isalnum() or ch == "_":
                    self.buf = self.buf + ch
                else:
                    self.emit("IDENT", self.buf)
                    -> $Start
                    @@:self.feed(ch)    # replay the delimiter
            }
        }
        $InNumber { /* analogous to $InIdent */ }
        $InString { /* modal string scanner */ }
    domain:
        buf: str = ""
        out: list = []
}
```

The characteristic idiom is delimiter replay: when `$InIdent` sees a non-identifier byte, it emits the token, transitions back to `$Start`, and replays the byte through `@@:self.feed(ch)`. Frame's kernel processes the transition before the replay, so the byte arrives in `$Start` and scanning resumes cleanly. This is the pattern that makes Frame scanners ergonomic — the logic that says "this byte is both the end of one token and the beginning of another" is expressed directly instead of as a peekback-and-retry dance.

---

## The Parsing Half Is the Interesting Question

Above the token level, the parser must recognize structure that regular languages cannot describe. A flat state machine can tokenize; it cannot match nested brackets, balance parentheses, or enforce that every `begin` has an `end`. These tasks require memory that grows with input depth, which is the defining capability of a pushdown automaton.

A recursive descent parser is a pushdown automaton in disguise: the call stack is the PDA's stack, each procedure activation is a stack frame, recursive calls push and returns pop. Frame's `push$` and `pop$` operations make the same machinery available at the state-machine layer. Executing `push$` saves the current compartment — the state's name together with its state variables — onto a runtime state stack. Executing `-> pop$` restores the saved compartment as the current state, with preserved variables intact. The compartment is the activation record, the state stack is the call stack, per-state variables are local variables, and transitions to and from popped states replace procedure calls and returns.

This means Frame is, by construction, a pushdown-automaton language. Anything expressible as a PDA is expressible in Frame. Recipe #54 demonstrates this concretely with a bracket recognizer matching `[1, [2, 3], 4]` in under fifty lines.

Whether the expressive power should be *used* in the form of a pure state-machine recursive descent parser is a separate question, and the answer is mostly no. A production grammar has dozens of nonterminals. Each becomes a state; each production rule becomes a sequence of `push$` calls interleaved with returns via `-> pop$` whose values are consumed in the caller's enter handler. The linear flow that makes hand-written recursive descent readable — "parse a term, then expect a plus or minus, then parse another term" — fragments across multiple handlers in the Frame version, because every "then" that crosses a call boundary becomes an asynchronous handoff between the caller's enter handler and its return handler.

This fragmentation is the cost of expressing an inherently sequential, call-oriented computation in an event-oriented, state-oriented language. The framepiler not using Frame for its own recursive descent parser is the relevant signal: the right tool for a parser is whatever tool makes the parser clearest, and for a full-sized grammar that is usually hand-written recursive descent, not a state machine encoding of the same algorithm.

If the story ended here, the conclusion would be: Frame can express parsers; it isn't the right choice for production parsers; the cookbook recipes are proofs of capability rather than recommended architecture. That conclusion is correct but incomplete. It treats the question as binary — should the whole parser be in Frame, yes or no — when the interesting architecture is compositional.

---

## The Composition Architecture

A parser is not a monolith. Every non-trivial parser contains several distinct sub-problems, each with its own internal state:

There is the lexer, a state machine. There is an indentation tracker, if the language is indentation-sensitive, maintaining a stack of indent levels and emitting virtual `INDENT`, `DEDENT`, and `NEWLINE` tokens. There is a string and comment scanner handling escape sequences, interpolation regions, raw string modes, and multi-line comments. There is a delimiter matcher tracking the depth of parentheses, brackets, and braces. There is a disambiguator that handles locally ambiguous cases requiring bounded lookahead — distinguishing a type declaration from a multiplication in C-like languages, resolving `<` between comparison and generic in TypeScript, handling Rust's path-vs-expression ambiguity. There is an error recovery subsystem responding to syntax errors by entering panic mode until a synchronization point is reached. There is, sometimes, a Pratt-style operator precedence parser. And there is the grammar-structural backbone corresponding to the nonterminals at the top level.

Each of these is internally stateful. Each could be expressed as a Frame system. The composition architecture is: express each of them as a Frame system, and let the grammar-structural backbone — also a Frame system — coordinate them through interface calls.

The backbone is a state machine whose states correspond to major nonterminals. When a state needs to make a decision that depends on classifying upcoming input, it does not encode the classification logic inline. It calls out to a specialist — an oracle — which runs its own state machine internally and returns a verdict through a method call. The backbone transitions based on the verdict. The specialist's internal complexity is invisible to the backbone, which sees only the interface contract.

This is not a novel programming pattern in the abstract. It is the strategy pattern, or dependency injection, or the oracle machine of theoretical computer science. What makes it distinctive in the Frame context is that every component in the composition has a visible, diagrammable structure rather than being a black-box procedure readable only by reading its source line by line.

---

## A Worked Specialist: The Statement Disambiguator

To make the architecture concrete, here is a disambiguator for the classic C-style ambiguity: is `foo * bar;` a multiplication statement or a pointer-variable declaration? The specialist consumes tokens until it can decide, then returns a verdict.

```frame
@@system StatementDisambiguator {
    interface:
        classify(tokens: list, start: int): str = ""
        tokens_consumed(): int = 0

    machine:
        $Ready {
            classify(tokens: list, start: int): str {
                self.tokens = tokens
                self.pos = start
                self.consumed = 0
                -> $SeenNothing
            }
            tokens_consumed(): int { @@:(self.consumed) }
        }

        $SeenNothing {
            $>() {
                tok = self.peek()
                self.consumed = self.consumed + 1
                if tok["kind"] == "IDENT" and self.is_type_name(tok["value"]):
                    -> $SeenTypeIdent
                elif tok["kind"] == "IDENT":
                    -> $SeenPlainIdent
                else:
                    -> ("expr") $Verdict
            }
        }

        $SeenTypeIdent {
            $>() {
                tok = self.peek()
                self.consumed = self.consumed + 1
                if tok["value"] == "*":
                    -> $SeenTypeStar
                elif tok["kind"] == "IDENT":
                    -> ("decl") $Verdict    # `foo bar` — value decl
                else:
                    -> ("expr") $Verdict    # `foo;` — bare expression
            }
        }

        $SeenTypeStar {
            $>() {
                tok = self.peek()
                self.consumed = self.consumed + 1
                if tok["kind"] == "IDENT":
                    -> ("decl") $Verdict    # `foo * bar` — pointer decl
                else:
                    -> ("expr") $Verdict    # `foo *` — error or expr
            }
        }

        $SeenPlainIdent {
            $>() {
                tok = self.peek()
                self.consumed = self.consumed + 1
                if tok["value"] == "(":
                    -> ("call") $Verdict    # `foo(` — function call
                else:
                    -> ("expr") $Verdict    # any other continuation
            }
        }

        $Verdict {
            $.result: str = ""
            $>(r: str) {
                $.result = r
                @@:($.result)
                -> $Ready
            }
        }

    actions:
        peek() {
            idx = self.pos + self.consumed
            if idx >= len(self.tokens):
                return { "kind": "EOF", "value": "" }
            return self.tokens[idx]
        }
        is_type_name(name: str): bool {
            return name in self.known_types
        }

    domain:
        tokens: list = []
        pos: int = 0
        consumed: int = 0
        known_types: set = set()
}
```

The state diagram is five states. `classify()` seeds the domain variables and transitions into `$SeenNothing`; the entire classification then runs as a chain of enter handlers inside that single `classify` dispatch. Each intermediate state reads the next token, advances the cursor, and transitions to the next state. The terminal state `$Verdict` receives the classification result as an enter argument (`-> ("expr") $Verdict` passes `"expr"` to `$Verdict.$>(r: str)`), its `$>` handler stores the result, sets the return value on the still-live `classify` context via `@@:($.result)`, and transitions back to `$Ready` so subsequent `classify()` calls find the right handler. The coordinator receives one of `"decl"`, `"call"`, or `"expr"` and separately calls `tokens_consumed()` to learn how far the lookahead walked.

The pattern follows cookbook recipe #31 (Pipeline Processor — Kernel Loop Validation): the interface call drives a sequence of enter-handler transitions that all execute before the call returns. Frame's "last writer wins" rule for `@@:return` across a transition chain ensures the terminal verdict is what the caller receives.

The backbone's use site:

```frame
$ClassifyStatement {
    $>() {
        verdict = self.disambiguator.classify(self.tokens, self.cursor)
        if verdict == "decl":
            -> $ParseDecl
        elif verdict == "call":
            -> $ParseCall
        else:
            -> $ParseExpr
    }
}
```

Two observations. First, the disambiguator's scope is genuinely bounded: it answers one question over a lookahead window of three tokens with five states. A larger language will have larger disambiguators — Rust's path-vs-expression classifier could easily reach a dozen states, and TypeScript's `<` disambiguator is famously subtle. The claim is not that every disambiguator is tiny; it is that each disambiguator's scope is bounded by the question it answers, which keeps its state count linear in the complexity of that question rather than in the complexity of the whole grammar.

Second, the specialist is a legitimate unit of reuse. A C parser and a C++ parser can share a disambiguator for the same ambiguity. A language's LSP server and compiler can share a classifier. The interface contract — tokens in, verdict out — is what the specialist exposes; its internal state machine is an implementation detail that can evolve without disturbing its consumers.

---

## Two Other Specialists in Outline

Error recovery follows the same shape. A specialist whose states are recovery contexts — `$SyncingToSemicolon`, `$SyncingToBrace`, `$InExpressionRecovery`, `$InStatementRecovery` — whose events are incoming tokens, and whose return value is a resumption point. The backbone transitions into an error-handling state on syntax failure, calls the recovery specialist with the current context, receives a resumption position, and transitions to the parsing state appropriate to that position. The recovery logic, which in hand-written parsers tends to be the most ad-hoc and hardest-to-maintain part of the codebase, becomes a dedicated artifact whose structure can be designed, reviewed, and modified independently.

Indentation tracking for Python-like grammars is a pushdown machine: an indent stack, pushes on deeper indentation, pops emitting `DEDENT` tokens on shallower indentation. Frame's `push$` and `pop$` are the exact primitives for this — the indent tracker as a Frame system makes its stack discipline visible in a way that a conventional implementation does not.

---

## Prior Art and Adjacent Patterns

Three existing systems deserve mention because they solve related problems in related ways.

Tree-sitter's *external scanners* are the closest existing instance of the "specialist that knows one stateful thing" pattern. Tree-sitter is a GLR-based incremental parser generator whose grammar notation is declarative, but certain lexical phenomena — Python's indent/dedent, heredocs, string interpolation, significant whitespace — cannot be expressed in its native regex-plus-grammar notation. For these cases, tree-sitter lets the grammar author write an external scanner in C: a small stateful module with a defined interface (scan, serialize, deserialize) that the parser delegates to for specific lexical tokens. Tree-sitter's external scanners are, in structure, exactly what this essay proposes: stateful specialists with a narrow interface, called by a coordinator that handles the rest of the parse. The composition architecture extends the pattern upward — external scanners solve the lexical-level version; Frame specialists can solve the same pattern at disambiguation, recovery, and sub-grammar levels — and it gives every specialist a visible state diagram, which C external scanners do not provide.

Pratt parsing (Vaughan Pratt, 1973) is the specialist pattern avant la lettre for expressions. The top-level parser recognizes statement structure and delegates expression parsing to a Pratt sub-parser that maintains its own operator stack and handles precedence climbing internally. Most production compilers use Pratt parsing for exactly this reason — expressions are a sub-language whose parsing logic is cleaner when isolated from statement parsing, and the Pratt sub-parser is the coordinator's go-to specialist for the expression sub-grammar. The composition architecture generalizes this: what Pratt parsing does for expressions, dedicated specialists can do for disambiguation, recovery, indentation, and any other internally stateful sub-problem. The difference is that Pratt sub-parsers are usually embedded as procedural code inside the main parser; the composition architecture makes each such specialist a first-class Frame system with its own visible diagram.

MLIR's *dialect* architecture is a looser analogy, worth mentioning for the compose-and-specialize principle rather than for structural similarity. MLIR (Multi-Level Intermediate Representation) is LLVM's framework for building compilers that mix multiple intermediate representations — a machine-learning dialect, a linear-algebra dialect, a parallel-loop dialect, a target-hardware dialect — each specialized to its concerns, with defined conversion passes between them. MLIR dialects are full IRs with their own operations and type systems, not stateful specialists in the parser sense, so the parallel is at the level of architectural philosophy (decompose by specialization, compose through defined interfaces) rather than at the level of implementation mechanism.

None of these precedents undermines the Frame proposal. They establish that the specialist pattern has traction in serious tools, and suggest that applying it at the parser-coordination level with diagrammable state machines is a reasonable architectural step rather than a speculative one.

---

## The Honest Caveats

Three caveats deserve direct attention.

First, the coordinator still has to drive the input cursor. Specialists operate on a shared notion of "where are we in the input," and in this architecture that position is a domain variable owned by the coordinator. The coordinator advances the cursor, peeks at upcoming tokens, and passes the current position to specialists. Specialists report how much input they consumed but cannot autonomously pull more; they are called with a snapshot and return a verdict. For fully-buffered token streams this is not a limitation. For streaming parsers the model has to be extended to let specialists request more input without taking control of the I/O loop.

Second, the performance overhead is real, and the design burden falls on the user. Each interface call between coordinator and specialist goes through the specialist's kernel: FrameEvent construction, context stack push, router dispatch, handler execution, context pop. In Python this costs on the order of a few microseconds per call. At 200,000 tokens per second — plausible for a fast compiler frontend — a naive architecture that consults a specialist on every token would spend tens of percent of its runtime in Frame dispatch overhead. Conversely, a granular architecture that consults a specialist roughly once per 50 tokens — the range where specialists answer questions about multi-token constructs rather than single-token classifications — keeps dispatch overhead comfortably under one percent. The mitigation is granularity: design specialists that answer questions spanning multiple tokens, so the per-dispatch overhead amortizes across real work. This pushes design discipline onto the user. A specialist that is too fine-grained becomes a bottleneck; a specialist that is too coarse-grained sacrifices the clarity benefit. Finding the right boundary is a design decision the architecture does not make for you.

Third, the scope discipline for specialists is real work. A specialist that grows to encompass multiple unrelated questions loses the benefit of the pattern, because its internal state diagram becomes as large and hard to read as the monolithic parser it was meant to decompose. The claim that each specialist's diagram stays bounded by its question is aspirational unless enforced. The discipline is: one question per specialist, one state machine whose size is bounded by the scope of that question. Resisting the temptation to fold related concerns into the same specialist is the design work the architecture requires.

---

## What This Architecture Is Not

It does not claim that every parser should be built this way. For a small DSL with a clean grammar and no disambiguation difficulties, hand-written recursive descent in a hundred lines is preferable. The composition architecture pays off as the parser grows complicated — as the grammar accumulates local ambiguities, as error recovery becomes a serious concern, as multiple specialized sub-parsers emerge.

It does not claim performance equivalence with hand-tuned parsers in performance-critical applications. Frame's kernel dispatch introduces per-event overhead that a hand-written parser avoids. For parsers on the critical path of a compiler's throughput, hand-tuning will outperform.

It does not claim to replace existing parser generators. ANTLR, bison, and tree-sitter are sophisticated tools with substantial strengths. The composition architecture is a different architectural choice with different tradeoffs, most appropriate when the value of structural inspectability across the whole parser pipeline is worth more than a specialized parser-generation notation.

---

## Closing

The architecture is proposed, not documented. The primitives it relies on — `push$`, `pop$`, multi-system composition, synchronous interface calls, compile-time diagram generation — are all present in Frame today, and the framepiler's own use of 44 Frame-generated scanner FSMs is evidence that the primitives scale to production parsing-adjacent work. The missing piece is a full parser built this way, with its specialists documented and its coordinator diagram rendered, to demonstrate that the theoretical fit translates into practical clarity. That is work for a future project, not a claim this essay makes.

What this essay does claim is that the architectural shape is coherent, that the primitives exist, that prior art in tree-sitter's external scanners, in Pratt parsing, and in MLIR's dialect composition establishes the compose-specialists pattern as a serious approach in serious tools, and that Frame's contribution is to make every specialist's state structure a first-class, diagrammable artifact. The parser, if built this way, is a document of its own design.
