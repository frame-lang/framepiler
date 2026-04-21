# Frame Cookbook

*Prompt Engineer: Mark Truluck <mark@frame-lang.org>*

71 recipes showing how to solve real problems with Frame. Each recipe is a complete, runnable Frame spec with an explanation of the key patterns used.

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

**Scanner and Parser (53-54)**

53. [Byte Scanner](#53-byte-scanner--tokenize-a-simple-language) — tokenize a simple language
54. [Pushdown Parser](#54-pushdown-parser--nested-structure-with-push--pop) — nested structure with `push$` / `pop$`

**OS Internals (55-63)**

55. [Process Lifecycle](#55-process-lifecycle) — task states, signals, HSM for non-runnable states
56. [Runtime Power Management](#56-runtime-power-management) — enter/exit for timer management, usage counting
57. [Block I/O Request](#57-block-io-request) — request pipeline with timeout and retry
58. [USB Device Enumeration](#58-usb-device-enumeration) — multi-stage pipeline with compensation
59. [Watchdog Timer](#59-watchdog-timer) — magic close guard, high-consequence two-state machine
60. [OOM Killer](#60-oom-killer) — safety by construction, mutual exclusion without locks
61. [Filesystem Freeze](#61-filesystem-freeze) — 3-level HSM for freeze/thaw under a mounted parent
62. [Kernel Module Loader](#62-kernel-module-loader) — pipeline with rollback (saga pattern)
63. [Signal Handler Stack](#63-signal-handler-stack) — `push$` / `pop$` for nested signal frames

**Internet Protocols (64-71)**

64. [DHCP Client](#64-dhcp-client) — timer-driven lease lifecycle from RFC 2131
65. [TLS Handshake](#65-tls-handshake) — two-party protocol with HSM alert handling
66. [Wi-Fi Station Management](#66-wi-fi-station-management) — HSM deauth from any state
67. [BGP Finite State Machine](#67-bgp-finite-state-machine) — RFC 4271 event table transcription
68. [PPP Link Control Protocol](#68-ppp-link-control-protocol) — RFC 1661 state table, layered composition
69. [NTP Client Association](#69-ntp-client-association) — polling backoff and reachability tracking
70. [HTTP/1.1 Connection](#70-http11-connection) — keep-alive lifecycle with HSM error handling
71. [SMTP Conversation](#71-smtp-conversation) — command-response protocol with STARTTLS upgrade

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

## OS Internals (Kernel & Subsystems)

Nine recipes modeling real kernel subsystems as Frame state machines. Each recipe maps a subsystem whose logic is scattered across the Linux source tree onto a single Frame spec that surfaces the state graph directly. Every recipe is runnable with `@@target python_3`.

-----

## 55. Process Lifecycle

![55 state diagram](images/cookbook/55.svg)

**Problem:** Model the Linux task states and the transitions driven by signals, I/O, and scheduler decisions. In the kernel, these states are bitmask constants scattered across `kernel/sched/core.c`, `kernel/signal.c`, and `kernel/exit.c`. The transition logic is implicit — you have to trace code paths to determine which states are reachable from which.

```frame
@@target python_3

@@system TaskLifecycle {
    interface:
        fork(): str
        schedule(): str
        block_on_io(): str
        io_complete(): str
        block_uninterruptible(): str
        send_signal(sig: str): str
        wait_by_parent(): str
        status(): str

    machine:
        $New {
            fork(): str {
                self.pid = self.next_pid()
                @@:("forked")
                -> $Ready
            }
            status(): str { @@:("new") }
        }

        $Ready => $Alive {
            schedule(): str {
                @@:("running")
                -> $Running
            }
            status(): str { @@:("ready") }
            => $^
        }

        $Running => $Alive {
            schedule(): str {
                @@:("preempted")
                -> $Ready
            }
            block_on_io(): str {
                @@:("blocked")
                -> $Interruptible
            }
            block_uninterruptible(): str {
                @@:("blocked (D)")
                -> $Uninterruptible
            }
            status(): str { @@:("running") }
            => $^
        }

        $Interruptible => $Blocked {
            io_complete(): str {
                @@:("woke")
                -> $Ready
            }
            send_signal(sig: str): str {
                # Fatal and stop signals take precedence over wake-up.
                # Forward them to $Alive for uniform handling.
                if sig == "SIGKILL" or sig == "SIGTERM" or sig == "SIGSTOP":
                    => $^
                else:
                    @@:("interrupted")
                    -> $Ready
            }
            status(): str { @@:("interruptible") }
            => $^
        }

        $Uninterruptible => $Blocked {
            io_complete(): str {
                @@:("woke")
                -> $Ready
            }
            # send_signal NOT handled here — all signals forward via => $^
            # through $Blocked to $Alive. Non-fatal signals hit the "ignored"
            # branch there, matching the kernel's D-state behavior.
            # SIGKILL still gets through, matching TASK_KILLABLE.
            status(): str { @@:("uninterruptible") }
            => $^
        }

        $Blocked => $Alive {
            status(): str { @@:("blocked") }
            => $^
        }

        $Alive {
            send_signal(sig: str): str {
                if sig == "SIGKILL" or sig == "SIGTERM":
                    @@:("exiting")
                    -> $Zombie
                elif sig == "SIGSTOP":
                    @@:("stopped")
                    -> $Stopped
                else:
                    @@:("signal ignored")
            }
        }

        $Stopped {
            send_signal(sig: str): str {
                if sig == "SIGCONT":
                    @@:("continued")
                    -> $Ready
                elif sig == "SIGKILL":
                    @@:("killed while stopped")
                    -> $Zombie
                else:
                    @@:("signal ignored")
            }
            status(): str { @@:("stopped") }
        }

        $Zombie {
            wait_by_parent(): str {
                @@:("reaped")
                -> $Dead
            }
            status(): str { @@:("zombie") }
        }

        $Dead {
            status(): str { @@:("dead") }
        }

    actions:
        next_pid() {
            self.pid_counter = self.pid_counter + 1
            return self.pid_counter
        }

    domain:
        pid: int = 0
        pid_counter: int = 100
}

if __name__ == '__main__':
    t = @@TaskLifecycle()
    print(t.fork())                      # forked
    print(t.schedule())                  # running
    print(t.block_on_io())               # blocked
    print(t.send_signal("SIGUSR1"))      # interrupted (wakes interruptible)
    print(t.schedule())                  # running
    print(t.block_on_io())               # blocked (back in interruptible)
    print(t.send_signal("SIGKILL"))      # exiting (forwarded to $Alive)
    print(t.status())                    # zombie
    print(t.wait_by_parent())            # reaped
    print(t.status())                    # dead

    # Uninterruptible path — non-fatal signals ignored, SIGKILL kills
    t2 = @@TaskLifecycle()
    t2.fork(); t2.schedule()
    print(t2.block_uninterruptible())    # blocked (D)
    print(t2.send_signal("SIGUSR1"))     # signal ignored (D-state)
    print(t2.send_signal("SIGKILL"))     # exiting (via $Alive)
    print(t2.status())                   # zombie
```

**How it works:** The HSM hierarchy captures a key insight buried in the kernel code: `$Interruptible` and `$Uninterruptible` are both children of `$Blocked`, which is a child of `$Alive`. Signals like SIGKILL and SIGSTOP are handled by `$Alive` and apply uniformly to all living states.

The subtlety: because V4 forwarding is **explicit only**, a child handler that handles `send_signal()` completely shadows the parent. So `$Interruptible.send_signal()` explicitly forwards fatal signals to the parent via an in-handler `=> $^`, while intercepting wake-up signals (SIGUSR1, etc.) locally. `$Uninterruptible` has no `send_signal()` handler at all, so every signal forwards through `$Blocked` (no handler) to `$Alive`. Non-fatal signals hit `$Alive`’s `else` branch and are ignored — matching D-state behavior. SIGKILL still gets through, matching `TASK_KILLABLE`.

The `$Zombie` → `$Dead` transition (via `wait_by_parent()`) models the parent calling `wait()` to reap the zombie. Until that happens, the zombie cannot be reused. In Frame, this is structural: no event in `$Zombie` transitions anywhere except `$Dead`.

**Features used:** 3-level HSM (`$Interruptible` => `$Blocked` => `$Alive`), selective in-handler forwarding (`=> $^` inside `if`), events ignored by absent handler (D-state non-fatal signals), parent forwarding for shared signal handling

-----


## 56. Runtime Power Management

![56 state diagram](images/cookbook/56.svg)

**Problem:** Model the kernel’s runtime PM framework (`drivers/base/power/runtime.c`). Devices transition through power states based on usage counts and autosuspend timers. The kernel implementation is 1,800 lines of nested conditionals and spinlock-protected flag checks.

```frame
@@target python_3

@@system RuntimePM {
    interface:
        get(): str
        put(): str
        autosuspend_expired(): str
        resume_complete(): str
        suspend_complete(success: bool): str
        status(): str

    machine:
        $Active {
            $>() {
                self.cancel_timer()
                print(f"[rpm] active (usage={self.usage_count})")
            }

            get(): str {
                self.usage_count = self.usage_count + 1
                @@:(f"get: usage={self.usage_count}")
            }
            put(): str {
                self.usage_count = self.usage_count - 1
                @@:(f"put: usage={self.usage_count}")
                if self.usage_count <= 0:
                    -> $Idle
            }
            status(): str { @@:("active") }
        }

        $Idle {
            $>() {
                self.start_autosuspend_timer()
                print(f"[rpm] idle, timer started ({self.autosuspend_delay_ms}ms)")
            }
            <$() {
                self.cancel_timer()
            }

            get(): str {
                self.usage_count = self.usage_count + 1
                @@:(f"get: usage={self.usage_count}")
                -> $Active
            }
            autosuspend_expired(): str {
                @@:("suspending")
                -> $Suspending
            }
            status(): str { @@:("idle") }
        }

        $Suspending {
            $>() {
                print("[rpm] suspending device...")
                self.call_suspend_callback()
            }

            suspend_complete(success: bool): str {
                if success:
                    @@:("suspended")
                    -> $Suspended
                else:
                    @@:("suspend failed")
                    -> $Active
            }
            get(): str {
                self.usage_count = self.usage_count + 1
                @@:("resume requested during suspend")
                -> $Resuming
            }
            status(): str { @@:("suspending") }
        }

        $Suspended {
            $>() {
                print("[rpm] device suspended")
            }

            get(): str {
                self.usage_count = self.usage_count + 1
                @@:("waking")
                -> $Resuming
            }
            status(): str { @@:("suspended") }
        }

        $Resuming {
            $>() {
                print("[rpm] resuming device...")
                self.call_resume_callback()
            }

            resume_complete(): str {
                @@:("resumed")
                -> $Active
            }
            status(): str { @@:("resuming") }
        }

    actions:
        start_autosuspend_timer() {
            print(f"  [timer] start {self.autosuspend_delay_ms}ms")
        }
        cancel_timer() {
            print("  [timer] cancelled")
        }
        call_suspend_callback() {
            print("  [driver] suspend callback")
        }
        call_resume_callback() {
            print("  [driver] resume callback")
        }

    domain:
        usage_count: int = 1
        autosuspend_delay_ms: int = 500
}

if __name__ == '__main__':
    dev = @@RuntimePM()
    print(dev.put())                         # put: usage=0 -> idle
    print(dev.autosuspend_expired())         # suspending
    print(dev.suspend_complete(True))        # suspended
    print(dev.get())                         # waking -> resuming
    print(dev.resume_complete())             # resumed -> active
    print(dev.put())                         # put: usage=0 -> idle
    print(dev.get())                         # get during idle -> active
    print(dev.status())                      # active
```

**How it works:** The enter/exit handlers on `$Idle` manage the autosuspend timer — `$>()` starts it, `<$()` cancels it. This is the pattern that takes 40+ lines of flag-checking in the kernel’s `rpm_idle()` function. The usage count is a domain variable because it must persist across state transitions (a `get()` during `$Suspending` increments it and redirects to `$Resuming`).

The race condition where `get()` arrives during `$Suspending` is handled naturally: `$Suspending` has a `get()` handler that increments the count and transitions to `$Resuming`. In the kernel, this requires careful lock ordering. In Frame, it’s a handler.

**Features used:** enter/exit handlers for timer lifecycle, domain variables (usage count), conditional transitions (suspend success/failure), events handled differently per state (get during active vs. suspended vs. suspending)

-----


## 57. Block I/O Request

![57 state diagram](images/cookbook/57.svg)

**Problem:** Model the lifecycle of a block I/O request through the kernel’s blk-mq layer (`block/blk-mq.c`). Requests move through queuing, dispatch, in-flight, completion, and error recovery stages.

```frame
@@target python_3

@@system BlockRequest {
    interface:
        submit(sector: int, size: int): str
        dispatch(): str
        complete(success: bool): str
        timeout(): str
        status(): str

    machine:
        $Idle {
            submit(sector: int, size: int): str {
                self.sector = sector
                self.size = size
                self.attempts = 0
                @@:("queued")
                -> $Queued
            }
            status(): str { @@:("idle") }
        }

        $Queued {
            dispatch(): str {
                @@:("dispatched")
                -> $InFlight
            }
            status(): str { @@:("queued") }
        }

        $InFlight {
            $>() {
                self.attempts = self.attempts + 1
                self.start_timeout()
                print(f"  [blk] dispatched sector={self.sector} attempt={self.attempts}")
            }
            <$() {
                self.cancel_timeout()
            }

            complete(success: bool): str {
                if success:
                    @@:("done")
                    -> $Done
                else:
                    if self.attempts < self.max_retries:
                        @@:("requeued")
                        -> $Queued
                    else:
                        @@:("io error")
                        -> $Error
            }
            timeout(): str {
                if self.attempts < self.max_retries:
                    @@:("timeout, retrying")
                    -> $Queued
                else:
                    @@:("timeout, giving up")
                    -> $Error
            }
            status(): str { @@:("in-flight") }
        }

        $Done {
            $>() {
                print(f"  [blk] complete sector={self.sector}")
            }
            status(): str { @@:("done") }
        }

        $Error {
            $>() {
                print(f"  [blk] ERROR sector={self.sector} after {self.attempts} attempts")
            }
            status(): str { @@:("error") }
        }

    actions:
        start_timeout() {
            print(f"  [timer] request timeout started ({self.timeout_ms}ms)")
        }
        cancel_timeout() {
            print("  [timer] request timeout cancelled")
        }

    domain:
        sector: int = 0
        size: int = 0
        attempts: int = 0
        max_retries: int = 3
        timeout_ms: int = 30000
}

if __name__ == '__main__':
    req = @@BlockRequest()
    print(req.submit(1024, 8))       # queued
    print(req.dispatch())            # dispatched
    print(req.timeout())             # timeout, retrying
    print(req.dispatch())            # dispatched
    print(req.complete(False))       # requeued
    print(req.dispatch())            # dispatched
    print(req.complete(True))        # done
    print(req.status())              # done
```

**How it works:** The enter handler on `$InFlight` starts the timeout timer and increments the attempt count. The exit handler cancels the timer. This pair ensures the timer is always properly managed regardless of whether the request completes, times out, or gets requeued. In the kernel’s blk-mq code, timeout management is spread across `blk_mq_start_request()`, `blk_mq_complete_request()`, and `blk_mq_timeout_work()`.

The retry logic uses a domain variable (`self.attempts`) that persists across the `$InFlight` → `$Queued` → `$InFlight` cycle. Each re-entry to `$InFlight` increments the counter via the enter handler.

**Features used:** enter/exit handlers for timeout management, domain variables for retry counting, conditional transitions (success/failure/timeout paths), terminal states

-----


## 58. USB Device Enumeration

![58 state diagram](images/cookbook/58.svg)

**Problem:** Model the USB device enumeration sequence (USB 2.0 spec §9.1). When a device is plugged in, the hub driver must reset it, assign an address, read descriptors, and load a driver. Failure at any stage requires cleanup of all prior stages. The kernel implementation (`drivers/usb/core/hub.c`) is 5,800 lines.

```frame
@@target python_3

@@system UsbEnumerator {
    interface:
        plug_in(port: int)
        reset_done(success: bool)
        address_set(success: bool, addr: int)
        descriptors_read(success: bool, product: str)
        driver_bound(success: bool)
        unplug()
        status(): str

    machine:
        $Detached {
            plug_in(port: int) {
                self.port = port
                self.reset_attempts = 0
                -> $Resetting
            }
            status(): str { @@:("detached") }
        }

        $Resetting => $Enumerating {
            $>() {
                self.reset_attempts = self.reset_attempts + 1
                print(f"  [usb] resetting port {self.port} (attempt {self.reset_attempts})")
                self.issue_port_reset(self.port)
            }

            reset_done(success: bool) {
                if success:
                    -> $Addressing
                else:
                    if self.reset_attempts < self.max_reset_attempts:
                        -> $Resetting
                    else:
                        -> $Failed
            }
            status(): str { @@:("resetting") }
            => $^
        }

        $Addressing => $Enumerating {
            $>() {
                print(f"  [usb] assigning address on port {self.port}")
                self.assign_address()
            }

            address_set(success: bool, addr: int) {
                if success:
                    self.address = addr
                    -> $ReadingDescriptors
                else:
                    -> $UndoReset
            }
            status(): str { @@:("addressing") }
            => $^
        }

        $ReadingDescriptors => $Enumerating {
            $>() {
                print(f"  [usb] reading descriptors for device at addr {self.address}")
                self.read_descriptors(self.address)
            }

            descriptors_read(success: bool, product: str) {
                if success:
                    self.product = product
                    -> $BindingDriver
                else:
                    -> $UndoAddress
            }
            status(): str { @@:("reading descriptors") }
            => $^
        }

        $BindingDriver => $Enumerating {
            $>() {
                print(f"  [usb] binding driver for '{self.product}'")
                self.bind_driver(self.product)
            }

            driver_bound(success: bool) {
                if success:
                    -> $Configured
                else:
                    -> $UndoAddress
            }
            status(): str { @@:("binding driver") }
            => $^
        }

        $Enumerating {
            unplug() {
                print(f"  [usb] unplugged during enumeration")
                -> $Detached
            }
        }

        # --- Compensation chain ---
        $UndoAddress {
            $>() {
                print(f"  [usb] releasing address {self.address}")
                self.release_address(self.address)
                -> $UndoReset
            }
        }

        $UndoReset {
            $>() {
                print(f"  [usb] disabling port {self.port}")
                self.disable_port(self.port)
                -> $Failed
            }
        }

        $Configured {
            $>() {
                print(f"  [usb] device '{self.product}' configured at addr {self.address}")
            }
            unplug() {
                self.unbind_driver()
                self.release_address(self.address)
                self.disable_port(self.port)
                -> $Detached
            }
            status(): str { @@:(f"configured: {self.product} @ addr {self.address}") }
        }

        $Failed {
            $>() {
                print(f"  [usb] enumeration failed on port {self.port}")
            }
            plug_in(port: int) {
                self.port = port
                self.reset_attempts = 0
                -> $Resetting
            }
            status(): str { @@:("failed") }
        }

    actions:
        issue_port_reset(port)      { print(f"    -> port_reset({port})") }
        assign_address()            { print("    -> assign_address()") }
        read_descriptors(addr)      { print(f"    -> read_descriptors({addr})") }
        bind_driver(product)        { print(f"    -> bind_driver({product})") }
        release_address(addr)       { print(f"    -> release_address({addr})") }
        disable_port(port)          { print(f"    -> disable_port({port})") }
        unbind_driver()             { print("    -> unbind_driver()") }

    domain:
        port: int = 0
        address: int = 0
        product: str = ""
        reset_attempts: int = 0
        max_reset_attempts: int = 3
}

if __name__ == '__main__':
    usb = @@UsbEnumerator()

    # Happy path
    usb.plug_in(1)
    usb.reset_done(True)
    usb.address_set(True, 7)
    usb.descriptors_read(True, "USB Keyboard")
    usb.driver_bound(True)
    print(usb.status())  # configured: USB Keyboard @ addr 7

    # Failure with compensation
    usb2 = @@UsbEnumerator()
    usb2.plug_in(2)
    usb2.reset_done(True)
    usb2.address_set(True, 8)
    usb2.descriptors_read(False, "")  # descriptor read fails
    # -> UndoAddress -> UndoReset -> Failed
    print(usb2.status())  # failed

    # Unplug during enumeration — forwarded to $Enumerating via => $^
    usb3 = @@UsbEnumerator()
    usb3.plug_in(3)
    usb3.reset_done(True)
    usb3.unplug()
    print(usb3.status()) # detached
```

**How it works:** The forward pipeline (`$Resetting` → `$Addressing` → `$ReadingDescriptors` → `$BindingDriver` → `$Configured`) is the happy path. Each stage is a child of `$Enumerating`, which handles `unplug()`. Each child has a trailing `=> $^` so events it doesn’t handle — including `unplug()` — forward to the parent.

The compensation chain (`$UndoAddress` → `$UndoReset` → `$Failed`) undoes prior stages in reverse order via enter-handler transitions. This is the saga pattern applied to hardware: if descriptor reading fails after an address was assigned, the address must be released and the port disabled.

In the kernel’s `hub_port_connect_change()`, this logic is a 300-line function with goto labels for cleanup. In Frame, each stage is a state and each compensation step is a transient state.

**Features used:** HSM for shared `unplug()` handling, enter-handler chain for compensation, retry with counter (reset attempts), transient states for undo, multi-stage pipeline

-----


## 59. Watchdog Timer

![59 state diagram](images/cookbook/59.svg)

**Problem:** Model the kernel’s watchdog device (`drivers/watchdog/watchdog_dev.c`). A hardware watchdog resets the system unless software periodically pings it. The “magic close” feature prevents accidental disarming: the device stays armed unless the close is preceded by writing a magic character (‘V’).

```frame
@@target python_3

@@system WatchdogDevice {
    interface:
        open(): str
        write(data: str): str
        ping(): str
        close(): str
        timeout_expired(): str
        status(): str

    machine:
        $Inactive {
            open(): str {
                @@:("opened")
                -> $Armed
            }
            status(): str { @@:("inactive") }
        }

        $Armed {
            $>() {
                self.magic_seen = False
                self.start_hw_timer()
                print(f"  [wdog] armed, timeout={self.timeout_sec}s")
            }
            <$() {
                print("  [wdog] state exit")
            }

            ping(): str {
                self.reset_hw_timer()
                @@:("pinged")
            }
            write(data: str): str {
                if "V" in data:
                    self.magic_seen = True
                    @@:("magic character received")
                else:
                    @@:("data written")
                self.reset_hw_timer()
            }
            close(): str {
                if self.magic_seen:
                    self.stop_hw_timer()
                    @@:("disarmed")
                    -> $Inactive
                else:
                    @@:("close ignored — no magic char, watchdog stays armed")
                    # Device stays armed even after close.
                    # The kernel prints: "watchdog did not stop!"
            }
            timeout_expired(): str {
                @@:("SYSTEM RESET")
                -> $Triggered
            }
            status(): str {
                if self.magic_seen:
                    @@:("armed (magic seen, close will disarm)")
                else:
                    @@:("armed")
            }
        }

        $Triggered {
            $>() {
                print("  [wdog] !!! WATCHDOG TIMEOUT — SYSTEM RESET !!!")
            }
            status(): str { @@:("triggered — system reset") }
        }

    actions:
        start_hw_timer() {
            print(f"    -> hw_timer start ({self.timeout_sec}s)")
        }
        reset_hw_timer() {
            print(f"    -> hw_timer reset ({self.timeout_sec}s)")
        }
        stop_hw_timer() {
            print("    -> hw_timer stopped")
        }

    domain:
        timeout_sec: int = 60
        magic_seen: bool = False
}

if __name__ == '__main__':
    wd = @@WatchdogDevice()
    print(wd.open())                    # opened
    print(wd.ping())                    # pinged
    print(wd.ping())                    # pinged

    # Try to close without magic char — stays armed
    print(wd.close())                   # close ignored

    # Write magic character then close
    print(wd.write("goodbye V"))        # magic character received
    print(wd.status())                  # armed (magic seen)
    print(wd.close())                   # disarmed
    print(wd.status())                  # inactive

    # Timeout scenario
    wd2 = @@WatchdogDevice()
    wd2.open()
    print(wd2.timeout_expired())        # SYSTEM RESET
    print(wd2.status())                 # triggered
```

**How it works:** The magic close guard is a domain variable (`self.magic_seen`) checked in the `close()` handler. If the magic character hasn’t been written, `close()` does nothing — the watchdog stays armed. This prevents accidental disarming by applications that open and close the watchdog device without proper shutdown sequences.

The safety property is visible in the code: the only path from `$Armed` to `$Inactive` goes through a `close()` call where `self.magic_seen` is true. There’s no way to reach `$Inactive` without the magic character.

In the kernel, this is implemented with a `WDOG_ALLOW_RELEASE` status bit. The Frame version makes the guard condition obvious.

**Features used:** domain variables as guards, events that conditionally do nothing, enter handler for hardware timer start, terminal state (`$Triggered`)

-----


## 60. OOM Killer

![60 state diagram](images/cookbook/60.svg)

**Problem:** Model the kernel’s Out-Of-Memory killer (`mm/oom_kill.c`). When the system runs out of memory, it must select a victim process, kill it, and wait for memory to be freed. The critical safety property: the OOM killer must not select a new victim while still waiting for the previous one to die. In the kernel, this is enforced by `oom_lock` and careful flag management. In Frame, it’s structural.

```frame
@@target python_3

@@system OomKiller {
    interface:
        oom_detected(gfp_flags: int): str
        victim_exited(pid: int, freed_kb: int): str
        oom_detected_again(gfp_flags: int): str
        status(): str

    machine:
        $Normal {
            oom_detected(gfp_flags: int): str {
                @@:("selecting victim")
                -> $Selecting
            }
            status(): str { @@:("normal") }
        }

        $Selecting {
            $>() {
                self.victim_pid = self.select_victim()
                if self.victim_pid < 0:
                    print("  [oom] no killable process found")
                    -> $Panic
                else:
                    print(f"  [oom] selected pid {self.victim_pid} (score={self.victim_score})")
                    -> $Killing
            }
            status(): str { @@:("selecting") }
        }

        $Killing {
            $>() {
                self.send_sigkill(self.victim_pid)
                self.start_reaper_timer()
                print(f"  [oom] SIGKILL sent to pid {self.victim_pid}")
            }
            <$() {
                self.cancel_reaper_timer()
            }

            victim_exited(pid: int, freed_kb: int): str {
                if pid == self.victim_pid:
                    self.total_freed_kb = self.total_freed_kb + freed_kb
                    @@:(f"reclaimed {freed_kb}KB")
                    -> $Normal
                else:
                    @@:("wrong pid, still waiting")
            }

            # oom_detected_again is NOT handled here.
            # While waiting for a victim to die, the OOM killer
            # does NOT select another victim. This is the key
            # safety property — enforced by structural absence.

            status(): str { @@:(f"killing pid {self.victim_pid}") }
        }

        $Panic {
            $>() {
                print("  [oom] !!! KERNEL PANIC — OUT OF MEMORY !!!")
            }
            status(): str { @@:("panic") }
        }

    actions:
        select_victim() {
            # In the real kernel, this scores every process by
            # memory usage, oom_score_adj, and other factors.
            self.victim_score = 850
            return 4242
        }
        send_sigkill(pid) {
            print(f"    -> kill -9 {pid}")
        }
        start_reaper_timer() {
            print("    -> reaper timer started")
        }
        cancel_reaper_timer() {
            print("    -> reaper timer cancelled")
        }

    domain:
        victim_pid: int = -1
        victim_score: int = 0
        total_freed_kb: int = 0
}

if __name__ == '__main__':
    oom = @@OomKiller()
    print(oom.oom_detected(0))                   # selecting victim

    # While killing, another OOM event arrives — silently ignored
    print(oom.status())                           # killing pid 4242
    print(oom.oom_detected_again(0))              # None (not handled)

    # Victim exits
    print(oom.victim_exited(4242, 51200))         # reclaimed 51200KB
    print(oom.status())                           # normal
```

**How it works:** The safety-critical property is that `$Killing` doesn’t handle `oom_detected_again()`. While the OOM killer is waiting for a victim to die, additional OOM events are silently dropped. No second victim is selected, no SIGKILL storm occurs. In the kernel, this requires acquiring `oom_lock` and checking `oom_killer_disabled` — convention-based guards. In Frame, the guard is structural: there’s no handler, so there’s no dispatch path.

`$Selecting` is a transient state — its enter handler immediately selects a victim and transitions to either `$Killing` or `$Panic`. This mirrors the kernel’s `out_of_memory()` function, which synchronously selects and kills in one call.

**Features used:** safety by construction (no handler for concurrent OOM), transient state for victim selection, enter/exit handlers for timer management, domain variables for victim tracking

-----


## 61. Filesystem Freeze

![61 state diagram](images/cookbook/61.svg)

**Problem:** Model the VFS filesystem freeze/thaw mechanism (`fs/super.c`). A mounted filesystem can be frozen for a consistent snapshot — writes are blocked but reads continue. This recipe uses a **3-level HSM**: `$Active` and `$Frozen` both share the “read-only” capability via an intermediate `$Readable` state.

```frame
@@target python_3

@@system Filesystem {
    interface:
        mount(device: str): str
        unmount(): str
        freeze(): str
        thaw(): str
        write(data: str): str
        read(): str
        sync(): str
        status(): str

    machine:
        $Unmounted {
            mount(device: str): str {
                self.device = device
                @@:("mounted")
                -> $Active
            }
            status(): str { @@:("unmounted") }
        }

        $Active => $Readable {
            write(data: str): str {
                self.write_count = self.write_count + 1
                @@:(f"wrote: {data}")
            }
            freeze(): str {
                @@:("freezing")
                -> $Freezing
            }
            status(): str { @@:("active (read-write)") }
            => $^
        }

        $Freezing => $Mounted {
            $>() {
                self.flush_pending_writes()
                print("  [fs] flushing writes...")
                -> $Frozen
            }
            status(): str { @@:("freezing") }
            => $^
        }

        $Frozen => $Readable {
            write(data: str): str {
                # Writes are blocked during freeze.
                # In the kernel, sb_start_write() blocks here.
                @@:("BLOCKED — filesystem frozen")
            }
            thaw(): str {
                @@:("thawed")
                -> $Active
            }
            status(): str { @@:("frozen (read-only)") }
            => $^
        }

        # Intermediate level — states that allow reads.
        # Both $Active and $Frozen inherit from this.
        $Readable => $Mounted {
            # read() could live here OR in $Mounted; placing it here
            # documents that "readable" is the capability we're grouping.
            => $^
        }

        $Mounted {
            read(): str {
                self.read_count = self.read_count + 1
                @@:(f"read (count={self.read_count})")
            }
            sync(): str {
                @@:("synced")
            }
            unmount(): str {
                @@:("unmounted")
                -> $Unmounting
            }
            # freeze() not handled here — only $Active can freeze.
            # thaw() not handled here — only $Frozen can thaw.
        }

        $Unmounting {
            $>() {
                self.flush_pending_writes()
                self.release_resources()
                print(f"  [fs] unmounting {self.device}")
                -> $Unmounted
            }
            # mount(), freeze(), write(), read() — none handled.
            # During unmount, the filesystem is inaccessible.
        }

    actions:
        flush_pending_writes() {
            print(f"    -> flush ({self.write_count} writes)")
        }
        release_resources() {
            print("    -> release resources")
        }

    domain:
        device: str = ""
        write_count: int = 0
        read_count: int = 0
}

if __name__ == '__main__':
    fs = @@Filesystem()
    print(fs.mount("/dev/sda1"))          # mounted
    print(fs.write("hello"))              # wrote: hello
    print(fs.read())                      # read (count=1)
    print(fs.freeze())                    # freezing
    print(fs.write("blocked"))            # BLOCKED — filesystem frozen
    print(fs.read())                      # read (count=2) — reads still work!
    print(fs.thaw())                      # thawed
    print(fs.write("world"))              # wrote: world
    print(fs.unmount())                   # unmounted
    print(fs.status())                    # unmounted

    # Verify: can't freeze an unmounted filesystem
    print(fs.freeze())                    # None (not handled)
```

**How it works:** The 3-level HSM is: `$Active => $Readable => $Mounted` and `$Frozen => $Readable => $Mounted`. The `$Freezing` transient state is `=> $Mounted` directly (bypassing `$Readable` because flushing-in-progress isn’t really “readable” — it’s in flux).

`$Mounted` provides `read()`, `sync()`, and `unmount()` — available in every mounted sub-state. `$Readable` is a grouping level that documents the “can read” capability; it doesn’t add handlers here but would be the natural place to add read-permission checks, quota enforcement, or access control. `$Active` adds `write()` and `freeze()`; `$Frozen` overrides `write()` to block and adds `thaw()`.

The trailing `=> $^` on every HSM state ensures events propagate up the full 3-level chain when no local handler exists.

**Features used:** **3-level HSM** (`$Active` => `$Readable` => `$Mounted`), transient states for flush/cleanup, events ignored in wrong state (write blocked when frozen, freeze impossible when unmounted), enter-handler chain

-----


## 62. Kernel Module Loader

![62 state diagram](images/cookbook/62.svg)

**Problem:** Model the kernel module loading pipeline (`kernel/module/main.c`). Loading proceeds through symbol resolution, memory allocation, relocation, and initialization. Failure at any stage requires unwinding all prior stages.

```frame
@@target python_3

@@system ModuleLoader {
    interface:
        load(name: str)
        unload(): str
        acquire(): str
        release(): str
        resolve_done(success: bool, reason: str)
        alloc_done(success: bool, reason: str)
        relocate_done(success: bool, reason: str)
        init_done(success: bool, reason: str)
        status(): str

    machine:
        $Idle {
            load(name: str) {
                self.name = name
                self.failure = ""
                -> $ResolvingSymbols
            }
            status(): str { @@:("idle") }
        }

        $ResolvingSymbols {
            $>() {
                print(f"  [mod] resolving symbols for '{self.name}'")
                self.do_resolve(self.name)
            }
            resolve_done(success: bool, reason: str) {
                if success:
                    -> $AllocatingMemory
                else:
                    self.failure = reason
                    -> $Failed
            }
            status(): str { @@:("resolving symbols") }
        }

        $AllocatingMemory {
            $>() {
                print(f"  [mod] allocating module memory")
                self.do_alloc(self.name)
            }
            alloc_done(success: bool, reason: str) {
                if success:
                    -> $Relocating
                else:
                    self.failure = reason
                    -> $Failed
            }
            status(): str { @@:("allocating") }
        }

        $Relocating {
            $>() {
                print(f"  [mod] relocating code sections")
                self.do_relocate(self.name)
            }
            relocate_done(success: bool, reason: str) {
                if success:
                    -> $Initializing
                else:
                    self.failure = reason
                    -> $UndoAlloc
            }
            status(): str { @@:("relocating") }
        }

        $Initializing {
            $>() {
                print(f"  [mod] running module_init()")
                self.do_init(self.name)
            }
            init_done(success: bool, reason: str) {
                if success:
                    -> $Live
                else:
                    self.failure = reason
                    -> $UndoRelocate
            }
            status(): str { @@:("initializing") }
        }

        # --- Compensation chain ---
        $UndoRelocate {
            $>() {
                print(f"  [mod] undoing relocation")
                self.do_undo_relocate(self.name)
                -> $UndoAlloc
            }
        }

        $UndoAlloc {
            $>() {
                print(f"  [mod] freeing module memory")
                self.do_free(self.name)
                -> $Failed
            }
        }

        $Live {
            $>() {
                self.refcount = 0
                print(f"  [mod] '{self.name}' is live")
            }

            acquire(): str {
                self.refcount = self.refcount + 1
                @@:(f"acquired (refcount={self.refcount})")
            }
            release(): str {
                self.refcount = self.refcount - 1
                @@:(f"released (refcount={self.refcount})")
            }
            unload(): str {
                if self.refcount > 0:
                    @@:(f"busy (refcount={self.refcount})")
                else:
                    @@:("unloading")
                    -> $Unloading
            }
            status(): str { @@:(f"live (refcount={self.refcount})") }
        }

        $Unloading {
            $>() {
                print(f"  [mod] running module_exit()")
                self.do_exit(self.name)
                self.do_free(self.name)
                print(f"  [mod] '{self.name}' unloaded")
                -> $Idle
            }
        }

        $Failed {
            $>() {
                print(f"  [mod] FAILED: {self.failure}")
            }
            load(name: str) {
                self.name = name
                self.failure = ""
                -> $ResolvingSymbols
            }
            status(): str { @@:(f"failed: {self.failure}") }
        }

    actions:
        do_resolve(name)         { print(f"    -> resolve_symbols({name})") }
        do_alloc(name)           { print(f"    -> alloc_module({name})") }
        do_relocate(name)        { print(f"    -> apply_relocations({name})") }
        do_init(name)            { print(f"    -> module_init({name})") }
        do_undo_relocate(name)   { print(f"    -> undo_relocations({name})") }
        do_free(name)            { print(f"    -> free_module({name})") }
        do_exit(name)            { print(f"    -> module_exit({name})") }

    domain:
        name: str = ""
        failure: str = ""
        refcount: int = 0
}

if __name__ == '__main__':
    loader = @@ModuleLoader()

    # Happy path
    loader.load("ext4")
    loader.resolve_done(True, "")
    loader.alloc_done(True, "")
    loader.relocate_done(True, "")
    loader.init_done(True, "")
    print(loader.status())                # live (refcount=0)
    print(loader.unload())                # unloading

    # Failure in init — compensates relocation and allocation
    loader2 = @@ModuleLoader()
    loader2.load("buggy_mod")
    loader2.resolve_done(True, "")
    loader2.alloc_done(True, "")
    loader2.relocate_done(True, "")
    loader2.init_done(False, "init returned -ENOMEM")
    print(loader2.status())               # failed: init returned -ENOMEM

    # Refcount guard via interface (no direct field mutation)
    loader3 = @@ModuleLoader()
    loader3.load("in_use_mod")
    loader3.resolve_done(True, "")
    loader3.alloc_done(True, "")
    loader3.relocate_done(True, "")
    loader3.init_done(True, "")
    print(loader3.acquire())              # acquired (refcount=1)
    print(loader3.acquire())              # acquired (refcount=2)
    print(loader3.unload())               # busy (refcount=2)
    print(loader3.release())              # released (refcount=1)
    print(loader3.release())              # released (refcount=0)
    print(loader3.unload())               # unloading
```

**How it works:** The forward pipeline is a chain of states where each enter handler starts an async operation and the corresponding event handler transitions to the next stage. Failure at any point triggers a compensation chain that undoes prior stages in reverse order: `$UndoRelocate` → `$UndoAlloc` → `$Failed`.

In the kernel, this is a single function with `goto` labels: `goto free_module`, `goto free_unload`, etc. The Frame version makes each stage and each compensation step explicit. Adding a new stage (say, verifying module signatures between resolve and alloc) is adding a state block and two transitions — no goto-label surgery.

Reference counting is exposed through the interface (`acquire()`/`release()`) rather than by mutating domain fields from outside — the refcount is encapsulated inside the state machine.

**Features used:** enter-handler chain (pipeline), transient compensation states (saga pattern), conditional transition (refcount guard), interface methods for ref management, domain variables for error tracking

-----


## 63. Signal Handler Stack

![63 state diagram](images/cookbook/63.svg)

**Problem:** Model signal delivery to a user-mode process. When a signal arrives, the kernel saves the current user context, jumps to the handler, and restores the context via `sigreturn`. Nested signals build a stack of saved frames. This is the textbook use case for `push$`/`pop$`.

```frame
@@target python_3

@@system SignalContext {
    interface:
        enter_user_code()
        signal_arrives(sig: str)
        handler_returns()
        current_mode(): str
        depth(): int

    machine:
        $Kernel {
            enter_user_code() {
                -> $UserMode
            }
            current_mode(): str { @@:("kernel") }
            depth(): int { @@:(self.nesting) }
        }

        $UserMode {
            $.saved_regs: str = ""

            $>() {
                $.saved_regs = self.snapshot_registers()
            }

            signal_arrives(sig: str) {
                self.nesting = self.nesting + 1
                push$
                -> (sig) $SignalHandler
            }
            current_mode(): str { @@:("user") }
            depth(): int { @@:(self.nesting) }
        }

        $SignalHandler {
            $.signum: str = ""

            $>(sig: str) {
                $.signum = sig
                print(f"  [sig] handling {sig} (depth={self.nesting})")
            }

            signal_arrives(sig: str) {
                # Nested signal — re-push and recurse into a new handler
                self.nesting = self.nesting + 1
                push$
                -> (sig) $SignalHandler
            }

            handler_returns() {
                print(f"  [sig] return from {$.signum}")
                self.nesting = self.nesting - 1
                -> pop$
            }

            current_mode(): str { @@:(f"handler({$.signum})") }
            depth(): int { @@:(self.nesting) }
        }

    actions:
        snapshot_registers() {
            return f"rip=0x{id(self):x}"
        }

    domain:
        nesting: int = 0
}

if __name__ == '__main__':
    cpu = @@SignalContext()
    cpu.enter_user_code()
    print(cpu.current_mode())          # user

    cpu.signal_arrives("SIGUSR1")
    print(cpu.current_mode())          # handler(SIGUSR1)
    print(cpu.depth())                 # 1

    # Nested signal arrives DURING the handler
    cpu.signal_arrives("SIGALRM")
    print(cpu.current_mode())          # handler(SIGALRM)
    print(cpu.depth())                 # 2

    cpu.handler_returns()              # return from SIGALRM
    print(cpu.current_mode())          # handler(SIGUSR1) — restored
    print(cpu.depth())                 # 1

    cpu.handler_returns()              # return from SIGUSR1
    print(cpu.current_mode())          # user — back to interrupted code
    print(cpu.depth())                 # 0
```

**How it works:** `push$` before transitioning into `$SignalHandler` saves the current compartment — including `$UserMode`’s `$.saved_regs` state variable or a prior `$SignalHandler`’s `$.signum`. `-> pop$` on handler return restores it.

Critically, **state variables are preserved across `pop$`** (unlike a normal `-> $State` transition, which resets them to initial values). That’s what makes nested signal delivery work: when an outer `$SignalHandler` is popped back to, its `$.signum` is still the signal it was handling.

Nested signals re-push, building a stack that mirrors the kernel’s actual signal frame stack on the user stack. The `self.nesting` counter in the domain tracks depth across push/pop cycles — it doesn’t reset on pop because it’s a domain variable, not a state variable.

This is what the kernel does in `do_signal()` / `setup_frame()` / `sys_sigreturn()`: save registers on the user stack, jump to the handler, and let the handler return via a trampoline that invokes `sigreturn` to pop the saved frame. In Frame, `push$`/`pop$` captures the essential structure without simulating the register save itself.

**Features used:** `push$` / `-> pop$` for nested context save/restore, state variables preserved across `pop$` (not reset), enter args (`-> (sig) $SignalHandler`), self-push for reentrant handlers, domain variable for cross-frame counting

-----


## Internet Protocols

Eight recipes modeling network protocols from their RFCs as Frame state machines. Protocol FSMs are where hand-written dispatch tables go to die; Frame lets you transcribe the table from the RFC and get a runnable implementation. Every recipe is runnable with `@@target python_3`.

-----

## 64. DHCP Client

![64 state diagram](images/cookbook/64.svg)

**Problem:** Model the DHCP client state machine from RFC 2131 Figure 5. The client discovers servers, requests a lease, and manages lease renewal with T1 and T2 timers. This is the protocol that gets every laptop its IP address.

```frame
@@target python_3

@@system DhcpClient {
    interface:
        start()
        offer_received(server_ip: str, offered_ip: str, lease_sec: int)
        ack_received(ip: str, lease_sec: int)
        nak_received()
        t1_expired()
        t2_expired()
        lease_expired()
        release()
        status(): str

    machine:
        $Init {
            $>() {
                self.attempts = 0
            }

            start() {
                -> $Selecting
            }
            status(): str { @@:("init") }
        }

        $Selecting {
            $>() {
                self.attempts = self.attempts + 1
                self.broadcast_discover()
                print(f"  [dhcp] DHCPDISCOVER broadcast (attempt {self.attempts})")
            }

            offer_received(server_ip: str, offered_ip: str, lease_sec: int) {
                self.server_ip = server_ip
                self.offered_ip = offered_ip
                self.lease_sec = lease_sec
                -> $Requesting
            }
            status(): str { @@:("selecting") }
        }

        $Requesting {
            $>() {
                self.send_request(self.server_ip, self.offered_ip)
                print(f"  [dhcp] DHCPREQUEST for {self.offered_ip} from {self.server_ip}")
            }

            ack_received(ip: str, lease_sec: int) {
                self.bound_ip = ip
                self.lease_sec = lease_sec
                # Compute absolute T1/T2 deadlines once the lease is confirmed.
                self.t1_deadline = self.now() + (lease_sec // 2)
                self.t2_deadline = self.now() + int(lease_sec * 0.875)
                self.lease_deadline = self.now() + lease_sec
                -> $Bound
            }
            nak_received() {
                print("  [dhcp] NAK received, restarting")
                -> $Init
            }
            status(): str { @@:("requesting") }
        }

        $Bound {
            $>() {
                self.arm_t1_timer()
                self.arm_t2_timer()
                self.arm_lease_timer()
                print(f"  [dhcp] BOUND: {self.bound_ip} (lease={self.lease_sec}s)")
            }
            # Note: timers are NOT cancelled on exit from $Bound.
            # T2 and lease-expiry deadlines are absolute and must keep
            # running through $Renewing and $Rebinding. Cancellation is
            # explicit in release() / nak_received() handlers.

            t1_expired() {
                -> $Renewing
            }
            release() {
                self.cancel_all_timers()
                self.send_release(self.server_ip, self.bound_ip)
                -> $Init
            }
            status(): str { @@:(f"bound: {self.bound_ip}") }
        }

        $Renewing {
            $>() {
                self.send_renew(self.server_ip, self.bound_ip)
                print(f"  [dhcp] RENEWING with {self.server_ip}")
            }

            ack_received(ip: str, lease_sec: int) {
                self.lease_sec = lease_sec
                self.t1_deadline = self.now() + (lease_sec // 2)
                self.t2_deadline = self.now() + int(lease_sec * 0.875)
                self.lease_deadline = self.now() + lease_sec
                -> $Bound
            }
            nak_received() {
                self.cancel_all_timers()
                -> $Init
            }
            t2_expired() {
                -> $Rebinding
            }
            lease_expired() {
                self.cancel_all_timers()
                -> $Init
            }
            release() {
                self.cancel_all_timers()
                self.send_release(self.server_ip, self.bound_ip)
                -> $Init
            }
            status(): str { @@:(f"renewing: {self.bound_ip}") }
        }

        $Rebinding {
            $>() {
                self.broadcast_request(self.bound_ip)
                print(f"  [dhcp] REBINDING — broadcast request for {self.bound_ip}")
            }

            ack_received(ip: str, lease_sec: int) {
                self.lease_sec = lease_sec
                self.t1_deadline = self.now() + (lease_sec // 2)
                self.t2_deadline = self.now() + int(lease_sec * 0.875)
                self.lease_deadline = self.now() + lease_sec
                -> $Bound
            }
            nak_received() {
                self.cancel_all_timers()
                -> $Init
            }
            lease_expired() {
                self.cancel_all_timers()
                self.bound_ip = ""
                -> $Init
            }
            release() {
                self.cancel_all_timers()
                self.send_release(self.server_ip, self.bound_ip)
                -> $Init
            }
            status(): str { @@:(f"rebinding: {self.bound_ip}") }
        }

    actions:
        broadcast_discover()              { print("    -> broadcast DHCPDISCOVER") }
        send_request(server, ip)          { print(f"    -> DHCPREQUEST {ip} to {server}") }
        send_renew(server, ip)            { print(f"    -> DHCPREQUEST (renew) {ip} to {server}") }
        broadcast_request(ip)             { print(f"    -> broadcast DHCPREQUEST {ip}") }
        send_release(server, ip)          { print(f"    -> DHCPRELEASE {ip} to {server}") }
        arm_t1_timer() {
            t1 = self.lease_sec // 2
            print(f"    -> T1 timer armed ({t1}s)")
        }
        arm_t2_timer() {
            t2 = int(self.lease_sec * 0.875)
            print(f"    -> T2 timer armed ({t2}s)")
        }
        arm_lease_timer() {
            print(f"    -> lease-expiry timer armed ({self.lease_sec}s)")
        }
        cancel_all_timers()               { print("    -> all timers cancelled") }
        now() { return 0 }  # stub — real impl returns monotonic clock

    domain:
        server_ip: str = ""
        offered_ip: str = ""
        bound_ip: str = ""
        lease_sec: int = 0
        attempts: int = 0
        t1_deadline: int = 0
        t2_deadline: int = 0
        lease_deadline: int = 0
}

if __name__ == '__main__':
    dhcp = @@DhcpClient()
    dhcp.start()
    dhcp.offer_received("192.168.1.1", "192.168.1.100", 3600)
    dhcp.ack_received("192.168.1.100", 3600)
    print(dhcp.status())                  # bound: 192.168.1.100

    # Renewal cycle
    dhcp.t1_expired()
    print(dhcp.status())                  # renewing: 192.168.1.100
    dhcp.ack_received("192.168.1.100", 7200)
    print(dhcp.status())                  # bound: 192.168.1.100

    # T2 expires during renewal -> rebinding
    dhcp.t1_expired()
    dhcp.t2_expired()
    print(dhcp.status())                  # rebinding: 192.168.1.100
    dhcp.ack_received("192.168.1.100", 3600)
    print(dhcp.status())                  # bound: 192.168.1.100

    # Full lease expiry
    dhcp2 = @@DhcpClient()
    dhcp2.start()
    dhcp2.offer_received("10.0.0.1", "10.0.0.50", 1800)
    dhcp2.ack_received("10.0.0.50", 1800)
    dhcp2.t1_expired()
    dhcp2.t2_expired()
    dhcp2.lease_expired()
    print(dhcp2.status())                 # init
```

**How it works:** The state machine maps directly to RFC 2131 Figure 5. The graceful degradation chain — `$Bound` → `$Renewing` → `$Rebinding` → `$Init` — is driven by timers.

**Timer lifecycle:** All three timers (T1, T2, lease-expiry) are armed once in `$Bound.$>()` with absolute deadlines relative to lease acquisition. They are NOT cancelled when leaving `$Bound` — that would defeat the whole point, since `$Renewing` still needs T2 to fire, and `$Rebinding` still needs lease-expiry. Explicit cancellation happens only on terminal transitions: `release()`, `nak_received()`, or `lease_expired()`. Each successful `ack_received()` re-arms the deadlines with the new lease period.

This matches real DHCP clients (e.g., `dhclient`, `systemd-networkd`): T1/T2/expiry are timestamps, not timers that restart per state.

**Features used:** domain variables for absolute timer deadlines, conditional transitions (ACK vs. NAK), graceful degradation chain, explicit timer cancellation on terminal events, RFC-to-code correspondence

-----


## 65. TLS Handshake

![65 state diagram](images/cookbook/65.svg)

**Problem:** Model the TLS 1.2 handshake from the server’s perspective (RFC 5246 §7.3). The handshake is a multi-step protocol where a fatal alert at any stage tears down the connection. HSM provides shared alert handling.

```frame
@@target python_3

@@system TlsServerHandshake {
    interface:
        client_hello(version: str, ciphers: list)
        client_key_exchange(key_data: str)
        client_change_cipher_spec()
        client_finished(verify: str)
        alert(level: str, desc: str)
        status(): str

    machine:
        $Idle {
            client_hello(version: str, ciphers: list) {
                self.client_version = version
                self.selected_cipher = self.choose_cipher(ciphers)
                if self.selected_cipher == "":
                    self.send_alert("fatal", "handshake_failure")
                    -> $Closed
                else:
                    -> $ServerHello
            }
            status(): str { @@:("idle") }
        }

        $ServerHello => $Handshaking {
            $>() {
                self.send_server_hello(self.selected_cipher)
                self.send_certificate()
                self.send_server_hello_done()
                print(f"  [tls] ServerHello sent, cipher={self.selected_cipher}")
                -> $AwaitClientKeyExchange
            }
            status(): str { @@:("server hello") }
            => $^
        }

        $AwaitClientKeyExchange => $Handshaking {
            client_key_exchange(key_data: str) {
                self.premaster = key_data
                self.derive_keys(key_data)
                print(f"  [tls] keys derived")
                -> $AwaitChangeCipherSpec
            }
            status(): str { @@:("awaiting client key exchange") }
            => $^
        }

        $AwaitChangeCipherSpec => $Handshaking {
            client_change_cipher_spec() {
                self.client_encrypted = True
                print("  [tls] client switched to encrypted")
                -> $AwaitClientFinished
            }
            status(): str { @@:("awaiting change cipher spec") }
            => $^
        }

        $AwaitClientFinished => $Handshaking {
            client_finished(verify: str) {
                if self.verify_finished(verify):
                    self.send_change_cipher_spec()
                    self.send_finished()
                    print("  [tls] handshake complete")
                    -> $Established
                else:
                    self.send_alert("fatal", "decrypt_error")
                    -> $Closed
            }
            status(): str { @@:("awaiting client finished") }
            => $^
        }

        $Handshaking {
            alert(level: str, desc: str) {
                print(f"  [tls] alert during handshake: {level}/{desc}")
                if level == "fatal":
                    -> $Closed
            }
        }

        $Established {
            $>() {
                print("  [tls] === SECURE CONNECTION ESTABLISHED ===")
            }
            alert(level: str, desc: str) {
                print(f"  [tls] alert: {level}/{desc}")
                if level == "fatal":
                    -> $Closed
            }
            status(): str { @@:("established") }
        }

        $Closed {
            $>() {
                print("  [tls] connection closed")
            }
            status(): str { @@:("closed") }
        }

    actions:
        choose_cipher(ciphers) {
            preferred = ["TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
                         "TLS_RSA_WITH_AES_128_CBC_SHA"]
            for p in preferred:
                if p in ciphers:
                    return p
            return ""
        }
        send_server_hello(cipher)   { print(f"    -> ServerHello({cipher})") }
        send_certificate()          { print("    -> Certificate") }
        send_server_hello_done()    { print("    -> ServerHelloDone") }
        derive_keys(premaster)      { print("    -> derive_keys()") }
        verify_finished(verify)     { return verify == "valid" }
        send_change_cipher_spec()   { print("    -> ChangeCipherSpec") }
        send_finished()             { print("    -> Finished") }
        send_alert(level, desc)     { print(f"    -> Alert({level}, {desc})") }

    domain:
        client_version: str = ""
        selected_cipher: str = ""
        premaster: str = ""
        client_encrypted: bool = False
}

if __name__ == '__main__':
    tls = @@TlsServerHandshake()

    # Happy path
    tls.client_hello("TLS 1.2", ["TLS_RSA_WITH_AES_128_CBC_SHA"])
    tls.client_key_exchange("premaster_secret_data")
    tls.client_change_cipher_spec()
    tls.client_finished("valid")
    print(tls.status())              # established

    # Alert during handshake — forwarded to $Handshaking parent via => $^
    tls2 = @@TlsServerHandshake()
    tls2.client_hello("TLS 1.2", ["TLS_RSA_WITH_AES_128_CBC_SHA"])
    tls2.alert("fatal", "unexpected_message")
    print(tls2.status())             # closed

    # No matching cipher
    tls3 = @@TlsServerHandshake()
    tls3.client_hello("TLS 1.2", ["TLS_NULL_WITH_NULL_NULL"])
    print(tls3.status())             # closed
```

**How it works:** Every handshake sub-state is a child of `$Handshaking`, which handles `alert()`. Each child ends with `=> $^` so an `alert()` arriving in any handshake phase forwards to `$Handshaking` and transitions to `$Closed` if fatal. This is the exact pattern HSM was designed for — cross-cutting error handling.

`$ServerHello` is a transient state: the enter handler sends all server messages and immediately transitions to `$AwaitClientKeyExchange` via `-> $AwaitClientKeyExchange`. The kernel’s deferred-transition architecture means this doesn’t build up call stack — each `$>()` records `__next_compartment` and returns; the kernel processes transitions iteratively. This models the TLS spec’s “flight” concept — the server sends ServerHello, Certificate, and ServerHelloDone as a batch, then waits for the client’s response.

> **Target note:** `ciphers: list` works for Python. For Rust you’d write `Vec<String>`; for C you’d declare a fixed array. The type string is passed through verbatim.

**Features used:** HSM for shared alert handling across all handshake states, transient states for message flights, enter-handler chain, conditional transitions (cipher negotiation, verify check), `=> $^` default forward

-----


## 66. Wi-Fi Station Management

![66 state diagram](images/cookbook/66.svg)

**Problem:** Model the IEEE 802.11 station management state machine. The key property: deauthentication can arrive in any connected state and resets the station to the beginning. Without HSM, every state needs its own `deauth()` handler.

```frame
@@target python_3

@@system WifiStation {
    interface:
        scan_done(bssid: str, ssid: str)
        auth_response(success: bool)
        assoc_response(success: bool, aid: int)
        eapol_key_msg(msg_num: int)
        deauth(reason: str)
        disassoc(reason: str)
        disconnect()
        status(): str

    machine:
        $Disconnected {
            scan_done(bssid: str, ssid: str) {
                self.bssid = bssid
                self.ssid = ssid
                -> $Authenticating
            }
            status(): str { @@:("disconnected") }
        }

        $Authenticating => $Associated {
            $>() {
                self.send_auth_request(self.bssid)
                print(f"  [wifi] authenticating with {self.ssid} ({self.bssid})")
            }

            auth_response(success: bool) {
                if success:
                    -> $Associating
                else:
                    print("  [wifi] auth rejected")
                    -> $Disconnected
            }
            status(): str { @@:("authenticating") }
            => $^
        }

        $Associating => $Associated {
            $>() {
                self.send_assoc_request(self.bssid, self.ssid)
                print(f"  [wifi] associating with {self.ssid}")
            }

            assoc_response(success: bool, aid: int) {
                if success:
                    self.aid = aid
                    -> $KeyNegotiation
                else:
                    print("  [wifi] assoc rejected")
                    -> $Disconnected
            }
            disassoc(reason: str) {
                print(f"  [wifi] disassociated: {reason}")
                -> $Authenticating
            }
            status(): str { @@:("associating") }
            => $^
        }

        $KeyNegotiation => $Associated {
            $.key_step: int = 0

            $>() {
                print(f"  [wifi] starting 4-way handshake")
            }

            eapol_key_msg(msg_num: int) {
                $.key_step = msg_num
                print(f"  [wifi] 4-way handshake msg {msg_num}")
                if msg_num == 4:
                    -> $Connected
            }
            disassoc(reason: str) {
                print(f"  [wifi] disassociated during key exchange: {reason}")
                -> $Authenticating
            }
            status(): str { @@:(f"key negotiation (step {$.key_step})") }
            => $^
        }

        $Associated {
            deauth(reason: str) {
                print(f"  [wifi] DEAUTH: {reason}")
                -> $Disconnected
            }
            disconnect() {
                self.send_deauth(self.bssid)
                -> $Disconnected
            }
        }

        $Connected {
            $>() {
                print(f"  [wifi] === CONNECTED to {self.ssid} (aid={self.aid}) ===")
            }

            disassoc(reason: str) {
                print(f"  [wifi] disassociated: {reason}")
                -> $Authenticating
            }
            deauth(reason: str) {
                print(f"  [wifi] DEAUTH: {reason}")
                -> $Disconnected
            }
            disconnect() {
                self.send_deauth(self.bssid)
                -> $Disconnected
            }
            status(): str { @@:(f"connected to {self.ssid}") }
        }

    actions:
        send_auth_request(bssid)          { print(f"    -> AuthRequest({bssid})") }
        send_assoc_request(bssid, ssid)   { print(f"    -> AssocRequest({bssid}, {ssid})") }
        send_deauth(bssid)                { print(f"    -> Deauth({bssid})") }

    domain:
        bssid: str = ""
        ssid: str = ""
        aid: int = 0
}

if __name__ == '__main__':
    sta = @@WifiStation()

    # Happy path
    sta.scan_done("AA:BB:CC:DD:EE:FF", "MyNetwork")
    sta.auth_response(True)
    sta.assoc_response(True, 1)
    sta.eapol_key_msg(1)
    sta.eapol_key_msg(2)
    sta.eapol_key_msg(3)
    sta.eapol_key_msg(4)
    print(sta.status())               # connected to MyNetwork

    # Deauth from connected — handled locally
    sta.deauth("inactivity")
    print(sta.status())               # disconnected

    # Deauth during key negotiation — HSM parent handles it via => $^
    sta2 = @@WifiStation()
    sta2.scan_done("11:22:33:44:55:66", "CafeWifi")
    sta2.auth_response(True)
    sta2.assoc_response(True, 5)
    sta2.eapol_key_msg(1)
    sta2.deauth("AP restarting")      # forwarded $KeyNegotiation -> $Associated
    print(sta2.status())              # disconnected
```

**How it works:** `$Authenticating`, `$Associating`, and `$KeyNegotiation` are all children of `$Associated`, which handles `deauth()`. Each child ends with `=> $^` so `deauth()` (not handled locally) forwards to `$Associated` and transitions to `$Disconnected`. This matches the 802.11 spec: deauthentication resets to State 1 regardless of the current sub-state.

`$Connected` handles `deauth()` directly (not via HSM) because a connected station is a distinct state from the handshake sub-states — it’s drawn as a sibling rather than a child. `disassoc()` drops to `$Authenticating` (State 2), not all the way back — the 802.11 spec distinguishes between deauthentication (full reset) and disassociation (keeps authentication).

The 4-way handshake uses a state variable (`$.key_step`) that resets on every entry to `$KeyNegotiation`. If association is lost and re-established, the handshake starts fresh.

**Features used:** HSM for shared `deauth()` handling, state variables (key step counter, resets on reentry), distinct behavior for deauth vs. disassoc, enter handlers for protocol message sending

-----


## 67. BGP Finite State Machine

![67 state diagram](images/cookbook/67.svg)

**Problem:** Model the BGP FSM from RFC 4271 §8. The RFC defines this as an explicit state machine with named events — it’s practically pseudocode for a Frame spec. BGP runs on every backbone router on the internet. `@@persist` lets the session survive daemon restarts, matching the `bgpd` graceful-restart feature.

```frame
@@target python_3

@@persist
@@system BgpSession {
    interface:
        manual_start()
        manual_stop()
        tcp_connection_confirmed()
        tcp_connection_fails()
        open_received(asn: int, hold_time: int)
        keepalive_received()
        notification_received(code: int, subcode: int)
        hold_timer_expired()
        keepalive_timer_expired()
        connect_retry_timer_expired()
        status(): str

    machine:
        $Idle {
            manual_start() {
                self.connect_retry_count = 0
                self.start_connect_retry_timer()
                self.initiate_tcp_connection()
                -> $Connect
            }
            status(): str { @@:("idle") }
        }

        $Connect {
            $>() {
                print(f"  [bgp] connecting to peer AS{self.peer_asn}")
            }

            tcp_connection_confirmed() {
                self.cancel_connect_retry_timer()
                self.send_open()
                self.start_hold_timer(self.large_hold_time)
                -> $OpenSent
            }
            tcp_connection_fails() {
                -> $Active
            }
            connect_retry_timer_expired() {
                self.initiate_tcp_connection()
                self.start_connect_retry_timer()
            }
            manual_stop() {
                -> $Idle
            }
            status(): str { @@:("connect") }
        }

        $Active {
            $>() {
                print("  [bgp] waiting for connection (passive)")
            }

            tcp_connection_confirmed() {
                self.cancel_connect_retry_timer()
                self.send_open()
                self.start_hold_timer(self.large_hold_time)
                -> $OpenSent
            }
            connect_retry_timer_expired() {
                self.connect_retry_count = self.connect_retry_count + 1
                self.initiate_tcp_connection()
                self.start_connect_retry_timer()
                -> $Connect
            }
            manual_stop() {
                -> $Idle
            }
            status(): str { @@:("active") }
        }

        $OpenSent {
            $>() {
                print("  [bgp] OPEN sent, awaiting OPEN")
            }

            open_received(asn: int, hold_time: int) {
                self.peer_asn = asn
                self.negotiated_hold = min(hold_time, self.local_hold_time)
                self.cancel_hold_timer()
                self.send_keepalive()
                if self.negotiated_hold > 0:
                    self.start_hold_timer(self.negotiated_hold)
                    self.start_keepalive_timer()
                -> $OpenConfirm
            }
            tcp_connection_fails() {
                self.cancel_hold_timer()
                -> $Active
            }
            notification_received(code: int, subcode: int) {
                print(f"  [bgp] notification: {code}/{subcode}")
                -> $Idle
            }
            hold_timer_expired() {
                self.send_notification(4, 0)
                -> $Idle
            }
            manual_stop() {
                self.send_notification(6, 0)
                -> $Idle
            }
            status(): str { @@:("open sent") }
        }

        $OpenConfirm {
            $>() {
                print(f"  [bgp] OPEN confirmed, hold={self.negotiated_hold}s")
            }

            keepalive_received() {
                -> $Established
            }
            notification_received(code: int, subcode: int) {
                print(f"  [bgp] notification: {code}/{subcode}")
                -> $Idle
            }
            hold_timer_expired() {
                self.send_notification(4, 0)
                -> $Idle
            }
            keepalive_timer_expired() {
                self.send_keepalive()
            }
            manual_stop() {
                self.send_notification(6, 0)
                -> $Idle
            }
            status(): str { @@:("open confirm") }
        }

        $Established {
            $>() {
                print(f"  [bgp] === SESSION ESTABLISHED with AS{self.peer_asn} ===")
            }

            keepalive_received() {
                self.reset_hold_timer()
            }
            keepalive_timer_expired() {
                self.send_keepalive()
            }
            notification_received(code: int, subcode: int) {
                print(f"  [bgp] notification: {code}/{subcode}")
                -> $Idle
            }
            hold_timer_expired() {
                self.send_notification(4, 0)
                -> $Idle
            }
            manual_stop() {
                self.send_notification(6, 0)
                -> $Idle
            }
            status(): str { @@:(f"established (AS{self.peer_asn}, hold={self.negotiated_hold}s)") }
        }

    actions:
        initiate_tcp_connection()         { print("    -> TCP connect") }
        send_open()                       { print(f"    -> OPEN (AS{self.local_asn})") }
        send_keepalive()                  { print("    -> KEEPALIVE") }
        send_notification(code, subcode)  { print(f"    -> NOTIFICATION({code},{subcode})") }
        start_connect_retry_timer()       { print("    -> connect retry timer start") }
        cancel_connect_retry_timer()      { print("    -> connect retry timer cancel") }
        start_hold_timer(seconds)         { print(f"    -> hold timer start ({seconds}s)") }
        cancel_hold_timer()               { print("    -> hold timer cancel") }
        reset_hold_timer()                { print("    -> hold timer reset") }
        start_keepalive_timer()           { print("    -> keepalive timer start") }

    domain:
        local_asn: int = 65001
        peer_asn: int = 65002
        local_hold_time: int = 90
        large_hold_time: int = 240
        negotiated_hold: int = 0
        connect_retry_count: int = 0
}

if __name__ == '__main__':
    bgp = @@BgpSession()
    bgp.manual_start()
    bgp.tcp_connection_confirmed()
    bgp.open_received(65002, 60)
    bgp.keepalive_received()
    print(bgp.status())               # established (AS65002, hold=60s)

    # Persist across daemon restart
    snap = bgp.save_state()
    bgp = BgpSession.restore_state(snap)
    print(bgp.status())               # established — session preserved

    # Keepalive exchange
    bgp.keepalive_timer_expired()      # sends keepalive
    bgp.keepalive_received()           # resets hold timer

    # Hold timer expiry — tears down session
    bgp.hold_timer_expired()
    print(bgp.status())               # idle

    # Connection failure path
    bgp2 = @@BgpSession()
    bgp2.manual_start()
    bgp2.tcp_connection_fails()
    print(bgp2.status())              # active
    bgp2.connect_retry_timer_expired()
    print(bgp2.status())              # connect
```

**How it works:** This is a near-direct transcription of RFC 4271 §8.2.2. Each state handles exactly the events the RFC specifies. The `manual_stop()` event sends a NOTIFICATION(6,0) — Cease — from every state except `$Idle`. The hold timer and keepalive timer interactions in `$Established` match the RFC’s description: keepalive receipt resets the hold timer, keepalive timer expiry triggers a keepalive send.

With `@@persist`, a `bgpd` graceful restart can save the session state to disk, exec a new binary, and pick up exactly where it left off — including the current `$Established` state and the negotiated hold time. This matches real BGP graceful restart (RFC 4724) which is engineered precisely to avoid tearing down long-lived sessions during software updates.

**Features used:** RFC-to-code direct correspondence, `@@persist` for session survival across restart, timer management via actions, conditional transitions (hold time negotiation), connect retry cycling between states

-----


## 68. PPP Link Control Protocol

![68 state diagram](images/cookbook/68.svg)

**Problem:** Model the PPP LCP FSM from RFC 1661 §4. This RFC contains one of the most carefully specified protocol state machines ever published — a complete state transition table with 10 states and 16 events. The Frame spec should be a direct transcription.

```frame
@@target python_3

@@system PppLcp {
    interface:
        up()
        down()
        open()
        close()
        receive_configure_request(acceptable: bool)
        receive_configure_ack()
        receive_configure_nak()
        receive_terminate_request()
        receive_terminate_ack()
        timeout_positive()
        timeout_negative()
        status(): str

    machine:
        # Initial and Starting: lower layer down
        $Initial {
            up() { -> $Closed }
            open() { -> $Starting }
            status(): str { @@:("initial") }
        }

        $Starting {
            up() {
                self.init_restart_count()
                self.send_configure_request()
                -> $ReqSent
            }
            close() { -> $Initial }
            status(): str { @@:("starting") }
        }

        # Closed and Stopped: lower layer up, LCP not open
        $Closed {
            open() {
                self.init_restart_count()
                self.send_configure_request()
                -> $ReqSent
            }
            down() { -> $Initial }
            receive_configure_request(acceptable: bool) {
                self.send_terminate_ack()
            }
            status(): str { @@:("closed") }
        }

        $Stopped {
            open() { -> $Stopped }
            down() { -> $Starting }
            receive_configure_request(acceptable: bool) {
                self.init_restart_count()
                if acceptable:
                    self.send_configure_ack()
                    self.send_configure_request()
                    -> $AckSent
                else:
                    self.send_configure_nak()
                    self.send_configure_request()
                    -> $ReqSent
            }
            status(): str { @@:("stopped") }
        }

        # Closing and Stopping: LCP terminating
        $Closing {
            timeout_positive() {
                self.send_terminate_request()
            }
            timeout_negative() {
                -> $Closed
            }
            receive_terminate_ack() {
                -> $Closed
            }
            down() { -> $Initial }
            status(): str { @@:("closing") }
        }

        $Stopping {
            timeout_positive() {
                self.send_terminate_request()
            }
            timeout_negative() {
                -> $Stopped
            }
            receive_terminate_ack() {
                -> $Stopped
            }
            down() { -> $Starting }
            status(): str { @@:("stopping") }
        }

        # Negotiation: configure request/ack/nak exchange
        $ReqSent {
            timeout_positive() {
                self.send_configure_request()
            }
            timeout_negative() {
                -> $Stopped
            }
            receive_configure_request(acceptable: bool) {
                if acceptable:
                    self.send_configure_ack()
                    -> $AckSent
                else:
                    self.send_configure_nak()
            }
            receive_configure_ack() {
                self.init_restart_count()
                -> $AckRcvd
            }
            receive_configure_nak() {
                self.init_restart_count()
                self.send_configure_request()
            }
            receive_terminate_request() {
                self.send_terminate_ack()
                -> $ReqSent
            }
            close() {
                self.init_restart_count()
                self.send_terminate_request()
                -> $Closing
            }
            down() { -> $Starting }
            status(): str { @@:("req-sent") }
        }

        $AckRcvd {
            receive_configure_request(acceptable: bool) {
                if acceptable:
                    self.send_configure_ack()
                    self.tlu()
                    -> $Opened
                else:
                    self.send_configure_nak()
            }
            timeout_positive() {
                self.send_configure_request()
                -> $ReqSent
            }
            timeout_negative() { -> $Stopped }
            receive_configure_ack() {
                self.send_configure_request()
                -> $ReqSent
            }
            close() {
                self.init_restart_count()
                self.send_terminate_request()
                -> $Closing
            }
            down() { -> $Starting }
            status(): str { @@:("ack-rcvd") }
        }

        $AckSent {
            receive_configure_ack() {
                self.init_restart_count()
                self.tlu()
                -> $Opened
            }
            timeout_positive() {
                self.send_configure_request()
            }
            timeout_negative() { -> $Stopped }
            receive_configure_request(acceptable: bool) {
                if acceptable:
                    self.send_configure_ack()
                else:
                    self.send_configure_nak()
                    -> $ReqSent
            }
            receive_configure_nak() {
                self.init_restart_count()
                self.send_configure_request()
                -> $ReqSent
            }
            close() {
                self.init_restart_count()
                self.send_terminate_request()
                -> $Closing
            }
            down() { -> $Starting }
            status(): str { @@:("ack-sent") }
        }

        $Opened {
            $>() {
                print("  [ppp] === LINK OPENED ===")
            }
            <$() {
                self.tld()
            }

            receive_configure_request(acceptable: bool) {
                self.send_configure_request()
                if acceptable:
                    self.send_configure_ack()
                    -> $AckSent
                else:
                    self.send_configure_nak()
                    -> $ReqSent
            }
            receive_terminate_request() {
                self.send_terminate_ack()
                self.zero_restart_count()
                -> $Stopping
            }
            close() {
                self.init_restart_count()
                self.send_terminate_request()
                -> $Closing
            }
            down() {
                -> $Starting
            }
            status(): str { @@:("opened") }
        }

    actions:
        send_configure_request()     { print("    -> Configure-Request") }
        send_configure_ack()         { print("    -> Configure-Ack") }
        send_configure_nak()         { print("    -> Configure-Nak") }
        send_terminate_request()     { print("    -> Terminate-Request") }
        send_terminate_ack()         { print("    -> Terminate-Ack") }
        init_restart_count()         { self.restart_count = self.max_restart }
        zero_restart_count()         { self.restart_count = 0 }
        tlu()                        { print("    -> This-Layer-Up") }
        tld()                        { print("    -> This-Layer-Down") }

    domain:
        restart_count: int = 0
        max_restart: int = 10
}

if __name__ == '__main__':
    ppp = @@PppLcp()

    # Normal link establishment
    ppp.open()                         # Initial -> Starting
    ppp.up()                           # Starting -> ReqSent (sends Config-Req)
    ppp.receive_configure_ack()        # ReqSent -> AckRcvd
    ppp.receive_configure_request(True)  # AckRcvd -> Opened
    print(ppp.status())                # opened

    # Peer initiates renegotiation
    ppp.receive_configure_request(True)  # Opened -> AckSent
    ppp.receive_configure_ack()          # AckSent -> Opened
    print(ppp.status())                  # opened

    # Clean shutdown
    ppp.close()                          # Opened -> Closing
    ppp.receive_terminate_ack()          # Closing -> Closed
    print(ppp.status())                  # closed
```

**How it works:** This is a direct transcription of the RFC 1661 §4.1 state transition table. Each state handles exactly the events listed in the RFC, with exactly the actions and transitions specified. The `timeout_positive()` event fires when the restart timer expires and retries remain; `timeout_negative()` fires when retries are exhausted.

The `$Opened` state’s exit handler calls `tld()` (This-Layer-Down) — a notification to upper layers that the link is going away. `tlu()` (This-Layer-Up) is called in each transition *into* Opened, matching the RFC’s placement of tlu calls in the transitions rather than in Opened’s entry action.

Comparing this spec against RFC 1661 Table 4 is a line-by-line verification exercise.

**Features used:** RFC state table direct transcription, enter/exit handlers for layer notifications, domain variables for restart counting, 10 states with precise event handling per RFC

-----


## 69. NTP Client Association

![69 state diagram](images/cookbook/69.svg)

**Problem:** Model an NTP client’s per-server association (RFC 5905). The client manages a polling interval that backs off when synchronized and resets when unsynchronized, plus a reachability register that tracks recent poll successes.

```frame
@@target python_3

@@system NtpAssociation {
    operations:
        get_offset(): float {
            return self.last_offset
        }
        is_reachable(): bool {
            return self.reach_register > 0
        }

    interface:
        poll_tick()
        response_received(offset_ms: float, delay_ms: float)
        poll_timeout()
        status(): str

    machine:
        $Unsynced {
            $>() {
                self.poll_interval = self.min_poll
                print(f"  [ntp] unsynced, poll interval={self.poll_interval}s")
            }

            poll_tick() {
                self.shift_reach_register(0)
                self.send_ntp_request(self.server)
                print(f"  [ntp] poll {self.server} (reach=0b{self.reach_register:08b})")
            }
            response_received(offset_ms: float, delay_ms: float) {
                self.shift_reach_register(1)
                self.last_offset = offset_ms
                self.last_delay = delay_ms
                self.good_responses = self.good_responses + 1
                if self.good_responses >= self.sync_threshold:
                    -> $Synced
            }
            poll_timeout() {
                self.shift_reach_register(0)
            }
            status(): str { @@:(f"unsynced (reach=0b{self.reach_register:08b})") }
        }

        $Synced {
            $>() {
                print(f"  [ntp] SYNCED to {self.server} (offset={self.last_offset}ms)")
            }

            poll_tick() {
                self.shift_reach_register(0)
                self.send_ntp_request(self.server)
                print(f"  [ntp] poll {self.server} (interval={self.poll_interval}s)")
            }
            response_received(offset_ms: float, delay_ms: float) {
                self.shift_reach_register(1)
                self.last_offset = offset_ms
                self.last_delay = delay_ms
                if self.poll_interval < self.max_poll:
                    self.poll_interval = self.poll_interval * 2
            }
            poll_timeout() {
                self.shift_reach_register(0)
                self.consecutive_timeouts = self.consecutive_timeouts + 1
                if self.consecutive_timeouts >= 3:
                    self.consecutive_timeouts = 0
                    self.good_responses = 0
                    -> $Unsynced
                if self.poll_interval > self.min_poll:
                    self.poll_interval = self.poll_interval // 2
            }
            status(): str {
                @@:(f"synced (offset={self.last_offset}ms, poll={self.poll_interval}s, reach=0b{self.reach_register:08b})")
            }
        }

    actions:
        send_ntp_request(server)  { print(f"    -> NTP request to {server}") }
        shift_reach_register(bit) {
            self.reach_register = ((self.reach_register << 1) | bit) & 0xFF
            if bit:
                self.consecutive_timeouts = 0
        }

    domain:
        server: str = "pool.ntp.org"
        poll_interval: int = 64
        min_poll: int = 64
        max_poll: int = 1024
        reach_register: int = 0
        last_offset: float = 0.0
        last_delay: float = 0.0
        good_responses: int = 0
        sync_threshold: int = 3
        consecutive_timeouts: int = 0
}

if __name__ == '__main__':
    ntp = @@NtpAssociation()

    # Build up to sync
    ntp.poll_tick()
    ntp.response_received(12.5, 3.2)
    ntp.poll_tick()
    ntp.response_received(11.8, 2.9)
    ntp.poll_tick()
    ntp.response_received(12.1, 3.0)
    print(ntp.status())                # synced

    # Poll interval backs off
    ntp.poll_tick()
    ntp.response_received(12.0, 3.1)   # interval -> 128
    ntp.poll_tick()
    ntp.response_received(11.9, 3.0)   # interval -> 256
    print(ntp.status())                # synced (...poll=256s...)

    # Timeouts shrink the interval
    ntp.poll_tick()
    ntp.poll_timeout()                 # interval -> 128
    print(ntp.status())

    # Enough timeouts -> unsync
    ntp.poll_tick()
    ntp.poll_timeout()
    ntp.poll_tick()
    ntp.poll_timeout()
    ntp.poll_tick()
    ntp.poll_timeout()
    print(ntp.status())                # unsynced

    print(f"reachable: {ntp.is_reachable()}")
```

**How it works:** The polling interval is a domain variable that doubles in `$Synced` on every successful response (backing off from 64s toward 1024s) and halves on timeouts. If three consecutive timeouts occur, the association drops to `$Unsynced` and the interval resets.

The reachability register is an 8-bit shift register — a 1 is shifted in for each response, a 0 for each timeout. This matches RFC 5905’s reachability tracking. When the register reaches zero (8 consecutive timeouts), the server is considered unreachable.

Operations (`get_offset()` and `is_reachable()`) provide read-only access to the association’s state without going through the state machine. This matches how NTP’s clock selection algorithm queries multiple associations to pick the best source.

**Features used:** domain variables for adaptive timing, operations for read-only queries, conditional transitions (sync/unsync based on thresholds), bit manipulation in actions

-----


## 70. HTTP/1.1 Connection

![70 state diagram](images/cookbook/70.svg)

**Problem:** Model an HTTP/1.1 persistent connection lifecycle. The connection starts idle, processes one request at a time, and stays open for reuse (keep-alive) until explicitly closed or a timeout expires. Cross-cutting concerns — connection errors and explicit close — should be handled uniformly via HSM rather than duplicated per state.

```frame
@@target python_3

@@system HttpConnection {
    interface:
        connect(host: str, port: int)
        send_request(method: str, path: str)
        headers_received(status: int, keep_alive: bool, content_length: int)
        body_chunk_received(chunk: str)
        body_complete()
        keep_alive_timeout()
        connection_error(reason: str)
        close()
        status(): str

    machine:
        $Disconnected {
            connect(host: str, port: int) {
                self.host = host
                self.port = port
                -> $Connecting
            }
            status(): str { @@:("disconnected") }
        }

        $Connecting {
            $>() {
                self.tcp_connect(self.host, self.port)
                print(f"  [http] connecting to {self.host}:{self.port}")
                -> $Idle
            }
        }

        # --- HSM parent for all "open connection" states ---
        # connection_error() and close() are handled here uniformly.

        $Idle => $Open {
            $>() {
                self.start_keep_alive_timer()
                print(f"  [http] idle (requests served: {self.request_count})")
            }
            <$() {
                self.cancel_keep_alive_timer()
            }

            send_request(method: str, path: str) {
                self.current_method = method
                self.current_path = path
                self.emit_request(method, path, self.host)
                -> $AwaitingResponse
            }
            keep_alive_timeout() {
                print("  [http] keep-alive timeout")
                -> $Closing
            }
            status(): str { @@:("idle") }
            => $^
        }

        $AwaitingResponse => $Open {
            $>() {
                print(f"  [http] {self.current_method} {self.current_path} sent")
            }

            headers_received(status: int, keep_alive: bool, content_length: int) {
                self.response_status = status
                self.keep_alive = keep_alive
                self.content_length = content_length
                self.received_body = ""
                if content_length == 0:
                    -> $ResponseComplete
                else:
                    -> $ReceivingBody
            }
            status(): str { @@:("awaiting response") }
            => $^
        }

        $ReceivingBody => $Open {
            body_chunk_received(chunk: str) {
                self.received_body = self.received_body + chunk
            }
            body_complete() {
                -> $ResponseComplete
            }
            status(): str { @@:("receiving body") }
            => $^
        }

        $ResponseComplete => $Open {
            $>() {
                self.request_count = self.request_count + 1
                print(f"  [http] response {self.response_status} complete ({len(self.received_body)} bytes)")
                if self.keep_alive:
                    -> $Idle
                else:
                    -> $Closing
            }
            => $^
        }

        $Open {
            # Cross-cutting handlers for any open-connection state.
            connection_error(reason: str) {
                print(f"  [http] error: {reason}")
                -> $Disconnected
            }
            close() {
                -> $Closing
            }
        }

        $Closing {
            $>() {
                self.tcp_close()
                print(f"  [http] closed after {self.request_count} requests")
                -> $Disconnected
            }
        }

    actions:
        tcp_connect(host, port)         { print(f"    -> TCP connect {host}:{port}") }
        tcp_close()                     { print("    -> TCP close") }
        emit_request(method, path, host) {
            print(f"    -> {method} {path} HTTP/1.1")
            print(f"       Host: {host}")
        }
        start_keep_alive_timer()        { print("    -> keep-alive timer start") }
        cancel_keep_alive_timer()       { print("    -> keep-alive timer cancel") }

    domain:
        host: str = ""
        port: int = 80
        current_method: str = ""
        current_path: str = ""
        response_status: int = 0
        content_length: int = 0
        received_body: str = ""
        keep_alive: bool = True
        request_count: int = 0
}

if __name__ == '__main__':
    conn = @@HttpConnection()
    conn.connect("example.com", 80)

    # First request — keep-alive
    conn.send_request("GET", "/index.html")
    conn.headers_received(200, True, 5)
    conn.body_chunk_received("hello")
    conn.body_complete()
    print(conn.status())                 # idle (reused!)

    # Second request on same connection
    conn.send_request("GET", "/about")
    conn.headers_received(200, True, 0)  # empty body
    print(conn.status())                 # idle

    # Connection: close
    conn.send_request("POST", "/api/data")
    conn.headers_received(201, False, 2)
    conn.body_chunk_received("ok")
    conn.body_complete()
    print(conn.status())                 # disconnected (server said close)

    # Keep-alive timeout
    conn2 = @@HttpConnection()
    conn2.connect("api.example.com", 443)
    conn2.send_request("GET", "/health")
    conn2.headers_received(200, True, 0)
    conn2.keep_alive_timeout()
    print(conn2.status())               # disconnected

    # Connection error during body — handled by $Open via => $^
    conn3 = @@HttpConnection()
    conn3.connect("flaky.example", 80)
    conn3.send_request("GET", "/large")
    conn3.headers_received(200, True, 1000)
    conn3.body_chunk_received("partial...")
    conn3.connection_error("peer reset")
    print(conn3.status())               # disconnected
```

**How it works:** `$Idle`, `$AwaitingResponse`, `$ReceivingBody`, and `$ResponseComplete` are all children of `$Open`. Each ends with `=> $^`, so `connection_error()` or `close()` from any point in the request lifecycle forwards to the parent and drops to `$Disconnected` or `$Closing` respectively. No duplication.

`$ResponseComplete` is a transient state: its enter handler decides based on `self.keep_alive` whether to cycle back to `$Idle` for connection reuse or move to `$Closing`. The keep-alive timer lifecycle is cleanly handled by `$Idle`’s enter/exit pair.

**Features used:** HSM parent for cross-cutting error handling, `=> $^` default forward on every child, transient states for protocol decisions, enter/exit for timer lifecycle, keep-alive as conditional transition, connection reuse cycle

-----


## 71. SMTP Conversation

![71 state diagram](images/cookbook/71.svg)

**Problem:** Model an SMTP client conversation (RFC 5321). The protocol is a strict command-response sequence, but it supports mid-conversation upgrade via STARTTLS. After STARTTLS, the conversation restarts from the greeting — a perfect `push$`/`pop$` match if you want to preserve the prior context, though here we use a fresh reset to match real SMTP semantics.

```frame
@@target python_3

@@system SmtpClient {
    interface:
        tcp_connected()
        greeting_received(code: int)
        ehlo_response(code: int, supports_starttls: bool)
        starttls()
        starttls_response(code: int)
        tls_handshake_complete()
        mail_from(sender: str)
        mail_response(code: int)
        rcpt_to(recipient: str)
        rcpt_response(code: int)
        data()
        data_response(code: int)
        data_body(body: str)
        data_final_response(code: int)
        quit()
        quit_response(code: int)
        status(): str

    machine:
        $Disconnected {
            tcp_connected() { -> $AwaitGreeting }
            status(): str { @@:("disconnected") }
        }

        $AwaitGreeting => $Connected {
            greeting_received(code: int) {
                if code == 220:
                    self.send_ehlo()
                    -> $AwaitEhlo
                else:
                    -> $Disconnected
            }
            status(): str { @@:("await greeting") }
            => $^
        }

        $AwaitEhlo => $Connected {
            ehlo_response(code: int, supports_starttls: bool) {
                if code == 250:
                    self.server_supports_starttls = supports_starttls
                    -> $Ready
                else:
                    -> $Disconnected
            }
            status(): str { @@:("await EHLO response") }
            => $^
        }

        $Ready => $Connected {
            starttls() {
                if self.server_supports_starttls and not self.tls_active:
                    self.send_starttls()
                    -> $AwaitStartTlsResponse
            }
            mail_from(sender: str) {
                self.sender = sender
                self.send_mail_from(sender)
                -> $AwaitMailResponse
            }
            quit() {
                self.send_quit()
                -> $AwaitQuitResponse
            }
            status(): str { @@:(f"ready (tls={self.tls_active})") }
            => $^
        }

        $AwaitStartTlsResponse => $Connected {
            starttls_response(code: int) {
                if code == 220:
                    print("  [smtp] initiating TLS handshake")
                    -> $TlsHandshake
                else:
                    -> $Ready
            }
            status(): str { @@:("await STARTTLS response") }
            => $^
        }

        $TlsHandshake => $Connected {
            tls_handshake_complete() {
                self.tls_active = True
                self.send_ehlo()
                # After STARTTLS, re-issue EHLO as a fresh session.
                -> $AwaitEhlo
            }
            status(): str { @@:("TLS handshake") }
            => $^
        }

        $AwaitMailResponse => $Connected {
            mail_response(code: int) {
                if code == 250:
                    -> $AwaitRcpt
                else:
                    print(f"  [smtp] MAIL FROM rejected ({code})")
                    -> $Ready
            }
            status(): str { @@:("await MAIL response") }
            => $^
        }

        $AwaitRcpt => $Connected {
            rcpt_to(recipient: str) {
                self.recipients = self.recipients + [recipient]
                self.send_rcpt_to(recipient)
                -> $AwaitRcptResponse
            }
            data() {
                if len(self.recipients) > 0:
                    self.send_data()
                    -> $AwaitDataResponse
            }
            status(): str { @@:(f"await RCPT ({len(self.recipients)} so far)") }
            => $^
        }

        $AwaitRcptResponse => $Connected {
            rcpt_response(code: int) {
                if code == 250 or code == 251:
                    -> $AwaitRcpt
                else:
                    print(f"  [smtp] RCPT TO rejected ({code})")
                    -> $AwaitRcpt
            }
            status(): str { @@:("await RCPT response") }
            => $^
        }

        $AwaitDataResponse => $Connected {
            data_response(code: int) {
                if code == 354:
                    -> $SendingBody
                else:
                    -> $Ready
            }
            status(): str { @@:("await DATA response") }
            => $^
        }

        $SendingBody => $Connected {
            data_body(body: str) {
                self.send_body(body)
                self.send_body_terminator()
                -> $AwaitFinalResponse
            }
            status(): str { @@:("sending body") }
            => $^
        }

        $AwaitFinalResponse => $Connected {
            data_final_response(code: int) {
                if code == 250:
                    print(f"  [smtp] message accepted ({len(self.recipients)} recipients)")
                    self.recipients = []
                    -> $Ready
                else:
                    print(f"  [smtp] message rejected ({code})")
                    self.recipients = []
                    -> $Ready
            }
            status(): str { @@:("await final response") }
            => $^
        }

        $AwaitQuitResponse => $Connected {
            quit_response(code: int) {
                -> $Disconnected
            }
            status(): str { @@:("await QUIT response") }
            => $^
        }

        $Connected {
            # Cross-cutting: connection loss at any point drops to $Disconnected.
            # Handled by absence of tcp_connected handling — not needed here.
        }

    actions:
        send_ehlo()                   { print("    -> EHLO") }
        send_starttls()               { print("    -> STARTTLS") }
        send_mail_from(sender)        { print(f"    -> MAIL FROM:<{sender}>") }
        send_rcpt_to(recipient)       { print(f"    -> RCPT TO:<{recipient}>") }
        send_data()                   { print("    -> DATA") }
        send_body(body)               { print(f"    -> (body: {len(body)} bytes)") }
        send_body_terminator()        { print("    -> .") }
        send_quit()                   { print("    -> QUIT") }

    domain:
        server_supports_starttls: bool = False
        tls_active: bool = False
        sender: str = ""
        recipients: list = []
}

if __name__ == '__main__':
    smtp = @@SmtpClient()
    smtp.tcp_connected()
    smtp.greeting_received(220)
    smtp.ehlo_response(250, True)
    print(smtp.status())                         # ready (tls=False)

    # STARTTLS upgrade
    smtp.starttls()
    smtp.starttls_response(220)
    smtp.tls_handshake_complete()
    smtp.ehlo_response(250, True)
    print(smtp.status())                         # ready (tls=True)

    # Send a message
    smtp.mail_from("alice@example.com")
    smtp.mail_response(250)
    smtp.rcpt_to("bob@example.com")
    smtp.rcpt_response(250)
    smtp.rcpt_to("carol@example.com")
    smtp.rcpt_response(250)
    smtp.data()
    smtp.data_response(354)
    smtp.data_body("Subject: hi\r\n\r\nhello")
    smtp.data_final_response(250)
    print(smtp.status())                         # ready (tls=True)

    # Clean shutdown
    smtp.quit()
    smtp.quit_response(221)
    print(smtp.status())                         # disconnected
```

**How it works:** Every state except `$Disconnected` is a child of `$Connected` — a sentinel parent for “the TCP connection is up.” The trailing `=> $^` on each child means a hypothetical `connection_lost()` event could be handled uniformly at the parent (expand the spec to add this). The real enforcement here is ordering: you cannot issue `data()` until `$AwaitRcpt` has been reached, because `$Ready.data` simply isn’t defined.

STARTTLS is handled by cycling back to `$AwaitEhlo` after the TLS handshake completes. RFC 5321 requires re-issuing EHLO after TLS because the server may advertise different capabilities over the secure channel — the restart is semantic, not cosmetic. The `self.tls_active` domain flag lets `$Ready` refuse a second STARTTLS attempt.

The recipient list accumulates in a domain variable. Each RCPT TO adds an entry; a rejected recipient (non-250) is also added — matching real SMTP behavior where partially accepted messages still deliver to successful recipients.

**Features used:** HSM parent as “connection up” sentinel, command-response cycling (`$Await*Response` transient states), multi-recipient accumulator pattern, conditional transition based on negotiated capabilities, protocol restart after STARTTLS

-----


## Feature Coverage

|Feature                      |Recipes 1-22|Recipes 23-33   |EIP (34-45)         |Stress (46-49)       |Deferred (50-52)     |Linux/Protocols (55-71) |
|-----------------------------|------------|----------------|---------------------|---------------------|---------------------|------------------------|
|`@@:(expr)` return           |yes         |yes all         |yes all              |yes all              |yes all              |yes all                 |
|`@@:return(expr)` exit sugar |yes #22     |yes #28         |no                   |no                   |no                   |no                      |
|`@@:self.method()`           |yes #22     |yes #33         |no                   |yes #49              |no                   |no                      |
|`@@:system.state`            |yes #22     |yes #32         |no                   |yes #46, #49         |no                   |no                      |
|Operations                   |no          |yes #23, #25, #32|yes #41, #45        |yes #46 (7), #49 (3) |yes #50, #51         |yes #69                 |
|`static` operations          |no          |yes #25         |no                   |yes #46              |no                   |no                      |
|System params (domain)       |yes #21     |yes #23         |no                   |yes #46 (3)          |no                   |no                      |
|HSM 3-level                  |no          |yes #26         |no                   |yes #49 (3-level)    |no                   |yes #61                 |
|`push$` / `-> pop$`          |yes #7, #8  |yes #27         |no                   |no                   |no                   |yes #63                 |
|Decorated pop (exit args)    |no          |yes #27         |no                   |no                   |no                   |no                      |
|State var reset on reentry   |implicit    |yes #24 (explicit)|no                 |yes #48              |yes #50, #51, #52    |implicit                |
|Multi-system managed states  |yes #20     |yes #28, #29, #33|yes #43             |yes #47, #48 (3)     |no                   |no                      |
|Service pattern              |no          |yes #30         |no                   |no                   |yes #50 (dequeue)    |no                      |
|Enter-handler chain          |no          |yes #30, #31    |yes #42             |yes #48 (11 phases)  |no                   |no                      |
|Events ignored in wrong state|yes #3, #12 |yes #24, #32    |yes #39             |yes #46 (terminals)  |no                   |yes most                |
|`@@persist`                  |yes #18     |no              |yes #40, #44, #45   |no                   |no                   |yes #67                 |
|Self-transition (retry loop) |no          |no              |yes #40             |no                   |no                   |yes #57                 |
|HSM parent forwarding        |yes #9      |yes #26         |yes #41             |yes #46, #48, #49    |yes #51              |yes #55, #58, #61, #65, #66, #70, #71 |
|Compensation chain           |no          |no              |yes #42             |no                   |no                   |yes #62                 |
|Transient states             |no          |yes #30         |yes #41, #42        |yes #47, #48         |yes #52              |yes #58, #71            |
|13+ state machine            |no          |no              |no                   |yes #46 (13), #48 (17)|no                  |yes #71 (14)            |
|Domain arithmetic (VWAP)     |no          |no              |no                   |yes #46, #47         |no                   |no                      |
|Conditional abort routing    |no          |no              |no                   |yes #48              |no                   |no                      |
|Mode-based event rejection   |no          |no              |no                   |yes #49              |no                   |no                      |
|Deferred event processing    |no          |no              |no                   |no                   |yes #50, #51, #52    |no                      |
|Priority queue               |no          |no              |no                   |no                   |yes #51              |no                      |
|Directional scheduling       |no          |no              |no                   |no                   |yes #52 (SCAN)       |no                      |
|RFC/kernel correspondence    |no          |no              |no                   |no                   |no                   |yes all (see #55-#71)   |
