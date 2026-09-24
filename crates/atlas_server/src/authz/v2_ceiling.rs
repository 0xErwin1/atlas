//! The V2 credential ceiling (`v2-e5-s5b-grant-authority`, D-S5b-3;
//! `v2-e7-s2`, ACTA-AUTHZ-7): what a credential may exercise at most,
//! before any grant is considered.
//!
//! A human session is unrestricted: the session carries the user's whole
//! authority. An API key is restricted to its V1 `scopes`, translated into
//! the V2 vocabulary through the explicit table below: each plural V1
//! family/action stands for the singular V2 actions of the kind it names.
//! `grants:read` stays `custos::grant::read`. No capability maps to
//! `custos::grant::create`, so a key can never delegate (GRANT-4, S7).

use atlas_core::ids::ActionId;
use atlas_custos::capability::{Capability, CapabilityAction, CapabilityFamily};
use atlas_custos::eval::Ceiling;

/// The ceiling of a human session: the user's authority is not narrowed by
/// the credential.
pub fn session_ceiling() -> Ceiling {
    Ceiling::Unrestricted
}

/// The ceiling of an API key with `scopes`: exactly the V2 actions the
/// scopes translate to, which never include `custos::grant::create`.
pub fn api_key_ceiling(scopes: &[Capability]) -> Ceiling {
    Ceiling::Restricted(scopes.iter().flat_map(v2_actions_for).collect())
}

/// The V2 actions one V1 capability stands for. The table is deliberately
/// explicit per family and action; an entry absent here translates to
/// nothing, never to a guessed action.
pub fn v2_actions_for(capability: &Capability) -> Vec<ActionId> {
    use CapabilityAction::{Create, Delete, Read, Update};
    use CapabilityFamily as F;

    let (product, kind, actions): (&str, &str, &[&str]) =
        match (capability.family, capability.action) {
            (F::Tasks, Read) => ("acta", "task", &["read", "read_activity"]),
            (F::Tasks, Create) => ("acta", "task", &["create"]),
            (F::Tasks, Update) => (
                "acta",
                "task",
                &[
                    "update",
                    "move",
                    "assign",
                    "reference",
                    "comment",
                    "attach",
                    "manage_checklist",
                ],
            ),
            (F::Tasks, Delete) => ("acta", "task", &["delete"]),
            (F::Docs, Read) => ("acta", "document", &["read", "read_history", "presence"]),
            (F::Docs, Create) => ("acta", "document", &["create"]),
            (F::Docs, Update) => (
                "acta",
                "document",
                &[
                    "update",
                    "update_content",
                    "move",
                    "copy",
                    "comment",
                    "attach",
                ],
            ),
            (F::Docs, Delete) => ("acta", "document", &["delete"]),
            (F::Boards, Read) => ("acta", "board", &["read", "presence"]),
            (F::Boards, Create) => ("acta", "board", &["create"]),
            (F::Boards, Update) => (
                "acta",
                "board",
                &["update", "move", "archive", "manage_columns"],
            ),
            (F::Boards, Delete) => ("acta", "board", &["delete"]),
            (F::Folders, Read) => ("acta", "folder", &["read"]),
            (F::Folders, Create) => ("acta", "folder", &["create"]),
            (F::Folders, Update) => ("acta", "folder", &["update", "move", "copy"]),
            (F::Folders, Delete) => ("acta", "folder", &["delete"]),
            (F::Projects, Read) => ("acta", "project", &["read"]),
            (F::Projects, Create) => ("acta", "project", &["create"]),
            (F::Projects, Update) => ("acta", "project", &["update"]),
            (F::Projects, Delete) => ("acta", "project", &["delete"]),
            (F::Webhooks, Read) => ("acta", "webhook", &["read", "read_deliveries"]),
            (F::Webhooks, Create) => ("acta", "webhook", &["create"]),
            (F::Webhooks, Update) => ("acta", "webhook", &["update"]),
            (F::Webhooks, Delete) => ("acta", "webhook", &["delete"]),
            // Config surfaces (tags, property definitions, status templates)
            // declare no actions of their own: reading them is reading the
            // workspace, changing them is managing its configuration.
            (F::Config, Read) => ("acta", "workspace", &["read"]),
            (F::Config, Create | Update | Delete) => ("acta", "workspace", &["manage_config"]),
            (F::Grants, Read) => ("custos", "grant", &["read"]),
            (F::Grants, Create | Update | Delete) => ("custos", "grant", &[]),
            (F::SavedSearches, Read) => ("acta", "saved_search", &["read"]),
            (F::SavedSearches, Create) => ("acta", "saved_search", &["create"]),
            (F::SavedSearches, Update) => ("acta", "saved_search", &["update"]),
            (F::SavedSearches, Delete) => ("acta", "saved_search", &["delete"]),
            (F::TaskViews, Read) => ("acta", "task_view", &["read"]),
            (F::TaskViews, Create) => ("acta", "task_view", &["create"]),
            (F::TaskViews, Update) => ("acta", "task_view", &["update"]),
            (F::TaskViews, Delete) => ("acta", "task_view", &["delete"]),
        };

    actions
        .iter()
        .filter_map(|action| ActionId::new(product, kind, action).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::authz::v2_service::product_specs;
    use crate::reg5::{StorageBackend, reg5_component_entries};

    fn action(raw: &str) -> ActionId {
        raw.parse().expect("valid action id")
    }

    fn catalog_actions() -> HashSet<ActionId> {
        let registry =
            atlas_core::registry::build(reg5_component_entries(StorageBackend::Filesystem))
                .expect("REG-5 entries build");

        product_specs(&registry)
            .into_iter()
            .flat_map(|spec| spec.actions)
            .collect()
    }

    #[test]
    fn a_session_is_unrestricted() {
        assert_eq!(session_ceiling(), Ceiling::Unrestricted);
    }

    #[test]
    fn grants_read_translates_to_the_v2_grant_read_action() {
        let ceiling = api_key_ceiling(&[Capability {
            family: CapabilityFamily::Grants,
            action: CapabilityAction::Read,
        }]);

        assert!(ceiling.permits(&action("custos::grant::read")));
        assert!(!ceiling.permits(&action("custos::grant::delete")));
        assert!(!ceiling.permits(&action("custos::grant::create")));
    }

    #[test]
    fn a_key_with_every_scope_still_lacks_grant_create() {
        let ceiling = api_key_ceiling(&Capability::ALL);

        assert!(!ceiling.permits(&action("custos::grant::create")));
        assert!(!ceiling.permits(&action("custos::grant::delete")));
        assert!(ceiling.permits(&action("custos::grant::read")));
        assert!(
            !ceiling.permits(&action("custos::group::read")),
            "no V1 capability stands for a Custos group action"
        );
        assert!(ceiling.permits(&action("acta::document::read")));
        assert!(
            !ceiling.permits(&action("acta::workspace::manage_members")),
            "no key scope stands for member management"
        );
    }

    #[test]
    fn a_key_without_scopes_permits_nothing() {
        let ceiling = api_key_ceiling(&[]);

        assert!(!ceiling.permits(&action("custos::grant::read")));
        assert!(!ceiling.permits(&action("acta::document::read")));
        assert!(ceiling != Ceiling::Unrestricted);
    }

    /// ACTA-AUTHZ-7: every one of the 37 capabilities translates, every
    /// translated action is declared in a published catalog, and none is
    /// the delegation action.
    #[test]
    fn every_capability_maps_onto_declared_catalog_actions() {
        let declared = catalog_actions();
        let grant_create = action("custos::grant::create");

        for capability in Capability::ALL {
            let mapped = v2_actions_for(&capability);
            assert!(
                !mapped.is_empty(),
                "{} translates to at least one V2 action",
                capability.as_str()
            );
            for mapped_action in &mapped {
                assert!(
                    declared.contains(mapped_action),
                    "{} maps to `{mapped_action}`, which no published catalog declares",
                    capability.as_str()
                );
                assert_ne!(*mapped_action, grant_create);
            }
        }
    }

    #[test]
    fn the_plural_families_stand_for_their_singular_kinds() {
        let docs_update = v2_actions_for(&Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Update,
        });

        assert!(docs_update.contains(&action("acta::document::update_content")));
        assert!(!docs_update.contains(&action("acta::document::delete")));

        let config_delete = v2_actions_for(&Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Delete,
        });
        assert_eq!(
            config_delete,
            vec![action("acta::workspace::manage_config")]
        );
    }
}
