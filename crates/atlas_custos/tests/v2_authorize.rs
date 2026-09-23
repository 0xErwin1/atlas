//! Behavioral tests for the authorization service (E5 S6a): fact loading
//! with one provider query and one store load per request (EVAL-5),
//! fail-closed mapping of provider, store and membership failures
//! (AVAIL-1, PROV-4), and the evaluator contract end to end.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_core::capabilities::{
    CapabilityError, ProviderCatalog, ResourceExistence, ResourceFacts, ResourceProvider,
};
use atlas_core::error::DomainError;
use atlas_core::ids::{PrincipalSetId, ResourcePath, ResourceRef};
use atlas_custos::authorize::{
    ActorContext, AuthorizationService, AuthorizationSettings, ProviderSet, Sleeper,
};
use atlas_custos::entities::authorization::{
    CustomRole, DenyRecord, DenyRuleId, GrantAuthority, GrantId, GrantRecord, RoleId,
    SubjectRecord, TargetRecord,
};
use atlas_custos::eval::{
    Ceiling, Decision, DenyMode, EvalError, FactFailure, VisibilityPredicate,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use atlas_custos::ports::authorize::{
    AuthorizationFactsStore, DeclaredSet, GroupMembershipSource, ProductScope,
    StoredAuthorizationFacts,
};
use chrono::Utc;
use support::{READ, UPDATE, action, catalog, ceiling};

const DOC: &str = "acta::document::d1";
const DOC_PATH: &str = "acta::workspace::w1/folder::f1/document::d1";
const GRANT_CREATE: &str = "custos::grant::create";

/// How the fake provider answers `resource_facts`.
#[derive(Clone, Copy)]
enum Answer {
    Facts,
    Fail,
    Hang,
}

/// A counting provider answering from a fixed table of resources.
struct FakeProvider {
    answer: Answer,
    resources: HashMap<ResourceRef, Option<ResourcePath>>,
    members: HashMap<PrincipalSetId, Result<Vec<atlas_core::ids::PrincipalId>, CapabilityError>>,
    hanging_sets: Vec<PrincipalSetId>,
    facts_calls: AtomicUsize,
    members_calls: AtomicUsize,
}

impl FakeProvider {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            resources: HashMap::new(),
            members: HashMap::new(),
            hanging_sets: Vec::new(),
            facts_calls: AtomicUsize::new(0),
            members_calls: AtomicUsize::new(0),
        }
    }

    fn with_resource(mut self, path: &str) -> Self {
        let path: ResourcePath = path.parse().unwrap();
        self.resources.insert(path.leaf_ref(), Some(path));
        self
    }

    fn with_members(
        mut self,
        set: &PrincipalSetId,
        members: Result<Vec<atlas_core::ids::PrincipalId>, CapabilityError>,
    ) -> Self {
        self.members.insert(set.clone(), members);
        self
    }

    fn facts_calls(&self) -> usize {
        self.facts_calls.load(Ordering::SeqCst)
    }

    fn members_calls(&self) -> usize {
        self.members_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ResourceProvider for FakeProvider {
    async fn validate_ref(&self, _resource: &ResourceRef) -> Result<bool, CapabilityError> {
        panic!("the service must use resource_facts")
    }

    async fn path_of(&self, _resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        panic!("the service must use resource_facts")
    }

    async fn ancestors(
        &self,
        _resource: &ResourceRef,
    ) -> Result<Vec<ResourceRef>, CapabilityError> {
        panic!("the service must use resource_facts")
    }

    async fn members_of(
        &self,
        set: &PrincipalSetId,
    ) -> Result<Vec<atlas_core::ids::PrincipalId>, CapabilityError> {
        self.members_calls.fetch_add(1, Ordering::SeqCst);

        if self.hanging_sets.contains(set) {
            std::future::pending::<()>().await;
        }

        self.members
            .get(set)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
        panic!("the service must not read the provider catalog")
    }

    async fn resource_facts(
        &self,
        resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        self.facts_calls.fetch_add(1, Ordering::SeqCst);

        match self.answer {
            Answer::Fail => return Err(CapabilityError::unavailable("backend down")),
            Answer::Hang => std::future::pending::<()>().await,
            Answer::Facts => {}
        }

        Ok(resources
            .iter()
            .map(|resource| match self.resources.get(resource) {
                Some(path) => ResourceFacts {
                    resource: resource.clone(),
                    existence: ResourceExistence::Exists,
                    path: path.clone(),
                },
                None => ResourceFacts {
                    resource: resource.clone(),
                    existence: ResourceExistence::Missing,
                    path: None,
                },
            })
            .collect())
    }
}

/// A counting store returning fixed facts, recording each load's scope.
#[derive(Default)]
struct FakeStore {
    facts: StoredAuthorizationFacts,
    fail: bool,
    scopes: Mutex<Vec<ProductScope>>,
    declared: Mutex<Vec<Vec<DeclaredSet>>>,
}

impl FakeStore {
    fn loads(&self) -> Vec<ProductScope> {
        self.scopes.lock().unwrap().clone()
    }
}

/// The service owns its store; tests keep a shared handle to count loads.
struct SharedStore(Arc<FakeStore>);

#[async_trait]
impl AuthorizationFactsStore for SharedStore {
    async fn load(
        &self,
        scope: &ProductScope,
        _principal: PrincipalId,
        _groups: &[GroupId],
        declared_sets: &[DeclaredSet],
    ) -> Result<StoredAuthorizationFacts, DomainError> {
        self.0.scopes.lock().unwrap().push(scope.clone());
        self.0.declared.lock().unwrap().push(declared_sets.to_vec());

        if self.0.fail {
            return Err(DomainError::Internal {
                message: "store down".to_string(),
            });
        }

        Ok(self.0.facts.clone())
    }
}

/// A counting membership source answering a fixed group list.
#[derive(Default)]
struct FakeMembership {
    groups: Vec<GroupId>,
    fail: bool,
    calls: AtomicUsize,
}

/// The service owns its membership source; tests keep a shared handle.
struct SharedMembership(Arc<FakeMembership>);

#[async_trait]
impl GroupMembershipSource for SharedMembership {
    async fn groups_of(&self, _principal: PrincipalId) -> Result<Vec<GroupId>, DomainError> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);

        if self.0.fail {
            return Err(DomainError::Internal {
                message: "groups down".to_string(),
            });
        }

        Ok(self.0.groups.clone())
    }
}

/// A sleeper whose timer never fires.
struct NeverSleeper;

#[async_trait]
impl Sleeper for NeverSleeper {
    async fn sleep(&self, _duration: Duration) {
        std::future::pending::<()>().await
    }
}

/// A sleeper whose timer fires at once, so any provider call that does not
/// answer on its first poll times out.
struct InstantSleeper;

#[async_trait]
impl Sleeper for InstantSleeper {
    async fn sleep(&self, _duration: Duration) {}
}

/// One service under test and the handles its fakes count through.
struct Harness {
    provider: Arc<FakeProvider>,
    store: Arc<FakeStore>,
    membership: Arc<FakeMembership>,
    service: AuthorizationService<SharedStore, SharedMembership>,
}

impl Harness {
    fn build(
        provider: FakeProvider,
        store: FakeStore,
        membership: FakeMembership,
        sleeper: Arc<dyn Sleeper>,
        deny_mode: DenyMode,
    ) -> Self {
        let provider = Arc::new(provider);
        let store = Arc::new(store);
        let membership = Arc::new(membership);
        let service = AuthorizationService::new(
            ProviderSet::new().with("acta", provider.clone()),
            SharedStore(store.clone()),
            SharedMembership(membership.clone()),
            sleeper,
            AuthorizationSettings::new(catalog(), deny_mode),
        );

        Self {
            provider,
            store,
            membership,
            service,
        }
    }

    fn new(provider: FakeProvider, store: FakeStore) -> Self {
        Self::build(
            provider,
            store,
            FakeMembership::default(),
            Arc::new(NeverSleeper),
            DenyMode::Enforced,
        )
    }

    fn membership_calls(&self) -> usize {
        self.membership.calls.load(Ordering::SeqCst)
    }
}

fn actor(principal: PrincipalId) -> ActorContext {
    ActorContext {
        principal,
        is_root: false,
        ceiling: Ceiling::Unrestricted,
    }
}

fn target(raw: &str) -> ResourceRef {
    raw.parse().unwrap()
}

fn grant_record(subject: SubjectRecord, target: &str, authority: GrantAuthority) -> GrantRecord {
    GrantRecord {
        id: GrantId::new(),
        subject,
        target: TargetRecord::Ref(target.parse().unwrap()),
        authority,
        created_by: PrincipalId::new(),
        created_at: Utc::now(),
    }
}

fn explicit(granted: &[&str]) -> GrantAuthority {
    GrantAuthority::Actions(granted.iter().map(|raw| action(raw)).collect())
}

fn deny_record(subject: SubjectRecord, target: &str, denied: &[&str]) -> DenyRecord {
    DenyRecord {
        id: DenyRuleId::new(),
        subject,
        target: TargetRecord::Ref(target.parse().unwrap()),
        actions: denied.iter().map(|raw| action(raw)).collect(),
        created_by: PrincipalId::new(),
        created_at: Utc::now(),
    }
}

fn store_with(grants: Vec<GrantRecord>, denies: Vec<DenyRecord>) -> FakeStore {
    FakeStore {
        facts: StoredAuthorizationFacts {
            grants,
            denies,
            custom_roles: Vec::new(),
        },
        ..FakeStore::default()
    }
}

fn reader_store(principal: PrincipalId, granted: &[&str]) -> FakeStore {
    store_with(
        vec![grant_record(
            SubjectRecord::Principal(principal),
            "acta::workspace::w1",
            explicit(granted),
        )],
        Vec::new(),
    )
}

fn unavailable(cause: FactFailure) -> Result<atlas_custos::eval::Evaluated, EvalError> {
    Err(EvalError::FactsUnavailable { cause })
}

#[tokio::test]
async fn authorize_uses_one_provider_call_and_one_store_load() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        reader_store(principal, &[READ]),
    );

    let outcome = harness
        .service
        .authorize(&actor(principal), &action(READ), &target(DOC))
        .await
        .unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
    assert_eq!(harness.provider.facts_calls(), 1);
    assert_eq!(
        harness.store.loads(),
        vec![ProductScope::Products(vec!["acta".to_string()])]
    );
    assert_eq!(harness.membership_calls(), 1);
    assert_eq!(harness.provider.members_calls(), 0);
}

#[tokio::test]
async fn a_missing_target_is_not_found_without_loading_grants() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts),
        reader_store(principal, &[READ]),
    );

    let outcome = harness
        .service
        .authorize(&actor(principal), &action(READ), &target(DOC))
        .await
        .unwrap();

    assert_eq!(outcome.decision, Decision::NotFound);
    assert_eq!(harness.provider.facts_calls(), 1);
    assert!(harness.store.loads().is_empty());
}

#[tokio::test]
async fn a_provider_failure_or_timeout_is_a_typed_technical_error_for_one_target() {
    let principal = PrincipalId::new();
    let failing = Harness::new(
        FakeProvider::new(Answer::Fail),
        reader_store(principal, &[READ]),
    );
    let hanging = Harness::build(
        FakeProvider::new(Answer::Hang),
        reader_store(principal, &[READ]),
        FakeMembership::default(),
        Arc::new(InstantSleeper),
        DenyMode::Enforced,
    );

    for (harness, cause) in [
        (&failing, FactFailure::Provider),
        (&hanging, FactFailure::Timeout),
    ] {
        let outcome = harness
            .service
            .authorize(&actor(principal), &action(READ), &target(DOC))
            .await;

        assert_eq!(outcome, unavailable(cause));
        assert_eq!(harness.provider.facts_calls(), 1);
    }
}

#[tokio::test]
async fn a_store_or_membership_failure_fails_closed_instead_of_narrowing() {
    let principal = PrincipalId::new();
    let broken_store = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        FakeStore {
            fail: true,
            ..reader_store(principal, &[READ])
        },
    );
    let broken_groups = Harness::build(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        reader_store(principal, &[READ]),
        FakeMembership {
            fail: true,
            ..FakeMembership::default()
        },
        Arc::new(NeverSleeper),
        DenyMode::Enforced,
    );

    for harness in [&broken_store, &broken_groups] {
        let outcome = harness
            .service
            .authorize(&actor(principal), &action(READ), &target(DOC))
            .await;

        assert_eq!(outcome, unavailable(FactFailure::Internal));
    }
}

#[tokio::test]
async fn root_needs_existence_only() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        FakeStore {
            fail: true,
            ..FakeStore::default()
        },
    );
    let root = ActorContext {
        is_root: true,
        ceiling: Ceiling::Restricted(Default::default()),
        ..actor(principal)
    };

    let present = harness
        .service
        .authorize(&root, &action(UPDATE), &target(DOC))
        .await
        .unwrap();
    let absent = harness
        .service
        .authorize(&root, &action(UPDATE), &target("acta::document::gone"))
        .await
        .unwrap();

    assert_eq!(present.decision, Decision::Allowed);
    assert_eq!(absent.decision, Decision::NotFound);
    assert!(harness.store.loads().is_empty());
    assert_eq!(harness.membership_calls(), 0);
}

#[tokio::test]
async fn the_caller_ceiling_narrows_stored_authority() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        reader_store(principal, &[READ, UPDATE]),
    );
    let narrowed = ActorContext {
        ceiling: ceiling(&[READ]),
        ..actor(principal)
    };

    let update = harness
        .service
        .authorize(&narrowed, &action(UPDATE), &target(DOC))
        .await
        .unwrap();

    assert_eq!(
        update.decision,
        Decision::Denied {
            because: atlas_custos::eval::DenyCause::NotGranted
        }
    );
}

#[tokio::test]
async fn stored_denies_apply_per_configured_mode() {
    let principal = PrincipalId::new();
    let expectations = [
        (DenyMode::Enforced, Decision::NotFound, 0),
        (DenyMode::Audit, Decision::Allowed, 1),
        (DenyMode::Disabled, Decision::Allowed, 0),
    ];

    for (mode, decision, evidence) in expectations {
        let harness = Harness::build(
            FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
            store_with(
                vec![grant_record(
                    SubjectRecord::Principal(principal),
                    "acta::workspace::w1",
                    explicit(&[READ]),
                )],
                vec![deny_record(
                    SubjectRecord::Principal(principal),
                    "acta::folder::f1",
                    &[READ],
                )],
            ),
            FakeMembership::default(),
            Arc::new(NeverSleeper),
            mode,
        );

        let outcome = harness
            .service
            .authorize(&actor(principal), &action(READ), &target(DOC))
            .await
            .unwrap();

        assert_eq!(outcome.decision, decision, "{mode:?}");
        assert_eq!(outcome.would_block.len(), evidence, "{mode:?}");
    }
}

#[tokio::test]
async fn group_grants_apply_through_the_membership_source() {
    let principal = PrincipalId::new();
    let editors = GroupId::new();
    let store = || {
        store_with(
            vec![grant_record(
                SubjectRecord::Group(editors),
                "acta::workspace::w1",
                explicit(&[READ]),
            )],
            Vec::new(),
        )
    };
    let member = Harness::build(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        store(),
        FakeMembership {
            groups: vec![editors],
            ..FakeMembership::default()
        },
        Arc::new(NeverSleeper),
        DenyMode::Enforced,
    );
    let outsider = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        store(),
    );

    let allowed = member
        .service
        .authorize(&actor(principal), &action(READ), &target(DOC))
        .await
        .unwrap();
    let hidden = outsider
        .service
        .authorize(&actor(principal), &action(READ), &target(DOC))
        .await
        .unwrap();

    assert_eq!(allowed.decision, Decision::Allowed);
    assert_eq!(hidden.decision, Decision::NotFound);
}

#[tokio::test]
async fn principal_sets_resolve_through_members_of_only_when_referenced() {
    let principal = PrincipalId::new();
    let reviewers: PrincipalSetId = "acta::workspace::w1::members".parse().unwrap();
    let as_member = atlas_core::ids::PrincipalId::new(&principal.0.to_string()).unwrap();
    let other = atlas_core::ids::PrincipalId::new(&PrincipalId::new().0.to_string()).unwrap();
    let store = || {
        store_with(
            vec![grant_record(
                SubjectRecord::PrincipalSet(reviewers.clone()),
                "acta::workspace::w1",
                explicit(&[READ]),
            )],
            Vec::new(),
        )
    };
    let cases = [
        (Ok(vec![other.clone(), as_member]), Decision::Allowed),
        (Ok(vec![other]), Decision::NotFound),
        (
            Err(CapabilityError::unavailable("set backend down")),
            Decision::NotFound,
        ),
    ];

    for (members, decision) in cases {
        let harness = Harness::new(
            FakeProvider::new(Answer::Facts)
                .with_resource(DOC_PATH)
                .with_members(&reviewers, members),
            store(),
        );

        let outcome = harness
            .service
            .authorize(&actor(principal), &action(READ), &target(DOC))
            .await
            .unwrap();

        assert_eq!(outcome.decision, decision);
        assert_eq!(harness.provider.members_calls(), 1);
    }

    let unreferenced = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        reader_store(principal, &[READ]),
    );
    unreferenced
        .service
        .authorize(&actor(principal), &action(READ), &target(DOC))
        .await
        .unwrap();
    assert_eq!(unreferenced.provider.members_calls(), 0);
}

#[tokio::test]
async fn stored_grants_resolve_builtin_and_custom_roles_through_the_catalog() {
    let principal = PrincipalId::new();
    let role = CustomRole {
        id: RoleId::new(),
        product: "acta".to_string(),
        name: "updater".to_string(),
        actions: vec![action(UPDATE)],
        created_by: principal,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let store = FakeStore {
        facts: StoredAuthorizationFacts {
            grants: vec![
                grant_record(
                    SubjectRecord::Principal(principal),
                    "acta::workspace::w1",
                    GrantAuthority::Builtin {
                        name: "viewer".to_string(),
                        version: 1,
                    },
                ),
                grant_record(
                    SubjectRecord::Principal(principal),
                    "acta::workspace::w1",
                    GrantAuthority::CustomRole(role.id),
                ),
            ],
            denies: Vec::new(),
            custom_roles: vec![role.clone()],
        },
        ..FakeStore::default()
    };
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        store,
    );

    for granted in [READ, UPDATE] {
        let outcome = harness
            .service
            .authorize(&actor(principal), &action(granted), &target(DOC))
            .await
            .unwrap();

        assert_eq!(outcome.decision, Decision::Allowed, "{granted}");
    }

    let orphan = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        store_with(
            vec![grant_record(
                SubjectRecord::Principal(principal),
                "acta::workspace::w1",
                GrantAuthority::CustomRole(RoleId::new()),
            )],
            Vec::new(),
        ),
    );
    assert!(matches!(
        orphan
            .service
            .authorize(&actor(principal), &action(READ), &target(DOC))
            .await,
        Err(EvalError::InconsistentFacts { .. })
    ));
}

#[tokio::test]
async fn a_target_the_provider_did_not_answer_for_is_unavailable() {
    struct Silent;

    #[async_trait]
    impl ResourceProvider for Silent {
        async fn validate_ref(&self, _r: &ResourceRef) -> Result<bool, CapabilityError> {
            panic!("unused")
        }
        async fn path_of(&self, _r: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
            panic!("unused")
        }
        async fn ancestors(&self, _r: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
            panic!("unused")
        }
        async fn members_of(
            &self,
            _s: &PrincipalSetId,
        ) -> Result<Vec<atlas_core::ids::PrincipalId>, CapabilityError> {
            panic!("unused")
        }
        async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
            panic!("unused")
        }
        async fn resource_facts(
            &self,
            _resources: &[ResourceRef],
        ) -> Result<Vec<ResourceFacts>, CapabilityError> {
            Ok(Vec::new())
        }
    }

    let principal = PrincipalId::new();
    let service = AuthorizationService::new(
        ProviderSet::new().with("acta", Arc::new(Silent)),
        SharedStore(Arc::new(reader_store(principal, &[READ]))),
        SharedMembership(Arc::new(FakeMembership::default())),
        Arc::new(NeverSleeper),
        AuthorizationSettings::new(catalog(), DenyMode::Enforced),
    );

    let outcome = service
        .authorize(&actor(principal), &action(READ), &target(DOC))
        .await;

    assert_eq!(outcome, unavailable(FactFailure::Provider));
}

#[tokio::test]
async fn a_batch_uses_one_provider_call_per_product_and_one_load() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts)
            .with_resource(DOC_PATH)
            .with_resource("acta::workspace::w1/folder::f1/document::d2"),
        reader_store(principal, &[READ]),
    );
    let targets = [
        target("acta::document::gone"),
        target(DOC),
        target("virtus::task::t1"),
        target("acta::document::d2"),
    ];

    let results = harness
        .service
        .authorize_batch(&actor(principal), &action(READ), &targets)
        .await
        .unwrap();

    let decisions: Vec<Result<Decision, EvalError>> = results
        .into_iter()
        .map(|result| result.map(|evaluated| evaluated.decision))
        .collect();
    assert_eq!(
        decisions,
        vec![
            Ok(Decision::NotFound),
            Ok(Decision::Allowed),
            Err(EvalError::CrossProductRequest {
                target_product: "virtus".to_string(),
                action_product: "acta".to_string(),
            }),
            Ok(Decision::Allowed),
        ]
    );
    assert_eq!(harness.provider.facts_calls(), 1);
    assert_eq!(harness.store.loads().len(), 1);
}

#[tokio::test]
async fn a_batch_turns_provider_failure_or_timeout_into_not_found_per_target() {
    let principal = PrincipalId::new();
    let failing = Harness::new(
        FakeProvider::new(Answer::Fail),
        reader_store(principal, &[READ]),
    );
    let hanging = Harness::build(
        FakeProvider::new(Answer::Hang),
        reader_store(principal, &[READ]),
        FakeMembership::default(),
        Arc::new(InstantSleeper),
        DenyMode::Enforced,
    );

    for harness in [&failing, &hanging] {
        let results = harness
            .service
            .authorize_batch(
                &actor(principal),
                &action(READ),
                &[target(DOC), target("acta::document::d2")],
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        for result in results {
            assert_eq!(result.unwrap().decision, Decision::NotFound);
        }
        assert_eq!(harness.provider.facts_calls(), 1);
    }
}

#[tokio::test]
async fn a_batch_store_failure_fails_the_whole_batch() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        FakeStore {
            fail: true,
            ..FakeStore::default()
        },
    );

    let outcome = harness
        .service
        .authorize_batch(&actor(principal), &action(READ), &[target(DOC)])
        .await;

    assert_eq!(
        outcome,
        Err(EvalError::FactsUnavailable {
            cause: FactFailure::Internal
        })
    );
}

#[tokio::test]
async fn the_visibility_filter_loads_once_without_resource_facts() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts),
        reader_store(principal, &[READ, GRANT_CREATE]),
    );

    let predicate = harness
        .service
        .visibility_filter(&actor(principal), "document", &action(READ))
        .await
        .unwrap();
    harness
        .service
        .visibility_filter(&actor(principal), "document", &action(GRANT_CREATE))
        .await
        .unwrap();

    assert!(predicate.permits(&DOC_PATH.parse().unwrap()));
    assert!(!predicate.permits(&"acta::workspace::w2/document::d9".parse().unwrap()));
    assert_eq!(harness.provider.facts_calls(), 0);
    assert_eq!(
        harness.store.loads(),
        vec![
            ProductScope::Products(vec!["acta".to_string()]),
            ProductScope::All
        ]
    );
}

#[tokio::test]
async fn the_visibility_filter_fails_closed_and_serves_root_without_loading() {
    let principal = PrincipalId::new();
    let broken = Harness::new(
        FakeProvider::new(Answer::Facts),
        FakeStore {
            fail: true,
            ..FakeStore::default()
        },
    );
    let root = ActorContext {
        is_root: true,
        ..actor(principal)
    };

    assert_eq!(
        broken
            .service
            .visibility_filter(&actor(principal), "document", &action(READ))
            .await,
        Err(EvalError::FactsUnavailable {
            cause: FactFailure::Internal
        })
    );
    assert_eq!(
        broken
            .service
            .visibility_filter(&root, "document", &action(READ))
            .await,
        Ok(VisibilityPredicate::All)
    );
}

#[tokio::test]
async fn effective_actions_use_one_provider_call_and_one_load() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        reader_store(principal, &[READ, GRANT_CREATE]),
    );

    let effective = harness
        .service
        .effective_actions(&actor(principal), &target(DOC))
        .await
        .unwrap();

    assert!(effective.contains(&action(READ)));
    assert!(effective.contains(&action(GRANT_CREATE)));
    assert!(!effective.contains(&action(UPDATE)));
    assert_eq!(harness.provider.facts_calls(), 1);
    assert_eq!(harness.store.loads().len(), 1);
}

#[tokio::test]
async fn effective_actions_fail_on_provider_failure_and_hold_everything_for_root() {
    let principal = PrincipalId::new();
    let failing = Harness::new(
        FakeProvider::new(Answer::Fail),
        reader_store(principal, &[READ]),
    );
    let present = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        FakeStore::default(),
    );
    let root = ActorContext {
        is_root: true,
        ..actor(principal)
    };

    assert_eq!(
        failing
            .service
            .effective_actions(&actor(principal), &target(DOC))
            .await,
        Err(EvalError::FactsUnavailable {
            cause: FactFailure::Provider
        })
    );
    assert!(
        present
            .service
            .effective_actions(&root, &target(DOC))
            .await
            .unwrap()
            .is_all()
    );
    assert!(present.store.loads().is_empty());
}

/// A Custos resource store answering from a fixed set of user rows.
struct UserRows(std::collections::HashSet<uuid::Uuid>);

#[async_trait]
impl atlas_custos::provider::CustosResourceStore for UserRows {
    async fn existing(
        &self,
        kind: atlas_custos::provider::CustosKind,
        ids: &[uuid::Uuid],
    ) -> Result<std::collections::HashSet<uuid::Uuid>, DomainError> {
        if kind != atlas_custos::provider::CustosKind::User {
            return Ok(Default::default());
        }

        Ok(ids
            .iter()
            .copied()
            .filter(|id| self.0.contains(id))
            .collect())
    }
}

#[tokio::test]
async fn an_enforced_deny_on_a_canonical_custos_id_cannot_be_bypassed_by_an_alias() {
    let principal = PrincipalId::new();
    let denied = uuid::Uuid::now_v7();
    let other = uuid::Uuid::now_v7();
    let provider = atlas_custos::provider::CustosResourceProvider::new(
        UserRows([denied, other].into_iter().collect()),
        &atlas_core::registry::Authorization {
            resource_kinds: vec!["user".to_string()],
            actions: vec![action("custos::user::read")],
            role_definitions: Vec::new(),
            role_definitions_v2: vec![],
            principal_sets: Vec::new(),
            provider: true,
        },
    );
    let catalog = atlas_custos::eval::Catalog::new([atlas_custos::eval::ProductSpec {
        product: "custos".to_string(),
        kinds: vec!["user".to_string()],
        actions: vec![action("custos::user::read")],
        roles: Vec::new(),
        principal_sets: Vec::new(),
    }])
    .unwrap();
    let store = FakeStore {
        facts: StoredAuthorizationFacts {
            grants: vec![GrantRecord {
                target: TargetRecord::Selector("custos::*".parse().unwrap()),
                ..grant_record(
                    SubjectRecord::Principal(principal),
                    "custos::platform::atlas",
                    explicit(&["custos::user::read"]),
                )
            }],
            denies: vec![deny_record(
                SubjectRecord::Principal(principal),
                &format!("custos::user::{denied}"),
                &["custos::user::read"],
            )],
            custom_roles: Vec::new(),
        },
        ..FakeStore::default()
    };
    let service = AuthorizationService::new(
        ProviderSet::new().with("custos", Arc::new(provider)),
        SharedStore(Arc::new(store)),
        SharedMembership(Arc::new(FakeMembership::default())),
        Arc::new(NeverSleeper),
        AuthorizationSettings::new(catalog, DenyMode::Enforced),
    );
    let read = action("custos::user::read");
    let decide = |id: String| {
        let target: ResourceRef = format!("custos::user::{id}").parse().unwrap();
        let service = &service;
        let read = &read;
        async move {
            service
                .authorize(&actor(principal), read, &target)
                .await
                .unwrap()
                .decision
        }
    };

    assert_eq!(decide(other.to_string()).await, Decision::Allowed);
    assert_eq!(decide(denied.to_string()).await, Decision::NotFound);
    for alias in [
        denied.hyphenated().to_string().to_uppercase(),
        denied.simple().to_string(),
        denied.braced().to_string(),
    ] {
        assert_eq!(decide(alias.clone()).await, Decision::NotFound, "{alias}");
    }
}

#[tokio::test]
async fn every_facts_load_receives_the_sets_the_catalog_declares() {
    let principal = PrincipalId::new();
    let harness = Harness::new(
        FakeProvider::new(Answer::Facts).with_resource(DOC_PATH),
        reader_store(principal, &[READ, GRANT_CREATE]),
    );
    let actor = actor(principal);

    harness
        .service
        .authorize(&actor, &action(READ), &target(DOC))
        .await
        .unwrap();
    harness
        .service
        .authorize_batch(&actor, &action(READ), &[target(DOC)])
        .await
        .unwrap();
    harness
        .service
        .visibility_filter(&actor, "document", &action(READ))
        .await
        .unwrap();
    harness
        .service
        .visibility_filter(&actor, "document", &action(GRANT_CREATE))
        .await
        .unwrap();
    harness
        .service
        .effective_actions(&actor, &target(DOC))
        .await
        .unwrap();

    let declared = vec![DeclaredSet {
        product: "acta".to_string(),
        name: "members".to_string(),
    }];
    assert_eq!(*harness.store.declared.lock().unwrap(), vec![declared; 5]);
}

/// Two principal sets granting read on the workspace, the first of which
/// cannot be resolved.
fn two_set_store() -> (FakeStore, PrincipalSetId, PrincipalSetId) {
    let first: PrincipalSetId = "acta::workspace::w1::members".parse().unwrap();
    let second: PrincipalSetId = "acta::workspace::w2::members".parse().unwrap();
    let store = store_with(
        [&first, &second]
            .into_iter()
            .map(|set| {
                grant_record(
                    SubjectRecord::PrincipalSet(set.clone()),
                    "acta::workspace::w1",
                    explicit(&[READ]),
                )
            })
            .collect(),
        Vec::new(),
    );
    (store, first, second)
}

#[tokio::test]
async fn a_failing_or_hanging_set_does_not_stop_the_next_set_from_resolving() {
    let principal = PrincipalId::new();
    let member = atlas_core::ids::PrincipalId::new(&principal.0.to_string()).unwrap();

    let (store, first, second) = two_set_store();
    let failing = Harness::new(
        FakeProvider::new(Answer::Facts)
            .with_resource(DOC_PATH)
            .with_members(
                &first,
                Err(CapabilityError::unavailable("set backend down")),
            )
            .with_members(&second, Ok(vec![member.clone()])),
        store,
    );

    let (store, first, second) = two_set_store();
    let mut provider = FakeProvider::new(Answer::Facts)
        .with_resource(DOC_PATH)
        .with_members(&second, Ok(vec![member]));
    provider.hanging_sets.push(first);
    let hanging = Harness::build(
        provider,
        store,
        FakeMembership::default(),
        Arc::new(InstantSleeper),
        DenyMode::Enforced,
    );

    for harness in [&failing, &hanging] {
        let outcome = harness
            .service
            .authorize(&actor(principal), &action(READ), &target(DOC))
            .await
            .unwrap();

        assert_eq!(outcome.decision, Decision::Allowed);
        assert_eq!(harness.provider.members_calls(), 2);
    }
}
