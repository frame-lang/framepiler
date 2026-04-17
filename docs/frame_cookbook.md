# Frame Cookbook

33 recipes showing how to solve real problems with Frame. Each recipe is a complete, runnable Frame spec with an explanation of the key patterns used.

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
                    -> $LoggedIn
                else:
                    @@:("invalid - try again")
                    -> $EnterUsername
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
                -> $Connected
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
                    -> $Succeeded
                else:
                    if self.attempts >= self.max_retries:
                        -> $Failed
                    else:
                        -> $Trying
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
                    -> $Processing
                else:
                    -> $Rejected
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
                    -> $Pressed
            }
            is_pressed(): bool { @@:(False) }
        }
        $Pressed {
            $.stable_count: int = 0

            raw_low() { $.stable_count = $.stable_count + 1 }
            raw_high() { $.stable_count = 0 }
            tick() {
                if $.stable_count >= 3:
                    -> $Released
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
                    -> $S1
                else:
                    @@:("0")
            }
        }
        $S1 {
            input(bit: int): str {
                if bit == 0:
                    @@:("1")
                    -> $S0
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
                    -> $Odd
            }
            output(): str { @@:("even") }
        }
        $Odd {
            input(bit: int) {
                if bit == 1:
                    -> $Even
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
                    -> $Processing
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
                    -> $Dispensing
            }
            cancel(): int {
                refund = self.balance
                self.balance = 0
                @@:(refund)
                -> $Idle
            }
        }
        $Dispensing {
            $>() {
                change = self.balance
                self.balance = 0
                if change > 0:
                    print(f"Change: {change}")
                -> $Idle
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
                    -> $Open
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
                    -> $HalfOpen
            }
            status(): str { @@:(f"open ({$.cooldown_remaining} ticks left)") }
        }
        $HalfOpen {
            call(): str { @@:("testing") }
            success() {
                print("Circuit recovered")
                -> $Closed
            }
            failure() {
                print("Still failing")
                -> $Open
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
                    -> $Throttled
            }
            tick() { self.tokens = min(self.tokens + 1, self.max_tokens) }
        }
        $Throttled {
            request(): str { @@:("throttled") }
            tick() {
                self.tokens = self.tokens + 1
                if self.tokens > 0:
                    -> $Accepting
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
                -> $LowHeat
            }
            status(): str { @@:("off") }
        }
        $LowHeat => $Heating {
            $>() { print("Low heat on") }
            tick() {
                if self.target - self.current > 5:
                    -> $HighHeat
            }
            status(): str { @@:(f"low heat ({self.current} to {self.target})") }
            => $^
        }
        $HighHeat => $Heating {
            $>() { print("High heat on") }
            tick() {
                if self.target - self.current <= 3:
                    -> $LowHeat
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
                    -> $Cooling
            }
            set_temp(target: int) { self.target = target }
        }
        $Cooling => $On {
            $>() { print("Cooling") }
            tick() {
                self.current = self.current - 1
                if self.current <= self.target:
                    -> $Off
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
                    -> $EnterPassword
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
                -> $LoggedIn
            }
            auth_cancelled() { print("[App] Cancelled") }
            auth_locked(username: str) { -> $Locked }
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
                    -> $Lost
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
                    -> $InLevel
                else:
                    -> $GameOver
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
                -> $FetchData
            }
        }
        $FetchData => $Running {
            $>() {
                if self.cycles >= self.max_cycles:
                    -> $ShuttingDown
                print(f"[Worker] Fetch (cycle {self.cycles + 1})")
                self.cycles = self.cycles + 1
                -> $ProcessData
            }
            => $^
        }
        $ProcessData => $Running {
            $>() {
                print(f"[Worker] Process (cycle {self.cycles})")
                -> $FetchData
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
                -> $Validate
            }
        }
        $Validate {
            $>() {
                self.log.append("validate")
                self.data = [x for x in self.data if x is not None and x != ""]
                if len(self.data) == 0:
                    -> $Error
                else:
                    -> $Normalize
            }
        }
        $Normalize {
            $>() {
                self.log.append("normalize")
                self.data = [str(x).lower().strip() for x in self.data]
                -> $Deduplicate
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
                -> $Sort
            }
        }
        $Sort {
            $>() {
                self.log.append("sort")
                self.data = sorted(self.data)
                -> $Complete
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
                if $.ticks >= self.green_dur: -> $Yellow
            }
            emergency() { -> $EmergencyRed }
        }
        $Yellow {
            $.ticks: int = 0
            next() {
                $.ticks = $.ticks + 1
                if $.ticks >= self.yellow_dur: -> $Red
            }
            emergency() { -> $EmergencyRed }
        }
        $Red {
            $.ticks: int = 0
            next() {
                $.ticks = $.ticks + 1
                if $.ticks >= self.red_dur: -> $Green
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
                -> $AwaitingApproval
            }
            => $^
        }
        $AwaitingApproval => $Active {
            approve() {
                self.step = 0
                -> $Coding
            }
            reject(feedback: str) { -> $Planning }
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
            coding_done() { -> $Testing }
            tool_completed(tool: str, success: bool, data: str) {
                if success:
                    print(f"  ok {data}")
                    self.step = self.step + 1
                    -> $Coding
                else:
                    print(f"  err {data}")
                    self.last_error = data
                    -> $ErrorRecovery
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
            tests_passed() { -> $Complete }
            tests_failed(failures: str) {
                self.retries = self.retries + 1
                if self.retries >= 2:
                    -> $Failed
                else:
                    self.task_desc = f"Fix: {failures}"
                    -> $Planning
            }
            => $^
        }
        $ErrorRecovery => $Active {
            $>() { print(f"  [warn] {self.last_error}") }
            approve() { -> $Coding }
            reject(feedback: str) { -> $Planning }
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
                -> $Planning
            }
            status(): str { @@:("complete") }
        }
        $Failed {
            $>() { print("\n[fail] Failed") }
            task(description: str) {
                self.reset()
                self.task_desc = description
                -> $Planning
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

## Feature Coverage

|Feature                      |Recipes 1-22|Recipes 23-33   |
|-----------------------------|------------|----------------|
|`@@:(expr)` return           |yes         |yes all         |
|`@@:return(expr)` exit sugar |yes #22     |yes #28         |
|`@@:self.method()`           |yes #22     |yes #33         |
|`@@:system.state`            |yes #22     |yes #32         |
|Operations                   |no          |yes #23, #25, #32|
|`static` operations          |no          |yes #25         |
|System params (domain)       |yes #21     |yes #23         |
|HSM 3-level                  |no          |yes #26         |
|`push$` / `-> pop$`          |yes #7, #8  |yes #27         |
|Decorated pop (exit args)    |no          |yes #27         |
|State var reset on reentry   |implicit    |yes #24 (explicit)|
|Multi-system managed states  |yes #20     |yes #28, #29, #33|
|Service pattern              |no          |yes #30         |
|Enter-handler chain          |no          |yes #30, #31    |
|Events ignored in wrong state|yes #3, #12 |yes #24, #32    |
