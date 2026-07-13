use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use atlas_domain::{
    DomainError,
    entities::identity::{MemberRole, Workspace},
    ids::{ApiKeyId, DocumentId, ProjectId, UserId, WorkspaceId},
    permissions::{
        Capability, CapabilityAction, CapabilityFamily, ChainSegment, Principal, ResourceChain,
        ResourceRef, ResourceRole, Visibility, VisibilityRole,
    },
};
use chrono::Utc;
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
    source.set_principal_facts(PrincipalFacts {
        principal_grants: vec![(ResourceRef::Project(project), ResourceRole::Viewer)],
        ..PrincipalFacts::default()
    });

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
async fn batch_authorization_caps_api_keys_and_requires_the_family_read_scope() {
    let document = DocumentId(Uuid::now_v7());
    let source = TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    source.set_principal_facts(PrincipalFacts {
        principal_grants: vec![(ResourceRef::Workspace, ResourceRole::Admin)],
        creator_membership: Some(MemberRole::Member),
        creator_grants: vec![(ResourceRef::Workspace, ResourceRole::Admin)],
    });
    let service = BatchAuthorizationService::new(source);

    let decisions = service
        .authorize(
            &api_key_context(vec![Capability {
                family: CapabilityFamily::Docs,
                action: CapabilityAction::Read,
            }]),
            &[ProjectionSubject::Document(document.0)],
        )
        .await
        .unwrap();

    assert_eq!(decisions, vec![true]);

    let missing_scope_source =
        TestSource::with_facts(vec![workspace_fact(0, SubjectFamily::Documents)]);
    missing_scope_source.set_principal_facts(PrincipalFacts {
        principal_grants: vec![(ResourceRef::Workspace, ResourceRole::Admin)],
        creator_membership: Some(MemberRole::Member),
        creator_grants: vec![(ResourceRef::Workspace, ResourceRole::Admin)],
    });
    let missing_scope = BatchAuthorizationService::new(missing_scope_source);
    let decisions = missing_scope
        .authorize(
            &api_key_context(Vec::new()),
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
            principal_facts: Arc::new(std::sync::Mutex::new(PrincipalFacts::default())),
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
    ProjectionAuthContext::from_validated(
        workspace(),
        Principal::User(UserId(Uuid::now_v7())),
        Some(MemberRole::Member),
        None,
    )
}

fn api_key_context(scopes: Vec<Capability>) -> ProjectionAuthContext {
    let key_id = ApiKeyId(Uuid::now_v7());
    let creator_id = UserId(Uuid::now_v7());
    ProjectionAuthContext::from_validated(
        workspace(),
        Principal::ApiKey(key_id),
        None,
        Some(atlas_domain::entities::identity::ApiKey {
            id: key_id,
            workspace_id: None,
            created_by_user_id: creator_id,
            name: "key".into(),
            token_hash: "hash".into(),
            type_: Default::default(),
            expires_at: None,
            last_used_at: None,
            revoked_at: None,
            created_at: Utc::now(),
            is_global: false,
            scopes,
        }),
    )
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

fn workspace() -> Workspace {
    Workspace {
        id: WorkspaceId(Uuid::now_v7()),
        name: "workspace".into(),
        slug: "workspace".into(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}
