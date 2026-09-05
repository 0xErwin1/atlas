//! One component table, total over all 28 [`crate::cli::Commands`] variants
//! (design D3).
//!
//! Declared here; audited against each command module's own
//! `.acta()`/`.custos()`/`.platform()` call sites in
//! `crates/atlas_server/tests/cli_mcp_component_derivation.rs` (design D6).
//! The declaration and the derivation are independent facts on purpose — a
//! hand-written row (e.g. `platform-status-templates` "corrected" to
//! `Platform` on its name) is a guess with a struct around it unless
//! something checks it against the command's own implementation.
//!
//! `COMMAND_COMPONENTS` and `NO_CALL_SITE` have no production consumer yet
//! in this PR — `help_group.rs` (PR2) and `alias.rs` (PR3) are the first
//! callers. `#[allow(dead_code)]` below is temporary and is expected to be
//! removed once either lands.

#![allow(dead_code)]

/// The three sub-client namespaces `AtlasClient` exposes
/// (`client.acta()` / `.custos()` / `.platform()`, E11-S4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Component {
    Acta,
    Custos,
    Platform,
}

/// One row per clap subcommand name (the derived kebab-case form of each
/// `Commands` variant). Total over `Commands`, asserted by
/// [`tests::declared_components_are_total_over_the_command_tree`].
///
/// **Do not "correct" a row to match its command's name.** Two rows here
/// read the other way from what their name suggests:
/// `platform-status-templates` calls exclusively through `client.acta()`
/// (`commands/platform_status_templates.rs`), and `audit` calls exclusively
/// through `client.custos()` (`commands/audit.rs`) even though its subject
/// is a workspace. Both are declared per their measured call site, and both
/// are audited against that call site in
/// `crates/atlas_server/tests/cli_mcp_component_derivation.rs`.
pub(crate) const COMMAND_COMPONENTS: &[(&str, Component)] = &[
    ("search", Component::Acta),
    ("tasks", Component::Acta),
    ("trash", Component::Acta),
    ("docs", Component::Acta),
    ("workspaces", Component::Acta),
    ("projects", Component::Acta),
    ("boards", Component::Acta),
    ("columns", Component::Acta),
    ("tags", Component::Acta),
    ("members", Component::Acta),
    ("folders", Component::Acta),
    ("activity", Component::Acta),
    ("status-templates", Component::Acta),
    ("platform-status-templates", Component::Acta),
    ("saved-searches", Component::Acta),
    ("task-views", Component::Acta),
    ("property-definitions", Component::Acta),
    ("import", Component::Acta),
    ("export", Component::Acta),
    ("users", Component::Custos),
    ("api-keys", Component::Custos),
    ("groups", Component::Custos),
    ("grants", Component::Custos),
    ("audit", Component::Custos),
    ("doctor", Component::Platform),
    ("version", Component::Platform),
    ("config", Component::Platform),
    ("completions", Component::Platform),
];

/// The commands that issue no HTTP request and call no sub-client at all,
/// each with the reason its component is nonetheless assigned (design
/// D3.2). Closed and bidirectionally checked by the derivation audit: an
/// unlisted zero-call-site command fails as an unnamed gap, and a listed
/// command that starts calling a sub-client fails as a stale entry.
pub(crate) const NO_CALL_SITE: &[(&str, &str)] = &[
    (
        "version",
        "prints CARGO_PKG_VERSION locally; issues no request. Platform owns build identity (SHELL-OPS-7)",
    ),
    (
        "config",
        "reads and writes the local config file only; issues no HTTP call. Assigned platform by SHELL-CLI-3",
    ),
    (
        "completions",
        "clap_complete renders shell completions locally; issues no HTTP call. Platform owns shell-level binary tooling",
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clap::CommandFactory;

    use super::*;
    use crate::cli::Cli;

    fn clap_subcommand_names() -> BTreeSet<String> {
        Cli::command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_string())
            .collect()
    }

    #[test]
    fn declared_components_table_has_28_rows() {
        assert_eq!(
            COMMAND_COMPONENTS.len(),
            28,
            "COMMAND_COMPONENTS must be total over all 28 Commands variants"
        );
    }

    #[test]
    fn declared_components_are_total_over_the_command_tree() {
        let declared: BTreeSet<String> = COMMAND_COMPONENTS
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();
        let parsed = clap_subcommand_names();

        let missing_row: Vec<&String> = parsed.difference(&declared).collect();
        assert!(
            missing_row.is_empty(),
            "clap subcommand(s) with no COMMAND_COMPONENTS row: {missing_row:?}"
        );

        let stale_row: Vec<&String> = declared.difference(&parsed).collect();
        assert!(
            stale_row.is_empty(),
            "COMMAND_COMPONENTS row(s) naming no clap subcommand: {stale_row:?}"
        );
    }

    #[test]
    fn declared_components_have_no_duplicate_names() {
        let mut names: Vec<&str> = COMMAND_COMPONENTS.iter().map(|(name, _)| *name).collect();
        let unique_count = {
            names.sort_unstable();
            names.dedup();
            names.len()
        };
        assert_eq!(
            unique_count,
            COMMAND_COMPONENTS.len(),
            "duplicate command name in COMMAND_COMPONENTS"
        );
    }

    #[test]
    fn no_call_site_is_closed_to_exactly_three_entries() {
        assert_eq!(NO_CALL_SITE.len(), 3);
        let names: BTreeSet<&str> = NO_CALL_SITE.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            BTreeSet::from(["version", "config", "completions"]),
            "NO_CALL_SITE must name exactly version, config, completions (design D3.2)"
        );
    }

    #[test]
    fn no_call_site_reasons_do_not_merely_restate_the_command_name() {
        for (name, reason) in NO_CALL_SITE {
            assert!(
                !reason.eq_ignore_ascii_case(name),
                "{name}'s NO_CALL_SITE reason must justify the assignment, not restate the name"
            );
            assert!(
                reason.len() > name.len(),
                "{name}'s NO_CALL_SITE reason reads as a restatement, not a justification"
            );
        }
    }

    #[test]
    fn platform_status_templates_and_audit_are_declared_per_call_site_not_name() {
        let component = |name: &str| {
            COMMAND_COMPONENTS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, c)| *c)
                .unwrap_or_else(|| panic!("{name} missing from COMMAND_COMPONENTS"))
        };

        assert_eq!(
            component("platform-status-templates"),
            Component::Acta,
            "platform-status-templates calls exclusively through client.acta()"
        );
        assert_eq!(
            component("audit"),
            Component::Custos,
            "audit calls exclusively through client.custos()"
        );
    }
}
