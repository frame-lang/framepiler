//! Cross-system registry used by nested-system codegen paths.
//!
//! Two thread-local registries, populated by the pipeline once per
//! compilation (before per-system codegen runs) and read by the
//! per-system code generators:
//!
//! - `NEW_CONTRACT_SYSTEMS` — set of system names that use the
//!   RFC-0012 amendment new persist contract (have `@@[save]` and/or
//!   `@@[load]` ops). Nested-system restore emission has to branch on
//!   the inner system's contract here: a new-contract `Inner` has only
//!   the instance-method `load`, not the legacy static factory, so the
//!   parent's restore code spells the call differently.
//!
//! - `NESTED_SYSTEM_DOMAIN_PARAMS` — `system_name → [(param_name,
//!   type_string), ...]`. The bare-form `@@system Inner(seed: int)`
//!   params (Domain-kind only, not state- or enter-args). Populated
//!   alongside `NEW_CONTRACT_SYSTEMS` so nested-system restore can
//!   extract the saved values from the child's saved JSON and pass
//!   them to the constructor — fixes FRAMEC_BUGS.md Issue #2
//!   (parameterized sub-system zero-arg restore crash).
//!
//! Both setters are called from `pipeline::compiler`. Both getters
//! are called from the per-system codegen path (notably
//! `rust_system` for the RFC-0012 contract switch).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

thread_local! {
    static NEW_CONTRACT_SYSTEMS: RefCell<HashSet<String>> =
        RefCell::new(HashSet::new());

    static NESTED_SYSTEM_DOMAIN_PARAMS: RefCell<HashMap<String, Vec<(String, String)>>> =
        RefCell::new(HashMap::new());
}

/// Set the names of systems using the new persist contract. Called
/// once per compilation, before per-system codegen runs.
pub fn set_new_contract_systems(names: HashSet<String>) {
    NEW_CONTRACT_SYSTEMS.with(|s| *s.borrow_mut() = names);
}

/// Set the per-system Domain-kind param signatures. Called once per
/// compilation, before per-system codegen runs. Each (param_name,
/// type) pair is in declaration order.
pub fn set_nested_system_domain_params(map: HashMap<String, Vec<(String, String)>>) {
    NESTED_SYSTEM_DOMAIN_PARAMS.with(|s| *s.borrow_mut() = map);
}

/// True if a system uses the new persist contract. Used by nested-
/// system restore emission to pick the right call shape.
pub fn nested_uses_new_contract(name: &str) -> bool {
    NEW_CONTRACT_SYSTEMS.with(|s| s.borrow().contains(name))
}

/// Get the Domain-kind params of a nested system by name. Returns an
/// empty Vec if the system isn't registered or has no Domain params.
pub fn get_nested_system_domain_params(name: &str) -> Vec<(String, String)> {
    NESTED_SYSTEM_DOMAIN_PARAMS.with(|s| s.borrow().get(name).cloned().unwrap_or_default())
}
