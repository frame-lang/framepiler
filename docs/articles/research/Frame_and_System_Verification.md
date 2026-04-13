# Frame and Formal Verification: Closing the Spec-Code Gap

Formal verification has a problem, and it isn't the math.

The math works. TLA+ can specify distributed protocols with mathematical precision. Model checkers can exhaustively explore state spaces and find violations of safety and liveness properties. Proof assistants can establish correctness guarantees that no amount of testing can match. The theory is mature, the tools exist, and the results — when applied — are impressive. Amazon has used TLA+ to find subtle bugs in AWS protocols that testing missed. Hardware companies routinely verify chip designs before committing to silicon.

The problem is the gap between the specification and the code.

A developer writes a TLA+ spec, verifies it, then writes the implementation in Python or TypeScript. The spec lives in one file, in one language, using one set of concepts. The code lives in another file, in another language, using a different set of concepts. The developer is the bridge, and the bridge is made of discipline. Over time — as the code evolves, as features are added, as edge cases are patched — the spec and the code drift apart. The spec describes the system as it was designed. The code describes the system as it is. They may or may not agree, and nobody knows for sure unless someone manually re-verifies the correspondence.

This is the spec-code gap, and it is the primary reason formal verification hasn't been widely adopted in mainstream software development. The math isn't the barrier. The workflow is.

Frame's architecture offers a structural way to close this gap. Not because Frame does formal verification — it doesn't, currently. But because Frame's single-source, multi-backend design means that a formal specification and the implementation code can be derived from the same source, making drift impossible.

---

## How Verification Works Today

To understand what Frame could change, it helps to understand how formal verification is currently practiced.

### TLA+ and Model Checking

TLA+ (Temporal Logic of Actions), created by Leslie Lamport, describes a system as a state machine: an initial-state predicate and a next-state relation that defines how the system transitions between states. The developer writes invariants ("the account balance is never negative") and temporal properties ("every request eventually gets a response"), and the TLC model checker exhaustively explores the reachable state space to find violations.

A TLA+ specification of a simple approval workflow might look like:

```
Init == state = "idle" /\ approved = FALSE

Next == \/ (state = "idle" /\ state' = "validating" /\ UNCHANGED approved)
        \/ (state = "validating" /\ state' = "awaiting_approval" /\ UNCHANGED approved)
        \/ (state = "awaiting_approval" /\ approved' = TRUE /\ state' = "executing")
        \/ (state = "awaiting_approval" /\ approved' = FALSE /\ state' = "rejected")
        \/ (state = "executing" /\ state' = "complete" /\ UNCHANGED approved)

Safety == state = "executing" => approved = TRUE
```

The `Safety` invariant states that whenever the system is in the "executing" state, approval must be true. TLC will check this across all reachable states. If any execution trace reaches "executing" without approval, the model checker reports the violation with a counterexample trace.

This is powerful. But notice what happens next: the developer takes this verified specification and goes to implement it in Python. They write a class with methods, conditionals, and state variables. The implementation is hand-written, informed by the spec but not derived from it. The spec guarantees that the *design* is correct. It says nothing about whether the *code* faithfully implements the design.

### Alloy and Bounded Model Checking

Alloy, created by Daniel Jackson at MIT, uses relational logic with bounded model checking. You describe structures and constraints, and the Alloy Analyzer finds concrete instances that satisfy or violate them. Alloy is particularly good at finding counterexamples — concrete scenarios where a property fails.

For state machines, Alloy can answer questions like: "Is there a sequence of events that reaches the Executing state from Idle without passing through Validating?" This is a reachability query on a constrained graph — exactly what Alloy is designed for.

### SPIN and Promela

SPIN is a model checker for concurrent systems. Its input language, Promela (Process Meta-Language), describes concurrent processes communicating via channels. SPIN is the natural tool for verifying properties of multi-component systems — where multiple state machines interact and the bugs live in the interaction, not in any individual machine.

For Frame's multi-system compositions — where several Frame systems interact through their interfaces — SPIN could verify properties of the composition: "Can these two systems deadlock? Can they reach a state where both are waiting for the other?"

---

## The Spec-Code Gap

Every verification tool described above operates on a specification that is separate from the implementation. The workflow is:

1. Write the specification (in TLA+, Alloy, Promela, or another formalism)
2. Verify the specification (the model checker finds bugs or confirms properties)
3. Write the implementation (in Python, TypeScript, Rust, or another language)
4. Hope the implementation matches the specification

Step 4 is where the gap lives. The specification and the implementation are written in different languages, stored in different files, maintained by different processes. Nothing structurally connects them. Their agreement depends entirely on the developer's discipline — and that discipline erodes under the normal pressures of software development: deadline urgency, team turnover, incremental feature additions, bug fixes that patch the code without updating the spec.

The result is that formal verification, in practice, is mostly applied to *initial designs* and rarely maintained through the *lifecycle* of the software. The spec is written and verified when the system is first designed. The code is written to match. Then the code evolves and the spec doesn't. A year later, the spec describes a system that no longer exists.

---

## What Frame Changes

Frame specifications and implementation code are not separate artifacts. A developer writes a Frame system — states, transitions, event handlers — and the framepiler generates both the implementation (a Python class, a Rust struct, a TypeScript class) and, via the GraphViz backend, a visual representation of the transition graph. Both outputs come from the same source. If the source changes, both outputs change. They can't drift because they're derived from the same AST.

This single-source property is the key to closing the spec-code gap. If the framepiler also had a backend that emitted a formal specification — in TLA+, Alloy, Promela, or another formalism — then the specification would also be derived from the same source. The verified spec and the running code would be guaranteed to describe the same state machine, because they're both generated from the same Frame system.

The developer's workflow would change from:

1. Write the spec → 2. Verify the spec → 3. Write the code → 4. Hope they match

to:

1. Write the Frame system → 2. Generate both the code and the spec → 3. Verify the spec → 4. Consistency is structural

Step 4 becomes a structural guarantee instead of a discipline requirement. The spec can't drift from the code because there is no separate spec to maintain. Changing the Frame system regenerates both artifacts. Verification can be re-run after every change as part of the build process, not as a one-time design activity.

---

## What Could Be Checked

Different kinds of properties require different levels of analysis. Some can be checked by the framepiler itself. Others require external tools.

### Graph-Structural Properties (Checkable During Compilation)

The framepiler already builds an internal representation of the transition graph — it uses this to generate GraphViz output. The same graph can be analyzed for structural properties:

**Reachability.** Can every state be reached from the start state? An unreachable state is either dead code or a specification error. This is a straightforward graph traversal.

**Deadlock freedom.** Is there a reachable state (other than a designated terminal state) from which no transition is possible? A state with no outgoing transitions and no event handlers is a potential deadlock. This is detectable by graph inspection.

**Mandatory waypoints.** "Every path from $Idle to $Executing passes through $Approved." This is a graph reachability query with constraints — computable by removing the waypoint node and checking whether the target is still reachable from the source. If it's not reachable without the waypoint, the waypoint is mandatory.

**Transition completeness.** "Every state handles every interface event, or explicitly delegates it to a parent." This is checkable by comparing the set of interface events against the handlers declared in each state (and its parent chain).

These properties don't require temporal logic or a model checker. They're static properties of the transition graph. The framepiler could check them as a validation pass — like a type checker, but for state machine structure. Violations would be reported as compile-time diagnostics.

### Temporal Properties (Requiring External Model Checkers)

More sophisticated properties involve reasoning about execution traces over time:

**Liveness.** "The system always eventually reaches $Complete or $Failed." This requires showing that no reachable cycle exists that excludes both terminal states — that the system can't loop forever without terminating. This is a temporal property (expressed in TLA+ as a fairness condition) that requires trace analysis beyond static graph inspection.

**Ordering with data dependencies.** "If the retry count exceeds the maximum, the system transitions to $Failed within the next 3 events." This involves both the transition graph and the data values in domain variables. The framepiler can see the graph; it can't (in general) reason about the native code that updates the data.

**Interaction properties.** "In a multi-system composition, system A never sends a message to system B while system B is in $Maintenance." This involves the joint state space of multiple Frame systems and requires reasoning about their composition — which is where SPIN/Promela excels.

For these properties, the framepiler would need to emit specifications in a format that external model checkers can consume. A TLA+ backend would emit the state machine as a TLA+ specification. A Promela backend would emit it as a SPIN-checkable process. The developer would then run the model checker on the generated spec and get verification results.

### The Property Annotation Question

One open question is where properties should be written. Options include:

**In a separate file.** The traditional approach — properties live in a TLA+ spec or a Promela model. This is flexible but re-introduces a gap: the properties file must be maintained alongside the Frame source.

**Inline in the Frame source.** Properties expressed directly in Frame notation, alongside the states and transitions they constrain. This keeps everything in one place but requires defining a property syntax within Frame. For graph-structural properties ("every path to $Executing passes through $Approved"), a simple annotation syntax might suffice. For temporal properties, the syntax would need to express liveness and fairness conditions, which risks importing the complexity of temporal logic into Frame's deliberately concise notation.

**Derived from conventions.** Some properties could be inferred from Frame's existing structure. A state with no outgoing transitions could be automatically flagged as a potential deadlock. A state named with a `$Failed` or `$Complete` suffix could be automatically treated as a terminal state for liveness checking. This requires no new syntax but limits the expressible properties to conventions the framepiler recognizes.

The right answer probably involves all three: automatic checking of graph-structural properties during compilation, a lightweight annotation syntax for common safety properties, and external model checker integration for complex temporal properties. The boundary between "what the framepiler checks" and "what an external tool checks" should be drawn at the point where the framepiler would need to reason about native code semantics — which it deliberately treats as opaque.

---

## The Boundary of Opacity

This is the fundamental tension in Frame's approach to verification. Frame handlers contain native code — Python, Rust, TypeScript, whatever the target language is. The framepiler parses the Frame constructs (states, transitions, event dispatching) but passes native code through without analysis. It doesn't know what the native code does.

This means the framepiler can verify properties that depend only on the state machine structure — states, transitions, which events are handled where — but cannot verify properties that depend on the behavior of the native code inside handlers. "Every path to $Executing passes through $Approved" is structural and verifiable. "The approval flag is never set to true without user input" depends on what the native code in the approval handler actually does, which is outside Frame's scope.

This boundary is the same one that exists in hardware verification: synthesis tools verify the structural properties of the design, but the functional correctness of the logic — does this adder actually add? — requires simulation or formal proof at a different level. Frame can verify the workflow structure. The correctness of the actions within each state is the responsibility of the native code and its own testing/verification tools.

For many important safety properties, the structural level is sufficient. "The agent can't execute without approval" is a structural property. "The approval check correctly validates the user's credentials" is a functional property. Frame addresses the first. Traditional testing and code review address the second. The value is that the structural properties — which are among the hardest to verify in hand-rolled state machines because they span the entire codebase — become checkable from a single source.

---

## The Practical Path

None of this requires building a model checker from scratch. The tools exist. What's needed is backends that emit specifications in formats those tools consume:

**A TLA+ backend** would emit a TLA+ module from a Frame system. States become a TLA+ enumeration. Transitions become the next-state relation. Interface events become actions. Invariants could be derived from Frame annotations or written separately. The developer would run TLC on the generated spec to check properties.

**An Alloy backend** would emit an Alloy model focused on reachability queries. "Can $Executing be reached without passing through $Approved?" becomes a natural Alloy constraint-satisfaction problem.

**A Promela backend** would emit SPIN-compatible process descriptions for multi-system Frame compositions. Each Frame system becomes a Promela process. Interface calls between systems become channel communications. SPIN verifies properties of the composition.

**A built-in graph analyzer** would check structural properties during compilation, without any external tool. This is the lowest-hanging fruit and the highest-impact addition — most developers will never run TLC, but they would benefit from automatic reachability and deadlock checking every time they compile.

The framepiler's pipeline — source → AST → symbol table → validation → IR → backend emission — already has the right shape for all of these. The AST carries the state and transition information. The symbol table (the Arcanum) catalogs every state, event, and variable. The validation pass already checks some structural properties. Adding graph analysis to the validator and formal-specification emission to the backend layer are extensions of existing infrastructure, not new architectures.

The result would be a development workflow where writing a Frame system automatically produces: implementation code (in your target language), a visual transition graph (via GraphViz), and optionally, a verifiable formal specification (via TLA+, Alloy, or Promela) — all from the same source, all guaranteed to describe the same state machine. The spec-code gap doesn't narrow. It disappears.