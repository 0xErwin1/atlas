//! Wire-form contract for the capability codec: the canonical wire/storage
//! spelling is `<product>::<kind>::<action>` (e.g. `acta::tasks::read`,
//! `custos::grants::read`), while the legacy `<family>:<action>` spelling
//! (e.g. `tasks:read`) must keep parsing so existing stored rows and old
//! clients stay valid without a data migration. Writing is always canonical.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::str::FromStr;

use atlas_custos::capability::{Capability, CapabilityAction, CapabilityFamily};

/// `(canonical, legacy)` for every catalog entry, in `Capability::ALL` order.
/// `grants` is read-only and deliberately plural in the canonical product
/// segment (`custos::grants::read`), matching the shipped registry.
const WIRE_FORMS: [(&str, &str); 37] = [
    ("acta::tasks::read", "tasks:read"),
    ("acta::tasks::create", "tasks:create"),
    ("acta::tasks::update", "tasks:update"),
    ("acta::tasks::delete", "tasks:delete"),
    ("acta::docs::read", "docs:read"),
    ("acta::docs::create", "docs:create"),
    ("acta::docs::update", "docs:update"),
    ("acta::docs::delete", "docs:delete"),
    ("acta::boards::read", "boards:read"),
    ("acta::boards::create", "boards:create"),
    ("acta::boards::update", "boards:update"),
    ("acta::boards::delete", "boards:delete"),
    ("acta::folders::read", "folders:read"),
    ("acta::folders::create", "folders:create"),
    ("acta::folders::update", "folders:update"),
    ("acta::folders::delete", "folders:delete"),
    ("acta::projects::read", "projects:read"),
    ("acta::projects::create", "projects:create"),
    ("acta::projects::update", "projects:update"),
    ("acta::projects::delete", "projects:delete"),
    ("acta::webhooks::read", "webhooks:read"),
    ("acta::webhooks::create", "webhooks:create"),
    ("acta::webhooks::update", "webhooks:update"),
    ("acta::webhooks::delete", "webhooks:delete"),
    ("acta::config::read", "config:read"),
    ("acta::config::create", "config:create"),
    ("acta::config::update", "config:update"),
    ("acta::config::delete", "config:delete"),
    ("custos::grants::read", "grants:read"),
    ("acta::saved_searches::read", "saved_searches:read"),
    ("acta::saved_searches::create", "saved_searches:create"),
    ("acta::saved_searches::update", "saved_searches:update"),
    ("acta::saved_searches::delete", "saved_searches:delete"),
    ("acta::task_views::read", "task_views:read"),
    ("acta::task_views::create", "task_views:create"),
    ("acta::task_views::update", "task_views:update"),
    ("acta::task_views::delete", "task_views:delete"),
];

#[test]
fn as_str_writes_the_canonical_form_for_every_catalog_entry() {
    assert_eq!(Capability::ALL.len(), WIRE_FORMS.len());
    for (cap, (canonical, _legacy)) in Capability::ALL.iter().zip(WIRE_FORMS) {
        assert_eq!(cap.as_str(), canonical, "canonical wire form for {cap:?}");
    }
}

#[test]
fn as_str_is_unique_across_the_catalog() {
    let forms: HashSet<&str> = Capability::ALL.iter().map(Capability::as_str).collect();
    assert_eq!(forms.len(), Capability::ALL.len());
}

#[test]
fn from_str_accepts_the_canonical_spelling() {
    for (canonical, _legacy) in WIRE_FORMS {
        let parsed = Capability::from_str(canonical)
            .unwrap_or_else(|err| panic!("{canonical} must parse: {err}"));
        assert_eq!(parsed.as_str(), canonical);
    }
}

#[test]
fn from_str_accepts_the_legacy_spelling_as_the_same_capability() {
    for (canonical, legacy) in WIRE_FORMS {
        let from_canonical = Capability::from_str(canonical)
            .unwrap_or_else(|err| panic!("{canonical} must parse: {err}"));
        let from_legacy =
            Capability::from_str(legacy).unwrap_or_else(|err| panic!("{legacy} must parse: {err}"));
        assert_eq!(from_canonical, from_legacy, "{legacy} aliases {canonical}");
    }
}

#[test]
fn from_str_rejects_unknown_and_malformed_strings() {
    let rejected = [
        "tasks:manage",
        "foo:read",
        "tasks",
        "",
        "acta::tasks",
        "acta::tasks::read::extra",
        "ACTA::tasks::read",
        // Grant writes are not in the catalog in either spelling.
        "grants:create",
        "grants:update",
        "grants:delete",
        "custos::grants::create",
        "custos::grants::update",
        "custos::grants::delete",
    ];
    for raw in rejected {
        assert!(
            Capability::from_str(raw).is_err(),
            "{raw} must not parse as a capability"
        );
    }
}

#[test]
fn default_read_only_keeps_its_exact_meaning() {
    let expected: HashSet<(CapabilityFamily, CapabilityAction)> = [
        (CapabilityFamily::Tasks, CapabilityAction::Read),
        (CapabilityFamily::Docs, CapabilityAction::Read),
        (CapabilityFamily::Boards, CapabilityAction::Read),
        (CapabilityFamily::Folders, CapabilityAction::Read),
        (CapabilityFamily::Projects, CapabilityAction::Read),
    ]
    .into_iter()
    .collect();

    let got: HashSet<(CapabilityFamily, CapabilityAction)> = Capability::DEFAULT_READ_ONLY
        .iter()
        .map(|cap| (cap.family, cap.action))
        .collect();

    assert_eq!(got, expected);
    assert_eq!(Capability::DEFAULT_READ_ONLY.len(), 5);
}
