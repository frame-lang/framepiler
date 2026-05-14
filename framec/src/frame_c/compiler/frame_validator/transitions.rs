//! Per-state, handler, and transition validation methods.
//!
//! These methods walk the state machine's per-state structure,
//! checking state-arg/enter-arg arity (E405/E417), handler
//! parameter shape (E406/E419), transition target legality
//! (E402), and `=> $^` forward-target legality (E403).

use super::{arity_error, count_args, required_event_params, FrameValidator, ValidationError};
use crate::frame_c::compiler::frame_ast::*;
use std::collections::{HashMap, HashSet};

impl FrameValidator {
    /// Validate a state
    pub(super) fn validate_state(
        &mut self,
        state: &StateAst,
        state_map: &HashMap<String, &StateAst>,
        interface_methods: &HashMap<String, &InterfaceMethod>,
        _actions: &HashSet<String>,
        _operations: &HashSet<String>,
    ) {
        // E410: Validate no duplicate state variable names
        {
            let mut seen_vars: HashSet<String> = HashSet::new();
            for sv in &state.state_vars {
                if !seen_vars.insert(sv.name.clone()) {
                    self.errors.push(
                        ValidationError::new(
                            "E410",
                            format!(
                                "Duplicate state variable '$.{}' in state '{}'",
                                sv.name, state.name
                            ),
                        )
                        .with_span(sv.span.clone()),
                    );
                }
            }
        }

        // E403: Validate parent state exists for HSM
        if let Some(parent_name) = &state.parent {
            if !state_map.contains_key(parent_name) {
                self.errors.push(
                    ValidationError::new(
                        "E403",
                        format!(
                            "State '{}' has invalid parent '{}'. Available states: {}",
                            state.name,
                            parent_name,
                            self.format_available_states(state_map)
                        ),
                    )
                    .with_span(state.span.clone()),
                );
            }
        }

        // E117: Validate no duplicate handlers in a state. Two handlers
        // with the same event name in the same state would silently
        // shadow each other in codegen — surface as a hard error so
        // authors don't end up with the unreachable-handler footgun.
        {
            let mut seen_events: HashSet<String> = HashSet::new();
            for handler in &state.handlers {
                if !seen_events.insert(handler.event.clone()) {
                    self.errors.push(
                        ValidationError::new(
                            "E117",
                            format!(
                                "Duplicate handler '{}' in state '{}'",
                                handler.event, state.name
                            ),
                        )
                        .with_span(handler.span.clone()),
                    );
                }
            }
        }

        // Validate handlers
        for handler in &state.handlers {
            self.validate_handler(
                handler,
                state,
                state_map,
                interface_methods,
                _actions,
                _operations,
            );
        }

        // Validate enter handler
        if let Some(enter) = &state.enter {
            self.validate_handler_body(&enter.body, state, state_map);
        }

        // Validate exit handler
        if let Some(exit) = &state.exit {
            self.validate_handler_body(&exit.body, state, state_map);
        }
    }

    /// Validate a handler
    pub(super) fn validate_handler(
        &mut self,
        handler: &HandlerAst,
        state: &StateAst,
        state_map: &HashMap<String, &StateAst>,
        interface_methods: &HashMap<String, &InterfaceMethod>,
        _actions: &HashSet<String>,
        _operations: &HashSet<String>,
    ) {
        // E406: Check if handler corresponds to interface method
        if let Some(method) = interface_methods.get(&handler.event) {
            // Validate parameter count matches
            if handler.params.len() != method.params.len() {
                self.errors.push(ValidationError::new(
                    "E406",
                    format!(
                        "Handler '{}' in state '{}' has {} parameters but interface method expects {}",
                        handler.event,
                        state.name,
                        handler.params.len(),
                        method.params.len()
                    )
                ).with_span(handler.span.clone()));
            }
        }

        self.validate_handler_body(&handler.body, state, state_map);
    }

    /// Validate handler body statements
    pub(super) fn validate_handler_body(
        &mut self,
        body: &HandlerBody,
        state: &StateAst,
        state_map: &HashMap<String, &StateAst>,
    ) {
        // E400: Check that terminal statements (transitions, forwards) are last
        self.validate_terminal_last(body);

        for statement in &body.statements {
            match statement {
                Statement::Transition(transition) => {
                    self.validate_transition(transition, state, state_map);
                }
                Statement::Forward(forward) => {
                    self.validate_forward(forward, state, state_map);
                }
                _ => {
                    // Other statements don't need validation yet
                }
            }
        }
    }

    /// E400: Validate that terminal statements (transition, forward) are last in the body
    pub(super) fn validate_terminal_last(&mut self, body: &HandlerBody) {
        let statements = &body.statements;
        if statements.is_empty() {
            return;
        }

        // Find the index of the last terminal statement
        let mut terminal_index: Option<usize> = None;
        for (i, stmt) in statements.iter().enumerate() {
            if self.is_terminal_statement(stmt) {
                terminal_index = Some(i);
            }
        }

        // Check if there's a terminal statement that isn't the last one
        if let Some(idx) = terminal_index {
            let last_idx = statements.len() - 1;
            if idx != last_idx {
                // Check if remaining statements are non-trivial Frame statements.
                // NativeCode is always trivial — Frame is a preprocessor and cannot
                // reason about native control flow (if/else, loops, switch, etc.).
                // The target language compiler handles native reachability.
                // E400 only catches Frame-level unreachability: transition → transition
                // with no native code between them.
                let has_non_trivial_after = statements[idx + 1..].iter().any(|s| match s {
                    Statement::Return(_) => false,
                    Statement::NativeCode(_) => false,
                    _ => true,
                });
                if has_non_trivial_after {
                    let span = match &statements[idx] {
                        Statement::Transition(t) => t.span.clone(),
                        Statement::Forward(f) => f.span.clone(),
                        _ => body.span.clone(),
                    };
                    self.errors.push(
                        ValidationError::new(
                            "E400",
                            "Transition/forward must be the last statement in its containing block"
                                .to_string(),
                        )
                        .with_span(span),
                    );
                }
            }
        }
    }

    /// Check if a statement is a terminal statement (transition only).
    /// Forwards (`=> $^`) are NOT terminal — they dispatch to the parent and return.
    pub(super) fn is_terminal_statement(&self, stmt: &Statement) -> bool {
        matches!(stmt, Statement::Transition(_))
    }

    /// Validate a transition statement
    pub(super) fn validate_transition(
        &mut self,
        transition: &TransitionAst,
        state: &StateAst,
        state_map: &HashMap<String, &StateAst>,
    ) {
        // E402: Check target state exists
        // Skip validation for pop-transition marker $$[-]
        if transition.target == "pop$" {
            // E607: state_args on pop$ are not allowed — the popped
            // compartment brings its own from the snapshot.
            if transition.state_args.is_some() {
                self.errors.push(
                    ValidationError::new(
                        "E607",
                        "State arguments on pop$ are not allowed. The popped compartment carries its own state from the snapshot.".to_string(),
                    )
                    .with_span(transition.span.clone()),
                );
            }
            return; // Pop-transition: target comes from stack at runtime
        }

        if !state_map.contains_key(&transition.target) {
            self.errors.push(
                ValidationError::new(
                    "E402",
                    format!(
                        "Unknown state '{}' in transition. Available states: {}",
                        transition.target,
                        self.format_available_states(state_map)
                    ),
                )
                .with_span(transition.span.clone()),
            );
        }

        // E419 / E417 (transition form) / E405 — argument-arity validation.
        //
        // Three semantically distinct sites where a transition can supply
        // arguments to a receiver, all governed by the same rule: the
        // receiver must exist (or be omitted-without-args) and the supplied
        // count must fit the receiver's declared signature, with trailing
        // defaults relaxing the lower bound.
        //
        // | Syntax                  | Receiver                        | Code  |
        // |-------------------------|---------------------------------|-------|
        // | `(args) -> $T`          | source state's `<$(...)`        | E419  |
        // | `-> (args) $T`          | target state's `$>(...)`        | E417  |
        // | `-> $T(args)`           | target state's state params     | E405  |
        //
        // For E419/E417, EventParam carries `default_value: Option<String>`,
        // so trailing defaults relax the lower bound (caller can omit those).
        // StateParam (E405) has no defaults today; check is exact-count.
        // V4 scanner enrichment (`enrich_handler_body_metadata`) populates
        // `transition.exit_args/enter_args/state_args` before this validator
        // runs — without it these checks would all be unreachable, which is
        // why these errors were documented but unfired before now.
        let target_state = state_map.get(&transition.target).copied();

        if let Some(ref exit_args_str) = transition.exit_args {
            let provided = count_args(exit_args_str);
            let transition_repr = format!("({}) -> ${}", exit_args_str, transition.target);
            match state.exit.as_ref() {
                None => self.errors.push(
                    ValidationError::new(
                        "E419",
                        format!(
                            "Exit args provided in transition `{}` but source state '{}' has no `<$()` exit handler to receive them",
                            transition_repr, state.name
                        ),
                    )
                    .with_span(transition.span.clone()),
                ),
                Some(handler) => {
                    if let Some(msg) = arity_error(
                        provided,
                        handler.params.len(),
                        required_event_params(&handler.params),
                        &format!("source state '{}' `<$()`", state.name),
                        &format!("`{}` exit args", transition_repr),
                    ) {
                        self.errors.push(
                            ValidationError::new("E419", msg)
                                .with_span(transition.span.clone()),
                        );
                    }
                }
            }
        }

        if let Some(ref enter_args_str) = transition.enter_args {
            if let Some(target) = target_state {
                let provided = count_args(enter_args_str);
                let transition_repr = format!("-> ({}) ${}", enter_args_str, transition.target);
                match target.enter.as_ref() {
                    None => self.errors.push(
                        ValidationError::new(
                            "E417",
                            format!(
                                "Enter args provided in transition `{}` but target state '{}' has no `$>()` enter handler to receive them",
                                transition_repr, target.name
                            ),
                        )
                        .with_span(transition.span.clone()),
                    ),
                    Some(handler) => {
                        if let Some(msg) = arity_error(
                            provided,
                            handler.params.len(),
                            required_event_params(&handler.params),
                            &format!("target state '{}' `$>()`", target.name),
                            &format!("`{}` enter args", transition_repr),
                        ) {
                            self.errors.push(
                                ValidationError::new("E417", msg)
                                    .with_span(transition.span.clone()),
                            );
                        }
                    }
                }
            }
        }

        if let Some(ref state_args_str) = transition.state_args {
            if let Some(target) = target_state {
                let provided = count_args(state_args_str);
                let transition_repr = format!("-> ${}({})", transition.target, state_args_str);
                let total = target.params.len();
                if total == 0 {
                    self.errors.push(
                        ValidationError::new(
                            "E405",
                            format!(
                                "State args provided in transition `{}` but target state '{}' declares no state parameters",
                                transition_repr, target.name
                            ),
                        )
                        .with_span(transition.span.clone()),
                    );
                } else if provided != total {
                    // StateParam has no `default_value` field, so the
                    // relaxation rule from E417/E419 doesn't apply here:
                    // exact-count match required.
                    self.errors.push(
                        ValidationError::new(
                            "E405",
                            format!(
                                "State args count mismatch in transition `{}`: target state '{}' declares {} state parameter{} but transition supplies {}",
                                transition_repr, target.name, total,
                                if total == 1 { "" } else { "s" }, provided
                            ),
                        )
                        .with_span(transition.span.clone()),
                    );
                }
            }
        }
    }

    /// Validate a forward statement
    pub(super) fn validate_forward(
        &mut self,
        forward: &ForwardAst,
        state: &StateAst,
        state_map: &HashMap<String, &StateAst>,
    ) {
        // E403: Forward is only valid if state has a parent
        if state.parent.is_none() {
            self.errors.push(
                ValidationError::new(
                    "E403",
                    format!(
                        "State '{}' cannot forward event '{}' - no parent state defined",
                        state.name, forward.event
                    ),
                )
                .with_span(forward.span.clone()),
            );
        } else {
            // Could validate that parent handles this event
            // For now, just check parent exists
            let Some(parent_name) = state.parent.as_ref() else {
                return;
            };
            if !state_map.contains_key(parent_name) {
                self.errors.push(
                    ValidationError::new(
                        "E403",
                        format!("Cannot forward to invalid parent state '{}'", parent_name),
                    )
                    .with_span(forward.span.clone()),
                );
            }
        }
    }

    /// Format available states for error messages
    pub(super) fn format_available_states(&self, state_map: &HashMap<String, &StateAst>) -> String {
        let mut states: Vec<String> = state_map.keys().cloned().collect();
        states.sort();
        states.join(", ")
    }
}
