# The Missing Primitive: Why Programming Languages Don't Have State

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

Software has state. Every developer knows this. We spend enormous effort managing it, debugging it, documenting it, and cleaning up after it. State management is consistently rated among the hardest problems in software engineering.

And yet the languages we use to build software have no concept of it.

Not *data* — languages handle data fine. Variables, types, structures, objects, collections. The tooling for declaring and manipulating data has never been richer. The problem is *logical state*: the behavioral mode a system is in at any given moment. The thing that determines not what values the system holds, but what the system *does* — what it responds to, what it ignores, and what happens next.

No mainstream programming language treats this as a first-class concept. The consequences of that absence touch every codebase of nontrivial complexity, and they follow a pattern so consistent it deserves a name: the state gap.

---

## What Logical State Actually Is

Consider an elevator. At any moment it's doing one of a few things: going up, going down, stopped with doors closed, stopped with doors open. These are its logical states — its behavioral modes. Each mode determines what the elevator does when it receives an input. A "floor requested" event means something completely different depending on whether the elevator is idle or already in motion.

This is distinct from the elevator's *data* — its current floor number, the queue of requested floors, the weight on the platform. Data and state interact, but they aren't the same thing. The current floor is a number. "Going up" is a mode of operation.

Now consider how this gets expressed in code. In every mainstream language, the answer is: indirectly. There is no `state GoingUp { ... }` declaration. Instead, developers infer logical state from data values using conditional expressions:

```
if (direction > 0) {
    // "Going Up" behavior
} else if (direction < 0) {
    // "Going Down" behavior  
} else {
    // "Stopped" behavior
}
```

The logical state exists, but only implicitly — as a human interpretation superimposed on ranges of a variable's value. The language has no knowledge that `direction > 0` means "Going Up." It's just a boolean test.

This works. It has always worked. And it is the root of a class of problems that plagues every large codebase.

---

## Three Emergent Pathologies

The absence of explicit state doesn't cause one problem. It causes three, and they compound.

### 1. Implicit Logical State

When state is inferred from data through conditional expressions, the same logical state can be expressed in multiple equivalent ways. `direction > 0`, `direction >= 1`, `!(direction <= 0)` — all identify the same mode. Nothing in the language connects the boolean expression to the concept it represents.

This means a developer reading the code must reverse-engineer the logical model from scattered conditionals. There's nothing about `(direction >= 0)` that announces "this means Going Up" to someone encountering the code cold. The mapping from data range to behavioral meaning lives entirely in the original developer's head — or, optimistically, in a comment that may or may not be current.

The fragility compounds. When the data model changes — say, `direction` shifts from a signed integer to an enum, or a new sentinel value is introduced — every conditional that inferred state from the old model must be found and updated. The compiler won't help, because the compiler doesn't know these conditionals represent state.

#### The Binary Partition Problem

There is a deeper structural issue at work here. A boolean test divides a value space into exactly two partitions: the values for which it's true, and everything else. But logical state spaces are rarely binary. An elevator has three or four modes. A network connection has five or six. A business process might have a dozen.

Consider water. It exists in three common phases — solid, liquid, gas — determined by temperature. The test `(temp < 32)` partitions the number line into two regions: below freezing, and *everything else*. But "everything else" conflates liquid and gas into a single undifferentiated mass. The test doesn't say "solid vs. liquid" — it says "solid vs. not-solid," and the code in the `else` branch must silently serve double duty for two completely different physical realities.

This mismatch between binary boolean logic and multi-valued logical state is baked into every `if/else` construct in every language. Each conditional imposes a two-way cut on a space that may require three, four, or more partitions. Developers compensate by chaining conditionals — `if/else if/else if/else` — but the chain is ordered, and each link inherits the residual ambiguity of the ones above it. The final `else` always means "everything I didn't think of," which is precisely where new states hide.

#### The Double Negative Trap

Implicit state also forces contorted logic when a state is defined by what it *isn't* rather than what it *is*. Determining whether water is liquid requires proving it's not solid and not gas:

```
bool isLiquid = (temp >= 32 && temp < 212);
```

That's manageable. But defensive programming demands we guard against invalid states, which produces the negation of an already-indirect test:

```
if (!(temp >= 32 && temp < 212)) {
    throw new IllegalStateError();
}
```

The developer is now reasoning through a double negative: "it is not the case that the temperature is both above freezing and below boiling." This is cognitively expensive for humans to parse, error-prone to modify, and entirely a consequence of the language having no way to say `if (not in state Liquid)`. The state is never named; it is perpetually re-derived from data through logical gymnastics.

### 2. State Fragmentation

Object-oriented languages organize code by *method* — by the event being handled. An `onFloorReached()` method, an `onButtonPressed()` method, an `onTimerExpired()` method. Within each method, conditional logic determines which behavioral mode the system is in, then executes the appropriate response.

This means the behavior for any single logical state is scattered across every method in the class. To understand what the system does when it's "Going Up," you must read every event handler, find the branch that corresponds to the up-state, and mentally stitch those fragments together.

#### The Inverted Matrix

One way to see this clearly is to think of system behavior as a matrix. The rows are states — the behavioral modes the system can be in. The columns are events — the inputs the system can receive. Each cell contains the behavior for that combination: what happens when this event arrives in this state.

|              | onFloorReached | onButtonPressed | onTimerExpired | status       |
|:-------------|:---------------|:----------------|:---------------|:-------------|
| **Going Up**    | check & maybe stop | queue the floor     | —              | "moving up"  |
| **Going Down**  | check & maybe stop | queue the floor     | —              | "moving down"|
| **Stopped**     | —              | start moving    | close doors    | "idle"       |

Humans reason about this matrix by row: "When the elevator is going up, here are all the things that can happen." Each row is a coherent story about one behavioral mode.

OOP code is organized by column: "When a button is pressed, here's what might happen depending on which state we're in." Each method contains fragments of every row, interleaved with conditional logic to pick the right fragment.

This is the organizational inverse of how humans think. The developer's brain must continuously defragment — assembling a coherent picture of one behavioral mode from pieces distributed across every method in the class. The analogy to disk fragmentation is precise: the data is all there, but the read head must seek across the disk to collect it. The cost isn't measured in milliseconds but in cognitive load, onboarding time, and bug rate.

### 3. State Entanglement

This is where implicit state becomes actively dangerous.

Suppose our elevator has two states — Going Up and Going Down — encoded as positive and negative values of `direction`. Now we need to add a third state: Stopped. We choose `direction == 0` for it.

The specification is clear: three states, three value ranges. But the existing code was written for two states. Every conditional in every method partitions the value space into two halves. The new third state doesn't get its own partition — it gets silently absorbed into whichever existing branch happens to capture its value.

If the original test was `direction >= 0`, then `Stopped` (direction == 0) is now entangled with `Going Up`. The system will treat a stopped elevator as if it's going up. Not because anyone intended this, but because the boolean logic predates the state it now accidentally includes.

The water example makes this more vivid. Suppose a system was built for three phases — solid, liquid, gas — and a physicist needs to add plasma (ionized gas at extreme temperatures). The existing conditional chain partitioned the temperature space into three regions. Plasma doesn't have its own region — it's been silently absorbed into the `else` clause of the gas test, or into whatever the final branch happens to be. The system will treat plasma as ordinary gas, with potentially dangerous consequences in any application where the distinction matters.

Fixing this requires visiting every conditional in every method and surgically disentangling the new state from the old ones. Miss one, and you have a system that is simultaneously in two logical states depending on which method you're looking at. The classic symptom: code that "mostly works" but exhibits bizarre behavior in specific edge cases that correspond to the entangled state.

The `else` clause is the most reliable trap. It's a catch-all — anything not explicitly matched falls through to `else`. When a new state is introduced, `else` silently absorbs it. The code compiles, the tests that don't cover the new state still pass, and the bug ships.

---

## Why This Isn't Just a Discipline Problem

The standard response to these pathologies is better engineering practices: use enums instead of magic numbers, use the State pattern, write more comments, add more tests, be more careful.

These mitigations help. They are also workarounds for a language-level deficiency, and they have limits.

**Enums** name the states but don't group the behavior. You still have `switch (state)` inside every event handler, which means state fragmentation persists. The behavior for `State.GoingUp` is still scattered across every method. Looking at the state × event matrix, enums label the rows but don't let you *write code organized by row*. You're still writing column by column.

**The State pattern** (GoF) groups behavior by state using polymorphic classes. This solves fragmentation — each state subclass contains all its behavior in one place. But it introduces a different kind of complexity: a class hierarchy that must be maintained in parallel with the main class, indirection that makes control flow harder to trace, and no enforcement that the set of states is complete or that transitions between them are valid.

**Switch statements on enum values** are arguably the most common approximation of explicit state in mainstream languages. They help with implicit state (the enum names the states) but don't help with fragmentation (the switch is still inside each event handler, not grouping behavior by state). And they do nothing about entanglement — adding a new enum value doesn't cause a compile error at every switch that lacks a case for it, in most languages.

Each of these mitigations addresses one or two of the three pathologies while leaving the others intact. The underlying issue — that state is not a language-level concept — remains.

### The Legitimate Role for Conditionals

This argument is emphatically *not* that conditional logic should be eliminated. There are situations that genuinely require testing data to determine state:

**Initial or restored state.** When a system starts up or is restored from persistence, the correct logical state must be determined from whatever data is available. This is a legitimate conditional: "given this temperature, which phase is the water in?"

**External state.** Determining facts about the environment — sensor readings, network conditions, user permissions — that exist outside the system's control and may change without notice.

**Volatile internal state.** Data that can be modified by external processes or concurrent threads while the system is running.

In all these cases, boolean logic is the right tool. The problem isn't that conditionals *exist* — it's that they're *everywhere*. In a well-structured system, state-determining logic would be confined to a single function — a "probe" that examines the data, determines the correct logical state, and transitions the system to it. Every other piece of code would simply operate within the current state without re-deriving it.

The difference is between a system that determines its state once and then *is* in that state (responding to events according to its current mode) versus a system that re-derives its state from raw data on every single event, in every single method, through a fresh thicket of boolean logic each time. The first is manageable. The second is the source of the pathologies described above.

---

## The Genealogy of the Gap

The state gap isn't an oversight. It's an inheritance — a consequence of where programming languages came from and what problems they were designed to solve. To understand why state was never made a primitive, it helps to trace the lineages.

### Lineage 1: Records on Tape

The earliest programming was data processing in the most literal sense. Punch cards, magnetic tape, sequential files. A COBOL program is structured around the DATA DIVISION: you declare your record layouts — field names, types, widths, positions — and then write a PROCEDURE DIVISION that reads records, transforms fields, and writes records. The fundamental unit of organization is the *record*. The program is a pipeline that processes them.

C inherited this DNA directly. A `struct` is a record. The language gives you tools to declare data layouts and write functions that operate on them. The functions are organized by *what they do* — sort, search, transform, allocate — not by *what mode the system is in*. The struct is a passive container, just like a row on tape.

OOP's contribution was wrapping functions around the struct. A class is a struct with methods attached. Smalltalk, then C++, then Java took the record and said: "what if the data knew how to operate on itself?" But the organizing principle didn't change. The class is still centered on *data* — its fields, its type, its inheritance hierarchy of data-bearing types. Methods are organized by operation ("here's what you can do to/with this data"), not by behavioral mode ("here's what this thing does when it's in *this* situation").

This is why OOP has no concept of state. It was never trying to model behavior-over-time. It was trying to model *things that have data and operations on that data*. An elevator object has fields (current_floor, direction, door_status) and methods (move, stop, open_doors). The methods operate on the fields. The question "what behavioral mode is the elevator in?" is not something the paradigm was built to express — because the paradigm grew out of records, not out of control systems.

Even the "message passing" framing of original Smalltalk-style OOP doesn't rescue this. Messages map to methods, and methods are organized by operation. Sending `buttonPressed` to an elevator object invokes a single method that must internally figure out which mode the elevator is in. The message is an event, but the dispatch is to an operation, not to a state.

The lineage runs: **tape records → C structs → C++ classes → Java/C#/Python/TypeScript objects**. At every step, the organizing principle is *data and operations on data*. State is inferred from data values. It was never given its own seat at the table.

### Lineage 2: Lambda Calculus

Functional programming has different roots but arrives at a similar gap by a different route. Its ancestry is mathematical: lambda calculus, Church's formalism for computable functions, the idea that computation is the evaluation of expressions rather than the mutation of state. In a deep sense, functional programming was *designed* to make state unnecessary — or at least invisible.

Immutability, pure functions, referential transparency — these are all strategies for eliminating the concept of "the system is in a mode" in favor of "the system transforms inputs to outputs." This works beautifully for transformations: data in, data out, no side effects. It is the tape-processing model elevated to mathematical elegance — the record pipeline without the mutable record.

But when a system genuinely *has* behavioral modes — when it must respond differently to the same input depending on its history — functional languages have to encode that structurally. Haskell does it through algebraic data types and pattern matching: you define a sum type for your states, carry the current state as a value through your function calls, and match on it everywhere you need state-dependent behavior. This is more honest than OOP's approach — the state is at least *visible* as an explicit value rather than hidden inside object fields. But it doesn't solve fragmentation. You still pattern-match inside each function (column-first in the matrix). The code for "what happens in state X across all events" is still scattered across every function that matches on the state type.

Erlang/OTP recognized this gap and built `gen_statem` as a dedicated behavior module — essentially an admission that state machines are common enough to deserve first-class support, even if the language doesn't provide it as syntax. It's one of the closest things to a state primitive in any widely-used ecosystem, but it's a library convention, not a language construct.

The lineage runs: **lambda calculus → Lisp → ML → Haskell/Erlang/Elixir**. The organizing principle is *expressions and transformations*. State is either avoided (pure FP) or carried as explicit data and pattern-matched — but never a structural unit of code organization.

### Lineage 3: The Road Not Taken

There is a third intellectual lineage: control theory, automata theory, statecharts. This tradition has always modeled systems as *modes and transitions*. A finite automaton is defined by its states, its inputs, and its transition function. Harel's statecharts (1987) added hierarchy, concurrency, and history to the model. The UML state machine diagram gave it a visual notation that became an industry standard for *designing* systems.

But this lineage never produced a mainstream general-purpose programming language. It produced modeling tools, verification frameworks, code generators, domain-specific languages, and academic formalisms. The theory is mature. The tools exist. What's missing is the integration into the languages developers actually write in every day.

The result is a peculiar split: developers routinely *design* systems using state diagrams on whiteboards, then go implement them in languages that have no way to express what they just drew. The state model lives in the diagram. The code lives in the language. The two are connected only by the developer's discipline — and that connection degrades with every maintenance cycle.

### How the Lineages Play Out

The genealogy explains the consistency of the gap across language families. Every mainstream language descends from one of the first two lineages, and neither lineage was designed to model behavioral modes.

**C** — pure Lineage 1. Structs and functions. State is flags and switch statements.

**C++, Java, C#** — Lineage 1 with OOP. The struct became a class; the organizing principle is still data-and-operations. The `switch` on an enum is the closest approximation to state dispatch.

**Python, JavaScript** — Lineage 1, dynamically typed. The same data-and-operations model with less compile-time enforcement. Boolean flags and string comparisons become the default state encoding.

**TypeScript** — Lineage 1 with a type system strong enough to approximate exhaustiveness checking through discriminated unions. One of the closest mainstream approaches to catching missing state handlers at compile time, but still organized by method, not by state.

**Rust** — Lineage 1 with Lineage 2 influences. `enum` with data and exhaustive `match` give the strongest compile-time state support in any mainstream imperative language. But code is still organized by function, and transitions are still hand-managed.

**Go** — Lineage 1, deliberately minimal. `iota` enums and switch statements. No exhaustiveness checking, no state grouping. The simplicity-first philosophy doesn't extend to making state management simpler.

**Haskell** — Pure Lineage 2. Algebraic data types can encode state at the type level, making invalid states unrepresentable. This is the most theoretically rigorous approach, but state-dependent behavior is still distributed across pattern-match sites in individual functions.

**Erlang/Elixir** — Lineage 2 with a pragmatic escape hatch. `gen_statem` provides dedicated state machine support as a library behavior — the closest any mainstream ecosystem comes to first-class state. But it's a framework convention, not syntax.

The common thread: every language provides tools that *can be used* to manage state, but none provides a construct that *is* state. The developer must always build the abstraction from lower-level parts, and the language cannot verify that the abstraction is complete or consistent.

---

## The Coroutine Irony

There is a revealing exception to the claim that languages don't generate state machines. They do — in exactly one case, and they hide the result.

An `async` function with multiple `await` points is a state machine. The compiler knows this. When you write:

```
async function fetchAndProcess(url) {
    let response = await fetch(url);
    let data = await response.json();
    let result = await transform(data);
    return result;
}
```

the compiler transforms this into a state machine with four states — one for each suspension point. The code between `await` points becomes the body of each state. A transition occurs when the awaited value arrives. The compiler generates the dispatch logic, manages the current-state variable, and handles resumption at the correct point.

This is a real, generated state machine. C#, Rust, JavaScript, Python, Kotlin, and others all perform this transformation. The compiler has the full machinery: state enumeration, dispatch tables, transition logic. It builds exactly the kind of structure that the language refuses to offer as a general-purpose construct.

The irony is precise. The language can *generate* state machines from sequential-looking code, but it only does so for the specific case of asynchronous suspension and resumption. The developer cannot name the states, cannot define which events each state handles, cannot control the transitions, and cannot inspect the generated machine. The states are defined by where the `await` happens to fall — by the mechanics of suspension, not by the logical modes of the system.

Generators and iterators are the same transformation applied to value production. A Python generator with `yield` statements compiles to a state machine that tracks which `yield` was last reached. Each `yield` is a state boundary. The compiler generates the machine; the developer never sees it.

In both cases, the language designers recognized that a specific kind of problem — suspending and resuming execution — is naturally modeled as a state machine, and they built compiler support for generating one. But they treated it as a special-purpose optimization rather than exposing the underlying abstraction. The developer gets `async/await` syntax for the narrow case of waiting on I/O, but no equivalent syntax for the general case of "this system has behavioral modes and transitions between them."

This is the state gap in miniature. The compiler already knows how to do it. The language just won't let you ask.

---

## Where It Hurts Most

The state gap is an inconvenience in simple programs. It becomes a serious engineering problem in systems with certain characteristics:

**Event-driven systems.** GUIs, protocol handlers, device controllers — anything that responds to asynchronous events from multiple sources. Each event handler must determine the current state and respond accordingly. As states multiply, the conditional logic in each handler grows combinatorially. Looking at the state × event matrix, every new row (state) adds a cell to every existing column (event handler), and every new column adds a cell to every existing row. The matrix grows in two dimensions; the code must account for every cell.

**Workflow orchestration.** Business processes with multiple phases, approval gates, error recovery, and retry logic. The "happy path" is simple; the full state space — including every combination of partial failure, timeout, manual override, and rollback — is where complexity explodes.

**AI agent systems.** This is a rapidly growing category where the state gap is particularly acute. An AI agent that starts as a simple prompt-response loop quickly grows retry logic, tool dispatch, error recovery, human approval gates, mode switching, and persistent checkpointing. Each addition weaves new conditionals into the existing tangle. The result is hundreds of lines of nested if/else managing implicit states through boolean flags — a state machine in denial.

**Long-lived services.** Systems that must handle not just individual requests but sessions, connections, or processes with lifecycles spanning many events over time. The longer the lifecycle, the more states accumulate, and the harder it becomes to verify that every state handles every possible event correctly.

In all these cases, the same progression occurs: the system starts simple, complexity grows incrementally, each increment adds conditionals rather than states (because the language has no states to add), and eventually the codebase reaches a point where no single developer can hold the full state model in their head. At that point, bugs become structural rather than incidental — they arise not from individual mistakes but from the fundamental difficulty of reasoning about implicit state distributed across fragmented code.

---

## What First-Class State Would Look Like

If a language treated state as a primitive, what would that mean in practice?

**State declarations.** A way to name the behavioral modes of a system, as directly as you name its variables or methods. Not an enum that *represents* states, but a construct that *is* a state — containing the behavior that applies when the system is in that mode.

**Event handlers scoped to state.** Instead of methods that contain switch statements to determine the current state, each state would declare which events it handles and what it does in response. The behavior for "Going Up" would be in one place, containing all the event responses for that mode. Code organization would match the mental model — and the state × event matrix: state first, events within state. You would read the system row by row, not column by column.

**Unhandled events as a defined concept.** If a state doesn't declare a handler for an event, the language defines what happens — either nothing (the event is ignored), or it's forwarded to a parent state, or it's a compile error. The key is that the absence of a handler is meaningful, not accidental. There's no `else` clause to accidentally capture it.

**Behavioral inheritance.** States could delegate unhandled events to a parent state, creating a hierarchy where common behavior is defined once and specialized behavior is defined per-state. This replaces the defensive programming pattern — all those guard clauses checking "am I in the right state for this operation?" — with a structural guarantee: if a state doesn't handle an event, the event either does nothing or propagates to a parent that defines the default response (like throwing an error). The guard logic is written once in the parent, not repeated in every method.

**Transitions as a language construct.** Moving from one state to another would be a dedicated operation, not an assignment to a variable. This makes transitions visible, searchable, and analyzable. A tool could extract the complete transition graph from the source code and verify properties: "Is every state reachable? Are there dead states? Can the system get stuck?"

**State-scoped variables.** Variables that exist only while the system is in a particular state, initialized on entry and discarded on exit. This prevents a common bug class where variables from a previous state carry stale values into a new one.

**Confined state probing.** State-determining logic — the conditionals that probe data to figure out which state the system should be in — would have a defined home: entry points, restore functions, or dedicated probe methods. The rest of the system would operate *within* a known state, not perpetually re-derive it. The language would make the distinction structural: code that determines state lives here, code that operates within state lives there.

None of these ideas are new. Statecharts (Harel, 1987) formalized most of them. The UML state machine diagram captures them visually. Numerous academic languages and libraries have implemented them. But they have not made it into the core syntax of the languages that most software is written in.

---

## The Cost of the Gap

The absence of first-class state imposes costs that are real but diffuse — easy to attribute to "complexity" in general rather than to a specific missing abstraction.

**Onboarding time.** New developers on a project must reverse-engineer the state model from implicit conditionals. This is one of the most time-consuming parts of joining a complex codebase, and it's entirely a consequence of state not being declared. If the system's behavior were expressed as a state × event matrix — states down the left, events across the top, behavior in each cell — a new developer could read the entire behavioral specification in minutes. Instead, they must reconstruct that matrix mentally from code organized in the transposed direction.

**Bug density in state-heavy code.** The conditional logic that manages implicit state is disproportionately represented in bug trackers. Off-by-one in a state boundary, forgotten case in a switch, stale flag from a previous state — these are structural consequences of the state gap.

**Resistance to change.** Adding a new state to a system with implicit state management requires touching every event handler. This creates a strong incentive to avoid adding states — to instead overload existing states with additional conditional logic, making the problem worse. The codebase resists the very refactoring that would clarify it.

**Invisible safety properties.** In a system with explicit states and transitions, safety properties are visible: "The system never enters Executing without passing through Validating" is a statement about the transition graph. In a system with implicit state, the equivalent property is distributed across dozens of conditionals in multiple methods. Verifying it requires tracing every code path. A single missed path is a security vulnerability or a correctness bug.

**Defensive programming overhead.** Without explicit state, every method that should only execute in certain states must include its own guard logic. These guards are the double-negative conditionals discussed earlier — "if the system is NOT in a valid state for this operation, throw an error." In a system with first-class state and behavioral inheritance, invalid-state handling is defined once in a parent state and inherited automatically. The guards disappear from the application code because the structure makes them unnecessary.

---

## A Thought Experiment

Imagine that the languages we use had no concept of *functions*. You could still write programs — you'd just inline everything, use `goto` for reuse, and manage return addresses manually. It would work. People would develop patterns and conventions for it. They'd write books about "structured goto usage." They'd build libraries that approximate function call semantics using macros.

And then someone would say: "What if we just made functions a language feature?"

State is in that position today. We have the workarounds. We have the patterns. We have the libraries. What we don't have — in the languages most software is written in — is the thing itself.

The question isn't whether explicit state management is valuable. Every developer who has used a state machine knows it is. The question is why we continue to build it from parts in every project, rather than expecting our languages to provide it as a primitive.

The genealogy provides the answer. Our languages descend from two traditions — records-on-tape and lambda calculus — and both were designed to model data processing: read, transform, write. Neither was designed to model systems that *are in behavioral modes and transition between them in response to events*. That's a third kind of thing, and it has a rich theoretical tradition (automata, statecharts, control theory) that produced everything except a mainstream language. The theory won in academia. The records won in industry. And the gap between them is where the complexity lives.

But the software we build has changed. Event-driven systems, workflow orchestration, and AI agents are not edge cases — they are increasingly the mainstream of software development. The state gap that was tolerable in a world of sequential record processing becomes a serious liability in a world where systems are always in some behavioral mode, always responding to asynchronous events, always transitioning between phases.

The primitive is missing. The workarounds are inadequate. The question is how much longer we keep building it from parts.