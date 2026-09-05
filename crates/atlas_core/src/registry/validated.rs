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

    /// The readiness-mandatory component set (E11-S2 design D4.2, §0.6):
    /// every entry with `diagnostics.readiness == true`. Ordered by
    /// `startup_order()` for the components present in it (worker-bearing
    /// components respect dependency/capability order); every other
    /// mandatory component — one that declares no worker yet, such as
    /// today's `platform`/`custos` — is placed ahead of them, in
    /// `entries()` insertion order. This split is a stated scope note, not
    /// a permanent rule: once every mandatory component declares a worker,
    /// `startup_order()` alone determines the whole result.
    pub fn readiness_components(&self) -> Vec<ComponentId> {
        self.components_where(|entry| entry.diagnostics.readiness)
    }

    /// The doctor-bearing component set (E11-S3b design D5): every entry
    /// with `diagnostics.doctor == true`, ordered the same way as
    /// [`Self::readiness_components`] — `startup_order()` for the
    /// components present in it, then every other doctor-bearing component
    /// in `entries()` insertion order.
    pub fn doctor_components(&self) -> Vec<ComponentId> {
        self.components_where(|entry| entry.diagnostics.doctor)
    }

    fn components_where(&self, predicate: impl Fn(&ComponentEntry) -> bool) -> Vec<ComponentId> {
        let matching: Vec<&ComponentId> = self
            .entries
            .iter()
            .filter(|entry| predicate(entry))
            .map(|entry| &entry.identity.stable_id)
            .collect();

        let mut ordered: Vec<ComponentId> = matching
            .iter()
            .filter(|id| !self.startup_order.contains(id))
            .map(|id| (*id).clone())
            .collect();

        for id in &self.startup_order {
            if matching.contains(&id) {
                ordered.push(id.clone());
            }
        }

        ordered
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

    fn readiness_entry(stable_id: &str, readiness: bool) -> ComponentEntry {
        let mut entry = minimal_entry(stable_id);
        entry.diagnostics.readiness = readiness;
        entry
    }

    #[test]
    fn readiness_components_uses_startup_order_when_every_mandatory_component_declares_a_worker() {
        // "reg5-shaped synthetic entries": platform, custos and acta all
        // declare a worker and are all readiness-mandatory, inserted out of
        // dependency order to prove the result is derived from
        // `startup_order()`, not from insertion order.
        let entries = vec![
            readiness_entry("acta", true),
            readiness_entry("custos", true),
            readiness_entry("platform", true),
        ];
        let index: BTreeMap<ComponentId, usize> = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.identity.stable_id.clone(), position))
            .collect();
        let migration_order = vec![
            ComponentId::new("platform").expect("valid component id"),
            ComponentId::new("custos").expect("valid component id"),
            ComponentId::new("acta").expect("valid component id"),
        ];
        let startup_order = migration_order.clone();

        let registry = Registry::new(entries, index, migration_order, startup_order);

        let mandatory: Vec<String> = registry
            .readiness_components()
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect();
        assert_eq!(mandatory, vec!["platform", "custos", "acta"]);
    }

    #[test]
    fn readiness_components_places_workerless_mandatory_components_ahead_of_worker_bearing_ones() {
        // Today's real REG-5 shape: `platform`/`custos` are
        // readiness-mandatory but declare no worker (absent from
        // `startup_order()`), while `acta` declares one.
        let entries = vec![
            readiness_entry("platform", true),
            readiness_entry("custos", true),
            readiness_entry("acta", true),
        ];
        let index: BTreeMap<ComponentId, usize> = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.identity.stable_id.clone(), position))
            .collect();
        let migration_order = vec![
            ComponentId::new("platform").expect("valid component id"),
            ComponentId::new("custos").expect("valid component id"),
            ComponentId::new("acta").expect("valid component id"),
        ];
        let startup_order = vec![ComponentId::new("acta").expect("valid component id")];

        let registry = Registry::new(entries, index, migration_order, startup_order);

        let mandatory: Vec<String> = registry
            .readiness_components()
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect();
        assert_eq!(mandatory, vec!["platform", "custos", "acta"]);
    }

    #[test]
    fn readiness_components_excludes_non_mandatory_entries() {
        let entries = vec![
            readiness_entry("platform", true),
            readiness_entry("storage.filesystem", false),
        ];
        let index: BTreeMap<ComponentId, usize> = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.identity.stable_id.clone(), position))
            .collect();
        let migration_order = vec![ComponentId::new("platform").expect("valid component id")];
        let startup_order = Vec::new();

        let registry = Registry::new(entries, index, migration_order, startup_order);

        let mandatory: Vec<String> = registry
            .readiness_components()
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect();
        assert_eq!(mandatory, vec!["platform"]);
    }

    fn doctor_entry(stable_id: &str, doctor: bool) -> ComponentEntry {
        let mut entry = minimal_entry(stable_id);
        entry.diagnostics.doctor = doctor;
        entry
    }

    #[test]
    fn doctor_components_mirrors_readiness_components_ordering_rule() {
        let entries = vec![
            doctor_entry("platform", true),
            doctor_entry("custos", true),
            doctor_entry("acta", true),
        ];
        let index: BTreeMap<ComponentId, usize> = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.identity.stable_id.clone(), position))
            .collect();
        let migration_order = vec![
            ComponentId::new("platform").expect("valid component id"),
            ComponentId::new("custos").expect("valid component id"),
            ComponentId::new("acta").expect("valid component id"),
        ];
        let startup_order = vec![ComponentId::new("acta").expect("valid component id")];

        let registry = Registry::new(entries, index, migration_order, startup_order);

        let doctors: Vec<String> = registry
            .doctor_components()
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect();
        assert_eq!(
            doctors,
            vec!["platform", "custos", "acta"],
            "workerless doctor components precede worker-bearing ones, same as readiness_components"
        );
    }

    #[test]
    fn doctor_components_is_empty_when_no_entry_declares_a_doctor() {
        let entries = vec![
            doctor_entry("platform", false),
            doctor_entry("custos", false),
        ];
        let index: BTreeMap<ComponentId, usize> = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.identity.stable_id.clone(), position))
            .collect();
        let migration_order = vec![ComponentId::new("platform").expect("valid component id")];
        let startup_order = Vec::new();

        let registry = Registry::new(entries, index, migration_order, startup_order);

        assert!(registry.doctor_components().is_empty());
    }
}
