//! Frame semantic validator using the Frame AST
//!
//! This module performs semantic validation on the Frame AST, checking for:
//!
//! ## Structural Errors (E1xx)
//! - E111: Duplicate system parameter
//! - E112: Missing '{' after state header (parser handles)
//! - E113: System blocks out of order
//! - E114: Duplicate section block in system
//! - E115: Multiple 'fn main' functions in module
//! - E116: Duplicate state name in machine
//! - E117: Duplicate handler in state
//!
//! ## Semantic Errors (E4xx)
//! - E400: Transition must be last statement in block
//! - E401: Frame statements not allowed in actions/operations
//! - E402: Unknown state in transition
//! - E403: Invalid parent forwards in HSM
//! - E116: Duplicate state name in machine
//! - E405: State parameter arity mismatch (-> $State(args)). Receiver
//!   is the target state's state-param list; defaults are not currently
//!   supported on `StateParam`, so the check is exact.
//! - E406: Interface handler parameter count mismatch
//! - E407: Frame statement (`->`, `=>`, `push$`, `pop$`) inside a nested
//!   function scope (closure / lambda). Detected by the unified scanner
//!   via each backend's `skip_nested_scope` implementation; the error is
//!   raised from the scanner rather than the post-parse validator
//!   because the scope detection has to happen during byte-walking.
//! - E410: Duplicate state variable in state ($.varName)
//! - E413: Cyclic HSM parent relationship
//! - E416: Start params must match start state params
//! - E417: Enter args must match $>() handler params. Two forms — system
//!   level (`@@system Foo($>(...))` against the start state's `$>()`)
//!   and transition level (`-> (args) $State` against the target
//!   state's `$>()`). Both fire E417.
//! - E418: Domain param has no matching variable
//! - E419: Exit args must match `<$()` handler params ((args) -> $State).
//!   Receiver is the source state's `<$()`.
//!
//! E405/E417-transition/E419 all enforce the same general rule: a
//! transition that supplies args requires a receiver that can take
//! them. EventParam-backed receivers (E417, E419) honor trailing
//! `default_value` to relax the lower bound; StateParam-backed
//! receivers (E405) require exact-count match because `StateParam`
//! has no defaults today. All three checks are reachable in v4
//! because `enrich_handler_body_metadata` populates
//! `transition.{exit,enter,state}_args` from the unified scanner
//! before validation runs.
//! - E420: `static` is only valid on operations (not interface methods or actions)
//! - E421: `@@:system.state` not allowed in static operations (no self/compartment access)
//! - E410: Duplicate state variable in same state
//!
//! ## Domain Errors (E6xx)
//! - E605: Static target requires explicit type on domain field
//! - E613: Domain field name shadows a system parameter
//! - E614: Duplicate domain field name
//! - E615: Assignment to const domain field in handler body
//!
//! ## Pop Transition Errors (E6xx)
//! - E607: State arguments on pop$ (popped compartment carries its own)
//!
//! ## Target-specific Errors (E5xx)
//! - E501: Interface method name collides with reserved target-language method (GDScript)
//!
//! ## Target-specific Warnings (W5xx)
//! - W501: System name shadows a TypeScript global (Worker, Buffer, Map, ...)
//!
//! ## Warnings (W4xx)
//! - W414: Unreachable state from start state
//! - W415: `return <expr>` in event handler — value is silently lost
//!
//! ## RFC-0012 amendment hard-cut (E814)
//! - E814: bare `@@[persist]` form rejected — declare `@@[save]` /
//!   `@@[load]` operation attributes per RFC-0012 amendment.
//!
//! ## Compartment Field Mapping (Canonical 6-field model)
//!
//! | Syntax                  | Field           | Error Code |
//! |-------------------------|-----------------|------------|
//! | `-> $State(args)`       | state_args      | E405       |
//! | `-> (args) $State`      | enter_args      | E417       |
//! | `(args) -> $State`      | exit_args       | E419       |
//! | `-> => $State`          | forward_event   | E410       |
//! | `$.varName`             | state_vars      | E410       |

use super::arcanum::Arcanum;
use super::frame_ast::*;
use super::native_region_scanner::{FrameSegmentKind, Region, SegmentMetadata};
use crate::frame_c::compiler::codegen::frame_expansion::get_native_scanner;
use std::collections::{HashMap, HashSet};

/// Validation error with error code and message
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
}

impl ValidationError {
    pub fn new(code: &str, message: String) -> Self {
        Self {
            code: code.to_string(),
            message,
            span: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// Frame AST validator
pub struct FrameValidator {
    errors: Vec<ValidationError>,
    /// Non-fatal validation diagnostics (W-prefixed codes). Currently
    /// populated only by `validate_target_specific` for soft target
    /// concerns like TypeScript built-in shadowing. The pipeline
    /// harvests these into `CompileResult.warnings` so the CLI can
    /// surface them to the user without failing the build.
    warnings: Vec<ValidationError>,
}

impl FrameValidator {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Drain the accumulated warnings out of the validator. Used by
    /// the pipeline to attach them to the `CompileResult`. After this
    /// returns, the validator's internal warning list is empty.
    pub fn take_warnings(&mut self) -> Vec<ValidationError> {
        std::mem::take(&mut self.warnings)
    }

    /// Validate a Frame AST
    pub fn validate(&mut self, ast: &FrameAst) -> Result<(), Vec<ValidationError>> {
        match ast {
            FrameAst::System(system) => self.validate_system(system),
            FrameAst::Module(module) => {
                for system in &module.systems {
                    self.validate_system(system);
                }
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    /// Validate a Frame AST using the enhanced Arcanum
    ///
    /// This is the preferred validation method when the Arcanum has been built.
    /// It uses the Arcanum's scope resolution for more thorough validation.
    pub fn validate_with_arcanum(
        &mut self,
        ast: &FrameAst,
        arcanum: &Arcanum,
    ) -> Result<(), Vec<ValidationError>> {
        match ast {
            FrameAst::System(system) => {
                self.validate_system(system);
                self.validate_system_with_arcanum(system, arcanum);
            }
            FrameAst::Module(module) => {
                for system in &module.systems {
                    self.validate_system(system);
                    self.validate_system_with_arcanum(system, arcanum);
                }
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    /// Validate target-specific concerns that the language-agnostic
    /// passes can't catch — e.g. interface method names that collide
    /// with reserved methods on the target language's base object
    /// (`Object.get`/`Object.set` in GDScript).
    ///
    /// This is invoked from the pipeline AFTER `validate_with_arcanum`
    /// so the general structural/semantic checks always run first. The
    /// `target` parameter is the canonical `visitors::TargetLanguage`
    /// (the legacy `frame_ast::TargetLanguage` only has 8 variants and
    /// doesn't include GDScript).
    /// Validate @@:self.method() calls and bare @@: references using the scanner.
    /// The scanner identifies Frame segments correctly (handling comments, strings),
    /// so the validator walks structured output instead of byte-scanning.
    /// RFC-0014 module-level pass: ensure that `.fgd` files with 2+
    /// `@@system` declarations include exactly one `@@[main]` to mark
    /// the file's primary system.
    ///
    /// - **E805**: multi-system module with zero `@@[main]`. The fix
    ///   is for the user to add `@@[main]` above the system that
    ///   callers expect to instantiate via the module's primary entry
    ///   point (e.g. `preload(file).new()` for GDScript).
    /// - **E806**: multi-system module with multiple `@@[main]`.
    ///   Only one system can occupy the script-level slot per file.
    ///
    /// Single-system files are exempt — the lone system is implicitly
    /// the primary; explicit `@@[main]` is allowed (redundant but
    /// harmless) for symmetry with multi-system corpora.
    ///
    /// Today this only affects targets where `@@[main]` is observable
    /// (GDScript file structure). Other targets (Python, JS, Rust,
    /// etc.) ignore the attribute. The validator runs unconditionally
    /// because shipping `@@[main]` semantics into the source contract
    /// shouldn't depend on which target is being compiled.
    pub fn validate_module_main_attr(
        &mut self,
        ast: &FrameAst,
    ) -> Result<(), Vec<ValidationError>> {
        if let FrameAst::Module(module) = ast {
            let systems = &module.systems;
            if systems.len() <= 1 {
                return if self.errors.is_empty() {
                    Ok(())
                } else {
                    Err(self.errors.clone())
                };
            }
            let main_systems: Vec<&SystemAst> = systems.iter().filter(|s| s.is_main()).collect();
            match main_systems.len() {
                0 => {
                    let names: Vec<&str> = systems.iter().map(|s| s.name.as_str()).collect();
                    self.errors.push(ValidationError::new(
                        "E805",
                        format!(
                            "Module declares {} systems ({}) but no `@@[main]` \
                             attribute. Add `@@[main]` above the system that \
                             callers should instantiate via the module's \
                             primary entry point. For GDScript this is the \
                             system returned by `preload(\"<file>.gd\").new()`. \
                             For Java this is the file's `public class`.",
                            systems.len(),
                            names.join(", ")
                        ),
                    ));
                }
                1 => {
                    // Exactly one main — the happy path.
                }
                _ => {
                    let names: Vec<&str> = main_systems.iter().map(|s| s.name.as_str()).collect();
                    self.errors.push(ValidationError::new(
                        "E806",
                        format!(
                            "Module declares multiple `@@[main]` attributes \
                             ({}). Only one system per file may be the \
                             primary; remove `@@[main]` from all but one.",
                            names.join(", ")
                        ),
                    ));
                }
            }
        }
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    pub fn validate_self_calls(
        &mut self,
        ast: &FrameAst,
        source: &[u8],
        target: crate::frame_c::visitors::TargetLanguage,
    ) -> Result<(), Vec<ValidationError>> {
        match ast {
            FrameAst::System(system) => self.validate_system_self_calls(system, source, target),
            FrameAst::Module(module) => {
                for system in &module.systems {
                    self.validate_system_self_calls(system, source, target);
                }
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    fn validate_system_self_calls(
        &mut self,
        system: &SystemAst,
        source: &[u8],
        target: crate::frame_c::visitors::TargetLanguage,
    ) {
        let interface_methods = self.build_interface_map(system);

        // Validate handler bodies using the scanner (handles comments, strings correctly)
        if let Some(machine) = &system.machine {
            for state in &machine.states {
                for handler in &state.handlers {
                    let span = &handler.span;
                    if span.start >= source.len() || span.end > source.len() {
                        continue;
                    }
                    let body = &source[span.start..span.end];
                    self.validate_frame_segments_in_body(
                        body,
                        &interface_methods,
                        &state.name,
                        &handler.event,
                        target,
                    );
                }
            }
        }

        // Also validate action bodies
        for action in &system.actions {
            let span = &action.span;
            if span.start >= source.len() || span.end > source.len() {
                continue;
            }
            let body = &source[span.start..span.end];
            self.validate_frame_segments_in_body(
                body,
                &interface_methods,
                "(action)",
                &action.name,
                target,
            );
        }
    }

    /// Validate Frame segments in a handler/action body using the scanner.
    /// Runs the language-specific scanner on the body text, then walks the
    /// identified segments. No byte-level scanning — the scanner handles
    /// comments, strings, and language-specific syntax.
    fn validate_frame_segments_in_body(
        &mut self,
        body: &[u8],
        interface_methods: &HashMap<String, &InterfaceMethod>,
        scope_outer: &str,
        scope_inner: &str,
        target: crate::frame_c::visitors::TargetLanguage,
    ) {
        // Find the opening brace
        let open_brace = match body.iter().position(|&b| b == b'{') {
            Some(pos) => pos,
            None => return,
        };

        // Run the scanner
        let mut scanner = get_native_scanner(target);
        let scan_result = match scanner.scan(body, open_brace) {
            Ok(r) => r,
            Err(_) => return, // Scanner error — can't validate
        };

        // Walk segments and validate
        for region in &scan_result.regions {
            if let Region::FrameSegment { kind, metadata, .. } = region {
                match kind {
                    // E601: @@:self.method() — check method exists in interface
                    FrameSegmentKind::ContextSelfCall => {
                        if let SegmentMetadata::SelfCall { method, args } = metadata {
                            if let Some(iface_method) = interface_methods.get(method.as_str()) {
                                // E602: check argument count
                                let arg_count = count_args(args);
                                let expected = iface_method.params.len();
                                if arg_count != expected {
                                    self.errors.push(ValidationError::new(
                                        "E602",
                                        format!(
                                            "@@:self.{}() in {}/{} has {} arguments but interface expects {}",
                                            method, scope_outer, scope_inner, arg_count, expected
                                        )
                                    ));
                                }
                            } else {
                                self.errors.push(ValidationError::new(
                                    "E601",
                                    format!(
                                        "@@:self.{}() in {}/{} — method '{}' not found in interface",
                                        method, scope_outer, scope_inner, method
                                    )
                                ));
                            }
                        }
                    }

                    // E603: bare @@:self without .method()
                    FrameSegmentKind::ContextSelf => {
                        self.errors.push(ValidationError::new(
                            "E603",
                            format!(
                                "bare `@@:self` in {}/{} — `@@:self` requires a member access (e.g. `@@:self.method(args)`)",
                                scope_outer, scope_inner
                            ),
                        ));
                    }

                    // E604: bare @@:system without .state
                    FrameSegmentKind::ContextSystemBare => {
                        self.errors.push(ValidationError::new(
                            "E604",
                            format!(
                                "bare `@@:system` in {}/{} — `@@:system` requires a member access (e.g. `@@:system.state`)",
                                scope_outer, scope_inner
                            ),
                        ));
                    }

                    _ => {}
                }
            }
        }
    }

    pub fn validate_target_specific(
        &mut self,
        ast: &FrameAst,
        target: crate::frame_c::visitors::TargetLanguage,
    ) -> Result<(), Vec<ValidationError>> {
        match ast {
            FrameAst::System(system) => self.validate_system_target_specific(system, target),
            FrameAst::Module(module) => {
                for system in &module.systems {
                    self.validate_system_target_specific(system, target);
                }
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    fn validate_system_target_specific(
        &mut self,
        system: &SystemAst,
        target: crate::frame_c::visitors::TargetLanguage,
    ) {
        // E605: Static targets require explicit type on domain fields
        self.validate_domain_types(system, target);

        match target {
            crate::frame_c::visitors::TargetLanguage::GDScript => {
                self.validate_gdscript_reserved_methods(system);
            }
            crate::frame_c::visitors::TargetLanguage::TypeScript
            | crate::frame_c::visitors::TargetLanguage::JavaScript => {
                self.validate_typescript_global_collision(system);
            }
            _ => {}
        }
    }

    /// W501: System name shadows a TypeScript / JavaScript global.
    /// The framepiler emits `class <SystemName> { ... }`, so naming a
    /// system `Worker`, `Buffer`, `Promise`, `Map`, etc. produces a
    /// class declaration that shadows the global of the same name in
    /// the surrounding TypeScript scope. This is rarely what the user
    /// wants — every `new Worker(...)` call site in the same file now
    /// instantiates the Frame system instead of the web API.
    ///
    /// Unlike the GDScript case (which is a hard error because the
    /// generated method silently overrides an inherited base-class
    /// method), this is a soft warning: shadowing is legal in
    /// TypeScript and the user may have intentionally chosen the
    /// name. We print the warning to stderr but the build still
    /// succeeds.
    fn validate_typescript_global_collision(&mut self, system: &SystemAst) {
        if let Some(suggested) = typescript_global_collision_rename(&system.name) {
            self.warnings.push(
                ValidationError::new(
                    "W501",
                    format!(
                        "System name '{0}' shadows the TypeScript/JavaScript global \
                         `{0}`. The generated `class {0} {{ ... }}` will mask the \
                         built-in within this module — every `new {0}(...)` in the \
                         surrounding code will instantiate the Frame system instead. \
                         Consider renaming (suggested: '{1}'). Pass --no-warnings to \
                         silence.",
                        system.name, suggested
                    ),
                )
                .with_span(system.span.clone()),
            );
        }
    }

    /// E501: Interface method names that would collide with
    /// `Object`'s built-in methods in GDScript. Frame compiles each
    /// interface method to a public method on the generated class,
    /// which inherits from `Object` (via `RefCounted` in Godot 4). If a
    /// user names an interface method `get`, `set`, `call`, etc., the
    /// generated method silently overrides the `Object` method and
    /// every call site that does `obj.get("foo")` ends up routed
    /// through the user's interface method instead. This is a common
    /// foot-gun in the Godot ecosystem; emit a structured error with
    /// a suggested rename rather than letting it surface at runtime.
    fn validate_gdscript_reserved_methods(&mut self, system: &SystemAst) {
        for method in &system.interface {
            if let Some(suggested) = gdscript_reserved_method_rename(&method.name) {
                self.errors.push(
                    ValidationError::new(
                        "E501",
                        format!(
                            "Interface method '{}' in system '{}' collides with GDScript's \
                             built-in `Object.{}` method. Calls like `obj.{}(...)` would \
                             silently invoke the Frame interface method instead of the engine \
                             method, breaking core GDScript reflection. Rename it (suggested: '{}').",
                            method.name, system.name, method.name, method.name, suggested
                        ),
                    )
                    .with_span(method.span.clone()),
                );
            }
        }
    }

    /// Additional validation using the Arcanum
    fn validate_system_with_arcanum(&mut self, system: &SystemAst, arcanum: &Arcanum) {
        // E402 enhanced: Validate transitions using Arcanum
        if let Some(machine) = &system.machine {
            for state in &machine.states {
                self.validate_state_transitions_with_arcanum(&system.name, state, arcanum);
            }
        }
    }

    /// Validate state transitions using Arcanum's state resolution
    fn validate_state_transitions_with_arcanum(
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
    fn validate_transition_with_arcanum(
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

    /// Validate a system
    fn validate_system(&mut self, system: &SystemAst) {
        // Phase 1: Structural validation
        self.validate_section_order(system);
        self.validate_duplicate_sections(system);
        self.validate_duplicate_system_params(system);

        // Build lookup tables
        let state_map = self.build_state_map(system);
        let interface_methods = self.build_interface_map(system);
        let actions = self.build_action_set(system);
        let operations = self.build_operation_set(system);

        // E413: Validate no HSM cycles
        self.validate_hsm_cycles(system, &state_map);

        // Validate machine if present
        if let Some(machine) = &system.machine {
            self.validate_machine(
                machine,
                &state_map,
                &interface_methods,
                &actions,
                &operations,
                &system.name,
            );
        }

        // E401: Validate no Frame statements in actions
        for action in &system.actions {
            self.validate_action_no_frame_statements(action);
        }

        // E401: Validate no Frame statements in operations
        // E421: @@:system.state not allowed in static operations
        for operation in &system.operations {
            self.validate_operation_no_frame_statements(operation);
            if operation.is_static {
                if let Some(ref code) = operation.body.code {
                    if code.contains("@@:system.state") {
                        self.errors.push(
                            ValidationError::new(
                                "E421",
                                format!(
                                    "'@@:system.state' is not allowed in static operation '{}' in system '{}'. \
                                     Static operations have no access to the system's compartment.",
                                    operation.name, system.name
                                ),
                            )
                            .with_span(operation.span.clone()),
                        );
                    }
                }
            }
        }

        // E420: `static` only valid on operations
        self.validate_static_placement(system);

        // Domain field validation
        self.validate_domain_fields(system);

        // E416 / E417 / E418: System parameter semantics — must align with
        // start state and domain declarations.
        self.validate_system_param_semantics(system);

        // E615: Assignment to const domain field in handler bodies
        self.validate_const_field_assignments(system);

        // E800/E801/E802: RFC-0013 attribute validation on
        // per-item `@@[name(args?)]` attachments.
        self.validate_attributes(system);
    }

    /// Validate `@@[name(args?)]` attributes attached to interface
    /// methods, handlers, and domain fields.
    ///
    /// - **E800**: unknown attribute name. Recognized per-item kinds
    ///   are currently just `target`.
    /// - **E801**: known attribute name attached at a position where
    ///   it isn't allowed. Today `persist` is module-scope-only and
    ///   reaching this validator on it means the user put it on a
    ///   per-item position.
    /// - **E802**: invalid arg shape. `target` requires a single
    ///   string argument naming a supported language.
    fn validate_attributes(&mut self, system: &SystemAst) {
        use crate::frame_c::compiler::frame_ast::Attribute;
        use crate::frame_c::visitors::TargetLanguage;
        use std::convert::TryFrom;

        fn unquote(s: &str) -> &str {
            let t = s.trim();
            let bytes = t.as_bytes();
            if bytes.len() >= 2
                && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                    || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
            {
                &t[1..t.len() - 1]
            } else {
                t
            }
        }

        let mut errs: Vec<ValidationError> = Vec::new();
        let check = |a: &Attribute, position: &str, errs: &mut Vec<ValidationError>| {
            match a.name.as_str() {
                "target" => match &a.args {
                    Some(raw) => {
                        let lang = unquote(raw);
                        if lang.is_empty() {
                            errs.push(
                                ValidationError::new(
                                    "E802",
                                    format!(
                                        "@@[target(...)] requires a string language name on {} (got empty arg).",
                                        position
                                    ),
                                )
                                .with_span(a.span.clone()),
                            );
                        } else if TargetLanguage::try_from(lang).is_err() {
                            errs.push(
                                ValidationError::new(
                                    "E802",
                                    format!(
                                        "@@[target(\"{}\")] on {} — '{}' is not a supported target language.",
                                        lang, position, lang
                                    ),
                                )
                                .with_span(a.span.clone()),
                            );
                        }
                    }
                    None => errs.push(
                        ValidationError::new(
                            "E802",
                            format!(
                                "@@[target] on {} requires a language argument, e.g. @@[target(\"python_3\")].",
                                position
                            ),
                        )
                        .with_span(a.span.clone()),
                    ),
                },
                "persist" => errs.push(
                    ValidationError::new(
                        "E801",
                        format!(
                            "@@[persist] is only valid at module scope on a @@system declaration; not on {}.",
                            position
                        ),
                    )
                    .with_span(a.span.clone()),
                ),
                // RFC-0012 amendment: @@[save] / @@[load] are valid
                // only on operations of @@[persist] systems. Reject
                // here if attached to a non-operation position
                // (interface method, handler, domain field).
                "save" | "load" => errs.push(
                    ValidationError::new(
                        "E801",
                        format!(
                            "@@[{}] is only valid on operations of @@[persist] systems; not on {}.",
                            a.name, position
                        ),
                    )
                    .with_span(a.span.clone()),
                ),
                // RFC-0012 amendment: @@[no_persist] is valid only on
                // domain fields. The per-position iteration below
                // exempts the domain check via the same name match.
                "no_persist" => errs.push(
                    ValidationError::new(
                        "E801",
                        format!(
                            "@@[no_persist] is only valid on domain fields of @@[persist] systems; not on {}.",
                            position
                        ),
                    )
                    .with_span(a.span.clone()),
                ),
                _ => errs.push(
                    ValidationError::new(
                        "E800",
                        format!("Unknown attribute @@[{}] on {}.", a.name, position),
                    )
                    .with_span(a.span.clone()),
                ),
            }
        };

        for m in &system.interface {
            for a in &m.attributes {
                check(a, &format!("interface method '{}'", m.name), &mut errs);
            }
        }
        // Domain field attributes — `no_persist` is valid here, so we
        // don't run the generic `check` for it; everything else falls
        // through to the standard set.
        for d in &system.domain {
            for a in &d.attributes {
                if a.name == "no_persist" {
                    if system.persist_attr.is_none() {
                        errs.push(
                            ValidationError::new(
                                "E801",
                                format!(
                                    "@@[no_persist] on domain field '{}' requires the system to have @@[persist].",
                                    d.name
                                ),
                            )
                            .with_span(a.span.clone()),
                        );
                    }
                    continue;
                }
                check(a, &format!("domain field '{}'", d.name), &mut errs);
            }
        }
        // Operation attributes — `save` / `load` are valid here; other
        // names fall through to the generic check.
        let mut save_count = 0usize;
        let mut load_count = 0usize;
        let mut first_save_span: Option<crate::frame_c::compiler::frame_ast::Span> = None;
        let mut first_load_span: Option<crate::frame_c::compiler::frame_ast::Span> = None;
        for op in &system.operations {
            for a in &op.attributes {
                if a.name == "save" || a.name == "load" {
                    if system.persist_attr.is_none() {
                        errs.push(
                            ValidationError::new(
                                "E801",
                                format!(
                                    "@@[{}] on operation '{}' requires the system to have @@[persist].",
                                    a.name, op.name
                                ),
                            )
                            .with_span(a.span.clone()),
                        );
                    }
                    if a.name == "save" {
                        save_count += 1;
                        if first_save_span.is_none() {
                            first_save_span = Some(a.span.clone());
                        }
                    } else {
                        load_count += 1;
                        if first_load_span.is_none() {
                            first_load_span = Some(a.span.clone());
                        }
                    }
                    continue;
                }
                check(a, &format!("operation '{}'", op.name), &mut errs);
            }
        }
        // E810: at most one @@[save] and one @@[load] per system. Two
        // ops can't both fill the same persist endpoint — codegen
        // wouldn't know which to invoke, and the contract requires
        // a single primary entry point per direction.
        if save_count > 1 {
            errs.push(
                ValidationError::new(
                    "E810",
                    format!(
                        "@@[save] declared {} times in system '{}'; expected at most one.",
                        save_count, system.name
                    ),
                )
                .with_span(first_save_span.unwrap_or_else(|| system.span.clone())),
            );
        }
        if load_count > 1 {
            errs.push(
                ValidationError::new(
                    "E810",
                    format!(
                        "@@[load] declared {} times in system '{}'; expected at most one.",
                        load_count, system.name
                    ),
                )
                .with_span(first_load_span.unwrap_or_else(|| system.span.clone())),
            );
        }
        // E814: hard-cut for the RFC-0012 amendment. `@@[persist]`
        // requires explicit `@@[save]` and `@@[load]` operation
        // attributes. The bare form is no longer accepted — the
        // legacy `save_state` / `restore_state` static-factory shape
        // doesn't work on GDScript (target-language scoping limit)
        // and the four-attribute contract is uniform across all 17
        // backends. Migration: declare two operations under the
        // `operations:` section, one tagged `@@[save]` (returns a
        // serialized blob), one tagged `@@[load]` (instance method
        // taking the blob). See docs/rfcs/rfc-0012.md.
        if system.persist_attr.is_some() && save_count == 0 && load_count == 0 {
            errs.push(
                ValidationError::new(
                    "E814",
                    format!(
                        "@@[persist] system '{}' must declare @@[save] and @@[load] operation \
                         attributes (RFC-0012 amendment). The bare `@@[persist]` form is no longer \
                         accepted. Add to your `operations:` section:\n\
                         \x20   @@[save]\n\
                         \x20   save_state(): <blob_type> {{}}\n\n\
                         \x20   @@[load]\n\
                         \x20   restore_state(data: <blob_type>) {{}}\n\
                         The save op returns a serialized blob; the load op is an instance method \
                         that mutates self. See docs/rfcs/rfc-0012.md \"Naming the save/load \
                         methods\".",
                        system.name
                    ),
                )
                .with_span(system.span.clone()),
            );
        }
        if let Some(machine) = &system.machine {
            for state in &machine.states {
                for h in &state.handlers {
                    for a in &h.attributes {
                        check(
                            a,
                            &format!("handler '{}' in state '{}'", h.event, state.name),
                            &mut errs,
                        );
                    }
                }
            }
        }
        self.errors.extend(errs);
    }

    /// Cross-check the system header parameter list against the start
    /// state's parameter list, the start state's `$>()` enter handler,
    /// and the domain block:
    ///
    /// - **E416**: `$(name)` start-args must match the start state's
    ///   declared params (order-insensitive, by name).
    /// - **E417**: `$>(name)` enter-args must match the start state's
    ///   `$>()` handler params; if no `$>()` handler exists, that's also
    ///   E417.
    /// - **E418**: each domain-kind param (bare name) must correspond
    ///   to a declared variable in the `domain:` block.
    fn validate_system_param_semantics(&mut self, system: &SystemAst) {
        // Bucket the system params by kind.
        let start_args: Vec<&str> = system
            .params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::StateArg))
            .map(|p| p.name.as_str())
            .collect();
        let enter_args: Vec<&str> = system
            .params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::EnterArg))
            .map(|p| p.name.as_str())
            .collect();
        let domain_args: Vec<&str> = system
            .params
            .iter()
            .filter(|p| matches!(p.kind, ParamKind::Domain))
            .map(|p| p.name.as_str())
            .collect();

        if start_args.is_empty() && enter_args.is_empty() && domain_args.is_empty() {
            return;
        }

        // Resolve the start state. By convention it's the first state
        // declared in the machine (the V4 parser preserves source order).
        let start_state = match system.machine.as_ref().and_then(|m| m.states.first()) {
            Some(s) => s,
            None => return,
        };

        // E416: order-insensitive name comparison.
        if !start_args.is_empty() || !start_state.params.is_empty() {
            let mut want: Vec<&str> = start_args.clone();
            want.sort_unstable();
            let mut have: Vec<&str> = start_state.params.iter().map(|p| p.name.as_str()).collect();
            have.sort_unstable();
            if want != have {
                self.errors.push(
                    ValidationError::new(
                        "E416",
                        format!(
                            "system '{}' start parameters ({:?}) must match start state '{}' parameters ({:?})",
                            system.name, start_args, start_state.name,
                            start_state.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()
                        ),
                    )
                    .with_span(system.span.clone()),
                );
            }
        }

        // E417: enter-args require a matching `$>()` handler on the start state.
        if !enter_args.is_empty() {
            match &start_state.enter {
                None => {
                    self.errors.push(
                        ValidationError::new(
                            "E417",
                            format!(
                                "system '{}' declares $>(...) enter parameters but start state '{}' has no $>() handler",
                                system.name, start_state.name
                            ),
                        )
                        .with_span(system.span.clone()),
                    );
                }
                Some(enter_handler) => {
                    let mut want: Vec<&str> = enter_args.clone();
                    want.sort_unstable();
                    let mut have: Vec<&str> = enter_handler
                        .params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect();
                    have.sort_unstable();
                    if want != have {
                        self.errors.push(
                            ValidationError::new(
                                "E417",
                                format!(
                                    "system '{}' enter parameters ({:?}) must match start state '{}' $>() parameters ({:?})",
                                    system.name, enter_args, start_state.name,
                                    enter_handler.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>()
                                ),
                            )
                            .with_span(system.span.clone()),
                        );
                    }
                }
            }
        }

        // E418: every domain-kind sys param must EITHER (a) match a
        // domain field name (the param value initializes that field),
        // OR (b) be referenced as an identifier inside some domain
        // field's initializer expression. The latter pattern lets a
        // user write `Counter(initial: int) { domain: value: int = initial }`
        // — `initial` doesn't name a field but feeds one.
        if !domain_args.is_empty() {
            let domain_names: HashSet<&str> =
                system.domain.iter().map(|v| v.name.as_str()).collect();
            for dp in &domain_args {
                let matches_field = domain_names.contains(dp);
                let matches_init = system.domain.iter().any(|v| {
                    v.initializer_text
                        .as_deref()
                        .map(|t| identifier_appears_in(t, dp))
                        .unwrap_or(false)
                });
                if !matches_field && !matches_init {
                    self.errors.push(
                        ValidationError::new(
                            "E418",
                            format!(
                                "system '{}' domain parameter '{}' has no matching variable in domain: block",
                                system.name, dp
                            ),
                        )
                        .with_span(system.span.clone()),
                    );
                }
            }
        }
    }

    /// E615: Direct assignment to a `const` domain field inside a handler
    /// body. Catches the obvious per-target self-access patterns; the target
    /// language compiler catches anything else via the emitted
    /// `final` / `readonly` / `const` / `val` / `let` keyword.
    fn validate_const_field_assignments(&mut self, system: &SystemAst) {
        let const_fields: Vec<&str> = system
            .domain
            .iter()
            .filter(|v| v.is_const)
            .map(|v| v.name.as_str())
            .collect();
        if const_fields.is_empty() {
            return;
        }

        let machine = match &system.machine {
            Some(m) => m,
            None => return,
        };

        for state in &machine.states {
            if let Some(ref h) = state.enter {
                self.scan_body_for_const_assigns(&h.body, &const_fields, &system.name, "$>");
            }
            if let Some(ref h) = state.exit {
                self.scan_body_for_const_assigns(&h.body, &const_fields, &system.name, "$<");
            }
            for h in &state.handlers {
                self.scan_body_for_const_assigns(&h.body, &const_fields, &system.name, &h.event);
            }
        }
    }

    fn scan_body_for_const_assigns(
        &mut self,
        body: &HandlerBody,
        const_fields: &[&str],
        system_name: &str,
        event_name: &str,
    ) {
        for stmt in &body.statements {
            let code = match stmt {
                Statement::NativeCode(s) => s.as_str(),
                _ => continue,
            };
            for field in const_fields {
                // Per-target self-access prefixes that resolve to the system
                // instance: catches `self.x =`, `this->x =`, `@x =`, etc.
                let prefixes = [
                    format!("self.{}", field),
                    format!("this.{}", field),
                    format!("self->{}", field),
                    format!("this->{}", field),
                    format!("$this->{}", field),
                    format!("@{}", field),
                ];
                let mut flagged = false;
                for prefix in &prefixes {
                    let mut search_from = 0usize;
                    while let Some(rel) = code[search_from..].find(prefix.as_str()) {
                        let abs = search_from + rel;
                        let after = &code[abs + prefix.len()..];
                        let trimmed = after.trim_start();
                        // Match `=` or augmented assignment, but NOT `==`.
                        let is_assign = (trimmed.starts_with('=')
                            && !trimmed.starts_with("==")
                            && !trimmed.starts_with("=>"))
                            || trimmed.starts_with("+=")
                            || trimmed.starts_with("-=")
                            || trimmed.starts_with("*=")
                            || trimmed.starts_with("/=")
                            || trimmed.starts_with("%=");
                        if is_assign {
                            // Reject access to a sub-field: `self.x.foo = ...`
                            // (the assignment is to `foo`, not to `x`).
                            // The trim already handled whitespace before `=`,
                            // so any `.` immediately after the prefix means
                            // the user is accessing a member of the field,
                            // not assigning to the field itself.
                            let raw_after = &code[abs + prefix.len()..];
                            if !raw_after.starts_with('.') && !raw_after.starts_with("->") {
                                self.errors.push(
                                    ValidationError::new(
                                        "E615",
                                        format!(
                                            "Assignment to const domain field '{}' \
                                             in system '{}' handler '{}'",
                                            field, system_name, event_name
                                        ),
                                    )
                                    .with_span(body.span.clone()),
                                );
                                flagged = true;
                                break;
                            }
                        }
                        search_from = abs + prefix.len();
                    }
                    if flagged {
                        break;
                    }
                }
            }
        }
    }

    /// E420: `static` is only valid on operations
    fn validate_static_placement(&mut self, system: &SystemAst) {
        for method in &system.interface {
            if method.is_static {
                self.errors.push(
                    ValidationError::new(
                        "E420",
                        format!(
                            "'static' is not valid on interface method '{}' in system '{}'. \
                             Only operations can be static.",
                            method.name, system.name
                        ),
                    )
                    .with_span(method.span.clone()),
                );
            }
        }
        for action in &system.actions {
            if action.is_static {
                self.errors.push(
                    ValidationError::new(
                        "E420",
                        format!(
                            "'static' is not valid on action '{}' in system '{}'. \
                             Only operations can be static.",
                            action.name, system.name
                        ),
                    )
                    .with_span(action.span.clone()),
                );
            }
        }
    }

    /// E613: Domain field shadows system parameter
    /// E614: Duplicate domain field name
    fn validate_domain_fields(&mut self, system: &SystemAst) {
        let param_names: HashSet<&str> = system.params.iter().map(|p| p.name.as_str()).collect();
        let mut seen: HashSet<&str> = HashSet::new();

        for var in &system.domain {
            // E614: Duplicate domain field name
            if !seen.insert(&var.name) {
                self.errors.push(
                    ValidationError::new(
                        "E614",
                        format!(
                            "Duplicate domain field '{}' in system '{}'",
                            var.name, system.name
                        ),
                    )
                    .with_span(var.span.clone()),
                );
            }

            // Note: Domain fields intentionally share names with Domain-kind system
            // params (the param initializes the field). E613 is reserved for future
            // use if we want to warn about non-Domain param shadowing.
        }
    }

    /// E605: Static targets require explicit type on domain fields
    fn validate_domain_types(
        &mut self,
        system: &SystemAst,
        target: crate::frame_c::visitors::TargetLanguage,
    ) {
        use crate::frame_c::visitors::TargetLanguage::*;
        // Only languages where the compiler cannot infer field types from init values.
        // Kotlin, Swift, Dart, TypeScript, C#, Rust all have type inference.
        let is_static = matches!(target, C | Cpp | Java | Go);
        if !is_static {
            return;
        }
        for var in &system.domain {
            if matches!(var.var_type, Type::Unknown) {
                self.errors.push(
                    ValidationError::new(
                        "E605",
                        format!(
                            "Domain field '{}' in system '{}' requires an explicit type for target '{:?}'",
                            var.name, system.name, target
                        ),
                    )
                    .with_span(var.span.clone()),
                );
            }
        }
    }

    /// E113: Validate system section order (operations:, interface:, machine:, actions:, domain:)
    fn validate_section_order(&mut self, system: &SystemAst) {
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
    fn validate_duplicate_system_params(&mut self, system: &SystemAst) {
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
    fn validate_duplicate_sections(&mut self, system: &SystemAst) {
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
    fn validate_action_no_frame_statements(&mut self, action: &ActionAst) {
        // Actions have native bodies, but we check if the native content
        // contains Frame statement patterns (this would be caught by the scanner
        // but we can add an extra check here)
        // For now, actions are pure native, so no validation needed here
        // The validation happens during scanning/parsing
        let _ = action; // suppress unused warning
    }

    /// E401: Validate no Frame statements in operation body
    fn validate_operation_no_frame_statements(&mut self, operation: &OperationAst) {
        // Operations have native bodies, same as actions
        let _ = operation; // suppress unused warning
    }

    /// Build a map of state names to state definitions
    fn build_state_map<'a>(&mut self, system: &'a SystemAst) -> HashMap<String, &'a StateAst> {
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

    /// E413: Detect circular parent chains in HSM hierarchy
    fn validate_hsm_cycles(&mut self, _system: &SystemAst, state_map: &HashMap<String, &StateAst>) {
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
    fn build_action_set(&self, system: &SystemAst) -> HashSet<String> {
        system.actions.iter().map(|a| a.name.clone()).collect()
    }

    /// Build a set of operation names
    fn build_operation_set(&self, system: &SystemAst) -> HashSet<String> {
        system.operations.iter().map(|o| o.name.clone()).collect()
    }

    /// Validate a machine
    fn validate_machine(
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
    fn validate_reachable_states(
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

    /// Validate a state
    fn validate_state(
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
    fn validate_handler(
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
    fn validate_handler_body(
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
    fn validate_terminal_last(&mut self, body: &HandlerBody) {
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
    fn is_terminal_statement(&self, stmt: &Statement) -> bool {
        matches!(stmt, Statement::Transition(_))
    }

    /// Validate a transition statement
    fn validate_transition(
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
    fn validate_forward(
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
    fn format_available_states(&self, state_map: &HashMap<String, &StateAst>) -> String {
        let mut states: Vec<String> = state_map.keys().cloned().collect();
        states.sort();
        states.join(", ")
    }
}

/// Reserved-method check for GDScript: returns `Some(rename_suggestion)`
/// if the given interface method name would collide with a method on
/// Godot's `Object` (or close ancestor) class hierarchy. Returns
/// `None` for names that are safe to use as-is.
///
/// The list is intentionally conservative — we only flag names that
/// are documented public methods on `Object` in Godot 4 and that a
/// Frame user might realistically want to name an interface method.
/// We don't flag every internal `_notification`-style helper because
/// underscore-prefixed names are uncommon as interface method names
/// anyway.
///
/// Source: Godot 4 documentation, `Object` class reference.
pub fn gdscript_reserved_method_rename(name: &str) -> Option<&'static str> {
    match name {
        // Property reflection — the most common collisions in practice.
        "get" => Some("get_value"),
        "set" => Some("set_value"),
        // Method reflection / dispatch
        "call" => Some("invoke"),
        "call_deferred" => Some("invoke_deferred"),
        "callv" => Some("invoke_with_args"),
        "has_method" => Some("supports_method"),
        // Lifecycle
        "free" => Some("dispose"),
        "queue_free" => Some("schedule_free"),
        "notification" => Some("notify"),
        // Signals
        "connect" => Some("connect_handler"),
        "disconnect" => Some("disconnect_handler"),
        "emit_signal" => Some("emit"),
        "has_signal" => Some("supports_signal"),
        "get_signal_list" => Some("list_signals"),
        // Class / script reflection
        "get_class" => Some("class_name"),
        "is_class" => Some("is_a"),
        "get_script" => Some("script"),
        "set_script" => Some("attach_script"),
        // Property list
        "get_property_list" => Some("list_properties"),
        // Metadata
        "get_meta" => Some("read_meta"),
        "set_meta" => Some("write_meta"),
        "has_meta" => Some("supports_meta"),
        "remove_meta" => Some("clear_meta"),
        // Stringification / translation
        "to_string" => Some("describe"),
        "tr" => Some("translate"),
        "tr_n" => Some("translate_plural"),
        // Instance identity
        "get_instance_id" => Some("instance_id"),
        // Object lifecycle helpers commonly used in tests
        "is_queued_for_deletion" => Some("is_pending_free"),
        _ => None,
    }
}

/// Global-shadowing check for TypeScript / JavaScript: returns
/// `Some(rename_suggestion)` if `name` would clash with a commonly
/// referenced built-in or web-API global when used as a system name.
/// Returns `None` for names that are safe.
///
/// We focus on names a Frame user might realistically choose for a
/// system class — `Worker` (web/service workers, also a planned
/// framepiler demo), `Buffer` (Node.js), `Map`/`Set`/`Promise` (ES
/// built-ins), `Request`/`Response` (Fetch API), etc. The list is
/// intentionally NOT exhaustive — it covers the high-confidence
/// foot-guns. Esoteric DOM types (`HTMLOListElement` etc.) are
/// excluded to keep the warning signal-to-noise high.
///
/// The suggested rename appends `Sys` so the user can easily
/// disambiguate (`Worker` → `WorkerSys`, `Map` → `MapSys`).
pub fn typescript_global_collision_rename(name: &str) -> Option<String> {
    let is_global = matches!(
        name,
        // Web Workers / Service Workers
        "Worker" | "ServiceWorker" | "SharedWorker" | "WorkerGlobalScope"
        // Node.js core
        | "Buffer" | "Process" | "Console"
        // ES built-in classes
        | "Promise" | "Map" | "Set" | "WeakMap" | "WeakSet"
        | "Date" | "RegExp" | "Error" | "TypeError" | "RangeError" | "SyntaxError"
        | "Array" | "Object" | "String" | "Number" | "Boolean" | "Symbol" | "BigInt"
        | "Function" | "Proxy" | "Reflect"
        | "ArrayBuffer" | "DataView"
        | "Int8Array" | "Uint8Array" | "Uint8ClampedArray"
        | "Int16Array" | "Uint16Array" | "Int32Array" | "Uint32Array"
        | "Float32Array" | "Float64Array" | "BigInt64Array" | "BigUint64Array"
        // DOM / browser globals
        | "Window" | "Document" | "Element" | "Node" | "Event" | "EventTarget"
        | "HTMLElement" | "Image" | "Audio" | "Video"
        | "Storage"
        // Fetch / network
        | "Request" | "Response" | "Headers" | "URL" | "URLSearchParams"
        | "WebSocket" | "XMLHttpRequest" | "FormData"
    );
    if is_global {
        Some(format!("{}Sys", name))
    } else {
        None
    }
}

/// Convenience function to validate Frame source code. Runs the full
/// V4 pipeline (parse + validate + codegen) and surfaces any errors.
/// Used only by this module's unit tests.
pub fn validate_frame_source(
    source: &str,
    target: TargetLanguage,
) -> Result<(), Vec<ValidationError>> {
    use crate::frame_c::compiler::pipeline::compile_module;
    use crate::frame_c::compiler::pipeline::config::PipelineConfig;
    use crate::frame_c::visitors::TargetLanguage as VTarget;

    // The frame_ast::TargetLanguage enum used here only knows the
    // languages the legacy native scanner cared about. Map to the
    // visitors::TargetLanguage variants the pipeline actually drives.
    let v_target = match target {
        TargetLanguage::Python3 => VTarget::Python3,
        TargetLanguage::TypeScript => VTarget::TypeScript,
        TargetLanguage::Rust => VTarget::Rust,
        TargetLanguage::CSharp => VTarget::CSharp,
        TargetLanguage::C => VTarget::C,
        TargetLanguage::Cpp => VTarget::Cpp,
        TargetLanguage::Java => VTarget::Java,
        // Graphviz isn't a runtime target language but the pipeline
        // accepts it for diagram generation; route validation through
        // a neutral target so we still get the structural checks.
        TargetLanguage::Graphviz => VTarget::Python3,
    };

    let config = PipelineConfig::production(v_target);
    let result = compile_module(source.as_bytes(), &config).map_err(|e| {
        vec![ValidationError::new(
            "E000",
            format!("Pipeline error: {}", e.error),
        )]
    })?;

    if result.errors.is_empty() {
        Ok(())
    } else {
        Err(result
            .errors
            .iter()
            .map(|e| ValidationError::new(&e.code, e.message.clone()))
            .collect())
    }
}

/// Count arguments in a parenthesized argument string like "(a, b, c)".
/// Returns 0 for "()" or empty.
/// Whether `ident` appears as a whole identifier inside `text`. Used by
/// E418 to detect domain-field initializers that reference a domain-kind
/// system parameter (e.g. `value: int = initial`).
fn identifier_appears_in(text: &str, ident: &str) -> bool {
    if ident.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let key = ident.as_bytes();
    let n = bytes.len();
    let m = key.len();
    if m > n {
        return false;
    }
    let mut i = 0;
    while i + m <= n {
        if &bytes[i..i + m] == key {
            let prev_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let next_ok =
                i + m == n || !(bytes[i + m].is_ascii_alphanumeric() || bytes[i + m] == b'_');
            if prev_ok && next_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Number of params the caller is *required* to supply (= position of
/// the first defaulted param, or total length if none have defaults).
///
/// The trailing-defaults rule is implicit: every target language we
/// generate to enforces it, and the codegen path will reject a
/// signature like `(a, b = 1, c)` long before runtime. We mirror that
/// assumption here so the relaxation is positional, not popcount-based.
fn required_event_params(params: &[EventParam]) -> usize {
    params
        .iter()
        .position(|p| p.default_value.is_some())
        .unwrap_or(params.len())
}

/// Format an arity-mismatch error message, returning `Some(msg)` when
/// `provided` falls outside `[required, total]`. Used by E417 (enter)
/// and E419 (exit) where defaults relax the lower bound.
fn arity_error(
    provided: usize,
    total: usize,
    required: usize,
    receiver: &str,
    site: &str,
) -> Option<String> {
    if provided < required || provided > total {
        let arity_desc = if required == total {
            format!("{} parameter{}", total, if total == 1 { "" } else { "s" })
        } else {
            format!("between {} and {} parameters", required, total)
        };
        Some(format!(
            "{}: {} accepts {} but transition supplies {}",
            site, receiver, arity_desc, provided
        ))
    } else {
        None
    }
}

fn count_args(args: &str) -> usize {
    let inner = args
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    if inner.is_empty() {
        return 0;
    }
    // Count commas at depth 0
    let mut count = 1;
    let mut depth = 0;
    for b in inner.bytes() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    // Test convenience: import the visitors TargetLanguage with an alias
    // since `use super::*` already imports frame_ast::TargetLanguage
    use crate::frame_c::visitors::TargetLanguage as VTarget;

    /// Helper: parse + run BOTH the general validator and the
    /// target-specific validator. Delegates to the V4 pipeline, which
    /// runs both phases in the same order as a real `framec compile`.
    fn validate_for_target(source: &str, target: VTarget) -> Result<(), Vec<ValidationError>> {
        use crate::frame_c::compiler::pipeline::compile_module;
        use crate::frame_c::compiler::pipeline::config::PipelineConfig;

        let config = PipelineConfig::production(target);
        let result = compile_module(source.as_bytes(), &config).map_err(|e| {
            vec![ValidationError::new(
                "E000",
                format!("Pipeline error: {}", e.error),
            )]
        })?;

        if result.errors.is_empty() {
            Ok(())
        } else {
            Err(result
                .errors
                .iter()
                .map(|e| ValidationError::new(&e.code, e.message.clone()))
                .collect())
        }
    }

    #[test]
    fn test_e501_gdscript_get_collision() {
        // The validator only inspects interface declarations and method
        // names, so the handler body shape doesn't matter.
        let source = r#"
@@system Robot {
    interface:
        get()
    machine:
        $Start {
            get() { }
        }
}"#;
        let result = validate_for_target(source, VTarget::GDScript);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "E501");
        assert!(errors[0].message.contains("'get'"));
        assert!(errors[0].message.contains("Object.get"));
        assert!(
            errors[0].message.contains("get_value"),
            "should suggest get_value rename"
        );
    }

    #[test]
    fn test_e501_gdscript_set_and_call_collision() {
        let source = r#"
@@system Robot {
    interface:
        set()
        call()
    machine:
        $Start {
            set() { }
            call() { }
        }
}"#;
        let result = validate_for_target(source, VTarget::GDScript);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|e| e.code == "E501"));
        assert!(errors.iter().any(|e| e.message.contains("'set'")));
        assert!(errors.iter().any(|e| e.message.contains("'call'")));
    }

    #[test]
    fn test_e501_gdscript_safe_names_pass() {
        let source = r#"
@@system Robot {
    interface:
        get_value()
        set_value()
        do_something()
    machine:
        $Start {
            get_value() { }
            set_value() { }
            do_something() { }
        }
}"#;
        let result = validate_for_target(source, VTarget::GDScript);
        assert!(result.is_ok(), "safe names must pass: {:?}", result.err());
    }

    /// Helper that mirrors `validate_for_target` but ALSO returns the
    /// pipeline's collected warnings (W501 etc.) alongside the
    /// errors. Delegates to the V4 pipeline so warnings emitted at any
    /// stage (including the V4-specific `frame_validator` warnings)
    /// flow through unchanged.
    fn validate_for_target_with_warnings(
        source: &str,
        target: VTarget,
    ) -> (Result<(), Vec<ValidationError>>, Vec<ValidationError>) {
        use crate::frame_c::compiler::pipeline::compile_module;
        use crate::frame_c::compiler::pipeline::config::PipelineConfig;

        let config = PipelineConfig::production(target);
        let compile_result = match compile_module(source.as_bytes(), &config) {
            Ok(r) => r,
            Err(e) => {
                return (
                    Err(vec![ValidationError::new(
                        "E000",
                        format!("Pipeline error: {}", e.error),
                    )]),
                    vec![],
                )
            }
        };

        let warnings: Vec<ValidationError> = compile_result
            .warnings
            .iter()
            .map(|w| ValidationError::new(&w.code, w.message.clone()))
            .collect();

        let result = if compile_result.errors.is_empty() {
            Ok(())
        } else {
            Err(compile_result
                .errors
                .iter()
                .map(|e| ValidationError::new(&e.code, e.message.clone()))
                .collect())
        };

        (result, warnings)
    }

    #[test]
    fn test_w501_typescript_worker_warning() {
        // `Worker` is a high-confidence collision: the framepiler
        // itself plans a Demo 22 named Worker and the warning needs
        // to fire there.
        let source = r#"
@@system Worker {
    interface:
        run()
    machine:
        $Idle {
            run() { }
        }
}"#;
        let (result, warnings) = validate_for_target_with_warnings(source, VTarget::TypeScript);
        assert!(
            result.is_ok(),
            "TS shadowing is a warning, not an error: {:?}",
            result.err()
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "W501");
        assert!(warnings[0].message.contains("'Worker'"));
        assert!(
            warnings[0].message.contains("WorkerSys"),
            "should suggest WorkerSys rename"
        );
    }

    #[test]
    fn test_w501_typescript_buffer_and_promise_warnings() {
        // Two systems in the same module, both flagged.
        let source = r#"
@@system Buffer {
    interface:
        write()
    machine:
        $Start {
            write() { }
        }
}
@@[main]
@@system Promise {
    interface:
        resolve()
    machine:
        $Start {
            resolve() { }
        }
}"#;
        let (result, warnings) = validate_for_target_with_warnings(source, VTarget::TypeScript);
        assert!(result.is_ok());
        // Both system names should be flagged. The validator runs
        // per-system, so the helper here only sees the warnings from
        // the LAST system. The pipeline-level harvest is what gets
        // both — covered by the integration assertion below.
        assert!(!warnings.is_empty());
        assert!(warnings.iter().all(|w| w.code == "W501"));
    }

    #[test]
    fn test_w501_typescript_safe_name_no_warning() {
        let source = r#"
@@system Robot {
    interface:
        move()
    machine:
        $Start {
            move() { }
        }
}"#;
        let (result, warnings) = validate_for_target_with_warnings(source, VTarget::TypeScript);
        assert!(result.is_ok());
        assert!(
            warnings.is_empty(),
            "safe names should not warn: {:?}",
            warnings
        );
    }

    #[test]
    fn test_w501_python_target_no_warning() {
        // The TS shadowing check only fires for TS/JS targets.
        // Compiling the same source for Python must produce no warning.
        let source = r#"
@@system Worker {
    interface:
        run()
    machine:
        $Start {
            run() { }
        }
}"#;
        let (result, warnings) = validate_for_target_with_warnings(source, VTarget::Python3);
        assert!(result.is_ok());
        assert!(
            warnings.is_empty(),
            "python should not flag TS-only collisions: {:?}",
            warnings
        );
    }

    #[test]
    fn test_e501_python_does_not_flag_get() {
        // The same source that fails for GDScript must succeed for
        // Python — the reserved-method check is target-specific.
        let source = r#"
@@system Robot {
    interface:
        get()
    machine:
        $Start {
            get() { }
        }
}"#;
        let result = validate_for_target(source, VTarget::Python3);
        assert!(
            result.is_ok(),
            "python should not flag GDScript-only collisions: {:?}",
            result.err()
        );
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

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_err());

        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "E402");
        assert!(errors[0].message.contains("Unknown state 'Unknown'"));
    }

    #[test]
    fn test_e403_invalid_parent() {
        let source = r#"
@@system Test {
    machine:
        $Child => $InvalidParent {
            event() { => event() }
        }
        $ActualParent {
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_err());

        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.code == "E403" && e.message.contains("invalid parent")));
    }

    #[test]
    fn test_e403_forward_without_parent() {
        // Test using the supported forward syntax: => $^
        // Note: The scanner currently only detects "=> $^" pattern
        let source = r#"
@@system Test {
    machine:
        $Standalone {
            unhandled() {
                => $^
            }
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_err());

        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.code == "E403" && e.message.contains("no parent")));
    }

    #[test]
    fn test_e405_state_param_count_mismatch() {
        // E405 — transition supplies 3 state args but target declares 1.
        // Reachable in v4 because `enrich_handler_body_metadata` writes
        // `state_args` onto `TransitionAst` from the unified scanner,
        // letting the validator see the count instead of treating the
        // expression as an opaque NativeExpr blob.
        let source = r#"
@@system Test {
    machine:
        $Start {
            go() { -> $Target(1, 2, 3) }
        }
        $Target(x: int) { }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        let errors = result.expect_err("E405 must fire for arity mismatch");
        let codes: Vec<&str> = errors.iter().map(|e| e.code.as_str()).collect();
        assert!(
            codes.iter().any(|c| *c == "E405"),
            "expected E405, got {:?}",
            codes
        );
    }

    #[test]
    fn test_e405_state_no_params_but_args_supplied() {
        // E405 — target state declares no state params but transition
        // supplies args. Distinct sub-form: "no receiver" vs "wrong
        // count". Validator emits a tailored message in this case.
        let source = r#"
@@system Test {
    machine:
        $Start {
            go() { -> $Target(1, 2) }
        }
        $Target { }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        let errors = result.expect_err("E405 must fire for no-receiver-with-args");
        let codes: Vec<&str> = errors.iter().map(|e| e.code.as_str()).collect();
        assert!(
            codes.iter().any(|c| *c == "E405"),
            "expected E405, got {:?}",
            codes
        );
    }

    #[test]
    fn test_valid_system() {
        let source = r#"
@@system Valid {
    interface:
        process(data: string): bool
        
    machine:
        $Idle {
            start() { -> $Active() }
        }
        $Active {
            stop() { -> $Idle() }
            process(data: string) {
                ^ true
            }
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_ok());
    }

    #[test]
    fn test_e419_no_exit_handler() {
        // E419 — transition supplies exit args but source state has
        // no `<$()` exit handler at all. Hard error.
        let source = r#"
@@system Test {
    machine:
        $A {
            go() { ("reason") -> $B }
        }
        $B {
            go() {}
        }
}"#;
        let errors =
            validate_frame_source(source, TargetLanguage::Python3).expect_err("E419 must fire");
        assert!(errors.iter().any(|e| e.code == "E419"));
    }

    #[test]
    fn test_e419_undersupply_below_required() {
        // E419 — handler `<$(a, b)` requires 2 args, transition supplies 1.
        // Defaults aren't declared, so required = total = 2.
        let source = r#"
@@system Test {
    machine:
        $A {
            <$(a: str, b: str) {}
            go() { ("only_one") -> $B }
        }
        $B { go() {} }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::Python3)
            .expect_err("E419 must fire on undersupply");
        assert!(errors.iter().any(|e| e.code == "E419"));
    }

    #[test]
    fn test_e419_oversupply_above_total() {
        // E419 — handler `<$(a)` accepts 1, transition supplies 2.
        let source = r#"
@@system Test {
    machine:
        $A {
            <$(a: str) {}
            go() { ("a", "b") -> $B }
        }
        $B { go() {} }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::Python3)
            .expect_err("E419 must fire on oversupply");
        assert!(errors.iter().any(|e| e.code == "E419"));
    }

    #[test]
    fn test_e419_default_relaxes_undersupply() {
        // Default-aware relaxation — handler `<$(a: str, b: str = "x")`
        // has required=1 (b is defaulted). Caller may supply 1 or 2 args.
        // Validator must NOT flag E419 here.
        let source = r#"
@@system Test {
    machine:
        $A {
            <$(a: str, b: str = "x") {}
            go() { ("only_a") -> $B }
        }
        $B { go() {} }
}"#;
        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(
            result.is_ok(),
            "default-relaxed undersupply must compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_e419_default_still_blocks_oversupply() {
        // Defaults relax the lower bound only. Handler
        // `<$(a: str, b: str = "x")` total = 2; oversupply at 3 still
        // fires E419.
        let source = r#"
@@system Test {
    machine:
        $A {
            <$(a: str, b: str = "x") {}
            go() { ("a", "b", "c") -> $B }
        }
        $B { go() {} }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::Python3)
            .expect_err("E419 must still fire above total");
        assert!(errors.iter().any(|e| e.code == "E419"));
    }

    #[test]
    fn test_e417_transition_no_enter_handler() {
        // E417 transition form — caller supplies enter args but the
        // target state has no `$>()` handler. Hard error.
        let source = r#"
@@system Test {
    machine:
        $A {
            go() { -> ("hello") $B }
        }
        $B { go() {} }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::Python3)
            .expect_err("E417 must fire transition form");
        assert!(errors.iter().any(|e| e.code == "E417"));
    }

    #[test]
    fn test_e417_transition_oversupply() {
        // E417 transition form, oversupply.
        let source = r#"
@@system Test {
    machine:
        $A {
            go() { -> ("a", "b") $B }
        }
        $B {
            $>(a: str) {}
            go() {}
        }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::Python3)
            .expect_err("E417 must fire on transition oversupply");
        assert!(errors.iter().any(|e| e.code == "E417"));
    }

    #[test]
    fn test_e407_java_lambda_with_frame_stmt() {
        // Java lambda body containing a Frame transition. The unified
        // scanner's `skip_nested_scope` (Java) detects `-> {`, the
        // scope-content check finds `-> $B`, and E407 surfaces through
        // the enrichment pass before validation.
        let source = r#"
@@system Test {
    machine:
        $A {
            run() {
                Runnable r = () -> {
                    -> $B
                };
                r.run();
            }
        }
        $B { run() {} }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::Java)
            .expect_err("E407 must fire for Java lambda body containing Frame stmt");
        assert!(
            errors.iter().any(|e| e.code == "E407"),
            "expected E407, got {:?}",
            errors.iter().map(|e| e.code.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_e407_typescript_arrow_with_frame_stmt() {
        let source = r#"
@@system Test {
    machine:
        $A {
            run() {
                const cb = () => {
                    -> $B
                };
                cb();
            }
        }
        $B { run() {} }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::TypeScript)
            .expect_err("E407 must fire for TS arrow body");
        assert!(errors.iter().any(|e| e.code == "E407"));
    }

    #[test]
    fn test_e407_rust_closure_with_frame_stmt() {
        let source = r#"
@@system Test {
    machine:
        $A {
            run() {
                let cb = || {
                    -> $B
                };
                cb();
            }
        }
        $B { run() {} }
}"#;
        let errors = validate_frame_source(source, TargetLanguage::Rust)
            .expect_err("E407 must fire for Rust closure body");
        assert!(errors.iter().any(|e| e.code == "E407"));
    }

    #[test]
    fn test_e407_no_false_positive_on_function_arrow_native() {
        // Rust closure with an expression body that returns a value:
        // `|| 1` (no `{`) is *not* skipped by `skip_nested_scope`,
        // and the body has no Frame markers. Compiles clean.
        // Catches the regression where `-> ` (3-byte check) would
        // have flagged any function-pointer / arrow-bearing source.
        let source = r#"
@@system Test {
    interface:
        e()
    machine:
        $A {
            e() {
                let _f: fn() -> i32 = || 1;
            }
        }
}"#;
        let result = validate_frame_source(source, TargetLanguage::Rust);
        if let Err(errors) = &result {
            assert!(
                !errors.iter().any(|e| e.code == "E407"),
                "E407 must not fire for native arrow syntax: {:?}",
                errors
            );
        }
    }

    #[test]
    fn test_e417_transition_default_relaxes_undersupply() {
        // Default-aware relaxation on enter args. Handler `$>(a, b = "x")`
        // accepts 1 or 2 args. Caller supplies 1 — must compile.
        let source = r#"
@@system Test {
    machine:
        $A {
            go() { -> ("just_a") $B }
        }
        $B {
            $>(a: str, b: str = "x") {}
            go() {}
        }
}"#;
        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(
            result.is_ok(),
            "default-relaxed enter args must compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_valid_hsm() {
        let source = r#"
@@system HSM {
    machine:
        $Parent {
            baseEvent() { }
        }
        $Child => $Parent {
            childEvent() { }
            unhandled() { => unhandled() }
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_ok());
    }

    #[test]
    fn test_e113_section_order() {
        // Test duplicate section detection using the AST directly
        let mut system = SystemAst::new("Test".to_string(), Span::new(0, 100));
        system.section_order = vec![
            SystemSectionKind::Machine,
            SystemSectionKind::Interface, // Wrong order - interface should come before machine
        ];

        let mut validator = FrameValidator::new();
        validator.validate_section_order(&system);

        assert!(!validator.errors.is_empty());
        assert!(validator.errors.iter().any(|e| e.code == "E113"));
    }

    #[test]
    fn test_e114_duplicate_section() {
        let mut system = SystemAst::new("Test".to_string(), Span::new(0, 100));
        system.section_order = vec![
            SystemSectionKind::Machine,
            SystemSectionKind::Actions,
            SystemSectionKind::Machine, // Duplicate!
        ];

        let mut validator = FrameValidator::new();
        validator.validate_duplicate_sections(&system);

        assert!(!validator.errors.is_empty());
        assert!(validator.errors.iter().any(|e| e.code == "E114"));
    }

    #[test]
    fn test_valid_section_order() {
        let mut system = SystemAst::new("Test".to_string(), Span::new(0, 100));
        system.section_order = vec![
            SystemSectionKind::Operations,
            SystemSectionKind::Interface,
            SystemSectionKind::Machine,
            SystemSectionKind::Actions,
            SystemSectionKind::Domain,
        ];

        let mut validator = FrameValidator::new();
        validator.validate_section_order(&system);
        validator.validate_duplicate_sections(&system);

        assert!(validator.errors.is_empty());
    }

    #[test]
    fn test_e400_transition_not_last() {
        // Create a handler body where transition is not last
        let body = HandlerBody {
            statements: vec![
                Statement::Transition(TransitionAst {
                    target: "Other".to_string(),
                    args: vec![],
                    label: None,
                    span: Span::new(10, 20),
                    indent: 0,
                    exit_args: None,
                    enter_args: None,
                    state_args: None,
                    is_pop: false,
                    is_forward: false,
                }),
                Statement::Transition(TransitionAst {
                    target: "Final".to_string(),
                    args: vec![],
                    label: None,
                    span: Span::new(30, 40),
                    indent: 0,
                    exit_args: None,
                    enter_args: None,
                    state_args: None,
                    is_pop: false,
                    is_forward: false,
                }),
            ],
            span: Span::new(0, 50),
        };

        let mut validator = FrameValidator::new();
        validator.validate_terminal_last(&body);

        // First transition is not last, but since both are transitions,
        // we only report if there's a non-terminal after a terminal
        // In this case both are terminals so only the last matters
    }

    #[test]
    fn test_validate_with_arcanum() {
        // Happy-path arcanum-backed validation: a system with valid
        // transitions must pass. The V4 pipeline runs Arcanum
        // construction + `validate_with_arcanum` internally, so this
        // exercises the same code path the legacy direct-call test
        // covered.
        let source = r#"
@@system TestArcanum {
    machine:
        $Idle {
            go() { -> $Active() }
        }
        $Active {
            back() { -> $Idle() }
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(
            result.is_ok(),
            "expected clean validation, got {:?}",
            result.err()
        );
    }

    #[test]
    fn test_e406_interface_handler_arity_mismatch() {
        let source = r#"
@@system Test {
    interface:
        process(data: string, count: int): bool

    machine:
        $Active {
            process(data: string) {
                ^ true
            }
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_err());

        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.code == "E406"
            && e.message
                .contains("has 1 parameters but interface method expects 2")));
    }

    #[test]
    fn test_e406_valid_interface_handler() {
        let source = r#"
@@system Test {
    interface:
        process(data: string): bool

    machine:
        $Active {
            process(data: string) {
                ^ true
            }
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_with_arcanum_invalid_state() {
        // Negative arcanum check: transition to an undefined state
        // must surface E402 from the validator's arcanum-backed pass.
        let source = r#"
@@system TestInvalid {
    machine:
        $Start {
            go() { -> $NonExistent() }
        }
}"#;

        let result = validate_frame_source(source, TargetLanguage::Python3);
        assert!(result.is_err());

        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.code == "E402"));
    }

    /// Helper: run the bare-context-ref scanner on a raw handler body
    /// and collect the errors it produces. The scanner is the hot path for
    /// E603/E604; the parser-level invocation is covered by integration
    /// tests via `framec compile`.
    fn scan_bare(body: &[u8]) -> Vec<ValidationError> {
        // Wrap the body in braces for the scanner
        let mut wrapped = Vec::with_capacity(body.len() + 2);
        wrapped.push(b'{');
        wrapped.extend_from_slice(body);
        wrapped.push(b'}');

        let mut v = FrameValidator::new();
        let empty_methods = std::collections::HashMap::new();
        v.validate_frame_segments_in_body(
            &wrapped,
            &empty_methods,
            "TestState",
            "test_evt",
            crate::frame_c::visitors::TargetLanguage::Python3,
        );
        v.errors
    }

    #[test]
    fn test_e603_bare_self_is_error() {
        let errs = scan_bare(b"let x = @@:self");
        assert!(
            errs.iter().any(|e| e.code == "E603"),
            "expected E603, got {:?}",
            errs
        );
    }

    #[test]
    fn test_e604_bare_system_is_error() {
        let errs = scan_bare(b"let x = @@:system");
        assert!(
            errs.iter().any(|e| e.code == "E604"),
            "expected E604, got {:?}",
            errs
        );
    }

    #[test]
    fn test_chained_self_call_does_not_trigger_e603() {
        let errs = scan_bare(b"let y = @@:self.ping()");
        assert!(
            !errs.iter().any(|e| e.code == "E603"),
            "E603 false-fired on @@:self.ping(): {:?}",
            errs
        );
    }

    #[test]
    fn test_chained_system_state_does_not_trigger_e604() {
        let errs = scan_bare(b"let s = @@:system.state");
        assert!(
            !errs.iter().any(|e| e.code == "E604"),
            "E604 false-fired on @@:system.state: {:?}",
            errs
        );
    }

    #[test]
    fn test_native_identifier_sharing_prefix_chars_does_not_false_positive() {
        // `selfish` and `systemic` are native identifiers that share letters
        // with the Frame prefixes but are not prefixed with `@@:`. They must
        // not trigger the new errors.
        let errs = scan_bare(b"let selfish = 1; let systemic = 2");
        assert!(errs.is_empty(), "false positive: {:?}", errs);
    }

    #[test]
    fn test_multiple_bare_refs_each_reported() {
        let errs = scan_bare(b"let x = @@:self; let y = @@:system");
        let e603 = errs.iter().filter(|e| e.code == "E603").count();
        let e604 = errs.iter().filter(|e| e.code == "E604").count();
        assert_eq!(e603, 1, "expected exactly 1 E603, got {:?}", errs);
        assert_eq!(e604, 1, "expected exactly 1 E604, got {:?}", errs);
    }

    /// Run the V4 pipeline and return the error codes.
    fn v4_codes(source: &str) -> Vec<String> {
        use crate::frame_c::compiler::pipeline::compile_module;
        use crate::frame_c::compiler::pipeline::config::PipelineConfig;
        let config = PipelineConfig::production(VTarget::Python3);
        let result = compile_module(source.as_bytes(), &config).expect("pipeline ran");
        result.errors.iter().map(|e| e.code.clone()).collect()
    }

    #[test]
    fn test_e615_assignment_to_const_field() {
        let source = r#"
@@system Sensor {
    interface:
        bump()
    machine:
        $Active {
            bump() {
                self.threshold = 999
            }
        }
    domain:
        const threshold : int = 100
}"#;
        let codes = v4_codes(source);
        assert!(
            codes.iter().any(|c| c == "E615"),
            "expected E615, got {:?}",
            codes
        );
    }

    #[test]
    fn test_e615_not_emitted_for_comparison() {
        let source = r#"
@@system Sensor {
    interface:
        check()
    machine:
        $Active {
            check() {
                if self.threshold == 100:
                    pass
            }
        }
    domain:
        const threshold : int = 100
}"#;
        let codes = v4_codes(source);
        assert!(
            !codes.iter().any(|c| c == "E615"),
            "false-positive E615 on comparison: {:?}",
            codes
        );
    }

    #[test]
    fn test_e615_not_emitted_for_mutable_field_assign() {
        let source = r#"
@@system Sensor {
    interface:
        bump()
    machine:
        $Active {
            bump() {
                self.value = 999
            }
        }
    domain:
        value : int = 0
        const threshold : int = 100
}"#;
        let codes = v4_codes(source);
        assert!(
            !codes.iter().any(|c| c == "E615"),
            "false-positive E615 on mutable assign: {:?}",
            codes
        );
    }

    #[test]
    fn test_e615_augmented_assign_caught() {
        let source = r#"
@@system Sensor {
    interface:
        bump()
    machine:
        $Active {
            bump() {
                self.threshold += 1
            }
        }
    domain:
        const threshold : int = 100
}"#;
        let codes = v4_codes(source);
        assert!(
            codes.iter().any(|c| c == "E615"),
            "expected E615 on +=, got {:?}",
            codes
        );
    }

    #[test]
    fn test_e111_duplicate_system_param() {
        let source = r#"
@@system C(dup, dup) {
    interface:
        bump()
    machine:
        $A {
            bump() { }
        }
    domain:
        dup : int = dup
}"#;
        let codes = v4_codes(source);
        assert!(
            codes.iter().any(|c| c == "E111"),
            "expected E111, got {:?}",
            codes
        );
    }

    #[test]
    fn test_e111_distinct_system_params_pass() {
        let source = r#"
@@system C(a, b) {
    interface:
        bump()
    machine:
        $A {
            bump() { }
        }
    domain:
        a : int = a
        b : int = b
}"#;
        let codes = v4_codes(source);
        assert!(
            !codes.iter().any(|c| c == "E111"),
            "false-positive E111 on distinct params: {:?}",
            codes
        );
    }

    #[test]
    fn test_e416_start_params_mismatch() {
        // System declares start arg `missing`; start state $A has no params.
        let source = r#"
@@system C($(missing)) {
    interface:
        bump()
    machine:
        $A {
            bump() { }
        }
}"#;
        let codes = v4_codes(source);
        assert!(
            codes.iter().any(|c| c == "E416"),
            "expected E416, got {:?}",
            codes
        );
    }

    #[test]
    fn test_e416_start_params_match_pass() {
        let source = r#"
@@system C($(x)) {
    interface:
        bump()
    machine:
        $A(x: int) {
            bump() { }
        }
}"#;
        let codes = v4_codes(source);
        assert!(
            !codes.iter().any(|c| c == "E416"),
            "false-positive E416 when params match: {:?}",
            codes
        );
    }

    #[test]
    fn test_e417_enter_params_no_handler() {
        // System declares enter arg but start state has no $>() handler.
        let source = r#"
@@system C($>(missing)) {
    interface:
        bump()
    machine:
        $A {
            bump() { }
        }
}"#;
        let codes = v4_codes(source);
        assert!(
            codes.iter().any(|c| c == "E417"),
            "expected E417 (no $>() handler), got {:?}",
            codes
        );
    }

    #[test]
    fn test_e418_domain_param_no_match() {
        // Domain param `missing` doesn't match any field name OR init reference.
        let source = r#"
@@system C(missing) {
    interface:
        bump()
    machine:
        $A {
            bump() { }
        }
    domain:
        value : int = 0
}"#;
        let codes = v4_codes(source);
        assert!(
            codes.iter().any(|c| c == "E418"),
            "expected E418, got {:?}",
            codes
        );
    }

    #[test]
    fn test_e418_param_matches_field_name_pass() {
        let source = r#"
@@system C(value) {
    interface:
        bump()
    machine:
        $A {
            bump() { }
        }
    domain:
        value : int = 0
}"#;
        let codes = v4_codes(source);
        assert!(
            !codes.iter().any(|c| c == "E418"),
            "false-positive E418 when param name matches field: {:?}",
            codes
        );
    }

    #[test]
    fn test_e418_param_referenced_in_init_pass() {
        // `initial` doesn't name a field but is referenced in `value: int = initial`.
        let source = r#"
@@system C(initial) {
    interface:
        bump()
    machine:
        $A {
            bump() { }
        }
    domain:
        value : int = initial
}"#;
        let codes = v4_codes(source);
        assert!(
            !codes.iter().any(|c| c == "E418"),
            "false-positive E418 when param referenced in field init: {:?}",
            codes
        );
    }

    // ── RFC-0008: Decorated pop$ transitions ─────────────────────

    /// Helper: compile source for a given target and return the generated code.
    fn v4_output_for(source: &str, target: VTarget) -> String {
        use crate::frame_c::compiler::pipeline::compile_module;
        use crate::frame_c::compiler::pipeline::config::PipelineConfig;
        let config = PipelineConfig::production(target);
        let result = compile_module(source.as_bytes(), &config).expect("pipeline ran");
        assert!(
            result.errors.is_empty(),
            "{:?} compilation errors: {:?}",
            target,
            result.errors
        );
        result.code
    }

    fn v4_output(source: &str) -> String {
        v4_output_for(source, VTarget::Python3)
    }

    /// Frame source with bare -> pop$ (regression baseline)
    const POP_BARE: &str = r#"
@@system S {
    interface:
        go()
        back()
    machine:
        $A {
            go() {
                push$
                -> $B
            }
        }
        $B {
            back() {
                -> pop$
            }
        }
}"#;

    /// Frame source with exit args on pop: ("finished") -> pop$
    const POP_EXIT: &str = r#"
@@system S {
    interface:
        go()
        done()
    machine:
        $A {
            go() {
                push$
                -> $B
            }
        }
        $B {
            done() {
                ("finished") -> pop$
            }
        }
}"#;

    /// Frame source with enter args on pop: -> (42) pop$
    const POP_ENTER: &str = r#"
@@system S {
    interface:
        go()
        done()
    machine:
        $A {
            go() {
                push$
                -> $B
            }
        }
        $B {
            done() {
                -> (42) pop$
            }
        }
}"#;

    /// Frame source with event forwarding on pop: -> => pop$
    const POP_FORWARD: &str = r#"
@@system S {
    interface:
        go()
        done()
    machine:
        $A {
            go() {
                push$
                -> $B
            }
        }
        $B {
            done() {
                -> => pop$
            }
        }
}"#;

    // All 17 targets (excluding Graphviz — not a runtime target)
    const ALL_TARGETS: &[VTarget] = &[
        VTarget::Python3,
        VTarget::TypeScript,
        VTarget::JavaScript,
        VTarget::Dart,
        VTarget::Rust,
        VTarget::C,
        VTarget::Cpp,
        VTarget::Java,
        VTarget::Kotlin,
        VTarget::Swift,
        VTarget::CSharp,
        VTarget::Go,
        VTarget::Php,
        VTarget::Ruby,
        VTarget::Lua,
        VTarget::GDScript,
        VTarget::Erlang,
    ];

    #[test]
    fn test_bare_pop_all_backends() {
        for &target in ALL_TARGETS {
            let code = v4_output_for(POP_BARE, target);
            assert!(
                !code.is_empty(),
                "{:?}: bare pop$ produced empty output",
                target
            );
        }
    }

    #[test]
    fn test_pop_exit_args_all_backends() {
        for &target in ALL_TARGETS {
            if matches!(target, VTarget::Erlang) {
                continue; // Erlang handles pop via gen_statem
            }
            let code = v4_output_for(POP_EXIT, target);
            assert!(
                code.contains("exit_args") || code.contains("exitArgs"),
                "{:?}: pop with exit args should write exit_args:\n{}",
                target,
                code
            );
        }
    }

    #[test]
    fn test_pop_enter_args_all_backends() {
        for &target in ALL_TARGETS {
            if matches!(target, VTarget::Erlang) {
                continue;
            }
            let code = v4_output_for(POP_ENTER, target);
            assert!(
                code.contains("enter_args") || code.contains("enterArgs"),
                "{:?}: pop with enter args should write enter_args:\n{}",
                target,
                code
            );
        }
    }

    #[test]
    fn test_pop_forward_all_backends() {
        for &target in ALL_TARGETS {
            if matches!(target, VTarget::Erlang) {
                continue;
            }
            let code = v4_output_for(POP_FORWARD, target);
            assert!(
                code.contains("forward_event")
                    || code.contains("forwardEvent")
                    || code.contains("forward_event"),
                "{:?}: pop with forward should set forward_event:\n{}",
                target,
                code
            );
        }
    }
}
