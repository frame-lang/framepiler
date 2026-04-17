//! Semantic validation pass for Frame V4
//!
//! Validates Frame semantics using the Arcanum symbol table:
//! - E400: Transition must be last statement in block
//! - E401: Frame statements not allowed in actions/operations
//! - E402: Unknown state in transition
//! - E403: Invalid parent forwards in HSM
//! - E404: Handler body must be inside a state block
//! - E405: State parameter arity mismatch
//! - E406: Invalid interface method calls
//! - E413: Cyclic HSM parent relationship
//! - W414: Unreachable state from start state

use crate::frame_c::compiler::arcanum::Arcanum;
use crate::frame_c::compiler::frame_ast::{
    Expression, ForwardAst, FrameAst, HandlerAst, HandlerBody, InterfaceMethod, StateAst,
    Statement, SystemAst, TransitionAst,
};
use crate::frame_c::compiler::validation::pass::{ValidationContext, ValidationPass};
use crate::frame_c::compiler::validation::types::ValidationIssue;
use std::collections::{HashMap, HashSet};

/// Semantic validation pass
///
/// Performs semantic validation using the Arcanum symbol table
/// to cross-reference Frame declarations.
pub struct SemanticPass;

impl ValidationPass for SemanticPass {
    fn name(&self) -> &'static str {
        "semantic"
    }

    fn run(
        &self,
        ast: &FrameAst,
        arcanum: &Arcanum,
        ctx: &mut ValidationContext,
    ) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();

        match ast {
            FrameAst::System(system) => {
                ctx.system_name = Some(system.name.clone());
                self.validate_system(system, arcanum, &mut issues);
            }
            FrameAst::Module(module) => {
                for system in &module.systems {
                    ctx.system_name = Some(system.name.clone());
                    self.validate_system(system, arcanum, &mut issues);
                }
            }
        }

        issues
    }
}

impl SemanticPass {
    /// Validate a system
    fn validate_system(
        &self,
        system: &SystemAst,
        arcanum: &Arcanum,
        issues: &mut Vec<ValidationIssue>,
    ) {
        // Build lookup tables
        let state_map = self.build_state_map(system);
        let interface_methods = self.build_interface_map(system);
        let actions = self.build_action_set(system);
        let operations = self.build_operation_set(system);

        // Validate machine if present
        if let Some(machine) = &system.machine {
            // E413: Check for cyclic parent relationships
            self.validate_hsm_acyclic(&system.name, &state_map, issues);

            // W414: Check for unreachable states
            self.validate_reachable_states(&system.name, machine, &state_map, issues);

            for state in &machine.states {
                self.validate_state(
                    &system.name,
                    state,
                    &state_map,
                    &interface_methods,
                    arcanum,
                    issues,
                );
            }
        }

        // E401 would be validated here if we tracked Frame statements in actions/operations
        // Currently the parser prevents this by not parsing Frame statements in those contexts
        let _ = (actions, operations);
    }

    /// Build map of state names to states
    fn build_state_map<'a>(&self, system: &'a SystemAst) -> HashMap<String, &'a StateAst> {
        let mut map = HashMap::new();
        if let Some(machine) = &system.machine {
            for state in &machine.states {
                map.insert(state.name.clone(), state);
            }
        }
        map
    }

    /// Build map of interface methods
    fn build_interface_map<'a>(
        &self,
        system: &'a SystemAst,
    ) -> HashMap<String, &'a InterfaceMethod> {
        let mut map = HashMap::new();
        for method in &system.interface {
            map.insert(method.name.clone(), method);
        }
        map
    }

    /// Build set of action names
    fn build_action_set(&self, system: &SystemAst) -> HashSet<String> {
        system.actions.iter().map(|a| a.name.clone()).collect()
    }

    /// Build set of operation names
    fn build_operation_set(&self, system: &SystemAst) -> HashSet<String> {
        system.operations.iter().map(|o| o.name.clone()).collect()
    }

    /// Validate a state
    fn validate_state(
        &self,
        system_name: &str,
        state: &StateAst,
        state_map: &HashMap<String, &StateAst>,
        interface_methods: &HashMap<String, &InterfaceMethod>,
        arcanum: &Arcanum,
        issues: &mut Vec<ValidationIssue>,
    ) {
        // E403: Validate parent state exists for HSM
        if let Some(parent_name) = &state.parent {
            if !state_map.contains_key(parent_name) {
                issues.push(
                    ValidationIssue::error(
                        "E403",
                        format!(
                            "State '{}' has invalid parent '{}'",
                            state.name, parent_name
                        ),
                    )
                    .with_span(state.span.clone())
                    .with_note(format!(
                        "Available states: {}",
                        self.format_available_states(state_map)
                    ))
                    .with_fix(format!(
                        "Change parent to an existing state or remove the parent reference"
                    )),
                );
            }
        }

        // Validate handlers
        for handler in &state.handlers {
            self.validate_handler(
                system_name,
                state,
                handler,
                state_map,
                interface_methods,
                arcanum,
                issues,
            );
        }

        // Validate enter handler
        if let Some(enter) = &state.enter {
            self.validate_handler_body(system_name, state, &enter.body, state_map, arcanum, issues);
        }

        // Validate exit handler
        if let Some(exit) = &state.exit {
            self.validate_handler_body(system_name, state, &exit.body, state_map, arcanum, issues);
        }
    }

    /// Validate a handler
    fn validate_handler(
        &self,
        system_name: &str,
        state: &StateAst,
        handler: &HandlerAst,
        state_map: &HashMap<String, &StateAst>,
        interface_methods: &HashMap<String, &InterfaceMethod>,
        arcanum: &Arcanum,
        issues: &mut Vec<ValidationIssue>,
    ) {
        // E406: Check if handler corresponds to interface method
        if let Some(method) = interface_methods.get(&handler.event) {
            if handler.params.len() != method.params.len() {
                issues.push(
                    ValidationIssue::error(
                        "E406",
                        format!(
                            "Handler '{}' in state '{}' has {} parameters but interface method expects {}",
                            handler.event,
                            state.name,
                            handler.params.len(),
                            method.params.len()
                        )
                    )
                    .with_span(handler.span.clone())
                    .with_fix(format!(
                        "Update handler to match interface signature: {}({} params)",
                        method.name, method.params.len()
                    ))
                );
            }
        }

        self.validate_handler_body(
            system_name,
            state,
            &handler.body,
            state_map,
            arcanum,
            issues,
        );
    }

    /// Validate handler body statements
    fn validate_handler_body(
        &self,
        system_name: &str,
        state: &StateAst,
        body: &HandlerBody,
        state_map: &HashMap<String, &StateAst>,
        arcanum: &Arcanum,
        issues: &mut Vec<ValidationIssue>,
    ) {
        // E400: Check that terminal statements are last
        self.validate_terminal_last(body, issues);

        for statement in &body.statements {
            match statement {
                Statement::Transition(transition) => {
                    self.validate_transition(
                        system_name,
                        state,
                        transition,
                        state_map,
                        arcanum,
                        issues,
                    );
                }
                Statement::Forward(forward) => {
                    self.validate_forward(state, forward, state_map, issues);
                }
                _ => {
                    // Other statements don't need validation yet
                }
            }
        }
    }

    /// E400: Validate terminal statements are last
    fn validate_terminal_last(&self, body: &HandlerBody, issues: &mut Vec<ValidationIssue>) {
        let statements = &body.statements;
        if statements.is_empty() {
            return;
        }

        // Find index of last terminal statement
        let mut terminal_index: Option<usize> = None;
        for (i, stmt) in statements.iter().enumerate() {
            if self.is_terminal_statement(stmt) {
                terminal_index = Some(i);
            }
        }

        // Check if terminal statement isn't last
        if let Some(idx) = terminal_index {
            let last_idx = statements.len() - 1;
            if idx != last_idx {
                // Check if remaining statements are non-trivial Frame statements.
                // NativeCode is always trivial — Frame is a preprocessor and cannot
                // reason about native control flow (if/else, loops, switch, etc.).
                let has_non_trivial_after = statements[idx + 1..].iter().any(|s| {
                    matches!(
                        s,
                        Statement::Transition(_)
                            | Statement::Forward(_)
                            | Statement::StackPush(_)
                            | Statement::StackPop(_)
                    )
                });
                if has_non_trivial_after {
                    let span = match &statements[idx] {
                        Statement::Transition(t) => t.span.clone(),
                        Statement::Forward(f) => f.span.clone(),
                        _ => body.span.clone(),
                    };
                    issues.push(
                        ValidationIssue::error(
                            "E400",
                            "Transition/forward must be the last statement in its containing block",
                        )
                        .with_span(span)
                        .with_note("Code after a transition is unreachable")
                        .with_fix(
                            "Move the transition to the end of the block or remove code after it",
                        ),
                    );
                }
            }
        }
    }

    /// Check if statement is terminal
    fn is_terminal_statement(&self, stmt: &Statement) -> bool {
        matches!(stmt, Statement::Transition(_) | Statement::Forward(_))
    }

    /// Validate transition using basic state map
    fn validate_transition(
        &self,
        system_name: &str,
        _state: &StateAst,
        transition: &TransitionAst,
        state_map: &HashMap<String, &StateAst>,
        arcanum: &Arcanum,
        issues: &mut Vec<ValidationIssue>,
    ) {
        // E402: Check target state exists (basic check)
        // Skip validation for pop-transition marker $$[-]
        if transition.target == "pop$" {
            // Pop-transition: target comes from stack at runtime
            return;
        }

        if !state_map.contains_key(&transition.target) {
            // Use Arcanum for "did you mean" suggestions
            let suggestion = arcanum
                .validate_transition(system_name, &transition.target)
                .err()
                .unwrap_or_else(|| format!("Unknown state '{}'", transition.target));

            issues.push(
                ValidationIssue::error("E402", suggestion)
                    .with_span(transition.span.clone())
                    .with_note(format!(
                        "Available states: {}",
                        self.format_available_states(state_map)
                    ))
                    .with_fix(format!(
                        "Add state ${}{{}} or correct the state name",
                        transition.target
                    )),
            );
        } else {
            // E405: Check STATE PARAMETER arity. Skip for NativeExpr blobs
            // (V4 lexer conflates enter_args and state_args).
            let has_native_args = transition
                .args
                .iter()
                .any(|a| matches!(a, Expression::NativeExpr(_)));
            if has_native_args {
                return;
            }
            let Some(target_state) = state_map.get(&transition.target) else {
                return;
            };
            if target_state.params.len() != transition.args.len() {
                issues.push(
                    ValidationIssue::error(
                        "E405",
                        format!(
                            "State '{}' expects {} parameters but {} provided",
                            transition.target,
                            target_state.params.len(),
                            transition.args.len()
                        ),
                    )
                    .with_span(transition.span.clone())
                    .with_note(format!(
                        "State '{}' parameters: {}",
                        transition.target,
                        self.format_params(target_state)
                    ))
                    .with_fix(format!(
                        "Provide {} argument(s) to the transition",
                        target_state.params.len()
                    )),
                );
            }
        }
    }

    /// Validate forward statement
    fn validate_forward(
        &self,
        state: &StateAst,
        forward: &ForwardAst,
        state_map: &HashMap<String, &StateAst>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        // E403: Forward requires parent
        if state.parent.is_none() {
            issues.push(
                ValidationIssue::error(
                    "E403",
                    format!(
                        "State '{}' cannot forward event '{}' - no parent state defined",
                        state.name, forward.event
                    ),
                )
                .with_span(forward.span.clone())
                .with_note("Forward (>>) is only valid in hierarchical state machines")
                .with_fix(format!(
                    "Add a parent state using '${}' => $ParentState {{ }}",
                    state.name
                )),
            );
        } else {
            // Check parent exists
            let Some(parent_name) = state.parent.as_ref() else {
                return;
            };
            if !state_map.contains_key(parent_name) {
                issues.push(
                    ValidationIssue::error(
                        "E403",
                        format!("Cannot forward to invalid parent state '{}'", parent_name),
                    )
                    .with_span(forward.span.clone())
                    .with_fix("Correct the parent state name"),
                );
            }
        }
    }

    /// Format available states for error messages
    fn format_available_states(&self, state_map: &HashMap<String, &StateAst>) -> String {
        let mut states: Vec<String> = state_map.keys().cloned().collect();
        states.sort();
        if states.is_empty() {
            "(none)".to_string()
        } else {
            states.join(", ")
        }
    }

    /// Format state parameters
    fn format_params(&self, state: &StateAst) -> String {
        if state.params.is_empty() {
            "(none)".to_string()
        } else {
            state
                .params
                .iter()
                .map(|p| format!("{}: {:?}", p.name, p.param_type))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// E413: Validate no cyclic parent relationships in HSM
    fn validate_hsm_acyclic(
        &self,
        system_name: &str,
        state_map: &HashMap<String, &StateAst>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        // For each state with a parent, follow the chain and check for cycles
        for (state_name, state) in state_map {
            if state.parent.is_some() {
                let mut visited = HashSet::new();
                visited.insert(state_name.clone());
                let mut current = state.parent.as_ref();

                while let Some(parent_name) = current {
                    if visited.contains(parent_name) {
                        // Found a cycle
                        issues.push(
                            ValidationIssue::error(
                                "E413",
                                format!(
                                    "Cyclic parent relationship detected: '{}' -> '{}'",
                                    state_name, parent_name
                                )
                            )
                            .with_span(state.span.clone())
                            .with_note(format!(
                                "State '{}' cannot be its own ancestor in system '{}'",
                                state_name, system_name
                            ))
                            .with_fix("Remove or change one of the parent relationships to break the cycle")
                        );
                        break;
                    }
                    visited.insert(parent_name.clone());
                    current = state_map.get(parent_name).and_then(|s| s.parent.as_ref());
                }
            }
        }
    }

    /// W414: Validate all states are reachable from start state
    fn validate_reachable_states(
        &self,
        system_name: &str,
        machine: &crate::frame_c::compiler::frame_ast::MachineAst,
        state_map: &HashMap<String, &StateAst>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        if machine.states.is_empty() {
            return;
        }

        // Start state is the first state
        let start_state = &machine.states[0].name;

        // BFS to find all reachable states
        let mut reachable = HashSet::new();
        let mut queue = vec![start_state.clone()];
        reachable.insert(start_state.clone());

        while let Some(current) = queue.pop() {
            if let Some(state) = state_map.get(&current) {
                // Find all transitions from this state
                for handler in &state.handlers {
                    for stmt in &handler.body.statements {
                        if let Statement::Transition(trans) = stmt {
                            if trans.target != "pop$" && !reachable.contains(&trans.target) {
                                reachable.insert(trans.target.clone());
                                queue.push(trans.target.clone());
                            }
                        }
                    }
                }
                // Check enter handler too
                if let Some(enter) = &state.enter {
                    for stmt in &enter.body.statements {
                        if let Statement::Transition(trans) = stmt {
                            if trans.target != "pop$" && !reachable.contains(&trans.target) {
                                reachable.insert(trans.target.clone());
                                queue.push(trans.target.clone());
                            }
                        }
                    }
                }
                // Check exit handler
                if let Some(exit) = &state.exit {
                    for stmt in &exit.body.statements {
                        if let Statement::Transition(trans) = stmt {
                            if trans.target != "pop$" && !reachable.contains(&trans.target) {
                                reachable.insert(trans.target.clone());
                                queue.push(trans.target.clone());
                            }
                        }
                    }
                }
            }
        }

        // Report unreachable states as warnings
        for state in &machine.states {
            if !reachable.contains(&state.name) {
                issues.push(
                    ValidationIssue::warning(
                        "W414",
                        format!(
                            "State '{}' is not reachable from start state '{}' in system '{}'",
                            state.name, start_state, system_name
                        ),
                    )
                    .with_span(state.span.clone())
                    .with_note("This state can never be entered during normal execution")
                    .with_fix("Add a transition to this state or remove it if unused"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_c::compiler::arcanum::build_arcanum_from_frame_ast;
    use crate::frame_c::compiler::frame_ast::{ModuleAst, Span};
    use crate::frame_c::compiler::{pipeline_parser, segmenter};
    use crate::frame_c::visitors::TargetLanguage;

    fn make_context() -> ValidationContext<'static> {
        static CONFIG: crate::frame_c::compiler::validation::types::ValidationConfig =
            crate::frame_c::compiler::validation::types::ValidationConfig {
                warnings_as_errors: false,
                suppress: Vec::new(),
                verbose: false,
                max_errors: 0,
            };
        ValidationContext::new(&CONFIG)
    }

    /// Build a FrameAst::Module from raw source via segmenter +
    /// pipeline_parser. Used to feed `SemanticPass.run` directly.
    fn parse_module_v4(source: &str) -> FrameAst {
        let bytes = source.as_bytes();
        let source_map = segmenter::segment_source(bytes, TargetLanguage::Python3)
            .expect("segmenter failed in test");

        let mut systems = Vec::new();
        for segment in &source_map.segments {
            if let segmenter::Segment::System {
                name,
                body_span,
                header_params_span,
                ..
            } = segment
            {
                let span = Span::new(body_span.start, body_span.end);
                let mut system = pipeline_parser::parse_system(
                    &source_map.source,
                    name.clone(),
                    span,
                    TargetLanguage::Python3,
                )
                .expect("parse_system failed in test");

                if let Some(hp_span) = header_params_span {
                    let hp = Span::new(hp_span.start, hp_span.end);
                    if let Ok(params) =
                        pipeline_parser::parse_system_header_params(&source_map.source, hp)
                    {
                        system.params = params;
                    }
                }

                systems.push(system);
            }
        }

        FrameAst::Module(ModuleAst {
            name: String::new(),
            systems,
            imports: Vec::new(),
            span: Span::new(0, 0),
        })
    }

    #[test]
    fn test_e402_unknown_state() {
        let source = r#"
@@system Test {
    machine:
        $Start {
            go() { -> $Unknown() }
        }
}"#;

        let ast = parse_module_v4(source);
        let arcanum = build_arcanum_from_frame_ast(&ast);
        let mut ctx = make_context();

        let pass = SemanticPass;
        let issues = pass.run(&ast, &arcanum, &mut ctx);

        assert!(issues.iter().any(|i| i.code == "E402"));
    }

    #[test]
    fn test_e403_invalid_parent() {
        let source = r#"
@@system Test {
    machine:
        $Child => $InvalidParent {
            event() { }
        }
        $ActualParent { }
}"#;

        let ast = parse_module_v4(source);
        let arcanum = build_arcanum_from_frame_ast(&ast);
        let mut ctx = make_context();

        let pass = SemanticPass;
        let issues = pass.run(&ast, &arcanum, &mut ctx);

        assert!(issues.iter().any(|i| i.code == "E403"));
    }

    #[test]
    fn test_e405_state_param_mismatch_deferred() {
        // V4 lexer conflates enter_args and state_args into the same
        // NativeExpr blob, so E405 is skipped for NativeExpr args.
        let source = r#"
@@system Test {
    machine:
        $Start {
            go() { -> $Target(1, 2, 3) }
        }
        $Target(x: int) { }
}"#;

        let ast = parse_module_v4(source);
        let arcanum = build_arcanum_from_frame_ast(&ast);
        let mut ctx = make_context();

        let pass = SemanticPass;
        let issues = pass.run(&ast, &arcanum, &mut ctx);

        // No E405 — deferred to target language compiler
        assert!(
            !issues.iter().any(|i| i.code == "E405"),
            "V4 defers NativeExpr arity; got unexpected E405: {:?}",
            issues
        );
    }

    #[test]
    fn test_e405_state_no_params_deferred() {
        let source = r#"
@@system Test {
    machine:
        $Start {
            go() { -> $Target(1, 2, 3) }
        }
        $Target { }
}"#;

        let ast = parse_module_v4(source);
        let arcanum = build_arcanum_from_frame_ast(&ast);
        let mut ctx = make_context();

        let pass = SemanticPass;
        let issues = pass.run(&ast, &arcanum, &mut ctx);

        assert!(
            !issues.iter().any(|i| i.code == "E405"),
            "V4 defers NativeExpr arity; got unexpected E405: {:?}",
            issues
        );
    }

    #[test]
    fn test_valid_system() {
        let source = r#"
@@system Valid {
    machine:
        $Idle {
            start() { -> $Active() }
        }
        $Active {
            stop() { -> $Idle() }
        }
}"#;

        let ast = parse_module_v4(source);
        let arcanum = build_arcanum_from_frame_ast(&ast);
        let mut ctx = make_context();

        let pass = SemanticPass;
        let issues = pass.run(&ast, &arcanum, &mut ctx);

        assert!(
            issues.is_empty(),
            "Expected no issues but got: {:?}",
            issues
        );
    }

    #[test]
    fn test_w414_unreachable_state() {
        // State $Orphan has no transition leading to it
        let source = r#"
@@system Test {
    machine:
        $Start {
            go() { -> $Active() }
        }
        $Active {
            back() { -> $Start() }
        }
        $Orphan {
            event() { }
        }
}"#;

        let ast = parse_module_v4(source);
        let arcanum = build_arcanum_from_frame_ast(&ast);
        let mut ctx = make_context();

        let pass = SemanticPass;
        let issues = pass.run(&ast, &arcanum, &mut ctx);

        // Should have a warning about Orphan being unreachable
        assert!(
            issues
                .iter()
                .any(|i| i.code == "W414" && i.message.contains("Orphan")),
            "Expected W414 warning for unreachable state, got: {:?}",
            issues
        );
    }

    #[test]
    fn test_e413_cyclic_parent() {
        // $A => $B, $B => $A creates a cycle
        let source = r#"
@@system Test {
    machine:
        $A => $B {
            event() { }
        }
        $B => $A {
            event() { }
        }
}"#;

        let ast = parse_module_v4(source);
        let arcanum = build_arcanum_from_frame_ast(&ast);
        let mut ctx = make_context();

        let pass = SemanticPass;
        let issues = pass.run(&ast, &arcanum, &mut ctx);

        // Should have an error about cyclic parent
        assert!(
            issues.iter().any(|i| i.code == "E413"),
            "Expected E413 error for cyclic parent, got: {:?}",
            issues
        );
    }
}
