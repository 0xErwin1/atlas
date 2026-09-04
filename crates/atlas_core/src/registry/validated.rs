use std::collections::BTreeMap;

use super::{ComponentEntry, ComponentId};

/// A validated, frozen registry (SHELL-REG-3). `Registry` has private fields
/// and is constructible only via `build()`: holding one is proof the
/// validation matrix passed.
#[derive(Debug)]
pub struct Registry {
    entries: Vec<ComponentEntry>,
    index: BTreeMap<ComponentId, usize>,
    migration_order: Vec<ComponentId>,
    startup_order: Vec<ComponentId>,
}

impl Registry {
    pub(super) fn new(
        entries: Vec<ComponentEntry>,
        index: BTreeMap<ComponentId, usize>,
        migration_order: Vec<ComponentId>,
        startup_order: Vec<ComponentId>,
    ) -> Self {
        Self {
            entries,
            index,
            migration_order,
            startup_order,
        }
    }

    /// Returns every entry in the exact order passed to `build()`.
    pub fn entries(&self) -> &[ComponentEntry] {
        &self.entries
    }

    /// Looks up an entry by its `ComponentId`.
    pub fn get(&self, id: &ComponentId) -> Option<&ComponentEntry> {
        self.index
            .get(id)
            .and_then(|position| self.entries.get(*position))
    }

    /// Returns the migration order computed by `validate_migration_order`.
    pub fn migration_order(&self) -> &[ComponentId] {
        &self.migration_order
    }

    /// Returns the worker startup order: a topological sort over dependency
    /// edges merged with capability provider→consumer edges, restricted to
    /// components that declare at least one worker (E11-S2 design D2.1).
    /// Drain order is this slice's exact reverse, computed by the caller.
    pub fn startup_order(&self) -> &[ComponentId] {
        &self.startup_order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{
        Api, Authorization, Capabilities, ComponentKind, ContractVersion, Diagnostics, Experience,
        Identity,
    };

    fn minimal_entry(stable_id: &str) -> ComponentEntry {
        ComponentEntry {
            identity: Identity {
                stable_id: ComponentId::new(stable_id).expect("valid component id"),
                kind: ComponentKind::Module,
                contract_version: ContractVersion::new(1),
            },
            dependencies: vec![],
            capabilities: Capabilities {
                provided: vec![],
                required_mandatory: vec![],
                required_optional: vec![],
            },
            api: Api {
                namespace: None,
                routes: vec![],
                dto_owner: None,
            },
            authorization: Authorization {
                resource_kinds: vec![],
                actions: vec![],
                role_definitions: vec![],
                principal_sets: vec![],
                provider: false,
            },
            diagnostics: Diagnostics {
                health: false,
                readiness: false,
                doctor: false,
            },
            experience: Experience {
                navigation_providers: vec![],
                context_providers: vec![],
            },
            persistence: None,
            config: None,
            workers: vec![],
            satellites: vec![],
        }
    }

    fn registry_with(stable_ids: &[&str]) -> Registry {
        let entries: Vec<ComponentEntry> = stable_ids
            .iter()
            .map(|stable_id| minimal_entry(stable_id))
            .collect();
        let index: BTreeMap<ComponentId, usize> = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.identity.stable_id.clone(), position))
            .collect();
        let migration_order = entries
            .iter()
            .map(|entry| entry.identity.stable_id.clone())
            .collect();
        let startup_order = Vec::new();

        Registry::new(entries, index, migration_order, startup_order)
    }

    #[test]
    fn entries_preserve_input_order() {
        let registry = registry_with(&["platform", "custos", "acta"]);

        let ids: Vec<&str> = registry
            .entries()
            .iter()
            .map(|entry| entry.identity.stable_id.as_str())
            .collect();
        assert_eq!(ids, vec!["platform", "custos", "acta"]);
    }

    #[test]
    fn get_resolves_a_known_id() {
        let registry = registry_with(&["platform", "acta"]);
        let id = ComponentId::new("acta").expect("valid component id");

        let entry = registry.get(&id).expect("acta is in the registry");
        assert_eq!(entry.identity.stable_id.as_str(), "acta");
    }

    #[test]
    fn get_misses_cleanly_for_unknown_id() {
        let registry = registry_with(&["platform"]);
        let id = ComponentId::new("missing").expect("valid component id");

        assert!(registry.get(&id).is_none());
    }

    #[test]
    fn migration_order_returns_constructor_input() {
        let registry = registry_with(&["platform", "custos", "acta"]);

        let order: Vec<&str> = registry
            .migration_order()
            .iter()
            .map(ComponentId::as_str)
            .collect();
        assert_eq!(order, vec!["platform", "custos", "acta"]);
    }
}
