//! The V2 credential ceiling (`v2-e5-s5b-grant-authority`, D-S5b-3): what
//! a credential may exercise at most, before any grant is considered.
//!
//! A human session is unrestricted: the session carries the user's whole
//! authority. An API key is restricted to its V1 `scopes`, translated into
//! the V2 vocabulary through the explicit table below. Only the Custos
//! vocabulary is translated in this release: V1 `grants:read` becomes
//! `custos::grant::read`, and every other capability maps to nothing in
//! the Custos product (the Acta families map to Acta's V2 actions in E7,
//! once Acta publishes its catalog). No key scope maps to
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
    Ceiling::Restricted(scopes.iter().filter_map(custos_action_for).collect())
}

/// The V2 Custos action a V1 capability stands for, if any. The table is
/// deliberately explicit: a family or action absent here translates to
/// nothing, never to a guessed V2 action.
fn custos_action_for(capability: &Capability) -> Option<ActionId> {
    match (capability.family, capability.action) {
        (CapabilityFamily::Grants, CapabilityAction::Read) => {
            ActionId::new("custos", "grant", "read").ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(raw: &str) -> ActionId {
        raw.parse().expect("valid action id")
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
        assert!(ceiling.permits(&action("custos::grant::read")));
        assert!(
            !ceiling.permits(&action("custos::group::read")),
            "no V1 capability stands for a Custos group action"
        );
        assert!(
            !ceiling.permits(&action("acta::doc::read")),
            "Acta families translate only once Acta publishes its V2 catalog"
        );
    }

    #[test]
    fn a_key_without_scopes_permits_nothing() {
        let ceiling = api_key_ceiling(&[]);

        assert!(!ceiling.permits(&action("custos::grant::read")));
        assert!(ceiling != Ceiling::Unrestricted);
    }

    #[test]
    fn the_translation_table_names_only_grants_read() {
        let translated: Vec<(&Capability, ActionId)> = Capability::ALL
            .iter()
            .filter_map(|capability| custos_action_for(capability).map(|a| (capability, a)))
            .collect();

        assert_eq!(translated.len(), 1);
        assert_eq!(translated[0].0.as_str(), "custos::grants::read");
        assert_eq!(translated[0].1, action("custos::grant::read"));
    }
}
