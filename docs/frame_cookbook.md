# Frame Cookbook

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

52 recipes showing how to solve real problems with Frame. Each recipe is a complete, runnable Frame spec with an explanation of the key patterns used.

For language syntax details, see the [Frame Language Reference](frame_language.md). For a tutorial introduction, see [Getting Started](frame_getting_started.md).

## Table of Contents

**Fundamentals (1-8)**

1. [Traffic Light](#1-traffic-light) — basic states and transitions
2. [Toggle Switch](#2-toggle-switch) — two-state with return values
3. [Turnstile](#3-turnstile) — event-driven guard logic
4. [Login Flow](#4-login-flow) — multi-step form wizard
5. [Connection Manager](#5-connection-manager) — lifecycle with enter/exit handlers
6. [Retry with Backoff](#6-retry-with-backoff) — state variables as counters
7. [Modal Dialog Stack](#7-modal-dialog-stack) — push/pop for navigation history
8. [State Stack](#8-state-stack-pushpop) — state stack for history

**Patterns (9-17)**

9. [Video Player](#9-video-player) — HSM with sub-states
10. [Order Processor](#10-order-processor) — business process with branches
11. [Approval Chain](#11-approval-chain) — multi-stage with forwarding
12. [Character Controller](#12-character-controller) — game state machine
13. [AI Agent](#13-ai-agent) — behavioral states with action logging
14. [LED Blink Controller](#14-led-blink-controller) — timer-driven state cycling
15. [Switch Debouncer](#15-switch-debouncer) — noise filtering
16. [Mealy Machine](#16-mealy-machine) — output depends on state + input
17. [Moore Machine](#17-moore-machine) — output depends on state only

**Advanced Features (18-22)**

18. [Session Persistence](#18-session-persistence) — save/restore with @@persist
19. [Async HTTP Client](#19-async-http-client) — async interface with two-phase init
20. [Multi-System Composition](#20-multi-system-composition) — two systems interacting
21. [Configurable Worker Pool](#21-configurable-worker-pool-parameterized-systems) — parameterized systems
22. [Self-Calibrating Sensor](#22-self-calibrating-sensor-self-interface-call) — `@@:self` reentrant dispatch

**Operations and Coverage (23-27)**

23. [Vending Machine](#23-vending-machine--operations-and-system-params) — operations and system params
24. [Circuit Breaker](#24-circuit-breaker--state-variable-reset-on-reentry) — state variable reset on reentry
25. [Rate Limiter](#25-rate-limiter--static-operations) — static operations
26. [Thermostat](#26-thermostat--3-level-hsm) — 3-level HSM
27. [Deployment Pipeline](#27-deployment-pipeline--push-and-enter-args) — push$ and enter args + decorated pop

**System-Managed States (28-29)**

28. [Auth Flow](#28-auth-flow--managed-loginsession) — managed login/session
29. [Game Level Manager](#29-game-level-manager--polymorphic-delegation) — polymorphic delegation

**Advanced Patterns (30-33)**

30. [Graceful Shutdown Service](#30-graceful-shutdown-service--hsm--enter-handler-chain) — HSM + enter-handler chain
31. [Pipeline Processor](#31-pipeline-processor--kernel-loop-validation) — kernel loop validation
32. [Test Harness](#32-test-harness--white-box-testing-with-operations) — white-box testing with operations
33. [AI Coding Agent](#33-ai-coding-agent--capstone) — capstone

**Enterprise Integration Patterns (34-45)**

34. [Idempotent Receiver](#34-idempotent-receiver) — dedupe by message ID
35. [Content-Based Router](#35-content-based-router) — route by message content
36. [Message Filter](#36-message-filter) — accept or drop by predicate
37. [Aggregator](#37-aggregator) — collect a correlation set, emit one message
38. [Resequencer](#38-resequencer) — buffer out-of-order messages, release in order
39. [Circuit Breaker](#39-circuit-breaker) — closed/open/half-open fault isolation
40. [Dead Letter Channel](#40-dead-letter-channel) — bounded retry with persistence
41. [Polling Consumer](#41-polling-consumer) — pull-driven message loop
42. [Process Manager (Saga)](#42-process-manager-saga) — multi-step orchestration with compensation
43. [Competing Consumers](#43-competing-consumers) — dispatcher + worker pool
44. [Message Store](#44-message-store) — audit log persisted across restarts
45. [Migrating Machine](#45-migrating-machine) — state machine as the message

**Protocol & Systems Stress Tests (46-49)**

46. [FIX Buy-Side Order](#46-fix-protocol--buy-side-order-lifecycle) — FIX OrdStatus state machine
47. [FIX Sell-Side Manager](#47-fix-protocol--sell-side-order-manager) — broker-side with buy-side interaction
48. [Launch Sequence Controller](#48-launch-sequence-controller--abort-from-any-phase) — 3-system flight computer with abort
49. [Robot Arm Controller](#49-robot-arm-controller--safety-overlay-with-hsm) — 3-level HSM safety overlay

**Deferred Event Processing (50-52)**

50. [Print Spooler](#50-print-spooler--basic-work-queue) — basic work queue with FIFO dequeue
51. [Manufacturing Cell](#51-manufacturing-cell--priority-queue-with-sub-phases) — priority queue with HSM sub-phases
52. [Elevator](#52-elevator--directional-scan-algorithm) — SCAN algorithm with request accumulation

-----

## 1. Traffic Light

![1 state diagram](images/cookbook/01.svg)

**Problem:** Cycle through a fixed sequence of states on each event.

```frame
@@target python_3

@@system TrafficLight {
    interface:
        next(): str

    machine:
        $Green {
            next(): str {
                @@:("green")
                -> $Yellow
            }
        }
        $Yellow {
            next(): str {
                @@:("yellow")
                -> $Red
            }
        }
        $Red {
            next(): str {
                @@:("red")
                -> $Green
            }
        }
}

if __name__ == '__main__':
    light = @@TrafficLight()
    for _ in range(6):
        print(light.next())
```

**How it works:** Three states form a cycle. Each `next()` call sets the return value via `@@:(expr)` and transitions to the next state. The return value is delivered to the caller after the transition completes.

**Features used:** transitions (`->`), return values

-----

## 2. Toggle Switch

![2 state diagram](images/cookbook/02.svg)

**Problem:** A switch that alternates between on and off.

```frame
@@target python_3

@@system Switch {
    interface:
        toggle(): str
        status(): str

    machine:
        $Off {
            toggle(): str {
                @@:("on")
                -> $On
            }
            status(): str { @@:("off") }
        }
        $On {
            toggle(): str {
                @@:("off")
                -> $Off
            }
            status(): str { @@:("on") }
        }
}

if __name__ == '__main__':
    sw = @@Switch()
    print(sw.status())   # off
    print(sw.toggle())   # on
    print(sw.toggle())   # off
```

**How it works:** Two states, each handling the same events differently. The same `toggle()` call produces different behavior depending on which state the system is in — the core value of state machines.

**Features used:** transitions, return values, multiple states handling the same event

-----

## 3. Turnstile

![3 state diagram](images/cookbook/03.svg)

**Problem:** A coin-operated turnstile that locks after each passage.

```frame
@@target python_3

@@system Turnstile {
    interface:
        coin()
        push(): str

    machine:
        $Locked {
            coin() { -> $Unlocked }
            push(): str { @@:("locked - insert coin") }
        }
        $Unlocked {
            coin() { }
            push(): str {
                @@:("welcome")
                -> $Locked
            }
        }
}

if __name__ == '__main__':
    t = @@Turnstile()
    print(t.push())   # locked - insert coin
    t.coin()
    print(t.push())   # welcome
    print(t.push())   # locked - insert coin
```

**How it works:** `coin()` in `$Locked` transitions to `$Unlocked`. `push()` in `$Unlocked` lets you through and re-locks. `coin()` in `$Unlocked` is a no-op (empty handler). `push()` in `$Locked` doesn't transition — just returns a message.

**Features used:** events with no effect (empty handler), guard-by-state

-----

## 4. Login Flow

![4 state diagram](images/cookbook/04.svg)

**Problem:** A multi-step login: enter username, enter password, authenticate.

```frame
@@target python_3

@@system LoginFlow {
    interface:
        submit(value: str): str

    machine:
        $EnterUsername {
            submit(value: str): str {
                self.username = value
                @@:("enter password")
                -> $EnterPassword
            }
        }
        $EnterPassword {
            submit(value: str): str {
                if self.authenticate(self.username, value):
                    @@:("welcome")
                    -> "authenticated" $LoggedIn
                else:
                    @@:("invalid - try again")
                    -> "bad credentials" $EnterUsername
            }
        }
        $LoggedIn {
            submit(value: str): str { @@:("already logged in") }
        }

    actions:
        authenticate(user, password) {
            return user == "admin" and password == "secret"
        }

    domain:
        username: str = ""
}

if __name__ == '__main__':
    login = @@LoginFlow()
    print(login.submit("admin"))    # enter password
    print(login.submit("wrong"))    # invalid - try again
    print(login.submit("admin"))    # enter password
    print(login.submit("secret"))   # welcome
```

**How it works:** Each state represents a step in the flow. `submit()` means different things in each state. Domain variable `username` persists across states. The action `authenticate` keeps validation logic out of the handler.

**Features used:** domain variables, actions, conditional transitions

-----

## 5. Connection Manager

![5 state diagram](images/cookbook/05.svg)

**Problem:** A network connection with proper setup/teardown lifecycle.

```frame
@@target python_3

@@system Connection {
    interface:
        connect(host: str)
        send(data: str): str
        disconnect()

    machine:
        $Disconnected {
            connect(host: str) {
                self.host = host
                -> $Connecting
            }
        }
        $Connecting {
            $>() {
                print(f"Connecting to {self.host}...")
                -> "connected" $Connected
            }
        }
        $Connected {
            $>() { print(f"Connected to {self.host}") }
            <$() { print(f"Disconnecting from {self.host}") }

            send(data: str): str {
                @@:(f"sent '{data}' to {self.host}")
            }
            disconnect() { -> $Disconnected }
        }

    domain:
        host: str = ""
}

if __name__ == '__main__':
    c = @@Connection()
    c.connect("example.com")   # Connecting... Connected
    print(c.send("hello"))     # sent 'hello' to example.com
    c.disconnect()             # Disconnecting...
```

**How it works:** `$>()` (enter) and `<$()` (exit) handlers run automatically during transitions. `$Connecting` transitions immediately in its enter handler — a common "transient state" pattern for setup work. The exit handler on `$Connected` ensures cleanup always happens.

**Features used:** enter/exit handlers, transient states, domain variables

-----

## 6. Retry with Backoff

![6 state diagram](images/cookbook/06.svg)

**Problem:** Retry an operation up to N times before failing.

```frame
@@target python_3

@@system Retrier {
    interface:
        start()
        status(): str

    machine:
        $Idle {
            start() {
                self.attempts = 0
                -> $Trying
            }
        }
        $Trying {
            $>() {
                self.attempts = self.attempts + 1
                if self.try_operation():
                    -> "success" $Succeeded
                else:
                    if self.attempts >= self.max_retries:
                        -> "exhausted" $Failed
                    else:
                        -> "retry" $Trying
            }
            status(): str { @@:("trying") }
        }
        $Succeeded {
            status(): str { @@:("succeeded") }
        }
        $Failed {
            status(): str { @@:("failed after max retries") }
        }

    actions:
        try_operation() {
            return False
        }

    domain:
        attempts: int = 0
        max_retries: int = 3
}

if __name__ == '__main__':
    r = @@Retrier()
    r.start()
    print(r.status())
```

**How it works:** The retry counter uses a **domain variable** (`self.attempts`), not a state variable, because state variables reset on every state entry. The enter handler increments the counter and either transitions to success, re-enters `$Trying` for another attempt, or gives up after `max_retries`. Each `-> $Trying` triggers a fresh enter handler call — the domain variable persists across re-entries.

**Features used:** domain variables for cross-state persistence, enter handler logic, self-transition for retry

-----

## 7. Modal Dialog Stack

![7 state diagram](images/cookbook/07.svg)

**Problem:** Open nested modal dialogs and return to the previous one on close.

```frame
@@target python_3

@@system DialogManager {
    interface:
        open(name: str)
        close(): str
        current(): str

    machine:
        $Main {
            open(name: str) {
                push$
                -> (name) $Dialog
            }
            current(): str { @@:("main") }
        }
        $Dialog {
            $.name: str = ""

            $>(name: str) { $.name = name }

            open(name: str) {
                push$
                -> (name) $Dialog
            }
            close(): str {
                @@:($.name)
                -> pop$
            }
            current(): str { @@:($.name) }
        }
}

if __name__ == '__main__':
    dm = @@DialogManager()
    print(dm.current())    # main
    dm.open("Settings")
    print(dm.current())    # Settings
    dm.open("Confirm")
    print(dm.current())    # Confirm
    print(dm.close())      # Confirm (closed)
    print(dm.current())    # Settings (restored)
    print(dm.close())      # Settings (closed)
    print(dm.current())    # main (restored)
```

**How it works:** `push$` saves the entire compartment (including state variables) onto the state stack before transitioning. `-> pop$` restores the previously saved compartment. Each dialog instance has its own `$.name` because state variables are per-compartment.

**Features used:** `push$`, `-> pop$`, enter args (`-> (name) $Dialog`), state variables

-----

## 8. State Stack (Push/Pop)

![8 state diagram](images/cookbook/08.svg)

**Problem:** Track state history and allow stepping backward.

```frame
@@target python_3

@@system Editor {
    interface:
        type_char(c: str)
        undo()
        get_buffer(): str

    machine:
        $Editing {
            $.buffer: str = ""

            type_char(c: str) {
                push$
                $.buffer = $.buffer + c
            }
            undo() {
                -> pop$
            }
            get_buffer(): str { @@:($.buffer) }
        }
}

if __name__ == '__main__':
    e = @@Editor()
    e.type_char("H")
    e.type_char("i")
    print(e.get_buffer())   # Hi
    e.undo()
    print(e.get_buffer())   # Hi (reference semantics — same compartment)
```

**How it works:** `push$` saves a **reference** to the current compartment, not a copy. After `push$`, both the stack entry and the current compartment point to the same object. Modifying `$.buffer` after push$ changes the shared compartment, so `-> pop$` restores the same object — the buffer retains its modified value.

For true snapshot undo, use `push$` with a transition (`push$ -> $Editing`) to create a new compartment. The old compartment on the stack preserves its pre-transition state.

**Features used:** `push$`, `-> pop$`, reference semantics

-----

## 9. Video Player

![9 state diagram](images/cookbook/09.svg)

**Problem:** A media player with play/pause/stop, where playing and paused are sub-states of "active."

```frame
@@target python_3

@@system VideoPlayer {
    interface:
        play()
        pause()
        stop()
        status(): str

    machine:
        $Stopped {
            play() { -> $Playing }
            status(): str { @@:("stopped") }
        }
        $Playing => $Active {
            pause() { -> $Paused }
            status(): str { @@:("playing") }
            => $^
        }
        $Paused => $Active {
            play() { -> $Playing }
            status(): str { @@:("paused") }
            => $^
        }
        $Active {
            stop() { -> $Stopped }
        }
}

if __name__ == '__main__':
    vp = @@VideoPlayer()
    print(vp.status())   # stopped
    vp.play()
    print(vp.status())   # playing
    vp.pause()
    print(vp.status())   # paused
    vp.stop()             # handled by $Active (parent)
    print(vp.status())   # stopped
```

**How it works:** `$Playing` and `$Paused` are children of `$Active` (declared with `=>`). The `stop()` event is only handled by `$Active` — children forward it via `=> $^` (default forward). This avoids duplicating the `stop()` handler in both child states.

**Features used:** HSM (`=> $Parent`), default forward (`=> $^`), event delegation

-----

## 10. Order Processor

![10 state diagram](images/cookbook/10.svg)

**Problem:** Process an order through validation, processing, and completion — with cancellation support.

```frame
@@target python_3

@@system OrderProcessor {
    interface:
        submit(item: str)
        cancel(reason: str)
        status(): str

    machine:
        $Idle {
            submit(item: str) {
                self.item = item
                -> $Validating
            }
            status(): str { @@:("idle") }
        }
        $Validating {
            $>() {
                if self.validate(self.item):
                    -> "valid" $Processing
                else:
                    -> "invalid" $Rejected
            }
            status(): str { @@:("validating") }
        }
        $Processing {
            cancel(reason: str) {
                print(f"Cancelled: {reason}")
                -> $Idle
            }
            status(): str { @@:("processing") }
        }
        $Rejected {
            status(): str { @@:("rejected") }
        }

    actions:
        validate(item) {
            return item is not None and len(item) > 0
        }

    domain:
        item: str = ""
}

if __name__ == '__main__':
    op = @@OrderProcessor()
    op.submit("widget")
    print(op.status())     # processing
    op.cancel("changed mind")
    print(op.status())     # idle
```

**How it works:** `$Validating` is a transient state — its enter handler immediately transitions based on validation. `cancel()` is only handled in `$Processing` — calling it in other states is a no-op (ignored). This is a key benefit of state machines: events are naturally ignored when they don't apply.

**Features used:** transient states, actions, events ignored in wrong state

-----

## 11. Approval Chain

![11 state diagram](images/cookbook/11.svg)

**Problem:** A document requires approval from two reviewers before it's published.

```frame
@@target python_3

@@system ApprovalChain {
    interface:
        submit()
        approve(reviewer: str)
        reject(reviewer: str)
        status(): str

    machine:
        $Draft {
            submit() { -> $Review1 }
            status(): str { @@:("draft") }
        }
        $Review1 {
            approve(reviewer: str) {
                print(f"Approved by {reviewer}")
                -> $Review2
            }
            reject(reviewer: str) {
                print(f"Rejected by {reviewer}")
                -> $Draft
            }
            status(): str { @@:("awaiting first review") }
        }
        $Review2 {
            approve(reviewer: str) {
                print(f"Approved by {reviewer}")
                -> $Published
            }
            reject(reviewer: str) {
                print(f"Rejected by {reviewer}")
                -> $Draft
            }
            status(): str { @@:("awaiting second review") }
        }
        $Published {
            status(): str { @@:("published") }
        }
}

if __name__ == '__main__':
    doc = @@ApprovalChain()
    doc.submit()
    print(doc.status())            # awaiting first review
    doc.approve("Alice")
    print(doc.status())            # awaiting second review
    doc.reject("Bob")
    print(doc.status())            # draft (back to start)
```

**How it works:** Each review stage is a separate state. Rejection at any stage returns to `$Draft`. The same `approve`/`reject` interface serves different stages — the state determines what happens. Events like `submit()` are silently ignored in states that don't handle them.

**Features used:** multi-stage workflow, rejection loops, silent event ignoring

-----

## 12. Character Controller

![12 state diagram](images/cookbook/12.svg)

**Problem:** A game character with idle, walking, running, and jumping states.

```frame
@@target python_3

@@system Character {
    interface:
        move()
        sprint()
        jump()
        land()
        stop()
        state(): str

    machine:
        $Idle {
            move() { -> $Walking }
            jump() { -> $Jumping }
            state(): str { @@:("idle") }
        }
        $Walking {
            sprint() { -> $Running }
            jump() { -> $Jumping }
            stop() { -> $Idle }
            state(): str { @@:("walking") }
        }
        $Running {
            stop() { -> $Walking }
            jump() { -> $Jumping }
            state(): str { @@:("running") }
        }
        $Jumping {
            land() { -> $Idle }
            state(): str { @@:("jumping") }
        }
}

if __name__ == '__main__':
    c = @@Character()
    print(c.state())    # idle
    c.move()
    print(c.state())    # walking
    c.sprint()
    print(c.state())    # running
    c.jump()
    print(c.state())    # jumping
    c.move()            # ignored while jumping
    print(c.state())    # jumping
    c.land()
    print(c.state())    # idle
```

**How it works:** The state determines which inputs are accepted. `move()` while jumping is silently ignored — no special code needed. `sprint()` only works from `$Walking`. This is much cleaner than `if (state == "jumping") return;` scattered through imperative code.

**Features used:** state-based input filtering, multiple states with overlapping events

-----

## 13. AI Agent

![13 state diagram](images/cookbook/13.svg)

**Problem:** An AI agent that explores, flees from threats, and tracks its actions.

```frame
@@target python_3

@@system Agent {
    interface:
        tick()
        threat()
        safe()
        get_log(): str

    machine:
        $Exploring {
            tick() {
                self.action_log = self.action_log + "explore,"
            }
            threat() {
                self.action_log = self.action_log + "flee,"
                -> $Fleeing
            }
            get_log(): str { @@:(self.action_log) }
        }
        $Fleeing {
            tick() {
                self.action_log = self.action_log + "run,"
            }
            safe() {
                self.action_log = self.action_log + "resume,"
                -> $Exploring
            }
            get_log(): str { @@:(self.action_log) }
        }

    domain:
        action_log: str = ""
}

if __name__ == '__main__':
    a = @@Agent()
    a.tick()
    a.tick()
    a.threat()
    a.tick()
    a.safe()
    print(a.get_log())  # explore,explore,flee,run,resume,
```

**How it works:** Domain variable `action_log` persists across all states. Each state appends its action on `tick()`. Threat/safe events trigger state transitions. Both states handle `get_log()` to return the accumulated log.

**Features used:** domain variables as accumulators, event-driven state transitions

-----

## 14. LED Blink Controller

![14 state diagram](images/cookbook/14.svg)

**Problem:** An LED that blinks on a timer, with on/off control.

```frame
@@target python_3

@@system LedBlinker {
    interface:
        enable()
        disable()
        timer_tick()
        is_lit(): bool

    machine:
        $Disabled {
            enable() { -> $LedOff }
        }
        $LedOff {
            timer_tick() { -> $LedOn }
            disable() { -> $Disabled }
            is_lit(): bool { @@:(False) }
        }
        $LedOn {
            timer_tick() { -> $LedOff }
            disable() { -> $Disabled }
            is_lit(): bool { @@:(True) }
        }
}

if __name__ == '__main__':
    led = @@LedBlinker()
    led.enable()
    for i in range(5):
        print(f"tick {i}: {'ON' if led.is_lit() else 'off'}")
        led.timer_tick()
```

**How it works:** External timer calls `timer_tick()`, which toggles between `$LedOn` and `$LedOff`. `disable()` works from either on or off state, returning to `$Disabled` where timer ticks are ignored.

**Features used:** timer-driven transitions, shared events across states

-----

## 15. Switch Debouncer

![15 state diagram](images/cookbook/15.svg)

**Problem:** Filter noisy switch input — only register a press after the signal stabilizes.

```frame
@@target python_3

@@system Debouncer {
    interface:
        raw_high()
        raw_low()
        tick()
        is_pressed(): bool

    machine:
        $Released {
            $.stable_count: int = 0

            raw_high() { $.stable_count = $.stable_count + 1 }
            raw_low() { $.stable_count = 0 }
            tick() {
                if $.stable_count >= 3:
                    -> "stable high" $Pressed
            }
            is_pressed(): bool { @@:(False) }
        }
        $Pressed {
            $.stable_count: int = 0

            raw_low() { $.stable_count = $.stable_count + 1 }
            raw_high() { $.stable_count = 0 }
            tick() {
                if $.stable_count >= 3:
                    -> "stable low" $Released
            }
            is_pressed(): bool { @@:(True) }
        }
}

if __name__ == '__main__':
    d = @@Debouncer()
    # Noisy signal: high, low, high, high, high (stabilizes after 3)
    for signal in [1, 0, 1, 1, 1]:
        if signal:
            d.raw_high()
        else:
            d.raw_low()
        d.tick()
    print(d.is_pressed())   # True
```

**How it works:** State variables `$.stable_count` track consecutive consistent readings. A bouncy signal resets the counter. Only after 3 consecutive stable readings does the state transition. State variables reset on entry, so both directions start clean.

**Features used:** state variables as counters, threshold-based transitions

-----

## 16. Mealy Machine

![16 state diagram](images/cookbook/16.svg)

**Problem:** Output depends on both the current state AND the input (classic Mealy machine).

```frame
@@target python_3

@@system MealyDetector {
    interface:
        input(bit: int): str

    machine:
        $S0 {
            input(bit: int): str {
                if bit == 1:
                    @@:("0")
                    -> "bit=1" $S1
                else:
                    @@:("0")
            }
        }
        $S1 {
            input(bit: int): str {
                if bit == 0:
                    @@:("1")
                    -> "bit=0" $S0
                else:
                    @@:("0")
            }
        }
}

if __name__ == '__main__':
    m = @@MealyDetector()
    for bit in [1, 0, 1, 1, 0]:
        print(f"in={bit} out={m.input(bit)}")
```

**How it works:** The output ("0" or "1") depends on BOTH the current state and the input bit. In `$S1`, receiving `0` outputs "1" (detected the pattern "10"). This is a sequence detector — it finds "10" patterns in a bitstream.

**Features used:** conditional transitions with different return values per branch

-----

## 17. Moore Machine

![17 state diagram](images/cookbook/17.svg)

**Problem:** Output depends only on the current state (classic Moore machine).

```frame
@@target python_3

@@system MooreParity {
    interface:
        input(bit: int)
        output(): str

    machine:
        $Even {
            input(bit: int) {
                if bit == 1:
                    -> "bit=1" $Odd
            }
            output(): str { @@:("even") }
        }
        $Odd {
            input(bit: int) {
                if bit == 1:
                    -> "bit=1" $Even
            }
            output(): str { @@:("odd") }
        }
}

if __name__ == '__main__':
    m = @@MooreParity()
    for bit in [1, 0, 1, 1, 0]:
        m.input(bit)
        print(f"in={bit} parity={m.output()}")
```

**How it works:** `output()` returns the same value regardless of input — it only depends on which state the system is in. This is a parity checker: it tracks whether an even or odd number of 1s have been seen.

**Features used:** state-determined output, input processing separated from output

-----

## 18. Session Persistence

![18 state diagram](images/cookbook/18.svg)

**Problem:** Save a user session to disk and restore it later.

```frame
@@target python_3

@@persist
@@system Session {
    interface:
        login(user: str)
        logout()
        who(): str

    machine:
        $LoggedOut {
            login(user: str) {
                self.user = user
                -> $LoggedIn
            }
            who(): str { @@:("nobody") }
        }
        $LoggedIn {
            logout() {
                self.user = ""
                -> $LoggedOut
            }
            who(): str { @@:(self.user) }
        }

    domain:
        user: str = ""
}

if __name__ == '__main__':
    s = @@Session()
    s.login("alice")
    print(s.who())               # alice

    # Save
    data = s.save_state()

    # Restore into a new instance
    s2 = Session.restore_state(data)
    print(s2.who())              # alice (state preserved)
```

**How it works:** `@@persist` generates `save_state()` and `restore_state()`. The saved data includes the current state (`$LoggedIn`), domain variables (`user = "alice"`), and the state stack. Restore does NOT fire the enter handler — it reconstructs the exact state.

**Features used:** `@@persist`, save/restore, domain variables

-----

## 19. Async HTTP Client

![19 state diagram](images/cookbook/19.svg)

**Problem:** An HTTP client with async connect/fetch/disconnect.

```frame
@@target python_3

import aiohttp
import asyncio

@@system HttpClient {
    interface:
        async connect(url: str)
        async fetch(path: str): str
        async disconnect()

    machine:
        $Idle {
            $>() {
                print("Ready")
            }
            connect(url: str) {
                self.base_url = url
                -> $Connected
            }
        }
        $Connected {
            $>() { print(f"Connected to {self.base_url}") }
            <$() { print("Closing connection") }

            fetch(path: str): str {
                async with aiohttp.ClientSession() as session:
                    async with session.get(self.base_url + path) as resp:
                        return await resp.text()
            }
            disconnect() { -> $Idle }
        }

    domain:
        base_url: str = ""
}

async def main():
    client = @@HttpClient()
    await client.init()          # async two-phase init
    await client.connect("https://example.com")
    html = await client.fetch("/")
    print(f"Got {len(html)} bytes")
    await client.disconnect()

asyncio.run(main())
```

**How it works:** `async` on interface methods makes the entire dispatch chain async. The constructor is synchronous — `await client.init()` fires the enter event separately (two-phase init). Native `await` in handler bodies works because the generated methods are async.

**Features used:** `async` interface methods, two-phase init, native async code in handlers

-----

## 20. Multi-System Composition

![20 state diagram](images/cookbook/20.svg)

**Problem:** A logger and an app as separate systems, with the app using the logger.

```frame
@@target python_3

@@system Logger {
    interface:
        log(msg: str)

    machine:
        $Active {
            log(msg: str) {
                print(f"[LOG] {msg}")
            }
        }
}

@@system App {
    interface:
        start()
        stop()

    machine:
        $Idle {
            start() {
                self.logger.log("App starting")
                -> $Running
            }
        }
        $Running {
            $>() { self.logger.log("App running") }
            stop() {
                self.logger.log("App stopping")
                -> $Idle
            }
        }

    domain:
        logger = @@Logger()
}

if __name__ == '__main__':
    app = @@App()
    app.start()
    app.stop()
```

**How it works:** Two `@@system` blocks in one file generate two independent classes. `@@Logger()` in the domain section instantiates the logger as a domain variable. Systems interact through their public interfaces — they don't share state.

**Features used:** multi-system files, `@@SystemName()` instantiation, domain variable initialization

> **Note:** Java and Erlang require one system per file. For these targets, split Logger and App into separate source files and import/require the dependency.

-----

## 21. Configurable Worker Pool (Parameterized Systems)

![21 state diagram](images/cookbook/21.svg)

A task executor whose pool size and retry policy are set at construction time. Domain, state, and enter parameters flow through the constructor to initialize the machine.

```frame
@@target python_3

@@system WorkerPool($(max_retries: int), $>(start_msg: str), pool_size: int) {
    interface:
        submit(task: str)
        get_status(): str

    machine:
        $Idle(max_retries: int) {
            $>(start_msg: str) {
                print(f"Pool ready: {start_msg}")
            }

            submit(task: str) {
                self.pending.append(task)
                if len(self.pending) >= self.pool_size:
                    -> "batch full" $Processing
            }

            get_status(): str {
                @@:(f"idle ({len(self.pending)}/{self.pool_size} pending)")
            }
        }

        $Processing {
            $>() {
                print(f"Processing batch of {len(self.pending)} tasks")
                self.pending.clear()
            }

            submit(task: str) {
                self.pending.append(task)
            }

            get_status(): str {
                @@:("processing")
            }
        }

    domain:
        pool_size: int = pool_size
        pending: list = []
}

if __name__ == '__main__':
    pool = @@WorkerPool($(5), $>("v1.0"), 3)
    pool.submit("task_a")
    print(pool.get_status())    # "idle (1/3 pending)"
    pool.submit("task_b")
    pool.submit("task_c")
    print(pool.get_status())    # "processing"
```

**Features used:** system parameters (state, enter, domain), sigil-tagged call-site syntax, `@@:(expr)` context return, state transitions triggered by threshold

-----

## 22. Self-Calibrating Sensor (@@:self Interface Call)

![22 state diagram](images/cookbook/22.svg)

**Problem:** A sensor that calibrates itself by reading its own value through the interface, then applying an offset.

```frame
@@target python_3

@@system Sensor {
    interface:
        calibrate(): str
        reading(): float
        trigger_shutdown()
        attempt_post_shutdown(): str
        status(): str

    machine:
        $Active {
            calibrate(): str {
                baseline = @@:self.reading()
                self.offset = baseline * -1
                @@:(f"calibrated: offset={self.offset}")
            }

            reading(): float {
                @@:(self.raw_value + self.offset)
            }

            trigger_shutdown() {
                -> $Shutdown
            }

            attempt_post_shutdown(): str {
                self.trace = "before"
                @@:self.trigger_shutdown()
                self.trace = "after"
                @@:(self.trace)
            }

            status(): str { @@:(@@:system.state) }
        }

        $Shutdown {
            status(): str { @@:(@@:system.state) }
        }

    domain:
        raw_value: float = 42.0
        offset: float = 0.0
        trace: str = ""
}

if __name__ == '__main__':
    s = @@Sensor()
    print(s.reading())       # 42.0
    print(s.calibrate())     # calibrated: offset=-42.0
    print(s.reading())       # 0.0

    s2 = @@Sensor()
    s2.attempt_post_shutdown()
    print(s2.trace)          # "before" — "after" was suppressed
    print(s2.status())       # "Shutdown" (via @@:system.state)
```

**How it works:** `@@:self.reading()` dispatches through the full kernel pipeline. The return value is available as a native expression.

**Transition guard:** In `attempt_post_shutdown()`, calling `@@:self.trigger_shutdown()` transitions to `$Shutdown`. When control returns, the guard detects the transition and suppresses remaining code — `self.trace = "after"` never executes because the system is no longer in `$Active`.

**Features used:** `@@:self.method()`, reentrant dispatch, return value from self-call, transition guard, `@@:system.state`

-----

## 23. Vending Machine — Operations and System Params

![23 state diagram](images/cookbook/23.svg)

**Problem:** A vending machine with admin operations that bypass the state machine.

```frame
@@target python_3

@@system VendingMachine(inventory: dict = {}) {
    operations:
        stock(product: str, qty: int) {
            self.inventory[product] = self.inventory.get(product, 0) + qty
        }
        check_stock(product: str): int {
            return self.inventory.get(product, 0)
        }

    interface:
        insert_coin(amount: int)
        select(product: str): str = "error"
        cancel(): int = 0

    machine:
        $Idle {
            insert_coin(amount: int) {
                self.balance = amount
                -> $HasCredit
            }
            select(product: str): str { @@:("insert coin first") }
            cancel(): int { @@:(0) }
        }
        $HasCredit {
            insert_coin(amount: int) {
                self.balance = self.balance + amount
            }
            select(product: str): str {
                price = self.get_price(product)
                if price is None:
                    @@:("unknown product")
                elif self.balance < price:
                    @@:(f"need {price - self.balance} more")
                elif self.inventory.get(product, 0) <= 0:
                    @@:("sold out")
                else:
                    self.inventory[product] = self.inventory[product] - 1
                    self.balance = self.balance - price
                    @@:(f"dispensing {product}")
                    -> "paid" $Dispensing
            }
            cancel(): int {
                refund = self.balance
                self.balance = 0
                @@:(refund)
                -> "refund" $Idle
            }
        }
        $Dispensing {
            $>() {
                change = self.balance
                self.balance = 0
                if change > 0:
                    print(f"Change: {change}")
                -> "give change" $Idle
            }
        }

    actions:
        get_price(product) {
            prices = {"cola": 100, "chips": 75, "water": 50}
            return prices.get(product)
        }

    domain:
        balance: int = 0
        inventory: dict = {}
}

if __name__ == '__main__':
    vm = @@VendingMachine(inventory={"cola": 2, "chips": 1})
    vm.stock("water", 5)
    print(f"Water stock: {vm.check_stock('water')}")  # 5
    print(vm.select("cola"))       # insert coin first
    vm.insert_coin(100)
    print(vm.select("cola"))       # dispensing cola
```

**Features stressed:** operations (bypass state machine), system params (domain overrides), transient state (`$Dispensing`), actions with native return

-----

## 24. Circuit Breaker — State Variable Reset on Reentry

![24 state diagram](images/cookbook/24.svg)

**Problem:** A circuit breaker where the failure counter resets each time we re-enter the closed state.

```frame
@@target python_3

@@system CircuitBreaker {
    interface:
        call(): str = "error"
        success()
        failure()
        tick()
        status(): str = ""

    machine:
        $Closed {
            $.failures: int = 0

            call(): str { @@:("allowed") }
            success() { $.failures = 0 }
            failure() {
                $.failures = $.failures + 1
                if $.failures >= self.threshold:
                    -> "tripped" $Open
            }
            status(): str { @@:(f"closed ({$.failures} failures)") }
        }
        $Open {
            $.cooldown_remaining: int = 0

            $>() {
                $.cooldown_remaining = self.cooldown
                print(f"Circuit OPEN — cooling down for {self.cooldown} ticks")
            }
            call(): str { @@:("blocked") }
            tick() {
                $.cooldown_remaining = $.cooldown_remaining - 1
                if $.cooldown_remaining <= 0:
                    -> "cooled down" $HalfOpen
            }
            status(): str { @@:(f"open ({$.cooldown_remaining} ticks left)") }
        }
        $HalfOpen {
            call(): str { @@:("testing") }
            success() {
                print("Circuit recovered")
                -> "recovered" $Closed
            }
            failure() {
                print("Still failing")
                -> "relapse" $Open
            }
            status(): str { @@:("half-open") }
        }

    domain:
        threshold: int = 3
        cooldown: int = 5
}

if __name__ == '__main__':
    cb = @@CircuitBreaker()
    cb.failure(); cb.failure(); cb.failure()  # Circuit OPEN
    print(cb.call())       # blocked
    for _ in range(5): cb.tick()
    cb.success()            # recovered
    print(cb.status())     # closed (0 failures)
```

**Features stressed:** state variable reset on reentry, state vars vs domain vars contrast

-----

## 25. Rate Limiter — Static Operations

![25 state diagram](images/cookbook/25.svg)

**Problem:** A token bucket rate limiter with a static utility function.

```frame
@@target python_3

@@system RateLimiter {
    operations:
        static tokens_for_rate(rps: int, interval_ms: int): int {
            return max(1, (rps * interval_ms) // 1000)
        }
        tokens_remaining(): int {
            return self.tokens
        }

    interface:
        request(): str = "error"
        tick()

    machine:
        $Accepting {
            request(): str {
                self.tokens = self.tokens - 1
                @@:("accepted")
                if self.tokens <= 0:
                    -> "exhausted" $Throttled
            }
            tick() { self.tokens = min(self.tokens + 1, self.max_tokens) }
        }
        $Throttled {
            request(): str { @@:("throttled") }
            tick() {
                self.tokens = self.tokens + 1
                if self.tokens > 0:
                    -> "replenished" $Accepting
            }
        }

    domain:
        tokens: int = 10
        max_tokens: int = 10
}

if __name__ == '__main__':
    print(RateLimiter.tokens_for_rate(100, 50))  # static — no instance needed
    rl = @@RateLimiter()
    for i in range(12):
        print(f"{i+1}: {rl.request()} ({rl.tokens_remaining()} left)")
```

**Features stressed:** `static` keyword on operations, instance operations, `@@:(expr)` return

-----

## 26. Thermostat — 3-Level HSM

![26 state diagram](images/cookbook/26.svg)

**Problem:** A smart thermostat with three hierarchy levels.

```frame
@@target python_3

@@system Thermostat {
    interface:
        set_temp(target: int)
        tick()
        status(): str = "unknown"

    machine:
        $Off {
            set_temp(target: int) {
                self.target = target
                -> "start" $LowHeat
            }
            status(): str { @@:("off") }
        }
        $LowHeat => $Heating {
            $>() { print("Low heat on") }
            tick() {
                if self.target - self.current > 5:
                    -> "gap large" $HighHeat
            }
            status(): str { @@:(f"low heat ({self.current} to {self.target})") }
            => $^
        }
        $HighHeat => $Heating {
            $>() { print("High heat on") }
            tick() {
                if self.target - self.current <= 3:
                    -> "gap small" $LowHeat
            }
            status(): str { @@:(f"high heat ({self.current} to {self.target})") }
            => $^
        }
        $Heating => $On {
            tick() {
                self.current = self.current + 1
                => $^
            }
        }
        $On {
            tick() {
                if self.current >= self.target:
                    -> "at target" $Cooling
            }
            set_temp(target: int) { self.target = target }
        }
        $Cooling => $On {
            $>() { print("Cooling") }
            tick() {
                self.current = self.current - 1
                if self.current <= self.target:
                    -> "cooled down" $Off
            }
            status(): str { @@:(f"cooling ({self.current} to {self.target})") }
            => $^
        }

    domain:
        current: int = 65
        target: int = 65
}

if __name__ == '__main__':
    t = @@Thermostat()
    t.set_temp(75)
    for _ in range(12):
        t.tick()
        print(t.status())
```

**Features stressed:** 3-level HSM hierarchy, default forward vs in-handler forward, event inheritance through parent chain

-----

## 27. Deployment Pipeline — push$ and Enter Args

![27 state diagram](images/cookbook/27.svg)

**Problem:** A deployment pipeline with rollback via push$/pop$, using decorated pop to signal rollback reason.

```frame
@@target python_3

@@system Deployer {
    interface:
        deploy(version: str)
        verify(): str = ""
        rollback(): str = ""
        status(): str = ""

    machine:
        $Ready {
            $.version: str = "none"
            deploy(version: str) {
                push$
                -> (version) $Deploying
            }
            status(): str { @@:(f"ready (v{$.version})") }
        }
        $Deploying {
            $.version: str = ""
            $.step: int = 0
            $.steps: list = []

            $>(version: str) {
                $.version = version
                $.steps = ["provision", "configure", "migrate", "healthcheck"]
                print(f"Deploying v{version}...")
            }
            verify(): str {
                if $.step < len($.steps):
                    current = $.steps[$.step]
                    $.step = $.step + 1
                    @@:(f"ok {current}")
                else:
                    @@:("all passed")
                    -> ($.version) $Live
            }
            rollback(): str {
                @@:(f"rolling back v{$.version}")
                ("deployment_aborted") -> pop$
            }
            status(): str { @@:(f"deploying v{$.version}") }
        }
        $Live {
            $.version: str = ""
            $>(version: str) {
                $.version = version
                print(f"v{version} is live")
            }
            deploy(version: str) {
                push$
                -> (version) $Deploying
            }
            rollback(): str {
                @@:(f"rolling back from v{$.version}")
                ("version_reverted") -> pop$
            }
            status(): str { @@:(f"live (v{$.version})") }
        }
}

if __name__ == '__main__':
    d = @@Deployer()
    d.deploy("1.0")
    for _ in range(4): print(d.verify())
    print(d.status())      # live (v1.0)
    d.deploy("2.0")
    print(d.verify())      # ok provision
    print(d.rollback())    # rolling back v2.0
    print(d.status())      # live (v1.0)
```

**How it works:** `push$` saves the current compartment before deploying. `-> (version) $Deploying` passes the version as enter args. `("deployment_aborted") -> pop$` is a **decorated pop** — it writes exit args on the leaving compartment before popping. The restored state gets back its original state variables intact.

**Other decorated pop forms:**
- `-> (enter_args) pop$` — replace the popped compartment's enter args
- `-> => pop$` — forward the current event to the restored state
- `(exit) -> (enter) => pop$` — all three combined

**Features stressed:** `push$`, `-> pop$`, enter args, decorated pop with exit args, state variables with list type

-----

## 28. Auth Flow — Managed Login/Session

![28 state diagram](images/cookbook/28.svg)

**Problem:** `$LoggedOut` creates a LoginManager; `$LoggedIn` creates a SessionManager. Managers are state variables — their lifecycle matches the state.

```frame
@@target python_3

@@system LoginManager {
    interface:
        submit(field: str, value: str): str = ""
        cancel()

    machine:
        $EnterUsername {
            submit(field: str, value: str): str {
                if field == "username":
                    self.username = value
                    @@:("enter password")
                    -> "accepted" $EnterPassword
                @@:("enter username")
            }
            cancel() { self.parent.auth_cancelled() }
        }
        $EnterPassword {
            $.attempts: int = 0
            submit(field: str, value: str): str {
                if field == "password":
                    if self.username == "admin" and value == "secret":
                        self.parent.auth_succeeded(self.username)
                        @@:return("authenticated")
                    $.attempts = $.attempts + 1
                    if $.attempts >= 3:
                        self.parent.auth_locked(self.username)
                        @@:return("account locked")
                    @@:return(f"wrong ({3 - $.attempts} left)")
                @@:("enter password")
            }
            cancel() { -> $EnterUsername }
        }

    domain:
        parent = None
        username: str = ""
}

@@system SessionManager {
    interface:
        tick()
        activity()
        request_logout()

    machine:
        $Active {
            $.idle_ticks: int = 0
            tick() {
                $.idle_ticks = $.idle_ticks + 1
                if $.idle_ticks >= self.timeout_ticks:
                    self.parent.session_ended("timeout")
            }
            activity() { $.idle_ticks = 0 }
            request_logout() { self.parent.session_ended("logout") }
        }

    domain:
        parent = None
        timeout_ticks: int = 10
}

@@system App {
    interface:
        submit(field: str, value: str): str = ""
        cancel()
        tick()
        activity()
        logout()
        auth_succeeded(username: str)
        auth_cancelled()
        auth_locked(username: str)
        session_ended(reason: str)
        status(): str = ""

    machine:
        $LoggedOut {
            $.login_mgr = None
            $>() {
                $.login_mgr = @@LoginManager()
                $.login_mgr.parent = self
            }
            <$() { $.login_mgr = None }

            submit(field: str, value: str): str { @@:($.login_mgr.submit(field, value)) }
            cancel() { $.login_mgr.cancel() }
            auth_succeeded(username: str) {
                self.current_user = username
                -> "login ok" $LoggedIn
            }
            auth_cancelled() { print("[App] Cancelled") }
            auth_locked(username: str) { -> "locked out" $Locked }
            status(): str { @@:("logged out") }
        }
        $LoggedIn {
            $.session_mgr = None
            $>() {
                $.session_mgr = @@SessionManager()
                $.session_mgr.parent = self
            }
            <$() { $.session_mgr = None }

            tick() { $.session_mgr.tick() }
            activity() { $.session_mgr.activity() }
            logout() { $.session_mgr.request_logout() }
            session_ended(reason: str) {
                self.current_user = ""
                -> $LoggedOut
            }
            status(): str { @@:(f"logged in as {self.current_user}") }
        }
        $Locked {
            status(): str { @@:("locked") }
        }

    domain:
        current_user: str = ""
}

if __name__ == '__main__':
    app = @@App()
    print(app.submit("username", "admin"))   # enter password
    print(app.submit("password", "secret"))  # authenticated
    print(app.status())                       # logged in as admin
    app.logout()
    print(app.status())                       # logged out
```

**Features stressed:** state variables for manager refs, multi-system composition, `@@:return(expr)` exit sugar, manager-to-parent callback

-----

## 29. Game Level Manager — Polymorphic Delegation

![29 state diagram](images/cookbook/29.svg)

**Problem:** Different level types created per config. Re-entering `$InLevel` automatically swaps managers.

```frame
@@target python_3

import random

@@system SurvivalLevel {
    interface:
        tick()
        input(action: str): str = ""

    machine:
        $Playing {
            $.health: int = 100
            $.score: int = 0

            tick() {
                $.health = $.health - random.randint(0, 10)
                if $.health <= 0:
                    self.final_score = $.score
                    -> "died" $Lost
            }
            input(action: str): str {
                if action == "attack":
                    $.score = $.score + 10
                    @@:(f"attack! score={$.score}")
            }
        }
        $Lost {
            $>() {
                print("You died!")
                self.parent.level_complete(False, self.final_score)
            }
        }

    domain:
        parent = None
        final_score: int = 0
}

@@system Game {
    interface:
        start()
        tick()
        input(action: str): str = ""
        level_complete(won: bool, score: int)
        status(): str = ""

    machine:
        $MainMenu {
            start() {
                self.total_score = 0
                -> $InLevel
            }
            status(): str { @@:("main menu") }
        }
        $InLevel {
            $.level_mgr = None
            $>() {
                print(f"\n=== Level {self.level_index + 1} ===")
                $.level_mgr = @@SurvivalLevel()
                $.level_mgr.parent = self
            }
            <$() { $.level_mgr = None }

            tick() { $.level_mgr.tick() }
            input(action: str): str { @@:($.level_mgr.input(action)) }
            level_complete(won: bool, score: int) {
                self.total_score = self.total_score + score
                if won:
                    self.level_index = self.level_index + 1
                    -> "next level" $InLevel
                else:
                    -> "game over" $GameOver
            }
        }
        $GameOver {
            $>() { print(f"\nGAME OVER — Score: {self.total_score}") }
            status(): str { @@:(f"game over ({self.total_score})") }
        }

    domain:
        level_index: int = 0
        total_score: int = 0
}

if __name__ == '__main__':
    random.seed(42)
    game = @@Game()
    game.start()
    for _ in range(10):
        game.tick()
        print(game.input("attack"))
    print(game.status())
```

**Features stressed:** `$.level_mgr` state variable, re-entry (`-> $InLevel`) creates fresh manager, polymorphic delegation

-----

## 30. Graceful Shutdown Service — HSM + Enter-Handler Chain

![30 state diagram](images/cookbook/30.svg)

**Problem:** A long-running service where the constructor never returns. HSM provides shared quit logic.

```frame
@@target python_3

@@system Worker {
    interface:
        quit()

    machine:
        $Init {
            $>() {
                print("[Worker] Starting...")
                -> "begin" $FetchData
            }
        }
        $FetchData => $Running {
            $>() {
                if self.cycles >= self.max_cycles:
                    -> "done" $ShuttingDown
                print(f"[Worker] Fetch (cycle {self.cycles + 1})")
                self.cycles = self.cycles + 1
                -> "process" $ProcessData
            }
            => $^
        }
        $ProcessData => $Running {
            $>() {
                print(f"[Worker] Process (cycle {self.cycles})")
                -> "fetch next" $FetchData
            }
            => $^
        }
        $Running {
            quit() {
                print(f"[Worker] Quit after {self.cycles} cycles")
                -> $ShuttingDown
            }
        }
        $ShuttingDown {
            $>() {
                print("[Worker] Cleanup...")
                print("[Worker] Goodbye.")
            }
        }

    domain:
        cycles: int = 0
        max_cycles: int = 3
}

if __name__ == '__main__':
    w = @@Worker()
    print(f"Worker ran {w.cycles} cycles")
```

**How it works:** The constructor triggers `$Init.$>()` which chains to `$FetchData.$>()` -> `$ProcessData.$>()` -> `$FetchData.$>()` -> ... through the kernel loop. The call stack stays flat — each enter handler sets `__next_compartment` and returns; the kernel processes transitions iteratively. After `max_cycles`, `$FetchData` transitions to `$ShuttingDown` instead. HSM means `$FetchData` and `$ProcessData` both forward `quit()` to `$Running` via `=> $^` — add more stages without touching quit logic.

**Features stressed:** enter-handler chain (kernel loop), HSM for shared `quit()`, cycle-bounded service pattern

-----

## 31. Pipeline Processor — Kernel Loop Validation

![31 state diagram](images/cookbook/31.svg)

**Problem:** Data flows through 5 stages via enter-handler transitions in a single interface call.

```frame
@@target python_3

@@system Pipeline {
    interface:
        run(data: list)
        result(): list = []
        status(): str = ""

    machine:
        $Idle {
            run(data: list) {
                self.data = data
                self.log = []
                -> "start" $Validate
            }
        }
        $Validate {
            $>() {
                self.log.append("validate")
                self.data = [x for x in self.data if x is not None and x != ""]
                if len(self.data) == 0:
                    -> "empty" $Error
                else:
                    -> "valid" $Normalize
            }
        }
        $Normalize {
            $>() {
                self.log.append("normalize")
                self.data = [str(x).lower().strip() for x in self.data]
                -> "deduplicate" $Deduplicate
            }
        }
        $Deduplicate {
            $>() {
                self.log.append("deduplicate")
                seen = set()
                unique = []
                for item in self.data:
                    if item not in seen:
                        seen.add(item)
                        unique.append(item)
                self.data = unique
                -> "sort" $Sort
            }
        }
        $Sort {
            $>() {
                self.log.append("sort")
                self.data = sorted(self.data)
                -> "finish" $Complete
            }
        }
        $Complete {
            $>() { print(f"Pipeline: {len(self.data)} items") }
            result(): list { @@:(self.data) }
            status(): str { @@:(' -> '.join(self.log)) }
        }
        $Error {
            $>() { print("Pipeline error: empty input") }
            result(): list { @@:([]) }
            status(): str { @@:("error") }
        }

    domain:
        data: list = []
        log: list = []
}

if __name__ == '__main__':
    p = @@Pipeline()
    p.run(["Banana", " apple ", "CHERRY", "apple", None, "banana"])
    print(p.result())    # ['apple', 'banana', 'cherry']
    print(p.status())    # validate -> normalize -> deduplicate -> sort -> complete
```

**Features stressed:** 5-stage enter-handler chain, kernel loop validation, conditional early exit to `$Error`

-----

## 32. Test Harness — White-Box Testing with Operations

![32 state diagram](images/cookbook/32.svg)

**Problem:** A system with operations for test inspection. `@@:system.state` is a read-only accessor — allowed in operations because it doesn't mutate the state machine.

```frame
@@target python_3

@@system TrafficLight {
    operations:
        current_state(): str {
            return @@:system.state
        }
        is_in_state(name: str): bool {
            return @@:system.state == name
        }
        get_config(): dict {
            return {"green": self.green_dur, "yellow": self.yellow_dur, "red": self.red_dur}
        }

    interface:
        next()
        emergency()
        resume()

    machine:
        $Green {
            $.ticks: int = 0
            next() {
                $.ticks = $.ticks + 1
                if $.ticks >= self.green_dur: -> "expired" $Yellow
            }
            emergency() { -> $EmergencyRed }
        }
        $Yellow {
            $.ticks: int = 0
            next() {
                $.ticks = $.ticks + 1
                if $.ticks >= self.yellow_dur: -> "expired" $Red
            }
            emergency() { -> $EmergencyRed }
        }
        $Red {
            $.ticks: int = 0
            next() {
                $.ticks = $.ticks + 1
                if $.ticks >= self.red_dur: -> "expired" $Green
            }
            emergency() { -> $EmergencyRed }
        }
        $EmergencyRed {
            resume() { -> $Red }
        }

    domain:
        green_dur: int = 3
        yellow_dur: int = 1
        red_dur: int = 2
}

if __name__ == '__main__':
    tl = @@TrafficLight()
    assert tl.is_in_state("Green")

    for _ in range(3): tl.next()
    assert tl.is_in_state("Yellow")

    tl.next()
    assert tl.is_in_state("Red")

    tl.emergency()
    assert tl.is_in_state("EmergencyRed")
    tl.next()
    assert tl.is_in_state("EmergencyRed")
    tl.resume()
    assert tl.is_in_state("Red")

    config = tl.get_config()
    assert config["green"] == 3

    print("All tests passed!")
```

**Features stressed:** `@@:system.state` in operations, white-box testing pattern, events ignored in wrong state, state variable reset (`$.ticks`)

-----

## 33. AI Coding Agent — Capstone

![33 state diagram](images/cookbook/33.svg)

**Problem:** An AI coding agent with planning, approval, tool execution, testing, and retry.

```frame
@@target python_3

@@system ToolRunner {
    interface:
        execute(tool: str, params: dict)

    machine:
        $Ready {
            execute(tool: str, params: dict) {
                self.current_tool = tool
                self.current_params = params
                -> $Running
            }
        }
        $Running {
            $>() {
                print(f"  [tool] [{self.current_tool}] executing...")
                result = self.simulate(self.current_tool, self.current_params)
                self.parent.tool_completed(
                    self.current_tool, result["success"], result["data"]
                )
            }
        }

    actions:
        simulate(tool, params) {
            if tool == "read_file":
                return {"success": True, "data": f"read {params.get('path', '?')}"}
            elif tool == "write_file":
                return {"success": True, "data": f"wrote {params.get('path', '?')}"}
            elif tool == "run_terminal":
                import random
                if random.random() < 0.7:
                    return {"success": True, "data": "tests passed"}
                return {"success": False, "data": "2 tests failed"}
            return {"success": False, "data": f"unknown: {tool}"}
        }

    domain:
        parent = None
        current_tool: str = ""
        current_params: dict = {}
}

@@system Agent {
    interface:
        task(description: str)
        approve()
        reject(feedback: str)
        abort()
        tool_completed(tool: str, success: bool, data: str)
        plan_ready(steps: list)
        coding_done()
        tests_passed()
        tests_failed(failures: str)
        status(): str = ""

    machine:
        $Idle {
            task(description: str) {
                self.task_desc = description
                print(f"\n[task] Task: {description}")
                -> $Planning
            }
            status(): str { @@:("idle") }
        }
        $Planning => $Active {
            $>() {
                print("[plan] Planning...")
                steps = self.make_plan(self.task_desc)
                @@:self.plan_ready(steps)
            }
            plan_ready(steps: list) {
                self.plan = steps
                for i, s in enumerate(steps):
                    print(f"   {i+1}. {s}")
                -> "review" $AwaitingApproval
            }
            => $^
        }
        $AwaitingApproval => $Active {
            approve() {
                self.step = 0
                -> "begin" $Coding
            }
            reject(feedback: str) { -> "revise" $Planning }
            => $^
        }
        $Coding => $Active {
            $>() {
                if self.step >= len(self.plan):
                    @@:self.coding_done()
                    return
                print(f"\n[code] Step {self.step + 1}: {self.plan[self.step]}")
                tool = self.step_to_tool(self.plan[self.step])
                self.tool_runner = @@ToolRunner()
                self.tool_runner.parent = self
                self.tool_runner.execute(tool["tool"], tool["params"])
            }
            coding_done() { -> "run tests" $Testing }
            tool_completed(tool: str, success: bool, data: str) {
                if success:
                    print(f"  ok {data}")
                    self.step = self.step + 1
                    -> "next step" $Coding
                else:
                    print(f"  err {data}")
                    self.last_error = data
                    -> "error" $ErrorRecovery
            }
            => $^
        }
        $Testing => $Active {
            $>() {
                print("\n[test] Testing...")
                self.tool_runner = @@ToolRunner()
                self.tool_runner.parent = self
                self.tool_runner.execute("run_terminal", {"command": "pytest"})
            }
            tool_completed(tool: str, success: bool, data: str) {
                if success:
                    @@:self.tests_passed()
                else:
                    @@:self.tests_failed(data)
            }
            tests_passed() { -> "passed" $Complete }
            tests_failed(failures: str) {
                self.retries = self.retries + 1
                if self.retries >= 2:
                    -> "give up" $Failed
                else:
                    self.task_desc = f"Fix: {failures}"
                    -> "retry" $Planning
            }
            => $^
        }
        $ErrorRecovery => $Active {
            $>() { print(f"  [warn] {self.last_error}") }
            approve() { -> "retry" $Coding }
            reject(feedback: str) { -> "replan" $Planning }
            => $^
        }
        $Active {
            abort() {
                print("\n[abort] Aborted")
                -> $Idle
            }
        }
        $Complete {
            $>() { print("\n[done] Done!") }
            task(description: str) {
                self.reset()
                self.task_desc = description
                -> "new task" $Planning
            }
            status(): str { @@:("complete") }
        }
        $Failed {
            $>() { print("\n[fail] Failed") }
            task(description: str) {
                self.reset()
                self.task_desc = description
                -> "new task" $Planning
            }
            status(): str { @@:("failed") }
        }

    actions:
        make_plan(desc) {
            return ["Read files", f"Implement: {desc[:40]}", "Write files"]
        }
        step_to_tool(step_text) {
            lower = step_text.lower()
            if "read" in lower:
                return {"tool": "read_file", "params": {"path": "src/main.py"}}
            elif "write" in lower:
                return {"tool": "write_file", "params": {"path": "src/main.py"}}
            return {"tool": "run_terminal", "params": {"command": f"echo '{step_text}'"}}
        }
        reset() {
            self.plan = []
            self.step = 0
            self.retries = 0
            self.last_error = ""
        }

    domain:
        task_desc: str = ""
        plan: list = []
        step: int = 0
        retries: int = 0
        last_error: str = ""
        tool_runner = None
}

if __name__ == '__main__':
    import random
    random.seed(42)
    agent = @@Agent()
    agent.task("Add input validation")
    agent.approve()
    print(agent.status())
```

**Features stressed:** HSM with shared `abort()`, `@@:self.method()` transition guard, system-managed `ToolRunner`, state re-entry, enter-handler chains, retry loops

-----

## Enterprise Integration Patterns

Recipes 34-45 implement patterns from *Enterprise Integration Patterns* (Hohpe & Woolf, 2003) as Frame state machines. Each maps a named EIP pattern onto the same shape as the recipes above. These are single-node implementations of the pattern's *logic* — the part a message broker, integration framework, or service would host. The transport is your host code; the state machine is what you hand to the host.

All recipes target Python 3 for readability; the patterns generate identically for all 17 Frame targets. For the canonical pattern catalog, see [enterpriseintegrationpatterns.com](https://www.enterpriseintegrationpatterns.com/patterns/messaging/).

-----

## 34. Idempotent Receiver

![34 state diagram](images/cookbook/34.svg)

**Problem:** A sender may redeliver the same message. The receiver must process each business message exactly once, even if it arrives multiple times.

```frame
@@target python_3

@@system IdempotentReceiver {
    interface:
        deliver(msg_id: str, payload: str): str

    machine:
        $Ready {
            deliver(msg_id: str, payload: str): str {
                if msg_id in self.seen:
                    @@:("duplicate")
                else:
                    self.seen.add(msg_id)
                    self.process(payload)
                    @@:("accepted")
            }
        }

    actions:
        process(payload) {
            print(f"processing: {payload}")
        }

    domain:
        seen: set = set()
}

if __name__ == '__main__':
    r = @@IdempotentReceiver()
    print(r.deliver("m1", "hello"))    # accepted
    print(r.deliver("m1", "hello"))    # duplicate
    print(r.deliver("m2", "world"))    # accepted
```

**How it works:** The `seen` set in the domain is the idempotency key store. On redelivery, the handler branches on set membership before doing any work. A single `$Ready` state is enough because the dedupe decision is data-driven, not state-driven.

**Features used:** domain variables as a dedupe store, data-driven branching, actions

-----

## 35. Content-Based Router

![35 state diagram](images/cookbook/35.svg)

**Problem:** A single inbound stream contains messages of different kinds. Each kind should be routed to a different downstream destination.

```frame
@@target python_3

@@system ContentBasedRouter {
    interface:
        route(kind: str, payload: str): str

    machine:
        $Routing {
            route(kind: str, payload: str): str {
                if kind == "order":
                    self.to_orders(payload)
                    @@:("orders")
                else:
                    if kind == "refund":
                        self.to_refunds(payload)
                        @@:("refunds")
                    else:
                        self.to_dlq(payload)
                        @@:("dlq")
            }
        }

    actions:
        to_orders(p)  { print(f"-> orders:  {p}") }
        to_refunds(p) { print(f"-> refunds: {p}") }
        to_dlq(p)     { print(f"-> dlq:     {p}") }
}

if __name__ == '__main__':
    r = @@ContentBasedRouter()
    r.route("order",   "SKU-1")
    r.route("refund",  "INV-7")
    r.route("unknown", "???")
```

**How it works:** The routing decision is a pure function of the message, so one state suffices. The router's contract (`route()`) and its decision table sit next to each other in one block, generating a class your host code can instantiate and call. Routers that learn downstream health graduate to multi-state — see [Circuit Breaker](#39-circuit-breaker).

**Features used:** single-state dispatcher, actions as routing sinks, content-driven branching

-----

## 36. Message Filter

![36 state diagram](images/cookbook/36.svg)

**Problem:** Drop messages that don't match a predicate. Count both accepted and rejected messages for observability.

```frame
@@target python_3

@@system MessageFilter {
    interface:
        consider(payload: str): str
        stats(): str

    machine:
        $Accepting {
            consider(payload: str): str {
                if self.matches(payload):
                    self.passed = self.passed + 1
                    self.forward(payload)
                    @@:("passed")
                else:
                    self.dropped = self.dropped + 1
                    @@:("dropped")
            }
            stats(): str {
                @@:(f"passed={self.passed} dropped={self.dropped}")
            }
        }

    actions:
        matches(payload) {
            return "urgent" in payload
        }
        forward(payload) {
            print(f"-> {payload}")
        }

    domain:
        passed: int = 0
        dropped: int = 0
}

if __name__ == '__main__':
    f = @@MessageFilter()
    f.consider("urgent: reboot")
    f.consider("weekly digest")
    f.consider("urgent: patch")
    print(f.stats())        # passed=2 dropped=1
```

**How it works:** Structurally identical to the router but with a boolean predicate instead of an N-way branch. The filter's policy (`matches`) is a named action, making it easy to swap or test in isolation.

**Features used:** predicate action, domain counters, observability via a second interface method

-----

## 37. Aggregator

![37 state diagram](images/cookbook/37.svg)

**Problem:** Correlated messages arrive separately. Wait until the full set has arrived, then emit a single combined message.

```frame
@@target python_3

@@system Aggregator {
    interface:
        receive(correlation_id: str, part: str): str
        status(): str

    machine:
        $Collecting {
            receive(correlation_id: str, part: str): str {
                if correlation_id not in self.groups:
                    self.groups[correlation_id] = []
                self.groups[correlation_id].append(part)

                if len(self.groups[correlation_id]) >= self.expected_parts:
                    combined = ",".join(self.groups[correlation_id])
                    del self.groups[correlation_id]
                    self.emit(correlation_id, combined)
                    @@:("complete")
                else:
                    @@:("collecting")
            }
            status(): str {
                @@:(f"in_flight={len(self.groups)}")
            }
        }

    actions:
        emit(cid, combined) {
            print(f"[{cid}] -> {combined}")
        }

    domain:
        groups: dict = {}
        expected_parts: int = 3
}

if __name__ == '__main__':
    a = @@Aggregator()
    print(a.receive("A", "p1"))   # collecting
    print(a.receive("B", "p1"))   # collecting
    print(a.receive("A", "p2"))   # collecting
    print(a.receive("A", "p3"))   # complete  -> emits "p1,p2,p3"
    print(a.status())             # in_flight=1   (B still open)
```

**How it works:** `groups` is a correlation map from ID to parts-so-far. When a group reaches `expected_parts`, the aggregator emits the combined payload and removes the entry. One state is correct because the branching is driven by the correlation ID, not by where the aggregator "is."

**Features used:** correlation-keyed domain state, completeness check, emit-and-cleanup

-----

## 38. Resequencer

![38 state diagram](images/cookbook/38.svg)

**Problem:** Messages arrive out of order (each tagged with a sequence number). Release them downstream strictly in order, buffering anything premature.

```frame
@@target python_3

@@system Resequencer {
    interface:
        arrive(seq: int, payload: str): str
        status(): str

    machine:
        $Buffering {
            arrive(seq: int, payload: str): str {
                self.buffer[seq] = payload
                released = 0
                while self.next_seq in self.buffer:
                    p = self.buffer[self.next_seq]
                    del self.buffer[self.next_seq]
                    self.release(self.next_seq, p)
                    self.next_seq = self.next_seq + 1
                    released = released + 1
                if released > 0:
                    @@:(f"released {released}")
                else:
                    @@:("buffered")
            }
            status(): str {
                @@:(f"next={self.next_seq} buffered={len(self.buffer)}")
            }
        }

    actions:
        release(seq, payload) {
            print(f"  out #{seq}: {payload}")
        }

    domain:
        buffer: dict = {}
        next_seq: int = 1
}

if __name__ == '__main__':
    rs = @@Resequencer()
    print(rs.arrive(3, "c"))    # buffered  (waiting for 1)
    print(rs.arrive(1, "a"))    # released 1
    print(rs.arrive(2, "b"))    # released 2  (drains 2 and 3)
    print(rs.status())          # next=4 buffered=0
```

**How it works:** `next_seq` is the watermark. Every `arrive()` call adds to the buffer, then drains every contiguous run that's ready. The drain loop is native Python; Frame owns the buffer, the watermark, and the handler contract.

**Features used:** native loop in a handler, contiguous-range release, watermark progression

-----

## 39. Circuit Breaker

![39 state diagram](images/cookbook/39.svg)

**Problem:** A downstream dependency is failing. Stop hammering it; let it recover. Periodically probe; resume full traffic only after probes succeed.

```frame
@@target python_3

@@system CircuitBreaker {
    interface:
        call(payload: str): str
        record_failure()
        record_success()
        probe()
        state_name(): str

    machine:
        $Closed {
            $>() {
                self.failures = 0
            }
            call(payload: str): str {
                @@:(self.invoke(payload))
            }
            record_failure() {
                self.failures = self.failures + 1
                if self.failures >= self.threshold:
                    -> "trip" $Open
            }
            record_success() {
                self.failures = 0
            }
            state_name(): str { @@:("closed") }
        }

        $Open {
            call(payload: str): str {
                @@:("rejected: circuit open")
            }
            probe() {
                -> $HalfOpen
            }
            state_name(): str { @@:("open") }
        }

        $HalfOpen {
            call(payload: str): str {
                @@:(self.invoke(payload))
            }
            record_success() {
                -> "recover" $Closed
            }
            record_failure() {
                -> "re-trip" $Open
            }
            state_name(): str { @@:("half_open") }
        }

    actions:
        invoke(payload) {
            return f"ok:{payload}"
        }

    domain:
        failures: int = 0
        threshold: int = 3
}

if __name__ == '__main__':
    cb = @@CircuitBreaker()
    for _ in range(3):
        cb.call("x")
        cb.record_failure()
    print(cb.state_name())         # open
    print(cb.call("x"))            # rejected: circuit open
    cb.probe()
    print(cb.state_name())         # half_open
    cb.call("x")
    cb.record_success()
    print(cb.state_name())         # closed
```

**How it works:** The three canonical breaker states map directly. `call()` means different things in each: full pass-through in `$Closed`, immediate rejection in `$Open`, guarded pass-through in `$HalfOpen`. Failure and success arrive as separate interface methods because the breaker doesn't know whether a call succeeded — the host does. `$Closed`'s enter handler resets `failures` on every close, including after half-open recovery.

**Features used:** three-state lifecycle, enter handler as reset point, external observability signals

-----

## 40. Dead Letter Channel

![40 state diagram](images/cookbook/40.svg)

**Problem:** A message that can't be processed after N attempts must not block the pipeline. Move it to a dead-letter channel for inspection. The processor must survive restarts without losing retry state.

```frame
@@target python_3

@@persist

@@system DeadLetterProcessor {
    interface:
        accept(msg_id: str, payload: str): str
        process_tick(): str
        reset()

    machine:
        $Idle {
            accept(msg_id: str, payload: str): str {
                self.msg_id = msg_id
                self.payload = payload
                self.attempts = 0
                @@:("accepted")
                -> $Processing
            }
            process_tick(): str { @@:("idle") }
        }

        $Processing {
            $>() {
                self.attempts = self.attempts + 1
            }
            process_tick(): str {
                if self.try_process(self.payload):
                    @@:("ok")
                    -> "success" $Done
                else:
                    if self.attempts >= self.max_attempts:
                        @@:("dead_lettered")
                        -> "exhausted" $DeadLettered
                    else:
                        @@:("retrying")
                        -> "retry" $Processing
            }
        }

        $Done {
            process_tick(): str { @@:("done") }
            reset() { -> $Idle }
        }

        $DeadLettered {
            $>() {
                self.to_dlq(self.msg_id, self.payload)
            }
            process_tick(): str { @@:("dead_lettered") }
            reset() { -> $Idle }
        }

    actions:
        try_process(payload) {
            return False
        }
        to_dlq(msg_id, payload) {
            print(f"DLQ <- {msg_id}: {payload}")
        }

    domain:
        msg_id: str = ""
        payload: str = ""
        attempts: int = 0
        max_attempts: int = 3
}

if __name__ == '__main__':
    p = @@DeadLetterProcessor()
    p.accept("m-42", "flaky work")
    p.process_tick()        # retrying
    p.process_tick()        # retrying

    snapshot = p.save_state()
    p2 = DeadLetterProcessor.restore_state(snapshot)
    p2.process_tick()        # dead_lettered
```

**How it works:** `@@persist` makes the whole machine serializable — including `attempts`, the current payload, and which state it's in. Crashing halfway through a retry sequence doesn't reset the counter. `$Processing`'s enter handler increments `attempts` and the handler re-enters itself on failure (`-> $Processing`) — the enter handler fires each time because a self-transition fully exits and re-enters.

**Features used:** `@@persist` for crash-safe retry state, self-transition for retry loop, enter handler as side effect

-----

## 41. Polling Consumer

![41 state diagram](images/cookbook/41.svg)

**Problem:** No push transport available. Poll a source for messages, process them, pause on empty, and stop cleanly when asked.

```frame
@@target python_3

@@system PollingConsumer {
    operations:
        supply(msg: str) {
            self.pending.append(msg)
        }

    interface:
        start()
        stop()
        tick(): str
        state_name(): str

    machine:
        $Stopped {
            start() { -> $Polling }
            tick(): str { @@:("stopped") }
            state_name(): str { @@:("stopped") }
        }

        $Polling => $Active {
            tick(): str {
                msg = self.poll_source()
                if msg is None:
                    @@:("empty")
                    -> "no work" $Idle
                else:
                    self.current = msg
                    @@:("got_msg")
                    -> "dispatch" $Handling
            }
            state_name(): str { @@:("polling") }
            => $^
        }

        $Handling => $Active {
            $>() {
                self.handle(self.current)
                -> "done" $Polling
            }
            state_name(): str { @@:("handling") }
            => $^
        }

        $Idle => $Active {
            tick(): str {
                @@:("woke")
                -> $Polling
            }
            state_name(): str { @@:("idle") }
            => $^
        }

        $Active {
            stop() { -> $Stopped }
        }

    actions:
        poll_source() {
            if self.pending:
                return self.pending.pop(0)
            return None
        }
        handle(msg) {
            print(f"handled: {msg}")
        }

    domain:
        pending: list = []
        current: str = ""
}

if __name__ == '__main__':
    c = @@PollingConsumer()
    c.supply("a"); c.supply("b")
    c.start()
    print(c.tick())         # got_msg -> handles "a"
    print(c.tick())         # got_msg -> handles "b"
    print(c.tick())         # empty -> idle
    print(c.tick())         # woke  -> polling
    c.stop()
    print(c.state_name())   # stopped
```

**How it works:** Four states: `$Stopped`, `$Polling`, `$Handling`, `$Idle`. All three active states are children of `$Active`, which owns `stop()` — so any `stop()` from any active state reaches `$Stopped` without duplication. `$Handling`'s enter handler does the work and transitions back immediately (the transient state pattern). `supply()` is an operation — infrastructure plumbing that bypasses the state machine.

**Features used:** HSM for shared `stop()`, transient processing state, operations for non-dispatched utility

-----

## 42. Process Manager (Saga)

![42 state diagram](images/cookbook/42.svg)

**Problem:** Orchestrate a multi-step business transaction across services with no distributed transaction. If any step fails, compensate the prior steps in reverse order.

```frame
@@target python_3

@@system OrderSaga {
    interface:
        start(order_id: str)
        reserved()
        reserve_failed(reason: str)
        charged(tx_id: str)
        charge_failed(reason: str)
        shipped(tracking: str)
        ship_failed(reason: str)
        status(): str

    machine:
        $New {
            start(order_id: str) {
                self.order_id = order_id
                -> $Reserving
            }
            status(): str { @@:("new") }
        }

        $Reserving {
            $>() { self.call_reserve(self.order_id) }
            reserved() { -> $Charging }
            reserve_failed(reason: str) {
                self.failure = reason
                -> "abort" $Failed
            }
            status(): str { @@:("reserving") }
        }

        $Charging {
            $>() { self.call_charge(self.order_id) }
            charged(tx_id: str) {
                self.tx_id = tx_id
                -> $Shipping
            }
            charge_failed(reason: str) {
                self.failure = reason
                -> "compensate" $CompensatingReservation
            }
            status(): str { @@:("charging") }
        }

        $Shipping {
            $>() { self.call_ship(self.order_id) }
            shipped(tracking: str) {
                self.tracking = tracking
                -> $Completed
            }
            ship_failed(reason: str) {
                self.failure = reason
                -> "compensate" $CompensatingCharge
            }
            status(): str { @@:("shipping") }
        }

        $CompensatingCharge {
            $>() {
                self.call_refund(self.tx_id)
                -> "refunded" $CompensatingReservation
            }
            status(): str { @@:("compensating_charge") }
        }

        $CompensatingReservation {
            $>() {
                self.call_release(self.order_id)
                -> "released" $Failed
            }
            status(): str { @@:("compensating_reservation") }
        }

        $Completed {
            status(): str { @@:("completed") }
        }

        $Failed {
            status(): str { @@:(f"failed: {self.failure}") }
        }

    actions:
        call_reserve(oid)  { print(f"reserve {oid}") }
        call_charge(oid)   { print(f"charge {oid}") }
        call_ship(oid)     { print(f"ship {oid}") }
        call_refund(tx)    { print(f"refund {tx}") }
        call_release(oid)  { print(f"release reservation for {oid}") }

    domain:
        order_id: str = ""
        tx_id: str = ""
        tracking: str = ""
        failure: str = ""
}

if __name__ == '__main__':
    # Happy path
    s = @@OrderSaga()
    s.start("O-1")
    s.reserved()
    s.charged("T-9")
    s.shipped("UPS-42")
    print(s.status())   # completed

    # Compensating path: charge succeeds, ship fails
    s2 = @@OrderSaga()
    s2.start("O-2")
    s2.reserved()
    s2.charged("T-10")
    s2.ship_failed("carrier down")
    print(s2.status())  # failed: carrier down
```

**How it works:** Each forward step is a state with an enter handler that calls the external service. Success and failure events are separate interface methods so the host can call back exactly one. Compensation states do their undo work in the enter handler and transition to the next compensation. A failure at `$Shipping` cascades through `$CompensatingCharge` (refunds the charge) then `$CompensatingReservation` (releases inventory) then `$Failed`. A failure at `$Charging` skips straight to `$CompensatingReservation` because there's no charge to refund yet.

**Features used:** transient compensation states with enter-handler work, separate success/failure events per step, explicit rollback topology

-----

## 43. Competing Consumers

![43 state diagram](images/cookbook/43.svg)

**Problem:** One queue of work, multiple workers pulling from it. The dispatcher hands each message to exactly one worker; workers process in parallel.

```frame
@@target python_3

@@system Worker {
    interface:
        assign(msg: str)
        finish()
        busy(): bool

    machine:
        $Idle {
            assign(msg: str) {
                self.current = msg
                -> $Working
            }
            busy(): bool { @@:(False) }
        }
        $Working {
            $>() { print(f"worker[{self.name}] start: {self.current}") }
            finish() {
                print(f"worker[{self.name}] done")
                self.current = ""
                -> $Idle
            }
            busy(): bool { @@:(True) }
        }

    domain:
        name: str = ""
        current: str = ""
}

@@system Dispatcher {
    interface:
        submit(msg: str): str
        worker_free(idx: int)

    machine:
        $Running {
            submit(msg: str): str {
                i = 0
                while i < len(self.workers):
                    if not self.workers[i].busy():
                        self.workers[i].assign(msg)
                        @@:(f"dispatched to {i}")
                        return
                    i = i + 1
                self.backlog.append(msg)
                @@:("queued")
            }
            worker_free(idx: int) {
                if self.backlog:
                    msg = self.backlog.pop(0)
                    self.workers[idx].assign(msg)
            }
        }

    domain:
        workers: list = []
        backlog: list = []
}

if __name__ == '__main__':
    d = @@Dispatcher()
    w0 = @@Worker(); w0.name = "A"
    w1 = @@Worker(); w1.name = "B"
    d.workers = [w0, w1]

    print(d.submit("job-1"))   # dispatched to 0
    print(d.submit("job-2"))   # dispatched to 1
    print(d.submit("job-3"))   # queued
    w0.finish(); d.worker_free(0)
    w1.finish()
    w0.finish()
```

**How it works:** Two systems composed — the dispatcher holds a list of workers in its domain. Each worker is a two-state machine (`$Idle` / `$Working`) exposing `busy()` so the dispatcher can pick one. The host tells the dispatcher when a worker frees up (`worker_free(idx)`); the dispatcher has no threading model. Frame systems are passive — the competing-consumers topology lives in whatever runtime the host chooses.

**Features used:** multi-system composition, list-of-systems in domain, read-only interface method for decision-making

-----

## 44. Message Store

![44 state diagram](images/cookbook/44.svg)

**Problem:** Every message that flows through an integration should be persisted for audit, replay, and debugging. The store survives restarts.

```frame
@@target python_3

@@persist

@@system MessageStore {
    interface:
        record(topic: str, payload: str)
        count_for(topic: str): int
        total(): int

    machine:
        $Recording {
            record(topic: str, payload: str) {
                entry = {"topic": topic, "payload": payload, "seq": self.next_seq}
                self.log.append(entry)
                self.next_seq = self.next_seq + 1
            }
            count_for(topic: str): int {
                c = 0
                for e in self.log:
                    if e["topic"] == topic:
                        c = c + 1
                @@:(c)
            }
            total(): int { @@:(len(self.log)) }
        }

    domain:
        log: list = []
        next_seq: int = 1
}

if __name__ == '__main__':
    s = @@MessageStore()
    s.record("orders", "O-1")
    s.record("orders", "O-2")
    s.record("refunds", "R-1")

    snap = s.save_state()
    s2 = MessageStore.restore_state(snap)
    print(s2.total())             # 3
    print(s2.count_for("orders")) # 2
    s2.record("orders", "O-3")
    print(s2.total())             # 4
```

**How it works:** The entire audit log is a domain variable. `@@persist` serializes it along with `next_seq` so the store's identity survives restarts. A one-state machine is sufficient because storage is always open. A production store would write-through to durable storage per record, but the snapshot approach demonstrates that Frame's persistence covers the full domain payload, not just the current state.

**Features used:** `@@persist` across a large domain payload, list-of-dicts as log, single-state store

-----

## 45. Migrating Machine

![45 state diagram](images/cookbook/45.svg)

**Problem:** A state machine drives a workflow that spans a client and a server. Each side has work only it can do. The machine persists itself, travels across the wire, and resumes on the other side.

```frame
@@target python_3

import json

@@persist

@@system WizardMachine {
    operations:
        next_event(): str {
            s = self.__compartment.state
            if s == "NeedsServerValidate":   return "server_validate"
            if s == "NeedsServerProvision":  return "server_provision"
            if s == "NeedsClientCollect":    return "client_collect"
            if s == "NeedsClientConfirm":    return "client_confirm"
            return ""
        }

        summary(): str {
            return (
                f"user={self.user} email={self.email} "
                f"notes={self.server_notes} acct={self.account_id}"
            )
        }

    interface:
        start(user: str)
        client_collect(email: str)
        server_validate()
        client_confirm(accept: bool)
        server_provision()
        where_next(): str

    machine:
        $Start {
            start(user: str) {
                self.user = user
                -> $NeedsClientCollect
            }
            where_next(): str { @@:("start") }
        }

        $NeedsClientCollect {
            client_collect(email: str) {
                self.email = email
                -> $NeedsServerValidate
            }
            where_next(): str { @@:("client") }
        }

        $NeedsServerValidate {
            server_validate() {
                if "@" in self.email:
                    self.server_notes = "email syntax ok"
                    -> "valid" $NeedsClientConfirm
                else:
                    self.server_notes = "email rejected"
                    -> "invalid" $Rejected
            }
            where_next(): str { @@:("server") }
        }

        $NeedsClientConfirm {
            client_confirm(accept: bool) {
                if accept:
                    -> "accepted" $NeedsServerProvision
                else:
                    -> "declined" $Cancelled
            }
            where_next(): str { @@:("client") }
        }

        $NeedsServerProvision {
            server_provision() {
                self.account_id = f"acct-{self.user}"
                -> $Done
            }
            where_next(): str { @@:("server") }
        }

        $Done      { where_next(): str { @@:("done") } }
        $Cancelled { where_next(): str { @@:("done") } }
        $Rejected  { where_next(): str { @@:("done") } }

    domain:
        user: str = ""
        email: str = ""
        server_notes: str = ""
        account_id: str = ""
}


def handle_on_server(blob: bytes) -> bytes:
    m = WizardMachine.restore_state(blob)
    while m.where_next() == "server":
        evt = m.next_event()
        if evt == "server_validate":
            m.server_validate()
        elif evt == "server_provision":
            m.server_provision()
        else:
            break
    return m.save_state()


if __name__ == '__main__':
    m = @@WizardMachine()
    m.start("alice")
    m.client_collect("alice@example.com")

    blob = handle_on_server(m.save_state())
    m = WizardMachine.restore_state(blob)
    print(m.where_next())           # client

    m.client_confirm(True)

    blob = handle_on_server(m.save_state())
    m = WizardMachine.restore_state(blob)

    print(m.where_next())           # done
    print(m.summary())
```

**How it works:** Each state is tagged via `where_next()` with which side of the wire drives the next step. The client calls `save_state()`, ships the bytes, and the server calls `restore_state()`, drives its steps, and ships back. The state machine *is* the message — correlation IDs, per-request context, and partial-progress tracking all collapse into "the state of the machine."

`next_event()` is an operation that reads `self.__compartment.state` directly. Operations bypass the state machine dispatch, making them suitable for read-only introspection. The host asks the machine what it wants next without knowing the machine's internals.

**Features used:** `@@persist` as a transport payload, operations for introspection, side-tagged states, symmetric client/server code generation

-----

## Protocol & Systems Stress Tests

Recipes 46-49 model real-world protocols and safety-critical systems at full fidelity. Each exercises multiple Frame features simultaneously at production scale.

-----

## 46. FIX Protocol — Buy-Side Order Lifecycle

![46 state diagram](images/cookbook/46.svg)

**Problem:** Model the complete FIX protocol OrdStatus state machine from the buy-side (order sender) perspective. All 10 active states, full quantity tracking, precedence rules.

**Reference:** FIX Trading Community, Order State Change Matrices (FIX 4.4 / FIX Latest)

```frame
@@target python_3

@@system FixBuySideOrder(symbol: str, side: str, order_qty: float) {
    operations:
        static ord_status_precedence(status: str): int {
            # Higher number = higher precedence when order is in multiple states
            prec = {
                "PendingReplace": 10, "PendingCancel": 9, "PendingNew": 8,
                "Stopped": 7, "New": 6, "PartiallyFilled": 5,
                "Rejected": 4, "DoneForDay": 3, "Canceled": 2, "Filled": 1
            }
            return prec.get(status, 0)
        }
        get_order_qty(): float { return self.order_qty }
        get_cum_qty(): float { return self.cum_qty }
        get_leaves_qty(): float { return self.leaves_qty }
        get_avg_px(): float { return self.avg_px }
        get_cl_ord_id(): str { return self.cl_ord_id }
        get_status(): str { return @@:system.state }
        is_terminal(): bool {
            return @@:system.state in ["Filled", "Canceled", "Rejected"]
        }

    interface:
        # --- Outbound: buy-side sends to sell-side ---
        send_new(cl_ord_id: str)
        send_cancel(cancel_cl_ord_id: str)
        send_replace(replace_cl_ord_id: str, new_qty: float, new_price: float)

        # --- Inbound: execution reports from sell-side ---
        exec_new(exec_id: str)
        exec_rejected(exec_id: str, reason: str)
        exec_fill(exec_id: str, last_qty: float, last_px: float)
        exec_partial_fill(exec_id: str, last_qty: float, last_px: float)
        exec_canceled(exec_id: str)
        exec_replaced(exec_id: str, new_qty: float, new_price: float)
        exec_cancel_rejected(reason: str)
        exec_replace_rejected(reason: str)
        exec_stopped(exec_id: str, stop_px: float)
        exec_done_for_day(exec_id: str)
        exec_expired(exec_id: str)
        exec_suspended(exec_id: str)
        exec_restated(exec_id: str)

    machine:
        # =============================================
        #  IDLE — order not yet submitted
        # =============================================
        $Idle {
            send_new(cl_ord_id: str) {
                self.cl_ord_id = cl_ord_id
                self.orig_cl_ord_id = cl_ord_id
                print(f"[BUY] NewOrderSingle: {self.symbol} {self.side} {self.order_qty} clOrdId={cl_ord_id}")
                -> $PendingNew
            }
        }

        # =============================================
        #  PENDING NEW — awaiting acknowledgment
        # =============================================
        $PendingNew => $Active {
            $>() { print(f"[BUY] → PendingNew") }

            exec_new(exec_id: str) {
                print(f"[BUY] Order acknowledged: execId={exec_id}")
                -> $New
            }
            exec_rejected(exec_id: str, reason: str) {
                print(f"[BUY] Order REJECTED: {reason}")
                self.reject_reason = reason
                -> $Rejected
            }
            # IOC/FOK: immediate fill before New ack
            exec_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                print(f"[BUY] Immediate fill: {last_qty}@{last_px}")
                -> $Filled
            }
            exec_partial_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                print(f"[BUY] Immediate partial: {last_qty}@{last_px}")
                -> $PartiallyFilled
            }
            # IOC not filled → canceled
            exec_canceled(exec_id: str) {
                print(f"[BUY] IOC/FOK canceled immediately")
                -> $Canceled
            }
            => $^
        }

        # =============================================
        #  NEW — order acknowledged, resting on book
        # =============================================
        $New => $Active {
            $>() { print(f"[BUY] → New") }

            exec_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                -> $Filled
            }
            exec_partial_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                -> $PartiallyFilled
            }
            send_cancel(cancel_cl_ord_id: str) {
                self.pending_cl_ord_id = cancel_cl_ord_id
                print(f"[BUY] CancelRequest: clOrdId={cancel_cl_ord_id}")
                -> $PendingCancel
            }
            send_replace(replace_cl_ord_id: str, new_qty: float, new_price: float) {
                self.pending_cl_ord_id = replace_cl_ord_id
                self.pending_qty = new_qty
                self.pending_price = new_price
                print(f"[BUY] ReplaceRequest: clOrdId={replace_cl_ord_id} qty={new_qty} px={new_price}")
                -> $PendingReplace
            }
            exec_stopped(exec_id: str, stop_px: float) {
                self.stop_px = stop_px
                -> $Stopped
            }
            exec_done_for_day(exec_id: str) { -> $DoneForDay }
            exec_expired(exec_id: str) { -> $Canceled }
            exec_suspended(exec_id: str) { -> $Suspended }
            => $^
        }

        # =============================================
        #  PARTIALLY FILLED — has executions, leaves > 0
        # =============================================
        $PartiallyFilled => $Active {
            $>() {
                print(f"[BUY] → PartiallyFilled: cum={self.cum_qty} leaves={self.leaves_qty} avgPx={self.avg_px:.2f}")
            }

            exec_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                -> $Filled
            }
            exec_partial_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                # Stay in PartiallyFilled — re-enter for updated quantities
                -> $PartiallyFilled
            }
            send_cancel(cancel_cl_ord_id: str) {
                self.pending_cl_ord_id = cancel_cl_ord_id
                print(f"[BUY] CancelRequest (partial): clOrdId={cancel_cl_ord_id}")
                -> $PendingCancel
            }
            send_replace(replace_cl_ord_id: str, new_qty: float, new_price: float) {
                self.pending_cl_ord_id = replace_cl_ord_id
                self.pending_qty = new_qty
                self.pending_price = new_price
                -> $PendingReplace
            }
            exec_done_for_day(exec_id: str) { -> $DoneForDay }
            exec_expired(exec_id: str) { -> $Canceled }
            => $^
        }

        # =============================================
        #  PENDING CANCEL — cancel request sent, awaiting response
        #  Can still receive fills while cancel is pending
        # =============================================
        $PendingCancel => $Active {
            $>() { print(f"[BUY] → PendingCancel") }

            exec_canceled(exec_id: str) {
                self.cl_ord_id = self.pending_cl_ord_id
                print(f"[BUY] Cancel confirmed. cum={self.cum_qty}")
                -> $Canceled
            }
            exec_cancel_rejected(reason: str) {
                print(f"[BUY] Cancel REJECTED: {reason}")
                self.pending_cl_ord_id = ""
                # Return to previous effective state
                if self.cum_qty > 0:
                    -> "has fills" $PartiallyFilled
                else:
                    -> "no fills" $New
            }
            # Fills can arrive while cancel is pending
            exec_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                # Filled supersedes cancel
                -> $Filled
            }
            exec_partial_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                # Stay in PendingCancel — cancel still outstanding
                print(f"[BUY] Fill during PendingCancel: cum={self.cum_qty}")
            }
            => $^
        }

        # =============================================
        #  PENDING REPLACE — replace request sent
        # =============================================
        $PendingReplace => $Active {
            $>() { print(f"[BUY] → PendingReplace") }

            exec_replaced(exec_id: str, new_qty: float, new_price: float) {
                self.cl_ord_id = self.pending_cl_ord_id
                self.order_qty = new_qty
                self.price = new_price
                self.leaves_qty = new_qty - self.cum_qty
                print(f"[BUY] Replaced: qty={new_qty} px={new_price}")
                if self.cum_qty > 0:
                    -> "has fills" $PartiallyFilled
                else:
                    -> "no fills" $New
            }
            exec_replace_rejected(reason: str) {
                print(f"[BUY] Replace REJECTED: {reason}")
                self.pending_cl_ord_id = ""
                if self.cum_qty > 0:
                    -> "has fills" $PartiallyFilled
                else:
                    -> "no fills" $New
            }
            # Fills during pending replace
            exec_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                -> $Filled
            }
            exec_partial_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                print(f"[BUY] Fill during PendingReplace: cum={self.cum_qty}")
            }
            => $^
        }

        # =============================================
        #  STOPPED — guaranteed price by specialist/MM
        # =============================================
        $Stopped => $Active {
            $>() { print(f"[BUY] → Stopped at {self.stop_px}") }

            exec_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                -> $Filled
            }
            exec_partial_fill(exec_id: str, last_qty: float, last_px: float) {
                self.apply_fill(last_qty, last_px)
                -> $PartiallyFilled
            }
            => $^
        }

        # =============================================
        #  SUSPENDED — order suspended by exchange
        # =============================================
        $Suspended => $Active {
            $>() { print(f"[BUY] → Suspended") }

            exec_restated(exec_id: str) {
                if self.cum_qty > 0:
                    -> "has fills" $PartiallyFilled
                else:
                    -> "no fills" $New
            }
            exec_canceled(exec_id: str) { -> $Canceled }
            => $^
        }

        # =============================================
        #  ACTIVE — parent for all non-terminal, non-idle states
        #  Handles events common to all active order states
        # =============================================
        $Active {
            # Unsolicited cancel by exchange
            exec_canceled(exec_id: str) {
                print(f"[BUY] Unsolicited cancel")
                -> $Canceled
            }
        }

        # =============================================
        #  TERMINAL STATES — Filled, Canceled, Rejected, DoneForDay
        # =============================================
        $Filled {
            $>() {
                print(f"[BUY] FILLED: {self.cum_qty}@{self.avg_px:.2f}")
            }
            # Terminal — all events ignored
        }
        $Canceled {
            $>() {
                print(f"[BUY] CANCELED: filled={self.cum_qty}/{self.order_qty}")
            }
        }
        $Rejected {
            $>() {
                print(f"[BUY] REJECTED: {self.reject_reason}")
            }
        }
        $DoneForDay {
            $>() {
                print(f"[BUY] DONE FOR DAY: filled={self.cum_qty}/{self.order_qty}")
            }
        }

    actions:
        apply_fill(last_qty, last_px) {
            # VWAP calculation
            total_value = self.avg_px * self.cum_qty + last_px * last_qty
            self.cum_qty = self.cum_qty + last_qty
            self.leaves_qty = self.order_qty - self.cum_qty
            if self.cum_qty > 0:
                self.avg_px = total_value / self.cum_qty
            self.fill_count = self.fill_count + 1
        }

    domain:
        symbol: str = symbol
        side: str = side
        order_qty: float = order_qty
        cum_qty: float = 0.0
        leaves_qty: float = order_qty
        avg_px: float = 0.0
        price: float = 0.0
        stop_px: float = 0.0
        cl_ord_id: str = ""
        orig_cl_ord_id: str = ""
        pending_cl_ord_id: str = ""
        pending_qty: float = 0.0
        pending_price: float = 0.0
        reject_reason: str = ""
        fill_count: int = 0
}

if __name__ == '__main__':
    # --- Scenario 1: Simple fill ---
    order = @@FixBuySideOrder("AAPL", "Buy", 1000)
    order.send_new("ORD-001")
    order.exec_new("E1")
    order.exec_partial_fill("E2", 400, 150.25)
    order.exec_fill("E3", 600, 150.50)
    print(f"AvgPx: {order.get_avg_px():.4f}")  # 150.40

    # --- Scenario 2: Cancel after partial fill ---
    order2 = @@FixBuySideOrder("GOOG", "Sell", 500)
    order2.send_new("ORD-002")
    order2.exec_new("E4")
    order2.exec_partial_fill("E5", 200, 2800.00)
    order2.send_cancel("CXL-002")
    # Fill arrives while cancel is pending
    order2.exec_partial_fill("E6", 100, 2801.00)
    order2.exec_canceled("E7")
    print(f"Filled: {order2.get_cum_qty()}/{order2.get_order_qty()}")

    # --- Scenario 3: Replace ---
    order3 = @@FixBuySideOrder("MSFT", "Buy", 1000)
    order3.send_new("ORD-003")
    order3.exec_new("E8")
    order3.send_replace("RPL-003", 1500, 400.00)
    order3.exec_replaced("E9", 1500, 400.00)
    print(f"New qty: {order3.get_order_qty()}")  # 1500

    # --- Scenario 4: Rejection ---
    order4 = @@FixBuySideOrder("???", "Buy", -100)
    order4.send_new("ORD-004")
    order4.exec_rejected("E10", "Invalid quantity")
    print(f"Terminal: {order4.is_terminal()}")  # True
```

**How it works:**

**10 states matching FIX OrdStatus values.** `$PendingNew`, `$New`, `$PartiallyFilled`, `$PendingCancel`, `$PendingReplace`, `$Stopped`, `$Suspended`, `$Filled`, `$Canceled`, `$Rejected`, plus `$DoneForDay` and `$Idle`.

**HSM with `$Active` parent.** All non-terminal, non-idle states are children of `$Active`. Unsolicited cancel (exchange removes order without request) is handled once in `$Active` and inherited by all children.

**Fills during pending cancel/replace.** The FIX spec explicitly allows fills to arrive while a cancel or replace request is outstanding. `$PendingCancel` and `$PendingReplace` handle `exec_fill` and `exec_partial_fill` — a full fill supersedes the pending action, while a partial fill is recorded but the pending action stays outstanding.

**VWAP calculation in `apply_fill()`.** The action computes volume-weighted average price across multiple partial fills — `(oldAvgPx * oldCumQty + lastPx * lastQty) / newCumQty`.

**Operations for inspection.** `get_cum_qty()`, `get_leaves_qty()`, `get_avg_px()`, `is_terminal()` bypass the state machine for clean test access.

**Features stressed:** 13-state machine, HSM, system params (3 domain overrides), operations with `@@:system.state`, domain arithmetic (VWAP), conditional transitions based on quantity, fills during pending states, terminal states ignoring all events, actions modifying domain vars

---

## 47. FIX Protocol — Sell-Side Order Manager

![47 state diagram](images/cookbook/47.svg)

**Problem:** The broker/exchange side of FIX. Receives orders from the buy-side, manages the book, and sends execution reports back. Interacts with the buy-side system.

```frame
@@target python_3

@@system FixSellSide {
    interface:
        # Inbound from buy-side
        new_order(cl_ord_id: str, symbol: str, side: str, qty: float, price: float, tif: str)
        cancel_request(cancel_cl_ord_id: str, orig_cl_ord_id: str)
        replace_request(replace_cl_ord_id: str, orig_cl_ord_id: str, new_qty: float, new_price: float)

        # Market simulation
        market_fill(fill_qty: float, fill_px: float)

        # Inspection
        status(): str = ""

    machine:
        $NoOrder {
            new_order(cl_ord_id: str, symbol: str, side: str, qty: float, price: float, tif: str) {
                self.cl_ord_id = cl_ord_id
                self.symbol = symbol
                self.side = side
                self.order_qty = qty
                self.leaves_qty = qty
                self.price = price
                self.tif = tif
                self.exec_seq = 0

                if not self.validate_order(symbol, qty, price):
                    self.send_exec_report("Rejected", "rejected", "Invalid order params")
                    -> "invalid" $Rejected
                else:
                    -> "evaluate" $Accepting
            }
            status(): str { @@:("no order") }
        }

        $Accepting {
            $>() {
                # Check for immediate execution (IOC/FOK/marketable limit)
                if self.tif == "IOC" or self.tif == "FOK":
                    # Try immediate fill
                    available = self.get_market_liquidity()
                    if self.tif == "FOK" and available < self.order_qty:
                        self.send_exec_report("Canceled", "canceled", "FOK not fillable")
                        -> "FOK unfillable" $Canceled
                    elif available > 0:
                        fill_qty = min(available, self.leaves_qty)
                        self.execute_fill(fill_qty, self.price)
                        if self.leaves_qty <= 0:
                            self.send_exec_report("Filled", "fill", "")
                            -> "filled" $Filled
                        else:
                            if self.tif == "IOC":
                                self.send_exec_report("Canceled", "canceled", "IOC partial cancel")
                                -> "IOC remainder" $Canceled
                            else:
                                self.send_exec_report("PartiallyFilled", "partial_fill", "")
                                -> "partial" $Working
                    else:
                        if self.tif == "IOC" or self.tif == "FOK":
                            self.send_exec_report("Canceled", "canceled", f"{self.tif} no liquidity")
                            -> "no liquidity" $Canceled
                        else:
                            self.send_exec_report("New", "new", "")
                            -> "rest on book" $Working
                else:
                    self.send_exec_report("New", "new", "")
                    -> "accepted" $Working
            }
        }

        $Working => $BookActive {
            $>() { print(f"[SELL] Order working: {self.symbol} {self.side} leaves={self.leaves_qty}") }

            market_fill(fill_qty: float, fill_px: float) {
                actual = min(fill_qty, self.leaves_qty)
                self.execute_fill(actual, fill_px)
                if self.leaves_qty <= 0:
                    self.send_exec_report("Filled", "fill", "")
                    -> "filled" $Filled
                else:
                    self.send_exec_report("PartiallyFilled", "partial_fill", "")
            }
            cancel_request(cancel_cl_ord_id: str, orig_cl_ord_id: str) {
                if orig_cl_ord_id != self.cl_ord_id:
                    self.send_cancel_reject(cancel_cl_ord_id, "Unknown origClOrdId")
                else:
                    self.pending_action_id = cancel_cl_ord_id
                    -> $PendingCancel
            }
            replace_request(replace_cl_ord_id: str, orig_cl_ord_id: str, new_qty: float, new_price: float) {
                if orig_cl_ord_id != self.cl_ord_id:
                    self.send_replace_reject(replace_cl_ord_id, "Unknown origClOrdId")
                elif new_qty < self.cum_qty:
                    self.send_replace_reject(replace_cl_ord_id, "New qty less than cumQty")
                else:
                    self.pending_action_id = replace_cl_ord_id
                    self.pending_qty = new_qty
                    self.pending_price = new_price
                    -> $PendingReplace
            }
            => $^
        }

        $PendingCancel => $BookActive {
            $>() {
                print(f"[SELL] Processing cancel request")
                # Auto-accept cancel (in production: may queue or delay)
                self.cl_ord_id = self.pending_action_id
                self.send_exec_report("Canceled", "canceled", "")
                -> "confirmed" $Canceled
            }
            # Fills can still arrive
            market_fill(fill_qty: float, fill_px: float) {
                actual = min(fill_qty, self.leaves_qty)
                self.execute_fill(actual, fill_px)
                if self.leaves_qty <= 0:
                    # Filled supersedes cancel
                    self.send_exec_report("Filled", "fill", "")
                    -> "fill supersedes" $Filled
            }
            => $^
        }

        $PendingReplace => $BookActive {
            $>() {
                # Process the replace
                self.cl_ord_id = self.pending_action_id
                self.order_qty = self.pending_qty
                self.price = self.pending_price
                self.leaves_qty = self.pending_qty - self.cum_qty
                self.send_exec_report("Replaced", "replaced", "")
                if self.cum_qty > 0:
                    -> "resume" $Working
                else:
                    -> "resume" $Working
            }
            => $^
        }

        $BookActive {
            # Common handling for all orders on the book
        }

        # --- Terminal States ---
        $Filled {
            $>() { print(f"[SELL] FILLED: {self.cum_qty}@{self.avg_px:.2f}") }
            status(): str { @@:(f"filled {self.cum_qty}@{self.avg_px:.2f}") }
        }
        $Canceled {
            $>() { print(f"[SELL] CANCELED: filled={self.cum_qty}/{self.order_qty}") }
            status(): str { @@:(f"canceled cum={self.cum_qty}") }
        }
        $Rejected {
            $>() { print(f"[SELL] REJECTED") }
            status(): str { @@:("rejected") }
        }

    actions:
        validate_order(symbol, qty, price) {
            return qty > 0 and price >= 0 and len(symbol) > 0
        }
        execute_fill(qty, px) {
            total_value = self.avg_px * self.cum_qty + px * qty
            self.cum_qty = self.cum_qty + qty
            self.leaves_qty = self.order_qty - self.cum_qty
            if self.cum_qty > 0:
                self.avg_px = total_value / self.cum_qty
        }
        get_market_liquidity() {
            # Simulated: return available liquidity at limit price
            return 500
        }
        send_exec_report(ord_status, exec_type, text) {
            self.exec_seq = self.exec_seq + 1
            exec_id = f"EX-{self.exec_seq}"
            print(f"  [SELL→BUY] ExecReport: status={ord_status} type={exec_type} cum={self.cum_qty} leaves={self.leaves_qty}")
            if self.buy_side is not None:
                if exec_type == "new":
                    self.buy_side.exec_new(exec_id)
                elif exec_type == "fill":
                    self.buy_side.exec_fill(exec_id, self.cum_qty, self.avg_px)
                elif exec_type == "partial_fill":
                    last_qty = self.cum_qty  # simplified
                    self.buy_side.exec_partial_fill(exec_id, last_qty, self.avg_px)
                elif exec_type == "canceled":
                    self.buy_side.exec_canceled(exec_id)
                elif exec_type == "rejected":
                    self.buy_side.exec_rejected(exec_id, text)
                elif exec_type == "replaced":
                    self.buy_side.exec_replaced(exec_id, self.order_qty, self.price)
        }
        send_cancel_reject(cl_ord_id, reason) {
            print(f"  [SELL→BUY] CancelReject: {reason}")
            if self.buy_side is not None:
                self.buy_side.exec_cancel_rejected(reason)
        }
        send_replace_reject(cl_ord_id, reason) {
            print(f"  [SELL→BUY] ReplaceReject: {reason}")
            if self.buy_side is not None:
                self.buy_side.exec_replace_rejected(reason)
        }

    domain:
        buy_side = None
        cl_ord_id: str = ""
        symbol: str = ""
        side: str = ""
        order_qty: float = 0.0
        cum_qty: float = 0.0
        leaves_qty: float = 0.0
        avg_px: float = 0.0
        price: float = 0.0
        tif: str = "Day"
        exec_seq: int = 0
        pending_action_id: str = ""
        pending_qty: float = 0.0
        pending_price: float = 0.0
}

if __name__ == '__main__':
    sell = @@FixSellSide()

    # Submit a Day order — accepted, resting on book
    sell.new_order("ORD-001", "AAPL", "Buy", 1000, 150.00, "Day")

    # Partial fill from market
    sell.market_fill(400, 150.25)
    sell.market_fill(600, 150.50)

    print(f"Sell side: {sell.status()}")
```

**How it works:**

**Two systems interacting.** `FixSellSide.send_exec_report()` calls methods on `self.buy_side` when wired up — a reference to a `FixBuySideOrder` instance (Recipe 46). This is the managed-state callback pattern applied to protocol simulation. The test above exercises the sell-side standalone; wire `sell.buy_side = buy` to drive both sides together.

**`$Accepting` is a transient state.** Its enter handler evaluates TIF (Time In Force) rules: IOC orders get immediate partial fill then cancel of remainder; FOK orders are all-or-nothing; Day orders rest on the book. Each path leads to a different state.

**HSM with `$BookActive`.** `$Working`, `$PendingCancel`, and `$PendingReplace` are children. Shared cancel logic could go in the parent.

**Features stressed:** multi-system interaction (buy-side ↔ sell-side), transient state with complex branching, domain arithmetic, actions calling interface on other system, HSM, system params

---

## 48. Launch Sequence Controller — Abort from Any Phase

![48 state diagram](images/cookbook/48.svg)

**Problem:** A two-stage rocket launch with abort capability from every flight phase. Different abort modes depending on altitude. Enter-handler chain for the flight sequence. Three coordinating systems: FlightComputer, PropulsionController, RangeOfficer.

**Reference:** Apollo/Shuttle abort modes, SpaceX Falcon 9 flight profile

```frame
@@target python_3

# --- Propulsion Controller ---
@@system PropulsionController {
    interface:
        ignite_stage(stage: int)
        throttle(percent: int)
        shutdown()
        separate_stage()
        engine_status(): str = ""

    machine:
        $Idle {
            ignite_stage(stage: int) {
                self.active_stage = stage
                -> $Igniting
            }
            engine_status(): str { @@:("idle") }
        }
        $Igniting {
            $>() {
                print(f"  Stage {self.active_stage} ignition sequence...")
                self.thrust_pct = 0
                -> "ramp up" $RampingUp
            }
        }
        $RampingUp {
            $>() {
                self.thrust_pct = 100
                print(f"  Stage {self.active_stage} full thrust ({self.thrust_pct}%)")
                self.parent.engine_ready(self.active_stage)
            }
            throttle(percent: int) {
                self.thrust_pct = percent
                print(f"  Throttle: {percent}%")
            }
            shutdown() {
                -> $ShuttingDown
            }
            separate_stage() {
                print(f"  Stage {self.active_stage} separated")
                self.active_stage = self.active_stage + 1
                -> $Idle
            }
            engine_status(): str { @@:(f"stage {self.active_stage} at {self.thrust_pct}%") }
        }
        $ShuttingDown {
            $>() {
                print(f"  Stage {self.active_stage} shutdown")
                self.thrust_pct = 0
                -> "engines off" $Idle
            }
        }

    domain:
        parent = None
        active_stage: int = 0
        thrust_pct: int = 0
}

# --- Range Safety Officer ---
@@system RangeOfficer {
    interface:
        telemetry(altitude_km: float, velocity_ms: float, deviation_deg: float)
        status(): str = ""

    machine:
        $Nominal {
            telemetry(altitude_km: float, velocity_ms: float, deviation_deg: float) {
                if deviation_deg > 30:
                    print(f"  Range: ABORT COMMANDED ({deviation_deg} deg)")
                    self.parent.abort("range_safety")
                    -> "abort" $AbortCommanded
                elif deviation_deg > 10:
                    print(f"  Range: Trajectory deviation {deviation_deg} deg")
                    -> "deviation" $Caution
            }
            status(): str { @@:("nominal") }
        }
        $Caution {
            telemetry(altitude_km: float, velocity_ms: float, deviation_deg: float) {
                if deviation_deg <= 5:
                    print(f"  Range: Trajectory nominal")
                    -> "recovered" $Nominal
                elif deviation_deg > 30:
                    print(f"  Range: FLIGHT TERMINATION")
                    self.parent.abort("flight_termination")
                    -> "terminate" $AbortCommanded
            }
            status(): str { @@:("caution") }
        }
        $AbortCommanded {
            status(): str { @@:("abort commanded") }
        }

    domain:
        parent = None
}

# --- Flight Computer (main controller) ---
@@system FlightComputer {
    interface:
        start_countdown()
        engine_ready(stage: int)
        telemetry_tick(altitude_km: float, velocity_ms: float, deviation_deg: float)
        abort(reason: str)
        status(): str = ""

    machine:
        # =============================================
        #  PRE-FLIGHT
        # =============================================
        $PreFlight {
            start_countdown() {
                print("\n=== LAUNCH SEQUENCE INITIATED ===")
                -> $Countdown
            }
            status(): str { @@:("pre-flight") }
        }

        $Countdown => $InFlight {
            $.t_minus: int = 10

            $>() {
                print(f"  T-{$.t_minus}...")
                $.t_minus = $.t_minus - 1
                if $.t_minus <= 0:
                    -> "T-zero" $EngineIgnition
            }
            telemetry_tick(altitude_km: float, velocity_ms: float, deviation_deg: float) {
                $.t_minus = $.t_minus - 1
                print(f"  T-{$.t_minus}...")
                if $.t_minus <= 0:
                    -> "T-zero" $EngineIgnition
            }
            => $^
        }

        $EngineIgnition => $InFlight {
            $>() {
                print("  Engine ignition command")
                self.propulsion = @@PropulsionController()
                self.propulsion.parent = self
                self.range_officer = @@RangeOfficer()
                self.range_officer.parent = self
                self.propulsion.ignite_stage(1)
            }
            engine_ready(stage: int) {
                print("  All engines nominal")
                -> $Liftoff
            }
            => $^
        }

        # =============================================
        #  ASCENT PHASES — enter-handler chain
        # =============================================
        $Liftoff => $InFlight {
            $>() {
                self.altitude = 0
                print("\nLIFTOFF!")
                self.phase = "liftoff"
            }
            telemetry_tick(altitude_km: float, velocity_ms: float, deviation_deg: float) {
                self.altitude = altitude_km
                self.velocity = velocity_ms
                self.range_officer.telemetry(altitude_km, velocity_ms, deviation_deg)
                if altitude_km >= 11:
                    -> $MaxQ
            }
            => $^
        }

        $MaxQ => $InFlight {
            $>() {
                print("  MAX-Q — throttle down")
                self.propulsion.throttle(70)
                self.phase = "max-q"
            }
            telemetry_tick(altitude_km: float, velocity_ms: float, deviation_deg: float) {
                self.altitude = altitude_km
                self.velocity = velocity_ms
                self.range_officer.telemetry(altitude_km, velocity_ms, deviation_deg)
                if altitude_km >= 15:
                    self.propulsion.throttle(100)
                    print("  Throttle up")
                    -> $Stage1Ascent
            }
            => $^
        }

        $Stage1Ascent => $InFlight {
            $>() {
                self.phase = "stage1-ascent"
                print("  Stage 1 ascent")
            }
            telemetry_tick(altitude_km: float, velocity_ms: float, deviation_deg: float) {
                self.altitude = altitude_km
                self.velocity = velocity_ms
                self.range_officer.telemetry(altitude_km, velocity_ms, deviation_deg)
                if altitude_km >= 80:
                    -> $MECO
            }
            => $^
        }

        $MECO => $InFlight {
            $>() {
                print("  MECO — Main Engine Cut-Off")
                self.propulsion.shutdown()
                self.phase = "meco"
                -> "separate" $StageSeparation
            }
            => $^
        }

        $StageSeparation => $InFlight {
            $>() {
                print("  Stage separation")
                self.propulsion.separate_stage()
                self.phase = "separation"
                -> "ignite S2" $Stage2Ignition
            }
            => $^
        }

        $Stage2Ignition => $InFlight {
            $>() {
                print("  Stage 2 ignition")
                self.propulsion.ignite_stage(2)
                self.phase = "stage2-ignition"
            }
            engine_ready(stage: int) {
                print("  Stage 2 nominal")
                -> $Stage2Ascent
            }
            => $^
        }

        $Stage2Ascent => $InFlight {
            $>() {
                self.phase = "stage2-ascent"
                print("  Stage 2 ascent to orbit")
            }
            telemetry_tick(altitude_km: float, velocity_ms: float, deviation_deg: float) {
                self.altitude = altitude_km
                self.velocity = velocity_ms
                self.range_officer.telemetry(altitude_km, velocity_ms, deviation_deg)
                if altitude_km >= 200 and velocity_ms >= 7800:
                    -> $SECO
            }
            => $^
        }

        $SECO => $InFlight {
            $>() {
                print("  SECO — Second Engine Cut-Off")
                self.propulsion.shutdown()
                self.phase = "seco"
                -> "insert orbit" $OrbitInsertion
            }
            => $^
        }

        $OrbitInsertion => $InFlight {
            $>() {
                print(f"\nORBIT INSERTION at {self.altitude:.0f} km, {self.velocity:.0f} m/s")
                self.phase = "orbit"
                -> "nominal" $OnOrbit
            }
            => $^
        }

        # =============================================
        #  IN-FLIGHT parent — handles abort for ALL phases
        # =============================================
        $InFlight {
            abort(reason: str) {
                self.abort_reason = reason
                print(f"\nABORT: {reason} at altitude {self.altitude:.1f} km")
                self.propulsion.shutdown()
                if self.altitude < 30:
                    -> "pad escape" $AbortPadEscape
                elif self.altitude < 100:
                    -> "downrange" $AbortDownrange
                else:
                    -> "abort to orbit" $AbortToOrbit
            }
        }

        # =============================================
        #  ABORT MODES
        # =============================================
        $AbortPadEscape {
            $>() {
                print("  PAD ABORT — Launch escape system activated")
                print("  Capsule separation, parachute deployment")
                self.phase = "abort-pad"
                -> "complete" $AbortComplete
            }
        }
        $AbortDownrange {
            $>() {
                print("  DOWNRANGE ABORT — Capsule separation")
                print(f"  Ballistic trajectory from {self.altitude:.0f} km")
                self.phase = "abort-downrange"
                -> "complete" $AbortComplete
            }
        }
        $AbortToOrbit {
            $>() {
                print(f"  ABORT TO ORBIT — Inserting at {self.altitude:.0f} km")
                print("  Degraded orbit achieved")
                self.phase = "abort-to-orbit"
                -> "complete" $AbortComplete
            }
        }
        $AbortComplete {
            $>() {
                print(f"\nAbort sequence complete. Reason: {self.abort_reason}")
            }
            status(): str { @@:(f"aborted: {self.abort_reason}") }
        }

        # =============================================
        #  ON ORBIT — mission continues
        # =============================================
        $OnOrbit {
            $>() {
                print("  On orbit — mission nominal")
            }
            status(): str { @@:(f"on orbit at {self.altitude:.0f} km") }
        }

    domain:
        propulsion = None
        range_officer = None
        altitude: float = 0.0
        velocity: float = 0.0
        phase: str = "pre-flight"
        abort_reason: str = ""
}

if __name__ == '__main__':
    fc = @@FlightComputer()
    fc.start_countdown()

    # Countdown
    for t in range(10):
        fc.telemetry_tick(0, 0, 0)

    # Ascent
    profiles = [
        (5, 300, 0.5), (8, 500, 0.3), (11, 700, 0.2),     # liftoff → max-q
        (13, 800, 0.1), (15, 1000, 0.4),                    # max-q → stage1
        (30, 2000, 0.2), (50, 3500, 0.1), (80, 5000, 0.3),  # stage1 → MECO
        # MECO, separation, stage2 ignition happen in enter handlers
        (120, 6000, 0.2), (160, 7000, 0.1),                  # stage2 ascent
        (200, 7800, 0.05),                                     # orbit insertion
    ]
    for alt, vel, dev in profiles:
        fc.telemetry_tick(alt, vel, dev)

    print(f"\nFinal: {fc.status()}")
```

**How it works:**

**11 flight phases chain through enter handlers.** `$Countdown → $EngineIgnition → $Liftoff → $MaxQ → $Stage1Ascent → $MECO → $StageSeparation → $Stage2Ignition → $Stage2Ascent → $SECO → $OrbitInsertion → $OnOrbit`. Some transitions are driven by telemetry (altitude thresholds); others are immediate enter-handler chains (`$MECO → $StageSeparation → $Stage2Ignition`). The kernel loop handles all of them.

**HSM: abort from any phase.** All flight phases are children of `$InFlight`. `abort()` is handled once in `$InFlight` and inherited by every phase via `=> $^`. The abort handler selects the abort mode based on altitude: pad escape (<30km), downrange (30–100km), or abort-to-orbit (>100km). This matches real abort mode logic from Apollo and Shuttle.

**Three coordinating systems.** `FlightComputer` creates `PropulsionController` and `RangeOfficer` as managed children in `$EngineIgnition`. The propulsion controller manages engine states independently. The range officer monitors telemetry and can trigger abort via `self.parent.abort("range_safety")`.

**Features stressed:** 15+ states, deep HSM, 3-system composition, enter-handler chains (kernel loop at scale), conditional abort routing, managed children, domain variables for telemetry accumulation

---

## 49. Robot Arm Controller — Safety Overlay with HSM

![49 state diagram](images/cookbook/49.svg)

**Problem:** An industrial robot arm with three concerns layered via HSM: safety (emergency stop), operational mode (manual/auto), and motion states (idle/moving/gripping). Safety overrides everything.

**Reference:** IEC 61800-5-2 safe stop categories, ISO 10218 robot safety

```frame
@@target python_3

@@system RobotArm {
    operations:
        get_position(): dict {
            return {"x": self.pos_x, "y": self.pos_y, "z": self.pos_z}
        }
        get_velocity(): float {
            return self.current_velocity
        }
        get_state(): str {
            return @@:system.state
        }

    interface:
        # Mode control
        set_auto()
        set_manual()

        # Motion commands
        move_to(x: float, y: float, z: float)
        grip()
        release()
        stop_motion()

        # Program execution (auto mode only)
        run_program(program: list)
        program_step_done()

        # Safety
        e_stop()
        reset_e_stop()
        safety_fault(fault_code: str)

        # Telemetry
        tick()
        status(): str = ""

    machine:
        # =============================================
        #  SAFETY LAYER — top of HSM hierarchy
        #  E-stop overrides EVERYTHING
        # =============================================
        $Operational => $Safety {
            e_stop() {
                print("E-STOP ACTIVATED")
                self.e_stop_active = True
                self.current_velocity = 0
                -> $EStopCategory0
            }
            safety_fault(fault_code: str) {
                print(f"Safety fault: {fault_code}")
                self.current_velocity = 0
                -> $SafetyFault
            }
            => $^
        }

        $EStopCategory0 {
            $>() {
                print("  CAT-0: Immediate power removal")
                print("  Brakes engaged, drives disabled")
                self.drives_enabled = False
            }
            reset_e_stop() {
                print("  E-stop reset — performing safety check")
                -> $SafetyCheck
            }
            status(): str { @@:("E-STOP") }
        }

        $SafetyFault {
            $>() {
                print("  Safety fault active — motion disabled")
                self.drives_enabled = False
            }
            reset_e_stop() {
                -> $SafetyCheck
            }
            status(): str { @@:("safety fault") }
        }

        $SafetyCheck {
            $>() {
                print("  Running safety checks...")
                # In production: verify encoder positions, brake status, etc.
                if self.validate_safety():
                    self.drives_enabled = True
                    self.e_stop_active = False
                    print("  Safety check passed — drives enabled")
                    -> "cleared" $ManualIdle
                else:
                    print("  Safety check FAILED")
                    -> "failed" $SafetyFault
            }
        }

        $Safety {
            # Top-level parent — nothing here by default
        }

        # =============================================
        #  MANUAL MODE — jogging, teach pendant
        # =============================================
        $ManualIdle => $Manual {
            move_to(x: float, y: float, z: float) {
                self.target_x = x
                self.target_y = y
                self.target_z = z
                -> $ManualMoving
            }
            grip() { -> $ManualGripping }
            release() { -> $ManualReleasing }
            set_auto() { -> $AutoIdle }
            status(): str { @@:("manual idle") }
            => $^
        }

        $ManualMoving => $Manual {
            $>() {
                self.current_velocity = self.manual_speed_limit
                print(f"  Moving to ({self.target_x},{self.target_y},{self.target_z}) at {self.current_velocity} mm/s")
            }
            <$() { self.current_velocity = 0 }

            tick() {
                # Simulate motion
                self.pos_x = self.pos_x + (self.target_x - self.pos_x) * 0.3
                self.pos_y = self.pos_y + (self.target_y - self.pos_y) * 0.3
                self.pos_z = self.pos_z + (self.target_z - self.pos_z) * 0.3
                dist = ((self.target_x - self.pos_x)**2 + (self.target_y - self.pos_y)**2 + (self.target_z - self.pos_z)**2) ** 0.5
                if dist < 0.1:
                    self.pos_x = self.target_x
                    self.pos_y = self.target_y
                    self.pos_z = self.target_z
                    print(f"  Arrived at ({self.pos_x},{self.pos_y},{self.pos_z})")
                    -> $ManualIdle
            }
            stop_motion() {
                print("  Motion stopped")
                -> $ManualIdle
            }
            status(): str { @@:("manual moving") }
            => $^
        }

        $ManualGripping => $Manual {
            $>() {
                self.gripper_closed = True
                print("  Gripper closed")
                -> "done" $ManualIdle
            }
            => $^
        }

        $ManualReleasing => $Manual {
            $>() {
                self.gripper_closed = False
                print("  Gripper opened")
                -> "done" $ManualIdle
            }
            => $^
        }

        $Manual => $Operational {
            # Shared for all manual states
            set_auto() { -> $AutoIdle }
            => $^
        }

        # =============================================
        #  AUTO MODE — program execution
        # =============================================
        $AutoIdle => $Auto {
            run_program(program: list) {
                self.program = program
                self.program_step = 0
                print(f"  Auto: running program ({len(program)} steps)")
                -> $AutoExecuting
            }
            set_manual() { -> $ManualIdle }
            status(): str { @@:("auto idle") }
            => $^
        }

        $AutoExecuting => $Auto {
            $>() {
                if self.program_step >= len(self.program):
                    print("  Program complete")
                    -> "complete" $AutoIdle
                    return

                step = self.program[self.program_step]
                cmd = step["cmd"]
                print(f"  Step {self.program_step + 1}: {cmd}")
                self.current_velocity = self.auto_speed_limit

                if cmd == "move":
                    self.target_x = step["x"]
                    self.target_y = step["y"]
                    self.target_z = step["z"]
                elif cmd == "grip":
                    self.gripper_closed = True
                    self.program_step = self.program_step + 1
                    @@:self.program_step_done()
                elif cmd == "release":
                    self.gripper_closed = False
                    self.program_step = self.program_step + 1
                    @@:self.program_step_done()
            }

            tick() {
                # Simulate auto motion
                self.pos_x = self.pos_x + (self.target_x - self.pos_x) * 0.5
                self.pos_y = self.pos_y + (self.target_y - self.pos_y) * 0.5
                self.pos_z = self.pos_z + (self.target_z - self.pos_z) * 0.5
                dist = ((self.target_x - self.pos_x)**2 + (self.target_y - self.pos_y)**2 + (self.target_z - self.pos_z)**2) ** 0.5
                if dist < 0.1:
                    self.pos_x = self.target_x
                    self.pos_y = self.target_y
                    self.pos_z = self.target_z
                    self.program_step = self.program_step + 1
                    @@:self.program_step_done()
            }

            program_step_done() {
                -> $AutoExecuting
            }

            stop_motion() {
                print("  Auto motion stopped")
                self.current_velocity = 0
                -> $AutoIdle
            }
            status(): str { @@:(f"auto executing step {self.program_step + 1}/{len(self.program)}") }
            => $^
        }

        $Auto => $Operational {
            # Shared for all auto states
            set_manual() { -> $ManualIdle }
            # Reject motion commands in auto mode
            move_to(x: float, y: float, z: float) {
                print("  Manual move rejected — in auto mode")
            }
            => $^
        }

    actions:
        validate_safety() {
            # In production: check encoders, brakes, limits
            return True
        }

    domain:
        # Position
        pos_x: float = 0.0
        pos_y: float = 0.0
        pos_z: float = 0.0
        target_x: float = 0.0
        target_y: float = 0.0
        target_z: float = 0.0
        current_velocity: float = 0.0

        # Gripper
        gripper_closed: bool = False

        # Mode limits (mm/s)
        manual_speed_limit: float = 250.0
        auto_speed_limit: float = 1000.0

        # Safety
        drives_enabled: bool = True
        e_stop_active: bool = False

        # Program
        program: list = []
        program_step: int = 0
}

if __name__ == '__main__':
    arm = @@RobotArm()

    # --- Manual operation ---
    arm.move_to(100, 50, 200)
    for _ in range(10): arm.tick()
    print(f"Position: {arm.get_position()}")

    arm.grip()
    print(f"State: {arm.get_state()}")

    # --- Switch to auto ---
    arm.set_auto()
    program = [
        {"cmd": "move", "x": 200, "y": 100, "z": 50},
        {"cmd": "grip"},
        {"cmd": "move", "x": 0, "y": 0, "z": 200},
        {"cmd": "release"},
    ]
    arm.run_program(program)
    for _ in range(20): arm.tick()

    # --- E-stop during operation ---
    arm2 = @@RobotArm()
    arm2.move_to(500, 500, 500)
    arm2.tick()
    arm2.e_stop()
    print(f"After e-stop: {arm2.get_state()}")
    arm2.reset_e_stop()
    print(f"After reset: {arm2.get_state()}")
```

**How it works:**

**3-level HSM: Safety → Mode → Motion.**

```
$Safety
  └── $Operational
        ├── $Manual
        │     ├── $ManualIdle
        │     ├── $ManualMoving
        │     ├── $ManualGripping
        │     └── $ManualReleasing
        └── $Auto
              ├── $AutoIdle
              └── $AutoExecuting
$EStopCategory0
$SafetyFault
$SafetyCheck
```

`$Operational` handles `e_stop()` and `safety_fault()` — inherited by ALL operational states. E-stop works from `$ManualMoving`, `$AutoExecuting`, or any other operational state. This is the ISO 10218 safety overlay: the safety controller can override any operational state.

**Mode rejection.** `$Auto` rejects `move_to()` with a message — you can't jog in auto mode. `$Manual` doesn't handle `run_program()` — it's silently ignored. The state machine enforces mode-dependent command availability.

**Auto program execution.** `$AutoExecuting` reads the program step list, dispatches each command, and uses `@@:self.program_step_done()` to re-enter itself for the next step. This is the enter-handler chain pattern applied to program execution.

**Features stressed:** 14 states, 3-level HSM (deepest in the cookbook), operations with `@@:system.state`, mode-based event rejection, `@@:self.method()` for program stepping, transient states, enter/exit handlers for velocity management, domain variables for 3D position

---

## Deferred Event Processing

Recipes 50-52 demonstrate the **work queue pattern**: a system receives events it can't handle immediately, queues them, and processes them when it returns to an idle state. The enter handler on `$Idle` is the dequeue point — every transition back to idle checks for pending work. This is fundamentally different from "events ignored in wrong state." Here, events are *accepted* in every state but *deferred* until the system is ready.

-----

## 50. Print Spooler — Basic Work Queue

![50 state diagram](images/cookbook/50.svg)

**Problem:** A printer that accepts jobs while busy. Jobs are queued and printed in FIFO order. The printer processes one job at a time.

```frame
@@target python_3

@@system PrintSpooler {
    operations:
        queue_depth(): int {
            return len(self.queue)
        }
        peek_queue(): list {
            return [j["name"] for j in self.queue]
        }

    interface:
        submit(name: str, pages: int)
        tick()
        cancel_job(name: str): bool = False
        status(): str = ""

    machine:
        $Idle {
            $>() {
                if len(self.queue) > 0:
                    self.current_job = self.queue.pop(0)
                    print(f"[PRINT] Starting: {self.current_job['name']} ({self.current_job['pages']} pages)")
                    -> "dequeue" $Printing
            }

            submit(name: str, pages: int) {
                self.current_job = {"name": name, "pages": pages, "printed": 0}
                print(f"[PRINT] Starting: {name} ({pages} pages)")
                -> $Printing
            }
            status(): str { @@:("idle") }
        }

        $Printing {
            $.pages_done: int = 0

            $>() {
                $.pages_done = self.current_job.get("printed", 0)
            }

            submit(name: str, pages: int) {
                self.queue.append({"name": name, "pages": pages, "printed": 0})
                print(f"  [QUEUE] Added: {name} (queue depth: {len(self.queue)})")
            }

            tick() {
                $.pages_done = $.pages_done + 1
                total = self.current_job["pages"]
                done = $.pages_done
                print(f"  [PAGE] {done}/{total}: {self.current_job['name']}")
                if $.pages_done >= total:
                    self.jobs_completed = self.jobs_completed + 1
                    print(f"  [DONE] {self.current_job['name']}")
                    self.current_job = None
                    -> $Idle
            }

            cancel_job(name: str): bool {
                for i, job in enumerate(self.queue):
                    if job["name"] == name:
                        self.queue.pop(i)
                        @@:(True)
                        return
                @@:(False)
            }

            status(): str {
                total = self.current_job["pages"]
                done = $.pages_done
                @@:(f"printing {self.current_job['name']} ({done}/{total}), {len(self.queue)} queued")
            }
        }

    domain:
        queue: list = []
        current_job = None
        jobs_completed: int = 0
}

if __name__ == '__main__':
    p = @@PrintSpooler()
    p.submit("Report.pdf", 3)
    p.submit("Invoice.pdf", 2)
    p.submit("Photo.jpg", 1)
    print(f"Queue: {p.peek_queue()}")

    for _ in range(20):
        p.tick()
        if p.queue_depth() == 0 and p.status() == "idle":
            break

    print(f"Completed: {p.jobs_completed} jobs")
```

**How it works:** The dequeue point is `$Idle.$>()`. Every transition to `$Idle` triggers the enter handler, which checks `self.queue`. If there's a pending job, it pops the first one and immediately transitions to `$Printing`. The system never rests in `$Idle` while there's queued work.

`submit()` behaves differently per state. In `$Idle`, it starts the job immediately. In `$Printing`, it appends to the queue. Same interface, different behavior — the core value of state machines.

**Features used:** deferred event processing, enter handler as dequeue point, operations for queue inspection, same event with different per-state behavior, state variables for progress tracking

-----

## 51. Manufacturing Cell — Priority Queue with Sub-Phases

![51 state diagram](images/cookbook/51.svg)

**Problem:** A CNC machine tool that processes work orders through setup, machining, and teardown phases. New orders arrive at any time and are queued with priority. The machine processes the highest-priority job next.

```frame
@@target python_3

@@system ManufacturingCell {
    operations:
        queue_depth(): int {
            return len(self.queue)
        }
        parts_produced(): int {
            return self.completed_count
        }

    interface:
        work_order(order_id: str, part: str, program: str, priority: int)
        tick()
        emergency_stop()
        reset()
        status(): str = ""

    machine:
        $Idle {
            $>() {
                self.phase = "idle"
                if len(self.queue) > 0:
                    self.queue.sort(key=lambda x: -x["priority"])
                    self.current_job = self.queue.pop(0)
                    print(f"[CELL] Next job: {self.current_job['order_id']} (priority {self.current_job['priority']})")
                    -> "dequeue" $Setup
            }

            work_order(order_id: str, part: str, program: str, priority: int) {
                self.current_job = {
                    "order_id": order_id, "part": part,
                    "program": program, "priority": priority
                }
                print(f"[CELL] Starting: {order_id} ({part})")
                -> $Setup
            }
            status(): str { @@:("idle") }
        }

        $Setup => $Active {
            $.setup_ticks: int = 0
            $>() { self.phase = "setup" }
            tick() {
                $.setup_ticks = $.setup_ticks + 1
                if $.setup_ticks >= self.setup_time:
                    -> $Machining
            }
            status(): str { @@:(f"setup ({$.setup_ticks}/{self.setup_time})") }
            => $^
        }

        $Machining => $Active {
            $.cycle_ticks: int = 0
            $>() { self.phase = "machining" }
            tick() {
                $.cycle_ticks = $.cycle_ticks + 1
                if $.cycle_ticks >= self.cycle_time:
                    -> $Teardown
            }
            status(): str { @@:(f"machining ({$.cycle_ticks}/{self.cycle_time})") }
            => $^
        }

        $Teardown => $Active {
            $.teardown_ticks: int = 0
            $>() { self.phase = "teardown" }
            tick() {
                $.teardown_ticks = $.teardown_ticks + 1
                if $.teardown_ticks >= self.teardown_time:
                    self.completed_count = self.completed_count + 1
                    self.current_job = None
                    -> $Idle
            }
            status(): str { @@:(f"teardown ({$.teardown_ticks}/{self.teardown_time})") }
            => $^
        }

        $Active {
            emergency_stop() {
                print(f"  [E-STOP] During {self.phase}")
                if self.current_job is not None:
                    self.current_job["priority"] = 999
                    self.queue.insert(0, self.current_job)
                    self.current_job = None
                -> $EStop
            }
            work_order(order_id: str, part: str, program: str, priority: int) {
                self.queue.append({
                    "order_id": order_id, "part": part,
                    "program": program, "priority": priority
                })
            }
        }

        $EStop {
            $>() { self.phase = "e-stop" }
            reset() { -> $Idle }
            work_order(order_id: str, part: str, program: str, priority: int) {
                self.queue.append({
                    "order_id": order_id, "part": part,
                    "program": program, "priority": priority
                })
            }
            status(): str { @@:(f"e-stop ({len(self.queue)} queued)") }
        }

    domain:
        queue: list = []
        current_job = None
        completed_count: int = 0
        phase: str = "idle"
        setup_time: int = 2
        cycle_time: int = 3
        teardown_time: int = 1
}

if __name__ == '__main__':
    cell = @@ManufacturingCell()
    cell.work_order("WO-001", "Bracket-A", "prog_bracket.nc", 1)
    cell.work_order("WO-002", "Shaft-B", "prog_shaft.nc", 2)
    cell.work_order("WO-003", "Housing-C", "prog_housing.nc", 5)

    for i in range(30):
        cell.tick()
        if cell.queue_depth() == 0 and cell.status() == "idle":
            break

    print(f"Parts produced: {cell.parts_produced()}")
```

**How it works:** Priority queue, not FIFO. `$Idle.$>()` sorts the queue by descending priority before popping. High-priority jobs jump the queue. Three sub-phases (`$Setup`, `$Machining`, `$Teardown`) are children of `$Active`, which handles `emergency_stop()` and `work_order()` for all of them. E-stop re-queues the interrupted job at priority 999 so it resumes first after reset.

**Features used:** priority queue dequeue, HSM with shared e-stop, sub-phase progression, state variables as phase timers, events accepted in all states including e-stop

-----

## 52. Elevator — Directional Scan Algorithm

![52 state diagram](images/cookbook/52.svg)

**Problem:** An elevator services floor requests using the SCAN algorithm: continue in the current direction until all requests in that direction are served, then reverse. Requests arrive at any time and are accumulated, not ignored.

```frame
@@target python_3

@@system Elevator {
    operations:
        current_floor(): int {
            return self.floor
        }
        pending_requests(): list {
            return sorted(self.requests)
        }

    interface:
        request(floor: int)
        tick()
        close_doors()
        status(): str = ""

    machine:
        $Idle {
            $>() {
                if len(self.requests) > 0:
                    self.select_direction()
                    -> "dequeue" $Moving
            }

            request(floor: int) {
                if floor == self.floor:
                    -> "at floor" $DoorsOpen
                else:
                    self.requests.add(floor)
                    self.select_direction()
                    -> $Moving
            }
            status(): str { @@:(f"idle at floor {self.floor}") }
        }

        $Moving {
            $>() {
                target = self.next_stop()
                if target is None:
                    -> "no target" $Idle
            }

            request(floor: int) {
                self.requests.add(floor)
            }

            tick() {
                if self.dir == "up":
                    self.floor = self.floor + 1
                else:
                    self.floor = self.floor - 1

                if self.floor in self.requests:
                    self.requests.discard(self.floor)
                    self.stops_made = self.stops_made + 1
                    -> "serve floor" $DoorsOpen
                else:
                    target = self.next_stop()
                    if target is None:
                        self.reverse_direction()
                        target = self.next_stop()
                        if target is None:
                            -> "all served" $Idle
            }

            status(): str { @@:(f"moving {self.dir} at floor {self.floor}") }
        }

        $DoorsOpen {
            $.dwell_ticks: int = 0

            $>() {
                print(f"  [DOORS] Open at floor {self.floor}")
            }

            request(floor: int) {
                if floor == self.floor:
                    $.dwell_ticks = 0
                else:
                    self.requests.add(floor)
            }

            tick() {
                $.dwell_ticks = $.dwell_ticks + 1
                if $.dwell_ticks >= self.dwell_time:
                    -> $DoorsClosing
            }

            close_doors() {
                -> $DoorsClosing
            }

            status(): str { @@:(f"doors open at floor {self.floor}") }
        }

        $DoorsClosing {
            $>() {
                if len(self.requests) > 0:
                    self.select_direction()
                    -> "resume" $Moving
                else:
                    -> "park" $Idle
            }
        }

    actions:
        select_direction() {
            up_requests = [f for f in self.requests if f > self.floor]
            down_requests = [f for f in self.requests if f < self.floor]
            if self.dir == "up":
                if len(up_requests) > 0:
                    self.dir = "up"
                elif len(down_requests) > 0:
                    self.dir = "down"
            else:
                if len(down_requests) > 0:
                    self.dir = "down"
                elif len(up_requests) > 0:
                    self.dir = "up"
        }

        next_stop() {
            if self.dir == "up":
                ahead = sorted([f for f in self.requests if f > self.floor])
                if len(ahead) > 0:
                    return ahead[0]
            else:
                ahead = sorted([f for f in self.requests if f < self.floor], reverse=True)
                if len(ahead) > 0:
                    return ahead[0]
            return None
        }

        reverse_direction() {
            if self.dir == "up":
                self.dir = "down"
            else:
                self.dir = "up"
        }

    domain:
        floor: int = 1
        dir: str = "up"
        requests: set = set()
        dwell_time: int = 2
        stops_made: int = 0
}

if __name__ == '__main__':
    elev = @@Elevator()
    elev.request(5)
    elev.request(3)
    elev.request(8)

    for _ in range(20):
        elev.tick()

    print(f"Stops made: {elev.stops_made}")
    print(f"Final floor: {elev.current_floor()}")
```

**How it works:** The elevator continues in its current direction as long as there are requests ahead, then reverses. Requests are accepted in every state — `$Moving`, `$DoorsOpen`, and `$DoorsClosing` all handle `request()` by adding to `self.requests` (a set, so duplicates are ignored). `$DoorsClosing` is a transient state whose enter handler selects direction and transitions to `$Moving` or `$Idle`. `$Idle.$>()` is the dequeue point — the elevator never idles with pending requests.

**Features used:** SCAN algorithm with directional logic, set as domain variable for deduplication, transient states, enter handler as dequeue and direction-select point, state variables for dwell timer, requests accepted in all states

-----

## Scanner and Parser Recipes

The next two recipes stand alone as a pair: a lexical scanner and a pushdown parser, composed to form a minimal "Frame as parser generator" pipeline. They answer the Ragel-style question — *can Frame do what a dedicated scanner/parser generator does?* — with two small systems totaling fewer than a hundred Frame lines. Every feature used is already in play elsewhere in the cookbook; the scanner uses state variables and `@@:self` for delimiter replay, the parser uses `push$` / `pop$` as a call stack.

-----

## 53. Byte Scanner — Tokenize a Simple Language

![53 state diagram](images/cookbook/53.svg)

**Problem:** Tokenize an input string into identifiers, numbers, strings, and punctuation — the scanning half of a parser. The input `set x = 42 "hello"` should emit:

```
IDENT(set) IDENT(x) PUNCT(=) NUMBER(42) STRING(hello) EOF
```

Scanners are the classic state-machine workload. Frame handles it naturally because every scanner is a state machine: you sit in a mode (`$Start`, `$InIdent`, `$InNumber`, `$InString`), consume a byte, decide whether to stay, emit, or change modes, and loop.

```frame
@@target python_3

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
                    self.buf = ""
                    -> $InString
                elif ch.isspace():
                    return
                else:
                    self.emit("PUNCT", ch)
            }
            eof() {
                self.emit("EOF", "")
            }
            tokens(): list {
                @@:(self.out)
            }
        }

        $InIdent {
            feed(ch: str) {
                if ch.isalnum() or ch == "_":
                    self.buf = self.buf + ch
                else:
                    self.emit("IDENT", self.buf)
                    -> $Start
                    @@:self.feed(ch)      # replay the delimiter byte
            }
            eof() {
                self.emit("IDENT", self.buf)
                self.emit("EOF", "")
            }
        }

        $InNumber {
            feed(ch: str) {
                if ch.isdigit():
                    self.buf = self.buf + ch
                else:
                    self.emit("NUMBER", self.buf)
                    -> $Start
                    @@:self.feed(ch)
            }
            eof() {
                self.emit("NUMBER", self.buf)
                self.emit("EOF", "")
            }
        }

        $InString {
            feed(ch: str) {
                if ch == '"':
                    self.emit("STRING", self.buf)
                    -> $Start
                else:
                    self.buf = self.buf + ch
            }
            eof() {
                self.emit("ERROR", "unterminated string")
            }
        }

    actions:
        emit(kind: str, lexeme: str) {
            self.out.append(f"{kind}({lexeme})")
        }

    domain:
        buf: str = ""
        out: list = []
}

if __name__ == '__main__':
    s = @@Scanner()
    for ch in 'set x = 42 "hello"':
        s.feed(ch)
    s.eof()
    print(s.tokens())
    # ['IDENT(set)', 'IDENT(x)', 'PUNCT(=)', 'NUMBER(42)', 'STRING(hello)', 'EOF()']
```

**How it works:** Each mode is a state. `self.buf` is a domain variable — persistent across transitions so the accumulated token survives the move back to `$Start`. The tokenized output accumulates in `self.out` for the same reason.

The characteristic bit is **delimiter replay**: when `$InIdent` sees a non-identifier byte, it has to emit the identifier *and* let that byte restart scanning as something else. `@@:self.feed(ch)` re-dispatches the byte through the kernel. Because the kernel processes the pending `-> $Start` transition before the replayed `feed` runs, the byte arrives in `$Start` and scanning resumes cleanly. This is the `@@:self` pattern (RFC-0006) — with the transition-guard semantics making the "replay after transition" form safe to write.

**Features used:** domain variables for buffer/output persistence across transitions, `@@:self.method()` for byte replay after a transition, states-as-modes, `return` to stay in the current state without emission.

-----

## 54. Pushdown Parser — Nested Structure with `push$` / `pop$`

![54 state diagram](images/cookbook/54.svg)

**Problem:** Recognize balanced bracket structures like `[1, [2, 3], 4]` and rebuild the nested Python list. A flat scanner cannot do this — nested structures need a stack. Frame's `push$` / `pop$` give you one without writing a separate data structure.

This recipe is the natural complement to #53: the scanner handles token recognition, the parser handles structure.

```frame
@@target python_3

@@system BracketParser {
    interface:
        open()
        close()
        value(v: int)
        result(): list

    machine:
        $Flat {
            open() {
                push$
                -> $Nested
            }
            close() {
                self.emit_error("unbalanced close at top level")
            }
            value(v: int) {
                self.items.append(v)
            }
            result(): list {
                @@:(self.items)
            }
        }

        $Nested {
            $.items: list

            open() {
                push$
                -> $Nested
            }
            close() {
                self.bubble_up($.items)
                -> pop$
            }
            value(v: int) {
                $.items.append(v)
            }
            result(): list {
                @@:($.items)
            }
        }

    actions:
        bubble_up(items: list) {
            # pop$ will restore _state_stack[-1] as the current compartment.
            # If it's another $Nested, append our items as a sublist
            # (preserves structure). If we're about to return to $Flat
            # (the outermost frame), spread into the domain so the
            # top-level list is flat at depth 0.
            if len(self._state_stack) > 0:
                parent = self._state_stack[-1]
                if "items" in parent.state_vars:
                    parent.state_vars["items"].append(items)
                else:
                    self.items.extend(items)
            else:
                self.items.extend(items)
        }
        emit_error(msg: str) {
            print(f"parse error: {msg}")
        }

    domain:
        items: list = []
}

if __name__ == '__main__':
    p = @@BracketParser()
    # Input: [1, [2, 3], 4]
    p.open()
    p.value(1)
    p.open()
    p.value(2)
    p.value(3)
    p.close()            # inner list [2, 3] bubbles into outer $Nested
    p.value(4)
    p.close()            # full [1, [2, 3], 4] bubbles out to $Flat's domain
    print(p.result())
    # [1, [2, 3], 4]
```

**How it works:** Each `open()` pushes the current compartment and enters a fresh `$Nested`. The new compartment has its own `$.items` (per-compartment state variable, empty by default for the declared `list` type), so sibling nested lists never alias. When `close()` fires, the action `bubble_up($.items)` delivers the collected list to the compartment that `pop$` is about to restore — peeked via `self._state_stack[-1]` — and only then does the transition happen. `$Nested → $Nested` appends the sub-list as a single element (preserving nesting); `$Nested → $Flat` spreads into the domain list (so the outer bracket pair contributes its contents directly, not as one wrapped element).

The shape of the code is the point: `push$` is `call`, `pop$` is `ret`, the compartment is the activation record, `$.items` is a local. Frame's state stack is a proper pushdown automaton — exactly what's needed to recognize nested grammars that regular state machines (flat ones, like #53) cannot.

**Features used:** `push$` for activation records, `-> pop$` for return, typed per-state variable (`$.items: list`) for compartment-local state, action reading the saved compartment via `self._state_stack[-1]` for cross-frame data transfer.

### Running #53 and #54 Together

```python
src = '[1, [2, 3], 4]'
s = Scanner()
for ch in src: s.feed(ch)
s.eof()

p = BracketParser()
for tok in s.tokens():
    if tok.startswith('PUNCT(['):      p.open()
    elif tok.startswith('PUNCT(]'):    p.close()
    elif tok.startswith('NUMBER('):    p.value(int(tok[7:-1]))
print(p.result())
# [1, [2, 3], 4]
```

Two small Frame systems, composed, produce a scanner + parser pipeline in plain dependency-free Python — and in any of the other sixteen target languages with no codegen changes.

-----

## Feature Coverage

|Feature                      |Recipes 1-22|Recipes 23-33   |EIP (34-45)         |Stress (46-49)       |Deferred (50-52)     |
|-----------------------------|------------|----------------|---------------------|---------------------|---------------------|
|`@@:(expr)` return           |yes         |yes all         |yes all              |yes all              |yes all              |
|`@@:return(expr)` exit sugar |yes #22     |yes #28         |no                   |no                   |no                   |
|`@@:self.method()`           |yes #22     |yes #33         |no                   |yes #49              |no                   |
|`@@:system.state`            |yes #22     |yes #32         |no                   |yes #46, #49         |no                   |
|Operations                   |no          |yes #23, #25, #32|yes #41, #45        |yes #46 (7), #49 (3) |yes #50, #51         |
|`static` operations          |no          |yes #25         |no                   |yes #46              |no                   |
|System params (domain)       |yes #21     |yes #23         |no                   |yes #46 (3)          |no                   |
|HSM 3-level                  |no          |yes #26         |no                   |yes #49 (3-level)    |no                   |
|`push$` / `-> pop$`          |yes #7, #8  |yes #27         |no                   |no                   |no                   |
|Decorated pop (exit args)    |no          |yes #27         |no                   |no                   |no                   |
|State var reset on reentry   |implicit    |yes #24 (explicit)|no                 |yes #48              |yes #50, #51, #52    |
|Multi-system managed states  |yes #20     |yes #28, #29, #33|yes #43             |yes #47, #48 (3)     |no                   |
|Service pattern              |no          |yes #30         |no                   |no                   |yes #50 (dequeue)    |
|Enter-handler chain          |no          |yes #30, #31    |yes #42             |yes #48 (11 phases)  |no                   |
|Events ignored in wrong state|yes #3, #12 |yes #24, #32    |yes #39             |yes #46 (terminals)  |no                   |
|`@@persist`                  |yes #18     |no              |yes #40, #44, #45   |no                   |no                   |
|Self-transition (retry loop) |no          |no              |yes #40             |no                   |no                   |
|HSM parent forwarding        |yes #9      |yes #26         |yes #41             |yes #46, #48, #49    |yes #51              |
|Compensation chain           |no          |no              |yes #42             |no                   |no                   |
|Transient states             |no          |yes #30         |yes #41, #42        |yes #47, #48         |yes #52              |
|13+ state machine            |no          |no              |no                   |yes #46 (13), #48 (17)|no                  |
|Domain arithmetic (VWAP)     |no          |no              |no                   |yes #46, #47         |no                   |
|Conditional abort routing    |no          |no              |no                   |yes #48              |no                   |
|Mode-based event rejection   |no          |no              |no                   |yes #49              |no                   |
|Deferred event processing    |no          |no              |no                   |no                   |yes #50, #51, #52    |
|Priority queue               |no          |no              |no                   |no                   |yes #51              |
|Directional scheduling       |no          |no              |no                   |no                   |yes #52 (SCAN)       |
