//! System-level validation: self-call routing, system-instantiation arity,
//! and the scanner-based walk that authenticates Frame `@@` segments inside
//! handler/action bodies.
//!
//! These methods catch:
//! - E416/E417/E418 — `@@:self(...)` shape and receiver
//! - E412/E413 — cross-system instantiation references and arity
//! - E419/E420 — `@@:return(...)` / `@@:(value)` placement and arity inside
//!   handler and action bodies (with terminal-statement awareness)

use super::{count_args, FrameValidator, ValidationError};
use crate::frame_c::compiler::codegen::frame_expansion::get_native_scanner;
use crate::frame_c::compiler::frame_ast::*;
use crate::frame_c::compiler::native_region_scanner::{FrameSegmentKind, Region, SegmentMetadata};
use std::collections::{HashMap, HashSet};

impl FrameValidator {
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

    pub(super) fn validate_system_self_calls(
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

    /// RFC-0015 D7: validate `@@SystemName(args)` and `@@!SystemName()` call
    /// sites against the set of declared systems and the kind-specific rules.
    ///
    /// - **E820**: `@@!Foo(args)` with non-empty args is rejected. The
    ///   no-initialization form is zero-arg by definition.
    /// - **E821**: `@@SystemName(...)` (or `@@!SystemName()`) referencing a
    ///   system not declared in the module is rejected.
    pub fn validate_system_instantiations(
        &mut self,
        ast: &FrameAst,
        source: &[u8],
        target: crate::frame_c::visitors::TargetLanguage,
    ) -> Result<(), Vec<ValidationError>> {
        let defined_systems: std::collections::HashSet<String> = match ast {
            FrameAst::System(s) => std::iter::once(s.name.clone()).collect(),
            FrameAst::Module(m) => m.systems.iter().map(|s| s.name.clone()).collect(),
        };

        match ast {
            FrameAst::System(system) => {
                self.validate_system_instantiations_in_system(
                    system,
                    source,
                    target,
                    &defined_systems,
                );
            }
            FrameAst::Module(module) => {
                for system in &module.systems {
                    self.validate_system_instantiations_in_system(
                        system,
                        source,
                        target,
                        &defined_systems,
                    );
                }
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    pub(super) fn validate_system_instantiations_in_system(
        &mut self,
        system: &SystemAst,
        source: &[u8],
        target: crate::frame_c::visitors::TargetLanguage,
        defined_systems: &std::collections::HashSet<String>,
    ) {
        if let Some(machine) = &system.machine {
            for state in &machine.states {
                for handler in &state.handlers {
                    let span = &handler.span;
                    if span.start >= source.len() || span.end > source.len() {
                        continue;
                    }
                    let body = &source[span.start..span.end];
                    self.validate_system_instantiations_in_body(body, target, defined_systems);
                }
            }
        }
        for action in &system.actions {
            let span = &action.span;
            if span.start >= source.len() || span.end > source.len() {
                continue;
            }
            let body = &source[span.start..span.end];
            self.validate_system_instantiations_in_body(body, target, defined_systems);
        }
    }

    pub(super) fn validate_system_instantiations_in_body(
        &mut self,
        body: &[u8],
        target: crate::frame_c::visitors::TargetLanguage,
        defined_systems: &std::collections::HashSet<String>,
    ) {
        use crate::frame_c::compiler::frame_ast::InstantiationKind;
        use crate::frame_c::compiler::native_region_scanner::{
            FrameSegmentKind, Region, SegmentMetadata,
        };

        let open_brace = match body.iter().position(|&b| b == b'{') {
            Some(pos) => pos,
            None => return,
        };
        let mut scanner = get_native_scanner(target);
        let scan_result = match scanner.scan(body, open_brace) {
            Ok(r) => r,
            Err(_) => return,
        };

        for region in &scan_result.regions {
            if let Region::FrameSegment {
                kind: FrameSegmentKind::SystemInstantiation,
                metadata:
                    SegmentMetadata::SystemInstantiation {
                        system_name,
                        args,
                        kind: inst_kind,
                    },
                ..
            } = region
            {
                // E820: no-initialization allocation must be zero-arg.
                if *inst_kind == InstantiationKind::NoInitialization {
                    let inner = args.trim_start_matches('(').trim_end_matches(')').trim();
                    if !inner.is_empty() {
                        self.errors.push(ValidationError::new(
                            "E820",
                            format!(
                                "no-initialization allocation `@@!{}({})` must be zero-arg; received: `{}`",
                                system_name, inner, inner
                            ),
                        ));
                    }
                }

                // E821: referenced system must be declared in the module.
                if !defined_systems.contains(system_name) {
                    let prefix = if *inst_kind == InstantiationKind::NoInitialization {
                        "@@!"
                    } else {
                        "@@"
                    };
                    let mut known: Vec<&String> = defined_systems.iter().collect();
                    known.sort();
                    self.errors.push(ValidationError::new(
                        "E821",
                        format!(
                            "undefined system '{}' in `{}{}{}` — known systems: {:?}",
                            system_name, prefix, system_name, args, known
                        ),
                    ));
                }
            }
        }
    }

    /// Validate Frame segments in a handler/action body using the scanner.
    /// Runs the language-specific scanner on the body text, then walks the
    /// identified segments. No byte-level scanning — the scanner handles
    /// comments, strings, and language-specific syntax.
    pub(super) fn validate_frame_segments_in_body(
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

        // W705: transition in a non-void handler without a preceding
        // `@@:(value)` may leak the return type's default (None /
        // Nil / null / 0) on the transition's execution path.
        //
        // Per `frame_language.md`: "Every transition is implicitly
        // followed by a `return` — code after a transition is
        // unreachable." The codegen's same-scope hoist makes the
        // simple shape `-> $X; @@:(value)` work (the @@:(value)
        // gets reordered before the bare return), but a
        // `@@:(value)` in an enclosing scope after the transition
        // remains genuinely unreachable on the transition path.
        //
        // Two safe shapes that suppress this warning:
        //   1. `@@:(value)` (or `@@:return(value)`) appears earlier
        //      in the body at an indent ≤ the transition's indent —
        //      `_return` was already set before the transition runs.
        //   2. `@@:(value)` immediately follows the transition at
        //      the same indent — the codegen hoists it before the
        //      bare return.
        //
        // The check is intentionally heuristic. It catches the
        // common "I wrote `@@:(value)` outside the if; why is it
        // returning Nil?" mistake (Issue #4 in FRAMEC_BUGS.md). It
        // can produce a false negative for sibling-block cases
        // where an earlier @@:(value) exists in a non-preceding
        // branch — that's accepted; the warning is meant to catch
        // the easy mistake without flagging legitimate patterns.
        if let Some(iface_method) = interface_methods.get(scope_inner) {
            // A handler "returns a value" if EITHER the interface
            // declares an explicit return type (`: int`) OR a default
            // return expression (`= "denied"`). Dynamic-typed targets
            // (Ruby, Lua, PHP, JS) commonly drop the type annotation
            // and rely on the default-expression form — `get_status()
            // = ""` is "returns a value, defaulting to empty string."
            let returns_value = {
                let has_type = match &iface_method.return_type {
                    Some(t) => {
                        let s = match t {
                            crate::frame_c::compiler::frame_ast::Type::Custom(s) => s.as_str(),
                            crate::frame_c::compiler::frame_ast::Type::Unknown => "",
                        };
                        !s.is_empty() && s != "void"
                    }
                    None => false,
                };
                has_type || iface_method.return_init.is_some()
            };
            // E606: `@@:(value)` (or `@@:return(value)`) in a handler
            // whose interface method is void. The write to `_return` has
            // no observable effect — the caller has no typed read path
            // for the value.
            //
            // RUST-ONLY: pre-Track-B `Box<dyn Any>` accepted this silently
            // on every backend, but Track B's per-event return enum on
            // the Rust target exposes it as a structural error (no enum
            // variant exists to write into). The other 16 backends still
            // use dynamic dispatch and tolerate the dead write — gating
            // this validator pass to Rust avoids breaking ~70 fixtures
            // across dynamic-typed targets where the pattern is benign.
            if !returns_value
                && matches!(target, crate::frame_c::visitors::TargetLanguage::Rust)
            {
                for r in scan_result.regions.iter() {
                    if let Region::FrameSegment { kind: k, .. } = r {
                        if matches!(
                            k,
                            FrameSegmentKind::ContextReturnExpr
                                | FrameSegmentKind::ReturnCall
                        ) {
                            self.errors.push(ValidationError::new(
                                "E606",
                                format!(
                                    "`@@:(value)` in {}/{} — interface method `{}` is void on the Rust target, so writing to `_return` has no observable effect (and Track B's per-event return enum has no variant for it). Remove the `@@:(value)` (or add a return type to `{}` in the interface).",
                                    scope_outer, scope_inner, scope_inner, scope_inner
                                ),
                            ));
                            break; // one error per handler is enough
                        }
                    }
                }
            }

            if returns_value {
                let frame_regs: Vec<&Region> = scan_result
                    .regions
                    .iter()
                    .filter(|r| matches!(r, Region::FrameSegment { .. }))
                    .collect();
                for (i, r) in frame_regs.iter().enumerate() {
                    if let Region::FrameSegment {
                        kind: FrameSegmentKind::Transition,
                        indent: t_indent,
                        ..
                    } = **r
                    {
                        // Check 1: any @@:(value) at indent ≤ t_indent earlier in body.
                        let preceded = frame_regs[..i].iter().any(|r2| {
                            if let Region::FrameSegment {
                                kind: k,
                                indent: i2,
                                ..
                            } = **r2
                            {
                                matches!(
                                    k,
                                    FrameSegmentKind::ContextReturnExpr
                                        | FrameSegmentKind::ReturnCall
                                ) && i2 <= t_indent
                            } else {
                                false
                            }
                        });
                        // Check 2: same-indent @@:(value) immediately following
                        // (codegen's same-scope hoist applies).
                        let immediately_followed = frame_regs
                            .get(i + 1)
                            .map(|r2| {
                                if let Region::FrameSegment {
                                    kind: k,
                                    indent: i2,
                                    ..
                                } = **r2
                                {
                                    matches!(
                                        k,
                                        FrameSegmentKind::ContextReturnExpr
                                            | FrameSegmentKind::ReturnCall
                                    ) && i2 == t_indent
                                } else {
                                    false
                                }
                            })
                            .unwrap_or(false);
                        if !preceded && !immediately_followed {
                            self.warnings.push(ValidationError::new(
                                "W705",
                                format!(
                                    "transition in {}/{} may leak the return type's default value \
                                     (None/Nil/null/0): no `@@:(value)` precedes the transition at \
                                     this scope or any enclosing scope, and no same-scope `@@:(value)` \
                                     immediately follows it. The transition's implicit `return` will \
                                     short-circuit before any later `@@:(value)` in an outer scope. \
                                     Fix: place `@@:(value)` before the transition, or use \
                                     `@@:return(value)` at the transition site.",
                                    scope_outer, scope_inner
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

}
