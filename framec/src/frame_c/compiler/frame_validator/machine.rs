//! Machine-walk validation: HSM cycle detection, top-level machine walk
//! (state-by-state, handler-by-handler), and reachability from the start
//! state.
//!
//! `validate_machine` is the entry point — it iterates over states and
//! delegates per-state checks to [`super::transitions::FrameValidator::validate_state`].
//! `validate_hsm_cycles` catches E413 (cyclic parent chain); `validate_reachable_states`
//! emits W414 for any state that can't be reached from the start state.

use super::{FrameValidator, ValidationError};
use crate::frame_c::compiler::frame_ast::*;
use std::collections::{HashMap, HashSet};

impl FrameValidator {
    /// E413: Detect circular parent chains in HSM hierarchy
    pub(super) fn validate_hsm_cycles(&mut self, _system: &SystemAst, state_map: &HashMap<String, &StateAst>) {
        for (state_name, state) in state_map {
            if state.parent.is_none() {
                continue;
            }
            let mut visited = HashSet::new();
            visited.insert(state_name.clone());
            let mut current = state.parent.as_deref();
            while let Some(parent_name) = current {
                if !visited.insert(parent_name.to_string()) {
                    // Cycle detected
                    self.errors.push(
                        ValidationError::new(
                            "E413",
                            format!(
                            "HSM cycle detected: state '{}' has circular parent chain through '{}'",
                            state_name, parent_name
                        ),
                        )
                        .with_span(state.span.clone()),
                    );
                    break;
                }
                current = state_map.get(parent_name).and_then(|s| s.parent.as_deref());
            }
        }
    }

    /// Build a set of action names
    pub(super) fn build_action_set(&self, system: &SystemAst) -> HashSet<String> {
        system.actions.iter().map(|a| a.name.clone()).collect()
    }

    /// Build a set of operation names
    pub(super) fn build_operation_set(&self, system: &SystemAst) -> HashSet<String> {
        system.operations.iter().map(|o| o.name.clone()).collect()
    }

    /// Validate a machine
    pub(super) fn validate_machine(
        &mut self,
        machine: &MachineAst,
        state_map: &HashMap<String, &StateAst>,
        interface_methods: &HashMap<String, &InterfaceMethod>,
        _actions: &HashSet<String>,
        _operations: &HashSet<String>,
        system_name: &str,
    ) {
        for state in &machine.states {
            self.validate_state(state, state_map, interface_methods, _actions, _operations);
        }
        self.validate_reachable_states(system_name, machine, state_map);
    }

    /// W414: warn when a state is not reachable from the start state via
    /// any direct `-> $State` transition in any handler / enter / exit
    /// body. BFS from machine.states[0] (Frame's start-state convention)
    /// over Transition statements only — `pop$` returns are treated as
    /// non-transitions (the destination is wherever the runtime stack
    /// last held, not a static target). HSM parents of reachable states
    /// are also considered reachable: the runtime visits a parent on
    /// every enter/exit cascade through its child even though no direct
    /// `-> $Parent` transition exists. States only reached through
    /// stack pop/push from outside the BFS frontier are best-effort
    /// flagged; the warning is advisory, not a build error.
    pub(super) fn validate_reachable_states(
        &mut self,
        system_name: &str,
        machine: &MachineAst,
        state_map: &HashMap<String, &StateAst>,
    ) {
        if machine.states.is_empty() {
            return;
        }
        let start_state = &machine.states[0].name;

        let mut reachable: HashSet<String> = HashSet::new();
        let mut queue: Vec<String> = vec![start_state.clone()];
        reachable.insert(start_state.clone());

        let visit_body =
            |body: &HandlerBody, reachable: &mut HashSet<String>, queue: &mut Vec<String>| {
                for stmt in &body.statements {
                    if let Statement::Transition(trans) = stmt {
                        if trans.target != "pop$" && reachable.insert(trans.target.clone()) {
                            queue.push(trans.target.clone());
                        }
                    }
                }
            };

        while let Some(current) = queue.pop() {
            if let Some(state) = state_map.get(&current) {
                for handler in &state.handlers {
                    visit_body(&handler.body, &mut reachable, &mut queue);
                }
                if let Some(enter) = &state.enter {
                    visit_body(&enter.body, &mut reachable, &mut queue);
                }
                if let Some(exit) = &state.exit {
                    visit_body(&exit.body, &mut reachable, &mut queue);
                }
                // HSM: walk the parent chain. Every ancestor of a
                // reachable state is itself reachable through enter/
                // exit cascades — no direct `-> $Parent` transition
                // is required.
                let mut ancestor = state.parent.clone();
                while let Some(parent_name) = ancestor {
                    if !reachable.insert(parent_name.clone()) {
                        break;
                    }
                    queue.push(parent_name.clone());
                    ancestor = state_map.get(&parent_name).and_then(|s| s.parent.clone());
                }
            }
        }

        for state in &machine.states {
            if !reachable.contains(&state.name) {
                self.warnings.push(
                    ValidationError::new(
                        "W414",
                        format!(
                            "State '{}' is not reachable from start state '{}' in system '{}'",
                            state.name, start_state, system_name
                        ),
                    )
                    .with_span(state.span.clone()),
                );
            }
        }
    }
}
