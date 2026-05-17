//! Attribute-related validation methods on `FrameValidator`.
//!
//! RFC-0013 per-item `@@[name(args?)]` attribute validation
//! (E800/E801/E802), system-parameter semantics that cross-
//! check the header param list against start-state args and
//! domain declarations (E416/E417/E418), and RFC-0015
//! system-level lifecycle attribute parsing (E817/E818).

use super::{identifier_appears_in, FrameValidator, ValidationError};
use crate::frame_c::compiler::frame_ast::{ParamKind, SystemAst};
use crate::frame_c::visitors::TargetLanguage;
use std::collections::HashSet;

impl FrameValidator {
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
    pub(super) fn validate_attributes(&mut self, system: &SystemAst) {
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
                // RFC-0015: `@@[create]` is system-level only. There is
                // no operation-attribute form for create — user code
                // never lives in the factory body. If `@@[create]`
                // appears on an interface method, handler, domain
                // field, or operation, reject with E815 and point to
                // the system-level form.
                "create" => errs.push(
                    ValidationError::new(
                        "E815",
                        format!(
                            "@@[create] is a system-level attribute only; not valid on {}. \
                             Place it above the @@system declaration as @@[create] or \
                             @@[create(name)].",
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
                    // E819: RFC-0015 hard-cut. The operation-attribute
                    // form is replaced by system-level
                    // `@@[save(<name>)]` / `@@[load(<name>)]`. The
                    // codemod at scripts/migrate_rfc0015.py converts
                    // the old shape mechanically.
                    errs.push(
                        ValidationError::new(
                            "E819",
                            format!(
                                "@@[{0}] on operation '{1}' is no longer accepted (RFC-0015 \
                                 hard-cut). Use the system-level form instead:\n\
                                 \x20   @@[persist(<type>)]\n\
                                 \x20   @@[save(<name>)]\n\
                                 \x20   @@[load(<name>)]\n\
                                 \x20   @@system {2} {{ … }}\n\
                                 The codemod at scripts/migrate_rfc0015.py rewrites old \
                                 fixtures automatically.",
                                a.name, op.name, system.name
                            ),
                        )
                        .with_span(a.span.clone()),
                    );
                    // Continue counting for E810 so multiple-decl
                    // diagnostics still surface alongside E819.
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
        // RFC-0015: also accept the system-level form
        // (`@@[save(name)]` / `@@[load(name)]` at module level).
        let has_rfc0015_save = system.save_op_name_rfc0015().is_some()
            || system.attributes.iter().any(|a| a.name == "save");
        let has_rfc0015_load = system.load_op_name_rfc0015().is_some()
            || system.attributes.iter().any(|a| a.name == "load");
        if system.persist_attr.is_some()
            && save_count == 0
            && load_count == 0
            && !has_rfc0015_save
            && !has_rfc0015_load
        {
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
                         methods\". RFC-0015 also accepts the equivalent system-level form: \
                         `@@[save(name)]` / `@@[load(name)]` above the `@@system` declaration.",
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
    pub(super) fn validate_system_param_semantics(&mut self, system: &SystemAst) {
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

    /// RFC-0015 system-level lifecycle attributes:
    ///   `@@[create(<name>?)]`, `@@[save(<name>?)]`, `@@[load(<name>?)]`
    ///
    /// - **E817**: name argument, when present, must be a valid identifier
    ///   (alphanumeric + underscore, starting with letter or underscore).
    ///   This rule also catches multi-argument forms like `@@[create(a, b)]`
    ///   because the parsed arg fails the identifier shape.
    /// - **E818**: at most one of each lifecycle attribute per system.
    ///   `@@[create(a)]` followed by `@@[create(b)]` on the same system is
    ///   ambiguous and rejected.
    pub(super) fn validate_rfc0015_lifecycle_attrs(&mut self, system: &SystemAst) {
        fn is_valid_identifier(s: &str) -> bool {
            if s.is_empty() {
                return false;
            }
            let mut chars = s.chars();
            let first = chars.next().expect("non-empty: is_empty() checked above");
            if !(first.is_ascii_alphabetic() || first == '_') {
                return false;
            }
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }

        let mut create_count = 0;
        let mut save_count = 0;
        let mut load_count = 0;

        for attr in &system.attributes {
            let name = attr.name.as_str();
            if !matches!(name, "create" | "save" | "load") {
                continue;
            }

            match name {
                "create" => create_count += 1,
                "save" => save_count += 1,
                "load" => load_count += 1,
                _ => unreachable!(),
            }

            // E817: argument shape — must be a valid host identifier.
            if let Some(arg) = &attr.args {
                let trimmed = arg.trim();
                if !is_valid_identifier(trimmed) {
                    self.errors.push(
                        ValidationError::new(
                            "E817",
                            format!(
                                "@@[{}({})] argument must be a single valid \
                                 identifier (alphanumeric + underscore, starting \
                                 with letter or underscore). Multi-argument forms \
                                 like @@[{}(a, b)] are not supported — RFC-0015 \
                                 lifecycle attributes take exactly one optional \
                                 name argument.",
                                name, arg, name
                            ),
                        )
                        .with_span(attr.span.clone()),
                    );
                }
            }
        }

        // E818: at most one of each.
        if create_count > 1 {
            self.errors.push(ValidationError::new(
                "E818",
                format!(
                    "System '{}' declares {} @@[create] attributes; at most one is \
                     allowed per system.",
                    system.name, create_count
                ),
            ));
        }
        if save_count > 1 {
            self.errors.push(ValidationError::new(
                "E818",
                format!(
                    "System '{}' declares {} @@[save] attributes; at most one is \
                     allowed per system.",
                    system.name, save_count
                ),
            ));
        }
        if load_count > 1 {
            self.errors.push(ValidationError::new(
                "E818",
                format!(
                    "System '{}' declares {} @@[load] attributes; at most one is \
                     allowed per system.",
                    system.name, load_count
                ),
            ));
        }
    }
}
