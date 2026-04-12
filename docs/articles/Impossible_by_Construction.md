# Impossible by Construction: Frame and Agent Security

How compile-time state machines enforce safety properties that runtime checks cannot — applied to the agent systems where it matters most.

## Table of Contents

- [The Thesis](#the-thesis)
- [Two Kinds of Safety](#two-kinds-of-safety)
- [The Principle in Abstract](#the-principle-in-abstract)
- [Spotlight 1: OpenClaw Skill Execution](#spotlight-1-openclaw-skill-execution)
- [Spotlight 2: MCP Tool Authorization](#spotlight-2-mcp-tool-authorization)
- [Spotlight 3: Browser Agent Navigation Control](#spotlight-3-browser-agent-navigation-control)
- [What This Does and Doesn't Protect Against](#what-this-does-and-doesnt-protect-against)
- [The Visibility Argument](#the-visibility-argument)
- [Appendix: Frame in 60 Seconds](#appendix-frame-in-60-seconds)

---

## The Thesis

Every safety mechanism in today's AI agent systems is a runtime check. Permission flags, allowlists, sandbox configurations, confirmation dialogs, tool policies, scope validators — each one is code that runs and decides whether to allow or block an action. Each one can be bypassed if control flow is redirected, if the check is misconfigured, if a code path was added that doesn't go through the check, or if the runtime state is manipulated by a prompt injection.

Frame offers a different kind of safety: **impossible by construction**. In a Frame state machine, if a state doesn't have a handler for an event, that event does nothing in that state. Not "the event is checked and rejected" — the event has no dispatch path. There is no code to bypass because there is no code. The capability doesn't exist in that state.

This distinction matters because **the LLM doesn't control the transition graph**. A prompt injection can corrupt what the LLM decides within a state — which tool to call, what arguments to pass, what text to generate. But it cannot make the state machine transition along a path that doesn't exist. The transition graph is fixed at compile time by the Frame specification. The LLM is powerful within the boundaries of the current state. It is powerless to alter which states are reachable or which events are handled.

This article applies that principle to three agent systems where security failures have been documented and consequential: OpenClaw's skill execution pipeline, MCP's tool authorization boundary, and browser agents' navigation control. Each faces different threats. Each benefits from the same structural guarantee.

---

## Two Kinds of Safety

**Safety by convention** means there is code that enforces a policy. The policy is correct as long as the code is correct, every execution path goes through the code, the code isn't modified, and the runtime state it depends on hasn't been tampered with.

```javascript
// Safety by convention: a check
async function executeTool(tool, args) {
    if (!tool.isValidated) {
        throw new Error("Tool not validated");
    }
    if (tool.requiresConfirmation && !args.userConfirmed) {
        throw new Error("User confirmation required");
    }
    return await tool.execute(args);
}
```

Every `if` is a check that assumes control flow reaches it. A new code path, a refactored function, a plugin that calls `tool.execute()` directly — any of these bypass the checks silently. The safety property is only as strong as the discipline of every contributor to every code path.

**Safety by construction** means the unsafe behavior has no implementation. There is no code to bypass because the capability was never provided in the relevant context.

```
$Validating {
    validation_passed() {
        -> $ConfirmingWithUser
    }
    validation_failed(reason: str) {
        -> $Rejected
    }
    # No execute() handler exists here.
    # Calling execute() in this state does nothing.
    # Not because of a check — because there is no dispatch path.
}
```

There is exactly one path to `$Executing`: through `$Validating` → `$ConfirmingWithUser` → `$Executing`. No shortcut exists. Adding one would require adding a handler or transition to the Frame specification — a visible, reviewable, diff-able change to the state machine definition.

---

## The Principle in Abstract

Before diving into specific systems, here is the general pattern. Every security-critical agent workflow has the same structure:

1. An action that must not happen without preconditions (execution, navigation, data access)
2. Preconditions that must be satisfied first (validation, authorization, user confirmation)
3. Runtime checks that enforce the ordering — and code paths that can potentially skip them

Frame replaces step 3 with a structural guarantee: the action only exists in a state that is only reachable through states that enforce the preconditions. The ordering is a property of the transition graph, not a property of runtime code.

The generated code is a plain class in your target language (TypeScript, Python, Rust, and 14 others). No runtime library, no dependencies. The transition graph can be extracted as a diagram with a single command:

```bash
framec system.fts -l graphviz | dot -Tpng -o transitions.png
```

The resulting image shows every state and every possible transition. Missing safety gates are visible as edges that bypass required states.

---

## Spotlight 1: OpenClaw Skill Execution

### The system

OpenClaw is a self-hosted, open-source AI agent framework with 310k+ GitHub stars. It connects LLMs to messaging platforms (WhatsApp, Telegram, Discord) and extends agent capabilities through a community-contributed skill system. Skills are the highest-risk component: they're third-party code executing with the agent's privileges.

### The threat

The security track record reflects the risk. A third-party OpenClaw skill was found performing data exfiltration and prompt injection without user awareness. The skill repository's vetting was insufficient to prevent the malicious submission. One of OpenClaw's own maintainers warned on Discord that the project is "far too dangerous" for users who can't review code.

The current defenses are convention-based: skill metadata checks, configurable user confirmation for sensitive operations, and optional sandboxing. Each is a runtime check in the execution path that can be bypassed by a code path that doesn't go through it.

### The Frame system

```
@@target typescript

@@system SkillExecutor {
    interface:
        submit(skill_id: str, args: str)
        validation_result(passed: bool, reason: str)
        user_decision(approved: bool)
        sandbox_result(ready: bool, error: str)
        execution_complete(output: str)
        execution_error(error: str)
        timeout()
        get_status(): str = "idle"

    machine:
        $Idle {
            submit(skill_id: str, args: str) {
                self.skill_id = skill_id
                self.args = args
                self.audit("submit", skill_id)
                -> $Validating
            }
            get_status(): str { @@:("idle") }
        }

        $Validating {
            $>() {
                self.start_timer(10)
                self.validate_skill(self.skill_id, self.args)
            }

            validation_result(passed: bool, reason: str) {
                self.cancel_timer()
                if passed:
                    if self.requires_confirmation(self.skill_id):
                        -> $ConfirmingWithUser
                    else:
                        -> $Sandboxing
                else:
                    self.audit("validation_failed", reason)
                    -> $Rejected
                
            }

            timeout() {
                self.audit("validation_timeout", self.skill_id)
                -> $Rejected
            }

            get_status(): str { @@:("validating") }

            # submit() not handled — can't queue during validation.
            # execution_complete() not handled — can't fake a result.
        }

        $ConfirmingWithUser {
            $>() {
                self.start_timer(300)
                self.prompt_user(self.skill_id, self.args)
            }

            user_decision(approved: bool) {
                self.cancel_timer()
                if approved:
                    self.audit("user_approved", self.skill_id)
                    -> $Sandboxing
                else:
                    self.audit("user_rejected", self.skill_id)
                    -> $Rejected
                
            }

            timeout() {
                self.audit("confirmation_timeout", self.skill_id)
                -> $Rejected
            }

            get_status(): str { @@:("awaiting user confirmation") }

            # The ONLY events that advance are user_decision() and timeout().
            # The LLM cannot approve on the user's behalf.
        }

        $Sandboxing {
            $>() {
                self.start_timer(15)
                self.setup_sandbox(self.skill_id)
            }

            sandbox_result(ready: bool, error: str) {
                self.cancel_timer()
                if ready:
                    -> $Executing
                else:
                    self.audit("sandbox_failed", error)
                    -> $Failed
                
            }

            timeout() {
                self.audit("sandbox_timeout", self.skill_id)
                -> $Failed
            }

            get_status(): str { @@:("sandboxing") }
        }

        $Executing {
            $>() {
                self.start_timer(self.exec_timeout)
                self.audit("executing", self.skill_id)
                self.run_skill(self.skill_id, self.args)
            }

            execution_complete(output: str) {
                self.cancel_timer()
                self.last_output = output
                -> $ValidatingOutput
            }

            execution_error(error: str) {
                self.cancel_timer()
                self.audit("execution_error", error)
                -> $Failed
            }

            timeout() {
                self.kill_sandbox()
                self.audit("execution_timeout", self.skill_id)
                -> $Failed
            }

            get_status(): str { @@:("executing") }
        }

        $ValidatingOutput {
            $>() {
                self.check_output(self.last_output)
            }

            validation_result(passed: bool, reason: str) {
                if passed:
                    -> $Complete
                else:
                    self.audit("output_blocked", reason)
                    self.last_output = ""
                    -> $Failed
                
            }

            get_status(): str { @@:("validating output") }
        }

        $Complete {
            submit(skill_id: str, args: str) {
                self.skill_id = skill_id
                self.args = args
                self.audit("submit", skill_id)
                -> $Validating
            }
            get_status(): str { @@:("complete") }
        }

        $Rejected {
            submit(skill_id: str, args: str) {
                self.skill_id = skill_id
                self.args = args
                self.audit("submit", skill_id)
                -> $Validating
            }
            get_status(): str { @@:("rejected") }
        }

        $Failed {
            submit(skill_id: str, args: str) {
                self.skill_id = skill_id
                self.args = args
                self.audit("submit", skill_id)
                -> $Validating
            }
            get_status(): str { @@:("failed") }
        }

    actions:
        validate_skill(skill_id, args) { }
        requires_confirmation(skill_id) { return true }
        prompt_user(skill_id, args) { }
        setup_sandbox(skill_id) { }
        run_skill(skill_id, args) { }
        check_output(output) { }
        kill_sandbox() { }
        audit(event, detail) { console.log("[audit] " + event + ": " + detail) }
        start_timer(seconds) { }
        cancel_timer() { }

    domain:
        skill_id: str = ""
        args: str = ""
        last_output: str = ""
        exec_timeout: int = 60
}
```

### What's impossible by construction

**Executing without validation.** No transition from `$Idle` to `$Executing` exists. Every path passes through `$Validating`.

**Executing without user confirmation (when required).** If `requires_confirmation()` returns true, the flow must pass through `$ConfirmingWithUser`. Only `user_decision()` and `timeout()` advance that state. The LLM cannot self-approve.

**Executing without sandboxing.** `$Sandboxing` must succeed to reach `$Executing`. No fallback path skips it.

**Concurrent execution.** While any skill is active, `submit()` is not handled. No queuing, no race conditions.

**Delivering unvalidated output.** Output passes through `$ValidatingOutput` before `$Complete`. Blocked output is cleared.

**Indefinite hangs.** Every active state has a timeout handler.

### Convention vs. construction

| Safety property | Convention | Construction |
|----------------|-----------|-------------|
| Validate before execute | `if (!validated) throw` | No handler in pre-validation states |
| User confirmation | `if (needsConfirm && !confirmed) throw` | Only `user_decision()` advances `$ConfirmingWithUser` |
| Sandbox required | `if (sandboxEnabled) runInSandbox()` | Only `$Sandboxing` → `$Executing` path exists |
| No concurrent runs | Mutex / queue logic | `submit()` absent in active states |
| Output validation | `if (checkOutput(result))` | `$ValidatingOutput` gates `$Complete` |

---

## Spotlight 2: MCP Tool Authorization

### The system

The Model Context Protocol (MCP) is the emerging standard for connecting LLMs to external tools and data sources. MCP servers expose capabilities — file access, database queries, API calls, code execution — that any MCP client can invoke on behalf of the LLM. The protocol is adopted across Claude Desktop, Claude Code, IDE integrations, and a growing ecosystem of third-party clients and servers.

### The threat

MCP's security surface is large and actively exploited. A critical command-injection vulnerability in `mcp-remote` (437,000+ downloads, used by Cloudflare, Hugging Face, Auth0) allowed malicious MCP servers to achieve remote code execution on client machines. A prompt-injection attack against the official GitHub MCP server hijacked an AI assistant to exfiltrate private repository contents via a public pull request. Tool poisoning attacks manipulate tool descriptions to trick agents into performing unintended actions.

The core architectural issue is the confused deputy problem: the MCP client acts with the user's privileges, but its decisions are influenced by LLM output, which is influenced by untrusted content (web pages, documents, tool descriptions). The boundary between "the LLM requested this tool" and "the tool executes with the user's authority" is a trust boundary that current implementations enforce through runtime checks — scope validation, user confirmation prompts, allowlists.

The MCP security best practices specification itself identifies the core problems: dynamic tool discovery that can change available tools at runtime, broad scope grants that give tokens access far beyond what's needed, and the absence of standardized mechanisms for verifying server provenance. Research has identified 57 distinct threats across five MCP components using STRIDE/DREAD frameworks.

### The Frame system

A Frame system wrapping an MCP client's tool invocation pipeline can enforce authorization ordering structurally.

```
@@target typescript

@@system McpToolGate {
    interface:
        tool_requested(server_id: str, tool_name: str, args: str)
        scope_check_result(allowed: bool, reason: str)
        user_decision(approved: bool)
        tool_result(result: str)
        tool_error(error: str)
        timeout()
        get_status(): str = "idle"

    machine:
        $Idle {
            tool_requested(server_id: str, tool_name: str, args: str) {
                self.server_id = server_id
                self.tool_name = tool_name
                self.args = args
                -> $CheckingProvenance
            }
            get_status(): str { @@:("idle") }
        }

        $CheckingProvenance {
            $>() {
                if self.is_trusted_server(self.server_id):
                    -> $CheckingScope
                else:
                    self.audit("untrusted_server", self.server_id)
                    -> $Blocked
                
            }
            get_status(): str { @@:("checking provenance") }
        }

        $CheckingScope {
            $>() {
                self.start_timer(5)
                self.check_scope(self.server_id, self.tool_name, self.args)
            }

            scope_check_result(allowed: bool, reason: str) {
                self.cancel_timer()
                if allowed:
                    if self.is_sensitive_tool(self.tool_name):
                        -> $ConfirmingWithUser
                    else:
                        -> $Invoking
                else:
                    self.audit("scope_denied", reason)
                    -> $Blocked
                
            }

            timeout() {
                self.audit("scope_check_timeout", self.tool_name)
                -> $Blocked
            }

            get_status(): str { @@:("checking scope") }
        }

        $ConfirmingWithUser {
            $>() {
                self.start_timer(120)
                self.prompt_user(self.tool_name, self.args)
            }

            user_decision(approved: bool) {
                self.cancel_timer()
                if approved:
                    self.audit("user_approved", self.tool_name)
                    -> $Invoking
                else:
                    self.audit("user_denied", self.tool_name)
                    -> $Blocked
                
            }

            timeout() {
                self.audit("confirmation_timeout", self.tool_name)
                -> $Blocked
            }

            get_status(): str { @@:("awaiting user confirmation") }
        }

        $Invoking {
            $>() {
                self.start_timer(self.tool_timeout)
                self.audit("invoking", self.tool_name)
                self.invoke_tool(self.server_id, self.tool_name, self.args)
            }

            tool_result(result: str) {
                self.cancel_timer()
                self.last_result = result
                -> $ScanningResult
            }

            tool_error(error: str) {
                self.cancel_timer()
                self.audit("tool_error", error)
                -> $Failed
            }

            timeout() {
                self.audit("tool_timeout", self.tool_name)
                -> $Failed
            }

            get_status(): str { @@:("invoking tool") }
        }

        $ScanningResult {
            $>() {
                self.scan_for_exfiltration(self.last_result)
            }

            scope_check_result(allowed: bool, reason: str) {
                if allowed:
                    -> $Complete
                else:
                    self.audit("exfiltration_blocked", reason)
                    self.last_result = ""
                    -> $Blocked
                
            }

            get_status(): str { @@:("scanning result") }
        }

        $Complete {
            tool_requested(server_id: str, tool_name: str, args: str) {
                self.server_id = server_id
                self.tool_name = tool_name
                self.args = args
                -> $CheckingProvenance
            }
            get_status(): str { @@:("complete") }
        }

        $Blocked {
            tool_requested(server_id: str, tool_name: str, args: str) {
                self.server_id = server_id
                self.tool_name = tool_name
                self.args = args
                -> $CheckingProvenance
            }
            get_status(): str { @@:("blocked") }
        }

        $Failed {
            tool_requested(server_id: str, tool_name: str, args: str) {
                self.server_id = server_id
                self.tool_name = tool_name
                self.args = args
                -> $CheckingProvenance
            }
            get_status(): str { @@:("failed") }
        }

    actions:
        is_trusted_server(server_id) { return false }
        check_scope(server_id, tool_name, args) { }
        is_sensitive_tool(tool_name) { return true }
        prompt_user(tool_name, args) { }
        invoke_tool(server_id, tool_name, args) { }
        scan_for_exfiltration(result) { }
        audit(event, detail) { }
        start_timer(seconds) { }
        cancel_timer() { }

    domain:
        server_id: str = ""
        tool_name: str = ""
        args: str = ""
        last_result: str = ""
        tool_timeout: int = 30
}
```

### What's impossible by construction

**Invoking a tool from an untrusted server.** `$CheckingProvenance` is the only exit from `$Idle`. Untrusted servers transition to `$Blocked`. There is no path from an untrusted server check to `$Invoking`.

**Invoking a tool outside its granted scope.** `$CheckingScope` gates `$Invoking`. A tool call that exceeds the session's scope transitions to `$Blocked`. The confused deputy cannot invoke tools it isn't authorized for because the authorization check is a mandatory waypoint, not a bypassable guard.

**Invoking a sensitive tool without user confirmation.** Sensitive tools must pass through `$ConfirmingWithUser`. The LLM cannot approve on the user's behalf — `user_decision()` must come from the UI layer, not the agent runtime.

**Returning unscanned results to the LLM.** Tool results pass through `$ScanningResult` before reaching `$Complete`. Data exfiltration patterns, PII, or credential leaks in tool output are caught before the LLM can incorporate them into a response. There is no path from `$Invoking` to `$Complete` that bypasses the scan.

**Tool invocation during any non-idle state.** `tool_requested()` is only handled in `$Idle`, `$Complete`, `$Blocked`, and `$Failed`. While a tool is being checked, confirmed, or invoked, new requests are silently ignored. No interleaving, no race conditions between concurrent tool calls.

### Why this matters for MCP specifically

MCP's dynamic tool discovery means that available tools can change at runtime — a server can add or remove tools between requests. In convention-based code, a tool that wasn't available during initial scope validation might become available later and bypass the check. In the Frame system, every tool request starts at `$Idle` and passes through `$CheckingProvenance` and `$CheckingScope` regardless of when the tool was discovered. The structural gate applies to every invocation, not just the first one.

---

## Spotlight 3: Browser Agent Navigation Control

### The system

Browser agents — OpenAI's ChatGPT Atlas, Perplexity's Comet, and similar products — navigate websites, fill forms, click buttons, and extract data on behalf of users. They're among the most powerful and most vulnerable AI agent systems deployed today.

### The threat

The threat model is uniquely adversarial: the agent reads untrusted web content on every page load, and that content can contain prompt injections. Documented attacks include hidden instructions in web pages that trick agents into exfiltrating user data, prompt injections embedded in images that bypass text-based filtering, calendar invites seeded with malicious prompts that cause agents to access local file systems and exfiltrate data, and URL fragment injections that hide instructions after the `#` character.

The industry acknowledges the problem's severity. OpenAI has stated that prompt injection attacks in AI browsers are "unlikely to ever" be fully solved. Brave's research calls it "a systemic challenge facing the entire category of AI-powered browsers." Perplexity has acknowledged the problem "demands rethinking security from the ground up."

The fundamental issue is that the browser agent operates with the user's authenticated privileges across all connected services — email, calendar, banking, cloud storage. A prompt injection on any web page can potentially direct the agent to take actions in any authenticated service.

### The Frame system

A Frame system for browser agent control can enforce structural boundaries on what actions the agent can take and when.

```
@@target typescript

@@system BrowserAgentGate {
    interface:
        navigate(url: str)
        fill_form(form_id: str, data: str)
        click_button(element_id: str)
        read_page(): str = ""
        domain_check_result(safe: bool)
        action_check_result(safe: bool, risk_level: str)
        user_decision(approved: bool)
        timeout()
        get_status(): str = "idle"

    machine:
        $Idle {
            navigate(url: str) {
                self.target_url = url
                -> $CheckingDomain
            }

            read_page(): str {
                @@:(self.extract_page_content())
            }

            get_status(): str { @@:("idle") }

            # fill_form() and click_button() are NOT handled in $Idle.
            # The agent cannot take actions on a page it hasn't navigated to
            # through the domain check.
        }

        $CheckingDomain {
            $>() {
                self.start_timer(5)
                self.check_domain_safety(self.target_url)
            }

            domain_check_result(safe: bool) {
                self.cancel_timer()
                if safe:
                    -> $Navigating
                else:
                    self.audit("blocked_domain", self.target_url)
                    -> $Blocked
                
            }

            timeout() {
                self.audit("domain_check_timeout", self.target_url)
                -> $Blocked
            }

            get_status(): str { @@:("checking domain") }
        }

        $Navigating {
            $>() {
                self.start_timer(30)
                self.perform_navigation(self.target_url)
                self.cancel_timer()
                -> $Browsing
            }
            get_status(): str { @@:("navigating") }
        }

        $Browsing {
            navigate(url: str) {
                self.target_url = url
                -> $CheckingDomain
            }

            read_page(): str {
                @@:(self.extract_page_content())
            }

            fill_form(form_id: str, data: str) {
                self.pending_action = "fill_form"
                self.action_detail = form_id + "|" + data
                -> $ClassifyingAction
            }

            click_button(element_id: str) {
                self.pending_action = "click_button"
                self.action_detail = element_id
                -> $ClassifyingAction
            }

            get_status(): str { @@:("browsing") }
        }

        $ClassifyingAction {
            $>() {
                self.classify_action_risk(self.pending_action, self.action_detail)
            }

            action_check_result(safe: bool, risk_level: str) {
                if risk_level == "low":
                    -> $ExecutingAction
                elif risk_level == "high":
                    -> $ConfirmingAction
                else:
                    self.audit("action_blocked", self.pending_action)
                    -> $Browsing
                
            }

            get_status(): str { @@:("classifying action") }
        }

        $ConfirmingAction {
            $>() {
                self.start_timer(120)
                self.prompt_user_action(self.pending_action, self.action_detail)
            }

            user_decision(approved: bool) {
                self.cancel_timer()
                if approved:
                    -> $ExecutingAction
                else:
                    self.audit("user_blocked_action", self.pending_action)
                    -> $Browsing
                
            }

            timeout() {
                -> $Browsing
            }

            get_status(): str { @@:("confirming action with user") }
        }

        $ExecutingAction {
            $>() {
                self.execute_pending_action()
                -> $Browsing
            }
            get_status(): str { @@:("executing action") }
        }

        $Blocked {
            navigate(url: str) {
                self.target_url = url
                -> $CheckingDomain
            }
            get_status(): str { @@:("blocked") }
        }

    actions:
        check_domain_safety(url) { }
        perform_navigation(url) { }
        extract_page_content() { return "" }
        classify_action_risk(action, detail) { }
        prompt_user_action(action, detail) { }
        execute_pending_action() { }
        audit(event, detail) { }
        start_timer(seconds) { }
        cancel_timer() { }

    domain:
        target_url: str = ""
        pending_action: str = ""
        action_detail: str = ""
}
```

### What's impossible by construction

**Navigating to an unchecked domain.** Every `navigate()` call passes through `$CheckingDomain`. Unsafe domains transition to `$Blocked`. There is no path from a user request to page content that bypasses the domain check. A prompt injection that tells the agent "now navigate to evil.com" still goes through the check.

**Taking page actions before navigation.** `fill_form()` and `click_button()` are only handled in `$Browsing`, which is only reachable through `$CheckingDomain` → `$Navigating`. The agent cannot fill a form or click a button on a page it hasn't reached through the validated navigation path.

**Executing high-risk actions without user confirmation.** Actions classified as high-risk must pass through `$ConfirmingAction`. Only `user_decision()` advances that state. A prompt injection that convinces the LLM to click "Purchase" or "Delete" still hits the confirmation gate — the LLM cannot bypass it because the transition doesn't exist.

**Executing actions during navigation or domain checking.** While the agent is in `$CheckingDomain` or `$Navigating`, action events (`fill_form()`, `click_button()`) are not handled. A redirect-then-act attack — where a malicious page redirects to a target site and immediately tries to trigger an action — fails because actions are only available in `$Browsing`, which requires the full navigation pipeline to complete.

**Interleaving navigation and actions.** While an action is being classified, confirmed, or executed, `navigate()` is not handled. The agent can't be redirected to a different page mid-action. The action completes (or is blocked) on the current page before any navigation can occur.

### Why this matters for browser agents specifically

The browser agent threat model is uniquely challenging because the adversary controls the content the agent reads. Every web page is potentially adversarial. Convention-based defenses — prompt hardening, secondary LLM critics, behavioral analysis — operate at the inference layer and can be circumvented by sufficiently clever prompt injections. Frame operates at the workflow layer and is immune to prompt injection: the transition graph doesn't change regardless of what the page says.

Frame doesn't prevent the LLM from being tricked into *wanting* to navigate to a malicious site or *wanting* to click a purchase button. It prevents the want from becoming an action without passing through structural gates. The LLM's decision is the input to the state machine; the state machine constrains which actions are actually available. This is the separation that convention-based defenses lack: they try to prevent the LLM from making bad decisions, while Frame constrains the consequences of bad decisions.

---

## What This Does and Doesn't Protect Against

### What "impossible by construction" covers

Frame constrains the **structure** of the workflow — which states are reachable from which other states, and which events are handled in which states. This means:

- Certain sequences of operations are impossible regardless of LLM output
- The transition graph is fixed at compile time and cannot be altered at runtime
- Safety properties that depend on ordering (A must happen before B) are structurally enforced
- The set of possible behavioral modes is finite, enumerable, and declared
- Every safety gate is visible in the transition graph and verifiable by inspection

### What it doesn't cover

Frame does NOT constrain the **content** of decisions within a state:

**Prompt injection within a state.** If the agent is in `$Executing` and the LLM has been manipulated to call a tool with malicious arguments, Frame doesn't prevent that. Frame ensures the agent reached `$Executing` through the proper sequence — it doesn't inspect the arguments. That's the sandbox's job.

**Malicious logic inside actions.** Frame's actions are native code. A malicious skill running inside `run_skill()` has whatever privileges the sandbox grants. Frame ensures the skill can't run without sandboxing; it doesn't control what happens inside the sandbox.

**Social engineering.** If a prompt injection convinces the LLM to tell the user "Please approve this action — it's required for your request," and the user approves, Frame's confirmation state faithfully transitions to execution. Frame enforces that the user was asked; it doesn't ensure the user made a good decision.

**LLM hallucination.** If the LLM generates a tool call for a tool that doesn't exist, or arguments that don't match the schema, Frame doesn't validate this. The tool dispatch layer handles schema validation.

### The boundary

Frame operates at the **workflow orchestration layer**. It guarantees the agent follows the prescribed sequence of phases. It does not operate at the **inference layer** (what the LLM produces) or the **execution layer** (what happens inside a sandboxed tool or on a web page). These layers need their own defenses — prompt hardening, output filtering, sandbox isolation, tool schema validation. Frame doesn't replace those defenses. It ensures they're invoked in the right order and can't be skipped.

The value proposition is precise: Frame eliminates an entire class of vulnerabilities — those caused by code paths that bypass safety checks — at the workflow orchestration layer.

---

## The Visibility Argument

Beyond "impossible by construction," Frame provides a property that matters for every system discussed in this article: **safety properties are visible**.

In imperative code, a safety check is one `if` statement among thousands. A PR that removes or refactors the check might not be caught in review. A new code path that bypasses it might not be recognized as a security regression. The safety property is implicit — you have to trace every execution path to verify it holds.

In a Frame specification, every state and every transition is visible in one place. The transition graph is generated with a single command and inspected visually. A PR that adds a transition from `$Validating` directly to `$Executing` is visible in the graph diff — it's a new edge that bypasses `$ConfirmingWithUser` and `$Sandboxing`. A reviewer doesn't need to trace code paths; they look at the graph.

This matters for:

**Fast-moving open-source projects** (OpenClaw's 600+ contributors, rapid releases) where security-critical changes can be masked by large PRs.

**Protocol implementations** (MCP clients and servers) where many independent developers implement the same security requirements, and structural enforcement prevents implementation variance.

**Adversarial environments** (browser agents) where the threat model demands defense in depth, and the workflow layer's safety properties need to be verifiable independently of the inference layer's defenses.

A policy like "every PR that modifies a Frame system must include the updated GraphViz diagram" gives reviewers a visual diff of behavioral changes. "This PR adds an edge from $CheckingScope to $Invoking that bypasses $ConfirmingWithUser" is a sentence a reviewer can evaluate in seconds.

---

## Appendix: Frame in 60 Seconds

Frame is a language for defining state machines that lives inside your source files. You write a `@@system` block; the framepiler generates a class in your target language (TypeScript, Python, Rust, and 14 others). No runtime library, no dependencies.

```
@@target typescript

@@system Example {
    interface:
        start()
        stop()
        get_status(): str = "idle"

    machine:
        $Idle {
            start() { -> $Running }
            get_status(): str { @@:("idle") }
        }
        $Running {
            stop() { -> $Idle }
            get_status(): str { @@:("running") }
        }
}
```

Key concepts: `$Idle` and `$Running` are **states**. `start()` and `stop()` are **events**. `-> $Running` is a **transition**. Events not handled in a state are silently ignored — `stop()` in `$Idle` does nothing. The first state listed is the start state. `@@:(expr)` sets the return value. `$>` and `<$` are enter/exit handlers.

For the full language, see [Getting Started with Frame](frame_getting_started.md).
For complete syntax, see the [Frame Language Reference](frame_language.md).
For agent workflow patterns, see [Frame for AI Agents](AGENTS.md).