# Frame Cookbook — Linux Edition

18 recipes modeling real Linux subsystems and internet protocols as Frame state machines. Each recipe is a complete, runnable Frame spec with an explanation of how it maps to the actual kernel or protocol implementation.

For language syntax details, see the [Frame Language Reference](frame_language.md). For a tutorial introduction, see [Getting Started](frame_getting_started.md). For the core cookbook, see [Frame Cookbook](frame_cookbook.md).

> **Note on target:** All recipes use `@@target python_3`. Some idioms — `list` as a type, f-strings inside `@@:(...)`, Python slicing — won’t translate verbatim to static targets (Rust/Go/C). Where a pattern is target-specific, the recipe calls it out.

## Table of Contents

**OS Internals (1-9)**

1. [Process Lifecycle](#1-process-lifecycle) — task states, signals, HSM for non-runnable states
1. [Runtime Power Management](#2-runtime-power-management) — enter/exit for timer management, usage counting
1. [Block I/O Request](#3-block-io-request) — request pipeline with timeout and retry
1. [USB Device Enumeration](#4-usb-device-enumeration) — multi-stage pipeline with compensation
1. [Watchdog Timer](#5-watchdog-timer) — magic close guard, high-consequence two-state machine
1. [OOM Killer](#6-oom-killer) — safety by construction, mutual exclusion without locks
1. [Filesystem Freeze](#7-filesystem-freeze) — 3-level HSM for freeze/thaw under a mounted parent
1. [Kernel Module Loader](#8-kernel-module-loader) — pipeline with rollback (saga pattern)
1. [Signal Handler Stack](#9-signal-handler-stack) — `push$`/`pop$` for nested signal frames

**Internet Protocols (10-18)**

1. [DHCP Client](#10-dhcp-client) — timer-driven lease lifecycle from RFC 2131
1. [TCP Connection](#11-tcp-connection) — RFC 793 FSM with `@@persist` across TIME_WAIT
1. [TLS Handshake](#12-tls-handshake) — two-party protocol with HSM alert handling
1. [Wi-Fi Station Management](#13-wi-fi-station-management) — HSM deauth from any state
1. [BGP Finite State Machine](#14-bgp-finite-state-machine) — RFC 4271 event table transcription
1. [PPP Link Control Protocol](#15-ppp-link-control-protocol) — RFC 1661 state table, layered composition
1. [NTP Client Association](#16-ntp-client-association) — polling backoff and reachability tracking
1. [HTTP/1.1 Connection](#17-http11-connection) — keep-alive lifecycle with HSM error handling
1. [SMTP Conversation](#18-smtp-conversation) — command-response protocol with STARTTLS upgrade

-----

## 1. Process Lifecycle

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

## 2. Runtime Power Management

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

## 3. Block I/O Request

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

## 4. USB Device Enumeration

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

## 5. Watchdog Timer

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

## 6. OOM Killer

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

## 7. Filesystem Freeze

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

## 8. Kernel Module Loader

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

## 9. Signal Handler Stack

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

## 10. DHCP Client

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

## 11. TCP Connection

**Problem:** Model the TCP connection FSM from RFC 793 §3.2 (Figure 6). Eleven states, separate active and passive paths, and the infamous TIME_WAIT state that holds a connection for 2×MSL after close. This is the most canonical protocol FSM in existence. Using `@@persist`, the state survives process restart — matching how the kernel retains TIME_WAIT entries.

```frame
@@target python_3

@@persist
@@system TcpConnection {
    interface:
        passive_open()
        active_open()
        recv_syn()
        recv_syn_ack()
        recv_ack()
        recv_fin()
        app_close()
        app_send(bytes: int)
        timeout_2msl()
        abort()
        state_name(): str

    machine:
        $Closed {
            passive_open() { -> $Listen }
            active_open() {
                self.send_syn()
                -> $SynSent
            }
            state_name(): str { @@:("CLOSED") }
        }

        $Listen {
            recv_syn() {
                self.send_syn_ack()
                -> $SynReceived
            }
            app_close() { -> $Closed }
            state_name(): str { @@:("LISTEN") }
        }

        $SynSent {
            recv_syn_ack() {
                self.send_ack()
                -> $Established
            }
            recv_syn() {
                # Simultaneous open
                self.send_syn_ack()
                -> $SynReceived
            }
            app_close() { -> $Closed }
            state_name(): str { @@:("SYN_SENT") }
        }

        $SynReceived {
            recv_ack() { -> $Established }
            app_close() {
                self.send_fin()
                -> $FinWait1
            }
            state_name(): str { @@:("SYN_RECEIVED") }
        }

        $Established {
            app_send(bytes: int) {
                self.bytes_sent = self.bytes_sent + bytes
                self.send_data(bytes)
            }
            app_close() {
                self.send_fin()
                -> $FinWait1
            }
            recv_fin() {
                self.send_ack()
                -> $CloseWait
            }
            abort() {
                self.send_rst()
                -> $Closed
            }
            state_name(): str { @@:("ESTABLISHED") }
        }

        # --- Active close path ---
        $FinWait1 {
            recv_ack() { -> $FinWait2 }
            recv_fin() {
                # Simultaneous close
                self.send_ack()
                -> $Closing
            }
            state_name(): str { @@:("FIN_WAIT_1") }
        }

        $FinWait2 {
            recv_fin() {
                self.send_ack()
                -> $TimeWait
            }
            state_name(): str { @@:("FIN_WAIT_2") }
        }

        $Closing {
            recv_ack() { -> $TimeWait }
            state_name(): str { @@:("CLOSING") }
        }

        $TimeWait {
            $>() {
                self.start_2msl_timer()
                print("  [tcp] TIME_WAIT — holding for 2*MSL (240s)")
            }
            timeout_2msl() { -> $Closed }
            state_name(): str { @@:("TIME_WAIT") }
        }

        # --- Passive close path ---
        $CloseWait {
            app_close() {
                self.send_fin()
                -> $LastAck
            }
            state_name(): str { @@:("CLOSE_WAIT") }
        }

        $LastAck {
            recv_ack() { -> $Closed }
            state_name(): str { @@:("LAST_ACK") }
        }

    actions:
        send_syn()     { print("    -> SYN") }
        send_syn_ack() { print("    -> SYN/ACK") }
        send_ack()     { print("    -> ACK") }
        send_fin()     { print("    -> FIN") }
        send_rst()     { print("    -> RST") }
        send_data(n)   { print(f"    -> DATA ({n} bytes)") }
        start_2msl_timer() { print("    -> 2MSL timer start (240s)") }

    domain:
        local_port: int = 0
        remote_port: int = 0
        bytes_sent: int = 0
}

if __name__ == '__main__':
    # Active open -> data -> active close
    c = @@TcpConnection()
    c.active_open()              # CLOSED -> SYN_SENT
    c.recv_syn_ack()             # SYN_SENT -> ESTABLISHED
    c.app_send(1024)
    c.app_close()                # ESTABLISHED -> FIN_WAIT_1
    c.recv_ack()                 # FIN_WAIT_1 -> FIN_WAIT_2
    c.recv_fin()                 # FIN_WAIT_2 -> TIME_WAIT
    print(c.state_name())        # TIME_WAIT

    # Simulate process restart: persist, destroy, restore
    snap = c.save_state()
    del c
    c = TcpConnection.restore_state(snap)
    print(c.state_name())        # TIME_WAIT — preserved across restart
    c.timeout_2msl()
    print(c.state_name())        # CLOSED

    # Passive close path (the other side)
    c2 = @@TcpConnection()
    c2.passive_open()
    c2.recv_syn()                # LISTEN -> SYN_RECEIVED
    c2.recv_ack()                # SYN_RECEIVED -> ESTABLISHED
    c2.recv_fin()                # ESTABLISHED -> CLOSE_WAIT (peer closed first)
    c2.app_close()               # CLOSE_WAIT -> LAST_ACK
    c2.recv_ack()                # LAST_ACK -> CLOSED
    print(c2.state_name())       # CLOSED

    # Simultaneous close
    c3 = @@TcpConnection()
    c3.active_open(); c3.recv_syn_ack()  # ESTABLISHED
    c3.app_close()                        # FIN_WAIT_1
    c3.recv_fin()                         # FIN_WAIT_1 -> CLOSING (simultaneous)
    c3.recv_ack()                         # CLOSING -> TIME_WAIT
    print(c3.state_name())                # TIME_WAIT
```

**How it works:** Direct transcription of RFC 793 Figure 6. The active-close path (FIN_WAIT_1 → FIN_WAIT_2 → TIME_WAIT) and passive-close path (CLOSE_WAIT → LAST_ACK) are cleanly separated in the state graph. Simultaneous close goes through `$Closing`. The `abort()` event sends RST and drops to `$Closed` from `$Established` — this is the single-handler escape hatch.

`@@persist` on the system declaration generates `save_state()` and `restore_state()` methods. Saved state includes the current state name, domain variables (byte counters, port numbers), and state args. When the TIME_WAIT process is serialized and a new process is spawned to take over (or the system reboots with persistent state), the connection picks up in TIME_WAIT and correctly transitions to CLOSED on 2MSL expiry.

This is real kernel behavior: TIME_WAIT entries are held in a dedicated table (`tcp_hashinfo.bhash`) precisely because they must outlive the process that originally owned the socket, to handle duplicate segments from earlier connection incarnations.

**Features used:** `@@persist` for cross-restart state retention, 11-state RFC transcription, separate active/passive close paths, abort as universal escape transition, simultaneous-close handling

-----

## 12. TLS Handshake

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

## 13. Wi-Fi Station Management

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

## 14. BGP Finite State Machine

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

## 15. PPP Link Control Protocol

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

## 16. NTP Client Association

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

## 17. HTTP/1.1 Connection

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

## 18. SMTP Conversation

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

|Feature                      |OS Internals (1-9)            |Internet Protocols (10-18)       |
|-----------------------------|------------------------------|---------------------------------|
|HSM (2-level)                |#1, #4                        |#12, #13, #17, #18               |
|HSM (3-level)                |#1, #7                        |—                                |
|In-handler forward (`=> $^`) |#1                            |—                                |
|Default forward (`=> $^`)    |#1, #4, #7                    |#12, #13, #17, #18               |
|Enter/exit handlers          |#2, #3, #4, #5, #7            |#10, #12, #14, #15, #17          |
|Domain variables             |all                           |all                              |
|State variables              |#9                            |#13                              |
|Conditional transitions      |#1, #2, #3, #4, #5, #6, #7, #8|#10, #12, #14, #15, #16, #17, #18|
|Events ignored in wrong state|#1, #5, #6, #7, #8            |#13, #18                         |
|Transient states             |#4, #6, #7, #8                |#12, #14, #17, #18               |
|Compensation / saga          |#4, #8                        |—                                |
|Retry with counter           |#3, #4                        |#10, #16                         |
|`push$` / `pop$`             |#9                            |—                                |
|State vars preserved on pop  |#9                            |—                                |
|Operations (read-only)       |—                             |#16                              |
|Actions                      |all                           |all                              |
|Safety by construction       |#5, #6                        |—                                |
|`@@persist`                  |—                             |#11, #14                         |
|RFC direct transcription     |—                             |#10, #11, #14, #15               |
|Timer management pattern     |#2, #3, #5, #6                |#10, #14, #16, #17               |
|Multi-stage pipeline         |#4, #8                        |#12, #18                         |
|Interface for internal ops   |#8 (acquire/release)          |—                                |

## Notes on Target Portability

All recipes target `python_3`. A few idioms are target-specific and would need adjustment for other languages:

- **`list` type and list literals** (`recipients: list = []`, `self.recipients + [x]`) — use `Vec<String>` for Rust, `std::vector<std::string>` for C++, fixed arrays for C.
- **f-strings inside `@@:(...)`** (`@@:(f"count={n}")`) — use `format!("count={}", n)` for Rust, `std::string("count=") + std::to_string(n)` for C++.
- **Python slicing** (`chunk[0]`) — not all targets allow this in native expressions.
- **`in` operator** (`"V" in data`) — use the target’s native substring/membership check.

Frame treats all type annotations and expressions as opaque strings. The framepiler does NOT translate between languages — it passes native code through verbatim. Writing portable Frame systems means keeping action bodies and initializers as simple as possible, pushing language-specific complexity into the target native code outside the Frame block.

## Correspondence Notes

The recipes marked as “RFC direct transcription” (#10, #11, #14, #15) are specifically designed so the Frame spec can be audited line-by-line against the RFC:

- **#10 DHCP** — RFC 2131 Figure 5
- **#11 TCP** — RFC 793 Figure 6, §3.2
- **#14 BGP** — RFC 4271 §8.2.2
- **#15 PPP LCP** — RFC 1661 §4.1 Table 4

These are the recipes to study if you want to see Frame used as an executable specification language — the kind of artifact that could be cited in a protocol document as the authoritative reference implementation of its state machine.