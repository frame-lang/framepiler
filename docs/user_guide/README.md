# The Frame User Guide

Frame is a domain-specific language for specifying state machines. You write Frame specifications inside your native source files, and the transpiler expands them into production code in your target language.

This guide teaches Frame progressively — each chapter builds on the last. All examples use Python as the target language, but everything applies to all supported languages.

## Chapters

1. [Introduction](01-introduction.md) — What Frame is and how it works
2. [Your First System](02-first-system.md) — Install, write, transpile, run
3. [States and Handlers](03-states.md) — Multiple states, event dispatch, transitions
4. [Events and the Interface](04-events.md) — Public API, parameters, return values
5. [Actions](05-actions.md) — Calling native code from handlers
6. [Variables](06-variables.md) — Domain variables, state variables, state parameters
7. [Transitions in Depth](07-transitions.md) — Enter/exit events, forwarding, history
8. [Hierarchical State Machines](08-hsm.md) — Parent states and event delegation
9. [Async](09-async.md) — Asynchronous interface methods and actions
10. [Advanced Topics](10-advanced.md) — System context, persistence, multi-system files, visualization

## Quick Reference

| Syntax | Meaning |
|--------|---------|
| `@@target python_3` | Declare target language |
| `@@system Name { }` | Declare a state machine |
| `-> $State` | Transition to a state |
| `=> $^` | Forward event to parent state |
| `push$` | Save current state to history stack |
| `-> pop$` | Return to saved state |
| `$.varName` | State variable |
| `@@.param` | Interface parameter access |
| `@@:return` | Interface return value |
