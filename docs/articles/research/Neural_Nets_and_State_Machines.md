# Neural Networks and State Machines: The Bidirectional Relationship

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

Neural networks and state machines occupy opposite ends of a spectrum. State machines are discrete, explicit, inspectable, and verifiable. Neural networks are continuous, implicit, opaque, and learned. State machines are designed; neural networks are trained. A state machine's behavior can be read from its specification. A neural network's behavior emerges from billions of parameters that no human can interpret directly.

And yet the two are deeply entangled. Neural networks — particularly recurrent networks and transformers — implicitly learn state machine representations when trained on sequential tasks. Researchers can extract discrete automata from trained networks, recovering the state machine hiding inside the continuous approximation. And in the other direction, explicit state machines are increasingly used to constrain neural network behavior, providing structural guarantees that the networks themselves cannot provide.

This bidirectional relationship — neural networks learning state machines, and state machines constraining neural networks — is one of the more active areas of research at the intersection of machine learning and formal methods. It also has direct implications for Frame and for anyone building systems where AI components need to be reliable.

---

## Direction 1: Neural Networks Learning State Machines

### RNNs as Implicit Automata

A recurrent neural network is, formally, a state machine. It maintains a hidden state vector that gets updated on each input. The hidden state is the "current state." The input is the "event." The weight matrices define the "transition function." The output is computed from the hidden state.

The difference from a classical state machine is that the hidden state is a continuous vector in high-dimensional space, not a discrete named mode. An RNN doesn't have a "Locked" state and an "Unlocked" state — it has a 256-dimensional vector whose position in that space *corresponds* to behavioral modes, but the correspondence is learned, implicit, and distributed across dimensions.

Foundational work by Omlin and Giles [1] showed that when second-order RNNs are trained on regular languages (languages that can be recognized by finite state machines), the hidden state vectors cluster into discrete groups that correspond to the states of a DFA for that language. By applying clustering algorithms to the output space of recurrent state neurons, they could extract deterministic finite-state automata from the trained networks. Earlier work by Cleeremans, Servan-Schreiber, and McClelland [2] had observed similar clustering in simple Elman networks, establishing that this is a general property of recurrent architectures trained on regular languages.

This was among the first demonstrations that neural networks, given appropriate training data, will spontaneously learn finite automata representations. The states are there. They're just not visible without analysis.

### Transformers and the Chomsky Hierarchy

Transformers lack the explicit recurrent state of RNNs — they process the entire context window at once via attention rather than maintaining a carried-forward hidden state. This raises a natural question: can transformers simulate state machines at all?

The research here is extensive and nuanced. A comprehensive survey by Strobl, Merrill, Weiss, Chiang, and Angluin [3] maps transformer expressiveness against the Chomsky hierarchy — the nested classification of formal languages by complexity. The findings are striking and sometimes counterintuitive.

Standard transformers (without chain-of-thought or scratchpad) occupy a limited position in the hierarchy. They can recognize some regular languages but fail on others — even simple ones like parity (determining whether a binary string has an even number of ones). This is a language that a two-state finite automaton recognizes trivially, yet transformers trained on it fail to generalize to longer sequences.

However, transformers with chain-of-thought — where the model is allowed to produce intermediate reasoning steps — gain significantly more power. Merrill and Sabharwal [4] proved that, under specific architectural conditions (projected pre-norm) and standard complexity-theoretic assumptions, a linear number of intermediate decoding steps enables transformers to recognize all regular languages — which is equivalent to simulating arbitrary finite state automata. Their work further showed that with a polynomial number of steps, chain-of-thought transformers are equivalent to the complexity class P. These are conditional results, not unconditional proofs — they depend on the architectural assumptions — but they establish that chain-of-thought is not just a prompting technique. It is a mechanism that fundamentally extends transformers' computational power in a way that connects directly to automata theory.

Delétang et al. [5] conducted a landmark empirical study training multiple neural architectures on tasks across the Chomsky hierarchy. Their results showed that LSTMs (not simple RNNs) can solve some tasks above the regular language level (like counting, which requires counter-machine capabilities), while standard transformers struggle with regular tasks that require tracking state across long sequences. It is worth noting that transformers can and do learn to track state in practice on many tasks — the limitations appear when testing generalization to arbitrary-length inputs on formal language tasks. Memory-augmented architectures — networks with access to an external stack or tape — can climb higher in the hierarchy, suggesting that explicit memory structures (which are what state machines provide) are a crucial capability that standard architectures lack.

### Extracting State Machines from Neural Networks

If neural networks implicitly learn state machines, can we extract the learned automaton and inspect it?

Weiss, Goldberg, and Yahav [6] adapted Angluin's classical L* algorithm [7] — an algorithm for learning finite automata from queries and counterexamples — to extract DFAs from trained RNNs. Their method treats the RNN as a black-box oracle: the L* algorithm queries the RNN to determine whether it accepts or rejects specific strings, then constructs a minimal DFA consistent with the RNN's behavior. This represented a different methodology from the earlier clustering-based approach of Omlin and Giles [1] — rather than partitioning the network's continuous state space and mapping transitions between clusters, it uses the classical exact-learning framework. The two approaches have different tradeoffs: clustering works directly on the internal representations but is sensitive to the granularity of the partition, while L*-based extraction treats the network as a black box but can recover minimal automata more reliably.

More recently, Adriaensen and Maene [8] extended the L*-based approach to transformers trained on regular languages. Using the querying process to identify state equivalences in the transformer's behavior, they were able to extract finite automata (specifically Moore machines, a variant of DFAs with output labels) that approximate the transformer's learned behavior. When a transformer has genuinely learned a regular language, the extraction process recovers the target automaton, with states that correspond to meaningful distinctions in the input.

This line of work is significant for interpretability, though important caveats apply. Current extraction methods work on small transformers (single-layer, modest hidden dimensions) trained on simple formal languages. Whether the techniques scale to production-scale models trained on natural language is an open question — one that is explicitly addressed in the research directions section below.

### Neural Networks as Universal Finite-State Machines

A recent preprint by Dhayalkar [9] provides formal arguments that feedforward neural networks can exactly simulate deterministic finite automata. The key claims: DFA transitions are linearly separable, binary threshold activations suffice for exponential state compression (encoding N states in O(log N) dimensions), and the Myhill-Nerode equivalence classes (the mathematical foundation of minimal DFAs) can be embedded into continuous latent spaces while preserving separability. The paper also formalizes an expressivity boundary: fixed-depth feedforward networks are claimed to be expressively equivalent to DFAs and strictly less powerful than pushdown automata.

These results, if confirmed, would place neural network expressiveness precisely within the automata hierarchy. However, this is a single-author preprint that has not yet undergone peer review, and the claims should be treated accordingly — as an interesting formal contribution awaiting independent verification, not as established results on the same footing as the peer-reviewed work cited above.

---

## Direction 2: State Machines Constraining Neural Networks

The other direction of the relationship is the one most relevant to building reliable systems: using explicit state machines to constrain what neural networks can do.

### The Agent Workflow Problem

An AI agent powered by an LLM makes decisions — which tool to call, what arguments to pass, when to ask for human approval, how to handle errors. The LLM's decisions are probabilistic, influenced by the prompt, the context, and the model's training. They cannot be formally guaranteed.

But the *structure* of the agent's workflow — which phases it passes through, what events it responds to in each phase, which transitions are possible — can be formally specified and enforced. A state machine around the LLM constrains the consequences of its decisions without constraining the decisions themselves.

This is the separation that makes state-machine-constrained agents qualitatively different from unconstrained ones. The LLM might be tricked by a prompt injection into *wanting* to execute a tool without approval. But if the state machine doesn't have a transition from the validation state to the execution state that bypasses the approval state, the want can't become action. The LLM is powerful within the boundaries of the current state. It is powerless to alter which states are reachable.

This is the architecture that Frame implements for agent systems. Frame's "impossible by construction" property works as follows: in a Frame state machine, if a state doesn't declare a handler for an event, that event has no dispatch path in the generated code. There is no code to bypass because there is no code — the capability doesn't exist in that state. The structural constraint is enforced by the absence of mechanism, not by the presence of a runtime check. (The full argument for why this provides qualitatively different guarantees from runtime checking is developed in the companion article "Impossible by Construction.")

### Why Structure Matters More Than Checking

The alternative to structural constraint is runtime checking: `if not approved: raise Error`. This works until a code path doesn't go through the check, a refactoring moves the check, or a new feature bypasses it. Every runtime check is a convention that depends on discipline. Every structural absence is a guarantee that depends on architecture.

The relevance to neural networks is that LLMs are the ultimate source of unpredictable control flow. A traditional program executes the code path the developer wrote. An LLM-driven agent executes whatever the model decides, within whatever structure the developer provides. The less structure, the more the agent's behavior depends on the model's judgment. The more structure, the more the agent's behavior is bounded regardless of the model's judgment.

This is not a claim that Frame-based state machines eliminate all safety risks in AI agents. They constrain the *workflow structure* — which states are reachable, which transitions are possible, which events are handled in which states. They do not constrain the *content* of decisions within a state (what the LLM generates, what arguments it passes to a tool) or protect against all classes of failure (implementation bugs in the generated code, unexpected interactions with external systems). The value is in making one important class of safety property — ordering and gating constraints on the workflow — structurally enforceable rather than convention-dependent.

---

## Where the Two Directions Meet

The bidirectional relationship between neural networks and state machines creates possibilities that neither direction provides alone.

### Extraction as Specification Recovery

If a trained neural network has implicitly learned a state machine, extracting that automaton and expressing it in a formal notation (like Frame) would produce a readable, verifiable specification of the network's learned behavior. This extracted specification could be:

**Inspected.** A developer could read the extracted state machine and understand what the model learned — which behavioral modes it distinguished, which transitions it identified, which inputs it treats as equivalent. This is mechanistic interpretability rendered as a state diagram rather than as attention head analysis.

**Compared to the intended behavior.** If the developer has an expected state machine (perhaps written in Frame as a specification), the extracted automaton can be compared to it. Differences — extra states, missing transitions, unexpected equivalences — reveal where the model's learned behavior diverges from the intended behavior.

**Verified.** The extracted automaton can be checked for properties: reachability, deadlock freedom, mandatory waypoints. If the model was supposed to learn a protocol that always passes through an authentication state, the extracted automaton can be checked for that property. A violation means the model learned an incorrect protocol.

**Used as a starting point.** If the extracted automaton is mostly correct, it could be refined by hand — adding missing transitions, removing incorrect ones — and then used as the explicit state machine going forward, replacing the neural network's implicit representation with an explicit, maintainable specification.

It is important to note that this is a research direction, not a deployable technique. Current extraction methods [6] [8] work on small networks trained on formal languages. Scaling to production models is an open problem (see below). The description above represents what would become possible *if* extraction scales — it should not be read as a description of current capabilities.

### Hybrid Architectures

A system could combine an explicit Frame state machine for workflow structure with a neural network for within-state decision-making. The state machine determines *which* state the system is in and *which* events are available. The neural network determines *how* to respond to those events — which tool to call, what arguments to use, what text to generate.

This is already the architecture that Frame-based agent systems use. The research opportunity is to tighten the integration: could the neural network be *aware* of the state machine's structure? Could it be trained with the transition graph as an input, so that it learns to make decisions that are compatible with the available transitions? Could chain-of-thought reasoning be structured around the state machine's states, so that each reasoning step corresponds to a state transition?

### Learning-Guided State Machine Design

The extraction research suggests an approach to state machine design that starts with data rather than specification. Instead of designing a state machine from requirements (top-down), you could:

1. Collect traces of a system's behavior (logs, event sequences, user sessions)
2. Train a neural network to predict the system's next action from its history
3. Extract a discrete automaton from the trained network
4. Express the extracted automaton in Frame notation
5. Refine the specification by hand — adding safety constraints, removing unintended behaviors
6. Generate the implementation from the refined Frame specification

This is a form of specification mining — using machine learning to discover the state machine that a system's behavior implies, then making it explicit and controllable. The neural network is a tool for discovery. Frame is the tool for formalization and implementation.

---

## The Theoretical Backdrop

The research on transformers and the Chomsky hierarchy provides a theoretical framing for why explicit state machines matter for AI systems.

Transformers, despite their enormous practical capabilities, have formal limitations in state tracking. Standard transformers without chain-of-thought provably cannot recognize all regular languages — the simplest class in the Chomsky hierarchy, recognizable by finite automata. They can fail at tasks that a two-state machine solves trivially. In practice, transformers do learn to track state on many tasks — the limitations manifest when testing generalization to arbitrary lengths on formal language benchmarks. But the theoretical results suggest that precise, long-range state tracking is not a natural strength of the architecture.

An explicit state machine externalizes the state tracking. The transformer doesn't need to track which phase the workflow is in, which approvals have been obtained, which error recovery attempts have been made — the state machine tracks all of that. The transformer focuses on what it's good at: making decisions within a defined context.

This division of labor — explicit structure for state management, neural networks for decision-making — aligns with the theoretical findings. It puts each component in the part of the Chomsky hierarchy where it operates best: the state machine handles the regular-language-level structure (states and transitions), and the transformer handles the complex, context-dependent decisions within each state.

---

## Open Research Questions

The following questions are identified as open in the cited literature. They represent genuine research gaps, not speculative feature requests.

**Can state machines be reliably extracted from production-scale LLMs?** Current extraction work [6] [8] uses small, single-layer transformers trained on regular languages. Scaling this to large language models trained on natural language is a fundamentally different challenge. The implicit state representations in a production LLM are vastly more complex, distributed across many layers, and entangled with other capabilities. Whether meaningful discrete automata can be extracted from these models is an open question identified by multiple research groups.

**What is the relationship between chain-of-thought and state machine simulation?** Merrill and Sabharwal [4] proved (under architectural conditions) that chain-of-thought gives transformers the sequential processing steps needed to simulate finite automata. Zhang et al. [10] provided mechanistic evidence that Transformer+CoT learns implicit FSA representations, with late-layer MLP neurons encoding distinct automaton states. An open question is whether structuring chain-of-thought prompts around explicit state machine states — so that each reasoning step corresponds to processing one event in one state — would make the LLM's reasoning more reliable and inspectable.

**Does formal language pre-training improve LLM capabilities on structured tasks?** A recent study by Hu, Petty, Shi, Merrill, and Linzen [11] found that pre-pretraining transformers on formal languages with hierarchical structure — before training on natural language — led to lower loss on natural language and better linguistic generalization compared to other formal languages. They found modest support for the hypothesis that the formal language should fall within the computational limitations of the architecture. Whether pre-training on state machine traces specifically would improve an LLM's ability to work within explicit state machine frameworks is an untested extension of this work.

The following questions are the author's speculations, informed by but not directly identified in the cited research:

**Could state machine properties be learned from examples?** Instead of specifying safety properties by hand ("every path to Executing passes through Approved"), could they be learned from positive and negative execution traces? A neural network trained to distinguish safe from unsafe traces might identify the structural properties that make the difference — properties that could then be expressed as verifiable constraints on the state machine.

**What is the right interface between a state machine and a neural network?** Frame's current approach treats the LLM as an action within a handler — the state machine calls the LLM when it needs a decision. Could the integration be tighter? Could the state machine's current state be injected into the LLM's prompt or context in a way that improves decision quality?

---

## The Convergence

State machines and neural networks are not competing paradigms. They are complementary tools that operate at different levels of the behavioral stack.

Neural networks are unsurpassed at learning patterns from data, generalizing to new situations, and making context-dependent decisions. They are poor at maintaining precise state over long sequences, enforcing structural constraints, and providing verifiable guarantees about their behavior.

State machines are unsurpassed at maintaining explicit state, enforcing transition constraints, and providing inspectable, verifiable behavioral specifications. They cannot learn from data, generalize to unseen situations, or make nuanced decisions.

The research reviewed here suggests that the productive future is integration, not competition. Neural networks that learn within state machine constraints. State machines whose structure is informed by what neural networks learn from data. Formal verification applied to the state machine layer, with neural network behavior bounded by the states that structure permits.

Frame's position in this landscape is as the explicit, inspectable, verifiable layer — the structure that makes neural network behavior bounded and auditable. The research into extraction, hybrid architectures, and learning-guided design suggests that this layer could become not just a constraint mechanism but a bridge: connecting what neural networks learn to what formal methods can verify, through the common language of states and transitions.

---

## References

[1] Omlin, C. W. and Giles, C. L. "Extraction of Rules from Discrete-Time Recurrent Neural Networks." *Neural Networks*, 9(1), 41–52, 1996.

[2] Cleeremans, A., Servan-Schreiber, D., and McClelland, J. L. "Finite State Automata and Simple Recurrent Networks." *Neural Computation*, 1(3), 372–381, 1989.

[3] Strobl, L., Merrill, W., Weiss, G., Chiang, D., and Angluin, D. "What Formal Languages Can Transformers Express? A Survey." *Transactions of the Association for Computational Linguistics*, 12, 543–561, 2024.

[4] Merrill, W. and Sabharwal, A. "The Expressive Power of Transformers with Chain of Thought." *The Twelfth International Conference on Learning Representations (ICLR)*, 2024. arXiv:2310.07923.

[5] Delétang, G., Ruoss, A., Grau-Moya, J., Genewein, T., Wenliang, L. K., Catt, E., Cundy, C., Hutter, M., Legg, S., Veness, J., and Ortega, P. A. "Neural Networks and the Chomsky Hierarchy." *The Eleventh International Conference on Learning Representations (ICLR)*, 2023. arXiv:2207.02098.

[6] Weiss, G., Goldberg, Y., and Yahav, E. "Extracting Automata from Recurrent Neural Networks Using Queries and Counterexamples." *Machine Learning*, 113(5), 2877–2919, 2024. arXiv:1711.09576.

[7] Angluin, D. "Learning Regular Sets from Queries and Counterexamples." *Information and Computation*, 75(2), 87–106, 1987.

[8] Adriaensen, R. and Maene, J. "Extracting Finite State Machines from Transformers." *Workshop on Mechanistic Interpretability, ICML*, 2024. arXiv:2410.06045.

[9] Dhayalkar, S. R. "Neural Networks as Universal Finite-State Machines: A Constructive Deterministic Finite Automaton Theory." Preprint, 2025. arXiv:2505.11694.

[10] Zhang, Y., Du, W., Jin, D., Fu, J., and Jin, Z. "Finite State Automata Inside Transformers with Chain-of-Thought: A Mechanistic Study on State Tracking." *Proceedings of the 63rd Annual Meeting of the Association for Computational Linguistics (ACL)*, 2025. arXiv:2502.20129.

[11] Hu, M. Y., Petty, J., Shi, C., Merrill, W., and Linzen, T. "Between Circuits and Chomsky: Pre-pretraining on Formal Languages Imparts Linguistic Biases." *Proceedings of the 63rd Annual Meeting of the Association for Computational Linguistics (ACL)*, pp. 9691–9709, 2025. arXiv:2502.19249.