# Reinforcement Learning and State Machines: The Foundation Hiding in Plain Sight

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

Reinforcement learning is built on state machines. This is not a metaphor — though the correspondence is looser than the opening sentence suggests, and the caveats matter.

A Markov Decision Process — the mathematical object at the foundation of virtually all RL — shares deep structural properties with state machines. It has states, actions (which are events by another name), a transition function that maps (state, action) pairs to probability distributions over next states, and a reward function that assigns value to transitions [1]. The Markov property — the future depends only on the current state, not on the history of how the agent got there — is a probabilistic independence condition, not the same thing as the defining property of finite state machines. But the shared intuition is real: in both formalisms, the current state is all you need to determine what happens next. History is irrelevant once the state is known.

An important caveat: the formal equivalence between MDPs and finite state machines applies cleanly only when the state space is finite and discrete. Many practical RL environments have continuous observation spaces (joint angles, pixel values, sensor readings), where the mapping to a finite-state model is approximate at best. The discussion that follows is most precise for finite MDPs, and should be read as increasingly analogical as state spaces become continuous and high-dimensional.

The RL community and the automata theory community developed largely in parallel, with different vocabulary, different tools, and different concerns. RL researchers talk about policies, value functions, and reward shaping. Automata researchers talk about states, transitions, and language recognition. The underlying structures are related. The conversations have rarely intersected — until recently.

In the last several years, that separation has begun to close. Reward machines define reward functions as explicit finite state machines. Hierarchical RL decomposes complex behaviors into temporal abstractions that share structural properties with hierarchical state machines. Safe RL synthesizes "shields" — reactive systems that constrain an agent's actions to enforce safety specifications. These developments suggest a growing recognition that explicit state structure can improve RL systems — though each area has significant open problems and none should be treated as settled.

This piece traces those connections and their implications. **A note on perspective:** the second half of this article discusses how these research threads relate to Frame, a state machine language the author has a stake in. The connections are genuine, but the interpretation is advocacy, not neutral survey. The reader should evaluate the Frame-specific claims accordingly.

---

## MDPs and State Machines: Structural Relatives, Not Identical Twins

It's worth being precise about what the formal connection is and what it isn't.

A finite state machine has: a finite set of states, a finite set of inputs, and a transition function that maps (state, input) to next state. The transition function is deterministic (for DFAs) or nondeterministic (for NFAs), but either way it's about state identity — which state you end up in.

A Markov Decision Process [1] has: a finite set of states S, a finite set of actions A, a transition probability function p(s'|s, a) that maps (state, action) to a *distribution* over next states, and a reward function r(s, a, s') that assigns a scalar reward to each transition. An MDP is a probabilistic system with an optimization objective attached.

The structural similarity is that both are indexed by (state, input/action) pairs. A Q-table in tabular Q-learning [2] maps (state, action) pairs to expected cumulative reward values. An automata transition table maps (state, input) pairs to next states. These are structurally parallel but semantically different: the Q-table is about *valuation* (how good is this action in this state?), not about *transition identity* (which state do I go to?). The analogy is illuminating but should not be mistaken for identity.

What the structural parallel does suggest is that tools for reasoning about state machines — explicit state declarations, transition graphs, state-dependent behavior — might be useful for reasoning about RL systems. The research reviewed below explores this hypothesis from multiple angles.

---

## Reward Machines: Making Task Structure Explicit

The most direct bridge between RL and automata theory is the reward machine, introduced by Toro Icarte, Klassen, Valenzano, and McIlraith [3] [4].

The observation is simple but powerful: RL methods treat reward functions as black boxes, but in practice, reward functions are *programmed* by humans who have structure in mind. A reward function for "pick up the coffee, then deliver it to the office" has sequential structure — the coffee must be picked up *before* it can be delivered. A reward function for "patrol areas A, B, and C in order" has cyclic structure. These structures are state machines: the reward depends not just on the current observation but on the *phase* of the task.

A reward machine is a finite state machine where:
- States represent phases of the task (not yet started, coffee picked up, delivered)
- Transitions are triggered by propositional formulas over environment events (the agent reaches the coffee location, the agent reaches the office)
- Each state assigns a reward value for being in that phase

The RL agent learns a separate Q-function for each state of the reward machine — effectively learning a different policy for each phase of the task. This decomposition improves sample efficiency because the agent is solving several smaller subproblems rather than one large one.

The empirical results show significant improvements in sample efficiency on tasks with sequential structure, though the magnitude varies by task complexity [3]. The JAIR 2022 paper [4] — which won the 2023 IJCAI-JAIR Best Paper Prize — provides the most complete treatment, including automated reward shaping, task decomposition, and counterfactual reasoning with off-policy learning. An important caveat: QRM's convergence guarantees apply in the tabular case with exact (known) reward machines. Scaling to deep RL required additional work on counterfactual experience replay (CRM), covered in [4].

Subsequent work showed that reward machines can be *learned* from experience rather than specified by the user [5], though this result applies in a constrained setting: the agent must be able to observe propositional events that correspond to the reward machine's edge labels. Learning reward machines in fully unstructured environments remains an open problem.

---

## Hierarchical RL: Options and Temporal Abstraction

The options framework, introduced by Sutton, Precup, and Singh [6], provides the formal theory of temporal abstraction in RL. An *option* is a temporally extended course of action — not a single primitive action, but a closed-loop policy that executes over multiple timesteps.

An option has three components:
- An **initiation set**: the states in which the option can be started
- An **internal policy**: the behavior the option follows while active
- A **termination condition**: a stochastic function β(s) giving the probability of termination in each state

There are suggestive parallels between options and states in hierarchical state machines. Both represent behavioral modes that persist over time — an option executes its internal policy across multiple timesteps, just as an HSM state handles events across its lifetime. Both can be composed hierarchically. A set of options defined over an MDP constitutes a semi-Markov decision process (SMDP) [6] — a higher-level decision problem where the "actions" are options rather than primitives.

However, the correspondence should not be overstated. Options have *stochastic* termination (the option ends with probability β(s) in each state), while HSM state transitions are typically deterministic. Options have initiation sets that constrain where they can be *started*, which has no exact HSM counterpart. And options include a full internal policy (mapping states to action probabilities), while HSM states contain event handlers. These are related abstractions — both decompose complex behavior into modular, temporal units — but they are not isomorphic.

Dietterich's MAXQ decomposition [7] extends the idea of hierarchical value decomposition to arbitrary subtask hierarchies. Parr and Russell's HAMs (Hierarchy of Abstract Machines) [8] define hierarchies of finite-state controllers that constrain the agent's policy. Both demonstrate that imposing explicit structure on RL — constraining which policies are considered — can dramatically improve learning efficiency. The common thread is that explicit behavioral structure, declared rather than learned from scratch, helps RL systems.

---

## Safe RL: Shields as Runtime Constraints

Safety is one of the hardest problems in RL. An agent optimizing for reward may discover policies that are highly rewarding but unsafe — a robot that moves fast by ignoring collision avoidance, a trading agent that maximizes returns by taking catastrophic risks. Constrained RL approaches try to balance reward with safety constraints, but constraints expressed as penalties in the reward function are soft — they can be outweighed by sufficiently high rewards.

Shielding, introduced by Alshiekh, Bloem, Ehlers, Könighofer, Niekum, and Topcu [9], takes a fundamentally different approach. A **shield** is a reactive system that monitors the agent's proposed actions and blocks those that would violate safety specifications expressed in temporal logic.

The shield architecture works in two modes [9]. In **pre-shielding**, the shield provides the agent with a list of safe actions before it chooses; the agent can only select from safe actions. In **post-shielding**, the agent chooses freely, and the shield overrides the choice if it would violate the specification. In both cases, the shield is a state machine — a reactive system with states, inputs (proposed actions), and outputs (allowed actions or overrides).

Subsequent work extended shielding to online settings where the shield is computed during execution [10] and to partially observable environments [11].

**An important distinction regarding Frame:** Shielding and Frame's "impossible by construction" property share the *goal* of preventing unsafe transitions, but they achieve it through different mechanisms. A shield is a *runtime monitor* that intercepts and overrides actions — the agent's policy may still internally "want" to take the blocked action, and the shield intervenes. Frame's approach (as described in the companion article "Impossible by Construction") means the dispatch path for the unsafe action doesn't exist in the generated code — there is no code to intercept because there is no code. Shielding is closer to a runtime check (albeit a formally verified one); Frame's approach is structural absence. These are architecturally different, and conflating them would misrepresent both.

That said, both approaches benefit from explicit state structure: the shield is defined over states and transitions (of the safety-relevant environment model), and Frame systems define states and transitions (of the agent's workflow). The shared insight is that safety properties are easier to enforce, verify, and reason about when the state structure is explicit.

---

## POMDPs and the Value of Explicit State

Not all RL problems have fully observable states. In a Partially Observable MDP (POMDP) [12], the agent cannot directly observe the environment's state — it receives observations that provide incomplete information. The agent must maintain a *belief state*: a probability distribution over possible environment states, updated as new observations arrive.

Exact POMDP planning is PSPACE-complete [13], though this applies to the exact case. In practice, approximate methods (PBVI, SARSOP, and others) are widely used and tractable on many real problems. The intractability result does not mean POMDPs are generally unsolvable — it means the worst case is very hard.

One approach that makes POMDPs more tractable is to provide the agent with *explicit* state information about the task structure. Reward machines serve this role in partially observable settings [5]: the reward machine's state represents the task phase, which the agent can track even when the environment state is partially hidden. The reward machine transforms a POMDP into a collection of simpler problems — because the task phase (the reward machine state) is known even when the environment state is not.

This suggests a general principle: when part of the relevant state can be made explicit and maintained by an external structure — a reward machine, a state machine, a workflow controller — the learning problem becomes easier. The explicit structure provides the scaffolding that the agent would otherwise have to learn from scratch.

---

## Model-Based RL and Learned Dynamics

In model-based RL, the agent learns a model of the environment's dynamics — a learned transition function that predicts the next state given the current state and action. World models [14] learn latent representations of environment dynamics that can be used for planning and imagination.

A speculative connection — not established by the cited literature, but worth noting as a research direction — is whether learned world models develop internal representations that correspond to discrete behavioral modes. If the environment has genuine mode structure (highway driving vs. city driving vs. parking), a sufficiently capable world model might learn latent states that cluster around these modes, analogously to how RNN hidden states cluster into DFA states when trained on regular languages. However, the RNN-DFA correspondence has been demonstrated only for simple formal languages, and whether anything similar occurs in the high-dimensional, continuous, stochastic environments typical of model-based RL is an open empirical question.

---

## Implications for Frame

*The following section represents the author's perspective on how the research threads above connect to Frame. These are genuine connections but should be read as advocacy for a specific tool, not as neutral assessment.*

Each of the research threads described above found, in its own domain, that making state structure explicit improves outcomes — reward machines improve sample efficiency through task decomposition, shielding enables formal safety guarantees through state-based monitoring, hierarchical RL benefits from declared behavioral abstractions. These are distinct findings in distinct subfields, not a single unified conclusion. But the pattern is suggestive: when researchers in different areas of RL add explicit state structure to their systems, things tend to get better. Whether this pattern generalizes — and whether a unified tool for expressing state structure would serve across these areas — is an open question. Each area also has significant open problems (how to specify reward machines automatically, how to scale shielding to continuous settings, how to learn options without hand-specified hierarchies), and none should be treated as settled.

Frame provides the notation, the tooling, and the code generation infrastructure for expressing state machines. Where the connections are specific:

**Reward machines** are structurally similar to Frame systems where handlers assign rewards. A Frame-based reward machine would be declared in the same notation as any other Frame system, and the framepiler could generate the decomposed Q-learning infrastructure. Whether this would improve on existing reward machine implementations is an engineering question, not a settled one.

**The options/HSM parallel** is suggestive but imprecise. Frame's HSM support (parent states, child states, event forwarding, enter/exit handlers, state-scoped variables) provides structural primitives that *resemble* hierarchical RL's temporal abstractions. The differences (stochastic vs. deterministic termination, initiation sets vs. event handlers) mean the mapping is not direct, and adapting Frame to serve as a hierarchical RL framework would require extensions to handle stochastic termination.

**Shields** and Frame's approach share the goal of safety enforcement but differ in mechanism (runtime monitoring vs. structural absence). A Frame system could serve as a *specification* for a shield — defining the safety-relevant states and allowed transitions — even if the enforcement mechanism is different from Frame's native dispatch architecture.

**Task phase tracking** in partially observable settings is what Frame systems provide by construction. The current state is always known, even when the environment is uncertain. This is a real and immediate benefit for agent workflows built on Frame.

---

## References

[1] Sutton, R. S. and Barto, A. G. *Reinforcement Learning: An Introduction*, 2nd edition. MIT Press, 2018.

[2] Watkins, C. J. C. H. "Learning from Delayed Rewards." PhD thesis, King's College, Cambridge, 1989.

[3] Toro Icarte, R., Klassen, T. Q., Valenzano, R., and McIlraith, S. A. "Using Reward Machines for High-Level Task Specification and Decomposition in Reinforcement Learning." *Proceedings of the 35th International Conference on Machine Learning (ICML)*, PMLR 80, 2018.

[4] Toro Icarte, R., Klassen, T. Q., Valenzano, R., and McIlraith, S. A. "Reward Machines: Exploiting Reward Function Structure in Reinforcement Learning." *Journal of Artificial Intelligence Research*, 73, 173–208, 2022.

[5] Toro Icarte, R., Waldie, E., Klassen, T. Q., Valenzano, R., Castro, M. P., and McIlraith, S. A. "Learning Reward Machines for Partially Observable Reinforcement Learning." *Proceedings of the 33rd Conference on Advances in Neural Information Processing Systems (NeurIPS)*, pp. 15497–15508, 2019.

[6] Sutton, R. S., Precup, D., and Singh, S. "Between MDPs and Semi-MDPs: A Framework for Temporal Abstraction in Reinforcement Learning." *Artificial Intelligence*, 112, 181–211, 1999.

[7] Dietterich, T. G. "Hierarchical Reinforcement Learning with the MAXQ Value Function Decomposition." *Journal of Artificial Intelligence Research*, 13, 227–303, 2000. (Earlier conference version: ICML 1998.)

[8] Parr, R. and Russell, S. "Reinforcement Learning with Hierarchies of Machines." *Advances in Neural Information Processing Systems 10 (NIPS 1997)*, MIT Press, 1998.

[9] Alshiekh, M., Bloem, R., Ehlers, R., Könighofer, B., Niekum, S., and Topcu, U. "Safe Reinforcement Learning via Shielding." *Proceedings of the 32nd AAAI Conference on Artificial Intelligence (AAAI)*, 2018.

[10] Könighofer, B., Rudolf, J., Palmisano, A., Tappler, M., and Bloem, R. "Online Shielding for Reinforcement Learning." *Innovations in Systems and Software Engineering*, 19, 379–394, 2023.

[11] Carr, S., Jansen, N., Junges, S., and Topcu, U. "Safe Reinforcement Learning via Shielding under Partial Observability." *Proceedings of the 37th AAAI Conference on Artificial Intelligence (AAAI)*, 37(12), 14748–14756, 2023. arXiv:2204.00755.

[12] Kaelbling, L. P., Littman, M. L., and Cassandra, A. R. "Planning and Acting in Partially Observable Stochastic Domains." *Artificial Intelligence*, 101(1–2), 99–134, 1998.

[13] Papadimitriou, C. H. and Tsitsiklis, J. N. "The Complexity of Markov Decision Processes." *Mathematics of Operations Research*, 12(3), 441–450, 1987.

[14] Ha, D. and Schmidhuber, J. "World Models." 2018. arXiv:1803.10122.