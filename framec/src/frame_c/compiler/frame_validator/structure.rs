//! System-section structural validation: section ordering, duplicate
//! sections / parameters, and the "no Frame statements in actions" guard.
//!
//! These methods enforce shape-level rules a system header must satisfy
//! before semantic checks run — wrong-order or duplicated sections produce
//! E113/E114, duplicate system params E111, and any `->` / `=>` / `push$`
//! / `pop$` inside an action or operation body fires E401.

use super::{FrameValidator, ValidationError};
use crate::frame_c::compiler::frame_ast::*;
use std::collections::{HashMap, HashSet};

impl FrameValidator {
    /// E113: Validate system section order (operations:, interface:, machine:, actions:, domain:)
    pub(super) fn validate_section_order(&mut self, system: &SystemAst) {
        if system.section_order.is_empty() {
            return;
        }

        // Canonical order: Operations=0, Interface=1, Machine=2, Actions=3, Domain=4
        let mut last_idx: i32 = -1;
        for kind in &system.section_order {
            let idx = match kind {
                SystemSectionKind::Operations => 0,
                SystemSectionKind::Interface => 1,
                SystemSectionKind::Machine => 2,
                SystemSectionKind::Actions => 3,
                SystemSectionKind::Domain => 4,
            };
            if (idx as i32) < last_idx {
                self.errors.push(ValidationError::new(
                    "E113",
                    format!(
                        "System '{}' blocks out of order. Expected: operations:, interface:, machine:, actions:, domain:",
                        system.name
                    )
                ).with_span(system.span.clone()));
                break; // Only report once per system
            }
            last_idx = idx as i32;
        }
    }

    /// E111: Reject duplicate names across the system parameter list.
    /// `@@system C(a, b, a)` collides on `a` regardless of which group
    /// (domain / state-arg / enter-arg) each instance came from.
    pub(super) fn validate_duplicate_system_params(&mut self, system: &SystemAst) {
        let mut seen: HashSet<&str> = HashSet::new();
        for param in &system.params {
            if !seen.insert(param.name.as_str()) {
                self.errors.push(
                    ValidationError::new(
                        "E111",
                        format!(
                            "duplicate system parameter '{}' in system {}",
                            param.name, system.name
                        ),
                    )
                    .with_span(param.span.clone()),
                );
            }
        }
    }

    /// E114: Validate no duplicate sections in system
    pub(super) fn validate_duplicate_sections(&mut self, system: &SystemAst) {
        if let Some(dup_kind) = system.has_duplicate_sections() {
            let section_name = match dup_kind {
                SystemSectionKind::Operations => "operations:",
                SystemSectionKind::Interface => "interface:",
                SystemSectionKind::Machine => "machine:",
                SystemSectionKind::Actions => "actions:",
                SystemSectionKind::Domain => "domain:",
            };
            self.errors.push(
                ValidationError::new(
                    "E114",
                    format!(
                        "Duplicate '{}' section in system '{}'",
                        section_name, system.name
                    ),
                )
                .with_span(system.span.clone()),
            );
        }
    }

    /// E401: Validate no Frame statements in action body
    pub(super) fn validate_action_no_frame_statements(&mut self, action: &ActionAst) {
        // Actions have native bodies, but we check if the native content
        // contains Frame statement patterns (this would be caught by the scanner
        // but we can add an extra check here)
        // For now, actions are pure native, so no validation needed here
        // The validation happens during scanning/parsing
        let _ = action; // suppress unused warning
    }

    /// E401: Validate no Frame statements in operation body
    pub(super) fn validate_operation_no_frame_statements(&mut self, operation: &OperationAst) {
        // Operations have native bodies, same as actions
        let _ = operation; // suppress unused warning
    }

    /// Build a map of state names to state definitions
    pub(super) fn build_state_map<'a>(&mut self, system: &'a SystemAst) -> HashMap<String, &'a StateAst> {
        let mut map = HashMap::new();
        if let Some(machine) = &system.machine {
            for state in &machine.states {
                if map.contains_key(&state.name) {
                    self.errors.push(
                        ValidationError::new(
                            "E116",
                            format!(
                                "Duplicate state name '{}' in system '{}'",
                                state.name, system.name
                            ),
                        )
                        .with_span(state.span.clone()),
                    );
                } else {
                    map.insert(state.name.clone(), state);
                }
            }
        }
        map
    }

    /// Build a map of interface method names to definitions
    pub(super) fn build_interface_map<'a>(
        &self,
        system: &'a SystemAst,
    ) -> HashMap<String, &'a InterfaceMethod> {
        let mut map = HashMap::new();
        for method in &system.interface {
            map.insert(method.name.clone(), method);
        }
        map
    }

}
