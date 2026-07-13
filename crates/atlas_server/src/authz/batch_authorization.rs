use std::collections::BTreeSet;

use async_trait::async_trait;
use atlas_domain::{
    DomainError,
    entities::identity::MemberRole,
    ids::{ApiKeyId, UserId, WorkspaceId},
    permissions::{
        Capability, CapabilityAction, CapabilityFamily, Principal, ResolutionInput, ResourceChain,
        ResourceRef, ResourceRole,
    },
};
use uuid::Uuid;

/// Request-bound authorization state for viewer-relative projections.
///
/// This value is constructed only by `Authorized` after middleware has validated
/// the request principal. Its fields deliberately remain private so handlers
/// cannot turn caller-provided identifiers into authorization state.
#[derive(Clone)]
pub(crate) struct ProjectionAuthContext {
    workspace_id: WorkspaceId,
    principal: Principal,
}

impl ProjectionAuthContext {
    pub(super) fn from_validated(workspace_id: WorkspaceId, principal: Principal) -> Self {
        Self {
            workspace_id,
            principal,
        }
    }

    pub(crate) fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectionSubject {
    Document(Uuid),
    Task(Uuid),
    Attachment(Uuid),
    SourceComment(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubjectFamily {
    Documents,
    Tasks,
}

pub(crate) struct SubjectFact {
    pub ordinal: usize,
    pub chain: ResourceChain,
    pub family: SubjectFamily,
}

#[derive(Debug, Clone)]
pub(crate) struct UserFacts {
    pub user_id: UserId,
    pub is_active: bool,
    pub effective_membership: Option<MemberRole>,
    pub is_root_admin: bool,
    pub grants: Vec<(ResourceRef, ResourceRole)>,
}

#[derive(Debug, Clone)]
pub(crate) struct ApiKeyFacts {
    pub key_id: ApiKeyId,
    pub is_active: bool,
    pub is_revoked: bool,
    pub is_expired: bool,
    pub is_global: bool,
    pub scopes: Vec<Capability>,
    pub grants: Vec<(ResourceRef, ResourceRole)>,
    pub creator: UserFacts,
}

#[derive(Debug, Clone)]
pub(crate) enum PrincipalFacts {
    User(UserFacts),
    ApiKey(ApiKeyFacts),
}

impl PrincipalFacts {
    #[cfg(test)]
    pub(super) fn active_user(
        membership: Option<MemberRole>,
        grants: Vec<(ResourceRef, ResourceRole)>,
    ) -> Self {
        Self::User(UserFacts {
            user_id: UserId(Uuid::nil()),
            is_active: true,
            effective_membership: membership,
            is_root_admin: false,
            grants,
        })
    }
}

#[async_trait]
pub(crate) trait BatchAuthorizationSource: Send + Sync {
    async fn load_subject_facts(
        &self,
        context: &ProjectionAuthContext,
        subjects: &[ProjectionSubject],
    ) -> Result<Vec<SubjectFact>, DomainError>;

    async fn load_principal_facts(
        &self,
        context: &ProjectionAuthContext,
        resources: &[ResourceRef],
    ) -> Result<PrincipalFacts, DomainError>;
}

/// Applies the existing permission resolver to a bounded heterogeneous batch.
///
/// The source is deliberately split into exactly two bulk operations. Missing
/// subject facts are denied; source failures propagate instead of becoming a
/// denial sentinel so callers can fail the whole projection safely.
pub(crate) struct BatchAuthorizationService<S> {
    source: S,
}

impl<S> BatchAuthorizationService<S>
where
    S: BatchAuthorizationSource,
{
    pub(crate) fn new(source: S) -> Self {
        Self { source }
    }

    pub(crate) async fn authorize(
        &self,
        context: &ProjectionAuthContext,
        subjects: &[ProjectionSubject],
    ) -> Result<Vec<bool>, DomainError> {
        if subjects.is_empty() {
            return Ok(Vec::new());
        }

        if subjects.len() > 200 {
            return Err(DomainError::InvalidInput {
                message: "batch authorization accepts at most 200 subjects".into(),
            });
        }

        let facts = self.source.load_subject_facts(context, subjects).await?;
        validate_subject_facts(&facts, subjects)?;
        let resources = distinct_resources(&facts);
        let principal_facts = self
            .source
            .load_principal_facts(context, &resources)
            .await?;
        validate_principal_facts(context, &principal_facts)?;

        let mut decisions = vec![false; subjects.len()];
        for fact in facts {
            let decision =
                decisions
                    .get_mut(fact.ordinal)
                    .ok_or_else(|| DomainError::Internal {
                        message: "batch authorization source returned an invalid ordinal".into(),
                    })?;
            *decision = authorize_fact(context, &principal_facts, &fact);
        }

        Ok(decisions)
    }
}

fn validate_subject_facts(
    facts: &[SubjectFact],
    subjects: &[ProjectionSubject],
) -> Result<(), DomainError> {
    let mut seen_ordinals = vec![false; subjects.len()];

    for fact in facts {
        let Some(subject) = subjects.get(fact.ordinal) else {
            return Err(DomainError::Internal {
                message: "batch authorization source returned an invalid ordinal".into(),
            });
        };

        let seen = seen_ordinals
            .get_mut(fact.ordinal)
            .ok_or_else(|| DomainError::Internal {
                message: "batch authorization source returned an invalid ordinal".into(),
            })?;

        if *seen {
            return Err(DomainError::Internal {
                message: "batch authorization source returned duplicate subject facts".into(),
            });
        }

        if !subject_matches_family(*subject, fact.family) {
            return Err(DomainError::Internal {
                message: "batch authorization source returned facts for the wrong subject family"
                    .into(),
            });
        }

        *seen = true;
    }

    Ok(())
}

fn subject_matches_family(subject: ProjectionSubject, family: SubjectFamily) -> bool {
    matches!(
        (subject, family),
        (ProjectionSubject::Document(_), SubjectFamily::Documents)
            | (ProjectionSubject::Task(_), SubjectFamily::Tasks)
            | (
                ProjectionSubject::Attachment(_),
                SubjectFamily::Documents | SubjectFamily::Tasks
            )
            | (
                ProjectionSubject::SourceComment(_),
                SubjectFamily::Documents | SubjectFamily::Tasks
            )
    )
}

fn distinct_resources(facts: &[SubjectFact]) -> Vec<ResourceRef> {
    let mut resources = BTreeSet::new();

    for fact in facts {
        for segment in &fact.chain.segments {
            resources.insert(resource_key(&segment.resource));
        }
    }

    resources.into_iter().map(resource_from_key).collect()
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ResourceKey {
    Workspace,
    Project(Uuid),
    Folder(Uuid),
    Document(Uuid),
    Board(Uuid),
}

fn resource_key(resource: &ResourceRef) -> ResourceKey {
    match resource {
        ResourceRef::Workspace => ResourceKey::Workspace,
        ResourceRef::Project(id) => ResourceKey::Project(id.0),
        ResourceRef::Folder(id) => ResourceKey::Folder(id.0),
        ResourceRef::Document(id) => ResourceKey::Document(id.0),
        ResourceRef::Board(id) => ResourceKey::Board(id.0),
    }
}

fn resource_from_key(key: ResourceKey) -> ResourceRef {
    match key {
        ResourceKey::Workspace => ResourceRef::Workspace,
        ResourceKey::Project(id) => ResourceRef::Project(atlas_domain::ids::ProjectId(id)),
        ResourceKey::Folder(id) => ResourceRef::Folder(atlas_domain::ids::FolderId(id)),
        ResourceKey::Document(id) => ResourceRef::Document(atlas_domain::ids::DocumentId(id)),
        ResourceKey::Board(id) => ResourceRef::Board(atlas_domain::ids::BoardId(id)),
    }
}

fn authorize_fact(
    context: &ProjectionAuthContext,
    principal_facts: &PrincipalFacts,
    fact: &SubjectFact,
) -> bool {
    let role = match principal_facts {
        PrincipalFacts::User(facts) => resolve_user_role(&context.principal, facts, &fact.chain),
        PrincipalFacts::ApiKey(facts) => resolve_api_key_role(context, facts, &fact.chain),
    };

    role.is_some_and(|role| role >= ResourceRole::Viewer)
        && api_key_has_read_capability(principal_facts, fact.family)
}

fn resolve_user_role(
    principal: &Principal,
    facts: &UserFacts,
    chain: &ResourceChain,
) -> Option<ResourceRole> {
    if !facts.is_active || (!facts.is_root_admin && facts.effective_membership.is_none()) {
        return None;
    }

    let membership = if facts.is_root_admin {
        Some(MemberRole::Admin)
    } else {
        facts.effective_membership.clone()
    };

    atlas_domain::permissions::resolve(&ResolutionInput {
        principal,
        membership,
        chain,
        grants: &facts.grants,
    })
}

fn resolve_api_key_role(
    context: &ProjectionAuthContext,
    facts: &ApiKeyFacts,
    chain: &ResourceChain,
) -> Option<ResourceRole> {
    if !facts.is_active || facts.is_revoked || facts.is_expired {
        return None;
    }

    let creator = Principal::User(facts.creator.user_id);
    let creator_role = resolve_user_role(&creator, &facts.creator, chain);

    let role = if facts.is_global {
        creator_role
    } else {
        let key_role = atlas_domain::permissions::resolve(&ResolutionInput {
            principal: &context.principal,
            membership: None,
            chain,
            grants: &facts.grants,
        });
        match (key_role, creator_role) {
            (Some(key_role), Some(creator_role)) => Some(key_role.min(creator_role)),
            _ => None,
        }
    };

    role.map(|role| role.min(ResourceRole::Editor))
}

fn api_key_has_read_capability(principal_facts: &PrincipalFacts, family: SubjectFamily) -> bool {
    let PrincipalFacts::ApiKey(facts) = principal_facts else {
        return true;
    };

    let family = match family {
        SubjectFamily::Documents => CapabilityFamily::Docs,
        SubjectFamily::Tasks => CapabilityFamily::Tasks,
    };
    let capability = Capability {
        family,
        action: CapabilityAction::Read,
    };

    facts.scopes.contains(&capability)
}

fn validate_principal_facts(
    context: &ProjectionAuthContext,
    facts: &PrincipalFacts,
) -> Result<(), DomainError> {
    match (&context.principal, facts) {
        (Principal::User(context_id), PrincipalFacts::User(facts))
            if *context_id == facts.user_id =>
        {
            validate_grants(&facts.grants)
        }
        (Principal::ApiKey(context_id), PrincipalFacts::ApiKey(facts))
            if *context_id == facts.key_id =>
        {
            validate_grants(&facts.grants)?;
            validate_user_facts(&facts.creator)?;
            validate_scopes(&facts.scopes)
        }
        _ => Err(DomainError::Internal {
            message: "batch authorization source returned facts for the wrong principal".into(),
        }),
    }
}

fn validate_user_facts(facts: &UserFacts) -> Result<(), DomainError> {
    validate_grants(&facts.grants)
}

fn validate_grants(grants: &[(ResourceRef, ResourceRole)]) -> Result<(), DomainError> {
    let mut seen = BTreeSet::new();
    for (resource, role) in grants {
        if !seen.insert(resource_key(resource)) {
            return Err(DomainError::Internal {
                message: "batch authorization source returned duplicate grants".into(),
            });
        }
        let _ = role;
    }
    Ok(())
}

fn validate_scopes(scopes: &[Capability]) -> Result<(), DomainError> {
    for (index, scope) in scopes.iter().enumerate() {
        if scopes.iter().take(index).any(|previous| previous == scope) {
            return Err(DomainError::Internal {
                message: "batch authorization source returned duplicate key scopes".into(),
            });
        }
    }
    Ok(())
}
