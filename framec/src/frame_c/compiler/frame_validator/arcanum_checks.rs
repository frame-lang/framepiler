//! Arcanum-aware semantic validation.
//!
//! The [`Arcanum`] is the compiler's authoritative state-resolution table.
//! Once it has been built, transition targets become statically verifiable
//! (does the state exist? does its parameter count match the call site?).
//!
//! These methods replace the structural transition checks in
//! [`super::transitions`] with their stronger Arcanum-backed equivalents:
//! E402 (unknown state) and E405 (wrong arity).

use super::{FrameValidator, ValidationError};
use crate::frame_c::compiler::arcanum::Arcanum;
use crate::frame_c::compiler::frame_ast::*;

impl FrameValidator {

    /// Additional validation using the Arcanum
    pub(super) fn validate_system_with_arcanum(&mut self, system: &SystemAst, arcanum: &Arcanum) {
        // E402 enhanced: Validate transitions using Arcanum
        if let Some(machine) = &system.machine {
            for state in &machine.states {
                self.validate_state_transitions_with_arcanum(&system.name, state, arcanum);
            }
        }
    }

    /// Validate state transitions using Arcanum's state resolution
    pub(super) fn validate_state_transitions_with_arcanum(
        &mut self,
        system_name: &str,
        state: &StateAst,
        arcanum: &Arcanum,
    ) {
        // Validate handlers
        for handler in &state.handlers {
            for stmt in &handler.body.statements {
                if let Statement::Transition(trans) = stmt {
                    self.validate_transition_with_arcanum(system_name, trans, arcanum);
                }
            }
        }

        // Validate enter handler
        if let Some(enter) = &state.enter {
            for stmt in &enter.body.statements {
                if let Statement::Transition(trans) = stmt {
                    self.validate_transition_with_arcanum(system_name, trans, arcanum);
                }
            }
        }

        // Validate exit handler
        if let Some(exit) = &state.exit {
            for stmt in &exit.body.statements {
                if let Statement::Transition(trans) = stmt {
                    self.validate_transition_with_arcanum(system_name, trans, arcanum);
                }
            }
        }
    }

    /// Validate a transition using Arcanum's state resolution
    pub(super) fn validate_transition_with_arcanum(
        &mut self,
        system_name: &str,
        trans: &TransitionAst,
        arcanum: &Arcanum,
    ) {
        // Skip validation for pop-transition marker $$[-]
        if trans.target == "pop$" {
            return; // Pop-transition: target comes from stack at runtime
        }

        // Use Arcanum's validate_transition which includes "did you mean" suggestions
        if let Err(msg) = arcanum.validate_transition(system_name, &trans.target) {
            // Only add if not already reported by basic validation
            if !self
                .errors
                .iter()
                .any(|e| e.code == "E402" && e.span.as_ref() == Some(&trans.span))
            {
                self.errors
                    .push(ValidationError::new("E402", msg).with_span(trans.span.clone()));
            }
        } else {
            // State exists, check transition argument arity against STATE PARAMS.
            // Skip when args are NativeExpr blobs — the V4 lexer conflates
            // `-> (enter_args) $State` and `-> $State(state_args)` into the
            // same token shape, so we can't distinguish them. Arity checking
            // for these cases defers to the target language compiler.
            let has_native_args = trans
                .args
                .iter()
                .any(|a| matches!(a, Expression::NativeExpr(_)));
            if !has_native_args {
                let args_count = trans.args.len();
                if let Some(expected) = arcanum.get_state_param_count(system_name, &trans.target) {
                    if expected != args_count
                        && !self
                            .errors
                            .iter()
                            .any(|e| e.code == "E405" && e.span.as_ref() == Some(&trans.span))
                    {
                        self.errors.push(
                            ValidationError::new(
                                "E405",
                                format!(
                                    "State '{}' expects {} parameters but {} provided",
                                    trans.target, expected, args_count
                                ),
                            )
                            .with_span(trans.span.clone()),
                        );
                    }
                }
            }
        }
    }

}
