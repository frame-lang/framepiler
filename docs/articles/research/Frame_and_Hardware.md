# Frame and Hardware: The Parallel That Points Somewhere

There is a world where state machines never stopped being the primary abstraction for building systems. That world is hardware design.

Digital circuits are physical state machines. Flip-flops store the current state. Combinational logic computes the next state. The clock edge triggers the transition. There is no abstraction gap between the theory and the implementation. A hardware engineer designing an FSM in Verilog isn't applying a "design pattern" on top of a language that doesn't understand state — they're working in a medium where state is the fundamental unit of structure.

Software took a different path, as this series has discussed. The languages most software is written in have no concept of state. Developers infer it from boolean flags, encode it in enum-and-switch patterns, and scatter it across event handlers. Frame exists to close that gap — to give software developers a way to declare states, transitions, and event handlers as first-class constructs, and to generate clean implementations from those declarations.

What's striking, looking at Frame and hardware description languages side by side, is how closely the two approaches parallel each other — and what that parallel suggests about where Frame could go.

---

## The Synthesis Parallel

A Verilog synthesis tool and the Frame transpiler do the same kind of work at different levels of the stack.

The hardware flow: a designer writes a behavioral description of an FSM in Verilog — states as an enum, transitions in a `case` statement, outputs as assignments. The synthesis tool reads this behavioral description and generates a gate-level implementation: flip-flops for the state register, lookup tables for the combinational logic, routing for the interconnect. The designer works at the state machine level. The tool produces the implementation.

The Frame flow: a developer writes a behavioral description of a system in Frame notation — states as named blocks, transitions as `->` operators, event handlers scoped to each state. The framepiler reads this specification and generates a class in the target language: a state variable, a dispatch router, handler methods, transition logic. The developer works at the state machine level. The tool produces the implementation.

The structural similarity is exact. Both start with a declaration of states and transitions. Both generate an implementation that the designer doesn't write by hand. Both produce output that is readable and inspectable — you can look at the generated Verilog netlist or the generated Python class and understand what it does. And both free the designer from the error-prone mechanics of implementing state dispatch, letting them focus on the behavioral specification.

The difference is the target substrate. Synthesis produces gates and silicon. The framepiler produces classes and bytecode. But the abstraction layer — the thing the designer actually writes — is the same kind of object: a state machine specification.

---

## The Clock Discipline

Hardware FSMs have a property that most software state machine implementations lack: atomic transitions.

In a synchronous digital circuit, the state register only updates on a clock edge. Between clock edges, the combinational logic computes the next state from the current state and inputs, but nothing changes. The state register holds its value until the clock ticks. This means the system is never "between states" — never in a condition where some outputs reflect the old state and others reflect the new one. The transition is atomic: old state on one clock edge, new state on the next, nothing in between.

Most software state machine implementations don't have this property. In a typical hand-rolled state machine, a transition is an assignment to a state variable (`self.state = "next_state"`) that takes effect immediately, in the middle of an event handler. Code that runs after the assignment sees the new state; code that ran before it sees the old state. If the handler has side effects — logging, sending messages, updating other variables — some of those side effects happen in the context of the old state and some in the context of the new state. This is a class of bugs that hardware designers never encounter because the clock discipline prevents it.

Frame's runtime architecture is closer to the hardware model than to the typical software implementation. Frame uses a deferred transition model: when a handler executes a `->` transition, the transition is not applied immediately. Instead, the target state is cached in a `__next_compartment` variable. The handler continues to completion in the current state. Only after the handler returns does the central kernel process the transition — calling the exit handler on the old state, switching the state, and calling the enter handler on the new state.

This is architecturally analogous to a clock edge. The handler execution is the "combinational logic" phase — it computes the next state but doesn't change the current one. The kernel's transition processing is the "clock edge" — the moment the state register actually updates. Between these two phases, the system is always in a well-defined state. There is no partial transition.

This isn't a coincidence. Both designs solve the same problem: ensuring that all event processing for a given state happens entirely within that state's context, with the transition applied as a discrete step afterward. Hardware solves it with the clock. Frame solves it with deferred transitions. The underlying principle is identical.

---

## What Hardware Gets Right

Hardware's state machine practice has several properties that software generally lacks, and that are worth understanding as reference points:

**States are always explicit.** No hardware engineer encodes FSM state in a collection of boolean flags and infers the current mode from their combination. The state register is declared. The state encoding is defined (one-hot, binary, Gray code). The synthesis tool may optimize the encoding for the target technology, but the logical states are always named and visible in the source. This is the property Frame brings to software.

**Formal verification is standard practice.** Hardware engineers routinely verify FSM properties using model checkers and formal proof tools. Reachability (can every state be reached?), deadlock freedom (can the machine get stuck?), mutual exclusion (are two states ever simultaneously active in an exclusive design?), and custom temporal properties are checked before tape-out. The cost of a bug in silicon makes this an economic necessity. Software has no equivalent culture of formal verification for state machines — though Frame's extractable transition graph makes such verification structurally feasible, which is a topic for separate discussion.

**The state × event matrix is the specification.** A hardware FSM's behavior is fully described by its state transition table: for each combination of current state and input, what is the next state and what are the outputs? This table is the specification, the documentation, and (in many design flows) the direct input to synthesis. Frame's state-first code organization achieves a similar correspondence — the Frame specification reads like a state × event table, and the framepiler's GraphViz output can render it as one.

---

## What Hardware Gets Wrong (For Software)

Hardware's approach has limitations that explain why it didn't cross over to general-purpose software:

**The two-process pattern fragments state behavior.** A standard Verilog FSM uses one `always` block for combinational next-state logic and another for the sequential state register. Output logic often goes in a third block. The behavior for a single state — what it transitions to, what it outputs, what side effects it has — is split across multiple processes. This is the same fragmentation problem that Frame's state-first organization solves for software. Hardware designers tolerate it because the two-process pattern maps cleanly to the underlying hardware (combinational logic and flip-flops are physically separate), but it makes the behavioral specification harder to read.

**HDL syntax is hardware-specific.** Verilog and VHDL are designed around hardware concepts: clock edges, sensitivity lists, wire vs. reg, blocking vs. non-blocking assignment. These concepts are meaningless for software. You can't use Verilog to describe a login workflow, an agent approval gate, or a business process.

**The target is fixed.** Synthesis produces hardware. There is no path from a Verilog FSM to a Python class or a TypeScript module. The abstraction is locked to the silicon substrate.

---

## The Research Direction: A Verilog Backend

Frame's architecture — a parsed AST, a symbol table, a validated intermediate representation, and pluggable backend emitters — is designed for multi-target code generation. The framepiler already has 17 software backends and a GraphViz backend. Each backend walks the same IR and emits target-specific output. Adding a new backend is the defined extension mechanism.

A Verilog (or SystemVerilog) backend would extend this to hardware. The same Frame specification that generates a Python class for a server-side implementation could generate a Verilog module for an FPGA-accelerated version. The behavioral specification would be identical. The implementation substrate would differ.

This is not as straightforward as adding another software language backend. There are real semantic questions to resolve:

**Handler restrictions.** Frame handlers can contain arbitrary native code — function calls, I/O, string manipulation, anything the target language supports. Verilog synthesis requires that combinational logic be expressible in terms of hardware primitives. A Verilog backend would need to restrict handlers to a synthesizable subset — arithmetic, comparison, signal assignment, and not much else. The question is whether the resulting subset of Frame is still useful for hardware-targeted systems. For protocol handlers and control logic, the answer is likely yes. For general application logic, likely not.

**Timing and concurrency.** Software Frame systems process events sequentially — one event at a time, handled to completion before the next. Hardware operates in parallel — all combinational logic evaluates simultaneously on every clock cycle. A Verilog backend would need to map Frame's sequential event model to hardware's parallel execution model. For single-FSM designs, this is straightforward (one event per clock cycle is the standard pattern). For multi-system Frame compositions, the mapping is more complex.

**State encoding.** Frame's software backends represent state as a function pointer or a string-matched dispatch. A Verilog backend would represent state as a register with an enumerated encoding. The framepiler would need to choose or allow configuration of the encoding scheme (binary, one-hot, Gray code), which affects synthesis results.

**I/O mapping.** Frame interface methods would map to Verilog module ports. Domain variables would map to registers. State variables would map to registers that are reset on state entry. The mappings are natural but need to be defined precisely.

### The Development and Testing Path

The open-source toolchain for hardware simulation runs natively on macOS and integrates with VSCode, which makes this a practical research project rather than a hypothetical one.

Icarus Verilog (`brew install icarus-verilog`) provides interpreted Verilog simulation. Verilator (`brew install verilator`) compiles Verilog to C++ for high-performance simulation. Cocotb provides Python-based testbench writing that works with both simulators. GTKWave provides waveform visualization. VSCode extensions (the `mshr-h.VerilogHDL` extension or TerosHDL) provide syntax highlighting, linting via Icarus/Verilator, and in the case of TerosHDL, a state machine viewer that can extract and visualize FSMs from Verilog code.

The practical development cycle would be:

1. Write a Frame system — a protocol handler, a control FSM, a simple agent workflow.
2. Generate both Python (via the existing backend) and Verilog (via the new backend) from the same Frame source.
3. Write a cocotb testbench in Python that drives both the Python class and the Verilog module with identical input sequences.
4. Compare outputs. If the Python class and the Verilog simulation produce the same state transitions and outputs for the same inputs, the backend is behaviorally correct.
5. Optionally, synthesize the Verilog to an FPGA (Yosys + nextpnr + Project IceStorm for Lattice iCE40 targets) to verify that the generated code is not just simulatable but synthesizable.

The cocotb approach is particularly appealing because it lets you test the hardware and software implementations *in the same test harness, in the same language*. The test drives the Python class directly and the Verilog module through cocotb's simulation interface. Behavioral equivalence is checked by assertion, not by manual inspection.

---

## What This Would Mean

If a Frame system could target both software and hardware from the same source, several things become possible:

**Protocol prototyping.** Design a protocol handler in Frame, test it as a Python class, then synthesize it to an FPGA for wire-speed packet processing. The behavioral specification doesn't change. The implementation moves from software to hardware.

**Hardware-software co-design.** A system with both software and hardware components (common in embedded systems, networking, and signal processing) could share the behavioral specification. The control FSM is defined once in Frame and generated for both substrates.

**Teaching.** Students learning digital design could see the same state machine expressed in Frame notation, generated as a Python class (which they can step through in a debugger), and generated as Verilog (which they can simulate and synthesize). The connection between the behavioral abstraction and the implementation becomes tangible.

**Testing hardware designs with software ergonomics.** Before committing to a Verilog implementation, a designer could test the behavioral logic as a Python class — with all of Python's testing tools, debuggers, and libraries. Once the behavior is correct, generate Verilog from the same source.

None of this requires Frame to become a hardware design language. It requires Frame to be what it already is — a state machine specification language — with an additional backend that targets a hardware description language instead of a software programming language. The abstraction is the same. The substrate is different. And the toolchain to validate the result is free, open-source, and runs on a Mac.