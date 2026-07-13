use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use atlas_domain::{
    DomainError,
    entities::identity::MemberRole,
    ids::{ApiKeyId, DocumentId, ProjectId, UserId, WorkspaceId},
    permissions::{
        Capability, CapabilityAction, CapabilityFamily, ChainSegment, Principal, ResourceChain,
        ResourceRef, ResourceRole, Visibility, VisibilityRole,
    },
};
use uuid::Uuid;

use super::batch_authorization::{
    BatchAuthorizationService, BatchAuthorizationSource, PrincipalFacts, ProjectionAuthContext,
    ProjectionSubject, SubjectFact, SubjectFamily,
};

#[tokio::test]
async fn batch_authorization_resolves_direct_and_inherited_grants_with_two_source_calls() {
    let document = DocumentId(Uuid::now_v7());
    let project = ProjectId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![SubjectFact {
        ordinal: 0,
        chain: ResourceChain {
            segments: vec![
                ChainSegment {
                    resource: ResourceRef::Document(document),
                    visibility: None,
                },
                ChainSegment {
                    resource: ResourceRef::Project(project),
                    visibility: Some(Visibility::Private),
                },
                ChainSegment {
                    resource: ResourceRef::Workspace,
                    visibility: None,
                },
            ],
        },
        family: SubjectFamily::Documents,
    }]);
    source.set_principal_facts(PrincipalFacts::active_user(
        Some(MemberRole::Member),
        vec![(ResourceRef::Project(project), ResourceRole::Viewer)],
    ));

    let service = BatchAuthorizationService::new(source.clone());
    let decisions = service
        .authorize(&user_context(), &[ProjectionSubject::Document(document.0)])
        .await
        .unwrap();

    assert_eq!(decisions, vec![true]);
    assert_eq!(source.subject_calls.load(Ordering::Relaxed), 1);
    assert_eq!(source.principal_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn batch_authorization_allows_visibility_and_denies_missing_grants() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![
        SubjectFact {
            ordinal: 0,
            chain: ResourceChain {
                segments: vec![
                    ChainSegment {
                        resource: ResourceRef::Document(document),
                        visibility: None,
                    },
                    ChainSegment {
                        resource: ResourceRef::Project(ProjectId(Uuid::now_v7())),
                        visibility: Some(Visibility::Workspace(VisibilityRole::Viewer)),
                    },
                    ChainSegment {
                        resource: ResourceRef::Workspace,
                        visibility: None,
                    },
                ],
            },
            family: SubjectFamily::Documents,
        },
        SubjectFact {
            ordinal: 1,
            chain: ResourceChain {
                segments: vec![ChainSegment {
                    resource: ResourceRef::Workspace,
                    visibility: None,
                }],
            },
            family: SubjectFamily::Documents,
        },
    ]);
    let service = BatchAuthorizationService::new(source);

    let decisions = service
        .authorize(
            &user_context(),
            &[
                ProjectionSubject::Document(document.0),
                ProjectionSubject::Document(Uuid::now_v7()),
            ],
        )
        .await
        .unwrap();

    assert_eq!(decisions, vec![true, false]);
}

#[tokio::test]
async fn batch_authorization_propagates_source_failures_without_a_decision() {
    let service = BatchAuthorizationService::new(TestSource::failing());

    let error = service
        .authorize(&user_context(), &[ProjectionSubject::Task(Uuid::now_v7())])
        .await
        .unwrap_err();

    assert!(matches!(error, DomainError::Internal { .. }));
}

#[tokio::test]
async fn batch_authorization_rejects_duplicate_subject_ordinals() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![
        workspace_fact(0, SubjectFamily::Documents),
        workspace_fact(0, SubjectFamily::Documents),
    ]);
    let service = BatchAuthorizationService::new(source);

    let error = service
        .authorize(&user_context(), &[ProjectionSubject::Document(document.0)])
        .await
        .unwrap_err();

    assert!(matches!(error, DomainError::Internal { .. }));
}

#[tokio::test]
async fn batch_authorization_caps_api_keys_and_requires_the_family_read_scope() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    source.set_principal_facts(PrincipalFacts::ApiKey(api_key_facts(vec![Capability {
        family: CapabilityFamily::Docs,
        action: CapabilityAction::Read,
    }])));
    let service = BatchAuthorizationService::new(source);

    let decisions = service
        .authorize(
            &api_key_context(),
            &[ProjectionSubject::Document(document.0)],
        )
        .await
        .unwrap();

    assert_eq!(decisions, vec![true]);

    let missing_scope_source =
        TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    missing_scope_source.set_principal_facts(PrincipalFacts::ApiKey(api_key_facts(Vec::new())));
    let missing_scope = BatchAuthorizationService::new(missing_scope_source);
    let decisions = missing_scope
        .authorize(
            &api_key_context(),
            &[ProjectionSubject::Document(document.0)],
        )
        .await
        .unwrap();

    assert_eq!(decisions, vec![false]);
}

#[tokio::test]
async fn batch_authorization_uses_the_same_two_source_calls_for_two_hundred_subjects() {
    let facts = (0..200)
        .map(|ordinal| workspace_fact(ordinal, SubjectFamily::Tasks))
        .collect();
    let source = TestSource::with_facts(facts);
    let service = BatchAuthorizationService::new(source.clone());
    let subjects = (0..200)
        .map(|_| ProjectionSubject::Task(Uuid::now_v7()))
        .collect::<Vec<_>>();

    let decisions = service.authorize(&user_context(), &subjects).await.unwrap();

    assert_eq!(decisions, vec![false; 200]);
    assert_eq!(source.subject_calls.load(Ordering::Relaxed), 1);
    assert_eq!(source.principal_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn batch_authorization_uses_fresh_user_facts_instead_of_request_membership() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    source.set_principal_facts(PrincipalFacts::active_user(
        Some(MemberRole::Member),
        vec![(ResourceRef::Workspace, ResourceRole::Viewer)],
    ));
    let service = BatchAuthorizationService::new(source);

    let decisions = service
        .authorize(
            &ProjectionAuthContext::from_validated(
                workspace_id(),
                Principal::User(UserId(Uuid::nil())),
            ),
            &[ProjectionSubject::Document(document.0)],
        )
        .await
        .unwrap();

    assert_eq!(decisions, vec![true]);
}

#[tokio::test]
async fn batch_authorization_denies_disabled_fresh_user_facts() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    source.set_principal_facts(PrincipalFacts::User(
        super::batch_authorization::UserFacts {
            user_id: UserId(Uuid::nil()),
            is_active: false,
            effective_membership: Some(MemberRole::Admin),
            is_root_admin: false,
            grants: vec![(ResourceRef::Workspace, ResourceRole::Admin)],
        },
    ));

    let decisions = BatchAuthorizationService::new(source)
        .authorize(
            &ProjectionAuthContext::from_validated(
                workspace_id(),
                Principal::User(UserId(Uuid::nil())),
            ),
            &[ProjectionSubject::Document(document.0)],
        )
        .await
        .unwrap();

    assert_eq!(decisions, vec![false]);
}

#[tokio::test]
async fn batch_authorization_uses_fresh_root_admin_facts_without_membership() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    source.set_principal_facts(PrincipalFacts::User(
        super::batch_authorization::UserFacts {
            user_id: UserId(Uuid::nil()),
            is_active: true,
            effective_membership: None,
            is_root_admin: true,
            grants: Vec::new(),
        },
    ));

    let decisions = BatchAuthorizationService::new(source)
        .authorize(&user_context(), &[ProjectionSubject::Document(document.0)])
        .await
        .unwrap();

    assert_eq!(decisions, vec![true]);
}

#[tokio::test]
async fn batch_authorization_denies_revoked_fresh_api_key_facts() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    let mut facts = api_key_facts(vec![Capability {
        family: CapabilityFamily::Docs,
        action: CapabilityAction::Read,
    }]);
    facts.is_revoked = true;
    source.set_principal_facts(PrincipalFacts::ApiKey(facts));

    let decisions = BatchAuthorizationService::new(source)
        .authorize(
            &api_key_context(),
            &[ProjectionSubject::Document(document.0)],
        )
        .await
        .unwrap();

    assert_eq!(decisions, vec![false]);
}

#[tokio::test]
async fn batch_authorization_rejects_conflicting_fresh_grants() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    source.set_principal_facts(PrincipalFacts::active_user(
        Some(MemberRole::Member),
        vec![
            (ResourceRef::Workspace, ResourceRole::Viewer),
            (ResourceRef::Workspace, ResourceRole::Editor),
        ],
    ));

    let error = BatchAuthorizationService::new(source)
        .authorize(&user_context(), &[ProjectionSubject::Document(document.0)])
        .await
        .unwrap_err();

    assert!(matches!(error, DomainError::Internal { .. }));
}

#[derive(Clone)]
struct TestSource {
    facts: Arc<std::sync::Mutex<Vec<SubjectFact>>>,
    principal_facts: Arc<std::sync::Mutex<PrincipalFacts>>,
    failure: bool,
    subject_calls: Arc<AtomicUsize>,
    principal_calls: Arc<AtomicUsize>,
}

impl TestSource {
    fn with_facts(facts: Vec<SubjectFact>) -> Self {
        Self {
            facts: Arc::new(std::sync::Mutex::new(facts)),
            principal_facts: Arc::new(std::sync::Mutex::new(PrincipalFacts::active_user(
                Some(MemberRole::Member),
                Vec::new(),
            ))),
            failure: false,
            subject_calls: Arc::new(AtomicUsize::new(0)),
            principal_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn failing() -> Self {
        Self {
            failure: true,
            ..Self::with_facts(Vec::new())
        }
    }

    fn set_principal_facts(&self, facts: PrincipalFacts) {
        *self.principal_facts.lock().unwrap() = facts;
    }
}

#[async_trait]
impl BatchAuthorizationSource for TestSource {
    async fn load_subject_facts(
        &self,
        _context: &ProjectionAuthContext,
        _subjects: &[ProjectionSubject],
    ) -> Result<Vec<SubjectFact>, DomainError> {
        self.subject_calls.fetch_add(1, Ordering::Relaxed);
        if self.failure {
            return Err(DomainError::Internal {
                message: "subject query failed".into(),
            });
        }

        Ok(std::mem::take(&mut *self.facts.lock().unwrap()))
    }

    async fn load_principal_facts(
        &self,
        _context: &ProjectionAuthContext,
        _resources: &[ResourceRef],
    ) -> Result<PrincipalFacts, DomainError> {
        self.principal_calls.fetch_add(1, Ordering::Relaxed);
        Ok(self.principal_facts.lock().unwrap().clone())
    }
}

fn user_context() -> ProjectionAuthContext {
    ProjectionAuthContext::from_validated(workspace_id(), Principal::User(UserId(Uuid::nil())))
}

fn api_key_context() -> ProjectionAuthContext {
    let key_id = ApiKeyId(Uuid::from_u128(1));
    ProjectionAuthContext::from_validated(workspace_id(), Principal::ApiKey(key_id))
}

fn api_key_facts(scopes: Vec<Capability>) -> super::batch_authorization::ApiKeyFacts {
    super::batch_authorization::ApiKeyFacts {
        key_id: ApiKeyId(Uuid::from_u128(1)),
        is_active: true,
        is_revoked: false,
        is_expired: false,
        is_global: false,
        scopes,
        grants: vec![(ResourceRef::Workspace, ResourceRole::Admin)],
        creator: super::batch_authorization::UserFacts {
            user_id: UserId(Uuid::from_u128(2)),
            is_active: true,
            effective_membership: Some(MemberRole::Member),
            is_root_admin: false,
            grants: vec![(ResourceRef::Workspace, ResourceRole::Admin)],
        },
    }
}

fn workspace_fact(ordinal: usize, family: SubjectFamily) -> SubjectFact {
    SubjectFact {
        ordinal,
        chain: ResourceChain {
            segments: vec![ChainSegment {
                resource: ResourceRef::Workspace,
                visibility: None,
            }],
        },
        family,
    }
}

fn workspace_id() -> WorkspaceId {
    WorkspaceId(Uuid::from_u128(3))
}
